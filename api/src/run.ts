import { RouteMap, RouteMapLike } from './routing.ts';
import { serve } from './serve.ts';
import { specialBefore, specialAfter } from './special.ts';

// TODO: explore what this does in more detail
Deno.core.opSync(
    'op_set_promise_reject_callback',
    (type: number, _promise: unknown, reason: unknown) => {
        if (type == 0) { // PromiseRejectWithNoHandler
            // Without this function deno pushes the exception to
            // pending_promise_exceptions, which eventually causes an unlucky
            // user of poll_event_loop to get an error. Since user code can
            // create and reject a promise that lacks a handler, we have to do
            // this. Throwing in here causes deno to at least log the stack.
            throw new Error('Promise rejected without a handler: ' + reason);
        }
    },
);

export default async function(userRouteMap: RouteMapLike): Promise<void> {
    const routeMap = new RouteMap();
    specialBefore(routeMap);
    routeMap.prefix('/', RouteMap.convert(userRouteMap));
    specialAfter(routeMap);
    await serve(routeMap);
}