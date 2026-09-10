-- What a harness adds to use balthasar.
--
-- Drop this into a harness's own configuration and load it. It wraps the client stub in the
-- three things a harness actually needs, and every one of them degrades: if nothing is
-- listening and nothing can be spawned, each returns nil and the harness carries on exactly as
-- it did before. A memory layer that can brick the agent is worse than no memory layer.
--
--   local memory = require("balthasar.harness")     -- or load this file
--   memory.observe(session, turn)              -- stream, fire and forget
--   local plan = memory.plan(session, window)  -- what to send
--   local text = memory.context(turn, budget)  -- what to inject
--
-- The client stub arrives as source. A harness gets it three ways, and a sandboxed one can
-- only use the first two: from a sibling that already carries it, from `balthasar lua-api`, or from
-- the `client` verb on an open connection.

local M = { _NAME = "balthasar.harness", _VERSION = 1 }

-- The connection, held. balthasar is asked several times per turn -- once to observe, once to plan,
-- once to recall -- which is not how a socket that gets polled occasionally is used.
local held = nil
local tried = false
local opened = nil

--- Load the stub, given whatever transport this host lends.
local function stub(source, transport)
  local chunk, why = load(source, "balthasar.lua")
  if not chunk then return nil, why end
  return chunk(transport)
end

--- The held connection, dialled again if a call gave up on one.
---
--- The client closes its handle on any read it could not finish, because the reply it abandoned
--- is still on the wire and would answer the next call. Holding the dead one instead would cost
--- the whole session's memory over a single slow first call -- and the first `observe` of a new
--- session waits for balthasar to open its store.
---
--- Once. A balthasar that is not there is not there, which is what `tried` already says.
local function reach()
  if not held or held.handle then return held end
  held = nil
  if not opened then return nil end
  local client = stub(opened.source, opened.transport)
  held = client and client.connect(opened.where) or nil
  return held
end

--- Connect once, and remember that we failed so every turn does not pay for a retry.
---
--- `opts.source` is the stub. `opts.transport` is the socket primitive. `opts.retry` asks for
--- one more attempt, for a harness that knows balthasar has just been started.
function M.open(opts)
  opts = opts or {}
  local live = reach()
  if live then return live end
  if tried and not opts.retry then return nil, "balthasar was not reachable earlier this session" end
  tried = true

  if not opts.source then return nil, "balthasar's client library was not installed" end
  local client, why = stub(opts.source, opts.transport)
  if not client then return nil, why end

  local mem, refused = client.connect(opts.where)
  if not mem then return nil, refused end
  opened, held = opts, mem
  return held
end

--- Whether balthasar is answering.
function M.live()
  return reach() ~= nil
end

--- Stream one turn as it settles.
---
--- Fire and forget, and deliberately so: a socket in the turn loop's path is a socket that can
--- make a turn wait. What is lost when balthasar is down is one turn's observation, and the harness
--- still has its own transcript to backfill from later.
function M.observe(session, turn)
  local mem = reach()
  if not mem then return false end
  local ok = pcall(function() return mem.observe(session, turn) end)
  return ok
end

--- Ask what to send.
---
--- `nil` means balthasar had nothing to say, and a harness that gets it should do whatever it did
--- before balthasar existed. That is the whole degradation story: not an error to handle, an absence
--- to carry on through.
function M.plan(session, window)
  local mem = reach()
  if not mem then return nil end
  local ok, answer = pcall(function() return mem.plan(session, window) end)
  if not ok or not answer then return nil end
  return answer
end

--- Ask what is worth injecting for this turn.
function M.recall(query, opts)
  local mem = reach()
  if not mem then return nil end
  local ok, answer = pcall(function() return mem.recall(query, opts) end)
  if not ok then return nil end
  return answer
end

--- Propose something worth keeping.
---
--- Capped at the far end by who the kernel says is calling: a harness proposes at a peer's
--- weight, cannot pin, and cannot reach the global store. It contributes; it does not decide.
function M.remember(text, opts)
  local mem = reach()
  if not mem then return nil end
  local ok, answer = pcall(function() return mem.remember(text, opts) end)
  if not ok then return nil end
  return answer
end

--- Let go.
function M.close()
  if held then held:close() end
  held = nil
end

return M
