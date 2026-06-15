def merge_with_default(base, override, default_value) {
    let out = {};
    for key in base.keys {
        out[key] = base[key];
    }
    for key in override.keys {
        let v = override[key];
        out[key] = v ?? default_value;
    }
    return out;
}
