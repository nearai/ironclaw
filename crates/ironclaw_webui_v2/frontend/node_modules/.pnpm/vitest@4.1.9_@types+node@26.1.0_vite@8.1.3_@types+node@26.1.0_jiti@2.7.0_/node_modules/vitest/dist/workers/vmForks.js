import { w as workerInit } from '../chunks/init-forks.B4VBaxRV.js';
import { r as runVmTests, s as setupVmWorker } from '../chunks/vm.B2aHeaNw.js';
import '../chunks/init.d4hAcNdp.js';
import 'node:fs';
import 'node:module';
import 'node:url';
import 'pathe';
import 'vite/module-runner';
import '../chunks/startVitestModuleRunner.CCBQZ9DQ.js';
import '@vitest/utils/helpers';
import '../chunks/modules.BJuCwlRJ.js';
import '../chunks/utils.BX5Fg8C4.js';
import '@vitest/utils/timers';
import '../path.js';
import 'node:path';
import '../module-evaluator.js';
import 'node:vm';
import '../chunks/traces.DT5aQ62U.js';
import '@vitest/mocker';
import '@vitest/mocker/redirect';
import '../chunks/index.DC7d2Pf8.js';
import 'node:console';
import '@vitest/utils/serialize';
import '@vitest/utils/error';
import 'tinyrainbow';
import '../chunks/rpc.MzXet3jl.js';
import '../chunks/index.Chj8NDwU.js';
import '@vitest/utils/source-map';
import '../chunks/inspector.CvyFGlXm.js';
import '../chunks/evaluatedModules.Dg1zASAC.js';
import '../chunks/console.3WNpx0tS.js';
import 'node:stream';
import '@vitest/utils/resolver';
import '@vitest/utils/constants';

workerInit({
	runTests: runVmTests,
	setup: setupVmWorker
});
