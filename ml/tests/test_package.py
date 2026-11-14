import hearo_ml


def test_version_matches_pyproject() -> None:
    import tomllib
    from pathlib import Path

    pyproject = tomllib.loads((Path(__file__).parents[1] / "pyproject.toml").read_text())
    assert hearo_ml.__version__ == pyproject["project"]["version"]
