import pytest

from rustler_engine.translit import to_latin, to_latin_many


@pytest.mark.parametrize(
    "src,expected",
    [
        ("क्या", "kya"),
        ("है", "hai"),
        ("नहीं", "nahi"),
        ("भाई", "bhai"),
        ("मैं", "main"),
        ("ज़िंदगी", "zindagi"),
        ("bro क्या scene है", "bro kya scene hai"),
    ],
)
def test_common_hinglish(src, expected):
    assert to_latin(src) == expected


@pytest.mark.parametrize(
    "src,expected",
    [
        # chh rule: both छ and च्छ are written "chh"
        ("अच्छा", "achha"),
        ("कुछ", "kuchh"),
        # final schwa dropped, medial schwa deleted (V C a C V), first syllable kept
        ("कमल", "kamal"),
        ("समझना", "samajhna"),
        ("मतलब", "matlab"),
        ("पहले", "pahle"),
        ("कमरा", "kamra"),
        ("लड़का", "larka"),
        ("पढ़ाई", "parhai"),
        ("बहुत", "bahut"),
        ("किताब", "kitab"),
        ("धन्यवाद", "dhanyavad"),
        ("सत्य", "satya"),
        ("आपका", "aapka"),
        # long a: aa word-initially and in closed monosyllables, a elsewhere
        ("आप", "aap"),
        ("बात", "baat"),
        ("काम", "kaam"),
        ("यार", "yaar"),
        ("तुम्हारा", "tumhara"),
        ("जाना", "jana"),
        # nasals
        ("हिंदी", "hindi"),
        ("हम", "ham"),
        ("संपर्क", "sampark"),
        ("कहाँ", "kahan"),
        ("हैं", "hain"),
        ("में", "mein"),
        ("लोगों", "logon"),
        ("क्यों", "kyon"),
        # vowels / consonants
        ("ठीक", "thik"),
        ("कृष्ण", "krishn"),
        ("ज्ञान", "gyaan"),
        ("ख़ुश", "khush"),
        ("शुक्रिया", "shukriya"),
        ("चाहिए", "chahiye"),
        ("और", "aur"),
        ("दोस्त", "dost"),
    ],
)
def test_more_words(src, expected):
    assert to_latin(src) == expected


def test_non_devanagari_passes_through():
    for s in ["hello world", "123 !?", "こんにちは", "café", "", "  spaced  out "]:
        assert to_latin(s) == s


def test_punctuation_and_mixed():
    assert to_latin("क्या है?") == "kya hai?"
    assert to_latin("नहीं। है") == "nahi. hai"
    assert to_latin("३ भाई") == "3 bhai"
    assert to_latin("it's  क्या") == "it's  kya"


def test_nukta_composed_and_decomposed_agree():
    assert to_latin("ज़िंदगी") == to_latin("ज़िंदगी") == "zindagi"
    assert to_latin("ज़") == to_latin("ज़")


def test_many_preserves_order_and_length():
    assert to_latin_many(["क्या", "x", "है"]) == ["kya", "x", "hai"]
