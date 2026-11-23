import apcore


# 1. No need to initialize anything if using the default global client
@apcore.module(id="math.add")
def add(a: int, b: int) -> int:
    return a + b


if __name__ == "__main__":
    # 2. Call directly via apcore.call
    result = apcore.call("math.add", {"a": 10, "b": 5})
    print(f"Global call result: {result}")  # {'result': 15}
