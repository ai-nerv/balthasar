-- What a harness is told, and in what order.
--
-- Sections are keyed, so re-running this file replaces rather than appends. Each one gets a
-- share of the budget in proportion to its weight, and unspent budget passes to the next --
-- so a thin `identity` does not waste the room it was allotted.

-- Who the person is. Small, stable, and worth the top of the context.
aeon.section("identity", {
  weight = 1, order = 10,
  tiers = { "fact" },
  where = { predicate = { "name", "pronouns", "timezone", "editor", "shell" } },
  render = "- %s: %s",
})

-- How this project is worked on. The largest share, because it is what a coding agent
-- rediscovers most expensively.
aeon.section("how-this-project-works", {
  weight = 4, order = 20,
  tiers = { "habit", "fact" },
  where = { scope = "project", importance = { "high", "critical" } },
  render = "- %s",
  -- A build command that is wrong is worse than no build command at all.
  min_confidence = 0.6,
})

-- What happened lately. Chronological: sorting episodes by salience reads as nonsense.
aeon.section("recent", {
  weight = 2, order = 30,
  tiers = { "episode" },
  limit = 5,
  preserve_order = true,
})

-- Whatever the current turn is actually about.
aeon.section("relevant", {
  weight = 3, order = 40,
  tiers = { "fact", "episode", "habit" },
  query = "turn",
})
