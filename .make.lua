-- balthasar's build, as recipes. This replaced the Makefile; there is no other.
--
--   make            the recipes, with what each of them says it does
--   make build      the binary
--   make test       the suite
--   make verify     the whole local gate
--
-- At an oslo prompt in this directory `make` is enough; everywhere else it is `oslo make`.
-- CI has no oslo, so it calls the language's own tool -- nothing here is on the release path.

local make = oslo.make

-- Name and version live in PROJECT, one per line, so every tool reads them from one place.
local function project()
  local found = {}
  for line in (oslo.fs.read("PROJECT") or ""):gmatch("[^\n]+") do
    local value = line:match("^%s*([^#%[%s]%S*)%s*$")
    if value then found[#found + 1] = value end
  end
  return found[1] or "balthasar", found[2] or "0.1.0"
end

local NAME, VERSION = project()
local PREFIX = os.getenv("PREFIX") or (os.getenv("HOME") .. "/.local")

------------------------------------------------------------------ what was built

local function dim(text)
  return oslo.ui.style(text, { dim = true })
end

local function line(label, value)
  print(dim(oslo.ui.pad(label, 8)) .. value)
end

-- `1524720` -> `1,524,720`. A number this long is read in groups or not at all.
local function grouped(n)
  local text = tostring(math.floor(n))
  local out = text:sub(-3)
  local at = #text - 3
  while at > 0 do
    out = text:sub(math.max(1, at - 2), at) .. "," .. out
    at = at - 3
  end
  return out
end

-- Asked of the ELF, not assumed. `ldd` is not enough on its own: it prints "statically linked" for
-- a binary that still carries an INTERP and will not start.
local function linkage(path)
  local segments = oslo.run{ "readelf", "-l", path, capture = true }
  if not segments.ok then return nil end
  local dynamic = oslo.run{ "readelf", "-d", path, capture = true }
  if (segments.out or ""):find("program interpreter") or (dynamic.out or ""):find("NEEDED") then
    return "dynamic"
  end
  return "static"
end

-- What was built, how big it is, and whether it needs anything on the target machine. Silent when
-- the artifact is not there, so a recipe that builds nothing does not pretend it did.
local function report(path)
  local stat = oslo.fs.stat(path)
  if not stat then return end
  local megabytes = ("%.2f MB"):format(stat.size / 1048576)

  print("")
  print(oslo.ui.title(("%s %s   %s"):format(NAME, VERSION, megabytes)))
  line("binary", path)
  -- Bytes beside megabytes: `1.45 MB` cannot be subtracted from last week's `1.42 MB` to get one.
  line("size", megabytes .. dim("   " .. grouped(stat.size) .. " bytes"))

  local kind = linkage(path)
  if kind == "static" then
    line("linking", oslo.ui.style("✓ static", { fg = "green" }) ..
                    dim("   no runtime dependencies"))
  elseif kind == "dynamic" then
    line("linking", oslo.ui.style("dynamic", { fg = "yellow" }) ..
                    dim("   needs a matching libc on the target machine"))
  end
  print("")
end


make.recipe{ name = "version", desc = "what this checkout calls itself",
             run = function() print(("%s v%s"):format(NAME, VERSION)) end }

local function need(tool, why)
  assert(oslo.run{ "sh", "-c", "command -v " .. tool, capture = true }.ok, why)
end

make.recipe{
  name = "release",
  desc = "cut a version: --type patch | minor | major | M.m.p",
  params = { { "--type", desc = "patch | minor | major | M.m.p" } },
  run = function(a)
    need("git-rel", "git-rel is not installed; install it first")
    assert(type(a.type) == "string",
           "which release? make release --type patch|minor|major|M.m.p")
    sh.git("rel", a.type)
  end,
}

make.recipe{
  name = "changelog",
  desc = "regenerate CHANGELOG.md",
  run = function()
    need("git-cliff", "git-cliff is not installed; install it first")
    sh.git("cliff", "-o", "CHANGELOG.md")
  end,
}

---------------------------------------------------------------------------- rust

-- Everything that produces a binary produces the real one.
--
-- The debug build is 62 MB and the release build is 6.7 MB, and the difference is entirely
-- symbols nobody reads. `[profile.release]` in Cargo.toml already does the work -- thin LTO and
-- stripped symbols -- and the recipes simply never asked for it.
--
-- Deliberately NOT applied to `test`, `check` and `clippy`. Release turns off integer overflow
-- panics and `debug_assert!`, so a suite that ran there would pass on an overflow the debug
-- build catches -- and this is a project full of scores, decay curves and counters. It also
-- costs a second full compile of everything. `test-release` exists for when the shipped code
-- itself is what needs testing.
local PROFILE = "--release"

-- Static, against musl. A dynamic binary needs a matching libc wherever it lands, which for a
-- tool people copy between machines is a failure that happens at somebody else's prompt.
--
-- The C toolchain comes from the flake as a path rather than a package: `rusqlite` compiles
-- SQLite's amalgamation, so the static build needs a compiler targeting musl -- and putting one
-- on the default search path would let an ordinary build compile against musl headers while
-- linking against glibc, which succeeds silently and crashes at startup.
local TRIPLE = "x86_64-unknown-linux-musl"
local BUILT = "target/" .. TRIPLE .. "/release"

local function musl()
  local cc = os.getenv("MUSL_CC")
  assert(cc, "MUSL_CC is not set -- this needs `nix develop`, where the flake hands it over")
  return {
    "CC_x86_64_unknown_linux_musl=" .. cc .. "/bin/cc",
    "AR_x86_64_unknown_linux_musl=" .. cc .. "/bin/ar",
    "CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=" .. cc .. "/bin/cc",
  }
end

make.recipe{ name = "build", desc = "the binary: static, optimized, stripped",
             run = function()
               local argv = musl()
               for _, a in ipairs({ "cargo", "build", "--workspace", PROFILE,
                                    "--target", TRIPLE }) do
                 argv[#argv + 1] = a
               end
               table.insert(argv, 1, "env")
               assert(oslo.run(argv).ok, "build failed")
               report(BUILT .. "/" .. NAME)
             end }
make.alias("b", "build")

make.recipe{ name = "build-host", desc = "the binary for this machine, dynamic",
             run = function()
               sh.cargo("build", "--workspace", PROFILE)
               report("target/release/" .. NAME)
             end }

make.recipe{ name = "build-debug", desc = "the binary, unoptimized and quick",
             run = function()
               sh.cargo("build", "--workspace")
               report("target/debug/" .. NAME)
             end }

make.recipe{
  name = "run",
  desc = "run balthasar: --args='recall \"make test\"'",
  params = { { "--args", desc = "what to pass balthasar", default = "" } },
  run = function(a)
    -- Split here rather than through oslo: `oslo.text` is not lent to a make script, and
    -- reaching for it made `make run` fail with "could not index into a nil value" for anyone
    -- who tried it.
    local args = {}
    for word in (a.args or ""):gmatch("%S+") do
      args[#args + 1] = word
    end
    sh.cargo("run", "--quiet", PROFILE, "--bin", NAME, "--", table.unpack(args))
  end,
}
make.alias("r", "run")

make.recipe{ name = "test", desc = "the suite",
             run = function() sh.cargo("test", "--workspace", "--all-targets") end }
make.alias("t", "test")

make.recipe{ name = "test-all", desc = "the suite, with every feature on",
             run = function() sh.cargo("test", "--workspace", "--all-targets", "--all-features") end }

-- What the binary actually does, rather than what the debug build does. Slower to compile and
-- blind to overflow, so it is a separate recipe rather than the default -- run it before a
-- release, not on every change.
make.recipe{ name = "test-release", desc = "the suite against the optimized build",
             run = function() sh.cargo("test", "--workspace", "--all-targets", PROFILE) end }

make.recipe{ name = "check", desc = "type-check every target",
             run = function() sh.cargo("check", "--workspace", "--all-targets") end }

make.recipe{ name = "check-all", desc = "type-check every target, every feature",
             run = function() sh.cargo("check", "--workspace", "--all-targets", "--all-features") end }

make.recipe{ name = "clippy", desc = "clippy, with warnings denied",
             run = function()
               sh.cargo("clippy", "--workspace", "--all-targets", "--all-features", "--", "-Dwarnings")
             end }

make.recipe{
  name = "rustdoc",
  desc = "build the docs, with warnings denied",
  run = function()
    local built = oslo.run{ "env", "RUSTDOCFLAGS=-Dwarnings",
                            "cargo", "doc", "--workspace", "--all-features", "--no-deps" }
    assert(built.ok, "rustdoc failed")
  end,
}

make.recipe{ name = "fmt", desc = "format the workspace",
             run = function() sh.cargo("fmt", "--all") end }

make.recipe{ name = "fmt-check", desc = "fail if anything is unformatted",
             run = function() sh.cargo("fmt", "--all", "--", "--check") end }

make.recipe{ name = "clean", desc = "remove every build output",
             run = function() sh.cargo("clean") end }

make.recipe{ name = "compile", desc = "clean, then build", deps = { "clean", "build" } }
make.alias("c", "compile")

------------------------------------------------------------------------- gates

-- Not advisory. Each one mechanizes a commitment the code states in its own doc comments,
-- and each one exists because
-- the alternative is remembering to check -- which is how every reference implementation in
-- xtra/ ended up with a 2,000-line file and a store that deletes.

local GATES = {
  { "gate-file-size",    "no .rs over 800 lines" },
  { "gate-no-delete",    "nothing is deleted outside purge.rs" },
  { "gate-independent",  "no Rust file names a harness" },
  { "gate-witnessed",    "every asserted memory answers for itself" },
  { "gate-untrusted",    "untrusted content cannot become durable instruction" },
  { "gate-no-exec",      "balthasar describes procedures and never runs them" },
}

for _, gate in ipairs(GATES) do
  local name, desc = gate[1], gate[2]
  make.recipe{
    name = name, desc = desc,
    run = function()
      local ran = oslo.run{ "sh", "scripts/" .. name .. ".sh" }
      assert(ran.ok, name .. " failed")
    end,
  }
end

make.recipe{
  name = "gates",
  desc = "every architectural gate",
  deps = {
    "gate-file-size",
    "gate-no-delete",
    "gate-independent",
    "gate-witnessed",
    "gate-untrusted",
    "gate-no-exec",
  },
}

make.recipe{
  name = "gate-no-llm",
  desc = "the suite passes with no key, no network, no embeddings",
  run = function()
    local ran = oslo.run{ "sh", "scripts/gate-no-llm.sh" }
    assert(ran.ok, "gate-no-llm failed")
  end,
}

make.recipe{
  name = "verify",
  desc = "the whole local gate",
  deps = { "fmt-check", "check", "test", "clippy", "rustdoc", "gates", "gate-no-llm" },
}
make.alias("v", "verify")
