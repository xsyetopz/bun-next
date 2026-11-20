// node's crypto.createMac and crypto.getMacs. BoringSSL has no EVP_MAC, so each MAC is built from a primitive Bun already has.
const LazyTransform = require("internal/streams/lazy_transform");
const { validateEncoding, validateObject, validateString, validateUint32 } = require("internal/validators");
const { isAnyArrayBuffer, isArrayBufferView } = require("node:util/types");

const normalizeEncoding: (encoding: string) => string | undefined = $bindgen(
  "node_util_binding.bind.ts",
  "normalizeEncoding",
);

const computeMac: (algorithm: string, cipher: string | undefined, key: Buffer, data: Buffer) => Buffer =
  $newCppFunction("CryptoMac.cpp", "jsFunctionCryptoComputeMac", 4);

const kHandle = Symbol("kHandle");
const kFinalized = Symbol("kFinalized");

type MacOptions = { digest?: string; cipher?: string; iv?: Buffer };
type MacHandle = { update(data: Buffer): void; final(): Buffer };

function invalidOption(option: string, value: unknown, reason: string) {
  return $ERR_INVALID_ARG_VALUE(`options.${option}`, value, reason);
}

// OpenSSL reports an unknown digest from the EVP layer, and node passes that error through.
function unsupportedDigest() {
  const error = new Error("error:0308010C:digital envelope routines::unsupported");
  Object.assign(error, {
    library: "digital envelope routines",
    reason: "unsupported",
    code: "ERR_OSSL_EVP_UNSUPPORTED",
  });
  return error;
}

function keyBytes(key: any): Buffer {
  return Buffer.isBuffer(key) ? key : key.export();
}

// BoringSSL computes these in one call, so the message is buffered until final().
function oneShot(algorithm: string, cipher: string | undefined, key: unknown): MacHandle {
  const bytes = keyBytes(key);
  computeMac(algorithm, cipher, bytes, Buffer.alloc(0));
  const chunks: Buffer[] = [];
  return {
    update: data => void chunks.push(Buffer.from(data)),
    final: () => computeMac(algorithm, cipher, bytes, Buffer.concat(chunks)),
  };
}

const macs: Record<string, { options: string[]; create(key: unknown, options: MacOptions): MacHandle }> = {
  __proto__: null as any,
  cmac: {
    options: ["cipher"],
    create(key, { cipher }) {
      if (cipher === undefined) throw invalidOption("cipher", cipher, "is required by cmac");
      cipher = cipher.toLowerCase();
      if (!cipher.endsWith("-cbc")) throw invalidOption("cipher", cipher, "must use CBC mode");
      return oneShot("cmac", cipher, key);
    },
  },
  gmac: {
    options: ["cipher", "iv"],
    create(key, { cipher, iv }) {
      if (cipher === undefined) throw invalidOption("cipher", cipher, "is required by gmac");
      if (!cipher.toLowerCase().endsWith("-gcm")) throw invalidOption("cipher", cipher, "must use GCM mode");
      if (iv === undefined || iv.length === 0) throw invalidOption("iv", iv, "must be non-empty for gmac");
      const gcm = require("node:crypto").createCipheriv(cipher, key, iv);
      return {
        update: data => gcm.setAAD(data),
        final() {
          gcm.final();
          return gcm.getAuthTag();
        },
      };
    },
  },
  hmac: {
    options: ["digest"],
    create(key, { digest }) {
      if (digest === undefined) throw invalidOption("digest", digest, "is required by hmac");
      let hmac;
      try {
        hmac = require("node:crypto").createHmac(digest, key);
      } catch (error) {
        if ((error as any)?.code === "ERR_CRYPTO_INVALID_DIGEST") throw unsupportedDigest();
        throw error;
      }
      return { update: data => hmac.update(data), final: () => hmac.digest() };
    },
  },
  poly1305: {
    options: [],
    create: key => oneShot("poly1305", undefined, key),
  },
};

function validateName(value: unknown, name: string): string {
  validateString(value, name);
  if (value.length === 0 || value.includes("\0")) {
    throw $ERR_INVALID_ARG_VALUE(name, value, "must be non-empty and contain no NUL bytes");
  }
  return value;
}

function toBuffer(value: ArrayBufferView | ArrayBufferLike): Buffer {
  if (isAnyArrayBuffer(value)) return Buffer.from(value as ArrayBufferLike);
  const view = value as ArrayBufferView;
  return Buffer.from(view.buffer, view.byteOffset, view.byteLength);
}

function normalizeBytes(value: unknown, name: string): Buffer {
  if (!isArrayBufferView(value) && !isAnyArrayBuffer(value)) {
    throw $ERR_INVALID_ARG_TYPE(name, ["ArrayBuffer", "Buffer", "TypedArray", "DataView"], value);
  }
  return toBuffer(value as ArrayBufferView);
}

function normalizeKey(key: unknown) {
  const { KeyObject } = require("node:crypto");
  if (key instanceof KeyObject && key.type === "secret") return key;
  if (!isArrayBufferView(key) && !isAnyArrayBuffer(key)) {
    throw $ERR_INVALID_ARG_TYPE("key", ["ArrayBuffer", "Buffer", "TypedArray", "DataView", "KeyObject"], key);
  }
  return toBuffer(key as ArrayBufferView);
}

function createHandle(algorithm: unknown, key: unknown, options: any): MacHandle {
  const name = validateName(algorithm, "algorithm");
  const given: Record<string, unknown> = { __proto__: null };
  if (options !== undefined) {
    validateObject(options, "options");
    if (options.digest !== undefined) given.digest = validateName(options.digest, "options.digest");
    if (options.cipher !== undefined) given.cipher = validateName(options.cipher, "options.cipher");
    for (const option of ["iv", "customization", "salt"]) {
      if (options[option] !== undefined) given[option] = normalizeBytes(options[option], `options.${option}`);
    }
    if (options.outputLength !== undefined) {
      validateUint32(options.outputLength, "options.outputLength");
      given.outputLength = options.outputLength;
    }
  }
  key = normalizeKey(key);
  const mac = macs[name.toLowerCase()];
  if (mac === undefined) throw $ERR_CRYPTO_INVALID_MAC(`Invalid MAC: ${name}`);
  for (const option in given) {
    if (!mac.options.includes(option)) throw invalidOption(option, given[option], `is not supported by ${name}`);
  }
  return mac.create(key, given as MacOptions);
}

function normalizeOutputEncoding(outputEncoding: unknown): string {
  if (outputEncoding === undefined) return "buffer";
  validateString(outputEncoding, "outputEncoding");
  if (outputEncoding.toLowerCase() === "buffer") return "buffer";
  const normalized = normalizeEncoding(outputEncoding);
  if (normalized === undefined) throw $ERR_INVALID_ARG_VALUE("outputEncoding", outputEncoding);
  return normalized;
}

function normalizeInputEncoding(data: string, inputEncoding: unknown): string | undefined {
  if (inputEncoding === undefined) return undefined;
  validateString(inputEncoding, "inputEncoding");
  const normalized = normalizeEncoding(inputEncoding);
  if (normalized === undefined) throw $ERR_INVALID_ARG_VALUE("inputEncoding", inputEncoding);
  validateEncoding(data, normalized);
  return normalized;
}

function macFinalized() {
  return $ERR_CRYPTO_MAC_FINALIZED("MAC already finalized");
}

// An update that fails leaves the MAC unusable, as in node.
function updateHandle(mac, data: string | ArrayBufferView, encoding?: string) {
  const bytes = typeof data === "string" ? Buffer.from(data, encoding as BufferEncoding) : toBuffer(data);
  try {
    mac[kHandle].update(bytes);
  } catch (error) {
    mac[kFinalized] = true;
    throw error;
  }
}

function finalizeHandle(mac): Buffer {
  mac[kFinalized] = true;
  return mac[kHandle].final();
}

function Mac(this: any, algorithm: unknown, key: unknown, options?: object) {
  if (!new.target) return new Mac(algorithm, key, options);
  this[kHandle] = createHandle(algorithm, key, options);
  this[kFinalized] = false;
  LazyTransform.$call(this, options);
}
$toClass(Mac, "Mac", LazyTransform);

Object.assign(Mac.prototype, {
  _transform(chunk, encoding, callback) {
    if (this[kFinalized]) return callback(macFinalized());
    try {
      updateHandle(this, chunk, encoding);
    } catch (error) {
      return callback(error);
    }
    callback();
  },
  _flush(callback) {
    if (this[kFinalized]) return callback(macFinalized());
    try {
      const result = finalizeHandle(this);
      if (result.length !== 0) this.push(result);
    } catch (error) {
      return callback(error);
    }
    callback();
  },
  update(data: unknown, encoding?: unknown) {
    if (this[kFinalized]) throw macFinalized();
    if (typeof data === "string") {
      encoding = normalizeInputEncoding(data, encoding);
    } else if (!isArrayBufferView(data)) {
      throw $ERR_INVALID_ARG_TYPE("data", ["string", "Buffer", "TypedArray", "DataView"], data);
    }
    updateHandle(this, data as string | ArrayBufferView, encoding as string | undefined);
    return this;
  },
  final(outputEncoding?: unknown) {
    if (this[kFinalized]) throw macFinalized();
    const encoding = normalizeOutputEncoding(outputEncoding);
    const tag = finalizeHandle(this);
    return encoding === "buffer" ? tag : tag.toString(encoding as BufferEncoding);
  },
});

export default {
  createMac(algorithm: unknown, key: unknown, options?: object) {
    return new Mac(algorithm, key, options);
  },
  getMacs(): string[] {
    return Object.keys(macs).sort();
  },
};
