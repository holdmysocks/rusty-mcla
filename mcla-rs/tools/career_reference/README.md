# Offline reference fixtures for the CAREER_RACES verification (2026-10-27)

Each `*_fx.py` cuts unchanged functions (and the closure of their callees) out of the static recompilation (`D:\MCLA-static`), compiles them with the repo's host
shim (`../recomp_vector.py`), runs prepared inputs and writes `out/<name>_reference.csv`; an `#[ignore]`d Rust test compares the port bit for bit.
Run from this folder with `g++` (WinLibs) on PATH: `python rewards_fx.py` (and `select_fx.py`, `line_fx.py`, `money_fx.py`), then
`MCLA_REWARDS_REFERENCE=... MCLA_SELECT_REFERENCE=... MCLA_LINE_REFERENCE=... MCLA_MONEY_REFERENCE=... cargo test --release -p mcla -- --include-ignored rewards_match select_matches line_getters money_text_matches`.
Paths at the top of `fx.py` (recomp, decrypted image `work/2026-10-10/original-image.bin`). Evidence and numbers: `docs/reference/career/verify_career_evidence.md`.
