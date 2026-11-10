"""Reference data files hold raw 32-bit words (float bits, generator states); a word that reads 82xxxxxx looks like an
address of the original to `mcla-tool registry check` (any bare 8-hex token in the code range counts as a citation).
`cite_safe` writes such a word with an `h` in front (h82481427); the Rust tests' hex readers drop it."""
import re

_WORD = re.compile(r'(?<![0-9A-Za-z])(82[0-9A-Fa-f]{6})(?![0-9A-Za-z])')


def cite_safe(text: str) -> str:
    return _WORD.sub(r'h\1', text)
