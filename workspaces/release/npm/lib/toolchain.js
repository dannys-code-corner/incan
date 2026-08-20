"use strict";

const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");

function packageRoot() {
  return path.resolve(__dirname, "..");
}

function toolchainHome() {
  return process.env.INCAN_NPM_TOOLCHAIN_HOME || path.join(packageRoot(), ".incan", "home");
}

function binDir() {
  return process.env.INCAN_NPM_BIN_DIR || path.join(packageRoot(), ".incan", "bin");
}

function packageVersion() {
  const packageJson = JSON.parse(fs.readFileSync(path.join(packageRoot(), "package.json"), "utf8"));
  return packageJson.version;
}

function packageManifestUrl() {
  const release = `v${packageVersion()}`;
  return `https://github.com/encero-systems/incan/releases/download/${release}/manifest.json`;
}

function installerScript() {
  const candidates = [
    path.join(packageRoot(), "vendor", "install-incan.sh"),
    path.resolve(packageRoot(), "..", "install-incan.sh"),
  ];
  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }
  throw new Error("could not find bundled install-incan.sh");
}

function hasValueOption(args, name) {
  return args.includes(name) || args.some((arg) => arg.startsWith(`${name}=`));
}

function installerArgs(args) {
  const next = args.filter((arg) => arg !== "--package-install");
  if (!hasValueOption(next, "--manifest") && !process.env.INCAN_TOOLCHAIN_MANIFEST) {
    next.push("--manifest", packageManifestUrl());
  }
  if (!hasValueOption(next, "--incan-home")) {
    next.push("--incan-home", toolchainHome());
  }
  if (!hasValueOption(next, "--bin-dir")) {
    next.push("--bin-dir", binDir());
  }
  return next;
}

function runInstaller(args, options = {}) {
  if (args.includes("--package-install") && process.env.INCAN_SKIP_NPM_INSTALL === "1") {
    return 0;
  }
  const result = childProcess.spawnSync("bash", [installerScript(), ...installerArgs(args)], {
    stdio: options.stdio || "inherit",
    env: process.env,
  });
  if (result.error) {
    throw result.error;
  }
  return result.status === null ? 1 : result.status;
}

function commandPath(command) {
  if (process.env.INCAN_NPM_TOOLCHAIN_DIR) {
    return path.join(process.env.INCAN_NPM_TOOLCHAIN_DIR, "bin", command);
  }
  return path.join(binDir(), command);
}

// The npm package is a reference shim: it carries no toolchain payload of its own. The first command invocation
// provisions the verified release archive through the bundled installer into the package-local toolchain home,
// exactly like the pip shim; later invocations reuse that installation. Install scripts never run.
function ensureToolchain(command) {
  const executable = commandPath(command);
  if (fs.existsSync(executable)) {
    return executable;
  }
  if (process.env.INCAN_NPM_TOOLCHAIN_DIR) {
    throw new Error(`missing ${command} binary in INCAN_NPM_TOOLCHAIN_DIR: ${executable}`);
  }
  console.error(`incan: provisioning the Incan toolchain (first run) ...`);
  const status = runInstaller(["--package-install"]);
  if (status !== 0) {
    throw new Error(`toolchain provisioning failed with exit status ${status}; rerun with \`install-incan\``);
  }
  if (!fs.existsSync(executable)) {
    throw new Error(
      `toolchain provisioning completed but ${command} is missing at ${executable}; rerun with \`install-incan\``,
    );
  }
  return executable;
}

function runCommand(command, args) {
  let executable;
  try {
    executable = ensureToolchain(command);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
  const child = childProcess.spawn(executable, args, {
    stdio: "inherit",
    env: process.env,
  });
  child.on("error", (error) => {
    console.error(error.message);
    process.exit(1);
  });
  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
    }
    process.exit(code === null ? 1 : code);
  });
}

module.exports = {
  runCommand,
  runInstaller,
};
