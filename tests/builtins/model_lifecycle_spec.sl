# ============================================================================
# Model lifecycle hooks, schema-DSL globals, and mock-query-driven APIs.
#
# Runs WITHOUT a database:
#   - Reads (all / live_where / variance) are served from query mocks
#     registered with Model.mock_query_result(query, rows).
#   - Writes fail persistence (no SolidB on localhost), which is exactly
#     what lets us observe hook gating: before_* callbacks run before the
#     write, after_* callbacks are suppressed when persistence fails.
#   - Schema DSL (soft_delete/timeseries/columnar/column/table/enum_field/
#     fulltext_index/state_machine) is exercised by defining classes with
#     it and asserting definition-time behavior + introspection.
# ============================================================================
class HookDoc < Model
  before_save("stamp_before_save")
  before_create("stamp_before_create")
  after_create("stamp_after_create")
  after_save("stamp_after_save")

  def stamp_before_save() {
    this.chain = (this.chain || "") + "before_save;"
  }
  def stamp_before_create() {
    this.chain = (this.chain || "") + "before_create;"
  }
  def stamp_after_create() {
    this.chain = (this.chain || "") + "after_create;"
  }
  def stamp_after_save() {
    this.chain = (this.chain || "") + "after_save;"
  }
end

class VetoCreateDoc < Model
  before_create("refuse")

  def refuse() {
    this.veto_ran = true
    return false
  }
end

class DeleteHookDoc < Model
  before_delete("log_before_delete")
  after_delete("log_after_delete")

  def log_before_delete() {
    this.delete_chain = (this.delete_chain || "") + "before_delete;"
  }
  def log_after_delete() {
    this.delete_chain = (this.delete_chain || "") + "after_delete;"
  }
end

class VetoDeleteDoc < Model
  before_delete("refuse_delete")

  def refuse_delete() {
    return false
  }
end

class UpdateHookDoc < Model
  before_update("stamp_before_update")
  after_update("stamp_after_update")

  def stamp_before_update() {
    this.update_chain = (this.update_chain || "") + "before_update;"
  }
  def stamp_after_update() {
    this.update_chain = (this.update_chain || "") + "after_update;"
  }
end

# --- Mock-backed reads ------------------------------------------------------

class MockWidget < Model
end

class HabtmPost < Model
  has_and_belongs_to_many("tags")
end

class UploadDoc < Model
  has_one_attached("avatar")
  has_many_attached("gallery", {"service": "s3"})
  uploader("raw_dump", {"service": "disk", "max_size": 1234, "content_types": ["application/pdf"]})
end

class AttrWhitelistDoc < Model
  attr_accessible("title")
end

# --- Schema DSL -------------------------------------------------------------

class SoftDoc < Model
  soft_delete
end

class TimeseriesReading < Model
  timeseries retention: "30d", timestamp: "recorded_at"
end

class ColumnarEvent < Model
  columnar compression: "lz4"
  column "url", "string"
  column "views", "int", nullable: true, indexed: true
end

class FulltextDoc < Model
  fulltext_index "title", "body"
end

class TableBoundDoc < Model
  table "legacy_widgets"
end

enum TrafficLight
  Off,
  On
end

class Lamp < Model
  enum_field :state, TrafficLight

  state_machine :state do
    initial TrafficLight.Off

    event :switch_on do
      transition from: TrafficLight.Off, to: TrafficLight.On
    end

    event :switch_off do
      transition from: TrafficLight.On, to: TrafficLight.Off
    end

    before_transition to: TrafficLight.On do
      this.flip_log = (this.flip_log || "") + "before_on;"
    end
    after_transition to: TrafficLight.On do
      this.flip_log = (this.flip_log || "") + "after_on;"
    end
  end
end

# ============================================================================
describe("create/save lifecycle hooks (persistence fails — no DB)", fn() {
  test("create fires before_save then before_create, never the afters", fn() {
    let result = HookDoc.create({"title": "hello"})
    # Persistence failed (no database) so _errors must be present…
    assert_not_null(result._errors)
    # …and only the pre-write hooks fired, in declaration order.
    assert_eq(result.chain, "before_save;before_create;")
  })

  test("a new-record save() runs the create chain", fn() {
    let rec = HookDoc.new({"title": "fresh"})
    assert(rec.save() == false)
    assert_eq(rec.chain, "before_save;before_create;")
  })

  test("a persisted record's save() runs the update chain", fn() {
    # `_key` is read-only on instances, so hydrate a keyed record from a mock.
    HookDoc.mock_query_result(
      "FOR doc IN hook_docs RETURN doc",
      [{"_key": "hk1", "title": "persisted"}]
    )
    let rec = HookDoc.all()[0]
    assert_eq(rec._key, "hk1")
    assert(rec.save() == false)
    assert_eq(rec.chain, "before_save;")
  })

  test("update() fires before_update but suppresses after_update on failure", fn() {
    UpdateHookDoc.mock_query_result(
      "FOR doc IN update_hook_docs RETURN doc",
      [{"_key": "uh1", "title": "x"}]
    )
    let rec = UpdateHookDoc.all()[0]
    assert_eq(rec._key, "uh1")
    assert(rec.update({"title": "y"}) == false)
    assert_eq(rec.update_chain, "before_update;")
  })
})

describe("callback veto (SEC-086a)", fn() {
  test("a before_create returning false aborts persistence", fn() {
    let doc = VetoCreateDoc.create({"title": "nope"})
    assert(doc.veto_ran == true)
    assert_not_null(doc._errors)
    assert(doc._errors[0]["message"].contains("aborted"))
  })

  test("a before_delete returning false vetoes delete()", fn() {
    let rec = VetoDeleteDoc.new({})
    assert_eq(rec.delete(), false)
    assert_not_null(rec._errors)
    assert(rec._errors[0]["message"].contains("aborted"))
  })
})

describe("delete lifecycle hooks", fn() {
  test("before_delete runs; after_delete is suppressed when delete fails", fn() {
    let rec = DeleteHookDoc.new({})
    # An unsaved record has no _key, so the native delete raises after the
    # before-callback has already run.
    let raised = false
    try
      rec.delete()
    catch e
      raised = true
    end
    assert(raised)
    assert(rec.delete_chain.contains("before_delete;"))
    assert(!rec.delete_chain.contains("after_delete;"))
  })
})

# ============================================================================
describe("mock_query_result serves reads without a database", fn() {
  test("Model.all hydrates mocked rows into instances", fn() {
    MockWidget.mock_query_result(
      "FOR doc IN mock_widgets RETURN doc",
      [
        {"_key": "w1", "name": "Alpha"},
        {"_key": "w2", "name": "Beta"}
      ]
    )
    let widgets = MockWidget.all()
    assert(widgets.is_a?("array"))
    assert_eq(len(widgets), 2)
    assert_eq(widgets[0].name, "Alpha")
    assert_eq(widgets[1]._key, "w2")
  })

  test("mocks are keyed by exact query string", fn() {
    MockWidget.mock_query_result(
      "FOR doc IN mock_widgets RETURN doc",
      [{"_key": "w1", "name": "Alpha"}]
    )
    # A different query has no mock → falls through to the (absent) DB.
    let miss = MockWidget.where({"name": "Alpha"}).all
    assert(miss.is_a?("array") && len(miss) == 0 || miss.is_a?("string"))
  })

  test("clear_mocks drops every registered response", fn() {
    MockWidget.mock_query_result(
      "FOR doc IN mock_widgets RETURN doc",
      [{"_key": "w1", "name": "Alpha"}]
    )
    assert_eq(len(MockWidget.all()), 1)
    MockWidget.clear_mocks()
    # Without the mock the read hits the wire and reports a connection error.
    assert(MockWidget.all().is_a?("string"))
    # Re-register so later suites still have their fixtures.
    MockWidget.mock_query_result(
      "FOR doc IN mock_widgets RETURN doc",
      [
        {"_key": "w1", "name": "Alpha"},
        {"_key": "w2", "name": "Beta"}
      ]
    )
  })
})

describe("live_where", fn() {
  test("runs like where() against a mock and returns instances", fn() {
    MockWidget.mock_query_result(
      "FOR doc IN mock_widgets FILTER doc.status == @status__eq_1 RETURN doc",
      [{"_key": "w9", "status": "paid", "name": "Paid Widget"}]
    )
    let rows = MockWidget.live_where({"status": "paid"})
    assert(rows.is_a?("array"))
    assert_eq(len(rows), 1)
    assert_eq(rows[0].name, "Paid Widget")
  })

  test("rejects a bind-vars hash with the hash-filter form", fn() {
    let raised = false
    try
      MockWidget.live_where({"status": "paid"}, {})
    catch e
      raised = true
    end
    assert(raised)
  })

  test("requires a filter argument", fn() {
    let raised = false
    try
      MockWidget.live_where()
    catch e
      raised = true
    end
    assert(raised)
  })
})

describe("variance aggregation", fn() {
  test("variance(field).first unwraps the mocked scalar", fn() {
    MockWidget.mock_query_result(
      "FOR doc IN mock_widgets COLLECT AGGREGATE __soli_vals = COLLECT_LIST(doc.amount) RETURN VARIANCE(__soli_vals)",
      [7.5]
    )
    assert_eq(MockWidget.variance("amount").first, 7.5)
  })

  test("variance rejects a non-string field name", fn() {
    let raised = false
    try
      MockWidget.variance(42)
    catch e
      raised = true
    end
    assert(raised)
  })
})

describe("broadcast", fn() {
  test("returns the subscriber count as an Int (0 with no subscribers)", fn() {
    let delivered = MockWidget.broadcast({"kind": "changed", "id": "w1"})
    assert(delivered.is_a?("int"))
    assert(delivered >= 0)
  })

  test("string payloads are accepted too", fn() {
    let delivered = MockWidget.broadcast("plain message")
    assert(delivered.is_a?("int"))
  })

  test("broadcast requires a payload argument", fn() {
    let raised = false
    try
      MockWidget.broadcast()
    catch e
      raised = true
    end
    assert(raised)
  })
})

describe("columnar_stats", fn() {
  test("raises a clean error on a non-columnar model", fn() {
    let msg = ""
    try
      MockWidget.columnar_stats()
    catch e
      msg = str(e)
    end
    assert(msg.contains("columnar"))
    assert(msg.contains("MockWidget"))
  })
})

# ============================================================================
describe("has_and_belongs_to_many DSL", fn() {
  test("generates association mutators that demand a saved owner", fn() {
    let post = HabtmPost.new({"title": "unsaved"})
    let raised = false
    try
      post.add_tag("t1")
    catch e
      raised = str(e).contains("_key")
    end
    assert(raised)
  })
})

describe("attachment DSL + uploader helpers", fn() {
  test("model_uploader_fields lists every declared attachment", fn() {
    let fields = model_uploader_fields(UploadDoc)
    assert(fields.includes?("avatar"))
    assert(fields.includes?("gallery"))
    assert(fields.includes?("raw_dump"))
  })

  test("model_uploader_fields accepts a string class name", fn() {
    let fields = model_uploader_fields("UploadDoc")
    assert_eq(len(fields), 3)
  })

  test("apply_uploader_transform passes non-images through untouched", fn() {
    let file = {
      "filename": "notes.pdf",
      "content_type": "application/pdf",
      "data": "AAAA",
      "size": 3
    }
    let out = apply_uploader_transform(file, {"max_width": 100})
    assert_eq(out["filename"], "notes.pdf")
    assert_eq(out["data"], "AAAA")
    assert_eq(out["size"], 3)
  })

  test("apply_uploader_transform with an empty config is a no-op", fn() {
    let file = {"filename": "pic.png", "content_type": "image/png", "data": "AAAA"}
    let out = apply_uploader_transform(file, {})
    assert_eq(out["filename"], "pic.png")
    assert_eq(out["data"], "AAAA")
  })

  test("find_model_class_by_collection resolves the registry", fn() {
    let klass = find_model_class_by_collection("mock_widgets")
    assert(!klass.nil?)
    assert_eq(str(klass), str(MockWidget))
  })

  test("find_model_class_by_collection returns null for unknown collections", fn() {
    assert(find_model_class_by_collection("no_such_collection").nil?)
  })
})

describe("attr_accessible strong params", fn() {
  test("non-whitelisted keys are dropped before instance population", fn() {
    let doc = AttrWhitelistDoc.create({"title": "kept", "is_admin": true})
    assert_eq(doc.title, "kept")
    assert(doc["is_admin"].nil?)
  })
})

# ============================================================================
describe("schema DSL: soft_delete", fn() {
  test("queries gain the deleted_at guard", fn() {
    let q = SoftDoc.where({"name": "x"}).to_query
    assert(q.contains("FILTER doc.deleted_at == null"))
  })
})

describe("schema DSL: timeseries", fn() {
  test("declares retention and timestamp options", fn() {
    let reading = TimeseriesReading.new({"value": 1})
    assert_eq(reading.value, 1)
  })

  test("rejects an unknown option at load time", fn() {
    let raised = false
    try
      class BadTimeseries < Model
        timeseries frobnicate: "10d"
      end
    catch e
      raised = true
    end
    assert(raised)
  })
})

describe("schema DSL: columnar + column", fn() {
  test("columns accept type, nullable and indexed options", fn() {
    let ev = ColumnarEvent.new({})
    assert(str(ev).contains("ColumnarEvent"))
  })

  test("column rejects an unknown type at load time", fn() {
    let raised = false
    try
      class BadColumnar < Model
        columnar
        column "url", "kryotype"
      end
    catch e
      raised = true
    end
    assert(raised)
  })
})

describe("schema DSL: fulltext_index", fn() {
  test("declares a multi-field index", fn() {
    let doc = FulltextDoc.new({"title": "hi"})
    assert_eq(doc.title, "hi")
  })

  test("fulltext_index requires at least one field", fn() {
    let raised = false
    try
      class NoFieldFulltext < Model
        fulltext_index()
      end
    catch e
      raised = true
    end
    assert(raised)
  })
})

describe("schema DSL: table", fn() {
  test("binds the model to an existing relational table", fn() {
    let row = TableBoundDoc.new({})
    assert(str(row).contains("TableBoundDoc"))
  })

  test("rejects an unusable SQL identifier at load time", fn() {
    let raised = false
    try
      class BadTableDoc < Model
        table "not; a table"
      end
    catch e
      raised = true
    end
    assert(raised)
  })
})

describe("schema DSL: enum_field + state_machine", fn() {
  test("transitions set the enum value and run both hooks", fn() {
    let lamp = Lamp.new({})
    assert_eq(lamp.off?, true)
    lamp.switch_on
    assert_eq(lamp.on?, true)
    assert_eq(lamp.state.variant(), "On")
    assert_eq(lamp.flip_log, "before_on;after_on;")
  })

  test("can_X? reflects legality from the current state", fn() {
    let lamp = Lamp.new({})
    assert_eq(lamp.can_switch_on?, true)
    assert_eq(lamp.can_switch_off?, false)
    lamp.switch_on
    assert_eq(lamp.can_switch_off?, true)
  })

  test("an illegal transition raises", fn() {
    let lamp = Lamp.new({})
    let raised = false
    try
      lamp.switch_off
    catch e
      raised = true
    end
    assert(raised)
  })

  test("enum_field demands an enum class as its second argument", fn() {
    let raised = false
    try
      class BadEnumDoc < Model
        enum_field :mood, "NotAClass"
      end
    catch e
      raised = true
    end
    assert(raised)
  })
})
