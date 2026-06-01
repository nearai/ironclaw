from math_utils import multiply

def test_multiply_positive():
    assert multiply(3, 4) == 12, f"Expected 12, got {multiply(3, 4)}"

def test_multiply_zero():
    assert multiply(0, 99) == 0, f"Expected 0, got {multiply(0, 99)}"

def test_multiply_negative():
    assert multiply(-2, 5) == -10, f"Expected -10, got {multiply(-2, 5)}"

if __name__ == "__main__":
    passed = 0
    failed = 0
    for name, fn in [
        ("test_multiply_positive", test_multiply_positive),
        ("test_multiply_zero",     test_multiply_zero),
        ("test_multiply_negative", test_multiply_negative),
    ]:
        try:
            fn()
            print(f"  PASS  {name}")
            passed += 1
        except AssertionError as e:
            print(f"  FAIL  {name}: {e}")
            failed += 1
    print(f"\n{passed} passed, {failed} failed")
    raise SystemExit(failed)
