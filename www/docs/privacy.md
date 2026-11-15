# Personal Data: Export and Erasure

Answer the two requests privacy law gives a person — "send me what you hold
about me" and "erase it" (GDPR articles 15, 17 and 20) — without hunting
through every table by hand.

```bash
soli generate auth       # a person is the auth scaffold's User
soli generate privacy
```

## What it generates

| File | |
|---|---|
| `app/services/privacy.sl` | The **register**: which models hold personal data, how each relates to a user, what an erasure does to it — and `Privacy.export_data` / `Privacy.erase` |
| `app/controllers/privacy_controller.sl` | `GET /privacy`, `POST /privacy/export`, `POST /privacy/erase` |
| `app/views/privacy/show.html.slv` | The page: download, erase |

No file of the auth scaffold is edited, and there is no migration.

## The register

List every model of yours that holds personal data in `PRIVACY_MODELS`:

```soli
const PRIVACY_MODELS = {
  "Comment": {"owner": "author_id", "erase": "delete"},
  "Order":   {"owner": "user_id", "erase": "anonymize", "fields": ["address", "phone"]},
  "Invoice": {"owner": "user_id", "erase": "keep"}
}
```

| Key | |
|---|---|
| `owner` | the field holding the user's key |
| `erase` | `"delete"` the records; `"anonymize"` them (set `fields` to `nil`); `"keep"` them — what the law makes you retain, such as invoices |
| `fields` | what `"anonymize"` blanks |
| `omit` | fields left out of an export |
| `export` | `false` to leave the model out of exports |

**A model that is not listed is neither exported nor erased.** A name that is
not a model raises, so a typo cannot leave data behind in silence. What the
other generators store about a user — OAuth and SAML identities, passkeys,
two-factor, API tokens, magic links — is already listed
(`PRIVACY_SCAFFOLD_MODELS`), deleted on erasure, exported without its secrets.

## Export

`/privacy` → *Download* returns one JSON file: the account, then each model's
records under its name. Credentials are left out (password digest, token
digests, the two-factor secret).

```soli
Privacy.export_data(user)
# {"exported_at": "…", "user": {…}, "Order": [{…}], "Comment": [{…}], …}
```

## Erasure

`/privacy` → *Erase my account* asks for the account's email and, when it has
one, its password. Then, model by model, by its rule; then the account.

```soli
report = Privacy.erase(user)
# {"deleted": {"Comment": 3, "ApiToken": 1}, "anonymized": {"Order": 2}, "kept": {"Invoice": 5}}
```

The account itself is kept as an empty shell (`PRIVACY_USER_ERASE =
"anonymize"`): its email becomes `erased-<key>@erased.invalid`, its name and
every credential are cleared, its sessions are closed and it can no longer
sign in — but whatever references it still resolves. Set it to `"delete"` to
remove the record instead.

The report is logged (`privacy: erased user …`), with counts and no values.

To answer a request made by mail, call the two methods from a script
(`soli db:seed . erase.sl`); the page takes the person's own session, not an
API token, and not an administrator acting as them.

## What erasure does not reach

Erasing works in bulk (`update_all`, `delete_all`), which **skips callbacks on
purpose**: an [audited](/docs/database/models#audit-log) model would otherwise
copy the very values being erased into its audit log. The consequences are
yours to cover:

- **Attachments** of erased records are not purged, and `dependent:` cascades
  do not fire. List the dependent models in the register, and purge files in
  `Privacy.erase_model` if you store some.
- **The audit log** keeps what it already recorded, old values included. A
  chained log cannot be edited without breaking the chain; decide a retention
  with `Audit.purge_before`, and withhold personal fields from it in the first
  place (`audit_redacted`).
- **Framework stores** are not touched: grouped errors (`/__soli/errors`),
  slow-query samples, the dev mail inbox, search indexes, logs, backups. They
  can hold request parameters; give them a retention, or keep personal data
  out of them.
- **Other systems** — a payment provider, a mailing tool — need their own
  erasure.

## Limits

- One file, written while the request waits: for accounts with a lot of data,
  build the export in a job and mail a link.
- JSON only; attachments are named in their records, not bundled.
- This is the mechanism, not the legal answer: what you must keep, and for how
  long, is yours to decide — the register is where it is written down.

## See also

- [Models → Audit Log](/docs/database/models#audit-log)
- [Authentication](/docs/security/authentication)
