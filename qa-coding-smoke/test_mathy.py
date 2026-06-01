import unittest

from mathy import add, factorial


class MathyTests(unittest.TestCase):
    def test_add_basic(self):
        self.assertEqual(add(2, 3), 5)

    def test_add_zero(self):
        self.assertEqual(add(0, 7), 7)

    def test_factorial(self):
        self.assertEqual(factorial(5), 120)


if __name__ == "__main__":
    unittest.main()
