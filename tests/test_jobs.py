from __future__ import annotations

import pytest

from loomex_runner.jobs import RunnerJobError, read_many, run_job, write_many


def test_file_write_many_stays_inside_workspace(tmp_path):
    result = write_many(
        workspace=tmp_path,
        payload={"files": [{"path": "src/app.py", "content": "print('ok')\n"}]},
    )

    assert result["writtenFiles"] == [{"path": "src/app.py", "sizeBytes": 12}]
    assert (tmp_path / "src" / "app.py").read_text(encoding="utf-8") == "print('ok')\n"


def test_file_write_many_rejects_traversal(tmp_path):
    with pytest.raises(RunnerJobError):
        write_many(workspace=tmp_path, payload={"files": [{"path": "../escape.txt", "content": "bad"}]})


def test_file_read_many(tmp_path):
    (tmp_path / "README.md").write_text("hello\n", encoding="utf-8")

    result = read_many(workspace=tmp_path, payload={"files": ["README.md"]})

    assert result == {"files": [{"path": "README.md", "content": "hello\n", "truncated": False}]}


def test_run_job_rejects_unknown_kind(tmp_path):
    with pytest.raises(RunnerJobError):
        run_job(workspace=tmp_path, job={"kind": "other", "payload": {}})

