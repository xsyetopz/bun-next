#pragma once

#include "root.h"
#include "ScriptExecutionContext.h"

namespace Bun {

// The process-wide lock manager behind node's worker_threads.locks (Web Locks).
JSC_DECLARE_HOST_FUNCTION(jsFunctionLocksRequest);
JSC_DECLARE_HOST_FUNCTION(jsFunctionLocksRelease);
JSC_DECLARE_HOST_FUNCTION(jsFunctionLocksAbort);
JSC_DECLARE_HOST_FUNCTION(jsFunctionLocksQuery);

// Drops every lock a terminating context holds or waits for, so other threads can take them.
void releaseLocksOfContext(WebCore::ScriptExecutionContextIdentifier);

}
