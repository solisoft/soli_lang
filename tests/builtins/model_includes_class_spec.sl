# .includes(): the SDBQL it generates (no database needed), and the class an
# eager-loaded relation is hydrated as — derived from the related row's _id.

class Organisation < Model
  has_many("contacts")
end

class Contact < Model
  belongs_to("organisation")
end

class Branch < Model
  has_many("contacts")
  has_many("employees")
end

describe("Model includes - query generation") do
  test("belongs_to loads the first matching parent by its foreign key") do
    assert_eq(Contact.includes("organisation").to_query,
      "FOR doc IN contacts " +
      "LET _rel_organisation = (FOR rel IN organisations FILTER rel._key == doc.organisation_id LIMIT 1 RETURN rel) " +
      "RETURN MERGE(doc, {organisation: FIRST(_rel_organisation)})")
  end

  test("has_many loads every child pointing back at the parent") do
    assert_eq(Organisation.includes("contacts").to_query,
      "FOR doc IN organisations " +
      "LET _rel_contacts = (FOR rel IN contacts FILTER rel.organisation_id == doc._key RETURN rel) " +
      "RETURN MERGE(doc, {contacts: _rel_contacts})")
  end

  test("several relations emit one LET each and merge them all") do
    query = Branch.includes("contacts", "employees").to_query

    assert_contains(query, "LET _rel_contacts = (FOR rel IN contacts FILTER rel.branch_id == doc._key RETURN rel)")
    assert_contains(query, "LET _rel_employees = (FOR rel IN employees FILTER rel.branch_id == doc._key RETURN rel)")
    assert_contains(query, "RETURN MERGE(doc, {contacts: _rel_contacts, employees: _rel_employees})")
  end

  test("chained includes calls build the same query as one call with both names") do
    assert_eq(Branch.includes("contacts").includes("employees").to_query,
      Branch.includes("contacts", "employees").to_query)
  end

  test("a where filter comes before the relation subqueries") do
    query = Contact.where("doc.active == @active", {"active": true}).includes("organisation").to_query

    assert_match(query, "^FOR doc IN contacts FILTER doc.active == @active LET _rel_organisation")
  end

  test("an unknown relation raises") do
    assert_raises("No relation 'nope' defined on Contact") do
      Contact.includes("nope").to_query
    end
  end
end

describe("Model includes - class derivation from _id") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    Contact.delete_all()
    Organisation.delete_all()
  end

  test("a belongs_to preload is an Organisation instance, not a hash") do
    organisation = Organisation.create({"name": "Acme"})
    contact = Contact.create({"name": "Bob", "organisation_id": organisation._key})

    loaded = Contact.where("doc._key == @key", {"key": contact._key}).includes("organisation").first

    assert_eq(loaded.class, "Contact")
    assert_eq(loaded.organisation.class, "Organisation")
    assert_eq(loaded.organisation.name, "Acme")
    assert_eq(loaded.organisation._key, organisation._key)
  end

  test("a belongs_to preload with no matching parent is nil") do
    Contact.create({"name": "Orphan", "organisation_id": "missing"})

    loaded = Contact.includes("organisation").first

    assert_eq(loaded.name, "Orphan")
    assert_null(loaded.organisation)
  end
end
