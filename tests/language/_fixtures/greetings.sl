# Fixture for imports_spec.sl: a module reached along two import paths.

export def greet(name)
  "hello #{name}"
end

export class Registry
  static def label
    "registry"
  end
end
