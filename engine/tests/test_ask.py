from engine.ask import ask
from engine.providers import Router
from engine.tests.conftest import FakeEmbedder


def test_ask_drops_tampered_quotes(env):
    r = ask("what did I write about the airport", env["cfg"], Router(env["cfg"]), FakeEmbedder())
    quotes = [c["quote"] for c in r["citations"] if c["quote"]]
    assert quotes == ["how bad it could get"]
    why = " ".join(x["why"] for x in r["removed"])
    assert "not found verbatim" in why and "not verbatim in any source" in why
    assert "forever" not in r["answer"]
    assert r["found"]
