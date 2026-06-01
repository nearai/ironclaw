"""Tests for is_palindrome — one will fail due to the bug."""

from palindrome import is_palindrome


def test_simple_palindrome():
    assert is_palindrome("racecar") is True


def test_not_palindrome():
    assert is_palindrome("hello") is False


def test_case_insensitive():
    # BUG: this fails because the function doesn't normalize case
    assert is_palindrome("Racecar") is True


def test_ignores_whitespace():
    # BUG: this fails because the function doesn't strip whitespace
    assert is_palindrome(" racecar ") is True
