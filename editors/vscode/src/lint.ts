// Pure helpers for the fallback lint and the server's failure modes. No
// `vscode` import, so the unit tests run under plain Node.

export interface LintFinding {
  /** 0-based. */
  line: number;
  /** 0-based. */
  column: number;
  rule: string;
  message: string;
}

/** `soli lint <file>` prints `file:line:col - [rule] message` per finding
 * (1-based), followed by code frames and a summary that are skipped here. */
export function parseLintOutput(output: string): LintFinding[] {
  const pattern = /^.+?:(\d+):(\d+) - \[([^\]]+)\] (.+)$/;
  const findings: LintFinding[] = [];
  for (const raw of output.split(/\r?\n/)) {
    const match = raw.match(pattern);
    if (!match) {
      continue;
    }
    findings.push({
      line: Math.max(0, parseInt(match[1], 10) - 1),
      column: Math.max(0, parseInt(match[2], 10) - 1),
      rule: match[3],
      message: match[4],
    });
  }
  return findings;
}

/** What `soli lsp` prints before exiting 1 when the binary was built without
 * the `lsp` cargo feature (src/cli/commands/mod.rs). */
export const NO_LSP_FEATURE_MARKER = "built without the `lsp` feature";

/** Whether a `soli lsp` that exited said it has no language server — a
 * build without the feature, which no restart will fix. `exitCode` is null
 * while the exit status is not known yet. */
export function lacksLanguageServer(exitCode: number | null, stderr: string): boolean {
  return exitCode !== 0 && stderr.includes(NO_LSP_FEATURE_MARKER);
}

export interface FallbackInputs {
  /** `soli.lint.enable`. */
  lintEnabled: boolean;
  /** The language server is running (or starting): it publishes the
   * diagnostics itself. */
  serverActive: boolean;
}

/** The fallback lint runs only when the language server does not: otherwise
 * every finding would be reported twice. */
export function fallbackLintActive(inputs: FallbackInputs): boolean {
  return inputs.lintEnabled && !inputs.serverActive;
}
