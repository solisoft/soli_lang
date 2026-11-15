class Invoice < Model
  include Audited
  encrypts(:iban)

  static def audit_redacted
    ["secret_note"]
  end

  static def audit_ignored
    ["views"]
  end
end
