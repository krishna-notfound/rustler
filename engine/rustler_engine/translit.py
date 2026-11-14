"""Devanagari -> readable Hinglish Latin.

Pipeline: every maximal run of Devanagari characters is transliterated to ITRANS with
``indic_transliteration`` and the ITRANS string is then normalised into the way Hindi is
usually typed in Latin script ("Hinglish"). Everything that is not Devanagari (Latin text,
digits, punctuation, emoji, other scripts) is passed through unchanged.

Rules (one consistent set, documented here and pinned by tests/test_translit.py):

* Output is lowercase ASCII. Danda ``।``/``॥`` become ``.``; Devanagari digits become ASCII digits.
* **Schwa deletion** (done on the phoneme sequence, right to left):
  - the final inherent "a" of a word is dropped (कमल -> kamal, बहुत -> bahut), unless it would
    leave the word without a vowel (न -> na) or it follows a conjunct ending in "y" (सत्य -> satya);
  - a medial inherent "a" is dropped in the pattern ``vowel + C + a + C + vowel``
    (समझना -> samajhna, पहले -> pahle, कमरा -> kamra); the first syllable is never touched and a
    nasal counts as a consonant (ज़िंदगी -> zindagi keeps its "a").
* **Long vowels**: ā is written ``a`` (क्या -> kya, तुम्हारा -> tumhara, भाई -> bhai) except
  word-initially and in closed one-syllable words, where it is ``aa`` (आप -> aap, बात -> baat,
  काम -> kaam). ī, ū are written ``i``, ``u`` (ठीक -> thik). ऋ -> ri.
* **Nasals** (anusvara, chandrabindu, ITRANS ``M``/``.N``/``.n``): ``n`` before a consonant
  (``m`` before p/ph/b/bh/m), ``n`` at the end of a word (मैं -> main, हैं -> hain, कहाँ -> kahan,
  लोगों -> logon, में -> mein), but dropped after long ī (नहीं -> nahi). ṅ/ñ -> n.
* **Consonants**: छ is ``chh`` and च्छ collapses to ``chh`` (अच्छा -> achha — this is the single
  rule for the accha/achha question); ख़ -> kh, ज़ -> z, फ़ -> f, ड़ -> r, ढ़ -> rh, ज्ञ -> gy,
  क्ष -> ksh, श/ष -> sh, व -> v. Other geminates are kept (पक्का -> pakka).
* A final ``ie`` becomes ``iye`` (चाहिए -> chahiye).
"""

from __future__ import annotations

import re
import unicodedata
from functools import lru_cache
from typing import Iterable, List, Tuple

from indic_transliteration import sanscript

# --- run extraction ---------------------------------------------------------------------

_DEV = "ऀ-ॣॱ-ॿ"
_RUN = re.compile(f"[{_DEV}][{_DEV}‌‍]*")
_PRE = str.maketrans(
    {
        "।": ".",
        "॥": ".",
        "॰": ".",
        **{chr(0x0966 + i): str(i) for i in range(10)},
    }
)

# --- ITRANS tokenisation ----------------------------------------------------------------

_VOWELS = {
    "a": "a",
    "A": "A",
    "aa": "A",
    "i": "i",
    "I": "I",
    "ii": "I",
    "u": "u",
    "U": "U",
    "uu": "U",
    "RRi": "ri",
    "RRI": "ri",
    "LLi": "li",
    "LLI": "li",
    "e": "e",
    "ai": "ai",
    "o": "o",
    "O": "o",
    "au": "au",
}
_NASALS = {"M", ".N", ".n"}
_CONS = {
    "kSh": "ksh", "x": "ksh", "j~n": "gy", "GY": "gy",
    "chh": "chh", "Ch": "chh", "ch": "ch",
    "kh": "kh", "gh": "gh", "jh": "jh", "Th": "th", "Dh": "dh", "th": "th", "dh": "dh",
    "ph": "ph", "bh": "bh", "sh": "sh", "Sh": "sh",
    ".Dh": "rh", ".D": "r", "~N": "n", "~n": "n", "N": "n",
    "k": "k", "K": "kh", "g": "g", "G": "g", "j": "j", "J": "z", "z": "z", "Z": "z",
    "T": "t", "D": "d", "t": "t", "d": "d", "n": "n", "p": "p", "f": "f", "F": "f",
    "b": "b", "m": "m", "y": "y", "r": "r", "R": "r", "l": "l", "L": "l", "v": "v", "w": "v",
    "s": "s", "h": "h", "H": "h", "q": "k",
}
_OTHER = {"|": ".", "||": ".", ".a": "", "OM": "om", "AUM": "om"}

_ALL = {**{k: ("V", v) for k, v in _VOWELS.items()},
        **{k: ("N", k) for k in _NASALS},
        **{k: ("C", v) for k, v in _CONS.items()},
        **{k: ("O", v) for k, v in _OTHER.items()}}
_TOKEN = re.compile("|".join(re.escape(k) for k in sorted(_ALL, key=len, reverse=True)))

_LABIAL = {"p", "ph", "b", "bh", "m"}

Tok = Tuple[str, str]  # (kind, value) with kind in C, V, N, O, X(literal)


def _tokenize(itrans: str) -> List[Tok]:
    out: List[Tok] = []
    pos = 0
    for m in _TOKEN.finditer(itrans):
        if m.start() > pos:
            out.append(("X", itrans[pos : m.start()]))
        out.append(_ALL[m.group(0)])
        pos = m.end()
    if pos < len(itrans):
        out.append(("X", itrans[pos:]))
    return out


def _delete_schwas(toks: List[Tok]) -> List[Tok]:
    nv = sum(1 for k, _ in toks if k == "V")
    if nv <= 1:
        return toks
    toks = list(toks)
    i = len(toks) - 1
    while i >= 1:
        if toks[i] == ("V", "a") and toks[i - 1][0] == "C" and nv > 1:
            if i == len(toks) - 1:
                keep = i >= 2 and toks[i - 1][1] == "y" and toks[i - 2][0] == "C"
                if not keep:
                    del toks[i]
                    nv -= 1
            elif (
                i >= 2
                and toks[i - 2][0] == "V"
                and i + 2 < len(toks)
                and toks[i + 1][0] == "C"
                and toks[i + 2][0] == "V"
            ):
                del toks[i]
                nv -= 1
        i -= 1
    return toks


def _render(toks: List[Tok]) -> str:
    n_vowels = sum(1 for k, _ in toks if k == "V")
    out: List[str] = []
    for idx, (kind, val) in enumerate(toks):
        prev = toks[idx - 1] if idx else None
        nxt = toks[idx + 1] if idx + 1 < len(toks) else None
        if kind == "V":
            if val == "A":
                closed_mono = n_vowels == 1 and nxt is not None and nxt[0] in ("C", "N")
                out.append("aa" if idx == 0 or closed_mono else "a")
            else:
                out.append(val)
        elif kind == "N":
            if nxt is None:
                if prev == ("V", "I"):
                    pass  # नहीं -> nahi
                elif prev == ("V", "e"):
                    out.append("in")  # में -> mein
                else:
                    out.append("n")
            elif nxt[0] == "C" and nxt[1] in _LABIAL:
                out.append("m")
            else:
                out.append("n")
        elif kind == "C":
            if val == "chh" and out and out[-1] == "ch":
                out[-1] = "chh"  # अच्छा -> achha
            else:
                out.append(val)
        else:
            out.append(val)
    word = "".join(out).lower()
    if word.endswith("ie"):
        word = word[:-2] + "iye"
    return word


@lru_cache(maxsize=8192)
def _convert_run(run: str) -> str:
    run = unicodedata.normalize("NFC", run).replace("‌", "").replace("‍", "")
    if not run:
        return ""
    itrans = sanscript.transliterate(run, sanscript.DEVANAGARI, sanscript.ITRANS)
    return _render(_delete_schwas(_tokenize(itrans)))


def to_latin(text: str) -> str:
    """Transliterate Devanagari in ``text`` to Hinglish Latin; other text is unchanged."""
    if not text:
        return text
    text = text.translate(_PRE) if any("ऀ" <= c <= "ॿ" for c in text) else text
    return _RUN.sub(lambda m: _convert_run(m.group(0)), text)


def to_latin_many(texts: Iterable[str]) -> List[str]:
    return [to_latin(t) for t in texts]
