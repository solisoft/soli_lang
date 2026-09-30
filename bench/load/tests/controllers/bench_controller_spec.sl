# E2E spec for BenchController — exercises every benchmark workload action
# plus the helper edge cases (clamping, 404s, validation failure).

describe("BenchController") do
    before_all() do
        test_server_start()
    end

    before_each() do
        as_guest()
        clear_headers()
    end

    describe("GET /bench/json") do
        test("returns a single JSON object") do
            response = get("/bench/json")
            assert_eq(res_status(response), 200)
            assert_eq(res_header(response, "Content-Type"), "application/json")
            body = res_json(response)
            assert_eq(body["name"], "Widget 1")
            assert_contains(body["tags"], "alpha")
        end
    end

    describe("GET /bench/json_many") do
        test("returns N generated objects") do
            response = get("/bench/json_many?n=5")
            assert_eq(res_status(response), 200)
            body = res_json(response)
            assert_eq(body["count"], 5)
            assert_eq(body["items"].length(), 5)
        end

        test("defaults the count when n is missing") do
            response = get("/bench/json_many")
            assert_eq(res_status(response), 200)
            assert_eq(res_json(response)["count"], 100)
        end

        test("clamps n below 1 up to 1") do
            response = get("/bench/json_many?n=0")
            assert_eq(res_json(response)["count"], 1)
        end

        test("clamps n above the maximum") do
            response = get("/bench/json_many?n=99999")
            assert_eq(res_json(response)["count"], 1000)
        end
    end

    describe("GET /bench/view") do
        test("renders the view template") do
            response = get("/bench/view?n=3")
            assert_eq(res_status(response), 200)
            assert_eq(view_path(), "bench/view.html")
            assert_eq(assign("items").length(), 3)
        end
    end

    describe("GET /bench/page") do
        before_each() do
            get("/bench/seed?n=50")
        end

        test("renders the page through the layout with partials") do
            response = get("/bench/page?step=1")
            assert_eq(res_status(response), 200)
            assert_eq(view_path(), "bench/page.html")
            assert_eq(assign("step"), 1)
            assert_hash_has_key(assigns(), "products")
        end

        test("ramps complexity to step 4 with stats") do
            response = get("/bench/page?step=4")
            assert_eq(res_status(response), 200)
            assert_eq(assign("step"), 4)
            assert_hash_has_key(assign("stats"), "rows")
        end

        test("clamps step into the 1..4 range") do
            get("/bench/page?step=0")
            assert_eq(assign("step"), 1)
            get("/bench/page?step=99")
            assert_eq(assign("step"), 4)
        end

        test("defaults to step 4 when no step is given") do
            get("/bench/page")
            assert_eq(assign("step"), 4)
        end
    end

    describe("GET /bench/seed") do
        test("ensures widgets exist and reports counts") do
            response = get("/bench/seed?n=10")
            assert_eq(res_status(response), 200)
            body = res_json(response)
            assert_eq(body["target"], 10)
            assert_gt(body["existing"] + body["created"], 9)
        end
    end

    describe("GET /bench/db_one") do
        test("returns one widget from the database") do
            get("/bench/seed?n=10")
            response = get("/bench/db_one")
            assert_eq(res_status(response), 200)
            assert_not_null(res_json(response)["id"])
        end
    end

    describe("GET /bench/db_list") do
        test("returns N widgets from the database") do
            get("/bench/seed?n=20")
            response = get("/bench/db_list?n=5")
            assert_eq(res_status(response), 200)
            assert_eq(res_json(response)["count"], 5)
        end
    end

    describe("CRUD lifecycle") do
        test("creates, reads, updates and deletes a widget") do
            created = post("/bench/crud", { "name": "Spec Widget", "price": "42" })
            assert_eq(res_status(created), 201)
            id = res_json(created)["id"]
            assert_not_null(id)

            shown = get("/bench/crud/" + id)
            assert_eq(res_status(shown), 200)
            assert_eq(res_json(shown)["name"], "Spec Widget")

            updated = put("/bench/crud/" + id, { "price": "99" })
            assert_eq(res_status(updated), 200)
            assert_eq(res_json(updated)["price"], 99)

            removed = delete("/bench/crud/" + id)
            assert_eq(res_status(removed), 200)
            assert_eq(res_json(removed)["deleted"], id)
        end

        test("rejects creation with a blank name") do
            response = post("/bench/crud", { "name": "" })
            assert_eq(res_status(response), 422)
            assert_hash_has_key(res_json(response), "errors")
        end

        test("returns 404 reading a missing widget") do
            response = get("/bench/crud/does-not-exist")
            assert_eq(res_status(response), 404)
        end

        test("returns 404 updating a missing widget") do
            response = put("/bench/crud/does-not-exist", { "price": "1" })
            assert_eq(res_status(response), 404)
        end

        test("returns 404 deleting a missing widget") do
            response = delete("/bench/crud/does-not-exist")
            assert_eq(res_status(response), 404)
        end
    end

    describe("DELETE /bench/crud (load workload)") do
        test("deletes one arbitrary row and reports it") do
            get("/bench/seed?n=10")
            response = delete("/bench/crud")
            assert_eq(res_status(response), 200)
            assert_eq(res_json(response)["deleted"], 1)
        end

        test("returns 200 with deleted=0 when the collection is empty") do
            Widget.delete_all()
            response = delete("/bench/crud")
            assert_eq(res_status(response), 200)
            assert_eq(res_json(response)["deleted"], 0)
        end
    end
end
