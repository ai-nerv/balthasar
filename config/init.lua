-- memo's configuration, and its only entry point.
--
-- A program, not a data file: it may probe the machine, loop, and branch. Settings are
-- assigned, behaviour is registered, and the file returns nothing.
--
-- Two namespaces, and which one a handler is in tells you its contract:
--
--   memo.on.<question>   ASKED.  nil = not mine, carry on. A table = do this instead.
--                                First non-nil wins.
--   memo.did.<verb>      TOLD.   Pure side effect. What it returns is ignored.
--
-- Nothing else here takes a function, so there is no third contract to remember.

-- Source adapters, so `memo ingest` has something to read. A harness is a file in here, not
-- a release: memo defines the transcript shape and each of these converts to it.
memo.load("sources/axon.lua")
memo.load("sections.lua")

-- ------------------------------------------------------------------ what is kept

-- Score a candidate must reach to leave the session it was learned in. Below `hold` it dies
-- with the session; between the two it waits in scratch for a second witness.
--
-- memo.promote_floor = 0.5
-- memo.hold_floor    = 0.3

-- Confidence a memory must have to be ASSERTED -- presented to a model as current truth.
-- Below it a memory is still found by an explicit search; it stops being stated as fact.
-- That gap is the whole answer to staleness, so the two are separate numbers on purpose.
--
-- memo.inject_floor = 0.35
-- memo.live_floor   = 0.10

-- Ebbinghaus, per importance class. Nested data stays nested: setting one leaves the rest.
--
-- memo.decay.normal  = 0.05    -- ~14-day half-life
-- memo.decay.high    = 0.01    -- ~69
-- memo.decay.inertia = true    -- what is needed often resists fading

-- What each witness is worth. Distillation is deliberately below `promote_floor`: a thing
-- that merely scrolled out of the context window is a candidate, not a fact.
--
-- memo.witness.distillation = 0.3
-- memo.witness.correction   = 0.8

-- Words that mark a user's turn as an instruction to remember.
--
-- memo.imperatives = { "remember", "always", "never", "from now on", "note that" }

-- ------------------------------------------------------------------ which memory

-- Which store a directory belongs to. Nothing registered means the repository root, so five
-- worktrees of one project share one memory rather than each starting the others' amnesia.

-- Everything under ~/scratch is not worth a project store of its own.
--
-- memo.on.scope(function(cwd)
--   if cwd:find("^" .. memo.home .. "/scratch/") then return { id = "global" } end
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

memo.mask["shell"] = function(item)
  return ("`shell` output elided (~%d tokens) -- run it again if you need it"):format(item.tokens)
end

memo.mask["read"] = function(item)
  return ("read a file (~%d tokens) -- it is still on disk, read it again if you need it"):format(item.tokens)
end

-- ------------------------------------------------------------------ gates

-- A credential must never become durable. The gate is here rather than at the injection
-- boundary because a store that never held it cannot leak it, and this store never deletes.
memo.on.promote(function(candidate)
  if memo.looks_like_secret(candidate.text or "") then
    return { promote = false, reason = "looks like a credential" }
  end
end)

-- A command learned by failing outranks a sentence about one.
memo.on.promote(function(candidate)
  if candidate.tier == "habit" and candidate.witness == "cost" then
    return { promote = true, importance = "high" }
  end
end)

-- ------------------------------------------------------------------ models, both optional

-- Three ways to reach a model, tried in order. With none of them reachable, extraction is
-- extractive and consolidation is clustering. Worse, and never absent: an agent whose memory
-- hard-fails without an API key is worse off than one with no memory at all.
--
-- memo.distiller = {
--   { kind = "peer",     tool = "axon" },
--   { kind = "command",  argv = { "llm", "-m", "claude-sonnet-4-5" } },
--   { kind = "endpoint", api = "openai-completions",
--     base_url = "https://api.anthropic.com/v1",
--     model = "claude-sonnet-4-5",
--     auth = { kind = "env", var = "ANTHROPIC_API_KEY" } },
-- }

-- Local, offline, downloaded once. Never on the critical path: recall works lexically until
-- the vector lands.
--
-- memo.embedder = { kind = "onnx", model = "bge-small-en-v1.5", dimensions = 384 }

-- ------------------------------------------------------------------ trust

-- Directories whose own `.memo.lua` may DECLARE as well as choose. A project file may
-- otherwise set a floor or add a section, but not name a source, an extractor, a tool, a
-- command to run or an endpoint to send text to.
--
-- memo.trusted = { "/home/you/work" }

-- ------------------------------------------------- the use-and-outcome ledger

-- Whether memo records what it retrieved, what was injected, what a caller then did, and how
-- it went. OFF BY DEFAULT, and deliberately so: this costs writes on the recall path, and a
-- memory layer that silently started recording what you search for because a new version
-- shipped is not one anybody should install.
--
-- Turning it on is what makes `memo trace`, `memo utility` and `memo outcomes` answer. Nothing
-- in it can touch confidence — truth and utility are separate judgments, and there is a test
-- holding the store to that.
--
-- Queries are stored as digests and actions as digests. The ledger records that something
-- happened and where in the transcript to look, never a copy of what was said.
--
-- memo.outcome = {
--   capture = true,
--   retention_days = 90,
-- }

-- Classify an outcome yourself. Called with what memo observed; return one of
-- `succeeded`, `failed`, `corrected`, `reverted`, `abstained`, `ignored`, or nil to leave it
-- unknown. Returning nil is a real answer: an action nobody evaluated must never drift into
-- being a failed one.
--
-- No handler is needed. The deterministic default is what a caller reports, and this exists for
-- the harness that knows more than it can say through the protocol.
--
-- memo.on.outcome(function(event)
--   if event.tool == "shell" and event.exit_code == 0 then
--     return "succeeded"
--   end
--   return nil
-- end)

-- Told, not asked: memo mentioning that a caller acted on something it was given. Side-effect
-- only — whatever this returns is ignored.
--
-- memo.did.used(function(action)
--   -- log it, count it, ignore it
-- end)
