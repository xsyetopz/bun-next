// Deep-strict-equality helpers shared by node:assert and node:util.
"use strict";

// Native comparator: node semantics including [[Prototype]] identity.
// Third arg truthy = skipPrototype (Assert class option, node's
// kStrictWithoutPrototypes mode). node v26.3.0 exposes fn.length === 3.
const isDeepStrictEqual = $newCppFunction("NodeUtilTypesModule.cpp", "jsFunctionIsDeepStrictEqual", 3);
// Compares only the properties that exist on the second argument.
const isPartialStrictEqual = $newCppFunction("NodeUtilTypesModule.cpp", "jsFunctionPartialDeepStrictEqual", 2);

export default { isDeepStrictEqual, isPartialStrictEqual };
