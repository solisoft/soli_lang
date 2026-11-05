//! A model instance read or written from bytecode obeys the model rules the
//! tree-walker applies: a declared relation and a translated field are
//! resolved by `model_instance_member`, and a write goes through
//! `set_instance_field`. The VM's field fast paths handed back the stored
//! value and wrote it in place, so on `--vm` a preloaded relation stayed a
//! Hash (its JSON kept `password_digest`), a translated field read and
//! overwrote the default language, and `post._key = …` was accepted.
//! No database is involved: the instances are built with `Model.new`.

use std::process::Command;

fn run(engine: &str, source: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("s.sl"), source).expect("write");
    let output = Command::new(env!("CARGO_BIN_EXE_soli"))
        .current_dir(dir.path())
        .args([engine, "s.sl"])
        .env_remove("SOLI_ENGINE")
        .output()
        .expect("run soli");
    assert!(
        output.status.success(),
        "{engine}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Both engines print `expected`.
fn assert_both(source: &str, expected: &str) {
    for engine in ["--tree", "--vm"] {
        assert_eq!(run(engine, source), expected, "engine {engine}");
    }
}

#[test]
fn a_preloaded_relations_json_hides_sensitive_fields() {
    // The model classes of a script are compiled by the VM here, which does
    // not register them for relation hydration (`soli serve` loads models on
    // the tree-walker, which does): the class the relation reads as is
    // checked in `vm_classes::field_write_rules_tests`.
    assert_both(
        r##"class User < Model
end
class Post < Model
  def author_json
    JSON.stringify({"author": @user})
  end
end
Post.belongs_to("user")
def author(post)
  post.user
end
post = Post.new({"title": "Hello", "user": {"_key": "u1", "name": "Ann", "password_digest": "SECRETHASH"}})
print(author(post).name)
print(JSON.stringify({"author": author(post)}).includes?("SECRETHASH"))
print(post.author_json.includes?("SECRETHASH"))
fresh = Post.new({"title": "Hello", "user": {"_key": "u1", "name": "Ann", "password_digest": "SECRETHASH"}})
print(JSON.stringify({"author": fresh.user}).includes?("SECRETHASH"))
"##,
        "Ann\nfalse\nfalse\nfalse\n",
    );
}

#[test]
fn a_translated_field_reads_and_stages_in_the_current_locale() {
    assert_both(
        r##"class Doc < Model
  def retitle(title)
    @title = title
  end
end
Doc.translate("title")
def show(doc)
  doc.title
end
doc = Doc.new({"title": "Hello", "translated_fields": {"title": {"fr": "Bonjour"}}})
I18n.set_locale("fr")
print(doc.title, show(doc))
fresh = Doc.new({"title": "Hello"})
fresh.title = "Salut"
fresh.retitle("Coucou")
I18n.set_locale("en")
print(show(fresh), fresh._pending_translations)
"##,
        "Bonjour Bonjour\nHello {title => Coucou}\n",
    );
}

#[test]
fn a_models_metadata_is_read_only() {
    assert_both(
        r##"class Post < Model
  def rekey
    @_key = "victim"
  end
end
def forge(post, key)
  post._key = key
end
post = Post.new({"title": "Hello"})
try
  forge(post, "victim-key")
  print("forged")
catch e
  print("refused")
end
try
  post.rekey
  print("forged")
catch e
  print("refused")
end
post._errors = ["kept"]
print(post._key, post._errors)
"##,
        "refused\nrefused\nnull [kept]\n",
    );
}
