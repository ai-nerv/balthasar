# Held-out F2 and F6

Written by a person after the freeze in `FROZEN`, never by the code's author and never tuned
against. `oslo make test-held-out` scores them.

`polarity.tsv`, one case a line, tab-separated:

    keep	never	never commit straight to main
    not	we never got round to it

`keep` is a directive and names the word its claim must keep; `not` instructs nothing. Cover
never, always, don't, can't, won't and no longer; narrative uses of the same words; quoted
examples; code fences; pasted conversations; "write N examples" requests.

`decision.tsv`:

    decision	let's go with zstd for the transport
    not	the migrations folder is generated

Cover change and choice sentences, noun uses of change verbs, bare-value replies and quantity
changes.

The owner waived writing this set on 2026-10-29, so with no files the runner says so and passes.

Passing: F2 precision on `not` at least 0.95 with no `keep` losing its word; F6 precision on
`not` at least 0.9. Recall is printed, whatever it is.
