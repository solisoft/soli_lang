# Model lifecycle hooks and vetoes, mock-served reads (mock_query_result,
# live_where, variance), class-level helpers (broadcast, columnar_stats,
# uploaders, attr_accessible) and the schema DSL (soft_delete, timeseries,
# columnar/column, fulltext_index, table, enum_field + state_machine).
#
# Persistence calls keep their parentheses here — `save()`, `update()`,
# `delete()` — because the bare forms skip every callback (pending tests below).

class HookDoc < Model
  before_save("stamp_before_save")
  before_create("stamp_before_create")
  after_create("stamp_after_create")
  after_save("stamp_after_save")

  def stamp_before_save
    @chain = (@chain || "") + "before_save;"
  end

  def stamp_before_create
    @chain = (@chain || "") + "before_create;"
  end

  def stamp_after_create
    @chain = (@chain || "") + "after_create;"
  end

  def stamp_after_save
    @chain = (@chain || "") + "after_save;"
  end
end

class VetoCreateDoc < Model
  before_create("refuse")

  def refuse
    @veto_ran = true
    false
  end
end

class DeleteHookDoc < Model
  before_delete("log_before_delete")
  after_delete("log_after_delete")

  def log_before_delete
    @delete_chain = (@delete_chain || "") + "before_delete;"
  end

  def log_after_delete
    @delete_chain = (@delete_chain || "") + "after_delete;"
  end
end

class VetoDeleteDoc < Model
  before_delete("refuse_delete")

  def refuse_delete
    false
  end
end

class UpdateHookDoc < Model
  before_update("stamp_before_update")
  after_update("stamp_after_update")

  def stamp_before_update
    @update_chain = (@update_chain || "") + "before_update;"
  end

  def stamp_after_update
    @update_chain = (@update_chain || "") + "after_update;"
  end
end

class MockWidget < Model
end

class HabtmPost < Model
  has_and_belongs_to_many("tags")
end

class UploadDoc < Model
  has_one_attached("avatar")
  has_many_attached("gallery", {"service": "s3"})
  uploader(
    "raw_dump",
    {
      "service": "disk",
      "max_size": 1234,
      "content_types": ["application/pdf"]
    }
  )
end

class AttrWhitelistDoc < Model
  attr_accessible("title")
end

class SoftDoc < Model
  soft_delete
end

class TimeseriesReading < Model
  timeseries(retention: "30d", timestamp: "recorded_at")
end

class ColumnarEvent < Model
  columnar(compression: "lz4")
  column("url", "string")
  column("views", "int", nullable: true, indexed: true)
end

class FulltextDoc < Model
  fulltext_index("title", "body")
end

class TableBoundDoc < Model
  table("legacy_widgets")
end

enum TrafficLight
  Off,
  On
end

class Lamp < Model
  enum_field(:state, TrafficLight)
  state_machine(:state) do
    initial(TrafficLight.Off)

    event(:switch_on) do
      transition(from: TrafficLight.Off, to: TrafficLight.On)
    end

    event(:switch_off) do
      transition(from: TrafficLight.On, to: TrafficLight.Off)
    end

    before_transition(to: TrafficLight.On) do
      this.flip_log = (this.flip_log || "") + "before_on;"
    end
    after_transition(to: TrafficLight.On) do
      this.flip_log = (this.flip_log || "") + "after_on;"
    end
  end
end

const CREATE_CHAIN = "before_save;before_create;after_create;after_save;"
const ABORTED_DELETE = "before_delete callback returned false; persistence aborted"
const WIDGETS_QUERY = "FOR doc IN mock_widgets RETURN doc"
const BARE_CALL_BUG = "bug: record.save / .update / .delete without () persist but skip every lifecycle callback"

# A record whose _key does not exist in the database (`_key` is read-only on
# instances, so it is hydrated from a mocked read).
def phantom(model, collection, key)
  model.mock_query_result("FOR doc IN #{collection} RETURN doc", [{"_key": key, "title": "persisted"}])
  model.all()[0]
end

describe("lifecycle hooks") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    [HookDoc, VetoCreateDoc, DeleteHookDoc, VetoDeleteDoc, UpdateHookDoc, AttrWhitelistDoc].each do |model|
      model.clear_mocks()
      model.delete_all()
    end
  end

  describe("create and save") do
    test("create runs the before hooks in declaration order, then the afters") do
      doc = HookDoc.create({"title": "hello"})
      assert_null(doc._errors)
      assert_eq(doc.chain, CREATE_CHAIN)
    end

    test("save() on a new record runs the create chain") do
      doc = HookDoc.new({"title": "fresh"})
      assert_eq(doc.save(), true)
      assert_eq(doc.chain, CREATE_CHAIN)
    end

    test("save() on a persisted record skips the create hooks") do
      doc = HookDoc.create({"title": "first"})
      doc.chain = ""
      assert_eq(doc.save(), true)
      assert_eq(doc.chain, "before_save;after_save;")
    end

    test("a failed save() runs before_save but suppresses after_save") do
      doc = phantom(HookDoc, "hook_docs", "hk1")
      assert_eq(doc._key, "hk1")
      assert_eq(doc.save(), false)
      assert_eq(doc.chain, "before_save;")
    end

    test("a bare save runs the hooks too") do
      pending(BARE_CALL_BUG)
      doc = HookDoc.new({"title": "bare"})
      doc.save
      assert_eq(doc.chain, CREATE_CHAIN)
    end
  end

  describe("update") do
    test("update(hash) runs before_update then after_update") do
      doc = UpdateHookDoc.create({"title": "x"})
      assert_eq(doc.update({"title": "y"}), true)
      assert_eq(doc.update_chain, "before_update;after_update;")
      assert_eq(UpdateHookDoc.find(doc._key).title, "y")
    end

    test("a failed update(hash) runs before_update but suppresses after_update") do
      doc = phantom(UpdateHookDoc, "update_hook_docs", "uh1")
      assert_eq(doc.update({"title": "y"}), false)
      assert_eq(doc.update_chain, "before_update;")
    end
  end

  describe("delete") do
    test("delete() runs before_delete then after_delete") do
      doc = DeleteHookDoc.create({})
      doc.delete()
      assert_eq(doc.delete_chain, "before_delete;after_delete;")
      assert_eq(DeleteHookDoc.count, 0)
    end

    test("an unsaved record raises after before_delete, and after_delete stays silent") do
      doc = DeleteHookDoc.new({})
      assert_raises("no _key") do
        doc.delete()
      end
      assert_eq(doc.delete_chain, "before_delete;")
    end
  end

  describe("callback veto (SEC-086a)") do
    test("a before_create returning false aborts persistence") do
      doc = VetoCreateDoc.create({"title": "nope"})
      assert_eq(doc.veto_ran, true)
      assert_eq(doc._errors, [{"message": "before_create / before_save callback returned false; persistence aborted"}])
      assert_null(doc._key)
      assert_eq(VetoCreateDoc.count, 0)
    end

    test("a before_delete returning false vetoes delete() on an unsaved record") do
      doc = VetoDeleteDoc.new({})
      assert_eq(doc.delete(), false)
      assert_eq(doc._errors, [{"message": ABORTED_DELETE}])
    end

    test("a before_delete returning false keeps a stored record") do
      doc = VetoDeleteDoc.create({"title": "keep"})
      assert_eq(doc.delete(), false)
      assert_eq(doc._errors, [{"message": ABORTED_DELETE}])
      assert_eq(VetoDeleteDoc.count, 1)
    end

    test("a bare delete honours the veto too") do
      pending(BARE_CALL_BUG)
      doc = VetoDeleteDoc.create({"title": "keep"})
      doc.delete
      assert_eq(VetoDeleteDoc.count, 1)
    end
  end

  describe("attr_accessible") do
    test("drops keys outside the whitelist before the write") do
      doc = AttrWhitelistDoc.create({"title": "kept", "is_admin": true})
      assert_eq(doc.title, "kept")
      assert_null(doc["is_admin"])
      stored = AttrWhitelistDoc.find(doc._key)
      assert_eq(stored.title, "kept")
      assert_null(stored["is_admin"])
    end
  end
end

describe("mock_query_result") do
  after_each() do
    MockWidget.clear_mocks()
  end

  test("Model.all hydrates mocked rows into instances") do
    MockWidget.mock_query_result(WIDGETS_QUERY, [{"_key": "w1", "name": "Alpha"}, {"_key": "w2", "name": "Beta"}])
    widgets = MockWidget.all
    assert(widgets[0].is_a?("MockWidget"))
    assert_eq(widgets.map { |widget| widget.name }, ["Alpha", "Beta"])
    assert_eq(widgets[1]._key, "w2")
  end

  describe("against SoliDB") do
    before_each() do
      requires_solidb()
    end

    test("a mock answers only its exact query string") do
      MockWidget.mock_query_result(WIDGETS_QUERY, [{"_key": "w1", "name": "Alpha"}])
      assert_eq(MockWidget.where({"name": "Alpha"}).all, [])
    end

    test("clear_mocks drops every registered response") do
      MockWidget.mock_query_result(WIDGETS_QUERY, [{"_key": "w1", "name": "Alpha"}])
      assert_eq(MockWidget.all.length, 1)
      MockWidget.clear_mocks()
      assert_eq(MockWidget.all, [])
    end
  end
end

describe("live_where") do
  after_each() do
    MockWidget.clear_mocks()
  end

  test("runs like where() against a mock and returns instances") do
    MockWidget.mock_query_result("FOR doc IN mock_widgets FILTER doc.status == @status__eq_1 RETURN doc", [{
      "_key": "w9",
      "status": "paid",
      "name": "Paid Widget"
    }])
    rows = MockWidget.live_where({"status": "paid"})
    assert_eq(rows.length, 1)
    assert(rows[0].is_a?("MockWidget"))
    assert_eq(rows[0].name, "Paid Widget")
  end

  test("refuses a bind-vars hash with the hash-filter form") do
    assert_raises("the bind-vars hash is only valid with the string filter form") do
      MockWidget.live_where({"status": "paid"}, {})
    end
  end

  test("requires a filter argument") do
    assert_raises("Model.live_where() requires a filter argument") do
      MockWidget.live_where()
    end
  end
end

describe("variance aggregation") do
  after_each() do
    MockWidget.clear_mocks()
  end

  test("variance(field).first unwraps the mocked scalar") do
    MockWidget.mock_query_result(
      "FOR doc IN mock_widgets COLLECT AGGREGATE __soli_vals = COLLECT_LIST(doc.amount) RETURN VARIANCE(__soli_vals)",
      [7.5]
    )
    assert_eq(MockWidget.variance("amount").first, 7.5)
  end

  test("refuses a field name that is not a string") do
    assert_raises("variance() expects a field name (string or symbol)") do
      MockWidget.variance(42)
    end
  end
end

describe("broadcast") do
  test("returns 0 deliveries when nobody subscribes") do
    assert_eq(MockWidget.broadcast({"kind": "changed", "id": "w1"}), 0)
  end

  test("accepts a string payload") do
    assert_eq(MockWidget.broadcast("plain message"), 0)
  end

  test("requires a payload argument") do
    assert_raises("Model.broadcast() requires a payload argument") do
      MockWidget.broadcast()
    end
  end
end

describe("columnar_stats") do
  test("names the model and the missing declaration on a regular model") do
    assert_raises("MockWidget.columnar_stats requires a `columnar` declaration") do
      MockWidget.columnar_stats()
    end
  end
end

describe("has_and_belongs_to_many DSL") do
  test("the generated mutators demand a saved owner") do
    post = HabtmPost.new({"title": "unsaved"})
    assert_raises("owner instance has no _key (save the record first)") do
      post.add_tag("t1")
    end
  end
end

describe("attachment DSL and uploader helpers") do
  test("model_uploader_fields lists every declared attachment") do
    fields = model_uploader_fields(UploadDoc)
    assert_eq(fields.sort(), ["avatar", "gallery", "raw_dump"])
  end

  test("model_uploader_fields accepts a string class name") do
    assert_eq(model_uploader_fields("UploadDoc").sort(), ["avatar", "gallery", "raw_dump"])
  end

  test("apply_uploader_transform passes non-images through untouched") do
    file = {"filename": "notes.pdf", "content_type": "application/pdf", "data": "AAAA", "size": 3}
    assert_eq(apply_uploader_transform(file, {"max_width": 100}), file)
  end

  test("apply_uploader_transform with an empty config is a no-op") do
    file = {"filename": "pic.png", "content_type": "image/png", "data": "AAAA"}
    assert_eq(apply_uploader_transform(file, {}), file)
  end

  test("find_model_class_by_collection resolves the registry") do
    assert_eq(find_model_class_by_collection("mock_widgets"), MockWidget)
  end

  test("find_model_class_by_collection returns nil for an unknown collection") do
    assert_null(find_model_class_by_collection("no_such_collection"))
  end
end

describe("schema DSL") do
  describe("soft_delete") do
    test("queries gain the deleted_at guard") do
      assert_contains(SoftDoc.where({"name": "x"}).to_query, "FILTER doc.deleted_at == null")
    end
  end

  describe("timeseries") do
    test("a declared model still builds instances") do
      assert_eq(TimeseriesReading.new({"value": 1}).value, 1)
    end

    test("rejects an unknown option at load time") do
      assert_raises() do
        class BadTimeseries < Model
          timeseries(frobnicate: "10d")
        end
      end
    end
  end

  describe("columnar and column") do
    test("a declared model still builds instances") do
      event = ColumnarEvent.new({"url": "/a", "views": 3})
      assert_eq(event.url, "/a")
      assert_eq(event.views, 3)
    end

    test("column rejects an unknown type at load time") do
      assert_raises() do
        class BadColumnar < Model
          columnar
          column("url", "kryotype")
        end
      end
    end
  end

  describe("fulltext_index") do
    test("requires at least one field") do
      assert_raises() do
        class NoFieldFulltext < Model
          fulltext_index()
        end
      end
    end
  end

  describe("table") do
    test("binds a SQL table without renaming the SoliDB collection") do
      assert_contains(TableBoundDoc.where({"a": 1}).to_query, "FOR doc IN table_bound_docs")
    end

    test("rejects an unusable SQL identifier at load time") do
      assert_raises() do
        class BadTableDoc < Model
          table("not; a table")
        end
      end
    end
  end

  describe("enum_field and state_machine") do
    test("a transition sets the enum value and runs both hooks") do
      lamp = Lamp.new({})
      assert_eq(lamp.off?, true)
      lamp.switch_on
      assert_eq(lamp.on?, true)
      assert_eq(lamp.state.variant(), "On")
      assert_eq(lamp.flip_log, "before_on;after_on;")
    end

    test("can_X? reflects legality from the current state") do
      lamp = Lamp.new({})
      assert_eq(lamp.can_switch_on?, true)
      assert_eq(lamp.can_switch_off?, false)
      lamp.switch_on
      assert_eq(lamp.can_switch_off?, true)
    end

    test("an illegal transition raises and keeps the state") do
      lamp = Lamp.new({})
      assert_raises() do
        lamp.switch_off
      end
      assert_eq(lamp.off?, true)
    end

    test("enum_field demands an enum class as its second argument") do
      assert_raises() do
        class BadEnumDoc < Model
          enum_field(:mood, "NotAClass")
        end
      end
    end
  end
end

describe("fulltext_index search") do
  before_each() do
    requires_solidb()
    __sync_model_indexes()
    FulltextDoc.create({"title": "hello world", "body": "x"})
    FulltextDoc.create({"title": "other", "body": "hello there"})
    FulltextDoc.create({"title": "unrelated", "body": "y"})
  end

  after_each() do
    FulltextDoc.delete_all()
  end

  test("searches the first declared field by default") do
    assert_eq(FulltextDoc.search("hello").map { |doc| doc.title }, ["hello world"])
  end

  test("field: picks another declared field") do
    assert_eq(FulltextDoc.search("hello", {"field": "body"}).map { |doc| doc.title }, ["other"])
  end
end
