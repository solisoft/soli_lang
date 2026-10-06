# Control flow: if/elsif/else, unless, postfix modifiers, while, for, break,
# next, and conditions that start with a parenthesis.
#
# `soli fmt` strips `then` and braces and folds a one-statement block `if`
# into a postfix one. Blocks whose form is the point of a test carry a comment
# or a second statement, which fmt keeps; the `then` and brace tests cannot be
# protected that way, so do not run fmt over this file without restoring them.
#
# Nested blocks are what many of these tests exercise; inside describe/context/
# test they pass the linter's depth limit by design.
# soli-lint-disable smell/deep-nesting

def letter_for(score)
  if score >= 90
    "A"
  elsif score >= 80
    "B"
  elsif score >= 70
    "C"
  else
    "F"
  end
end

def first_even(numbers)
  for number in numbers
    return number if number % 2 == 0
  end
  nil
end

def compute_log
  log = []
  log.push("work")
  if true
    # a block if right after a statement, then a return value
    log.push("done")
  end
  log
end

class BoundaryWorker
  def process(log)
    log.push("processing")
    if log.length > 0
      # a block if right after a call, inside a method
      log.push("has items")
    end
    log
  end
end

describe("if") do
  context("block form") do
    test("runs the body when the condition is true") do
      result = 0
      if true
        # block form on purpose
        result = 1
      end
      assert_eq(result, 1)
    end

    test("skips the body when the condition is false") do
      result = 0
      if false
        # block form on purpose
        result = 1
      end
      assert_eq(result, 0)
    end

    test("runs else when the condition is false") do
      result = 0
      if false
        result = 1
      else
        result = 2
      end
      assert_eq(result, 2)
    end

    test("takes the first true branch of an elsif chain, as its value") do
      assert_eq(letter_for(95), "A")
      assert_eq(letter_for(85), "B")
      assert_eq(letter_for(75), "C")
      assert_eq(letter_for(10), "F")
    end

    test("an elsif chain with no true branch and no else runs nothing") do
      result = "unchanged"
      if false
        result = "if"
      elsif false
        result = "elsif"
      end
      assert_eq(result, "unchanged")
    end

    test("nests") do
      outer = true
      inner = true
      result = 0
      if outer
        if inner
          # nested block on purpose
          result = 1
        end
      end
      assert_eq(result, 1)
    end

    test("takes a compound condition") do
      x = 5
      y = 10
      result = ""
      if x > 0 && y > 0
        # block form on purpose
        result = "both positive"
      end
      assert_eq(result, "both positive")
    end

    test("tests truthiness: 0 is falsy, 0.0 is truthy") do
      ran = []
      if 0
        ran.push("zero")
      end
      if 0.0
        ran.push("zero float")
      end
      assert_eq(ran, ["zero float"])
    end
  end

  context("then keyword") do
    test("then after the condition runs the body") do
      result = 0
      if true then
        result = 1
      end
      assert_eq(result, 1)
    end

    test("then skips the body when false") do
      result = 0
      if false then
        result = 1
      end
      assert_eq(result, 0)
    end

    test("then with else") do
      result = 0
      if false then
        result = 1
      else
        result = 2
      end
      assert_eq(result, 2)
    end

    test("then on elsif") do
      x = 2
      result = ""
      if x == 1 then
        result = "one"
      elsif x == 2 then
        result = "two"
      else
        result = "other"
      end
      assert_eq(result, "two")
    end

    test("then with a parenthesized condition") do
      result = 0
      if (true) then
        result = 1
      end
      assert_eq(result, 1)
    end

    test("then puts the body on the condition's line") do
      result = 0
      if true then result = 5 end
      assert_eq(result, 5)
    end
  end

  context("parentheses and braces") do
    test("a parenthesized condition") do
      result = 0
      if (1 + 1 == 2)
        # block form on purpose
        result = 1
      end
      assert_eq(result, 1)
    end

    test("braces instead of end, with elsif and else") do
      result = 0
      if (false) {
        result = 1
      } elsif (true) {
        result = 2
      } else {
        result = 3
      }
      assert_eq(result, 2)
    end
  end
end

describe("unless") do
  test("runs the body when the condition is false") do
    result = 0
    unless false
      # block form on purpose
      result = 1
    end
    assert_eq(result, 1)
  end

  test("skips the body when the condition is true") do
    result = 0
    unless true
      # block form on purpose
      result = 1
    end
    assert_eq(result, 0)
  end

  test("takes then") do
    result = 0
    unless false then
      result = 1
    end
    assert_eq(result, 1)
  end

  test("runs else when the condition is true") do
    result = 0
    unless true
      result = 1
    else
      result = 2
    end
    assert_eq(result, 2)
  end

  test("skips else when the condition is false") do
    result = 0
    unless false
      result = 1
    else
      result = 2
    end
    assert_eq(result, 1)
  end

  test("negates the whole condition: unless a || b") do
    result = ""
    unless false || true
      result = "neither"
    else
      result = "some"
    end
    assert_eq(result, "some")
  end

  test("guards a multi-line body") do
    status = "banned"
    flagged = []
    unless ["up", "late", "overdue"].includes?(status)
      flagged.push(status)
      flagged.push("checked")
    end
    assert_eq(flagged, ["banned", "checked"])
  end

  test("takes a parenthesized condition") do
    n = 5
    result = false
    unless (n + 1) == 99
      # block form on purpose
      result = true
    end
    assert(result)
  end
end

describe("Postfix modifiers") do
  test("if runs the statement when true") do
    result = 0
    result = 42 if true
    assert_eq(result, 42)
  end

  test("if skips the statement when false") do
    result = 0
    result = 42 if false
    assert_eq(result, 0)
  end

  test("unless runs the statement when false") do
    result = 0
    result = 42 unless false
    assert_eq(result, 42)
  end

  test("unless skips the statement when true") do
    result = 0
    result = 42 unless true
    assert_eq(result, 0)
  end

  test("the condition can be parenthesized") do
    result = 0
    result = 42 if (1 + 1 == 2)
    assert_eq(result, 42)
    result = 7 unless (true)
    assert_eq(result, 42)
  end

  test("modify a call") do
    log = []
    log.push("yes") if true
    log.push("no") if false
    assert_eq(log, ["yes"])
  end

  test("modify an index assignment and read its own target") do
    data = {"name": "test", "value": 42}
    data["value"] = data["value"] + 1 if data["value"] > 0
    assert_eq(data["value"], 43)
  end

  test("compound conditions choose which fields to update") do
    user = {"name": "Alice", "email": "alice@test.com"}
    params = {"name": "Alice B", "email": ""}
    updates = {}
    updates["name"] = params["name"] if params["name"].present?
    updates["email"] = params["email"] if params["email"].present?
    updates.keys.each do |key|
      user[key] = updates[key]
    end
    assert_eq(user, {"name": "Alice B", "email": "alice@test.com"})
  end

  test("return if exits a function from inside a loop") do
    assert_eq(first_even([1, 3, 4, 6]), 4)
    assert_null(first_even([1, 3]))
  end
end

# A statement followed by a block `if` on the NEXT line must not be read as a
# postfix `if` of that statement. Each block keeps a comment so `soli fmt`
# leaves it a block.
describe("A statement followed by a block on the next line") do
  test("a call then if/end") do
    log = []
    log.push("a")
    if true
      # separate block
      log.push("b")
    end
    assert_eq(log, ["a", "b"])
  end

  test("a call then if/else/end") do
    log = []
    log.push("before")
    if false
      log.push("then")
    else
      log.push("else")
    end
    assert_eq(log, ["before", "else"])
  end

  test("a call then if/elsif/else/end") do
    x = 2
    log = []
    log.push("start")
    if x == 1
      log.push("one")
    elsif x == 2
      log.push("two")
    else
      log.push("other")
    end
    assert_eq(log, ["start", "two"])
  end

  test("a call then nested if blocks") do
    log = []
    log.push("outer")
    if true
      if true
        # separate block
        log.push("inner")
      end
    end
    assert_eq(log, ["outer", "inner"])
  end

  test("inside a function, before its return value") do
    assert_eq(compute_log(), ["work", "done"])
  end

  test("inside a method") do
    assert_eq(new BoundaryWorker().process([]), ["processing", "has items"])
  end

  test("a call with a hash argument then if/end") do
    items = []
    items.push({"key": "a"})
    if true
      # separate block
      items.push({"key": "b"})
    end
    assert_eq(items, [{"key": "a"}, {"key": "b"}])
  end

  test("a call with a hash argument then if/else/end") do
    items = []
    items.push({"status": "created"})
    if false
      items.push({"status": "running"})
    else
      items.push({"status": "error"})
    end
    assert_eq(items, [{"status": "created"}, {"status": "error"}])
  end

  test("several calls then if/end") do
    log = []
    log.push("one")
    log.push("two")
    log.push("three")
    if true
      # separate block
      log.push("four")
    end
    assert_eq(log, ["one", "two", "three", "four"])
  end

  test("a call then a for loop") do
    log = []
    log.push("before")
    for i in range(0, 3)
      log.push(str(i))
    end
    assert_eq(log, ["before", "0", "1", "2"])
  end

  test("a call then a while loop") do
    log = []
    log.push("start")
    i = 0
    while i < 3
      log.push(str(i))
      i = i + 1
    end
    assert_eq(log, ["start", "0", "1", "2"])
  end

  test("a call then if/end inside a for loop") do
    log = []
    for i in range(0, 3)
      log.push("iter")
      if i == 1
        # separate block
        log.push("match")
      end
    end
    assert_eq(log, ["iter", "iter", "match", "iter"])
  end

  test("a call then if/end inside if/end") do
    log = []
    if true
      log.push("outer")
      if true
        # separate block
        log.push("inner")
      end
    end
    assert_eq(log, ["outer", "inner"])
  end

  test("alternating calls and if blocks") do
    log = []
    log.push("a")
    if true
      # separate block
      log.push("b")
    end
    log.push("c")
    if true
      # separate block
      log.push("d")
    end
    assert_eq(log, ["a", "b", "c", "d"])
  end

  test("an if/else after statements, with a loop in the taken branch") do
    port = 3001
    items = [{"app_id": "a1"}]
    if port > 3000
      ["s1", "s2"].each do |server|
        items.push({"server": server, "status": "ready"})
      end
    else
      items.push({"status": "error"})
    end
    assert_eq(items.length, 3)
    assert_eq(items[2], {"server": "s2", "status": "ready"})
  end

  test("an assignment then if/end") do
    x = 0
    x = 10
    if x > 5
      # separate block
      x = x + 1
    end
    assert_eq(x, 11)
  end

  test("a call result stored then if/end") do
    name = "hello"
    size = name.length
    if size > 3
      # separate block
      name = name + " world"
    end
    assert_eq(name, "hello world")
  end
end

describe("Branches with loops inside") do
  test("if/elsif with nested loops and ifs") do
    desired = 3
    current = 1
    scaled = []
    if desired > current
      for i in range(0, desired - current)
        name = "worker-" + str(current + i + 1)
        scaled.push(name) if name.length > 0
      end
    elsif desired < current
      for i in range(0, current - desired)
        scaled.push("removed-" + str(i))
      end
    end
    assert_eq(scaled, ["worker-2", "worker-3"])
  end

  test("a loop with a postfix if finds a match") do
    users = [{"id": "1", "email": "user@test.com"}, {"id": "2", "email": "other@test.com"}]
    found = nil
    for user in users
      found = user if user["email"] == "user@test.com"
    end
    assert_eq(found["id"], "1")
  end

  test("a loop with a postfix if filters") do
    actions = ["restart:my-app"]
    workers = [
      {"ip": "10.0.0.2", "status": "ready"},
      {"ip": "10.0.0.3", "status": "ready"},
      {"ip": "10.0.0.4", "status": "error"}
    ]
    for worker in workers
      actions.push("restart-worker:" + worker["ip"]) if worker["status"] == "ready"
    end
    assert_eq(actions, ["restart:my-app", "restart-worker:10.0.0.2", "restart-worker:10.0.0.3"])
  end

  test("a loop in a nested if mutates hashes held in an array") do
    server = {"status": "provisioning"}
    app = {"status": "creating"}
    vm_ready = true
    exit_code = 0
    if vm_ready
      if exit_code == 0
        app["status"] = "running"
        [server].each do |each_server|
          each_server["status"] = "ready"
        end
      else
        app["status"] = "error"
      end
    end
    assert_eq(app["status"], "running")
    assert_eq(server["status"], "ready")
  end
end

describe("while") do
  test("repeats while the condition holds") do
    count = 0
    while count < 5
      count = count + 1
    end
    assert_eq(count, 5)
  end

  test("never runs on a false condition") do
    executed = false
    while false
      executed = true
    end
    assert_not(executed)
  end

  test("stops on the first part of a compound condition to fail") do
    i = 0
    sum = 0
    while i < 10 && sum < 20
      sum = sum + i
      i = i + 1
    end
    # sum runs 0, 1, 3, 6, 10, 15, 21: the sum condition ends it first
    assert_eq(sum, 21)
    assert_eq(i, 7)
  end

  test("a parenthesized condition") do
    count = 0
    while (count < 3)
      count = count + 1
    end
    assert_eq(count, 3)
  end

  test("while true with break runs at least once, like do-while") do
    count = 0
    while true
      count = count + 1
      break if count >= 3
    end
    assert_eq(count, 3)
  end
end

describe("for") do
  test("iterates over an array") do
    sum = 0
    for x in [1, 2, 3]
      sum = sum + x
    end
    assert_eq(sum, 6)
  end

  test("iterates over range()") do
    sum = 0
    for i in range(1, 5)
      sum = sum + i
    end
    assert_eq(sum, 10)
  end

  test("iterates over range() with a step") do
    seen = []
    for i in range(0, 10, 3)
      seen.push(i)
    end
    assert_eq(seen, [0, 3, 6, 9])
  end

  test("never runs over an empty array") do
    count = 0
    for x in []
      count = count + 1
    end
    assert_eq(count, 0)
  end

  test("gives the body the loop variable") do
    result = []
    for i in range(0, 3)
      result.push(i * 2)
    end
    assert_eq(result, [0, 2, 4])
  end

  test("nests") do
    pairs = []
    for i in range(0, 2)
      for j in range(0, 2)
        pairs.push([i, j])
      end
    end
    assert_eq(pairs, [[0, 0], [0, 1], [1, 0], [1, 1]])
  end

  test("iterates over a range literal") do
    sum = 0
    for i in 0..5
      sum = sum + i
    end
    assert_eq(sum, 10)
  end

  test("an empty or inverted range literal does not iterate") do
    count = 0
    for i in 5..5
      count = count + 1
    end
    for i in 5..2
      count = count + 1
    end
    assert_eq(count, 0)
  end

  test("a second variable takes the index, on a range") do
    pairs = []
    for value, i in 10..13
      pairs.push(str(i) + ":" + str(value))
    end
    assert_eq(pairs.join(","), "0:10,1:11,2:12")
  end

  test("a second variable takes the index, on an array") do
    labels = []
    for fruit, i in ["apple", "banana"]
      labels.push("#{i}: #{fruit}")
    end
    assert_eq(labels, ["0: apple", "1: banana"])
  end

  test("iterates the array live when the body appends") do
    # Iteration is live (bounds-checked indexing): items appended by the body
    # are visited. Pinned so both engines keep agreeing on this.
    numbers = [1, 2, 3]
    for x in numbers
      numbers.push(x + 10) if x < 3
    end
    assert_eq(numbers, [1, 2, 3, 11, 12])
  end

  test("sees a shortened array when the body removes items") do
    numbers = [1, 2, 3, 4]
    visited = []
    for x in numbers
      visited.push(x)
      numbers.pop
    end
    # visit 1, pop 4; visit 2, pop 3; length 2, stop
    assert_eq(visited, [1, 2])
  end

  test("refuses a hash") do
    assert_raises("cannot iterate over hash") do
      for key in {"a": 1}
        key
      end
    end
  end
end

describe("break") do
  test("exits a while loop") do
    i = 0
    while true
      i = i + 1
      if i >= 3
        # block form on purpose
        break
      end
    end
    assert_eq(i, 3)
  end

  test("exits a for loop over an array") do
    seen = []
    for n in [1, 2, 3, 4, 5]
      if n > 3
        # block form on purpose
        break
      end
      seen.push(n)
    end
    assert_eq(seen, [1, 2, 3])
  end

  test("exits a for loop over a range") do
    total = 0
    for x in 1..100
      total = total + x
      break if total > 10
    end
    assert_eq(total, 15)
  end

  test("takes a postfix if") do
    count = 0
    for n in [1, 2, 3, 4, 5]
      break if n > 3
      count = count + 1
    end
    assert_eq(count, 3)
  end

  test("takes a postfix unless") do
    count = 0
    for n in [1, 2, 3, 4]
      break unless n < 3
      count = count + 1
    end
    assert_eq(count, 2)
  end

  test("exits only the innermost loop") do
    pairs = []
    for outer in [1, 2]
      for inner in [10, 20, 30]
        break if inner == 20
        pairs.push([outer, inner])
      end
    end
    assert_eq(pairs, [[1, 10], [2, 10]])
  end

  test("inside try runs finally, then exits the loop") do
    log = []
    for v in [1, 2, 3]
      try
        log.push(v)
        break if v == 2
      finally
        log.push("f")
      end
    end
    assert_eq(log, [1, "f", 2, "f"])
  end

  test("inside a lambda is absorbed at the function boundary") do
    # It stops the lambda body but must not break the enclosing for loop.
    seen = []
    for n in [1, 2]
      [10, 20, 30].each(fn(x) {
        break
        seen.push(x)
      })
      seen.push(n)
    end
    assert_eq(seen, [1, 2])
  end

  test("inside nested ifs still exits the loop") do
    seen = []
    for n in [1, 2, 3, 4]
      if n > 0
        break if n == 3
        seen.push(n)
      end
    end
    assert_eq(seen, [1, 2])
  end
end

describe("next") do
  test("skips the rest of a for iteration") do
    odd = []
    for n in [1, 2, 3, 4, 5]
      next if n % 2 == 0
      odd.push(n)
    end
    assert_eq(odd, [1, 3, 5])
  end

  test("skips the rest of a while iteration") do
    seen = []
    i = 0
    while i < 5
      i += 1
      next if i == 2
      seen.push(i)
    end
    assert_eq(seen, [1, 3, 4, 5])
  end

  test("skips only the innermost loop's iteration") do
    pairs = []
    for outer in [1, 2]
      for inner in [1, 2, 3]
        next if inner == 2
        pairs.push([outer, inner])
      end
    end
    assert_eq(pairs, [[1, 1], [1, 3], [2, 1], [2, 3]])
  end

  test("skips the rest of an each block") do
    seen = []
    [1, 2, 3].each do |x|
      next if x == 2
      seen.push(x)
    end
    assert_eq(seen, [1, 3])
  end

  test("inside try runs finally before the next iteration") do
    log = []
    for n in [1, 2, 3]
      try
        next if n == 2
        log.push("t#{n}")
      finally
        log.push("f#{n}")
      end
    end
    assert_eq(log, ["t1", "f1", "f2", "t3", "f3"])
  end

  test("combines with break") do
    total = 0
    for n in range(1, 11)
      next if n % 2 == 0
      break if n > 7
      total = total + n
    end
    assert_eq(total, 16)
  end
end

# A condition that begins with a parenthesis used to end at the matching `)`,
# so `if (a ?? "") == ""` reported `Unexpected token '=='`.
describe("Conditions starting with a parenthesis") do
  test("a block if continues past the closing paren") do
    row = {"url": ""}
    fired = false
    # The guard is written the way it was reported; the point is how it parses.
    # soli-lint-disable-next-line idiom/prefer-blank, idiom/prefer-to-s
    if (row["url"] ?? "") == ""
      # block form on purpose
      fired = true
    end
    assert(fired)
  end

  test("postfix if and unless continue past it too") do
    n = 1
    by_if = false
    by_unless = false
    by_if = true if (n + 1) == 2
    by_unless = true unless (n + 1) == 3
    assert(by_if)
    assert(by_unless)
  end

  test("while and elsif conditions too") do
    i = 0
    while (i + 1) <= 2
      i = i + 1
    end
    assert_eq(i, 2)
    label = ""
    if (i) == 9
      label = "no"
    elsif (i + 1) == 3
      label = "elsif"
    end
    assert_eq(label, "elsif")
  end

  test("the fully parenthesized form still works") do
    n = 1
    seen = false
    seen = true if (n == 1)
    assert(seen)
    partly = false
    partly = true if (n) == 1
    assert(partly)
  end
end
