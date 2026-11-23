from apcore import APCore

# 1. Initialize the simplified client
client = APCore()


# 2. Define a module using the client's decorator (auto-registers to client.registry)
@client.module(id="math.add", description="Add two integers")
def add(a: int, b: int) -> int:
    """Adds two numbers."""
    return a + b


# 3. Call the module directly through the client
if __name__ == "__main__":
    # Sync call
    result = client.call("math.add", {"a": 10, "b": 5})
    print(f"Sync result: {result}")  # {'result': 15}

    # Example of a more complex return
    @client.module(id="greet")
    def greet(name: str, greeting: str = "Hello") -> dict:
        return {"message": f"{greeting}, {name}!"}

    result = client.call("greet", {"name": "Alice"})
    print(f"Greet result: {result}")  # {'message': 'Hello, Alice!'}
