-- A source for something that is not a harness.
--
-- Copy this into `~/.config/balthasar/plugin/` and balthasar reads it on the next run -- no edit
-- to anything shipped, no rebuild. This is balthasar's independence commitment made usable: no
-- Rust file here names a harness, and a new source is a file like this rather than a release.
--
-- What a source owes:
--
--   sessions()      where the records are, as a list of paths.
--   meta(first)     the first line as `{ id, cwd, opened }`, or nil for "not one of ours".
--   line(raw)       one record as zero or more turns, or nil to skip it.
--
-- Answering nil rather than raising is the whole discipline here: a glob that catches somebody
-- else's file must stay harmless, because the alternative is a source that refuses to ingest
-- anything the day a neighbouring program changes its filenames.
--
-- This one reads zsh's extended history -- `: <when>:<elapsed>;<command>` -- and remembers what
-- was actually run in a directory. It is deliberately not a harness: the point is that a source
-- is a shape, not a category, and anything that can be read as "a person did this, then this"
-- fits it.

balthasar.source("zsh-history", {
  sessions = function()
    -- One file, not a glob: zsh keeps a single history. The list is still a list, because the
    -- shape is what balthasar reads and a source with one record should not be a special case.
    local home = os.getenv("HOME") or ""
    return balthasar.fs.glob(home .. "/.zsh_history")
  end,

  meta = function(first)
    -- There is no header. The first line is history like every other line, so this claims the
    -- file only when it looks like zsh's extended format -- and answers nil otherwise, which is
    -- what keeps a plain-text history from being read as this.
    if not first:match("^: %d+:%d*;") then return nil end
    return { id = "zsh-history", cwd = "", opened = tonumber(first:match("^: (%d+):")) or 0 }
  end,

  line = function(raw)
    local when, command = raw:match("^: (%d+):%d*;(.+)$")
    if not command then return nil end

    -- A trivial command teaches nothing and costs a row in a store that never deletes. `cd`,
    -- `ls` and `git status` are what a person types between the things they meant to do.
    if command:match("^%s*$") or #command < 8 then return nil end
    if command:match("^%s*cd%s") or command:match("^%s*ls%f[%s\0]") then return nil end

    return {
      cursor = tonumber(when) or 0,
      role = "user",
      kind = "tool_result",
      tool = "shell",
      args = { command = command },
      text = command,
      -- No `ok`: a shell history does not record whether anything worked. Saying nothing is
      -- right; inventing a `true` would put a fact in a store that never deletes.
    }
  end,
})
