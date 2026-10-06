# Spreadsheet: parsing CSV (string or file) and Excel into arrays of hashes,
# and exporting arrays of hashes back to CSV and Excel.

const CSV_OUT = "tests/fixtures/_spreadsheet_spec.csv"
const XLSX_OUT = "tests/fixtures/_spreadsheet_spec.xlsx"

describe("Spreadsheet") do
  after_each() do
    [CSV_OUT, XLSX_OUT].each do |path|
      File.delete(path) if File.exists(path)
    end
  end

  describe("Spreadsheet.csv") do
    test("parses rows into hashes keyed by the header") do
      data = Spreadsheet.csv("name,email,age\nAlice,alice@example.com,30\nBob,bob@example.com,25")
      assert_eq(data, [
        {"name": "Alice", "email": "alice@example.com", "age": "30"},
        {"name": "Bob", "email": "bob@example.com", "age": "25"}
      ])
    end

    test("reads empty cells as nil") do
      data = Spreadsheet.csv("a,b,c\n1,,3")
      assert_eq(data[0]["a"], "1")
      assert_null(data[0]["b"])
      assert_eq(data[0]["c"], "3")
    end

    test("ignores a trailing newline") do
      assert_eq(Spreadsheet.csv("x,y\n1,2\n"), [{"x": "1", "y": "2"}])
    end

    test("handles quoted fields with commas and escaped quotes") do
      data = Spreadsheet.csv("name,desc\nAlice,\"Hello, World\"\nBob,\"say \"\"hi\"\"\"")
      assert_eq(data[0]["desc"], "Hello, World")
      assert_eq(data[1]["desc"], "say \"hi\"")
    end

    test("leaves out the cells a short row does not have") do
      assert_eq(Spreadsheet.csv("a,b\n1"), [{"a": "1"}])
    end

    test("returns no rows for an empty string or a header alone") do
      assert_eq(Spreadsheet.csv(""), [])
      assert_eq(Spreadsheet.csv("a,b\n"), [])
    end

    test("refuses something that is not a string") do
      assert_raises("Spreadsheet.csv() expects string, got int") do
        Spreadsheet.csv(5)
      end
    end
  end

  describe("Spreadsheet.csv_file") do
    test("parses a CSV file from disk") do
      data = Spreadsheet.csv_file("tests/fixtures/test.csv")
      assert_eq(data.length, 3)
      assert_eq(data[0], {"name": "Alice", "email": "alice@example.com", "age": "30"})
      assert_eq(data[2]["name"], "Charlie")
    end

    test("returns an empty array for an empty file") do
      assert_eq(Spreadsheet.csv_file("tests/fixtures/empty.csv"), [])
    end

    test("raises on a missing file") do
      assert_raises("Spreadsheet.csv_file() cannot open tests/fixtures/nope.csv") do
        Spreadsheet.csv_file("tests/fixtures/nope.csv")
      end
    end
  end

  describe("Spreadsheet.excel") do
    test("parses the first sheet of an Excel file") do
      data = Spreadsheet.excel("tests/fixtures/test.xlsx")
      assert_eq(data, [{"Name": "Alice", "Email": "alice@example.com", "Age": "30"}])
    end

    test("keeps the header order of the sheet") do
      assert_eq(Spreadsheet.excel("tests/fixtures/test.xlsx")[0].keys, ["Name", "Email", "Age"])
    end

    test("raises on a missing file") do
      assert_raises("Spreadsheet.excel() cannot open tests/fixtures/nope.xlsx") do
        Spreadsheet.excel("tests/fixtures/nope.xlsx")
      end
    end
  end

  describe("working with parsed rows") do
    test("rows iterate, filter and map like any array of hashes") do
      data = Spreadsheet.csv("name,score\nAlice,85\nBob,92\nCharlie,78")
      total = data.map { |row| int(row["score"]) }.sum
      high_scorers = data.filter { |row| int(row["score"]) > 80 }.map { |row| row["name"] }
      assert_eq(total, 255)
      assert_eq(high_scorers, ["Alice", "Bob"])
    end
  end

  describe("Spreadsheet.to_csv") do
    test("writes a header of the first row's keys, sorted, then one line per row") do
      data = [{"name": "Alice", "age": "30"}, {"name": "Bob", "age": 25}]
      assert_eq(Spreadsheet.to_csv(data), "age,name\n30,Alice\n25,Bob\n")
    end

    test("writes nil as an empty cell and a missing key as an empty cell") do
      assert_eq(Spreadsheet.to_csv([{"b": nil, "a": true, "c": 1.5}]), "a,b,c\ntrue,,1.5\n")
      assert_eq(Spreadsheet.to_csv([{"a": "1", "b": "2"}, {"a": "3"}]), "a,b\n1,2\n3,\n")
    end

    test("an empty array gives an empty header line") do
      assert_eq(Spreadsheet.to_csv([]), "\n")
    end

    test("refuses rows that are not hashes") do
      assert_raises("Data must be an array of hashes") do
        Spreadsheet.to_csv([1, 2])
      end
    end

    test("quotes a cell that contains a comma, so it parses back") do
      pending("bug: Spreadsheet.to_csv does not quote cells containing a comma or a quote")
      data = [{"desc": "Hello, World", "n": "1"}]
      assert_eq(Spreadsheet.csv(Spreadsheet.to_csv(data)), data)
    end
  end

  describe("Spreadsheet.csv_write") do
    test("round-trips through csv_file") do
      original = [{"name": "Charlie", "score": "95"}, {"name": "Diana", "score": "88"}]
      assert_null(Spreadsheet.csv_write(original, CSV_OUT))
      assert_eq(File.read(CSV_OUT), "name,score\nCharlie,95\nDiana,88\n")
      assert_eq(Spreadsheet.csv_file(CSV_OUT), original)
    end

    test("an empty array reads back as no rows") do
      Spreadsheet.csv_write([], CSV_OUT)
      assert_eq(Spreadsheet.csv_file(CSV_OUT), [])
    end
  end

  describe("Spreadsheet.excel_write") do
    test("round-trips through excel") do
      Spreadsheet.excel_write([{"name": "Eve", "id": "1"}, {"name": "Frank", "id": "2"}], XLSX_OUT)
      assert_eq(Spreadsheet.excel(XLSX_OUT), [{"id": "1", "name": "Eve"}, {"id": "2", "name": "Frank"}])
    end

    test("takes the column order and keeps numbers numeric") do
      data = [{"Montant": 25.5, "N°": "00042", "Places": 2}, {"Montant": 10, "N°": "43"}]
      Spreadsheet.excel_write(data, XLSX_OUT, ["N°", "Places", "Montant"])
      result = Spreadsheet.excel(XLSX_OUT)
      assert_eq(result.length, 2)
      assert_eq(result[0].keys, ["N°", "Places", "Montant"])
      assert_eq(result[0]["N°"], "00042")
      assert_eq(result[0]["Places"], 2.0)
      assert_eq(result[0]["Montant"], 25.5)
      assert_eq(result[1]["Montant"], 10.0)
    end

    test("writes one row per hash") do
      Spreadsheet.excel_write([{"a": "1"}, {"a": "2"}, {"a": "3"}], XLSX_OUT)
      assert_eq(Spreadsheet.excel(XLSX_OUT).map { |row| row["a"] }, ["1", "2", "3"])
    end

    test("an empty array writes a sheet with no rows") do
      Spreadsheet.excel_write([], XLSX_OUT)
      assert_eq(Spreadsheet.excel(XLSX_OUT), [])
    end

    test("checks its arguments") do
      assert_raises("Spreadsheet.excel_write() expects 2-3 arguments (data, path, columns?), got 1") do
        Spreadsheet.excel_write([{"a": 1}])
      end
      assert_raises("Spreadsheet.excel_write() columns must be strings, got int") do
        Spreadsheet.excel_write([{"a": 1}], XLSX_OUT, [1])
      end
    end
  end
end
