-- balthasar's configuration, and its only entry point.
--
-- A program, not a data file: it may probe the machine, loop, and branch. Settings are
-- assigned, behaviour is registered, and the file returns nothing.
--
-- Two namespaces, and which one a handler is in tells you its contract:
--
--   balthasar.on.<question>   ASKED.  nil = not mine, carry on. A table = do this instead.
--                                First non-nil wins.
--   balthasar.did.<verb>      TOLD.   Pure side effect. What it returns is ignored.
--
-- Nothing else here takes a function, so there is no third contract to remember.

-- Source adapters, so `balthasar ingest` has something to read. A harness is a file in here, not
-- a release: balthasar defines the transcript shape and each of these converts to it.
balthasar.load("sources/magi.lua")
balthasar.load("sections.lua")

-- ------------------------------------------------------------------ what is kept

-- Score a candidate must reach to leave the session it was learned in. Below `hold` it dies
-- with the session; between the two it waits in scratch for a second witness.
--
-- balthasar.promote_floor = 0.5
-- balthasar.hold_floor    = 0.3

-- Confidence a memory must have to be ASSERTED -- presented to a model as current truth.
-- Below it a memory is still found by an explicit search; it stops being stated as fact.
-- That gap is the whole answer to staleness, so the two are separate numbers on purpose.
--
-- balthasar.inject_floor = 0.35
-- balthasar.live_floor   = 0.10

-- Ebbinghaus, per importance class. Nested data stays nested: setting one leaves the rest.
--
-- balthasar.decay.normal  = 0.05    -- ~14-day half-life
-- balthasar.decay.high    = 0.01    -- ~69
-- balthasar.decay.inertia = true    -- what is needed often resists fading

-- What each witness is worth. Distillation is deliberately below `promote_floor`: a thing
-- that merely scrolled out of the context window is a candidate, not a fact.
--
-- balthasar.witness.distillation = 0.3
-- balthasar.witness.correction   = 0.8

-- Words that mark a user's turn as an instruction to remember.
--
-- balthasar.imperatives = { "remember", "always", "never", "from now on", "note that" }

-- ------------------------------------------------------------------ which memory

-- Which store a directory belongs to. Nothing registered means the repository root, so five
-- worktrees of one project share one memory rather than each starting the others' amnesia.

-- Everything under ~/scratch is not worth a project store of its own.
--
-- balthasar.on.scope(function(cwd)
--   if cwd:find("^" .. balthasar.home .. "/scratch/") then return { id = "global" } end
-- end)

-- ------------------------------------------------------------------ the window

-- What a masked tool result says instead of itself.
--
-- Masking is tried before summarising, always: it is free, it is reversible because the text
-- is still in scratch, and tool output is most of a coding session's window. Only the tool's
-- author knows what a useful stub says, so a tool with no handler here is left alone -- an
-- uninformative stub is worse than the output it replaced.
--
-- Keyed, so re-reading this file replaces rather than accumulating.

balthasar.mask["shell"] = function(item)
  return ("`shell` output elided (~%d tokens) -- run it again if you need it"):format(item.tokens)
end

balthasar.mask["read"] = function(item)
  return ("read a file (~%d tokens) -- it is still on disk, read it again if you need it"):format(item.tokens)
end

-- ------------------------------------------------------------------ gates

-- A credential must never become durable. The gate is here rather than at the injection
-- boundary because a store that never held it cannot leak it, and this store never deletes.
balthasar.on.promote(function(candidate)
  if balthasar.looks_like_secret(candidate.text or "") then
    return { promote = false, reason = "looks like a credential" }
  end
end)

-- A command learned by failing outranks a sentence about one.
balthasar.on.promote(function(candidate)
  if candidate.tier == "habit" and candidate.witness == "cost" then
    return { promote = true, importance = "high" }
  end
end)

-- ------------------------------------------------------------------ models, both optional

-- Three ways to reach a model, tried in order. With none of them reachable, extraction is
-- extractive and consolidation is clustering. Worse, and never absent: an agent whose memory
-- hard-fails without an API key is worse off than one with no memory at all.
--
-- balthasar.distiller = {
--   { kind = "peer",     tool = "magi" },
--   { kind = "command",  argv = { "llm", "-m", "claude-sonnet-4-5" } },
--   { kind = "endpoint", api = "openai-completions",
--     base_url = "https://api.anthropic.com/v1",
--     model = "claude-sonnet-4-5",
--     auth = { kind = "env", var = "ANTHROPIC_API_KEY" } },
-- }

-- Local, offline, downloaded once. Never on the critical path: recall works lexically until
-- the vector lands.
--
-- balthasar.embedder = { kind = "onnx", model = "bge-small-en-v1.5", dimensions = 384 }

-- ------------------------------------------------------------------ trust

-- Directories whose own `.balthasar.lua` may DECLARE as well as choose. A project file may
-- otherwise set a floor or add a section, but not name a source, an extractor, a tool, a
-- command to run or an endpoint to send text to.
--
-- balthasar.trusted = { "/home/you/work" }

-- ------------------------------------------------- the use-and-outcome ledger

-- Whether balthasar records what it retrieved, what was injected, what a caller then did, and how
-- it went. OFF BY DEFAULT, and deliberately so: this costs writes on the recall path, and a
-- memory layer that silently started recording what you search for because a new version
-- shipped is not one anybody should install.
--
-- Turning it on is what makes `balthasar trace`, `balthasar utility` and `balthasar outcomes` answer. Nothing
-- in it can touch confidence — truth and utility are separate judgments, and there is a test
-- holding the store to that.
--
-- Queries are stored as digests and actions as digests. The ledger records that something
-- happened and where in the transcript to look, never a copy of what was said.
--
-- balthasar.outcome = {
--   capture = true,
--   retention_days = 90,
-- }

-- Classify an outcome yourself. Called with what balthasar observed; return one of
-- `succeeded`, `failed`, `corrected`, `reverted`, `abstained`, `ignored`, or nil to leave it
-- unknown. Returning nil is a real answer: an action nobody evaluated must never drift into
-- being a failed one.
--
-- No handler is needed. The deterministic default is what a caller reports, and this exists for
-- the harness that knows more than it can say through the protocol.
--
-- balthasar.on.outcome(function(event)
--   if event.tool == "shell" and event.exit_code == 0 then
--     return "succeeded"
--   end
--   return nil
-- end)

-- Told, not asked: balthasar mentioning that a caller acted on something it was given. Side-effect
-- only — whatever this returns is ignored.
--
-- balthasar.did.used(function(action)
--   -- log it, count it, ignore it
-- end)
