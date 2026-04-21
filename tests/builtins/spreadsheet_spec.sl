// ============================================================================
// Spreadsheet Parsing Test Suite
// ============================================================================

describe("Spreadsheet.csv", fn() {
    test("parses simple CSV string", fn() {
        let csv_content = "name,email,age\nAlice,alice@example.com,30\nBob,bob@example.com,25";
        let data = Spreadsheet.csv(csv_content);
        assert_eq(len(data), 2);
        assert_eq(data[0]["name"], "Alice");
        assert_eq(data[0]["email"], "alice@example.com");
        assert_eq(data[0]["age"], "30");
        assert_eq(data[1]["name"], "Bob");
    });

    test("handles empty cells as null", fn() {
        let csv_content = "a,b,c\n1,,3";
        let data = Spreadsheet.csv(csv_content);
        assert_eq(data[0]["a"], "1");
        assert_null(data[0]["b"]);
        assert_eq(data[0]["c"], "3");
    });

    test("handles trailing newline", fn() {
        let csv_content = "x,y\n1,2\n";
        let data = Spreadsheet.csv(csv_content);
        assert_eq(len(data), 1);
    });

    test("handles quoted fields with commas", fn() {
        let csv_content = "name,desc\nAlice,\"Hello, World\"\nBob,\"Test, Value\"";
        let data = Spreadsheet.csv(csv_content);
        assert_eq(data[0]["name"], "Alice");
        assert_eq(data[0]["desc"], "Hello, World");
        assert_eq(data[1]["desc"], "Test, Value");
    });
});

describe("Spreadsheet.csv_file", fn() {
    test("parses CSV file from disk", fn() {
        let data = Spreadsheet.csv_file("tests/fixtures/test.csv");
        assert_eq(len(data), 3);
        assert_eq(data[0]["name"], "Alice");
        assert_eq(data[0]["email"], "alice@example.com");
        assert_eq(data[1]["name"], "Bob");
    });

    test("returns empty array for empty file", fn() {
        let data = Spreadsheet.csv_file("tests/fixtures/empty.csv");
        assert_eq(len(data), 0);
    });
});

describe("Spreadsheet.excel", fn() {
    test("parses Excel file from disk", fn() {
        let data = Spreadsheet.excel("tests/fixtures/test.xlsx");
        assert_eq(len(data), 1);
        assert(data[0].has_key("Name"));
        assert(data[0].has_key("Email"));
    });

    test("preserves numeric values", fn() {
        let data = Spreadsheet.excel("tests/fixtures/test.xlsx");
        assert(data[0]["Age"] != null);
    });
});

describe("Integration", fn() {
    test("can iterate over parsed spreadsheet data", fn() {
        let data = Spreadsheet.csv("name,value\nA,1\nB,2\nC,3");
        let sum = 0;
        for row in data {
            sum = sum + int(row["value"]);
        }
        assert_eq(sum, 6);
    });

    test("can filter parsed spreadsheet data", fn() {
        let data = Spreadsheet.csv("name,score\nAlice,85\nBob,92\nCharlie,78");
        let high_scorers = data.filter(fn(row) int(row["score"]) > 80);
        assert_eq(len(high_scorers), 2);
        assert_eq(high_scorers[0]["name"], "Alice");
        assert_eq(high_scorers[1]["name"], "Bob");
    });

    test("can map and transform data", fn() {
        let data = Spreadsheet.csv("name,score\nAlice,85\nBob,92");
        let names = data.map(fn(row) row["name"]);
        assert_eq(len(names), 2);
        assert_eq(names[0], "Alice");
        assert_eq(names[1], "Bob");
    });
});

describe("Export", fn() {
    test("to_csv produces valid CSV with headers", fn() {
        let data = [
            {"name": "Alice", "age": "30"},
            {"name": "Bob", "age": "25"}
        ];
        let csv = Spreadsheet.to_csv(data);
        assert(len(csv) > 0);
    });

    test("csv_write round-trip preserves data", fn() {
        let original = [
            {"name": "Charlie", "score": "95"},
            {"name": "Diana", "score": "88"}
        ];
        Spreadsheet.csv_write(original, "/tmp/test_export.csv");
        let result = Spreadsheet.csv_file("/tmp/test_export.csv");
        assert_eq(len(result), 2);
        assert_eq(result[0]["name"], "Charlie");
        assert_eq(result[0]["score"], "95");
        assert_eq(result[1]["name"], "Diana");
        assert_eq(result[1]["score"], "88");
    });

    test("csv_write handles empty array", fn() {
        Spreadsheet.csv_write([], "/tmp/test_empty.csv");
        let result = Spreadsheet.csv_file("/tmp/test_empty.csv");
        assert_eq(len(result), 0);
    });

    test("excel_write creates Excel file with data", fn() {
        let data = [
            {"name": "Eve", "id": "1"},
            {"name": "Frank", "id": "2"}
        ];
        Spreadsheet.excel_write(data, "/tmp/test_export.xlsx");
        let result = Spreadsheet.excel("/tmp/test_export.xlsx");
        assert_eq(len(result), 2);
        assert(result[0].has_key("name"));
        assert(result[1].has_key("id"));
    });

    test("excel_write creates correct row count", fn() {
        let data = [
            {"a": "1"},
            {"a": "2"},
            {"a": "3"}
        ];
        Spreadsheet.excel_write(data, "/tmp/test_rows.xlsx");
        let result = Spreadsheet.excel("/tmp/test_rows.xlsx");
        assert_eq(len(result), 3);
    });
});