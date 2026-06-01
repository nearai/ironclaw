import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(__file__)))

from calc import add


def test_add():
    assert add(2, 3) == 5
