// ============================================================================
// Crypto / IDs / Files Offline Coverage Test Suite
// ============================================================================
// Covers currently-untested offline builtins: Crypto.ledger_hash /
// merkle_root, x25519 globals, RsaKey.public_from_pem, X509.spki_pin,
// UUID/ULID/NanoID generators, File/Trusted glob+modified, file write
// helpers, Logger.warn/clear_entries, Factory.create_list/sequence,
// Expectation.to_match, string globals, puts, dotenv!, and the comparison
// assertions (including failure paths).
//
// Note: `cache` (SoliKV-backed) and env-var mutation via `dotenv!` are NOT
// offline-coverable — see the dotenv! test below for why.
// ============================================================================

const RSA_KEY_PEM = "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCca/ZMr23wsIM9\nLY5nPcV4ViQxdQ30Ca65FZMCZXQYQy+edBMeXeVF34DzVKSZUkyAKZ93U6ShZVkb\nkwvqFJP1jYrzMU+ov2eempYg0N9WDG02NaMw65ZVaKHbR5N4axHHxGLpdZbwVPPl\nsAIm7A1YMk4fF4gtneED4jjXtCVYwzTkB2HWegAYqpSkuzeyp4F/6LFCPOQ5VDqg\nZRNSXCZVQC42Dmt0QMNICqyKjcfTB/IAS1DtRYwqBfADNPS+OGnARd5iU60gBCCL\nBD38G3ZDAI/P3HsonXpfo7w+6sMn2B8V8VPvd9O32p5PB+NrmIcIJpocipIngM0L\nOPLaC8eNAgMBAAECggEAKAb+i4gWz5EzvDuEpcmqVw1gDKHiFLFHm0g4itPwXecP\nb/JPFCW97l/vzRS7XBqxxdgg3PWz+rMHFuXNljR22k7CoFHdixaTywPO6A3bINdk\nOQuHu5SFr0xrosPRqm5nqeGI2CoFmnF6yit8mX4tOgUBdbZdXCL6+jXxCs2oAurm\ne4hfNmrxoG599cpl58p8Yg+JPrsiPWQxeRU7tVNRGUbovT3jYYDsOpcI4qlNZIcd\nfcZHIkI1/5J5B08cWcyc88xIEFUrHFoe01TMXTrt3GosmF+1VSyZGv1P2y7CX7j6\nUqxYyom0nK7eWXI+XbpqIAmLCykMIqofs3lyRyXIJwKBgQDSLtGpcw4XbTVhMjF8\nxjQk2TaI5cky+hr9y19mfHdY/xld9k0aiT8CbNTUhlyFf8ZapwussBhdTBiOKYtQ\nPWyzIyjq99pedep9Ze4RuhFNCMfdogZcj6RT1Bi1tNtedKaCIJCNo/RdmH3t8798\n2lW7KMDC6bQd5pE+bmyvT8fnowKBgQC+hQVRtGBkDNqL2YpC7lZm2O8XKUFbV+Z7\nJZhEZGYaxiCN+1fywB+EL41xGuVmOxlxuOrQ33ugHtlt8YEarX3vUQlxxPXsU+Fm\nECgTXEiAYn9TmNTB/q075Ed5HC5+poyRfgTpNSKeXMvDcMY9Xi4nIIAC1IefJRDU\nOAwc8P1HDwKBgQCBXhPqakjYHn3mj1BqbkyWCaRJarYGTG7km5Lir+V9v7ZLYVhf\n5u4Dfh0ZmoHEIbti/MJwzgqREk9i4StAfi4zrIZ46Ylc7tMfz+dSveX8NlVek2W6\n/yaz+i4jWWhUoRQDsCuJIss7+Ko6Ffdcz75I7nKHBfW5Gbt4Y9s9pKt0ZQKBgEwn\n9iFb3fAAZ1fhxG/Ov8Dq1F/IwPRXZa0yMPSdwWbQbfDzWIuTmsWHEJ32p14/H4Oi\n7FJEEzHFQxq8n+PfF+kS1pigp8EpIn9e0/YxPFX9iXIMNHe7atn2/U7/IeLEhood\n+q6R692rsFPWf5fGTuKbDjCTbgcClQCPyt/CwSunAoGBAM/VB/icg3q/pyRERic2\nXKTomPi/5gruQgSlrHw93k1XObk+jOt1Xba6uDhUiXRxWwRz7lqpZ2l6NflbngNp\nY/0tUksNJ0PRhV3NLdVKsZdE/1fl4kbxNE5VMCUn1IHBFFHSaM4Rk5vfAYBLwAyW\np1uYKl5VM/9SLSt39wJM9nqx\n-----END PRIVATE KEY-----\n";

// SPKI public half of RSA_KEY_PEM — what RsaKey.public_from_pem parses.
const RSA_PUBLIC_PEM = "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAnGv2TK9t8LCDPS2OZz3F\neFYkMXUN9AmuuRWTAmV0GEMvnnQTHl3lRd+A81SkmVJMgCmfd1OkoWVZG5ML6hST\n9Y2K8zFPqL9nnpqWINDfVgxtNjWjMOuWVWih20eTeGsRx8Ri6XWW8FTz5bACJuwN\nWDJOHxeILZ3hA+I417QlWMM05Adh1noAGKqUpLs3sqeBf+ixQjzkOVQ6oGUTUlwm\nVUAuNg5rdEDDSAqsio3H0wfyAEtQ7UWMKgXwAzT0vjhpwEXeYlOtIAQgiwQ9/Bt2\nQwCPz9x7KJ16X6O8PurDJ9gfFfFT73fTt9qeTwfja5iHCCaaHIqSJ4DNCzjy2gvH\njQIDAQAB\n-----END PUBLIC KEY-----\n";

// A *different* certificate (different key) from xml_dsig_sign_spec's, used
// to prove spki_pin tracks the key material.
const CERT_PEM_A = "-----BEGIN CERTIFICATE-----\nMIICqzCCAZOgAwIBAgIBATANBgkqhkiG9w0BAQsFADAZMRcwFQYDVQQDDA5zcC5l\neGFtcGxlLmNvbTAeFw0yMDAxMDEwMDAwMDBaFw0zNTAxMDEwMDAwMDBaMBkxFzAV\nBgNVBAMMDnNwLmV4YW1wbGUuY29tMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIB\nCgKCAQEAtAJl23hnRhGvpJ5QQ4haLfWK3mjd8Slf8fnhUWD/Qzoo3I1bsvcr8HAj\nkpaIKne5aLU1jp/migvcdgDx33JAck23I7pz9Yq47sKA1/KP+TFxmzOAA08M34xZ\nO4BPD1VWDBNZyfwtX5FA8eQiuBI0UIkLksulLb9wPjqd28vDAFVldGvlXMQRVZEW\nn0Y7xqC2gGCNAM2y7N3oDNPvAIuIstFNGExh+bP/J7PDZeTIhR2q1FsGoeZwGMO1\nU8EzJQj+D2ZahfF2aKXtHOo511EQsmXEx/DhpoKrgvClv9/jZpYxxSmUocsYCiEQ\ngRi/0YadfTJzLAUGGgMNRGgfzhfiqwIDAQABMA0GCSqGSIb3DQEBCwUAA4IBAQCp\n03SxaAvxD1c+pMg4Q3YTPFUe5eFRvYxaXG8BdDCH9P+uD+TVWstPr5Rx7pWDGsuV\n9QlyNmA3bFea4Ps8n7CiEuiJeDzbtTznOBHbF5/AUj7fhNHu9Su0Ka4Fg5QCuRGZ\nB6Z6fkJDkZ0NVRJXwqXgOByvm7i0VE0mtFaf1kyqApPV2IohF/CxfqsMz/dySWPl\nODWt3qmRBU3Wk5wUtD+71Opmb+qfXZoqFuKoY1MHSf14rXcV/tETLyhXp8oaA/OM\nb9onERfWbd8xx//ct4TFLUo64uvsyXtFhnAcjVec/qOMUzZ/OmzVW8caCThFIhnt\n2StVAKOJevnZUkGGHuyN\n-----END CERTIFICATE-----\n";

const CERT_DER_B64_B = "MIICqzCCAZOgAwIBAgIBATANBgkqhkiG9w0BAQsFADAZMRcwFQYDVQQDDA5zcC5leGFtcGxlLmNvbTAeFw0yMDAxMDEwMDAwMDBaFw0zNTAxMDEwMDAwMDBaMBkxFzAVBgNVBAMMDnNwLmV4YW1wbGUuY29tMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAnGv2TK9t8LCDPS2OZz3FeFYkMXUN9AmuuRWTAmV0GEMvnnQTHl3lRd+A81SkmVJMgCmfd1OkoWVZG5ML6hST9Y2K8zFPqL9nnpqWINDfVgxtNjWjMOuWVWih20eTeGsRx8Ri6XWW8FTz5bACJuwNWDJOHxeILZ3hA+I417QlWMM05Adh1noAGKqUpLs3sqeBf+ixQjzkOVQ6oGUTUlwmVUAuNg5rdEDDSAqsio3H0wfyAEtQ7UWMKgXwAzT0vjhpwEXeYlOtIAQgiwQ9/Bt2QwCPz9x7KJ16X6O8PurDJ9gfFfFT73fTt9qeTwfja5iHCCaaHIqSJ4DNCzjy2gvHjQIDAQABMA0GCSqGSIb3DQEBCwUAA4IBAQBUHvn9HJEMkw4NZ95aSbmNAF+uMoCOCuj9NotJrmbUFdjLpfSvWpx4lSI6ZoUdXa7QyN/Wi7gBJgSl1ex6axSAD3Al5fEZqBuEqsMMncRGf2y463MGDgUYutzWPP4KDomdDVMzogpz0EuG2HGrS09YWy34KiBJ9bP48TaWn5iosauGFzv+zE2fTF/YK6sxbOTOgU8mTteXwbHwYZSBrCLUj/dcAbrNCrE8IezxmUJR6W/byABg+tCSW1cVhjH9BrBp6uFgXESMY+tnUJeZcMTf+24hadsr0X4ivV1BXE20O2lZblesg6oxjvjnSsBNH2BDdZYMcX+4eEI3MN83Tesw";

fn fresh_tmp_dir() {
    return "/tmp/soli_cif_spec_" + uuid_v4();
}

fn write_glob_fixture(directory) {
    mkdir_p(directory);
    mkdir_p(directory + "/logs");
    file_write_bytes(directory + "/alpha.txt", [104, 101, 108, 108, 111]);
    file_write_base64(directory + "/data.bin", "AAECAw==");
    barf(directory + "/logs/deep.txt", "deep content");
}

fn cleanup_glob_fixture(directory) {
    try {
        File.delete(directory + "/alpha.txt");
        File.delete(directory + "/data.bin");
        File.delete(directory + "/logs/deep.txt");
    } catch (e) {
        // Best-effort cleanup; leftover /tmp files must not fail the suite.
    }
}

describe("Crypto ledger_hash", fn() {
    test("ledger_hash() equals sha256 over prev:seq:canonical_json", fn() {
        let data = {"b": 2, "a": 1};
        let ledger_digest = Crypto.ledger_hash("genesis", 1, data);
        let expected = sha256("genesis:1:" + "{\"a\":1,\"b\":2}");
        assert_eq(ledger_digest, expected);
    });

    test("ledger_hash() is deterministic and seq-sensitive", fn() {
        let data = {"amount": 10};
        let first = Crypto.ledger_hash("prev", 1, data);
        let again = Crypto.ledger_hash("prev", 1, data);
        let bumped = Crypto.ledger_hash("prev", 2, data);
        assert_eq(first, again);
        assert_ne(first, bumped);

        let other_prev = Crypto.ledger_hash("genesis", 1, data);
        assert_ne(first, other_prev);
    });
});

describe("Crypto merkle_root", fn() {
    test("merkle_root([]) is sha256 of the empty string", fn() {
        assert_eq(Crypto.merkle_root([]), sha256(""));
    });

    test("a single leaf is its own root", fn() {
        let leaf = sha256("only leaf");
        assert_eq(Crypto.merkle_root([leaf]), leaf);
    });

    test("two leaves combine pairwise as sha256(left_hex + right_hex)", fn() {
        let leaf_a = sha256("a");
        let leaf_b = sha256("b");
        let expected = sha256(leaf_a + leaf_b);
        assert_eq(Crypto.merkle_root([leaf_a, leaf_b]), expected);
    });

    test("an odd node pairs with itself (Bitcoin convention)", fn() {
        let leaf_a = sha256("a");
        let leaf_b = sha256("b");
        let leaf_c = sha256("c");
        let expected = sha256(sha256(leaf_a + leaf_b) + sha256(leaf_c + leaf_c));
        assert_eq(Crypto.merkle_root([leaf_a, leaf_b, leaf_c]), expected);
    });

    test("merkle_root() changes when any leaf changes", fn() {
        let leaves = [sha256("tx1"), sha256("tx2"), sha256("tx3")];
        let tampered = [sha256("tx1"), sha256("EVIL"), sha256("tx3")];
        assert_ne(Crypto.merkle_root(leaves), Crypto.merkle_root(tampered));
    });
});

describe("x25519 standalone globals", fn() {
    test("x25519_keypair() yields 32-byte hex keys", fn() {
        let keypair = x25519_keypair();
        assert_eq(len(keypair["private"]), 64);
        assert_eq(len(keypair["public"]), 64);
        assert(keypair["private"] != keypair["public"]);
    });

    test("x25519_public_key() derives the pair's public half", fn() {
        let keypair = x25519_keypair();
        assert_eq(x25519_public_key(keypair["private"]), keypair["public"]);
    });

    test("x25519_shared_secret() agrees both directions", fn() {
        let alice = x25519_keypair();
        let bob = x25519_keypair();
        let alice_side = x25519_shared_secret(alice["private"], bob["public"]);
        let bob_side = x25519_shared_secret(bob["private"], alice["public"]);
        assert_eq(alice_side, bob_side);
        assert_eq(len(alice_side), 64);
    });
});

describe("RsaKey.public_from_pem and X509.spki_pin", fn() {
    test("public_from_pem() extracts the same modulus/exponent as private_from_pem()", fn() {
        let private_key = RsaKey.private_from_pem(RSA_KEY_PEM);
        let public_key = RsaKey.public_from_pem(RSA_PUBLIC_PEM);
        assert_eq(public_key["algorithm"], "RSA");
        assert_eq(public_key["bits"], 2048);
        assert_eq(public_key["n"], private_key["n"]);
        assert_eq(public_key["e"], private_key["e"]);
    });

    test("spki_pin() returns a sha256 pin that is stable per certificate", fn() {
        let pin_one = X509.spki_pin(CERT_PEM_A);
        let pin_two = X509.spki_pin(CERT_PEM_A);
        assert(pin_one.starts_with("sha256/"));
        assert_eq(pin_one, pin_two);
    });

    test("spki_pin() tracks the key material, not the encoding", fn() {
        // CERT_DER_B64_B carries a different public key than CERT_PEM_A;
        // its pin must differ even though it is fed in as raw DER base64.
        let pin_a = X509.spki_pin(CERT_PEM_A);
        let pin_b = X509.spki_pin(CERT_DER_B64_B);
        assert(pin_b.starts_with("sha256/"));
        assert_ne(pin_a, pin_b);
    });
});

describe("UUID generation", fn() {
    test("uuid_v4() returns a distinct hyphenated v4 UUID", fn() {
        let first = uuid_v4();
        let second = uuid_v4();
        assert_eq(len(first), 36);
        assert_ne(first, second);
        assert_match(first, "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$");
    });

    test("uuid_v7() returns a time-ordered v7 UUID", fn() {
        let value = uuid_v7();
        assert_eq(len(value), 36);
        assert_match(value, "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$");
    });

    test("UUID.v4()/UUID.v7() statics behave like the globals", fn() {
        let v4_value = UUID.v4();
        let v7_value = UUID.v7();
        assert_eq(len(v4_value), 36);
        assert_eq(len(v7_value), 36);
        assert_match(v4_value, "-4[0-9a-f]{3}-");
        assert_match(v7_value, "-7[0-9a-f]{3}-");
        assert_ne(UUID.v4(), v4_value);
    });
});

describe("ULID and NanoID generation", fn() {
    test("ulid() returns a 26-char Crockford Base32 identifier", fn() {
        let first = ulid();
        let second = ulid();
        assert_eq(len(first), 26);
        assert_match(first, "^[0-9ABCDEFGHJKMNPQRSTVWXYZ]{26}$");
        assert_ne(first, second);
    });

    test("nanoid() defaults to 21 URL-safe chars and honors size/alphabet", fn() {
        let default_id = nanoid();
        assert_eq(len(default_id), 21);
        assert_match(default_id, "^[A-Za-z0-9_-]+$");

        let sized = nanoid(10);
        assert_eq(len(sized), 10);

        let custom = nanoid(8, "abc");
        assert_eq(len(custom), 8);
        let chars_ok = true;
        for ch in custom.split("") {
            if ch != "" && !contains("abc", ch) {
                chars_ok = false;
            }
        }
        assert(chars_ok);
    });
});

describe("Filesystem write helpers and existence", fn() {
    test("file_write_bytes() writes raw bytes and file_exists() sees them", fn() {
        let directory = fresh_tmp_dir();
        mkdir_p(directory);
        let path = directory + "/bytes.bin";

        assert(file_exists(path) == false);
        assert_eq(file_write_bytes(path, [0, 1, 2, 255]), true);
        assert(file_exists(path));

        let read_back = slurp(path, "binary");
        assert_eq(read_back, [0, 1, 2, 255]);
        File.delete(path);
    });

    test("file_write_base64() decodes and writes binary content", fn() {
        let directory = fresh_tmp_dir();
        mkdir_p(directory);
        let path = directory + "/decoded.bin";

        // base64([0,1,2,3]) == "AAECAw=="
        assert_eq(file_write_base64(path, "AAECAw=="), true);
        assert(file_exists(path));
        assert_eq(slurp(path, "binary"), [0, 1, 2, 3]);
        File.delete(path);
    });
});

describe("File globbing and modification time", fn() {
    test("File.glob() lists matching basenames in a directory", fn() {
        let directory = fresh_tmp_dir();
        write_glob_fixture(directory);

        let txt_files = File.glob(directory + "/*.txt");
        assert_eq(len(txt_files), 1);
        assert(txt_files[0].ends_with("/alpha.txt"));

        let all_entries = File.glob(directory + "/*");
        assert_eq(len(all_entries), 3); // alpha.txt, data.bin, logs/

        cleanup_glob_fixture(directory);
    });

    test("File.glob() misses non-matching patterns", fn() {
        let directory = fresh_tmp_dir();
        write_glob_fixture(directory);

        assert_eq(len(File.glob(directory + "/*.md")), 0);
        assert_eq(len(File.glob(directory + "/*.bin")), 1);

        cleanup_glob_fixture(directory);
    });

    test("File.glob_recursive() descends into subdirectories", fn() {
        let directory = fresh_tmp_dir();
        write_glob_fixture(directory);

        // Unlike File.glob, the recursive form walks the whole subtree: the
        // nested logs/deep.txt is matched alongside the top-level alpha.txt.
        let txt_matches = File.glob_recursive(directory + "/*.txt");
        assert_eq(len(txt_matches), 2);

        let has_top = txt_matches[0].ends_with("/alpha.txt") || txt_matches[1].ends_with("/alpha.txt");
        let has_deep = txt_matches[0].ends_with("/logs/deep.txt") || txt_matches[1].ends_with("/logs/deep.txt");
        assert(has_top);
        assert(has_deep);

        cleanup_glob_fixture(directory);
    });

    test("File.modified() returns a plausible epoch-seconds timestamp", fn() {
        let directory = fresh_tmp_dir();
        write_glob_fixture(directory);

        let modified_at = File.modified(directory + "/alpha.txt");
        assert_gt(modified_at, 1600000000);
        assert_lt(modified_at, 4102444800); // before 2100-01-01

        cleanup_glob_fixture(directory);
    });

    test("Trusted.glob() mirrors File.glob()", fn() {
        let directory = fresh_tmp_dir();
        write_glob_fixture(directory);

        let trusted_txt = Trusted.glob(directory + "/*.txt");
        assert_eq(len(trusted_txt), 1);
        assert(trusted_txt[0].ends_with("/alpha.txt"));

        cleanup_glob_fixture(directory);
    });

    test("Trusted.glob_recursive() finds nested files", fn() {
        let directory = fresh_tmp_dir();
        write_glob_fixture(directory);

        let deep_matches = Trusted.glob_recursive(directory + "/*.txt");
        assert_eq(len(deep_matches), 2);
        let has_deep = deep_matches[0].ends_with("/logs/deep.txt") || deep_matches[1].ends_with("/logs/deep.txt");
        assert(has_deep);

        cleanup_glob_fixture(directory);
    });

    test("Trusted.modified() reports the mtime", fn() {
        let directory = fresh_tmp_dir();
        write_glob_fixture(directory);

        let trusted_modified = Trusted.modified(directory + "/logs/deep.txt");
        assert_gt(trusted_modified, 1600000000);
        assert_eq(trusted_modified, File.modified(directory + "/logs/deep.txt"));

        cleanup_glob_fixture(directory);
    });
});

describe("Logger warn and capture buffer", fn() {
    test("Logger.warn() records a WARN entry; clear_entries() empties the buffer", fn() {
        Logger.set_capture(true);
        Logger.clear_entries();

        Logger.warn("disk almost full", {"used_pct": 91});
        let entries = Logger.entries();
        assert_eq(len(entries), 1);
        assert(entries[0].contains("[WARN]"));
        assert(entries[0].contains("disk almost full"));
        assert(entries[0].contains("used_pct=91"));

        Logger.clear_entries();
        assert_eq(len(Logger.entries()), 0);

        Logger.set_capture(false);
        Logger.clear_entries();
    });

    test("Logger.warn() below the configured level is dropped", fn() {
        Logger.set_capture(true);
        Logger.clear_entries();
        Logger.configure({"level": "error"});

        Logger.warn("should not be captured");
        assert_eq(len(Logger.entries()), 0);

        // Restore defaults so later suites keep their normal level.
        Logger.configure({"level": "info"});
        Logger.set_capture(false);
        Logger.clear_entries();
    });
});

describe("Factory DSL", fn() {
    test("Factory.create_list() builds N interpolated attributes", fn() {
        // Assemble the "#{n}" placeholder by concatenation: the lexer would
        // otherwise try to interpolate `n` as a language variable.
        let sequence_token = "#" + "{n}";
        Factory.define("cif_user", {"email": "user" + sequence_token + "@example.com", "role": "member"});
        let users = Factory.create_list("cif_user", 3);
        assert_eq(len(users), 3);
        assert_eq(users[0]["email"], "user0@example.com");
        assert_eq(users[1]["email"], "user1@example.com");
        assert_eq(users[2]["email"], "user2@example.com");
        assert_eq(users[2]["role"], "member");
    });

    test("Factory.create_list() of zero yields an empty array", fn() {
        Factory.define("cif_empty", {"name": "nobody"});
        assert_eq(Factory.create_list("cif_empty", 0), []);
    });

    test("Factory.sequence() advances by one per name", fn() {
        let first = Factory.sequence("cif_seq");
        let second = Factory.sequence("cif_seq");
        let other_name = Factory.sequence("cif_other_seq");
        assert_eq(second, first + 1);
        assert_eq(other_name, Factory.sequence("cif_other_seq") - 1);
        Factory.clear();
    });
});

describe("Expectation.to_match", fn() {
    test("expect(...).to_match(...) passes on substring match", fn() {
        expect("hello world").to_match("world");
        expect(uuid_v4()).to_match("-");
    });

    test("expect(...).to_match(...) throws on mismatch", fn() {
        let threw = false;
        try {
            expect("hello").to_match("zzz-not-there");
        } catch (e) {
            threw = true;
        }
        assert(threw);
    });
});

describe("Standalone string globals and puts", fn() {
    test("contains() checks substrings", fn() {
        assert(contains("hello world", "lo wo"));
        assert_not(contains("hello world", "goodbye"));
    });

    test("starts_with() and ends_with() check affixes", fn() {
        assert(starts_with("soli.txt", "soli"));
        assert_not(starts_with("soli.txt", ".txt"));
        assert(ends_with("soli.txt", ".txt"));
        assert_not(ends_with("soli.txt", "soli"));
    });

    test("replace() substitutes every occurrence", fn() {
        assert_eq(replace("a-b-c", "-", "+"), "a+b+c");
        assert_eq(replace("aaa", "aa", "b"), "ba");
        assert_eq(replace("unchanged", "zzz", "y"), "unchanged");
    });

    test("puts() writes a line and returns null", fn() {
        assert_null(puts("crypto_ids_files_spec puts probe"));
    });
});

describe("dotenv!", fn() {
    test("dotenv!() is removed and raises the SEC-033 migration error", fn() {
        let threw = false;
        let message = "";
        try {
            dotenv!("tests/fixtures/.env.test");
        } catch (e) {
            threw = true;
            message = str(e);
        }
        assert(threw);
        assert(message.contains("SEC-033"));
        assert(message.contains(".env"));
    });
});

describe("Comparison and match assertions", fn() {
    test("assert_gt/assert_lt/assert_ne/assert_match pass on valid input", fn() {
        assert_gt(5, 4);
        assert_lt(4, 5);
        assert_ne("one", "two");
        assert_match("hello.sol", "\\.sol$");
    });

    test("assert_gt() throws when the first value is not greater", fn() {
        let threw = false;
        try {
            assert_gt(3, 3);
        } catch (e) {
            threw = true;
        }
        assert(threw);
    });

    test("assert_lt() throws when the first value is not smaller", fn() {
        let threw = false;
        try {
            assert_lt(5, 4);
        } catch (e) {
            threw = true;
        }
        assert(threw);
    });

    test("assert_ne() throws on equal values", fn() {
        let threw = false;
        try {
            assert_ne(1, 1);
        } catch (e) {
            threw = true;
        }
        assert(threw);
    });

    test("assert_match() throws when the pattern does not match", fn() {
        let threw = false;
        try {
            assert_match("hello", "^\\d+$");
        } catch (e) {
            threw = true;
        }
        assert(threw);
    });
});
