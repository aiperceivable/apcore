import { AsyncLocalStorage } from 'node:async_hooks';
import { _setLifecycleCallbackContext } from './lifecycle.js';

_setLifecycleCallbackContext(new AsyncLocalStorage());
