/**
 * Incan Language Extension for VS Code / Cursor
 * 
 * Provides:
 * - Syntax highlighting (via TextMate grammar)
 * - LSP integration for real-time diagnostics, hover, and go-to-definition
 * - Run/Check commands for Incan files
 */

import * as path from 'path';
import * as fs from 'fs';
import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;
let outputChannel: vscode.OutputChannel;

function findWorkspaceBinary(binaryName: string): string | undefined {
    const folders = vscode.workspace.workspaceFolders;
    if (!folders || folders.length === 0) {
        return undefined;
    }

    // Prefer debug while developing; fall back to release.
    const candidates = [
        path.join('target', 'debug', binaryName),
        path.join('target', 'release', binaryName),
    ];

    for (const folder of folders) {
        for (const rel of candidates) {
            const abs = path.join(folder.uri.fsPath, rel);
            if (fs.existsSync(abs)) {
                return abs;
            }
        }
    }
    return undefined;
}

function getCompilerPath(): string {
    const config = vscode.workspace.getConfiguration('incan');
    const compilerPath = config.get<string>('compiler.path', '');
    return compilerPath || 'incan';
}

function getFileToRun(uri?: vscode.Uri): string | undefined {
    // If URI provided (from explorer context menu), use it
    if (uri) {
        return uri.fsPath;
    }
    // Otherwise use active editor
    const editor = vscode.window.activeTextEditor;
    if (editor && (editor.document.languageId === 'incan' || 
                   editor.document.fileName.endsWith('.incn') ||
                   editor.document.fileName.endsWith('.incan'))) {
        return editor.document.fileName;
    }
    return undefined;
}

async function runIncanFile(uri?: vscode.Uri) {
    const filePath = getFileToRun(uri);
    if (!filePath) {
        vscode.window.showErrorMessage('No Incan file to run. Open an .incn file first.');
        return;
    }

    // Save the file before running
    const doc = vscode.workspace.textDocuments.find(d => d.fileName === filePath);
    if (doc?.isDirty) {
        await doc.save();
    }

    const compiler = getCompilerPath();
    const terminal = vscode.window.createTerminal({
        name: `Incan: ${path.basename(filePath)}`,
        cwd: path.dirname(filePath),
    });
    
    terminal.show();
    terminal.sendText(`${compiler} run "${filePath}"`);
}

async function checkIncanFile(uri?: vscode.Uri) {
    const filePath = getFileToRun(uri);
    if (!filePath) {
        vscode.window.showErrorMessage('No Incan file to check. Open an .incn file first.');
        return;
    }

    // Save the file before checking
    const doc = vscode.workspace.textDocuments.find(d => d.fileName === filePath);
    if (doc?.isDirty) {
        await doc.save();
    }

    const compiler = getCompilerPath();
    const terminal = vscode.window.createTerminal({
        name: `Incan Check: ${path.basename(filePath)}`,
        cwd: path.dirname(filePath),
    });
    
    terminal.show();
    terminal.sendText(`${compiler} "${filePath}"`);
}

export function activate(context: vscode.ExtensionContext) {
    outputChannel = vscode.window.createOutputChannel('Incan');
    
    // Register run/check commands
    context.subscriptions.push(
        vscode.commands.registerCommand('incan.runFile', runIncanFile),
        vscode.commands.registerCommand('incan.checkFile', checkIncanFile)
    );

    const config = vscode.workspace.getConfiguration('incan');
    const lspEnabled = config.get<boolean>('lsp.enabled', true);

    if (!lspEnabled) {
        console.log('Incan LSP is disabled');
        return;
    }

    // Get the path to incan-lsp
    let serverPath = config.get<string>('lsp.path', '');
    if (!serverPath) {
        // When working inside the compiler repo, prefer the workspace-built binary so
        // diagnostics match the checked-out language features (e.g. newly added syntax).
        serverPath = findWorkspaceBinary('incan-lsp') ?? 'incan-lsp';
    }

    // Server options - run the LSP binary
    const serverOptions: ServerOptions = {
        run: {
            command: serverPath,
            transport: TransportKind.stdio,
        },
        debug: {
            command: serverPath,
            transport: TransportKind.stdio,
        },
    };

    // Client options
    const clientOptions: LanguageClientOptions = {
        // Register for Incan files
        documentSelector: [
            { scheme: 'file', language: 'incan' },
            { scheme: 'untitled', language: 'incan' },
        ],
        synchronize: {
            // Watch .incn files for changes
            fileEvents: vscode.workspace.createFileSystemWatcher('**/*.incn'),
        },
    };

    // Create and start the client
    client = new LanguageClient(
        'incanLanguageServer',
        'Incan Language Server',
        serverOptions,
        clientOptions
    );

    // Start the client (also launches the server)
    client.start().then(() => {
        console.log('Incan Language Server started');
    }).catch((error) => {
        console.error('Failed to start Incan Language Server:', error);
        vscode.window.showWarningMessage(
            `Incan LSP failed to start. Make sure 'incan-lsp' is installed and in your PATH. ` +
            `You can also set the path in settings (incan.lsp.path).`
        );
    });

    // Register the client for disposal
    context.subscriptions.push({
        dispose: () => {
            if (client) {
                client.stop();
            }
        }
    });
}

export function deactivate(): Thenable<void> | undefined {
    if (!client) {
        return undefined;
    }
    return client.stop();
}












