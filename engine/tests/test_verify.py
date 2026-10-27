from engine.verify import check_quotes, find_verbatim, strip_unverified_inline_quotes

SRC = "I’m 30 pancakes in and I’m still hungry. It’s just a brunch thing."


def test_exact_quote_kept():
    assert find_verbatim("It’s just a brunch thing.", SRC) == "It’s just a brunch thing."


def test_typography_only_change_is_replaced_by_exact_source_text():
    q = find_verbatim("I'm 30 pancakes in", SRC)  # straight apostrophe from the model
    assert q == "I’m 30 pancakes in"
    assert q in SRC


def test_tampered_quote_dropped():
    assert find_verbatim("I’m 40 pancakes in", SRC) is None
    assert find_verbatim("i’m 30 pancakes in", SRC) is None  # a case change is a word change
    kept, dropped = check_quotes([{"node": "u1", "quote": "I’m 30 pancakes in"},
                                  {"node": "u1", "quote": "I’m 30 pan cakes in"},
                                  {"node": "u2", "quote": "anything"}], {"u1": SRC})
    assert [k["quote"] for k in kept] == ["I’m 30 pancakes in"]
    assert len(dropped) == 2 and "unknown source" in dropped[1]["why"]


def test_inline_prose_quote_removed():
    out, flags = strip_unverified_inline_quotes(
        'He says "I’m 30 pancakes in and" and "I am fifty pancakes deep".', [SRC])
    assert flags == ["I am fifty pancakes deep"]
    assert "fifty" not in out and "30 pancakes" in out
