# System: running commands (argv vs shell, sync vs future) and System.argv.

ARGV_SCRIPT = "/tmp/soli_system_spec_argv.sl"

describe("System.run_sync") do
  test("returns stdout, stderr and the exit code") do
    result = System.run_sync("echo hello")
    assert_eq(result, {"stdout": "hello\n", "stderr": "", "exit_code": 0})
  end

  test("reports stderr and a non-zero exit code without raising") do
    result = System.run_sync(["sh", "-c", "echo oops 1>&2; exit 3"])
    assert_eq(result["stdout"], "")
    assert_eq(result["stderr"], "oops\n")
    assert_eq(result["exit_code"], 3)
  end

  test("accepts an argv array") do
    result = System.run_sync(["echo", "from", "array"])
    assert_eq(result["exit_code"], 0)
    assert_eq(result["stdout"], "from array\n")
  end

  test("passes argv arguments verbatim, with no shell") do
    # `*` is a literal argument, not a glob: a shell would expand it.
    result = System.run_sync(["echo", "*"])
    assert_eq(result["stdout"].trim, "*")
  end

  test("refuses shell metacharacters in the string form") do
    message = assert_raises("refuses to auto-shell") do
      System.run_sync("echo hi > /tmp/should-not-exist.txt")
    end
    assert_contains(message, "System.shell()")
    assert_not(File.exists("/tmp/should-not-exist.txt"))
  end

  test("raises when the program does not exist") do
    assert_raises("Failed to execute") do
      System.run_sync(["definitely-not-a-program-xyz"])
    end
  end

  test("rejects an empty argv array") do
    assert_raises("received an empty array") do
      System.run_sync([])
    end
  end

  test("rejects a command that is neither a string nor an array") do
    assert_raises("expects a string or array of strings, got int") do
      System.run_sync(42)
    end
  end
end

describe("System.run") do
  test("returns a Future that resolves on first use") do
    future = System.run("echo hello")
    assert_eq(type(future), "Future")
    assert_eq(future.stdout, "hello\n")
    assert_eq(future.exit_code, 0)
  end

  test("await resolves the Future to the result hash") do
    result = await(System.run(["echo", "awaited"]))
    assert_eq(result, {"stdout": "awaited\n", "stderr": "", "exit_code": 0})
  end
end

describe("System.shell") do
  test("shell_sync runs through sh -c, so pipes work") do
    result = System.shell_sync("echo one | tr a-z A-Z")
    assert_eq(result["exit_code"], 0)
    assert_eq(result["stdout"], "ONE\n")
  end

  test("shell returns a Future") do
    future = System.shell("echo shellfut")
    assert_eq(type(future), "Future")
    assert_eq(future.stdout, "shellfut\n")
  end
end

describe("System.argv") do
  after_each() do
    File.delete(ARGV_SCRIPT) if File.exists(ARGV_SCRIPT)
  end

  test("is an empty array under soli test") do
    assert_eq(System.argv, [])
  end

  test("holds a script's own arguments as strings") do
    soli_path = System.run_sync(["sh", "-c", "command -v soli"])["stdout"].trim
    skip("no soli binary on PATH") if soli_path.blank?

    barf(ARGV_SCRIPT, "print(System.argv)\n")
    result = System.run_sync([soli_path, ARGV_SCRIPT, "32", "fast"])
    assert_eq(result["stdout"].trim, "[32, fast]")
  end

  test("hands runner options after -- to the script") do
    soli_path = System.run_sync(["sh", "-c", "command -v soli"])["stdout"].trim
    skip("no soli binary on PATH") if soli_path.blank?

    barf(ARGV_SCRIPT, "print(System.argv.length)\nprint(System.argv[0])\n")
    result = System.run_sync([soli_path, ARGV_SCRIPT, "--", "--tree"])
    assert_eq(result["stdout"], "1\n--tree\n")
  end
end
