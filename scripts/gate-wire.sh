#!/bin/sh
# The family speaks one way, and a sixth wire cannot invent a sixth spelling.
#
# Five wires had grown five ways to say the same thing: `say`/`heard` on melchior's pipe,
# `to`/`from` on the surface pipe, `message` on the tool-peer pipe, `said` on the streamed
# answer, and a call envelope on the sockets. Nothing anywhere said which was meant, and nothing
# could: each was internally consistent, and each was written by somebody who had not read the
# other four.
#
# **The failure is silent.** Two of those wires exist as byte-identical copies in two
# repositories -- the surface frames, and the streamed answer -- so when two spellings drift
# nothing fails and no test goes red. The surface simply stops being answered, and it presents
# much later as "that tool never draws".
#
# Two shapes only. A **call** is answered and carries a reply; an **event** is not. Every enum
# whose variants cross a process boundary is tagged `event`. The rule is written out in this
# repository's wire module.
#
# A **data** discriminator is a different thing: a tagged enum nested *inside* a message, which
# names a shape rather than an event. Those are allowed their own key, and each is listed below
# with why. The list is the point -- adding to it is a deliberate act, and a new wire that
# invents a tag is not on it.
#
# POSIX for the same reason the others are: /bin/sh on the runner is dash.
set -eu
ROOT="${GATE_ROOT:-crates}"

# Data discriminators, and what each names. Nested inside a message; never a frame of its own.
allowed() {
  case "$1" in
    # a transport, a scope, an action, a body, an endpoint: shapes, named where they are used
    kind) return 0 ;;
    # a provider's own content blocks, which are not this family's to name
    type) return 0 ;;
    # how a tool result is drawn, carried inside the result
    shown) return 0 ;;
    # what a surface was told, carried inside an `answer` event
    answer) return 0 ;;
    # the journal on disk. A file is not a wire, and its records outlive every protocol here
    record) return 0 ;;
    # magi's own UI-to-daemon protocol: CBOR in an envelope, two halves of one program that
    # ship together, deliberately not the family shape -- a peer from another build is turned
    # away at the boundary rather than after its fields have been read
    command) return 0 ;;
    *) return 1 ;;
  esac
}

wrong=""
for file in $(find "$ROOT" -name '*.rs' -not -path '*/target/*' 2>/dev/null | sort); do
  # `#[serde(… tag = "x" …)]`, whatever order the attributes are written in.
  for found in $(grep -o 'tag *= *"[a-z_]*"' "$file" 2>/dev/null | sed 's/.*"\(.*\)"/\1/'); do
    [ "$found" = "event" ] && continue
    allowed "$found" && continue
    wrong="$wrong$file: tag = \"$found\"
"
  done
done

if [ -n "$wrong" ]; then
  echo "gate-wire: a tagged type is neither an \"event\" nor a listed data discriminator:" >&2
  printf '%s' "$wrong" | sed 's/^/  /' >&2
  echo "gate-wire: an event crossing a process boundary is tagged \"event\"; see the wire module" >&2
  echo "gate-wire: a shape nested inside a message goes on this gate's list, with its reason" >&2
  echo "gate-wire: failed" >&2
  exit 1
fi

# A reply says which revision it is written in. Without it a version skew arrives as a missing
# field at the point of use, which reads as the peer being broken rather than as a peer from a
# different build.
if grep -rql 'pub struct Reply' "$ROOT" 2>/dev/null; then
  if ! grep -rql 'pub family: u16' "$ROOT" 2>/dev/null; then
    echo "gate-wire: a Reply carries no \`family\`: a skew would arrive as a missing field" >&2
    echo "gate-wire: failed" >&2
    exit 1
  fi
fi

broken=""
broke() { broken="$broken  $1
"; }

# The text of function `$2` in file `$1`, from its signature to its closing brace.
body_of() {
  awk -v name="$2" '
    !inside && $0 ~ ("fn " name "[<(]") { inside = 1 }
    inside {
      print
      opened += gsub(/\{/, "{"); closed += gsub(/\}/, "}")
      if (opened > 0 && opened == closed) exit
    }
  ' "$1"
}

# File `$1` without its comments and its `#[cfg(test)]` items: the code that ships.
shipped() {
  awk '
    /^[ \t]*#\[cfg\(test\)\]/ { skipping = 1; depth = 0; started = 0 }
    skipping {
      opened = gsub(/\{/, "{"); depth += opened
      depth -= gsub(/\}/, "}")
      if (opened > 0) started = 1
      if (!started && $0 ~ /;[ \t]*$/) skipping = 0
      else if (started && depth <= 0) skipping = 0
      next
    }
    { sub(/\/\/.*$/, ""); print }
  ' "$1"
}

# Every file a parent module includes only under `#[cfg(test)]`, one per line.
test_only() {
  for file in $(find "$ROOT" -name '*.rs' -not -path '*/target/*' | sort); do
    base=${file%.rs}
    case "$file" in */lib.rs|*/main.rs|*/mod.rs) base=$(dirname "$file") ;; esac
    awk '
      /^[ \t]*#\[cfg\(test\)\][ \t]*$/ { held = 1; next }
      held && /^[ \t]*(pub(\([a-z]+\))? )?mod [a-z_0-9]+;/ { sub(/.*mod /, ""); sub(/;.*/, ""); print }
      { held = 0 }
    ' "$file" | sed "s|^|$base/|; s|\$|.rs|"
  done
}
tests_only=$(test_only)

# Every file that ships: not a test module, not under `tests/`.
shipping() {
  find "$ROOT" -name '*.rs' -not -name 'tests.rs' -not -path '*/tests/*' -not -path '*/target/*' |
    sort | while read -r file; do
      printf '%s\n' "$tests_only" | grep -qxF "$file" || echo "$file"
    done
}

# The value of `const $2: usize = …;` in file `$1`, worked out.
constant() {
  said=$(sed -n "s/^[a-z() ]*const $2: usize = \([0-9_ *]*\);\$/\1/p" "$1" 2>/dev/null |
    head -n 1 | tr -d '_')
  [ -n "$said" ] && echo $(( $said ))
}

# ---- one frame cap, FAMILY.md's, on every read and every write -------------------------------
cap=$(sed -n 's/.*A frame is at most [0-9]* MiB (\([0-9 ]*\) bytes).*/\1/p' FAMILY.md | head -n 1 | tr -d ' ')
frame="$ROOT/balthasar-ipc/src/frame.rs"
serve="$ROOT/balthasar-ipc/src/serve.rs"
call="$ROOT/balthasar-lua/src/stream/call.rs"
stream="$ROOT/balthasar-lua/src/stream.rs"
client="$ROOT/balthasar-lua/lua/balthasar.lua"
[ -n "$cap" ] || broke "FAMILY.md names no frame cap"
for held in "$frame:MAX_FRAME" "$call:CAP" "$stream:MAX_RECV"; do
  file=${held%%:*} name=${held#*:}
  value=$(constant "$file" "$name")
  [ "${value:-}" = "$cap" ] || broke "$file caps a frame at ${value:-nothing} ($name), and FAMILY.md at $cap"
done
body_of "$frame" send | grep -q 'n > MAX_FRAME' || broke "send() writes a frame over MAX_FRAME"
body_of "$frame" recv_within | grep -q 'n > MAX_FRAME' || broke "recv_within() reads a frame over MAX_FRAME"
body_of "$serve" say | grep -q '> frame::MAX_FRAME' || broke "say() writes a reply over the cap"
body_of "$serve" converse_within | grep -q 'body.is_empty()' || broke "a frame of length zero is answered as a call"
body_of "$call" framed | grep -q '<= CAP' || broke "call frames a request over CAP"
body_of "$call" exchange | grep -q 'length == 0 || length > CAP' || broke "call reads a reply over CAP or of length zero"
limit=$(sed -n 's/^local MAX_FRAME = \([0-9]*\)$/\1/p' "$client")
[ "$limit" = "$cap" ] || broke "$client caps a frame at ${limit:-nothing}, and FAMILY.md at $cap"
grep -q '#encoded > MAX_FRAME' "$client" || broke "$client sends a call without the cap"
grep -q 'length == 0 or length > MAX_FRAME' "$client" || broke "$client reads a reply without the cap"
for file in $(shipping); do
  case "$file" in
    "$frame"|"$call") continue ;;
    # a digest of the schema folded into four bytes, not a frame's length
    "$ROOT/balthasar-store/src/transcript.rs") continue ;;
  esac
  shipped "$file" | grep -q 'from_be_bytes' && broke "$file reads a frame length outside the framers"
done

# ---- a runtime directory is made by the private-directory rule -------------------------------
runtime="$ROOT/balthasar-model/src/runtime.rs"
trusted=$(body_of "$runtime" safe_runtime)
for held in 'is_absolute()' '== uid' '0o077'; do
  printf '%s' "$trusted" | grep -qF "$held" || broke "safe_runtime() trusts XDG_RUNTIME_DIR without $held"
done
private=$(body_of "$runtime" private_dir)
for held in '.mode(0o700)' 'symlink_metadata' 'checked_dir('; do
  printf '%s' "$private" | grep -qF "$held" || broke "private_dir() makes a directory without $held"
done
checked=$(body_of "$runtime" checked_dir)
for held in 'is_symlink()' '!= uid' '0o077'; do
  printf '%s' "$checked" | grep -qF "$held" || broke "checked_dir() passes a directory without $held"
done
for fn in bind_in tool_descriptor; do
  body_of "$serve" "$fn" | grep -qF 'private_dir(' || broke "$fn() makes its directory outside private_dir()"
done
for file in $(shipping); do
  code=$(shipped "$file")
  if printf '%s' "$code" | grep -q '"XDG_RUNTIME_DIR"' && [ "$file" != "$runtime" ]; then
    broke "$file reads XDG_RUNTIME_DIR itself, not through runtime_dir()"
  fi
  printf '%s' "$code" | grep -qE 'create_dir|DirBuilder' || continue
  case "$file" in
    # the rule itself
    "$runtime") ;;
    # a test's scratch directory, in the system temporary directory
    "$ROOT/balthasar-model/src/scratch.rs") ;;
    # the stores' own directories, made 0700 by the store's rule
    "$ROOT/balthasar-store/src/private.rs"|"$ROOT/balthasar-store/src/paths.rs") ;;
    # where `eval`, `configs`, the acknowledgement manifest and a trained model are written
    "$ROOT/balthasar-cli/src/eval.rs"|"$ROOT/balthasar-cli/src/configs.rs") ;;
    "$ROOT/balthasar-lua/src/acknowledged.rs"|"$ROOT/balthasar-recall/src/model.rs") ;;
    *) broke "$file makes a directory, and is not on this gate's list" ;;
  esac
done

if [ -n "$broken" ]; then
  echo "gate-wire: the frame cap or the runtime directory rule is not what FAMILY.md says:" >&2
  printf '%s' "$broken" >&2
  echo "gate-wire: failed" >&2
  exit 1
fi
echo "gate-wire: ok"
