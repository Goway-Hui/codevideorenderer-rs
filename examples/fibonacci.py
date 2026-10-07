"""Example snippet rendered by `cvr`."""


def fibonacci(n):
    """Calculate the nth Fibonacci number."""
    if n <= 1:
        return n
    return fibonacci(n - 1) + fibonacci(n - 2)


# Example usage
result = fibonacci(10)
print(f"Fibonacci(10) = {result}")

values = [fibonacci(i) for i in range(10)]
print(", ".join(str(v) for v in values))
