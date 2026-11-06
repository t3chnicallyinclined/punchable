"""search_api.lease_acquire: a write conflict is asked again, "already exists" is an immediate refusal (run 4 of the
group-6a gate saw eight first-ever askers all refused, so nobody held the lease). The real race is the Rust gate's
eight-asker test against a scratch SurrealDB; this is the retry logic over a fake connection."""
import search_api as S


class Scripted:
    """r.last() answers from a script: an exception instance is raised, anything else returned."""

    def __init__(self, *answers):
        self.answers = list(answers)
        self.calls = 0

    def last(self, sql, vars=None):
        self.calls += 1
        a = self.answers.pop(0)
        if isinstance(a, Exception):
            raise a
        return a


def test_a_conflict_is_asked_again_and_then_holds_it():
    r = Scripted(RuntimeError("Transaction write conflict: this transaction can be retried"), [{"owner": "a"}])
    assert S.lease_acquire(r, "a") is True and r.calls == 2


def test_a_conflict_that_outlasts_the_retries_is_a_refusal_for_this_pass():
    c = RuntimeError("Transaction write conflict")
    r = Scripted(c, c, c)
    assert S.lease_acquire(r, "a") is False and r.calls == 3


def test_already_exists_is_an_immediate_refusal():
    r = Scripted(RuntimeError("Database record `lease:embedder` already exists"))
    assert S.lease_acquire(r, "a") is False and r.calls == 1


def test_another_error_surfaces():
    r = Scripted(RuntimeError("the database is down"))
    try:
        S.lease_acquire(r, "a")
    except RuntimeError as e:
        assert "down" in str(e)
    else:
        raise AssertionError("must raise")


def test_someone_elses_live_lease_is_no_rows():
    assert S.lease_acquire(Scripted([]), "b") is False
