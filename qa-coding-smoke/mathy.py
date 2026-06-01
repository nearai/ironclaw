def add(a, b):
    # Intentional bug: subtraction instead of addition
    return a - b


def factorial(n):
    if n < 0:
        raise ValueError("n must be non-negative")
    result = 1
    for i in range(2, n + 1):
        result *= i
    return result
