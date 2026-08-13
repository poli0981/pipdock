# The sample the ruff fixtures were captured from

`../ruff/*.json` are documents ruff wrote. This is the input it wrote them about.

It is committed because reconstructing it is the expensive part of a tool bump, and getting it
subtly wrong looks exactly like the tool having changed. Two reconstructions during the 0.16.2 bump
differed from the committed fixtures — an `except:` body of `pass` instead of `return None` pulled
in an extra `S110`, and a one-line `def parse(` produced *"unexpected EOF while parsing"* where the
real sample produces two errors. Both read as regressions until the previous ruff was installed
alongside and run on the same input.

| File | Exists to produce |
|---|---|
| `pkg/app.py` | `findings.json` — `I001` plus two `F401`, all safe-fixable, one file. Also the vulture fixture: all eight of its message shapes, including the `unreachable code after 'return'` one that names no identifier. `requests` is *used*, which is why it is not a third `F401`. |
| `nofix.py` | `nofix.json` — one `E722` and **nothing else**. The handler returns rather than passing, or `S110` joins it and the "a finding with no fix" case stops being a single finding. |
| `broken.py` | `syntax-error.json` — two `invalid-syntax` findings. Line 1 is exactly ten characters ending in a token that cannot start a parameter, and line 2 must have content: the second finding is *"Expected `)`, found newline"*, and a file that simply ends gives *"unexpected EOF"* instead. |
| `clean.py` | `clean.json` — `[]`, two bytes, no trailing newline. |

## Re-capturing

```
python capture.py <path to ruff.exe> ../ruff
```

Then **install the previous ruff too and run it against this same sample.** A diff between versions
on one interpreter cannot tell "ruff changed" from "the sample changed"; a diff between the old
version's output and the committed fixture can.

Line endings are load-bearing and `.gitattributes` marks this whole tree `-text` for that reason:
`findings.json` carries a fix whose replacement text is LF-joined, so a CRLF checkout of `app.py`
would produce a document no tool on a real machine emits.
