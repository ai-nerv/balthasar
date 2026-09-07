-- A section of your own: what a harness is told, and where in the context it goes.
--
-- Copy this into `~/.config/balthasar/plugin/`. Sections are keyed, so this adds one rather than
-- replacing the shipped set -- and naming a shipped key would mean it, which is what
-- `after/plugin/` is for.
--
-- What a section owes:
--
--   weight          its share of the budget, in proportion to the other sections. Unspent budget
--                   passes to the next, so a thin section does not waste the room it was allotted.
--   order           where it appears. Lower is earlier, and earlier is more expensive context.
--   tiers           which kinds of memory it draws from -- `fact`, `habit`, and so on.
--   where           the filter. Every field is an `and`.
--   render          the format string each memory is drawn with.
--   min_confidence  the floor. A wrong build command is worse than no build command at all.
--
-- This one pulls out the handful of commands that actually work in this project, and puts them
-- near the top -- because "what is the test command here" is the question a coding agent
-- rediscovers most often and most expensively.

balthasar.section("commands-that-work", {
  weight = 2,

  -- After `identity` (10) and before `how-this-project-works` (20). A section that fights for a
  -- position rather than choosing one is a section whose place changes when somebody else adds
  -- theirs.
  order = 15,

  tiers = { "habit", "fact" },

  where = {
    scope = "project",
    predicate = { "build", "test", "lint", "run", "format" },
  },

  render = "- `%s`: %s",

  -- High, on purpose. This section exists to be trusted without checking, and a command that
  -- fails teaches the next session to work around it rather than to fix it.
  min_confidence = 0.8,
})
