import unittest
from check_version import compare, parse_version


class VersionTests(unittest.TestCase):
    def test_precedence(self):
        versions = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta",
                    "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11",
                    "1.0.0-rc.1", "1.0.0", "1.0.1", "1.1.0", "2.0.0"]
        for left, right in zip(versions, versions[1:]):
            self.assertLess(compare(left, right), 0)
            self.assertGreater(compare(right, left), 0)
        self.assertEqual(compare("1.0.0+build.1", "1.0.0+build.2"), 0)

    def test_invalid_versions(self):
        for value in ["1", "1.2", "v1.0.0", "01.0.0", "1.0.0-01", "1.0.0-"]:
            with self.assertRaises(ValueError):
                parse_version(value)


if __name__ == "__main__":
    unittest.main()
