-- Reading magi's journals.
--
-- This file is the whole of balthasar's independence commitment. balthasar defines the transcript shape;
-- a harness converts to it, here, in Lua. No Rust file in balthasar names a harness, and a new one
-- is a file like this rather than a release.
--
-- magi's journals are JSONL: a `meta` record first, then one `entry` record per settled turn.

balthasar.source("magi", {
  -- Where the journals are. A glob, because magi keeps them flat and names them by time.
  sessions = function()
    return balthasar.fs.glob(balthasar.data_home .. "/magi/sessions/*.jsonl")
  end,

  -- The first line names the session: its id, where it ran, and when it started.
  -- Answering nil means "this is not one of ours", which is how a glob that catches
  -- something else stays harmless.
  meta = function(first)
    local m = balthasar.json.decode(first)
    if not m or m.record ~= "meta" then return nil end
    return { id = m.session, cwd = m.cwd or "", opened = m.started or 0 }
  end,

  -- One journal line to zero or more turns.
  --
  -- `Entry` is internally tagged -- {"type":"user",...} -- so the discriminant is a field and
  -- not a wrapper. Getting this wrong reads every line as unrecognised and ingests an empty
  -- store without erroring, which is the worst way to be wrong.
  line = function(raw)
    local r = balthasar.json.decode(raw)
    if not r or r.record ~= "entry" then return nil end
    local e = r.entry
    if not e then return nil end

    if e.type == "user" then
      -- `aside` is context the model sees and nobody is shown. It is not the person speaking,
      -- so it stays out of the text the imperative rules read.
      return { cursor = r.cursor, role = "user", kind = "user", text = e.text or "" }
    end

    -- Another session speaking. Carried so it can be quoted and searched, and marked so the
    -- rules that mint a 1.0 imperative never read it as this person asking for something.
    if e.type == "from" then
      return {
        cursor = r.cursor, role = "other", kind = "from",
        text = e.text or "",
      }
    end

    -- A branch point: a count of what it keeps, not something anybody said. Kept so the
    -- cursors either side of it stay where magi put them.
    if e.type == "branch" then
      return { cursor = r.cursor, role = "other", kind = "branch", text = "" }
    end

    -- magi's own summary standing in for what it replaced.
    if e.type == "compaction" then
      return {
        cursor = r.cursor, role = "other", kind = "summary",
        text = e.summary or "",
      }
    end

    if e.type == "assistant" then
      -- An errored turn is not something the model said. Remembering it would teach the
      -- next session to produce more of them.
      if e.stop_reason == "error" then return nil end
      local u = e.usage
      return {
        cursor = r.cursor, role = "assistant", kind = "prose", text = e.text or "",
        -- No `total` field: magi bills four counters separately.
        tokens = u and ((u.input or 0) + (u.cache_read or 0) + (u.cache_write or 0) + (u.output or 0)) or nil,
      }
    end

    if e.type == "tool" then
      -- `result` is absent while a call is in flight, and stays absent for one the daemon
      -- died during. Neither is an observation worth keeping.
      if not e.result then return nil end
      local args = e.args and balthasar.json.decode(e.args) or nil
      return {
        cursor = r.cursor, role = "tool", kind = "tool_result",
        tool = e.name, args = args,
        text = e.result.output or "",
        -- The cost signal rides in on the ordinary path rather than needing its own.
        ok = not e.result.is_error,
        -- magi does not journal a duration yet. Saying nothing is right; inventing one
        -- would put a fact in a store that never deletes.
        ms = e.result.duration_ms,
      }
    end
  end,
})
