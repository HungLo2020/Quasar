import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from benchmark_runs import run_cases


class BenchmarkRunsTest(unittest.TestCase):
    def test_collects_repeated_real_process_outputs_and_refuses_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / 'fake.py'
            script.write_text('import json,sys\njson.dump({"window_samples":1,"cpu_frame":{"p50":12},"submit":{"p50":5},"draws":{"p50":2}},open(sys.argv[1],"w"))\n')
            spec = {'cases': [{'name': 'example', 'command': [sys.executable, str(script), '{output}']}]}
            with contextlib.redirect_stdout(io.StringIO()):
                report = run_cases(spec, root/'runs', 2, 5)
            self.assertEqual(len(report['cases']['example']['runs']), 2)
            self.assertEqual(json.loads((root/'runs/comparison.json').read_text())['repeats'], 2)
            with self.assertRaises(FileExistsError):
                run_cases(spec, root/'runs', 2, 5)

    def test_failed_command_and_invalid_specs_do_not_emit_a_success_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(subprocess.CalledProcessError):
                run_cases({'cases': [{'name': 'failure', 'command': [sys.executable, '-c', 'raise SystemExit(3)', '{output}']}]}, root, 1, 5)
            self.assertFalse((root/'comparison.json').exists())
            with self.assertRaises(ValueError):
                run_cases({'cases': [{'name': '../escape', 'command': ['app', '{output}']}]}, root, 1, 5)
            with self.assertRaises(ValueError):
                run_cases({'cases': []}, root, 0, 5)


if __name__ == '__main__':
    unittest.main()
