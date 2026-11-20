#pragma once

#include "root.h"

namespace Bun {

// node's crypto.createMac backends that BoringSSL provides as primitives: CMAC and Poly1305.
JSC_DECLARE_HOST_FUNCTION(jsFunctionCryptoComputeMac);

}
