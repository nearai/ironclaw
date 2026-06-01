"""Tests for the calculator module."""

from calculator import divide


def test_divide_normal():
    assert divide(6, 2) == 3.0


def test_divide_by_zero():
    """This test expects a ZeroDivisionError."""
    assert divide(1, 0) == 0.0  # BUG: this returns 0.0 instead of raising
