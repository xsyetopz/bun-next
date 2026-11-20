// Implementation for `require('node:module')._initPaths`. Exists only as a
// compatibility stub. Calling this does not affect the actual CommonJS loader.
export function _initPaths() {
  const homeDir: string | undefined = process.platform === "win32" ? process.env.USERPROFILE : Bun.env.HOME;
  const nodePath: string | undefined = process.platform === "win32" ? process.env.NODE_PATH : Bun.env.NODE_PATH;

  // process.execPath is $PREFIX/bin/node except on Windows where it is
  // $PREFIX\node.exe where $PREFIX is the root of the Node.js installation.
  const path = require("node:path");
  const prefixDir =
    process.platform === "win32" ? path.resolve(process.execPath, "..") : path.resolve(process.execPath, "..", "..");

  const paths = [path.resolve(prefixDir, "lib", "node")];

  if (homeDir) {
    paths.unshift(path.resolve(homeDir, ".node_libraries"));
    paths.unshift(path.resolve(homeDir, ".node_modules"));
  }

  if (nodePath) {
    paths.unshift(...nodePath.split(path.delimiter).filter(Boolean));
  }

  const M = require("node:module");
  M.globalPaths = paths;
}

// Implementation for `require('node:module').findPackageJSON`, following
// lib/internal/modules/package_json_reader.js in Node.js.
export function findPackageJSON(specifier, base) {
  if (arguments.length === 0) throw $ERR_MISSING_ARGS("specifier");
  if (base === undefined) base = "data:";
  const { validateString } = require("internal/validators");
  try {
    specifier = `${specifier}`;
  } catch {
    validateString(specifier, "specifier");
  }

  const path = require("node:path");
  const { readFileSync, realpathSync, statSync } = require("node:fs");
  const { fileURLToPath, pathToFileURL } = require("node:url");

  const codedError = (Ctor, code, message) => {
    const err = new Ctor(message);
    err.code = code;
    return err;
  };
  // Like Node, a package.json that cannot be read counts as absent and one that cannot be parsed throws.
  const readPackageJSON = (file: string) => {
    let source;
    try {
      source = readFileSync(file, "utf8");
    } catch {
      return undefined;
    }
    try {
      return JSON.parse(source) ?? {};
    } catch {
      throw codedError(Error, "ERR_INVALID_PACKAGE_CONFIG", `Invalid package config ${file}.`);
    }
  };

  let parentURL = base;
  const isURL =
    base != null && typeof base.href === "string" && base.protocol !== undefined && base.auth === undefined && base.path === undefined;
  if (!isURL) {
    validateString(base, "base");
    parentURL = path.isAbsolute(base) ? pathToFileURL(base) : new URL(base);
  }

  if (specifier && specifier[0] !== "." && specifier[0] !== "/" && !URL.canParse(specifier)) {
    let separatorIndex = specifier.indexOf("/");
    const isScoped = specifier[0] === "@";
    let validPackageName = true;
    if (isScoped) {
      if (separatorIndex === -1) validPackageName = false;
      else separatorIndex = specifier.indexOf("/", separatorIndex + 1);
    }
    const packageName = separatorIndex === -1 ? specifier : specifier.slice(0, separatorIndex);
    if (!validPackageName || /^\.|%|\\/.test(packageName)) {
      throw codedError(
        TypeError,
        "ERR_INVALID_MODULE_SPECIFIER",
        `Invalid module "${specifier}" is not a valid package name imported from ${fileURLToPath(parentURL)}`,
      );
    }

    // A package can name itself when its package.json has "exports".
    if (parentURL.protocol === "file:") {
      let scopeURL = new URL("./package.json", parentURL);
      while (!scopeURL.pathname.endsWith("node_modules/package.json")) {
        const scopePath = fileURLToPath(scopeURL);
        const scope = readPackageJSON(scopePath);
        if (scope !== undefined) {
          if (scope.exports != null && scope.name === packageName) return scopePath;
          break;
        }
        const lastPathname = scopeURL.pathname;
        scopeURL = new URL("../package.json", scopeURL);
        if (scopeURL.pathname === lastPathname) break;
      }
    }

    let packageJSONUrl = new URL(`./node_modules/${packageName}/package.json`, parentURL);
    let packageJSONPath = fileURLToPath(packageJSONUrl);
    let lastPath;
    do {
      const stat = statSync(packageJSONPath.slice(0, -13), { throwIfNoEntry: false });
      if (stat?.isDirectory()) return packageJSONPath;
      lastPath = packageJSONPath;
      packageJSONUrl = new URL(`${isScoped ? "../" : ""}../../../node_modules/${packageName}/package.json`, packageJSONUrl);
      packageJSONPath = fileURLToPath(packageJSONUrl);
    } while (packageJSONPath.length !== lastPath.length);

    throw codedError(
      Error,
      "ERR_MODULE_NOT_FOUND",
      `Cannot find package '${packageName}' imported from ${fileURLToPath(parentURL)}`,
    );
  }

  let target = fileURLToPath(new URL(specifier, parentURL));
  const stat = statSync(target, { throwIfNoEntry: false });
  if (!stat) {
    throw codedError(
      Error,
      "ERR_MODULE_NOT_FOUND",
      `Cannot find module '${target}' imported from ${fileURLToPath(parentURL)}`,
    );
  }
  if (!stat.isDirectory()) target = realpathSync(target);

  // The nearest package.json above the target, never the one at the file system root,
  // and the search stops at a node_modules directory.
  let dir = target.endsWith(path.sep) ? target.slice(0, -1) : path.dirname(target);
  while (path.dirname(dir) !== dir && path.basename(dir) !== "node_modules") {
    const file = path.join(dir, "package.json");
    if (readPackageJSON(file) !== undefined) return file;
    dir = path.dirname(dir);
  }
  return undefined;
}
