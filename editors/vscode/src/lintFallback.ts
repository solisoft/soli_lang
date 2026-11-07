// `soli lint <file>` on open and save, for when the language server is not
// running. While the server runs this stays off: the server reports the same
// rules, path-aware ones included.

import { execFile } from "child_process";
import * as vscode from "vscode";
import { parseLintOutput } from "./lint";

const SOLI_LANGUAGES = new Set(["soli", "soli-template"]);

export class LintFallback implements vscode.Disposable {
  private readonly diagnostics = vscode.languages.createDiagnosticCollection("soli-lint");
  private listeners: vscode.Disposable[] = [];
  private active = false;
  /** Bumped by `disable`, so a lint still running then reports nothing. */
  private generation = 0;

  constructor(private readonly executable: () => string | undefined) {}

  get isActive(): boolean {
    return this.active;
  }

  /** Start linting open and saved Soli files. */
  enable(): void {
    if (this.active) {
      return;
    }
    this.active = true;
    this.listeners.push(
      vscode.workspace.onDidOpenTextDocument((document) => this.lint(document)),
      vscode.workspace.onDidSaveTextDocument((document) => {
        if (vscode.workspace.getConfiguration("soli.lint").get<boolean>("onSave", true)) {
          this.lint(document);
        }
      }),
      vscode.workspace.onDidCloseTextDocument((document) => this.diagnostics.delete(document.uri)),
    );
    for (const document of vscode.workspace.textDocuments) {
      this.lint(document);
    }
  }

  /** Stop, and drop what it reported. */
  disable(): void {
    this.active = false;
    this.generation++;
    for (const listener of this.listeners) {
      listener.dispose();
    }
    this.listeners = [];
    this.diagnostics.clear();
  }

  /** Lint one document now (also the `soli.lint` command). */
  lint(document: vscode.TextDocument): void {
    if (!SOLI_LANGUAGES.has(document.languageId) || document.uri.scheme !== "file") {
      return;
    }
    const executable = this.executable();
    if (!executable) {
      return;
    }
    const uri = document.uri;
    const generation = this.generation;
    execFile(executable, ["lint", uri.fsPath], { timeout: 10000 }, (_error, stdout, stderr) => {
      if (generation !== this.generation) {
        return;
      }
      const findings = parseLintOutput(`${stdout || ""}\n${stderr || ""}`);
      this.diagnostics.set(
        uri,
        findings.map((finding) => {
          const range = new vscode.Range(finding.line, finding.column, finding.line, finding.column + 1);
          const diagnostic = new vscode.Diagnostic(range, finding.message, vscode.DiagnosticSeverity.Warning);
          diagnostic.source = "soli lint";
          diagnostic.code = finding.rule;
          return diagnostic;
        }),
      );
    });
  }

  dispose(): void {
    this.disable();
    this.diagnostics.dispose();
  }
}
