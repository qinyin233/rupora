"""Exercise the release guard with the same exit-code handling as Actions pwsh."""

import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/native-release.yml"


def run_guard(status, response):
    workflow = WORKFLOW.read_text(encoding="utf-8")
    step = workflow.split(
        "      - name: Refuse to mutate an existing published release\n", 1
    )[1].split("      - name:", 1)[0]
    body = textwrap.dedent(step.split("        run: |\n", 1)[1])
    script = """$ErrorActionPreference = 'Stop'
function gh {
    $global:LASTEXITCODE = [int]$env:RELEASE_GUARD_STATUS
    Write-Output $env:RELEASE_GUARD_RESPONSE
}
""" + body + """
# GitHub Actions appends this to its PowerShell scripts.
if (Test-Path -LiteralPath variable:\\LASTEXITCODE) { exit $LASTEXITCODE }
"""
    with tempfile.TemporaryDirectory(prefix="rupora-release-guard-") as directory:
        path = Path(directory) / "guard.ps1"
        path.write_text(script, encoding="utf-8")
        return subprocess.run(
            ["pwsh", "-NoProfile", "-NonInteractive", "-File", str(path)],
            env={
                **os.environ,
                "RELEASE_TAG": "v2.0.0-alpha.1",
                "GITHUB_REPOSITORY": "qinyin233/rupora",
                "RELEASE_GUARD_STATUS": str(status),
                "RELEASE_GUARD_RESPONSE": response,
            },
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=60,
        )


class ReleaseGuardTests(unittest.TestCase):
    def test_new_release_can_be_published(self):
        result = run_guard(1, 'gh: Not Found (HTTP 404)')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_existing_draft_can_be_completed(self):
        result = run_guard(0, '{"draft": true}')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_published_release_cannot_be_replaced(self):
        result = run_guard(0, '{"draft": false}')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("already published", result.stderr)

    def test_api_failure_still_blocks_publication(self):
        result = run_guard(1, 'gh: Internal Server Error (HTTP 500)')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Could not check existing release", result.stderr)


if __name__ == "__main__":
    unittest.main()
