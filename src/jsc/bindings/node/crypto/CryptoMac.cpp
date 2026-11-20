#include "root.h"

#include "CryptoMac.h"

#include "ErrorCode.h"
#include "JSBuffer.h"
#include "ncrypto.h"
#include <JavaScriptCore/JSArrayBufferView.h>
#include <openssl/cmac.h>
#include <openssl/poly1305.h>

namespace Bun {

using namespace JSC;

// computeMac(algorithm, cipher, key, data): the tag of a MAC that BoringSSL has as a primitive rather than an EVP_MAC.
// The caller validates the arguments; a key the MAC cannot use throws as node's createMac does.
JSC_DEFINE_HOST_FUNCTION(jsFunctionCryptoComputeMac, (JSGlobalObject * globalObject, CallFrame* callFrame))
{
    auto& vm = globalObject->vm();
    auto scope = DECLARE_THROW_SCOPE(vm);
    String algorithm = callFrame->argument(0).toWTFString(globalObject);
    RETURN_IF_EXCEPTION(scope, {});
    auto* key = dynamicDowncast<JSArrayBufferView>(callFrame->argument(2));
    auto* data = dynamicDowncast<JSArrayBufferView>(callFrame->argument(3));
    if (!key || !data) [[unlikely]]
        return ERR::INVALID_ARG_TYPE(scope, globalObject, "key"_s, "ArrayBufferView"_s, callFrame->argument(2));
    auto keyBytes = key->span();
    auto dataBytes = data->span();

    if (algorithm == "poly1305"_s) {
        if (keyBytes.size() != 32)
            return ERR::CRYPTO_OPERATION_FAILED(scope, globalObject, "Failed to initialize MAC"_s);
        poly1305_state state;
        uint8_t tag[16];
        CRYPTO_poly1305_init(&state, keyBytes.data());
        CRYPTO_poly1305_update(&state, dataBytes.data(), dataBytes.size());
        CRYPTO_poly1305_finish(&state, tag);
        RELEASE_AND_RETURN(scope, JSValue::encode(createBuffer(globalObject, tag, sizeof(tag))));
    }

    String cipherName = callFrame->argument(1).toWTFString(globalObject);
    RETURN_IF_EXCEPTION(scope, {});
    const EVP_CIPHER* cipher = ncrypto::getCipherByName(cipherName);
    bssl::UniquePtr<CMAC_CTX> ctx(CMAC_CTX_new());
    if (!cipher || !ctx || EVP_CIPHER_key_length(cipher) != keyBytes.size() || !CMAC_Init(ctx.get(), keyBytes.data(), keyBytes.size(), cipher, nullptr))
        return ERR::CRYPTO_OPERATION_FAILED(scope, globalObject, "Failed to initialize MAC"_s);
    uint8_t tag[EVP_MAX_BLOCK_LENGTH];
    size_t tagLength = 0;
    if (!CMAC_Update(ctx.get(), dataBytes.data(), dataBytes.size()) || !CMAC_Final(ctx.get(), tag, &tagLength))
        return ERR::CRYPTO_OPERATION_FAILED(scope, globalObject, "Failed to finalize MAC"_s);
    RELEASE_AND_RETURN(scope, JSValue::encode(createBuffer(globalObject, tag, tagLength)));
}

}
