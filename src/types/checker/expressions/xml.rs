//! The two places where XML typing needs more than a signature
//! (`types::xml_classes` has the signatures).
//!
//! - **Queries.** `find`, `find_all`, `at_xpath` and `xpath` answer nodes or
//!   Strings depending on the expression (`//item` gives nodes, `//item/@id`
//!   and `//title/text()` Strings), so their signature says `Any`. When the
//!   expression is a string literal the checker compiles it with the
//!   runtime's own XPath compiler: an expression the runtime would refuse is
//!   refused here, with the same message, and the call is typed `XmlNode` or
//!   `String` (an Array of them for `find_all` / `xpath`). A union of both
//!   kinds, or an expression built at run time, stays `Any`.
//! - **Writes.** `node.text = s` and `node.name = s` are the only dot writes
//!   an XML node takes, and an `XmlReader` takes none: the runtime refuses
//!   every other one ("XmlNode has no writable property 'foo'").

use crate::ast::expr::Argument;
use crate::ast::*;
use crate::error::TypeError;
use crate::span::Span;
use crate::types::type_repr::Type;
use crate::types::xml_classes::node_type;

use super::{TypeChecker, TypeResult};

/// `find`, `find_all`, `at_xpath`, `xpath`: the methods whose answer the
/// expression decides.
pub(super) fn is_query_method(name: &str) -> bool {
    matches!(name, "find" | "find_all" | "at_xpath" | "xpath")
}

impl TypeChecker {
    /// The class of `ty` when it is `XmlNode`, `XmlDocument` or `XmlReader`.
    fn xml_class_of(&self, ty: &Type) -> Option<&'static str> {
        let Type::Class(class) = ty else {
            return None;
        };
        let class_def = self.env.get_class(&class.name)?;
        if class_def.is_or_inherits("XmlNode") {
            Some("XmlNode")
        } else if class_def.name == "XmlReader" {
            Some("XmlReader")
        } else {
            None
        }
    }

    /// The type of `receiver.name(arguments…)` for an XML query whose
    /// expression is a literal, or `None` to keep the declared `Any`.
    pub(super) fn xml_query_type(
        &self,
        receiver: &Type,
        name: &str,
        arguments: &[Argument],
    ) -> TypeResult<Option<Type>> {
        if self.xml_class_of(receiver) != Some("XmlNode") {
            return Ok(None);
        }
        let Some(Argument::Positional(expr)) = arguments.first() else {
            return Ok(None);
        };
        let ExprKind::StringLiteral(source) = &expr.kind else {
            return Ok(None);
        };
        let who = match name {
            "find" | "find_all" => "Xml.find",
            _ => "Xml.xpath",
        };
        let xpath = crate::xml::XPath::compile(source).map_err(|error| TypeError::General {
            message: format!("{who}: {error}"),
            span: expr.span,
        })?;
        let one = match xpath.yields_strings() {
            Some(true) => Type::String,
            Some(false) => node_type(),
            None => return Ok(None),
        };
        Ok(Some(match name {
            "find" | "at_xpath" => one,
            _ => Type::Array(Box::new(one)),
        }))
    }

    /// Refuse `receiver.name = value` when the runtime would: on an XML node
    /// anything but `text` and `name` (and a `name` that is no String), on an
    /// `XmlReader` anything.
    pub(crate) fn check_xml_property_write(
        &self,
        receiver: &Type,
        name: &str,
        value_type: &Type,
        span: Span,
    ) -> TypeResult<()> {
        let Some(class) = self.xml_class_of(receiver) else {
            return Ok(());
        };
        let writable = class == "XmlNode" && matches!(name, "text" | "name");
        if !writable {
            let shown = match receiver {
                Type::Class(c) => c.name.clone(),
                _ => class.to_string(),
            };
            return Err(TypeError::General {
                message: format!("{shown} has no writable property '{name}'"),
                span,
            });
        }
        if name == "name" && !value_type.is_assignable_to(&Type::String) {
            return Err(TypeError::mismatch("String", format!("{value_type}"), span));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::types::TypeChecker;

    fn check(source: &str) -> Result<(), String> {
        let tokens = crate::lexer::Scanner::new(source)
            .scan_tokens()
            .map_err(|e| e.to_string())?;
        let program = crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| e.to_string())?;
        TypeChecker::new().check(&program).map_err(|errs| {
            errs.iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    fn accepts(source: &str) {
        if let Err(e) = check(source) {
            panic!("{source:?} should type-check: {e}");
        }
    }

    /// Refused, with `needle` in the message.
    fn refuses(source: &str, needle: &str) {
        match check(source) {
            Ok(()) => panic!("{source:?} should be refused"),
            Err(e) => assert!(e.contains(needle), "{source:?}: {e:?} lacks {needle:?}"),
        }
    }

    const FEED: &str =
        "doc = Xml.parse(\"<rss><channel><title>T</title><item id='1'/></channel></rss>\")\n";

    #[test]
    fn parsing_and_the_statics() {
        accepts(&format!(
            "{FEED}doc2 = Xml.parse(\"<a/>\", strip_blank: true, max_depth: 32)\n\
             doc3 = Xml.parse(\"<a/>\", {{\"strip_blank\": true}})\n\
             doc4 = Xml.parse_file(\"imports/invoice.xml\")\n\
             ok = Xml.well_formed?(\"<a/>\")\n\
             let e: String = Xml.escape(\"a < b\")\n\
             print(Xml.escape(42), ok, doc2, doc3, doc4, e)\n\
             let c: String = Xml.c14n_exclusive(\"<a/>\")\n\
             let c2: String = Xml.c14n_exclusive(\"<a/>\", {{\"enveloped\": true}})\n\
             let one: String = Xml.get_element_by_id(\"<a ID='x'/>\", \"x\")\n\
             all = Xml.get_elements_by_tag(\"<a><b/></a>\", \"b\")\n\
             print(c, c2, one, all[0].upcase)\n"
        ));
    }

    #[test]
    fn identity_navigation_text_and_attributes() {
        accepts(&format!(
            "{FEED}root = doc.root\n\
             let k: String = root.kind\n\
             let flags: Bool = root.element? && !root.text? && !root.cdata? && !root.comment?\n\
             let more: Bool = root.pi? || doc.document?\n\
             let qn: String = root.name\n\
             print(k, flags, more, qn, root.local_name, root.prefix, root.namespace, root.path)\n\
             print(root.namespaces[\"\"], root.namespace_declarations, doc.declaration)\n\
             first = root.children[0]\n\
             print(first.name, root.nodes.length, first.parent, first.document.root)\n\
             print(first.next_sibling, first.previous_sibling, first.next_node, first.previous_node)\n\
             print(first.ancestors[0].name, root.descendants.length)\n\
             let t: String = root.text\n\
             print(t, root.own_text, root.inner_xml, root[\"id\"], root.get(\"id\"))\n\
             print(root.attribute(\"href\", \"http://www.w3.org/1999/xlink\"), root.attributes)\n\
             let has: Bool = root.has_attribute?(\"id\")\n\
             print(has, root.is_a?(\"XmlNode\"), root.class, root.nil?, root.to_h)\n"
        ));
    }

    #[test]
    fn annotations_and_the_document_subclass() {
        accepts(
            "def title_of(node: XmlNode) -> String\n  node.text\nend\n\
             def root_of(doc: XmlDocument) -> XmlNode\n  doc.root\nend\n\
             doc = Xml.parse(\"<a>x</a>\")\n\
             print(title_of(doc), title_of(doc.root), title_of(root_of(doc)))\n\
             let maybe: XmlNode = doc.root.parent\n\
             print(maybe)\n",
        );
        refuses(
            "def title_of(node: XmlNode) -> String\n  node.text\nend\nprint(title_of(\"<a/>\"))\n",
            "XmlNode",
        );
        refuses(
            "def root_of(doc: XmlDocument) -> String\n  doc.root.name\nend\n\
             print(root_of(Xml.element(\"a\")))\n",
            "XmlDocument",
        );
        refuses("let s: String = Xml.parse(\"<a/>\")\nprint(s)\n", "String");
    }

    #[test]
    fn building_mutating_and_serialising() {
        accepts(
            "doc = Xml.document(\"rss\", version: \"2.0\")\n\
             channel = doc.root.add(\"channel\")\n\
             channel.add(\"title\", \"News & views\")\n\
             [1, 2].each do |i|\n  item = channel.add(\"item\", id: i)\n  item.add(\"title\", \"P#{i}\")\n  \
             item.add(\"link\", \"u\", rel: \"alternate\", type: \"text/html\")\n  item.add_cdata(\"<b/>\")\nend\n\
             let xml: String = doc.to_xml(indent: 2)\n\
             let compact: String = doc.to_xml\n\
             let same: String = doc.to_s\n\
             let bare: XmlDocument = Xml.document\n\
             el = Xml.element(\"a\", [\"t\", Xml.element(\"b\")], {\"x\": 1})\n\
             el.children[0].remove\n\
             el.prepend(\"s\")\n\
             el.append(Xml.element(\"c\"))\n\
             el.children[0].insert_before(\"x\")\n\
             el.children[0].insert_after(Xml.element(\"d\"))\n\
             el.children[0].replace(Xml.element(\"e\"))\n\
             el.add_text(\"t\")\n\
             el.add_comment(\"c\")\n\
             el.add_pi(\"php\", \"echo 1\")\n\
             old = el.remove_attribute(\"x\")\n\
             copy = el.dup\n\
             el[\"id\"] = 1\n\
             el[\"id\"] = nil\n\
             el.set(\"k\", \"v\")\n\
             el.text = \"new\"\n\
             el.name = \"renamed\"\n\
             print(xml, compact, same, bare, old, copy == el)\n",
        );
    }

    #[test]
    fn the_block_builder_and_streaming() {
        accepts(
            "doc = Xml.build(\"feed\", {\"xmlns\": \"urn:a\"}) do |f|\n  f.add(\"title\", \"T\")\n  \
             f.add(\"entry\", id: 1) do |e|\n    e.add(\"n\", 10)\n  end\nend\n\
             c = doc.root.add(\"c\", nil, {\"k\": \"v\"}) do |x|\n  x[\"at\"] = \"1\"\nend\n\
             print(doc.to_xml(indent: 2), c)\n\
             total = Xml.each(\"exports/catalog.xml\", \"product[@status='active']\") do |product|\n  \
             print(product.to_h[\"product\"])\nend\n\
             let counted: Int = total\n\
             reader = Xml.reader(\"exports/big.xml\", \"item\", {\"p\": \"urn:p\"})\n\
             while (item = reader.next)\n  break if item[\"id\"] == \"7\"\nend\n\
             let seen: Int = reader.count\n\
             reader.close\n\
             print(counted, seen, reader)\n",
        );
    }

    #[test]
    fn a_literal_query_is_typed_from_its_expression() {
        accepts(&format!(
            "{FEED}let title: String = doc.find(\"channel/title\").text\n\
             let safe: String = doc.find(\"channel/nothing\")&.text\n\
             let ids: String = doc.find_all(\"//item/@id\")[0].upcase\n\
             let texts: String = doc.xpath(\"//title/text()\")[0]\n\
             let names: String = doc.xpath(\"rss/channel/item\")[0].name\n\
             let id: String = doc.at_xpath(\"//item/@id\").upcase\n\
             uri = \"urn:x\"\n\
             seller = doc.at_xpath(\"//ram:Name\", {{\"ram\": uri}})\n\
             lines = doc.xpath(\"//r:x\", r: uri)\n\
             expr = \"//item\"\n\
             dynamic = doc.find(expr)\n\
             either = doc.find_all(\"//item | //item/@id\")\n\
             print(title, safe, ids, texts, names, id, seller.text, lines.length, dynamic, either)\n"
        ));
        refuses(
            &format!("{FEED}print(doc.find(\"//item/@id\").children)\n"),
            "children",
        );
        refuses(
            &format!("{FEED}print(doc.find_all(\"//item\")[0].upcase)\n"),
            "upcase",
        );
        refuses(
            &format!("{FEED}print(doc.find(\"a[\"))\n"),
            "Xml.find: invalid XPath",
        );
        refuses(
            &format!("{FEED}print(doc.xpath(\"child::a\"))\n"),
            "Xml.xpath: unsupported XPath",
        );
    }

    /// A node may be nil, and testing it is the nil check.
    #[test]
    fn a_node_is_a_condition() {
        accepts(&format!(
            "{FEED}if doc.find(\"channel/title\")\n  print(\"yes\")\nend\n\
             node = doc.root.children[0]\n\
             while node\n  node = node.next_sibling\nend\n\
             print(doc.root.parent ? \"child\" : \"root\")\n\
             print(\"orphan\") unless doc.root.parent\n"
        ));
        // Any value is a condition (#693): a String one is a truthiness
        // test, `""` being falsy.
        accepts(&format!("{FEED}if doc.root.text\n  print(1)\nend\n"));
    }

    #[test]
    fn clear_misuse_is_refused() {
        refuses("print(Xml.parse(42))\n", "String");
        refuses("print(Xml.reader(1, \"item\"))\n", "String");
        refuses(&format!("{FEED}print(doc.find(42))\n"), "String");
        refuses(&format!("{FEED}print(doc.root.get(1))\n"), "String");
        refuses(
            &format!("{FEED}print(doc.root.has_attribute?(1))\n"),
            "String",
        );
        refuses(&format!("{FEED}print(doc.root.frobnicate)\n"), "frobnicate");
        refuses("print(Xml.parsee(\"<a/>\"))\n", "parsee");
        refuses(
            &format!("{FEED}print(doc.root.has_key(\"id\"))\n"),
            "has_key",
        );
        // A String result used as a node.
        refuses(
            &format!("{FEED}print(doc.root.text.children)\n"),
            "children",
        );
        refuses(&format!("{FEED}print(doc.root.name.add(\"x\"))\n"), "add");
        refuses(
            &format!("{FEED}let n: XmlNode = doc.root.text\nprint(n)\n"),
            "XmlNode",
        );
        // And a node used as a String.
        refuses(&format!("{FEED}print(doc.root.upcase)\n"), "upcase");
        refuses(
            "reader = Xml.reader(\"a.xml\", \"item\")\nprint(reader.next.upcase)\n",
            "upcase",
        );
    }

    #[test]
    fn only_text_and_name_are_writable() {
        refuses(
            &format!("{FEED}doc.root.foo = 1\n"),
            "no writable property 'foo'",
        );
        refuses(
            &format!("{FEED}doc.root.kind = \"x\"\n"),
            "no writable property 'kind'",
        );
        refuses(
            &format!("{FEED}doc.foo = 1\n"),
            "XmlDocument has no writable property",
        );
        refuses(
            "reader = Xml.reader(\"a.xml\", \"item\")\nreader.count = 1\n",
            "XmlReader has no writable property 'count'",
        );
        refuses(&format!("{FEED}doc.root.name = 5\n"), "String");
        accepts(&format!("{FEED}doc.root.text = 5\ndoc.root.name = \"b\"\n"));
    }
}
