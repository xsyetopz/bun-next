// What the compiler reads in place of src/js/node/http.ts, which `bun run` evaluates. The two do the same.
// Written against that file with the SHA-256 4566e81998e718cb7ec20a13fa0ec2ce5c624116dd7b54e5a65e9bc6e9577d41.
// When that file changes, bring the change here and write the new hash: test/internal/second-texts-of-modules.test.ts.

const { validateInteger, validateObject } = require("internal/validators");
const httpAgent = require("node:_http_agent");
const { Agent } = require("node:_http_agent");
const { ClientRequest } = require("node:_http_client");
const { validateHeaderName, validateHeaderValue, parsers } = require("node:_http_common");
const { IncomingMessage } = require("node:_http_incoming");
const { OutgoingMessage, isValidHeaderName, isValidHeaderValue } = require("node:_http_outgoing");
const { Server, ServerResponse } = require("node:_http_server");

const { METHODS, STATUS_CODES, setMaxHTTPHeaderSize, getMaxHTTPHeaderSize } = require("internal/http");

if (process.env.NODE_DEBUG) require("node:util").debuglog("http");

type ServerParameters = Parameters<typeof Server>;
type ClientRequestParameters = Parameters<typeof ClientRequest>;

function createServer(options?: ServerParameters[0], callback?: ServerParameters[1]): $Node.HttpServer {
  return new Server(options, callback);
}

/**
 * Makes an HTTP request.
 * @param {string | URL} url
 * @param {HTTPRequestOptions} [options]
 * @param {Function} [cb]
 * @returns {ClientRequest}
 */
function request(
  url?: ClientRequestParameters[0],
  options?: ClientRequestParameters[1],
  cb?: ClientRequestParameters[2],
): $Node.ClientRequest {
  return new ClientRequest(url, options, cb);
}

/**
 * Makes a `GET` HTTP request.
 * @param {string | URL} url
 * @param {HTTPRequestOptions} [options]
 * @param {Function} [cb]
 * @returns {ClientRequest}
 */
function get(url?: ClientRequestParameters[0], options?: ClientRequestParameters[1], cb?: ClientRequestParameters[2]) {
  const req = request(url, options, cb);
  req.end();
  return req;
}

const http_exports = {
  Agent,
  Server,
  METHODS,
  STATUS_CODES,
  createServer,
  ServerResponse,
  IncomingMessage,
  request,
  get,
  get maxHeaderSize() {
    return getMaxHTTPHeaderSize();
  },
  set maxHeaderSize(value) {
    setMaxHTTPHeaderSize(value);
  },
  isValidHeaderName,
  isValidHeaderValue,
  validateHeaderName,
  validateHeaderValue,
  setMaxIdleHTTPParsers(max: number) {
    validateInteger(max, "max", 1);
    parsers.max = max;
  },
  setGlobalProxyFromEnv,
  websocketMask,
  websocketUnmask,
  // Assigning http.globalAgent must affect the agent used by http.request
  // (which reads it from the node:_http_agent module), like Node.js.
  get globalAgent() {
    return httpAgent.globalAgent;
  },
  set globalAgent(value) {
    httpAgent.globalAgent = value;
  },
  ClientRequest,
  OutgoingMessage,
  WebSocket,
  CloseEvent,
  MessageEvent,
};

function bytesOf(name: string, view: unknown): Uint8Array {
  if (!ArrayBuffer.isView(view)) throw $ERR_INVALID_ARG_TYPE(name, ["Buffer", "TypedArray", "DataView"], view);
  // A detached view has no bytes, and its buffer cannot be viewed again.
  if (view.byteLength === 0) return new Uint8Array(0);
  return new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
}

function maskKeyBytes(key: unknown): number[] {
  const bytes = bytesOf("mask", key);
  if (bytes.length !== 4) throw $ERR_INVALID_ARG_VALUE("mask", key, "must be 4 bytes long");
  // Copied, because the key may share memory with the output.
  return [bytes[0], bytes[1], bytes[2], bytes[3]];
}

// RFC 6455, Section 5.3. The source is copied first when it shares memory with the output.
function xorMask(source: Uint8Array, key: number[], output: Uint8Array, offset: number, length: number) {
  if (source.buffer === output.buffer && source.byteOffset !== output.byteOffset + offset) {
    source = source.slice(0, length);
  }
  for (let i = 0; i < length; i++) output[offset + i] = source[i] ^ key[i & 3];
}

/**
 * XORs `length` bytes of `source` with the repeating 4-byte `mask` into `output` at `offset`.
 * @see https://nodejs.org/api/http.html#httpwebsocketmasksource-mask-output-offset-length
 */
function websocketMask(source: unknown, mask: unknown, output: unknown, offset: number, length: number) {
  const sourceBytes = bytesOf("source", source);
  const key = maskKeyBytes(mask);
  const outputBytes = bytesOf("output", output);
  validateInteger(offset, "offset", 0, outputBytes.length);
  validateInteger(length, "length", 0, sourceBytes.length);
  if (length > outputBytes.length - offset) {
    throw $ERR_OUT_OF_RANGE("length", `<= ${outputBytes.length - offset}`, length);
  }
  xorMask(sourceBytes, key, outputBytes, offset, length);
}

/**
 * XORs every byte of `buffer` with the repeating 4-byte `mask`, in place.
 * @see https://nodejs.org/api/http.html#httpwebsocketunmaskbuffer-mask
 */
function websocketUnmask(buffer: unknown, mask: unknown) {
  const bytes = bytesOf("buffer", buffer);
  xorMask(bytes, maskKeyBytes(mask), bytes, 0, bytes.length);
}

function noopRestore() {}

// Port of Node.js's http.setGlobalProxyFromEnv() (lib/http.js):
// https://github.com/nodejs/node/blob/v26.3.0/lib/http.js
// Points the global http and
// https agents at the proxy servers configured through the given environment
// variables, returning a function that restores the previous agents.
// (The fetch() global dispatcher half is not applicable here.)
function setGlobalProxyFromEnv(env: Record<string, string | undefined> = process.env) {
  validateObject(env, "proxyEnv");
  const { parseProxyUrl, redactInvalidProxyUrl } = require("internal/http");
  const httpProxy = parseProxyUrl(env, "http:");
  const httpsProxy = parseProxyUrl(env, "https:");

  if (httpProxy === null && httpsProxy === null) {
    return noopRestore;
  }

  if (httpProxy !== null && URL.canParse(httpProxy) === false) {
    throw $ERR_PROXY_INVALID_CONFIG(redactInvalidProxyUrl(httpProxy));
  }
  if (httpsProxy !== null && URL.canParse(httpsProxy) === false) {
    throw $ERR_PROXY_INVALID_CONFIG(redactInvalidProxyUrl(httpsProxy));
  }

  let originalHttpsAgent: $Node.Agent | undefined, originalHttpAgent: $Node.Agent | undefined;
  if (httpProxy !== null) {
    originalHttpAgent = httpAgent.globalAgent;
    httpAgent.globalAgent = new Agent({
      keepAlive: true,
      scheduling: "lifo",
      timeout: 5000,
      proxyEnv: env,
    });
  }
  if (httpsProxy !== null) {
    const https = require("node:https");
    originalHttpsAgent = https.globalAgent;
    https.globalAgent = new https.Agent({
      keepAlive: true,
      scheduling: "lifo",
      timeout: 5000,
      proxyEnv: env,
    });
  }

  return function restore() {
    if (originalHttpAgent !== undefined) {
      httpAgent.globalAgent = originalHttpAgent;
    }
    if (originalHttpsAgent !== undefined) {
      require("node:https").globalAgent = originalHttpsAgent;
    }
  };
}

export default http_exports;
