// ============================================================================
// Rest Operator and Hash Spread Examples for AI/LLM Code Generation
// ============================================================================
//
// AI AGENT GUIDE:
// ---------------
// This file demonstrates rest operator and hash spread in Soli.
//
// REST OPERATOR (in function parameters):
// fn sum(...numbers: Int[]) -> Int { }
// fn log(message: String, ...args: Any[]) -> Any { }
//
// REST OPERATOR (in array destructuring):
// let [first, ...rest] = array;
//
// HASH SPREAD:
// let merged = { ...defaults, ...overrides };
// let config = { ...base_config, ...user_config, "new_key": value };
//
// ============================================================================

// ============================================================================
// EXAMPLE 1: Rest Operator in Function Parameters
// ============================================================================

class RestOperatorController extends Controller {
    fn sum(req: Any) -> Any {
        let numbers = [1, 2, 3, 4, 5];
        let result = calculate_sum(...numbers);

        return {
            "status": 200,
            "body": json_stringify({
                "numbers": numbers,
                "sum": result
            })
        };
    }

    fn calculate_sum(...numbers: Int[]) -> Int {
        let total = 0;
        for n in numbers {
            total = total + n;
        }
        return total;
    }

    fn average(req: Any) -> Any {
        let data = [10, 20, 30, 40, 50];
        let result = calculate_average(...data);

        return {
            "status": 200,
            "body": json_stringify({
                "data": data,
                "average": result
            })
        };
    }

    fn calculate_average(...values: Int[]) -> Float {
        if (len(values) == 0) { return 0; }

        let total = 0;
        for v in values {
            total = total + v;
        }
        return total / len(values);
    }
}

// ============================================================================
// EXAMPLE 2: Variable Arguments with Logging
// ============================================================================

class LoggingController extends Controller {
    fn log_example(req: Any) -> Any {
        log("User action", "login", "user_id", 123);
        log("Order created", "order_id", "ORD-456", "amount", 99.99);

        return { "status": 200, "body": "Logged successfully" };
    }

    fn log(level: String, message: String, ...args: Any[]) -> Any {
        let timestamp = DateTime.now();
        let formatted_args = [];

        let i = 0;
        while (i < len(args)) {
            let key = args[i];
            let value = args[i + 1];
            formatted_args.push(key + ": " + str(value));
            i = i + 2;
        }

        let log_entry = {
            "timestamp": timestamp,
            "level": level,
            "message": message,
            "args": formatted_args
        };

        print(json_stringify(log_entry));
        return log_entry;
    }

    fn format_log(...parts: String[]) -> String {
        return join(parts, " | ");
    }

    fn debug(...messages: Any[]) -> Any {
        let formatted = messages.map(fn(m) {
            return str(m);
        });
    }
}

// ============================================================================
// EXAMPLE 3: String Formatting with Rest
// ============================================================================

class FormattingController extends Controller {
    fn format_example(req: Any) -> Any {
        let result = format("Hello", "World", "!");
        let formatted = sprintf("Name: %s, Age: %d, Score: %.2f", "John", 25, 95.5);

        return {
            "status": 200,
            "body": json_stringify({
                "simple": result,
                "formatted": formatted
            })
        };
    }

    fn format(...parts: Any[]) -> String {
        return parts.map(fn(p) { return str(p); }).join("");
    }

    fn sprintf(format_str: String, ...args: Any[]) -> String {
        let result = format_str;
        let arg_index = 0;

        let i = 0;
        while (i < len(result)) {
            if (substring(result, i, 1) == "%") {
                let format_char = substring(result, i + 1, 1);
                if (format_char == "s") {
                    result = replace_range(result, i, 2, str(args[arg_index]));
                    arg_index = arg_index + 1;
                } else if (format_char == "d") {
                    result = replace_range(result, i, 2, str(to_number(args[arg_index])));
                    arg_index = arg_index + 1;
                } else if (format_char == "f") {
                    result = replace_range(result, i, 2, str(args[arg_index]));
                    arg_index = arg_index + 1;
                }
            }
            i = i + 1;
        }

        return result;
    }

    fn replace_range(str: String, start: Int, length: Int, replacement: String) -> String {
        return substring(str, 0, start) + replacement + substring(str, start + length);
    }
}

// ============================================================================
// EXAMPLE 4: HTTP Request Builder with Rest
// ============================================================================

class RequestBuilderController extends Controller {
    fn build_request(req: Any) -> Any {
        let get_request = build_get_request("https://api.example.com/users", {
            "page": 1,
            "limit": 10
        });

        let post_request = build_post_request("https://api.example.com/users", {
            "name": "John",
            "email": "john@example.com"
        }, {
            "Content-Type": "application/json"
        });

        return {
            "status": 200,
            "body": json_stringify({
                "get": get_request,
                "post": post_request
            })
        };
    }

    fn build_get_request(url: String, ...headers: Hash[]) -> Any {
        let merged_headers = {};
        for h in headers {
            merged_headers = { ...merged_headers, ...h };
        }

        return {
            "method": "GET",
            "url": url,
            "headers": merged_headers
        };
    }

    fn build_post_request(url: String, body: Hash, ...headers: Hash[]) -> Any {
        let merged_headers = { "Content-Type": "application/json" };
        for h in headers {
            merged_headers = { ...merged_headers, ...h };
        }

        return {
            "method": "POST",
            "url": url,
            "headers": merged_headers,
            "body": body
        };
    }
}

// ============================================================================
// EXAMPLE 5: Array Operations with Rest
// ============================================================================

class ArrayRestController extends Controller {
    fn array_operations(req: Any) -> Any {
        let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        let [first, second, ...rest] = numbers;

        let [head, ...tail] = numbers;

        let last_n = get_last_n(numbers, 3);

        return {
            "status": 200,
            "body": json_stringify({
                "first": first,
                "second": second,
                "rest": rest,
                "head": head,
                "tail": tail,
                "last_3": last_n
            })
        };
    }

    fn get_last_n(arr: Array, n: Int) -> Array {
        if (n >= len(arr)) { return arr; }
        let start = len(arr) - n;
        return substring_array(arr, start);
    }

    fn substring_array(arr: Array, start: Int) -> Array {
        let result = [];
        let i = start;
        while (i < len(arr)) {
            result.push(arr[i]);
            i = i + 1;
        }
        return result;
    }

    fn chunk_array(req: Any) -> Any {
        let data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let chunks = chunk(data, 3);

        return {
            "status": 200,
            "body": json_stringify({
                "original": data,
                "chunks": chunks
            })
        };
    }

    fn chunk(arr: Array, size: Int) -> Array {
        let result = [];
        let current = [];
        let i = 0;

        for item in arr {
            current.push(item);
            if (len(current) == size) {
                result.push(current);
                current = [];
            }
        }

        if (len(current) > 0) {
            result.push(current);
        }

        return result;
    }
}

// ============================================================================
// EXAMPLE 6: Hash Spread - Basic Usage
// ============================================================================

class HashSpreadController extends Controller {
    fn basic_spread(req: Any) -> Any {
        let defaults = {
            "name": "Default Name",
            "email": "default@example.com",
            "age": 0,
            "country": "USA",
            "language": "English"
        };

        let user_input = {
            "name": "John Doe",
            "email": "john@example.com"
        };

        let user = { ...defaults, ...user_input };

        return {
            "status": 200,
            "body": json_stringify({
                "defaults": defaults,
                "user_input": user_input,
                "merged": user
            })
        };
    }

    fn override_values(req: Any) -> Any {
        let base_config = {
            "debug": false,
            "log_level": "info",
            "max_connections": 100,
            "timeout": 30,
            "retries": 3
        };

        let overrides = {
            "debug": true,
            "timeout": 60
        };

        let final_config = { ...base_config, ...overrides };

        return {
            "status": 200,
            "body": json_stringify({
                "base": base_config,
                "overrides": overrides,
                "final": final_config
            })
        };
    }

    fn add_new_keys(req: Any) -> Any {
        let user = {
            "id": 1,
            "name": "John",
            "email": "john@example.com"
        };

        let enriched = {
            ...user,
            "created_at": DateTime.now(),
            "last_login": DateTime.now(),
            "status": "active"
        };

        return {
            "status": 200,
            "body": json_stringify({
                "user": user,
                "enriched": enriched
            })
        };
    }
}

// ============================================================================
// EXAMPLE 7: Configuration Management with Hash Spread
// ============================================================================

class ConfigController extends Controller {
    fn load_config(req: Any) -> Any {
        let default_config = {
            "app_name": "MyApp",
            "version": "1.0.0",
            "environment": "development",
            "debug": true,
            "log_level": "debug",
            "database": {
                "host": "localhost",
                "port": 6745,
                "name": "myapp"
            },
            "cache": {
                "enabled": true,
                "ttl": 300,
                "max_size": 1000
            }
        };

        let env_config = {
            "environment": "production",
            "debug": false,
            "log_level": "error"
        };

        let override_config = {
            "database": {
                "host": "prod-db.example.com",
                "port": 6745
            }
        };

        let config = {
            ...default_config,
            ...env_config,
            ...override_config,
            "cache": {
                ...default_config["cache"],
                ...override_config["cache"] ?? {},
                "ttl": 600
            }
        };

        return {
            "status": 200,
            "body": json_stringify({
                "config": config
            })
        };
    }

    fn merge_options(req: Any) -> Any {
        let request_options = req["json"] ?? {};

        let default_options = {
            "method": "GET",
            "headers": {
                "Content-Type": "application/json"
            },
            "timeout": 30,
            "retries": 3,
            "cache": false
        };

        let final_options = { ...default_options, ...request_options };

        if (has_key(request_options, "headers")) {
            final_options["headers"] = {
                ...default_options["headers"],
                ...request_options["headers"]
            };
        }

        return {
            "status": 200,
            "body": json_stringify({
                "options": final_options
            })
        };
    }
}

// ============================================================================
// EXAMPLE 8: Component Props with Rest and Spread
// ============================================================================

class ComponentController extends Controller {
    fn render_component(req: Any) -> Any {
        let props = {
            "title": "My Component",
            "class": "custom-class",
            "data": {
                "id": 123,
                "name": "Test"
            }
        };

        let rendered = render_button({
            ...props,
            "variant": "primary",
            "disabled": false
        });

        return {
            "status": 200,
            "body": json_stringify({
                "props": props,
                "rendered": rendered
            })
        };
    }

    fn render_button(...props: Hash[]) -> Any {
        let merged = {};
        for p in props {
            merged = { ...merged, ...p };
        }

        let { title, class: className, variant = "default", disabled = false, data } = merged;

        return {
            "type": "button",
            "props": {
                "title": title,
                "class": className,
                "variant": variant,
                "disabled": disabled,
                "data": data
            }
        };
    }

    fn extract_props(req: Any) -> Any {
        let component_props = {
            "visible": true,
            "title": "Card Title",
            "subtitle": "Card Subtitle",
            "content": "Card content goes here",
            "footer": "Card footer",
            "class": "custom-card",
            "style": { "padding": "20px" },
            "onClick": "handleClick"
        };

        let { visible, ...spreadable_props } = component_props;

        return {
            "status": 200,
            "body": json_stringify({
                "visible": visible,
                "spreadable": spreadable_props
            })
        };
    }
}

// ============================================================================
// EXAMPLE 9: API Response Building with Spread
// ============================================================================

class ResponseController extends Controller {
    fn build_response(req: Any) -> Any {
        let base_response = {
            "success": true,
            "timestamp": DateTime.now(),
            "request_id": generate_request_id()
        };

        let data = {
            "users": [
                { "id": 1, "name": "John" },
                { "id": 2, "name": "Jane" }
            ],
            "count": 2
        };

        let response = {
            ...base_response,
            "data": data
        };

        return {
            "status": 200,
            "body": json_stringify(response)
        };
    }

    fn build_error_response(req: Any) -> Any {
        let error = req["json"]["error"] ?? "Unknown error";

        let base_error = {
            "success": false,
            "timestamp": DateTime.now(),
            "error": {
                "code": "INTERNAL_ERROR",
                "message": error
            }
        };

        let detailed_error = {
            ...base_error,
            "error": {
                ...base_error["error"],
                "details": req["json"]["details"] ?? {},
                "request_id": generate_request_id()
            }
        };

        return {
            "status": 500,
            "body": json_stringify(detailed_error)
        };
    }

    fn build_paginated_response(req: Any) -> Any {
        let page = req["query"]["page"] ?? 1;
        let limit = req["query"]["limit"] ?? 10;

        let items = get_items(page, limit);
        let total = get_total_count();

        let base_pagination = {
            "success": true,
            "timestamp": DateTime.now(),
            "pagination": {
                "page": page,
                "limit": limit,
                "total": total,
                "total_pages": ceil(total / limit)
            }
        };

        return {
            ...base_pagination,
            "data": items
        };
    }
}

// ============================================================================
// EXAMPLE 10: Event Handling with Rest Parameters
// ============================================================================

class EventController extends Controller {
    fn handle_events(req: Any) -> Any {
        emit("user.login", "user_id", 123, "ip", "192.168.1.1");
        emit("order.created", "order_id", "ORD-456", "amount", 99.99, "items", 3);

        return { "status": 200, "body": "Events emitted" };
    }

    fn emit(event_type: String, ...key_values: Any[]) -> Any {
        let event = {
            "type": event_type,
            "timestamp": DateTime.now(),
            "data": {}
        };

        let i = 0;
        while (i < len(key_values)) {
            let key = key_values[i];
            let value = key_values[i + 1];
            event["data"][key] = value;
            i = i + 2;
        }

        print("[EVENT] " + json_stringify(event));
        return event;
    }

    fn subscribe(event_type: String, callback: Fn) -> Any {
        return {
            "event": event_type,
            "callback": callback
        };
    }

    fn trigger_handlers(...handlers: Fn[]) -> Any {
        let results = [];

        for handler in handlers {
            results.push(handler());
        }

        return results;
    }
}

// ============================================================================
// EXAMPLE 11: Function Composition with Rest
// ============================================================================

class CompositionController extends Controller {
    fn compose_functions(req: Any) -> Any {
        let double = fn(x) { return x * 2; };
        let add_one = fn(x) { return x + 1; };
        let square = fn(x) { return x * x; };

        let pipeline = [double, add_one, square];
        let result = run_pipeline(5, ...pipeline);

        return {
            "status": 200,
            "body": json_stringify({
                "result": result,
                "steps": "5 -> double(10) -> add_one(11) -> square(121)"
            })
        };
    }

    fn run_pipeline(initial_value: Any, ...fns: Fn[]) -> Any {
        let result = initial_value;

        for fn_item in fns {
            result = fn_item(result);
        }

        return result;
    }

    fn pipe(...fns: Fn[]) -> Fn {
        return fn(value) {
            return run_pipeline(value, ...fns);
        };
    }
}

// ============================================================================
// EXAMPLE 12: Middleware with Rest Parameters
// ============================================================================

class MiddlewareController extends Controller {
    fn apply_middleware(req: Any) -> Any {
        let middlewares = [
            fn(r) { r["logged"] = true; return r; },
            fn(r) { r["authenticated"] = true; return r; },
            fn(r) { r["processed"] = true; return r; }
        ];

        let result = apply_all(req, ...middlewares);

        return {
            "status": 200,
            "body": json_stringify({
                "request": result
            })
        };
    }

    fn apply_all(initial: Any, ...middlewares: Fn[]) -> Any {
        let current = initial;

        for mw in middlewares {
            current = mw(current);
        }

        return current;
    }

    fn create_chain(...fns: Fn[]) -> Fn {
        return fn(req) {
            return apply_all(req, ...fns);
        };
    }

    fn log_request(req: Any) -> Any {
        let { method, path, headers } = req;
        print("[REQUEST] " + method + " " + path);
        return req;
    }

    fn add_request_id(req: Any) -> Any {
        return {
            ...req,
            "request_id": generate_request_id()
        };
    }

    fn measure_time(req: Any) -> Any {
        let start = clock();
        req["start_time"] = start;
        return req;
    }
}

// ============================================================================
// EXAMPLE 13: Complex Real-World Example
// ============================================================================

class ComplexRestSpreadController extends Controller {
    fn handle_api_request(req: Any) -> Any {
        let request_data = req["json"];

        let validated = validate_request(
            request_data,
            validate_required_fields,
            validate_types,
            validate_business_rules
        );

        let processed = process_with_options(
            validated,
            ...get_processing_steps()
        );

        let response = build_api_response(
            processed,
            ...get_response_parts()
        );

        return response;
    }

    fn validate_request(data: Hash, ...validators: Fn[]) -> Any {
        let errors = [];

        for validator in validators {
            let result = validator(data);
            if (result != null) {
                errors.push(result);
            }
        }

        if (len(errors) > 0) {
            throw ValueError.new("Validation failed", { "errors": errors });
        }

        return data;
    }

    fn validate_required_fields(data: Hash) -> Any {
        let required = ["name", "email"];
        let missing = [];

        for field in required {
            if (!has_key(data, field) || data[field] == null) {
                missing.push(field);
            }
        }

        if (len(missing) > 0) {
            return { "code": "MISSING_FIELDS", "fields": missing };
        }

        return null;
    }

    fn validate_types(data: Hash) -> Any {
        return null;
    }

    fn validate_business_rules(data: Hash) -> Any {
        return null;
    }

    fn get_processing_steps() -> Array {
        return [
            fn(d) { d["normalized"] = true; return d; },
            fn(d) { d["transformed"] = true; return d; },
            fn(d) { d["enriched"] = true; return d; }
        ];
    }

    fn process_with_options(data: Hash, ...steps: Fn[]) -> Any {
        let result = data;
        for step in steps {
            result = step(result);
        }
        return result;
    }

    fn get_response_parts() -> Array {
        return [
            { "success": true },
            { "version": "1.0" },
            { "generated_at": DateTime.now() }
        ];
    }

    fn build_api_response(data: Hash, ...parts: Hash[]) -> Any {
        let response = { ...data };

        for part in parts {
            response = { ...response, ...part };
        }

        return {
            "status": 200,
            "body": json_stringify(response)
        };
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

fn generate_request_id() -> String {
    return "req_" + str(clock()) + "_" + str(random());
}

fn random() -> Int {
    return 12345;
}

fn ceil(n: Int) -> Int {
    return n;
}

fn get_items(page: Int, limit: Int) -> Array {
    return [];
}

fn get_total_count() -> Int {
    return 100;
}

// ============================================================================
// COMPARISON: Before and After Rest/Spread
// ============================================================================

class ComparisonController extends Controller {
    fn without_rest(req: Any) -> Any {
        let numbers = [1, 2, 3, 4, 5];
        let result = sum_array(numbers);
        return { "status": 200, "body": json_stringify({ "sum": result }) };
    }

    fn sum_array(arr: Array) -> Int {
        let total = 0;
        for n in arr {
            total = total + n;
        }
        return total;
    }

    fn with_rest(req: Any) -> Any {
        let numbers = [1, 2, 3, 4, 5];
        let result = calculate_sum(...numbers);
        return { "status": 200, "body": json_stringify({ "sum": result }) };
    }

    fn calculate_sum(...numbers: Int[]) -> Int {
        let total = 0;
        for n in numbers {
            total = total + n;
        }
        return total;
    }

    fn without_spread(req: Any) -> Any {
        let defaults = { "a": 1, "b": 2, "c": 3 };
        let overrides = { "b": 20, "d": 4 };

        let merged = {};
        for key in keys(defaults) {
            merged[key] = defaults[key];
        }
        for key in keys(overrides) {
            merged[key] = overrides[key];
        }

        return { "status": 200, "body": json_stringify({ "merged": merged }) };
    }

    fn with_spread(req: Any) -> Any {
        let defaults = { "a": 1, "b": 2, "c": 3 };
        let overrides = { "b": 20, "d": 4 };

        let merged = { ...defaults, ...overrides };

        return { "status": 200, "body": json_stringify({ "merged": merged }) };
    }
}

// ============================================================================
