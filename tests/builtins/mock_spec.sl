describe("Mock") do
  test("stubs values and lambdas, records calls") do
    repo = new Mock("repo", {"count": 3, "find": fn(id) { {"id": id} }})
    assert_eq(repo.count(), 3)
    assert_eq(repo.find(7)["id"], 7)
    repo.assert_received("find", [7])
    repo.assert_not_received("delete")
    assert_eq(repo.call_count("count"), 1)
  end

  test("unexpected message throws") do
    repo = new Mock("repo")
    failed = false
    try
      repo.nope()
    catch error
      failed = true
    end
    assert(failed)
  end

  test("assert_received fails when never called") do
    repo = new Mock("repo", {"a": 1})
    failed = false
    try
      repo.assert_received("a")
    catch error
      failed = true
    end
    assert(failed)
  end
end

class CatchAll
  def method_missing(name, args)
    "#{name}:#{args.length}"
  end
end

describe("instance method_missing arity") do
  test("a trailing args parameter receives every argument") do
    catcher = new CatchAll()
    assert_eq(catcher.a(), "a:0")
    assert_eq(catcher.b(1, 2, 3), "b:3")
  end
end
