"""The worker suite's shared fixtures: the engine's `env` (a temp config + the mock provider), re-exported here so
test_hosted.py's tests take it by name without importing a fixture into the module (ruff F811)."""
from engine.tests.conftest import env

__all__ = ["env"]
