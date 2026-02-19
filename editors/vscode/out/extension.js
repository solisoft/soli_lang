const vscode = require("vscode");
const { execFile } = require("child_process");
const path = require("path");

/** @type {vscode.DiagnosticCollection} */
let diagnosticCollection;

/** @param {vscode.ExtensionContext} context */
function activate(context) {
  diagnosticCollection =
    vscode.languages.createDiagnosticCollection("soli-lint");
  context.subscriptions.push(diagnosticCollection);

  // Lint on save
  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument((document) => {
      if (document.languageId !== "soli") return;
      const config = vscode.workspace.getConfiguration("soli.lint");
      if (config.get("enable") && config.get("onSave")) {
        lintDocument(document);
      }
    })
  );

  // Lint on open
  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument((document) => {
      if (document.languageId !== "soli") return;
      const config = vscode.workspace.getConfiguration("soli.lint");
      if (config.get("enable")) {
        lintDocument(document);
      }
    })
  );

  // Clear diagnostics when file is closed
  context.subscriptions.push(
    vscode.workspace.onDidCloseTextDocument((document) => {
      diagnosticCollection.delete(document.uri);
    })
  );

  // Manual lint command
  context.subscriptions.push(
    vscode.commands.registerCommand("soli.lint", () => {
      const editor = vscode.window.activeTextEditor;
      if (editor && editor.document.languageId === "soli") {
        lintDocument(editor.document);
      }
    })
  );

  // Lint already-open .sl files
  vscode.workspace.textDocuments.forEach((document) => {
    if (document.languageId === "soli") {
      const config = vscode.workspace.getConfiguration("soli.lint");
      if (config.get("enable")) {
        lintDocument(document);
      }
    }
  });
}

/**
 * Run `soli lint` on a document and update diagnostics.
 * @param {vscode.TextDocument} document
 */
function lintDocument(document) {
  const filePath = document.uri.fsPath;
  if (!filePath) return;

  const config = vscode.workspace.getConfiguration("soli.lint");
  const executable = config.get("executablePath") || "soli";

  execFile(executable, ["lint", filePath], { timeout: 10000 }, (err, stdout, stderr) => {
    const diagnostics = [];

    // soli lint outputs to stdout; parse each line
    // Format: file:line:column - [rule] message
    const pattern = /^.+?:(\d+):(\d+) - \[(.+?)\] (.+)$/;
    const output = (stdout || "") + (stderr || "");

    for (const line of output.split("\n")) {
      const match = line.match(pattern);
      if (!match) continue;

      const lineNum = parseInt(match[1], 10) - 1; // VS Code is 0-indexed
      const colNum = parseInt(match[2], 10) - 1;
      const rule = match[3];
      const message = match[4];

      const range = new vscode.Range(lineNum, colNum, lineNum, colNum + 1);
      const diagnostic = new vscode.Diagnostic(
        range,
        `${message} [${rule}]`,
        vscode.DiagnosticSeverity.Warning
      );
      diagnostic.source = "soli lint";
      diagnostic.code = rule;
      diagnostics.push(diagnostic);
    }

    diagnosticCollection.set(document.uri, diagnostics);
  });
}

function deactivate() {
  if (diagnosticCollection) {
    diagnosticCollection.dispose();
  }
}

module.exports = { activate, deactivate };
