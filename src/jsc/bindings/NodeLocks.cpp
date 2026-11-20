#include "root.h"

#include "NodeLocks.h"

#include "InternalModuleRegistry.h"
#include "JSDOMExceptionHandling.h"
#include "ZigGlobalObject.h"
#include <wtf/Lock.h>
#include <wtf/Locker.h>
#include <wtf/NeverDestroyed.h>

namespace Bun {

using namespace JSC;
using WebCore::ScriptExecutionContext;
using WebCore::ScriptExecutionContextIdentifier;

namespace {

struct WebLockEntry {
    uint64_t id;
    String name;
    String clientId;
    ScriptExecutionContextIdentifier context;
    bool exclusive;
    bool steal;
};

struct WebLockNotification {
    uint64_t id;
    ScriptExecutionContextIdentifier context;
    // false: a steal took the held lock away.
    bool granted;
};

struct WebLockState {
    WTF::Lock lock;
    Vector<WebLockEntry> pending WTF_GUARDED_BY_LOCK(lock);
    Vector<WebLockEntry> held WTF_GUARDED_BY_LOCK(lock);
    uint64_t nextId WTF_GUARDED_BY_LOCK(lock) { 1 };
};

WebLockState& webLockState()
{
    static NeverDestroyed<WebLockState> instance;
    return instance;
}

// https://w3c.github.io/web-locks/#grantable, with node's rule that a request waits only on the oldest pending request of its name.
bool isWebLockGrantable(WebLockState& s, size_t index) WTF_REQUIRES_LOCK(s.lock)
{
    auto& request = s.pending[index];
    if (request.steal)
        return true;
    for (size_t i = 0; i < index; i++) {
        if (s.pending[i].name != request.name)
            continue;
        if (request.exclusive || s.pending[i].exclusive)
            return false;
        break;
    }
    for (auto& held : s.held) {
        if (held.name == request.name && (request.exclusive || held.exclusive))
            return false;
    }
    return true;
}

void processWebLockQueue(WebLockState& s, Vector<WebLockNotification>& out) WTF_REQUIRES_LOCK(s.lock)
{
    for (size_t i = 0; i < s.pending.size();) {
        if (!isWebLockGrantable(s, i)) {
            i++;
            continue;
        }
        auto request = WTF::move(s.pending[i]);
        s.pending.removeAt(i);
        if (request.steal) {
            s.held.removeAllMatching([&](const WebLockEntry& held) {
                if (held.name != request.name)
                    return false;
                out.append({ held.id, held.context, false });
                return true;
            });
        }
        out.append({ request.id, request.context, true });
        s.held.append(WTF::move(request));
    }
}

void settleWebLock(ScriptExecutionContext& context, uint64_t id, bool granted)
{
    auto* globalObject = defaultGlobalObject(context.jsGlobalObject());
    auto& vm = globalObject->vm();
    auto scope = DECLARE_TOP_EXCEPTION_SCOPE(vm);
    JSValue settleFunction = BUN_REQUIRE_INTERNAL_MODULE(globalObject, InternalLocks, "node:worker_threads", "settle"_s);
    if (!scope.exception()) {
        MarkedArgumentBuffer args;
        args.append(jsNumber(id));
        args.append(jsBoolean(granted));
        JSC::call(globalObject, settleFunction, JSC::getCallData(settleFunction), jsUndefined(), args);
    }
    if (auto* exception = scope.exception()) {
        if (vm.hasPendingTerminationException())
            return;
        scope.clearException();
        WebCore::reportException(globalObject, exception);
    }
}

// Runs without the lock: a grant whose context is gone is released, which can grant other requests.
// As in node, the calling context settles its own requests before returning, so a steal rejects the victim at once.
void deliverWebLockNotifications(Vector<WebLockNotification>&& notifications, ScriptExecutionContext* current = nullptr)
{
    auto& s = webLockState();
    while (!notifications.isEmpty()) {
        Vector<uint64_t> undelivered;
        for (auto& notification : notifications) {
            if (current && notification.context == current->identifier()) {
                settleWebLock(*current, notification.id, notification.granted);
                continue;
            }
            bool posted = ScriptExecutionContext::postTaskTo(notification.context, BunLoopKind::Regular, [id = notification.id, granted = notification.granted](ScriptExecutionContext& context) {
                settleWebLock(context, id, granted);
            });
            if (!posted && notification.granted)
                undelivered.append(notification.id);
        }
        notifications.clear();
        if (undelivered.isEmpty())
            return;
        Locker locker { s.lock };
        s.held.removeAllMatching([&](const WebLockEntry& held) { return undelivered.contains(held.id); });
        processWebLockQueue(s, notifications);
    }
}

ScriptExecutionContext& webLockContextOf(JSGlobalObject* globalObject)
{
    return *defaultGlobalObject(globalObject)->scriptExecutionContext();
}

}

// request(name, clientId, exclusive, steal, ifAvailable): the request id, negated when the lock is already granted, or 0 when ifAvailable cannot be granted now.
JSC_DEFINE_HOST_FUNCTION(jsFunctionLocksRequest, (JSGlobalObject * globalObject, CallFrame* callFrame))
{
    auto& vm = globalObject->vm();
    auto scope = DECLARE_THROW_SCOPE(vm);
    String name = callFrame->argument(0).toWTFString(globalObject);
    RETURN_IF_EXCEPTION(scope, {});
    String clientId = callFrame->argument(1).toWTFString(globalObject);
    RETURN_IF_EXCEPTION(scope, {});
    bool exclusive = callFrame->argument(2).toBoolean(globalObject);
    bool steal = callFrame->argument(3).toBoolean(globalObject);
    bool ifAvailable = callFrame->argument(4).toBoolean(globalObject);

    auto& s = webLockState();
    Vector<WebLockNotification> notifications;
    uint64_t id;
    {
        Locker locker { s.lock };
        id = s.nextId++;
        WebLockEntry request { id, name.isolatedCopy(), clientId.isolatedCopy(), webLockContextOf(globalObject).identifier(), exclusive, steal };
        if (steal) {
            s.pending.insert(0, WTF::move(request));
        } else {
            s.pending.append(WTF::move(request));
            if (ifAvailable && !isWebLockGrantable(s, s.pending.size() - 1)) {
                s.pending.removeLast();
                return JSValue::encode(jsNumber(0));
            }
        }
        processWebLockQueue(s, notifications);
    }
    // As in node, a request granted at once runs its callback before request() returns.
    bool grantedNow = notifications.removeFirstMatching([&](const WebLockNotification& notification) { return notification.id == id && notification.granted; });
    deliverWebLockNotifications(WTF::move(notifications), &webLockContextOf(globalObject));
    return JSValue::encode(jsNumber(grantedNow ? -static_cast<double>(id) : static_cast<double>(id)));
}

JSC_DEFINE_HOST_FUNCTION(jsFunctionLocksRelease, (JSGlobalObject * globalObject, CallFrame* callFrame))
{
    uint64_t id = static_cast<uint64_t>(callFrame->argument(0).asNumber());
    auto& s = webLockState();
    Vector<WebLockNotification> notifications;
    {
        Locker locker { s.lock };
        if (!s.held.removeFirstMatching([&](const WebLockEntry& held) { return held.id == id; }))
            return JSValue::encode(jsUndefined());
        processWebLockQueue(s, notifications);
    }
    deliverWebLockNotifications(WTF::move(notifications), &webLockContextOf(globalObject));
    return JSValue::encode(jsUndefined());
}

// abort(id): true when the request was still pending and is now withdrawn.
JSC_DEFINE_HOST_FUNCTION(jsFunctionLocksAbort, (JSGlobalObject * globalObject, CallFrame* callFrame))
{
    uint64_t id = static_cast<uint64_t>(callFrame->argument(0).asNumber());
    auto& s = webLockState();
    Vector<WebLockNotification> notifications;
    {
        Locker locker { s.lock };
        if (!s.pending.removeFirstMatching([&](const WebLockEntry& request) { return request.id == id; }))
            return JSValue::encode(jsBoolean(false));
        processWebLockQueue(s, notifications);
    }
    deliverWebLockNotifications(WTF::move(notifications), &webLockContextOf(globalObject));
    return JSValue::encode(jsBoolean(true));
}

// The held and pending requests of the calling context, as node's LockManagerSnapshot.
JSC_DEFINE_HOST_FUNCTION(jsFunctionLocksQuery, (JSGlobalObject * globalObject, CallFrame* callFrame))
{
    auto& vm = globalObject->vm();
    auto scope = DECLARE_THROW_SCOPE(vm);
    auto context = webLockContextOf(globalObject).identifier();
    Vector<WebLockEntry> held;
    Vector<WebLockEntry> pending;
    {
        auto& s = webLockState();
        Locker locker { s.lock };
        auto copy = [&](const Vector<WebLockEntry>& from, Vector<WebLockEntry>& to) {
            for (auto& request : from) {
                if (request.context == context)
                    to.append({ request.id, request.name.isolatedCopy(), request.clientId.isolatedCopy(), context, request.exclusive, request.steal });
            }
        };
        copy(s.held, held);
        copy(s.pending, pending);
    }

    auto toArray = [&](const Vector<WebLockEntry>& requests) -> JSArray* {
        auto* array = constructEmptyArray(globalObject, nullptr, requests.size());
        RETURN_IF_EXCEPTION(scope, nullptr);
        for (size_t i = 0; i < requests.size(); i++) {
            auto* info = constructEmptyObject(globalObject);
            info->putDirect(vm, Identifier::fromString(vm, "name"_s), jsString(vm, requests[i].name));
            info->putDirect(vm, Identifier::fromString(vm, "mode"_s), jsNontrivialString(vm, requests[i].exclusive ? "exclusive"_s : "shared"_s));
            info->putDirect(vm, Identifier::fromString(vm, "clientId"_s), jsString(vm, requests[i].clientId));
            array->putDirectIndex(globalObject, i, info);
            RETURN_IF_EXCEPTION(scope, nullptr);
        }
        return array;
    };

    auto* result = constructEmptyObject(globalObject);
    auto* heldArray = toArray(held);
    RETURN_IF_EXCEPTION(scope, {});
    result->putDirect(vm, Identifier::fromString(vm, "held"_s), heldArray);
    auto* pendingArray = toArray(pending);
    RETURN_IF_EXCEPTION(scope, {});
    result->putDirect(vm, Identifier::fromString(vm, "pending"_s), pendingArray);
    return JSValue::encode(result);
}

void releaseLocksOfContext(ScriptExecutionContextIdentifier context)
{
    auto& s = webLockState();
    Vector<WebLockNotification> notifications;
    {
        Locker locker { s.lock };
        auto ofContext = [&](const WebLockEntry& request) { return request.context == context; };
        size_t removed = s.pending.removeAllMatching(ofContext) + s.held.removeAllMatching(ofContext);
        if (!removed)
            return;
        processWebLockQueue(s, notifications);
    }
    deliverWebLockNotifications(WTF::move(notifications));
}

}
