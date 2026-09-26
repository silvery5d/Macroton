import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parents[1] / "scripts" / "summarize-macos-sample.py"
SPEC = importlib.util.spec_from_file_location("sample_summary", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class SampleSummaryTests(unittest.TestCase):
    def test_counts_top_level_branches_once_and_preserves_overlap(self):
        fixture = """Analysis of sampling wine (pid 1) every 1 millisecond
Process:         Game.exe [1]
Code Type:       X86-64 (translated)
Call graph:
    10 Thread_1: Game Thread
    + 7 ???  (in wine)
    + ! 7 ???  (in Rosetta JIT)
    + 2 ???  (in ntdll.so)
    + ! 2 semaphore_wait
    + 1 vkQueueSubmit  (in libMoltenVK.dylib)
    10 Thread_2: dxvk-submit
    + 10 ???  (in ntdll.so)
"""
        result = MODULE.summarize(fixture, "Game Thread")
        self.assertEqual(result["process"], "Game.exe")
        self.assertEqual(result["total_samples"], 10)
        self.assertEqual(result["direct_branch_samples"], 10)
        self.assertEqual(result["marker_branch_samples"]["rosetta_jit"], 7)
        self.assertEqual(result["marker_branch_samples"]["ntdll"], 2)
        self.assertEqual(result["marker_branch_samples"]["semaphore_wait"], 2)
        self.assertEqual(result["marker_branch_samples"]["moltenvk"], 1)

    def test_missing_thread_fails_with_available_names(self):
        with self.assertRaisesRegex(ValueError, "available named threads: Game Thread"):
            MODULE.summarize("    1 Thread_1: Game Thread\n", "Missing")


if __name__ == "__main__":
    unittest.main()
