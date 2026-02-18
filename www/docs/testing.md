# Testing MVC Applications

Soli provides a comprehensive testing framework for MVC applications with BDD-style DSL, parallel execution, and coverage reporting.

## Test Structure

Tests live in the `tests/` directory of your application:

```
myapp/
├── app/
│   ├── controllers/
│   ├── models/
│   └── views/
└── tests/
    ├── users_spec.sl
    ├── posts_spec.sl
    └── integration/
        └── api_spec.sl
```

## Test DSL

### Basic Test Structure

```soli
describe("UsersController", fn()
    test("creates a new user", fn()
        # Test code here
        expect(true).to_be(true)
    end)
    
    context("when valid", fn()
        test("returns success", fn()
            # Nested context
        end)
    end)
end)
```

### Available Functions

| Function | Purpose |
|----------|---------|
| `describe(name, fn)` | Group related tests |
| `context(name, fn)` | Group tests with conditions |
| `test(name, fn)` | Define a test case |
| `it(name, fn)` | Alias for test |
| `specify(name, fn)` | Alias for test |
| `before_each(fn)` | Setup before each test |
| `after_each(fn)` | Teardown after each test |
| `before_all(fn)` | Setup before all tests |
| `after_all(fn)` | Teardown after all tests |
| `pending()` | Skip a test |

### Expectations

```soli
expect(value).to_equal(expected);
expect(value).to_be(expected);
expect(value).to_not_equal(other);
expect(value).to_be_null();
expect(value).to_not_be_null();
expect(value).to_be_greater_than(10);
expect(value).to_be_less_than(100);
expect(value).to_contain("substring");
expect(value).to_match(regex);
expect(hash).to_have_key("name");
expect(json_string).to_be_valid_json();
```

## HTTP Integration Testing

### Making Requests

```soli
describe("Users API", fn()
    test("GET /users returns list", fn()
        let response = TestHTTP.get("/users")
        expect(response.status).to_equal(200)
        expect(response.body).to_contain("users")
    end)
    
    test("POST /users creates user", fn()
        let response = TestHTTP.post("/users", hash(
            "email": "test@example.com",
            "name": "Test User"
        ))
        expect(response.status).to_equal(201)
    end)
    
    test("PUT /users/:id updates user", fn()
        let response = TestHTTP.put("/users/1", hash("name": "Updated"))
        expect(response.status).to_equal(200)
    end)
    
    test("DELETE /users/:id removes user", fn()
        let response = TestHTTP.delete("/users/1")
        expect(response.status).to_equal(204)
    end)
end)
```

### Request Options

```soli
TestHTTP.get("/users");
TestHTTP.get("/users", query: hash("page": "2"));
TestHTTP.post("/users", payload);
TestHTTP.post("/users", payload, headers: hash("Content-Type": "application/json"));
TestHTTP.put("/users/1", payload);
TestHTTP.patch("/users/1", payload);
TestHTTP.delete("/users/1");
```

## Controller Testing

### Direct Action Calls

```soli
describe("UsersController", fn()
    before_each(fn()
        Factory.clear()
    end)
    
    test("create action", fn()
        let result = ControllerTest.helpers.users_controller.create(
            params: hash("email": "test@example.com"),
            session: Session.new(),
            headers: Headers.new()
        )
        expect(result.status).to_equal(201)
    end)
    
    test("show action", fn()
        let user = Factory.create("user")
        let result = ControllerTest.helpers.users_controller.show(
            params: hash("id": user.id),
            session: Session.new(),
            headers: Headers.new()
        )
        expect(result.status).to_equal(200)
    end)
end)
```

## Database Testing

### Transaction Rollback

Tests are isolated using database transactions:

```soli
describe("User model", fn()
    test("creates user", fn()
        with_transaction(fn()
            let user = Factory.create("user", hash("name": "Test"))
            expect(User.count()).to_equal(1)
            expect(user.name).to_equal("Test")
        end)
        # Transaction automatically rolls back
    end)
end)
```

### Factory Pattern

```soli
# Define factories
Factory.define("user", hash(
    "email": "user@example.com",
    "name": "Test User"
))

Factory.define("post", hash(
    "title": "Test Post",
    "content": "Content here"
))

# Use factories
let user = Factory.create("user")
let post = Factory.create("post", hash("title": "Custom Title"))
let users = Factory.create_list("user", 5)
```

## Parallel Execution

Tests run in parallel by default:

```bash
soli test                    # Parallel (default)
soli test --jobs=4           # 4 workers
soli test --jobs=1           # Sequential (debug)
```

## Coverage Reporting

```bash
soli test --coverage                 # Generate coverage
soli test --coverage=html            # HTML report
soli test --coverage=json            # JSON for CI
soli test --coverage=xml             # Cobertura XML
soli test --coverage-min=80          # Fail if < 80%
```

### Coverage Output

```
Coverage: 87.5% (1250/1428 lines) ✓

src/controllers/users.sl     ▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░  94.2%
src/models/user.sl           ▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░  91.1%
src/controllers/posts.sl     ▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░  78.5%
```

### Coverage Configuration

```soli
coverage_threshold(80)           # Fail if < 80%
coverage_exclude("**/migrations/**")
```

## Complete Example

```soli
describe("UsersController", fn()
    before_each(fn()
        Factory.clear()
        Database.clean_all()
    end)
    
    context("POST /users", fn()
        test("creates user with valid data", fn()
            let response = TestHTTP.post("/users", hash(
                "email": "user@example.com",
                "name": "Test User"
            ))
            expect(response.status).to_equal(201)
            expect(response.body).to_contain("Test User")
        end)
        
        test("returns 422 with invalid email", fn()
            let response = TestHTTP.post("/users", hash(
                "email": "invalid-email"
            ))
            expect(response.status).to_equal(422)
        end)
        
        test("returns 422 without email", fn()
            let response = TestHTTP.post("/users", hash(
                "name": "Test"
            ))
            expect(response.status).to_equal(422)
        end)
    end)
    
    context("GET /users/:id", fn()
        test("shows user profile", fn()
            let user = Factory.create("user")
            let response = TestHTTP.get("/users/" + user.id)
            expect(response.status).to_equal(200)
            expect(response.body).to_contain(user.name)
        end)
        
        test("returns 404 for unknown user", fn()
            let response = TestHTTP.get("/users/99999")
            expect(response.status).to_equal(404)
        end)
    end)
    
    context("DELETE /users/:id", fn()
        test("removes user", fn()
            let user = Factory.create("user")
            let response = TestHTTP.delete("/users/" + user.id)
            expect(response.status).to_equal(204)
            expect(User.find(user.id)).to_be_null()
        end)
    end)
end)
```

## Running Tests

```bash
# Run all tests
soli test

# Run specific file
soli test tests/users_spec.sl

# Run with coverage
soli test --coverage

# Sequential execution
soli test --jobs=1

# JSON output for CI
soli test --reporter=json
```

## Test Results

```
Tests: 45 passed, 2 failed
Coverage: 87.5% (1250/1428 lines) ✓

Failed tests:
  - "returns 422 with invalid email" (users_spec.sl:42)
  - "shows deleted user" (users_spec.sl:89)
```
