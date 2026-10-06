# Futures and await: System.run / System.shell return a Future that await()
# resolves to {stdout, stderr, exit_code}; property access auto-resolves it.

describe("System.run futures") do
  test("System.run returns a Future") do
    assert_eq(type(System.run("echo test")), "Future")
  end

  test("await resolves the future to the command's result hash") do
    result = await(System.run("echo hello"))
    assert_eq(type(result), "hash")
    assert_eq(result, {"stdout": "hello\n", "stderr": "", "exit_code": 0})
  end

  test("property access auto-resolves a future without await") do
    future = System.run("echo hello")
    assert_eq(future.stdout, "hello\n")
    assert_eq(future.exit_code, 0)
  end

  test("futures resolve independently, in any await order") do
    first = System.run("echo first")
    second = System.run("echo second")
    assert_eq(await(second).stdout, "second\n")
    assert_eq(await(first).stdout, "first\n")
  end

  test("a failing command resolves with its stderr and exit code") do
    result = await(System.run(["sh", "-c", "echo err >&2; exit 3"]))
    assert_eq(result, {"stdout": "", "stderr": "err\n", "exit_code": 3})
  end

  test("the argv form passes an argument with spaces verbatim") do
    assert_eq(await(System.run(["echo", "x y"])).stdout, "x y\n")
  end

  test("a string command with shell metacharacters is refused") do
    assert_raises("refuses to auto-shell") do
      System.run("echo a | wc")
    end
  end

  test("awaiting a missing program raises") do
    future = System.run(["nonexistent_command_sweep3"])
    assert_raises("Failed to execute") do
      await(future)
    end
  end
end

describe("System.shell futures") do
  test("await resolves a shell pipeline") do
    assert_eq(await(System.shell("echo a && echo b")).stdout, "a\nb\n")
  end
end

describe("await on non-futures") do
  test("returns a plain value unchanged") do
    assert_eq(await(42), 42)
    assert_null(await(nil))
  end

  test("returns an already-resolved hash unchanged") do
    resolved = await(System.run("echo x"))
    assert_eq(await(resolved), resolved)
  end
end
