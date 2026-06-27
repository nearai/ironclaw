import{a as An,b as ze,c as Ke,d as p,e as u,f as ev,g as tv,h as gl,i as C,j as yl}from"./chunks/chunk-OAH67JV7.js";var xv=An(kl=>{"use strict";var qk=Symbol.for("react.transitional.element"),Ik=Symbol.for("react.fragment");function bv(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:qk,type:e,key:n,ref:t!==void 0?t:null,props:a}}kl.Fragment=Ik;kl.jsx=bv;kl.jsxs=bv});var Ad=An((C6,$v)=>{"use strict";$v.exports=xv()});var Lv=An(Oe=>{"use strict";function Ud(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<Pl(r,t))e[n]=t,e[a]=r,a=n;else break e}}function Fa(e){return e.length===0?null:e[0]}function Ul(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],l=i+1,c=e[l];if(0>Pl(o,a))l<r&&0>Pl(c,o)?(e[n]=c,e[l]=a,n=l):(e[n]=o,e[i]=a,n=i);else if(l<r&&0>Pl(c,a))e[n]=c,e[l]=a,n=l;else break e}}return t}function Pl(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}Oe.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(Rv=performance,Oe.unstable_now=function(){return Rv.now()}):(Ld=Date,kv=Ld.now(),Oe.unstable_now=function(){return Ld.now()-kv});var Rv,Ld,kv,rn=[],On=[],Vk=1,ua=null,gt=3,Fd=!1,Ui=!1,Fi=!1,Bd=!1,Tv=typeof setTimeout=="function"?setTimeout:null,Av=typeof clearTimeout=="function"?clearTimeout:null,Cv=typeof setImmediate<"u"?setImmediate:null;function jl(e){for(var t=Fa(On);t!==null;){if(t.callback===null)Ul(On);else if(t.startTime<=e)Ul(On),t.sortIndex=t.expirationTime,Ud(rn,t);else break;t=Fa(On)}}function zd(e){if(Fi=!1,jl(e),!Ui)if(Fa(rn)!==null)Ui=!0,is||(is=!0,ss());else{var t=Fa(On);t!==null&&qd(zd,t.startTime-e)}}var is=!1,Bi=-1,Dv=5,Mv=-1;function Ov(){return Bd?!0:!(Oe.unstable_now()-Mv<Dv)}function Pd(){if(Bd=!1,is){var e=Oe.unstable_now();Mv=e;var t=!0;try{e:{Ui=!1,Fi&&(Fi=!1,Av(Bi),Bi=-1),Fd=!0;var a=gt;try{t:{for(jl(e),ua=Fa(rn);ua!==null&&!(ua.expirationTime>e&&Ov());){var n=ua.callback;if(typeof n=="function"){ua.callback=null,gt=ua.priorityLevel;var r=n(ua.expirationTime<=e);if(e=Oe.unstable_now(),typeof r=="function"){ua.callback=r,jl(e),t=!0;break t}ua===Fa(rn)&&Ul(rn),jl(e)}else Ul(rn);ua=Fa(rn)}if(ua!==null)t=!0;else{var s=Fa(On);s!==null&&qd(zd,s.startTime-e),t=!1}}break e}finally{ua=null,gt=a,Fd=!1}t=void 0}}finally{t?ss():is=!1}}}var ss;typeof Cv=="function"?ss=function(){Cv(Pd)}:typeof MessageChannel<"u"?(jd=new MessageChannel,Ev=jd.port2,jd.port1.onmessage=Pd,ss=function(){Ev.postMessage(null)}):ss=function(){Tv(Pd,0)};var jd,Ev;function qd(e,t){Bi=Tv(function(){e(Oe.unstable_now())},t)}Oe.unstable_IdlePriority=5;Oe.unstable_ImmediatePriority=1;Oe.unstable_LowPriority=4;Oe.unstable_NormalPriority=3;Oe.unstable_Profiling=null;Oe.unstable_UserBlockingPriority=2;Oe.unstable_cancelCallback=function(e){e.callback=null};Oe.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):Dv=0<e?Math.floor(1e3/e):5};Oe.unstable_getCurrentPriorityLevel=function(){return gt};Oe.unstable_next=function(e){switch(gt){case 1:case 2:case 3:var t=3;break;default:t=gt}var a=gt;gt=t;try{return e()}finally{gt=a}};Oe.unstable_requestPaint=function(){Bd=!0};Oe.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=gt;gt=e;try{return t()}finally{gt=a}};Oe.unstable_scheduleCallback=function(e,t,a){var n=Oe.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:Vk++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,Ud(On,e),Fa(rn)===null&&e===Fa(On)&&(Fi?(Av(Bi),Bi=-1):Fi=!0,qd(zd,a-n))):(e.sortIndex=r,Ud(rn,e),Ui||Fd||(Ui=!0,is||(is=!0,ss()))),e};Oe.unstable_shouldYield=Ov;Oe.unstable_wrapCallback=function(e){var t=gt;return function(){var a=gt;gt=t;try{return e.apply(this,arguments)}finally{gt=a}}}});var jv=An((cP,Pv)=>{"use strict";Pv.exports=Lv()});var Fv=An(Et=>{"use strict";var Gk=Ke();function Uv(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Ln(){}var Ct={d:{f:Ln,r:function(){throw Error(Uv(522))},D:Ln,C:Ln,L:Ln,m:Ln,X:Ln,S:Ln,M:Ln},p:0,findDOMNode:null},Yk=Symbol.for("react.portal");function Jk(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:Yk,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var zi=Gk.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function Fl(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}Et.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=Ct;Et.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(Uv(299));return Jk(e,t,null,a)};Et.flushSync=function(e){var t=zi.T,a=Ct.p;try{if(zi.T=null,Ct.p=2,e)return e()}finally{zi.T=t,Ct.p=a,Ct.d.f()}};Et.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,Ct.d.C(e,t))};Et.prefetchDNS=function(e){typeof e=="string"&&Ct.d.D(e)};Et.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=Fl(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?Ct.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&Ct.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};Et.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=Fl(t.as,t.crossOrigin);Ct.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&Ct.d.M(e)};Et.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=Fl(a,t.crossOrigin);Ct.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};Et.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=Fl(t.as,t.crossOrigin);Ct.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else Ct.d.m(e)};Et.requestFormReset=function(e){Ct.d.r(e)};Et.unstable_batchedUpdates=function(e,t){return e(t)};Et.useFormState=function(e,t,a){return zi.H.useFormState(e,t,a)};Et.useFormStatus=function(){return zi.H.useHostTransitionStatus()};Et.version="19.1.0"});var qv=An((mP,zv)=>{"use strict";function Bv(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(Bv)}catch(e){console.error(e)}}Bv(),zv.exports=Fv()});var H0=An(oc=>{"use strict";var rt=jv(),cy=Ke(),Xk=qv();function j(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function dy(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function Eo(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function my(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function Iv(e){if(Eo(e)!==e)throw Error(j(188))}function Wk(e){var t=e.alternate;if(!t){if(t=Eo(e),t===null)throw Error(j(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return Iv(r),e;if(s===n)return Iv(r),t;s=s.sibling}throw Error(j(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(j(189))}}if(a.alternate!==n)throw Error(j(190))}if(a.tag!==3)throw Error(j(188));return a.stateNode.current===a?e:t}function fy(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=fy(e),t!==null)return t;e=e.sibling}return null}var De=Object.assign,Zk=Symbol.for("react.element"),Bl=Symbol.for("react.transitional.element"),Ji=Symbol.for("react.portal"),fs=Symbol.for("react.fragment"),py=Symbol.for("react.strict_mode"),xm=Symbol.for("react.profiler"),eC=Symbol.for("react.provider"),hy=Symbol.for("react.consumer"),cn=Symbol.for("react.context"),vf=Symbol.for("react.forward_ref"),$m=Symbol.for("react.suspense"),wm=Symbol.for("react.suspense_list"),gf=Symbol.for("react.memo"),Un=Symbol.for("react.lazy");Symbol.for("react.scope");var Sm=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var tC=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var Hv=Symbol.iterator;function qi(e){return e===null||typeof e!="object"?null:(e=Hv&&e[Hv]||e["@@iterator"],typeof e=="function"?e:null)}var aC=Symbol.for("react.client.reference");function Nm(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===aC?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case fs:return"Fragment";case xm:return"Profiler";case py:return"StrictMode";case $m:return"Suspense";case wm:return"SuspenseList";case Sm:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case Ji:return"Portal";case cn:return(e.displayName||"Context")+".Provider";case hy:return(e._context.displayName||"Context")+".Consumer";case vf:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case gf:return t=e.displayName||null,t!==null?t:Nm(e.type)||"Memo";case Un:t=e._payload,e=e._init;try{return Nm(e(t))}catch{}}return null}var Xi=Array.isArray,ne=cy.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,ye=Xk.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,wr={pending:!1,data:null,method:null,action:null},_m=[],ps=-1;function Qa(e){return{current:e}}function dt(e){0>ps||(e.current=_m[ps],_m[ps]=null,ps--)}function Pe(e,t){ps++,_m[ps]=e.current,e.current=t}var Ia=Qa(null),ho=Qa(null),Gn=Qa(null),vu=Qa(null);function gu(e,t){switch(Pe(Gn,t),Pe(ho,e),Pe(Ia,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?Jg(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=Jg(t),e=D0(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}dt(Ia),Pe(Ia,e)}function Ms(){dt(Ia),dt(ho),dt(Gn)}function Rm(e){e.memoizedState!==null&&Pe(vu,e);var t=Ia.current,a=D0(t,e.type);t!==a&&(Pe(ho,e),Pe(Ia,a))}function yu(e){ho.current===e&&(dt(Ia),dt(ho)),vu.current===e&&(dt(vu),_o._currentValue=wr)}var km=Object.prototype.hasOwnProperty,yf=rt.unstable_scheduleCallback,Id=rt.unstable_cancelCallback,nC=rt.unstable_shouldYield,rC=rt.unstable_requestPaint,Ha=rt.unstable_now,sC=rt.unstable_getCurrentPriorityLevel,vy=rt.unstable_ImmediatePriority,gy=rt.unstable_UserBlockingPriority,bu=rt.unstable_NormalPriority,iC=rt.unstable_LowPriority,yy=rt.unstable_IdlePriority,oC=rt.log,lC=rt.unstable_setDisableYieldValue,To=null,Wt=null;function Hn(e){if(typeof oC=="function"&&lC(e),Wt&&typeof Wt.setStrictMode=="function")try{Wt.setStrictMode(To,e)}catch{}}var Zt=Math.clz32?Math.clz32:dC,uC=Math.log,cC=Math.LN2;function dC(e){return e>>>=0,e===0?32:31-(uC(e)/cC|0)|0}var zl=256,ql=4194304;function br(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Qu(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=br(n):(i&=o,i!==0?r=br(i):a||(a=o&~e,a!==0&&(r=br(a))))):(o=n&~s,o!==0?r=br(o):i!==0?r=br(i):a||(a=n&~e,a!==0&&(r=br(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function Ao(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function mC(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function by(){var e=zl;return zl<<=1,(zl&4194048)===0&&(zl=256),e}function xy(){var e=ql;return ql<<=1,(ql&62914560)===0&&(ql=4194304),e}function Hd(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function Do(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function fC(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,l=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Zt(a),m=1<<d;o[d]=0,l[d]=-1;var f=c[d];if(f!==null)for(c[d]=null,d=0;d<f.length;d++){var h=f[d];h!==null&&(h.lane&=-536870913)}a&=~m}n!==0&&$y(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function $y(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Zt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function wy(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Zt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function bf(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function xf(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function Sy(){var e=ye.p;return e!==0?e:(e=window.event,e===void 0?32:q0(e.type))}function pC(e,t){var a=ye.p;try{return ye.p=e,t()}finally{ye.p=a}}var sr=Math.random().toString(36).slice(2),yt="__reactFiber$"+sr,zt="__reactProps$"+sr,Hs="__reactContainer$"+sr,Cm="__reactEvents$"+sr,hC="__reactListeners$"+sr,vC="__reactHandles$"+sr,Kv="__reactResources$"+sr,Mo="__reactMarker$"+sr;function $f(e){delete e[yt],delete e[zt],delete e[Cm],delete e[hC],delete e[vC]}function hs(e){var t=e[yt];if(t)return t;for(var a=e.parentNode;a;){if(t=a[Hs]||a[yt]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=Zg(e);e!==null;){if(a=e[yt])return a;e=Zg(e)}return t}e=a,a=e.parentNode}return null}function Ks(e){if(e=e[yt]||e[Hs]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function Wi(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(j(33))}function _s(e){var t=e[Kv];return t||(t=e[Kv]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function ut(e){e[Mo]=!0}var Ny=new Set,_y={};function Mr(e,t){Os(e,t),Os(e+"Capture",t)}function Os(e,t){for(_y[e]=t,e=0;e<t.length;e++)Ny.add(t[e])}var gC=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),Qv={},Vv={};function yC(e){return km.call(Vv,e)?!0:km.call(Qv,e)?!1:gC.test(e)?Vv[e]=!0:(Qv[e]=!0,!1)}function nu(e,t,a){if(yC(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function Il(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function sn(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var Kd,Gv;function cs(e){if(Kd===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);Kd=t&&t[1]||"",Gv=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+Kd+e+Gv}var Qd=!1;function Vd(e,t){if(!e||Qd)return"";Qd=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var m=function(){throw Error()};if(Object.defineProperty(m.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(m,[])}catch(h){var f=h}Reflect.construct(e,[],m)}else{try{m.call()}catch(h){f=h}e.call(m.prototype)}}else{try{throw Error()}catch(h){f=h}(m=e())&&typeof m.catch=="function"&&m.catch(function(){})}}catch(h){if(h&&f&&typeof h.stack=="string")return[h.stack,f.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var l=i.split(`
`),c=o.split(`
`);for(r=n=0;n<l.length&&!l[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===l.length||r===c.length)for(n=l.length-1,r=c.length-1;1<=n&&0<=r&&l[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(l[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||l[n]!==c[r]){var d=`
`+l[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{Qd=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?cs(a):""}function bC(e){switch(e.tag){case 26:case 27:case 5:return cs(e.type);case 16:return cs("Lazy");case 13:return cs("Suspense");case 19:return cs("SuspenseList");case 0:case 15:return Vd(e.type,!1);case 11:return Vd(e.type.render,!1);case 1:return Vd(e.type,!0);case 31:return cs("Activity");default:return""}}function Yv(e){try{var t="";do t+=bC(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function da(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function Ry(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function xC(e){var t=Ry(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function xu(e){e._valueTracker||(e._valueTracker=xC(e))}function ky(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=Ry(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function $u(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var $C=/[\n"\\]/g;function pa(e){return e.replace($C,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function Em(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+da(t)):e.value!==""+da(t)&&(e.value=""+da(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?Tm(e,i,da(t)):a!=null?Tm(e,i,da(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+da(o):e.removeAttribute("name")}function Cy(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+da(a):"",t=t!=null?""+da(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function Tm(e,t,a){t==="number"&&$u(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function Rs(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+da(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function Ey(e,t,a){if(t!=null&&(t=""+da(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+da(a):""}function Ty(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(j(92));if(Xi(n)){if(1<n.length)throw Error(j(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=da(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function Ls(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var wC=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function Jv(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||wC.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function Ay(e,t,a){if(t!=null&&typeof t!="object")throw Error(j(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&Jv(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&Jv(e,s,t[s])}function wf(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var SC=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),NC=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function ru(e){return NC.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var Am=null;function Sf(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var vs=null,ks=null;function Xv(e){var t=Ks(e);if(t&&(e=t.stateNode)){var a=e[zt]||null;e:switch(e=t.stateNode,t.type){case"input":if(Em(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+pa(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[zt]||null;if(!r)throw Error(j(90));Em(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&ky(n)}break e;case"textarea":Ey(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&Rs(e,!!a.multiple,t,!1)}}}var Gd=!1;function Dy(e,t,a){if(Gd)return e(t,a);Gd=!0;try{var n=e(t);return n}finally{if(Gd=!1,(vs!==null||ks!==null)&&(ac(),vs&&(t=vs,e=ks,ks=vs=null,Xv(t),e)))for(t=0;t<e.length;t++)Xv(e[t])}}function vo(e,t){var a=e.stateNode;if(a===null)return null;var n=a[zt]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(j(231,t,typeof a));return a}var gn=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),Dm=!1;if(gn)try{os={},Object.defineProperty(os,"passive",{get:function(){Dm=!0}}),window.addEventListener("test",os,os),window.removeEventListener("test",os,os)}catch{Dm=!1}var os,Kn=null,Nf=null,su=null;function My(){if(su)return su;var e,t=Nf,a=t.length,n,r="value"in Kn?Kn.value:Kn.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return su=r.slice(e,1<n?1-n:void 0)}function iu(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function Hl(){return!0}function Wv(){return!1}function qt(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?Hl:Wv,this.isPropagationStopped=Wv,this}return De(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=Hl)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=Hl)},persist:function(){},isPersistent:Hl}),t}var Or={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Vu=qt(Or),Oo=De({},Or,{view:0,detail:0}),_C=qt(Oo),Yd,Jd,Ii,Gu=De({},Oo,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:_f,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==Ii&&(Ii&&e.type==="mousemove"?(Yd=e.screenX-Ii.screenX,Jd=e.screenY-Ii.screenY):Jd=Yd=0,Ii=e),Yd)},movementY:function(e){return"movementY"in e?e.movementY:Jd}}),Zv=qt(Gu),RC=De({},Gu,{dataTransfer:0}),kC=qt(RC),CC=De({},Oo,{relatedTarget:0}),Xd=qt(CC),EC=De({},Or,{animationName:0,elapsedTime:0,pseudoElement:0}),TC=qt(EC),AC=De({},Or,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),DC=qt(AC),MC=De({},Or,{data:0}),eg=qt(MC),OC={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},LC={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},PC={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function jC(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=PC[e])?!!t[e]:!1}function _f(){return jC}var UC=De({},Oo,{key:function(e){if(e.key){var t=OC[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=iu(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?LC[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:_f,charCode:function(e){return e.type==="keypress"?iu(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?iu(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),FC=qt(UC),BC=De({},Gu,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),tg=qt(BC),zC=De({},Oo,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:_f}),qC=qt(zC),IC=De({},Or,{propertyName:0,elapsedTime:0,pseudoElement:0}),HC=qt(IC),KC=De({},Gu,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),QC=qt(KC),VC=De({},Or,{newState:0,oldState:0}),GC=qt(VC),YC=[9,13,27,32],Rf=gn&&"CompositionEvent"in window,eo=null;gn&&"documentMode"in document&&(eo=document.documentMode);var JC=gn&&"TextEvent"in window&&!eo,Oy=gn&&(!Rf||eo&&8<eo&&11>=eo),ag=" ",ng=!1;function Ly(e,t){switch(e){case"keyup":return YC.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function Py(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var gs=!1;function XC(e,t){switch(e){case"compositionend":return Py(t);case"keypress":return t.which!==32?null:(ng=!0,ag);case"textInput":return e=t.data,e===ag&&ng?null:e;default:return null}}function WC(e,t){if(gs)return e==="compositionend"||!Rf&&Ly(e,t)?(e=My(),su=Nf=Kn=null,gs=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return Oy&&t.locale!=="ko"?null:t.data;default:return null}}var ZC={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function rg(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!ZC[e.type]:t==="textarea"}function jy(e,t,a,n){vs?ks?ks.push(n):ks=[n]:vs=n,t=Fu(t,"onChange"),0<t.length&&(a=new Vu("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var to=null,go=null;function eE(e){E0(e,0)}function Yu(e){var t=Wi(e);if(ky(t))return e}function sg(e,t){if(e==="change")return t}var Uy=!1;gn&&(gn?(Ql="oninput"in document,Ql||(Wd=document.createElement("div"),Wd.setAttribute("oninput","return;"),Ql=typeof Wd.oninput=="function"),Kl=Ql):Kl=!1,Uy=Kl&&(!document.documentMode||9<document.documentMode));var Kl,Ql,Wd;function ig(){to&&(to.detachEvent("onpropertychange",Fy),go=to=null)}function Fy(e){if(e.propertyName==="value"&&Yu(go)){var t=[];jy(t,go,e,Sf(e)),Dy(eE,t)}}function tE(e,t,a){e==="focusin"?(ig(),to=t,go=a,to.attachEvent("onpropertychange",Fy)):e==="focusout"&&ig()}function aE(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return Yu(go)}function nE(e,t){if(e==="click")return Yu(t)}function rE(e,t){if(e==="input"||e==="change")return Yu(t)}function sE(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var aa=typeof Object.is=="function"?Object.is:sE;function yo(e,t){if(aa(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!km.call(t,r)||!aa(e[r],t[r]))return!1}return!0}function og(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function lg(e,t){var a=og(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=og(a)}}function By(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?By(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function zy(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=$u(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=$u(e.document)}return t}function kf(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var iE=gn&&"documentMode"in document&&11>=document.documentMode,ys=null,Mm=null,ao=null,Om=!1;function ug(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;Om||ys==null||ys!==$u(n)||(n=ys,"selectionStart"in n&&kf(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),ao&&yo(ao,n)||(ao=n,n=Fu(Mm,"onSelect"),0<n.length&&(t=new Vu("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=ys)))}function yr(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var bs={animationend:yr("Animation","AnimationEnd"),animationiteration:yr("Animation","AnimationIteration"),animationstart:yr("Animation","AnimationStart"),transitionrun:yr("Transition","TransitionRun"),transitionstart:yr("Transition","TransitionStart"),transitioncancel:yr("Transition","TransitionCancel"),transitionend:yr("Transition","TransitionEnd")},Zd={},qy={};gn&&(qy=document.createElement("div").style,"AnimationEvent"in window||(delete bs.animationend.animation,delete bs.animationiteration.animation,delete bs.animationstart.animation),"TransitionEvent"in window||delete bs.transitionend.transition);function Lr(e){if(Zd[e])return Zd[e];if(!bs[e])return e;var t=bs[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in qy)return Zd[e]=t[a];return e}var Iy=Lr("animationend"),Hy=Lr("animationiteration"),Ky=Lr("animationstart"),oE=Lr("transitionrun"),lE=Lr("transitionstart"),uE=Lr("transitioncancel"),Qy=Lr("transitionend"),Vy=new Map,Lm="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");Lm.push("scrollEnd");function Ra(e,t){Vy.set(e,t),Mr(t,[e])}var cg=new WeakMap;function ha(e,t){if(typeof e=="object"&&e!==null){var a=cg.get(e);return a!==void 0?a:(t={value:e,source:t,stack:Yv(t)},cg.set(e,t),t)}return{value:e,source:t,stack:Yv(t)}}var ca=[],xs=0,Cf=0;function Ju(){for(var e=xs,t=Cf=xs=0;t<e;){var a=ca[t];ca[t++]=null;var n=ca[t];ca[t++]=null;var r=ca[t];ca[t++]=null;var s=ca[t];if(ca[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&Gy(a,r,s)}}function Xu(e,t,a,n){ca[xs++]=e,ca[xs++]=t,ca[xs++]=a,ca[xs++]=n,Cf|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function Ef(e,t,a,n){return Xu(e,t,a,n),wu(e)}function Qs(e,t){return Xu(e,null,null,t),wu(e)}function Gy(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Zt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function wu(e){if(50<fo)throw fo=0,af=null,Error(j(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var $s={};function cE(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Xt(e,t,a,n){return new cE(e,t,a,n)}function Tf(e){return e=e.prototype,!(!e||!e.isReactComponent)}function hn(e,t){var a=e.alternate;return a===null?(a=Xt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function Yy(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function ou(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")Tf(e)&&(i=1);else if(typeof e=="string")i=c3(e,a,Ia.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case Sm:return e=Xt(31,a,t,r),e.elementType=Sm,e.lanes=s,e;case fs:return Sr(a.children,r,s,t);case py:i=8,r|=24;break;case xm:return e=Xt(12,a,t,r|2),e.elementType=xm,e.lanes=s,e;case $m:return e=Xt(13,a,t,r),e.elementType=$m,e.lanes=s,e;case wm:return e=Xt(19,a,t,r),e.elementType=wm,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case eC:case cn:i=10;break e;case hy:i=9;break e;case vf:i=11;break e;case gf:i=14;break e;case Un:i=16,n=null;break e}i=29,a=Error(j(130,e===null?"null":typeof e,"")),n=null}return t=Xt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function Sr(e,t,a,n){return e=Xt(7,e,n,t),e.lanes=a,e}function em(e,t,a){return e=Xt(6,e,null,t),e.lanes=a,e}function tm(e,t,a){return t=Xt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var ws=[],Ss=0,Su=null,Nu=0,ma=[],fa=0,Nr=null,dn=1,mn="";function xr(e,t){ws[Ss++]=Nu,ws[Ss++]=Su,Su=e,Nu=t}function Jy(e,t,a){ma[fa++]=dn,ma[fa++]=mn,ma[fa++]=Nr,Nr=e;var n=dn;e=mn;var r=32-Zt(n)-1;n&=~(1<<r),a+=1;var s=32-Zt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,dn=1<<32-Zt(t)+r|a<<r|n,mn=s+e}else dn=1<<s|a<<r|n,mn=e}function Af(e){e.return!==null&&(xr(e,1),Jy(e,1,0))}function Df(e){for(;e===Su;)Su=ws[--Ss],ws[Ss]=null,Nu=ws[--Ss],ws[Ss]=null;for(;e===Nr;)Nr=ma[--fa],ma[fa]=null,mn=ma[--fa],ma[fa]=null,dn=ma[--fa],ma[fa]=null}var Tt=null,qe=null,ge=!1,_r=null,za=!1,Pm=Error(j(519));function Er(e){var t=Error(j(418,""));throw bo(ha(t,e)),Pm}function dg(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[yt]=e,t[zt]=n,a){case"dialog":le("cancel",t),le("close",t);break;case"iframe":case"object":case"embed":le("load",t);break;case"video":case"audio":for(a=0;a<wo.length;a++)le(wo[a],t);break;case"source":le("error",t);break;case"img":case"image":case"link":le("error",t),le("load",t);break;case"details":le("toggle",t);break;case"input":le("invalid",t),Cy(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),xu(t);break;case"select":le("invalid",t);break;case"textarea":le("invalid",t),Ty(t,n.value,n.defaultValue,n.children),xu(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||A0(t.textContent,a)?(n.popover!=null&&(le("beforetoggle",t),le("toggle",t)),n.onScroll!=null&&le("scroll",t),n.onScrollEnd!=null&&le("scrollend",t),n.onClick!=null&&(t.onclick=sc),t=!0):t=!1,t||Er(e)}function mg(e){for(Tt=e.return;Tt;)switch(Tt.tag){case 5:case 13:za=!1;return;case 27:case 3:za=!0;return;default:Tt=Tt.return}}function Hi(e){if(e!==Tt)return!1;if(!ge)return mg(e),ge=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||uf(e.type,e.memoizedProps)),a=!a),a&&qe&&Er(e),mg(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(j(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){qe=_a(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}qe=null}}else t===27?(t=qe,ir(e.type)?(e=mf,mf=null,qe=e):qe=t):qe=Tt?_a(e.stateNode.nextSibling):null;return!0}function Lo(){qe=Tt=null,ge=!1}function fg(){var e=_r;return e!==null&&(Bt===null?Bt=e:Bt.push.apply(Bt,e),_r=null),e}function bo(e){_r===null?_r=[e]:_r.push(e)}var jm=Qa(null),Pr=null,fn=null;function Bn(e,t,a){Pe(jm,t._currentValue),t._currentValue=a}function vn(e){e._currentValue=jm.current,dt(jm)}function Um(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function Fm(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var l=0;l<t.length;l++)if(o.context===t[l]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),Um(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(j(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),Um(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function Po(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(j(387));if(i=i.memoizedProps,i!==null){var o=r.type;aa(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===vu.current){if(i=r.alternate,i===null)throw Error(j(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(_o):e=[_o])}r=r.return}e!==null&&Fm(t,e,a,n),t.flags|=262144}function _u(e){for(e=e.firstContext;e!==null;){if(!aa(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function Tr(e){Pr=e,fn=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function bt(e){return Xy(Pr,e)}function Vl(e,t){return Pr===null&&Tr(e),Xy(e,t)}function Xy(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},fn===null){if(e===null)throw Error(j(308));fn=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else fn=fn.next=t;return a}var dE=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},mE=rt.unstable_scheduleCallback,fE=rt.unstable_NormalPriority,at={$$typeof:cn,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function Mf(){return{controller:new dE,data:new Map,refCount:0}}function jo(e){e.refCount--,e.refCount===0&&mE(fE,function(){e.controller.abort()})}var no=null,Bm=0,Ps=0,Cs=null;function pE(e,t){if(no===null){var a=no=[];Bm=0,Ps=ap(),Cs={status:"pending",value:void 0,then:function(n){a.push(n)}}}return Bm++,t.then(pg,pg),t}function pg(){if(--Bm===0&&no!==null){Cs!==null&&(Cs.status="fulfilled");var e=no;no=null,Ps=0,Cs=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function hE(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var hg=ne.S;ne.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&pE(e,t),hg!==null&&hg(e,t)};var Rr=Qa(null);function Of(){var e=Rr.current;return e!==null?e:Ee.pooledCache}function lu(e,t){t===null?Pe(Rr,Rr.current):Pe(Rr,t.pool)}function Wy(){var e=Of();return e===null?null:{parent:at._currentValue,pool:e}}var Uo=Error(j(460)),Zy=Error(j(474)),Wu=Error(j(542)),zm={then:function(){}};function vg(e){return e=e.status,e==="fulfilled"||e==="rejected"}function Gl(){}function eb(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(Gl,Gl),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,yg(e),e;default:if(typeof t.status=="string")t.then(Gl,Gl);else{if(e=Ee,e!==null&&100<e.shellSuspendCounter)throw Error(j(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,yg(e),e}throw ro=t,Uo}}var ro=null;function gg(){if(ro===null)throw Error(j(459));var e=ro;return ro=null,e}function yg(e){if(e===Uo||e===Wu)throw Error(j(483))}var Fn=!1;function Lf(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function qm(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function Yn(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function Jn(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,(we&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=wu(e),Gy(e,null,a),t}return Xu(e,n,t,a),wu(e)}function so(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,wy(e,a)}}function am(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var Im=!1;function io(){if(Im){var e=Cs;if(e!==null)throw e}}function oo(e,t,a,n){Im=!1;var r=e.updateQueue;Fn=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var l=o,c=l.next;l.next=null,i===null?s=c:i.next=c,i=l;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=l))}if(s!==null){var m=r.baseState;i=0,d=c=l=null,o=s;do{var f=o.lane&-536870913,h=f!==o.lane;if(h?(fe&f)===f:(n&f)===f){f!==0&&f===Ps&&(Im=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var x=e,y=o;f=t;var $=a;switch(y.tag){case 1:if(x=y.payload,typeof x=="function"){m=x.call($,m,f);break e}m=x;break e;case 3:x.flags=x.flags&-65537|128;case 0:if(x=y.payload,f=typeof x=="function"?x.call($,m,f):x,f==null)break e;m=De({},m,f);break e;case 2:Fn=!0}}f=o.callback,f!==null&&(e.flags|=64,h&&(e.flags|=8192),h=r.callbacks,h===null?r.callbacks=[f]:h.push(f))}else h={lane:f,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=h,l=m):d=d.next=h,i|=f;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;h=o,o=h.next,h.next=null,r.lastBaseUpdate=h,r.shared.pending=null}}while(!0);d===null&&(l=m),r.baseState=l,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),rr|=i,e.lanes=i,e.memoizedState=m}}function tb(e,t){if(typeof e!="function")throw Error(j(191,e));e.call(t)}function ab(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)tb(a[e],t)}var js=Qa(null),Ru=Qa(0);function bg(e,t){e=xn,Pe(Ru,e),Pe(js,t),xn=e|t.baseLanes}function Hm(){Pe(Ru,xn),Pe(js,js.current)}function Pf(){xn=Ru.current,dt(js),dt(Ru)}var ar=0,oe=null,_e=null,Ye=null,ku=!1,Es=!1,Ar=!1,Cu=0,xo=0,Ts=null,vE=0;function Qe(){throw Error(j(321))}function jf(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!aa(e[a],t[a]))return!1;return!0}function Uf(e,t,a,n,r,s){return ar=s,oe=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,ne.H=e===null||e.memoizedState===null?Mb:Ob,Ar=!1,s=a(n,r),Ar=!1,Es&&(s=rb(t,a,n,r)),nb(e),s}function nb(e){ne.H=Eu;var t=_e!==null&&_e.next!==null;if(ar=0,Ye=_e=oe=null,ku=!1,xo=0,Ts=null,t)throw Error(j(300));e===null||ct||(e=e.dependencies,e!==null&&_u(e)&&(ct=!0))}function rb(e,t,a,n){oe=e;var r=0;do{if(Es&&(Ts=null),xo=0,Es=!1,25<=r)throw Error(j(301));if(r+=1,Ye=_e=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}ne.H=SE,s=t(a,n)}while(Es);return s}function gE(){var e=ne.H,t=e.useState()[0];return t=typeof t.then=="function"?Fo(t):t,e=e.useState()[0],(_e!==null?_e.memoizedState:null)!==e&&(oe.flags|=1024),t}function Ff(){var e=Cu!==0;return Cu=0,e}function Bf(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function zf(e){if(ku){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}ku=!1}ar=0,Ye=_e=oe=null,Es=!1,xo=Cu=0,Ts=null}function Ut(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return Ye===null?oe.memoizedState=Ye=e:Ye=Ye.next=e,Ye}function Je(){if(_e===null){var e=oe.alternate;e=e!==null?e.memoizedState:null}else e=_e.next;var t=Ye===null?oe.memoizedState:Ye.next;if(t!==null)Ye=t,_e=e;else{if(e===null)throw oe.alternate===null?Error(j(467)):Error(j(310));_e=e,e={memoizedState:_e.memoizedState,baseState:_e.baseState,baseQueue:_e.baseQueue,queue:_e.queue,next:null},Ye===null?oe.memoizedState=Ye=e:Ye=Ye.next=e}return Ye}function qf(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function Fo(e){var t=xo;return xo+=1,Ts===null&&(Ts=[]),e=eb(Ts,e,t),t=oe,(Ye===null?t.memoizedState:Ye.next)===null&&(t=t.alternate,ne.H=t===null||t.memoizedState===null?Mb:Ob),e}function Zu(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return Fo(e);if(e.$$typeof===cn)return bt(e)}throw Error(j(438,String(e)))}function If(e){var t=null,a=oe.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=oe.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=qf(),oe.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=tC;return t.index++,a}function yn(e,t){return typeof t=="function"?t(e):t}function uu(e){var t=Je();return Hf(t,_e,e)}function Hf(e,t,a){var n=e.queue;if(n===null)throw Error(j(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,l=null,c=t,d=!1;do{var m=c.lane&-536870913;if(m!==c.lane?(fe&m)===m:(ar&m)===m){var f=c.revertLane;if(f===0)l!==null&&(l=l.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),m===Ps&&(d=!0);else if((ar&f)===f){c=c.next,f===Ps&&(d=!0);continue}else m={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},l===null?(o=l=m,i=s):l=l.next=m,oe.lanes|=f,rr|=f;m=c.action,Ar&&a(s,m),s=c.hasEagerState?c.eagerState:a(s,m)}else f={lane:m,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},l===null?(o=l=f,i=s):l=l.next=f,oe.lanes|=m,rr|=m;c=c.next}while(c!==null&&c!==t);if(l===null?i=s:l.next=o,!aa(s,e.memoizedState)&&(ct=!0,d&&(a=Cs,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=l,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function nm(e){var t=Je(),a=t.queue;if(a===null)throw Error(j(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);aa(s,t.memoizedState)||(ct=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function sb(e,t,a){var n=oe,r=Je(),s=ge;if(s){if(a===void 0)throw Error(j(407));a=a()}else a=t();var i=!aa((_e||r).memoizedState,a);i&&(r.memoizedState=a,ct=!0),r=r.queue;var o=lb.bind(null,n,r,e);if(Bo(2048,8,o,[e]),r.getSnapshot!==t||i||Ye!==null&&Ye.memoizedState.tag&1){if(n.flags|=2048,Us(9,ec(),ob.bind(null,n,r,a,t),null),Ee===null)throw Error(j(349));s||(ar&124)!==0||ib(n,t,a)}return a}function ib(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=oe.updateQueue,t===null?(t=qf(),oe.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function ob(e,t,a,n){t.value=a,t.getSnapshot=n,ub(t)&&cb(e)}function lb(e,t,a){return a(function(){ub(t)&&cb(e)})}function ub(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!aa(e,a)}catch{return!0}}function cb(e){var t=Qs(e,2);t!==null&&ta(t,e,2)}function Km(e){var t=Ut();if(typeof e=="function"){var a=e;if(e=a(),Ar){Hn(!0);try{a()}finally{Hn(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:yn,lastRenderedState:e},t}function db(e,t,a,n){return e.baseState=a,Hf(e,_e,typeof n=="function"?n:yn)}function yE(e,t,a,n,r){if(tc(e))throw Error(j(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};ne.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,mb(t,s)):(s.next=a.next,t.pending=a.next=s)}}function mb(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=ne.T,i={};ne.T=i;try{var o=a(r,n),l=ne.S;l!==null&&l(i,o),xg(e,t,o)}catch(c){Qm(e,t,c)}finally{ne.T=s}}else try{s=a(r,n),xg(e,t,s)}catch(c){Qm(e,t,c)}}function xg(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){$g(e,t,n)},function(n){return Qm(e,t,n)}):$g(e,t,a)}function $g(e,t,a){t.status="fulfilled",t.value=a,fb(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,mb(e,a)))}function Qm(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,fb(t),t=t.next;while(t!==n)}e.action=null}function fb(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function pb(e,t){return t}function wg(e,t){if(ge){var a=Ee.formState;if(a!==null){e:{var n=oe;if(ge){if(qe){t:{for(var r=qe,s=za;r.nodeType!==8;){if(!s){r=null;break t}if(r=_a(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){qe=_a(r.nextSibling),n=r.data==="F!";break e}}Er(n)}n=!1}n&&(t=a[0])}}return a=Ut(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:pb,lastRenderedState:t},a.queue=n,a=Tb.bind(null,oe,n),n.dispatch=a,n=Km(!1),s=Gf.bind(null,oe,!1,n.queue),n=Ut(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=yE.bind(null,oe,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function Sg(e){var t=Je();return hb(t,_e,e)}function hb(e,t,a){if(t=Hf(e,t,pb)[0],e=uu(yn)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=Fo(t)}catch(i){throw i===Uo?Wu:i}else n=t;t=Je();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(oe.flags|=2048,Us(9,ec(),bE.bind(null,r,a),null)),[n,s,e]}function bE(e,t){e.action=t}function Ng(e){var t=Je(),a=_e;if(a!==null)return hb(t,a,e);Je(),t=t.memoizedState,a=Je();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function Us(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=oe.updateQueue,t===null&&(t=qf(),oe.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function ec(){return{destroy:void 0,resource:void 0}}function vb(){return Je().memoizedState}function cu(e,t,a,n){var r=Ut();n=n===void 0?null:n,oe.flags|=e,r.memoizedState=Us(1|t,ec(),a,n)}function Bo(e,t,a,n){var r=Je();n=n===void 0?null:n;var s=r.memoizedState.inst;_e!==null&&n!==null&&jf(n,_e.memoizedState.deps)?r.memoizedState=Us(t,s,a,n):(oe.flags|=e,r.memoizedState=Us(1|t,s,a,n))}function _g(e,t){cu(8390656,8,e,t)}function gb(e,t){Bo(2048,8,e,t)}function yb(e,t){return Bo(4,2,e,t)}function bb(e,t){return Bo(4,4,e,t)}function xb(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function $b(e,t,a){a=a!=null?a.concat([e]):null,Bo(4,4,xb.bind(null,t,e),a)}function Kf(){}function wb(e,t){var a=Je();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&jf(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function Sb(e,t){var a=Je();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&jf(t,n[1]))return n[0];if(n=e(),Ar){Hn(!0);try{e()}finally{Hn(!1)}}return a.memoizedState=[n,t],n}function Qf(e,t,a){return a===void 0||(ar&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=f0(),oe.lanes|=e,rr|=e,a)}function Nb(e,t,a,n){return aa(a,t)?a:js.current!==null?(e=Qf(e,a,n),aa(e,t)||(ct=!0),e):(ar&42)===0?(ct=!0,e.memoizedState=a):(e=f0(),oe.lanes|=e,rr|=e,t)}function _b(e,t,a,n,r){var s=ye.p;ye.p=s!==0&&8>s?s:8;var i=ne.T,o={};ne.T=o,Gf(e,!1,t,a);try{var l=r(),c=ne.S;if(c!==null&&c(o,l),l!==null&&typeof l=="object"&&typeof l.then=="function"){var d=hE(l,n);lo(e,t,d,ea(e))}else lo(e,t,n,ea(e))}catch(m){lo(e,t,{then:function(){},status:"rejected",reason:m},ea())}finally{ye.p=s,ne.T=i}}function xE(){}function Vm(e,t,a,n){if(e.tag!==5)throw Error(j(476));var r=Rb(e).queue;_b(e,r,t,wr,a===null?xE:function(){return kb(e),a(n)})}function Rb(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:wr,baseState:wr,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:yn,lastRenderedState:wr},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:yn,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function kb(e){var t=Rb(e).next.queue;lo(e,t,{},ea())}function Vf(){return bt(_o)}function Cb(){return Je().memoizedState}function Eb(){return Je().memoizedState}function $E(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=ea();e=Yn(a);var n=Jn(t,e,a);n!==null&&(ta(n,t,a),so(n,t,a)),t={cache:Mf()},e.payload=t;return}t=t.return}}function wE(e,t,a){var n=ea();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},tc(e)?Ab(t,a):(a=Ef(e,t,a,n),a!==null&&(ta(a,e,n),Db(a,t,n)))}function Tb(e,t,a){var n=ea();lo(e,t,a,n)}function lo(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(tc(e))Ab(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,aa(o,i))return Xu(e,t,r,0),Ee===null&&Ju(),!1}catch{}finally{}if(a=Ef(e,t,r,n),a!==null)return ta(a,e,n),Db(a,t,n),!0}return!1}function Gf(e,t,a,n){if(n={lane:2,revertLane:ap(),action:n,hasEagerState:!1,eagerState:null,next:null},tc(e)){if(t)throw Error(j(479))}else t=Ef(e,a,n,2),t!==null&&ta(t,e,2)}function tc(e){var t=e.alternate;return e===oe||t!==null&&t===oe}function Ab(e,t){Es=ku=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function Db(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,wy(e,a)}}var Eu={readContext:bt,use:Zu,useCallback:Qe,useContext:Qe,useEffect:Qe,useImperativeHandle:Qe,useLayoutEffect:Qe,useInsertionEffect:Qe,useMemo:Qe,useReducer:Qe,useRef:Qe,useState:Qe,useDebugValue:Qe,useDeferredValue:Qe,useTransition:Qe,useSyncExternalStore:Qe,useId:Qe,useHostTransitionStatus:Qe,useFormState:Qe,useActionState:Qe,useOptimistic:Qe,useMemoCache:Qe,useCacheRefresh:Qe},Mb={readContext:bt,use:Zu,useCallback:function(e,t){return Ut().memoizedState=[e,t===void 0?null:t],e},useContext:bt,useEffect:_g,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,cu(4194308,4,xb.bind(null,t,e),a)},useLayoutEffect:function(e,t){return cu(4194308,4,e,t)},useInsertionEffect:function(e,t){cu(4,2,e,t)},useMemo:function(e,t){var a=Ut();t=t===void 0?null:t;var n=e();if(Ar){Hn(!0);try{e()}finally{Hn(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=Ut();if(a!==void 0){var r=a(t);if(Ar){Hn(!0);try{a(t)}finally{Hn(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=wE.bind(null,oe,e),[n.memoizedState,e]},useRef:function(e){var t=Ut();return e={current:e},t.memoizedState=e},useState:function(e){e=Km(e);var t=e.queue,a=Tb.bind(null,oe,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:Kf,useDeferredValue:function(e,t){var a=Ut();return Qf(a,e,t)},useTransition:function(){var e=Km(!1);return e=_b.bind(null,oe,e.queue,!0,!1),Ut().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=oe,r=Ut();if(ge){if(a===void 0)throw Error(j(407));a=a()}else{if(a=t(),Ee===null)throw Error(j(349));(fe&124)!==0||ib(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,_g(lb.bind(null,n,s,e),[e]),n.flags|=2048,Us(9,ec(),ob.bind(null,n,s,a,t),null),a},useId:function(){var e=Ut(),t=Ee.identifierPrefix;if(ge){var a=mn,n=dn;a=(n&~(1<<32-Zt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=Cu++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=vE++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:Vf,useFormState:wg,useActionState:wg,useOptimistic:function(e){var t=Ut();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=Gf.bind(null,oe,!0,a),a.dispatch=t,[e,t]},useMemoCache:If,useCacheRefresh:function(){return Ut().memoizedState=$E.bind(null,oe)}},Ob={readContext:bt,use:Zu,useCallback:wb,useContext:bt,useEffect:gb,useImperativeHandle:$b,useInsertionEffect:yb,useLayoutEffect:bb,useMemo:Sb,useReducer:uu,useRef:vb,useState:function(){return uu(yn)},useDebugValue:Kf,useDeferredValue:function(e,t){var a=Je();return Nb(a,_e.memoizedState,e,t)},useTransition:function(){var e=uu(yn)[0],t=Je().memoizedState;return[typeof e=="boolean"?e:Fo(e),t]},useSyncExternalStore:sb,useId:Cb,useHostTransitionStatus:Vf,useFormState:Sg,useActionState:Sg,useOptimistic:function(e,t){var a=Je();return db(a,_e,e,t)},useMemoCache:If,useCacheRefresh:Eb},SE={readContext:bt,use:Zu,useCallback:wb,useContext:bt,useEffect:gb,useImperativeHandle:$b,useInsertionEffect:yb,useLayoutEffect:bb,useMemo:Sb,useReducer:nm,useRef:vb,useState:function(){return nm(yn)},useDebugValue:Kf,useDeferredValue:function(e,t){var a=Je();return _e===null?Qf(a,e,t):Nb(a,_e.memoizedState,e,t)},useTransition:function(){var e=nm(yn)[0],t=Je().memoizedState;return[typeof e=="boolean"?e:Fo(e),t]},useSyncExternalStore:sb,useId:Cb,useHostTransitionStatus:Vf,useFormState:Ng,useActionState:Ng,useOptimistic:function(e,t){var a=Je();return _e!==null?db(a,_e,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:If,useCacheRefresh:Eb},As=null,$o=0;function Yl(e){var t=$o;return $o+=1,As===null&&(As=[]),eb(As,e,t)}function Ki(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function Jl(e,t){throw t.$$typeof===Zk?Error(j(525)):(e=Object.prototype.toString.call(t),Error(j(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function Rg(e){var t=e._init;return t(e._payload)}function Lb(e){function t(g,v){if(e){var b=g.deletions;b===null?(g.deletions=[v],g.flags|=16):b.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=hn(g,v),g.index=0,g.sibling=null,g}function s(g,v,b){return g.index=b,e?(b=g.alternate,b!==null?(b=b.index,b<v?(g.flags|=67108866,v):b):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,b,w){return v===null||v.tag!==6?(v=em(b,g.mode,w),v.return=g,v):(v=r(v,b),v.return=g,v)}function l(g,v,b,w){var S=b.type;return S===fs?d(g,v,b.props.children,w,b.key):v!==null&&(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Un&&Rg(S)===v.type)?(v=r(v,b.props),Ki(v,b),v.return=g,v):(v=ou(b.type,b.key,b.props,null,g.mode,w),Ki(v,b),v.return=g,v)}function c(g,v,b,w){return v===null||v.tag!==4||v.stateNode.containerInfo!==b.containerInfo||v.stateNode.implementation!==b.implementation?(v=tm(b,g.mode,w),v.return=g,v):(v=r(v,b.children||[]),v.return=g,v)}function d(g,v,b,w,S){return v===null||v.tag!==7?(v=Sr(b,g.mode,w,S),v.return=g,v):(v=r(v,b),v.return=g,v)}function m(g,v,b){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=em(""+v,g.mode,b),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case Bl:return b=ou(v.type,v.key,v.props,null,g.mode,b),Ki(b,v),b.return=g,b;case Ji:return v=tm(v,g.mode,b),v.return=g,v;case Un:var w=v._init;return v=w(v._payload),m(g,v,b)}if(Xi(v)||qi(v))return v=Sr(v,g.mode,b,null),v.return=g,v;if(typeof v.then=="function")return m(g,Yl(v),b);if(v.$$typeof===cn)return m(g,Vl(g,v),b);Jl(g,v)}return null}function f(g,v,b,w){var S=v!==null?v.key:null;if(typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint")return S!==null?null:o(g,v,""+b,w);if(typeof b=="object"&&b!==null){switch(b.$$typeof){case Bl:return b.key===S?l(g,v,b,w):null;case Ji:return b.key===S?c(g,v,b,w):null;case Un:return S=b._init,b=S(b._payload),f(g,v,b,w)}if(Xi(b)||qi(b))return S!==null?null:d(g,v,b,w,null);if(typeof b.then=="function")return f(g,v,Yl(b),w);if(b.$$typeof===cn)return f(g,v,Vl(g,b),w);Jl(g,b)}return null}function h(g,v,b,w,S){if(typeof w=="string"&&w!==""||typeof w=="number"||typeof w=="bigint")return g=g.get(b)||null,o(v,g,""+w,S);if(typeof w=="object"&&w!==null){switch(w.$$typeof){case Bl:return g=g.get(w.key===null?b:w.key)||null,l(v,g,w,S);case Ji:return g=g.get(w.key===null?b:w.key)||null,c(v,g,w,S);case Un:var E=w._init;return w=E(w._payload),h(g,v,b,w,S)}if(Xi(w)||qi(w))return g=g.get(b)||null,d(v,g,w,S,null);if(typeof w.then=="function")return h(g,v,b,Yl(w),S);if(w.$$typeof===cn)return h(g,v,b,Vl(v,w),S);Jl(v,w)}return null}function x(g,v,b,w){for(var S=null,E=null,N=v,T=v=0,L=null;N!==null&&T<b.length;T++){N.index>T?(L=N,N=null):L=N.sibling;var O=f(g,N,b[T],w);if(O===null){N===null&&(N=L);break}e&&N&&O.alternate===null&&t(g,N),v=s(O,v,T),E===null?S=O:E.sibling=O,E=O,N=L}if(T===b.length)return a(g,N),ge&&xr(g,T),S;if(N===null){for(;T<b.length;T++)N=m(g,b[T],w),N!==null&&(v=s(N,v,T),E===null?S=N:E.sibling=N,E=N);return ge&&xr(g,T),S}for(N=n(N);T<b.length;T++)L=h(N,g,T,b[T],w),L!==null&&(e&&L.alternate!==null&&N.delete(L.key===null?T:L.key),v=s(L,v,T),E===null?S=L:E.sibling=L,E=L);return e&&N.forEach(function(P){return t(g,P)}),ge&&xr(g,T),S}function y(g,v,b,w){if(b==null)throw Error(j(151));for(var S=null,E=null,N=v,T=v=0,L=null,O=b.next();N!==null&&!O.done;T++,O=b.next()){N.index>T?(L=N,N=null):L=N.sibling;var P=f(g,N,O.value,w);if(P===null){N===null&&(N=L);break}e&&N&&P.alternate===null&&t(g,N),v=s(P,v,T),E===null?S=P:E.sibling=P,E=P,N=L}if(O.done)return a(g,N),ge&&xr(g,T),S;if(N===null){for(;!O.done;T++,O=b.next())O=m(g,O.value,w),O!==null&&(v=s(O,v,T),E===null?S=O:E.sibling=O,E=O);return ge&&xr(g,T),S}for(N=n(N);!O.done;T++,O=b.next())O=h(N,g,T,O.value,w),O!==null&&(e&&O.alternate!==null&&N.delete(O.key===null?T:O.key),v=s(O,v,T),E===null?S=O:E.sibling=O,E=O);return e&&N.forEach(function(k){return t(g,k)}),ge&&xr(g,T),S}function $(g,v,b,w){if(typeof b=="object"&&b!==null&&b.type===fs&&b.key===null&&(b=b.props.children),typeof b=="object"&&b!==null){switch(b.$$typeof){case Bl:e:{for(var S=b.key;v!==null;){if(v.key===S){if(S=b.type,S===fs){if(v.tag===7){a(g,v.sibling),w=r(v,b.props.children),w.return=g,g=w;break e}}else if(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Un&&Rg(S)===v.type){a(g,v.sibling),w=r(v,b.props),Ki(w,b),w.return=g,g=w;break e}a(g,v);break}else t(g,v);v=v.sibling}b.type===fs?(w=Sr(b.props.children,g.mode,w,b.key),w.return=g,g=w):(w=ou(b.type,b.key,b.props,null,g.mode,w),Ki(w,b),w.return=g,g=w)}return i(g);case Ji:e:{for(S=b.key;v!==null;){if(v.key===S)if(v.tag===4&&v.stateNode.containerInfo===b.containerInfo&&v.stateNode.implementation===b.implementation){a(g,v.sibling),w=r(v,b.children||[]),w.return=g,g=w;break e}else{a(g,v);break}else t(g,v);v=v.sibling}w=tm(b,g.mode,w),w.return=g,g=w}return i(g);case Un:return S=b._init,b=S(b._payload),$(g,v,b,w)}if(Xi(b))return x(g,v,b,w);if(qi(b)){if(S=qi(b),typeof S!="function")throw Error(j(150));return b=S.call(b),y(g,v,b,w)}if(typeof b.then=="function")return $(g,v,Yl(b),w);if(b.$$typeof===cn)return $(g,v,Vl(g,b),w);Jl(g,b)}return typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint"?(b=""+b,v!==null&&v.tag===6?(a(g,v.sibling),w=r(v,b),w.return=g,g=w):(a(g,v),w=em(b,g.mode,w),w.return=g,g=w),i(g)):a(g,v)}return function(g,v,b,w){try{$o=0;var S=$(g,v,b,w);return As=null,S}catch(N){if(N===Uo||N===Wu)throw N;var E=Xt(29,N,null,g.mode);return E.lanes=w,E.return=g,E}finally{}}}var Fs=Lb(!0),Pb=Lb(!1),ga=Qa(null),Ka=null;function zn(e){var t=e.alternate;Pe(nt,nt.current&1),Pe(ga,e),Ka===null&&(t===null||js.current!==null||t.memoizedState!==null)&&(Ka=e)}function jb(e){if(e.tag===22){if(Pe(nt,nt.current),Pe(ga,e),Ka===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(Ka=e)}}else qn(e)}function qn(){Pe(nt,nt.current),Pe(ga,ga.current)}function pn(e){dt(ga),Ka===e&&(Ka=null),dt(nt)}var nt=Qa(0);function Tu(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||df(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function rm(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:De({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var Gm={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=ea(),r=Yn(n);r.payload=t,a!=null&&(r.callback=a),t=Jn(e,r,n),t!==null&&(ta(t,e,n),so(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=ea(),r=Yn(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=Jn(e,r,n),t!==null&&(ta(t,e,n),so(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=ea(),n=Yn(a);n.tag=2,t!=null&&(n.callback=t),t=Jn(e,n,a),t!==null&&(ta(t,e,a),so(t,e,a))}};function kg(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!yo(a,n)||!yo(r,s):!0}function Cg(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&Gm.enqueueReplaceState(t,t.state,null)}function Dr(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=De({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var Au=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function Ub(e){Au(e)}function Fb(e){console.error(e)}function Bb(e){Au(e)}function Du(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function Eg(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function Ym(e,t,a){return a=Yn(a),a.tag=3,a.payload={element:null},a.callback=function(){Du(e,t)},a}function zb(e){return e=Yn(e),e.tag=3,e}function qb(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){Eg(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){Eg(t,a,n),typeof r!="function"&&(Xn===null?Xn=new Set([this]):Xn.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function NE(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&Po(t,a,r,!0),a=ga.current,a!==null){switch(a.tag){case 13:return Ka===null?nf():a.alternate===null&&Ie===0&&(Ie=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===zm?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),hm(e,n,r)),!1;case 22:return a.flags|=65536,n===zm?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),hm(e,n,r)),!1}throw Error(j(435,a.tag))}return hm(e,n,r),nf(),!1}if(ge)return t=ga.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==Pm&&(e=Error(j(422),{cause:n}),bo(ha(e,a)))):(n!==Pm&&(t=Error(j(423),{cause:n}),bo(ha(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=ha(n,a),r=Ym(e.stateNode,n,r),am(e,r),Ie!==4&&(Ie=2)),!1;var s=Error(j(520),{cause:n});if(s=ha(s,a),mo===null?mo=[s]:mo.push(s),Ie!==4&&(Ie=2),t===null)return!0;n=ha(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=Ym(a.stateNode,n,e),am(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(Xn===null||!Xn.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=zb(r),qb(r,e,a,n),am(a,r),!1}a=a.return}while(a!==null);return!1}var Ib=Error(j(461)),ct=!1;function pt(e,t,a,n){t.child=e===null?Pb(t,null,a,n):Fs(t,e.child,a,n)}function Tg(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return Tr(t),n=Uf(e,t,a,i,s,r),o=Ff(),e!==null&&!ct?(Bf(e,t,r),bn(e,t,r)):(ge&&o&&Af(t),t.flags|=1,pt(e,t,n,r),t.child)}function Ag(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!Tf(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,Hb(e,t,s,n,r)):(e=ou(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!Yf(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:yo,a(i,n)&&e.ref===t.ref)return bn(e,t,r)}return t.flags|=1,e=hn(s,n),e.ref=t.ref,e.return=t,t.child=e}function Hb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if(yo(s,n)&&e.ref===t.ref)if(ct=!1,t.pendingProps=n=s,Yf(e,r))(e.flags&131072)!==0&&(ct=!0);else return t.lanes=e.lanes,bn(e,t,r)}return Jm(e,t,a,n,r)}function Kb(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return Dg(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&lu(t,s!==null?s.cachePool:null),s!==null?bg(t,s):Hm(),jb(t);else return t.lanes=t.childLanes=536870912,Dg(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(lu(t,s.cachePool),bg(t,s),qn(t),t.memoizedState=null):(e!==null&&lu(t,null),Hm(),qn(t));return pt(e,t,r,a),t.child}function Dg(e,t,a,n){var r=Of();return r=r===null?null:{parent:at._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&lu(t,null),Hm(),jb(t),e!==null&&Po(e,t,n,!0),null}function du(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(j(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function Jm(e,t,a,n,r){return Tr(t),a=Uf(e,t,a,n,void 0,r),n=Ff(),e!==null&&!ct?(Bf(e,t,r),bn(e,t,r)):(ge&&n&&Af(t),t.flags|=1,pt(e,t,a,r),t.child)}function Mg(e,t,a,n,r,s){return Tr(t),t.updateQueue=null,a=rb(t,n,a,r),nb(e),n=Ff(),e!==null&&!ct?(Bf(e,t,s),bn(e,t,s)):(ge&&n&&Af(t),t.flags|=1,pt(e,t,a,s),t.child)}function Og(e,t,a,n,r){if(Tr(t),t.stateNode===null){var s=$s,i=a.contextType;typeof i=="object"&&i!==null&&(s=bt(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=Gm,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},Lf(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?bt(i):$s,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(rm(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&Gm.enqueueReplaceState(s,s.state,null),oo(t,n,s,r),io(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,l=Dr(a,o);s.props=l;var c=s.context,d=a.contextType;i=$s,typeof d=="object"&&d!==null&&(i=bt(d));var m=a.getDerivedStateFromProps;d=typeof m=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&Cg(t,s,n,i),Fn=!1;var f=t.memoizedState;s.state=f,oo(t,n,s,r),io(),c=t.memoizedState,o||f!==c||Fn?(typeof m=="function"&&(rm(t,a,m,n),c=t.memoizedState),(l=Fn||kg(t,a,l,n,f,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=l):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,qm(e,t),i=t.memoizedProps,d=Dr(a,i),s.props=d,m=t.pendingProps,f=s.context,c=a.contextType,l=$s,typeof c=="object"&&c!==null&&(l=bt(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==m||f!==l)&&Cg(t,s,n,l),Fn=!1,f=t.memoizedState,s.state=f,oo(t,n,s,r),io();var h=t.memoizedState;i!==m||f!==h||Fn||e!==null&&e.dependencies!==null&&_u(e.dependencies)?(typeof o=="function"&&(rm(t,a,o,n),h=t.memoizedState),(d=Fn||kg(t,a,d,n,f,h,l)||e!==null&&e.dependencies!==null&&_u(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,h,l),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,h,l)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=h),s.props=n,s.state=h,s.context=l,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,du(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=Fs(t,e.child,null,r),t.child=Fs(t,null,a,r)):pt(e,t,a,r),t.memoizedState=s.state,e=t.child):e=bn(e,t,r),e}function Lg(e,t,a,n){return Lo(),t.flags|=256,pt(e,t,a,n),t.child}var sm={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function im(e){return{baseLanes:e,cachePool:Wy()}}function om(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=va),e}function Qb(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(nt.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(ge){if(r?zn(t):qn(t),ge){var o=qe,l;if(l=o){e:{for(l=o,o=za;l.nodeType!==8;){if(!o){o=null;break e}if(l=_a(l.nextSibling),l===null){o=null;break e}}o=l}o!==null?(t.memoizedState={dehydrated:o,treeContext:Nr!==null?{id:dn,overflow:mn}:null,retryLane:536870912,hydrationErrors:null},l=Xt(18,null,null,0),l.stateNode=o,l.return=t,t.child=l,Tt=t,qe=null,l=!0):l=!1}l||Er(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return df(o)?t.lanes=32:t.lanes=536870912,null;pn(t)}return o=n.children,n=n.fallback,r?(qn(t),r=t.mode,o=Mu({mode:"hidden",children:o},r),n=Sr(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=im(a),r.childLanes=om(e,i,a),t.memoizedState=sm,n):(zn(t),Xm(t,o))}if(l=e.memoizedState,l!==null&&(o=l.dehydrated,o!==null)){if(s)t.flags&256?(zn(t),t.flags&=-257,t=lm(e,t,a)):t.memoizedState!==null?(qn(t),t.child=e.child,t.flags|=128,t=null):(qn(t),r=n.fallback,o=t.mode,n=Mu({mode:"visible",children:n.children},o),r=Sr(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,Fs(t,e.child,null,a),n=t.child,n.memoizedState=im(a),n.childLanes=om(e,i,a),t.memoizedState=sm,t=r);else if(zn(t),df(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(j(419)),n.stack="",n.digest=i,bo({value:n,source:null,stack:null}),t=lm(e,t,a)}else if(ct||Po(e,t,a,!1),i=(a&e.childLanes)!==0,ct||i){if(i=Ee,i!==null&&(n=a&-a,n=(n&42)!==0?1:bf(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==l.retryLane))throw l.retryLane=n,Qs(e,n),ta(i,e,n),Ib;o.data==="$?"||nf(),t=lm(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=l.treeContext,qe=_a(o.nextSibling),Tt=t,ge=!0,_r=null,za=!1,e!==null&&(ma[fa++]=dn,ma[fa++]=mn,ma[fa++]=Nr,dn=e.id,mn=e.overflow,Nr=t),t=Xm(t,n.children),t.flags|=4096);return t}return r?(qn(t),r=n.fallback,o=t.mode,l=e.child,c=l.sibling,n=hn(l,{mode:"hidden",children:n.children}),n.subtreeFlags=l.subtreeFlags&65011712,c!==null?r=hn(c,r):(r=Sr(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=im(a):(l=o.cachePool,l!==null?(c=at._currentValue,l=l.parent!==c?{parent:c,pool:c}:l):l=Wy(),o={baseLanes:o.baseLanes|a,cachePool:l}),r.memoizedState=o,r.childLanes=om(e,i,a),t.memoizedState=sm,n):(zn(t),a=e.child,e=a.sibling,a=hn(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function Xm(e,t){return t=Mu({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function Mu(e,t){return e=Xt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function lm(e,t,a){return Fs(t,e.child,null,a),e=Xm(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function Pg(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),Um(e.return,t,a)}function um(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function Vb(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(pt(e,t,n.children,a),n=nt.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&Pg(e,a,t);else if(e.tag===19)Pg(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Pe(nt,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&Tu(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),um(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&Tu(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}um(t,!0,a,null,s);break;case"together":um(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function bn(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),rr|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(Po(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(j(153));if(t.child!==null){for(e=t.child,a=hn(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=hn(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function Yf(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&_u(e)))}function _E(e,t,a){switch(t.tag){case 3:gu(t,t.stateNode.containerInfo),Bn(t,at,e.memoizedState.cache),Lo();break;case 27:case 5:Rm(t);break;case 4:gu(t,t.stateNode.containerInfo);break;case 10:Bn(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(zn(t),t.flags|=128,null):(a&t.child.childLanes)!==0?Qb(e,t,a):(zn(t),e=bn(e,t,a),e!==null?e.sibling:null);zn(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(Po(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return Vb(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Pe(nt,nt.current),n)break;return null;case 22:case 23:return t.lanes=0,Kb(e,t,a);case 24:Bn(t,at,e.memoizedState.cache)}return bn(e,t,a)}function Gb(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)ct=!0;else{if(!Yf(e,a)&&(t.flags&128)===0)return ct=!1,_E(e,t,a);ct=(e.flags&131072)!==0}else ct=!1,ge&&(t.flags&1048576)!==0&&Jy(t,Nu,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")Tf(n)?(e=Dr(n,e),t.tag=1,t=Og(null,t,n,e,a)):(t.tag=0,t=Jm(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===vf){t.tag=11,t=Tg(null,t,n,e,a);break e}else if(r===gf){t.tag=14,t=Ag(null,t,n,e,a);break e}}throw t=Nm(n)||n,Error(j(306,t,""))}}return t;case 0:return Jm(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Dr(n,t.pendingProps),Og(e,t,n,r,a);case 3:e:{if(gu(t,t.stateNode.containerInfo),e===null)throw Error(j(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,qm(e,t),oo(t,n,null,a);var i=t.memoizedState;if(n=i.cache,Bn(t,at,n),n!==s.cache&&Fm(t,[at],a,!0),io(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=Lg(e,t,n,a);break e}else if(n!==r){r=ha(Error(j(424)),t),bo(r),t=Lg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(qe=_a(e.firstChild),Tt=t,ge=!0,_r=null,za=!0,a=Pb(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if(Lo(),n===r){t=bn(e,t,a);break e}pt(e,t,n,a)}t=t.child}return t;case 26:return du(e,t),e===null?(a=ty(t.type,null,t.pendingProps,null))?t.memoizedState=a:ge||(a=t.type,e=t.pendingProps,n=Bu(Gn.current).createElement(a),n[yt]=t,n[zt]=e,vt(n,a,e),ut(n),t.stateNode=n):t.memoizedState=ty(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return Rm(t),e===null&&ge&&(n=t.stateNode=O0(t.type,t.pendingProps,Gn.current),Tt=t,za=!0,r=qe,ir(t.type)?(mf=r,qe=_a(n.firstChild)):qe=r),pt(e,t,t.pendingProps.children,a),du(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&ge&&((r=n=qe)&&(n=XE(n,t.type,t.pendingProps,za),n!==null?(t.stateNode=n,Tt=t,qe=_a(n.firstChild),za=!1,r=!0):r=!1),r||Er(t)),Rm(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,uf(r,s)?n=null:i!==null&&uf(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=Uf(e,t,gE,null,null,a),_o._currentValue=r),du(e,t),pt(e,t,n,a),t.child;case 6:return e===null&&ge&&((e=a=qe)&&(a=WE(a,t.pendingProps,za),a!==null?(t.stateNode=a,Tt=t,qe=null,e=!0):e=!1),e||Er(t)),null;case 13:return Qb(e,t,a);case 4:return gu(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=Fs(t,null,n,a):pt(e,t,n,a),t.child;case 11:return Tg(e,t,t.type,t.pendingProps,a);case 7:return pt(e,t,t.pendingProps,a),t.child;case 8:return pt(e,t,t.pendingProps.children,a),t.child;case 12:return pt(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,Bn(t,t.type,n.value),pt(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,Tr(t),r=bt(r),n=n(r),t.flags|=1,pt(e,t,n,a),t.child;case 14:return Ag(e,t,t.type,t.pendingProps,a);case 15:return Hb(e,t,t.type,t.pendingProps,a);case 19:return Vb(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=Mu(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=hn(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return Kb(e,t,a);case 24:return Tr(t),n=bt(at),e===null?(r=Of(),r===null&&(r=Ee,s=Mf(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},Lf(t),Bn(t,at,r)):((e.lanes&a)!==0&&(qm(e,t),oo(t,null,null,a),io()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),Bn(t,at,n)):(n=s.cache,Bn(t,at,n),n!==r.cache&&Fm(t,[at],a,!0))),pt(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(j(156,t.tag))}function on(e){e.flags|=4}function jg(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!j0(t)){if(t=ga.current,t!==null&&((fe&4194048)===fe?Ka!==null:(fe&62914560)!==fe&&(fe&536870912)===0||t!==Ka))throw ro=zm,Zy;e.flags|=8192}}function Xl(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?xy():536870912,e.lanes|=t,Bs|=t)}function Qi(e,t){if(!ge)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function Be(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function RE(e,t,a){var n=t.pendingProps;switch(Df(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return Be(t),null;case 1:return Be(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),vn(at),Ms(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Hi(t)?on(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,fg())),Be(t),null;case 26:return a=t.memoizedState,e===null?(on(t),a!==null?(Be(t),jg(t,a)):(Be(t),t.flags&=-16777217)):a?a!==e.memoizedState?(on(t),Be(t),jg(t,a)):(Be(t),t.flags&=-16777217):(e.memoizedProps!==n&&on(t),Be(t),t.flags&=-16777217),null;case 27:yu(t),a=Gn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&on(t);else{if(!n){if(t.stateNode===null)throw Error(j(166));return Be(t),null}e=Ia.current,Hi(t)?dg(t,e):(e=O0(r,n,a),t.stateNode=e,on(t))}return Be(t),null;case 5:if(yu(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&on(t);else{if(!n){if(t.stateNode===null)throw Error(j(166));return Be(t),null}if(e=Ia.current,Hi(t))dg(t,e);else{switch(r=Bu(Gn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[yt]=t,e[zt]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(vt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&on(t)}}return Be(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&on(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(j(166));if(e=Gn.current,Hi(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=Tt,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[yt]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||A0(e.nodeValue,a)),e||Er(t)}else e=Bu(e).createTextNode(n),e[yt]=t,t.stateNode=e}return Be(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Hi(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(j(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(j(317));r[yt]=t}else Lo(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;Be(t),r=!1}else r=fg(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(pn(t),t):(pn(t),null)}if(pn(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),Xl(t,t.updateQueue),Be(t),null;case 4:return Ms(),e===null&&np(t.stateNode.containerInfo),Be(t),null;case 10:return vn(t.type),Be(t),null;case 19:if(dt(nt),r=t.memoizedState,r===null)return Be(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)Qi(r,!1);else{if(Ie!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=Tu(e),s!==null){for(t.flags|=128,Qi(r,!1),e=s.updateQueue,t.updateQueue=e,Xl(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)Yy(a,e),a=a.sibling;return Pe(nt,nt.current&1|2),t.child}e=e.sibling}r.tail!==null&&Ha()>Lu&&(t.flags|=128,n=!0,Qi(r,!1),t.lanes=4194304)}else{if(!n)if(e=Tu(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,Xl(t,e),Qi(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!ge)return Be(t),null}else 2*Ha()-r.renderingStartTime>Lu&&a!==536870912&&(t.flags|=128,n=!0,Qi(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=Ha(),t.sibling=null,e=nt.current,Pe(nt,n?e&1|2:e&1),t):(Be(t),null);case 22:case 23:return pn(t),Pf(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(Be(t),t.subtreeFlags&6&&(t.flags|=8192)):Be(t),a=t.updateQueue,a!==null&&Xl(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&dt(Rr),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),vn(at),Be(t),null;case 25:return null;case 30:return null}throw Error(j(156,t.tag))}function kE(e,t){switch(Df(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return vn(at),Ms(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return yu(t),null;case 13:if(pn(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(j(340));Lo()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return dt(nt),null;case 4:return Ms(),null;case 10:return vn(t.type),null;case 22:case 23:return pn(t),Pf(),e!==null&&dt(Rr),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return vn(at),null;case 25:return null;default:return null}}function Yb(e,t){switch(Df(t),t.tag){case 3:vn(at),Ms();break;case 26:case 27:case 5:yu(t);break;case 4:Ms();break;case 13:pn(t);break;case 19:dt(nt);break;case 10:vn(t.type);break;case 22:case 23:pn(t),Pf(),e!==null&&dt(Rr);break;case 24:vn(at)}}function zo(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){Re(t,t.return,o)}}function nr(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var l=a,c=o;try{c()}catch(d){Re(r,l,d)}}}n=n.next}while(n!==s)}}catch(d){Re(t,t.return,d)}}function Jb(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{ab(t,a)}catch(n){Re(e,e.return,n)}}}function Xb(e,t,a){a.props=Dr(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){Re(e,t,n)}}function uo(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){Re(e,t,r)}}function qa(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){Re(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){Re(e,t,r)}else a.current=null}function Wb(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){Re(e,e.return,r)}}function cm(e,t,a){try{var n=e.stateNode;QE(n,e.type,a,t),n[zt]=t}catch(r){Re(e,e.return,r)}}function Zb(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&ir(e.type)||e.tag===4}function dm(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||Zb(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&ir(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function Wm(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=sc));else if(n!==4&&(n===27&&ir(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(Wm(e,t,a),e=e.sibling;e!==null;)Wm(e,t,a),e=e.sibling}function Ou(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&ir(e.type)&&(a=e.stateNode),e=e.child,e!==null))for(Ou(e,t,a),e=e.sibling;e!==null;)Ou(e,t,a),e=e.sibling}function e0(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);vt(t,n,a),t[yt]=e,t[zt]=a}catch(s){Re(e,e.return,s)}}var un=!1,Ve=!1,mm=!1,Ug=typeof WeakSet=="function"?WeakSet:Set,lt=null;function CE(e,t){if(e=e.containerInfo,of=Hu,e=zy(e),kf(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,l=-1,c=0,d=0,m=e,f=null;t:for(;;){for(var h;m!==a||r!==0&&m.nodeType!==3||(o=i+r),m!==s||n!==0&&m.nodeType!==3||(l=i+n),m.nodeType===3&&(i+=m.nodeValue.length),(h=m.firstChild)!==null;)f=m,m=h;for(;;){if(m===e)break t;if(f===a&&++c===r&&(o=i),f===s&&++d===n&&(l=i),(h=m.nextSibling)!==null)break;m=f,f=m.parentNode}m=h}a=o===-1||l===-1?null:{start:o,end:l}}else a=null}a=a||{start:0,end:0}}else a=null;for(lf={focusedElem:e,selectionRange:a},Hu=!1,lt=t;lt!==null;)if(t=lt,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,lt=e;else for(;lt!==null;){switch(t=lt,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var x=Dr(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(x,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(y){Re(a,a.return,y)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)cf(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":cf(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(j(163))}if(e=t.sibling,e!==null){e.return=t.return,lt=e;break}lt=t.return}}function t0(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:Pn(e,a),n&4&&zo(5,a);break;case 1:if(Pn(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){Re(a,a.return,i)}else{var r=Dr(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){Re(a,a.return,i)}}n&64&&Jb(a),n&512&&uo(a,a.return);break;case 3:if(Pn(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{ab(e,t)}catch(i){Re(a,a.return,i)}}break;case 27:t===null&&n&4&&e0(a);case 26:case 5:Pn(e,a),t===null&&n&4&&Wb(a),n&512&&uo(a,a.return);break;case 12:Pn(e,a);break;case 13:Pn(e,a),n&4&&r0(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=jE.bind(null,a),ZE(e,a))));break;case 22:if(n=a.memoizedState!==null||un,!n){t=t!==null&&t.memoizedState!==null||Ve,r=un;var s=Ve;un=n,(Ve=t)&&!s?jn(e,a,(a.subtreeFlags&8772)!==0):Pn(e,a),un=r,Ve=s}break;case 30:break;default:Pn(e,a)}}function a0(e){var t=e.alternate;t!==null&&(e.alternate=null,a0(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&$f(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var Le=null,Ft=!1;function ln(e,t,a){for(a=a.child;a!==null;)n0(e,t,a),a=a.sibling}function n0(e,t,a){if(Wt&&typeof Wt.onCommitFiberUnmount=="function")try{Wt.onCommitFiberUnmount(To,a)}catch{}switch(a.tag){case 26:Ve||qa(a,t),ln(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:Ve||qa(a,t);var n=Le,r=Ft;ir(a.type)&&(Le=a.stateNode,Ft=!1),ln(e,t,a),po(a.stateNode),Le=n,Ft=r;break;case 5:Ve||qa(a,t);case 6:if(n=Le,r=Ft,Le=null,ln(e,t,a),Le=n,Ft=r,Le!==null)if(Ft)try{(Le.nodeType===9?Le.body:Le.nodeName==="HTML"?Le.ownerDocument.body:Le).removeChild(a.stateNode)}catch(s){Re(a,t,s)}else try{Le.removeChild(a.stateNode)}catch(s){Re(a,t,s)}break;case 18:Le!==null&&(Ft?(e=Le,Wg(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),Co(e)):Wg(Le,a.stateNode));break;case 4:n=Le,r=Ft,Le=a.stateNode.containerInfo,Ft=!0,ln(e,t,a),Le=n,Ft=r;break;case 0:case 11:case 14:case 15:Ve||nr(2,a,t),Ve||nr(4,a,t),ln(e,t,a);break;case 1:Ve||(qa(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&Xb(a,t,n)),ln(e,t,a);break;case 21:ln(e,t,a);break;case 22:Ve=(n=Ve)||a.memoizedState!==null,ln(e,t,a),Ve=n;break;default:ln(e,t,a)}}function r0(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{Co(e)}catch(a){Re(t,t.return,a)}}function EE(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new Ug),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new Ug),t;default:throw Error(j(435,e.tag))}}function fm(e,t){var a=EE(e);t.forEach(function(n){var r=UE.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function Gt(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(ir(o.type)){Le=o.stateNode,Ft=!1;break e}break;case 5:Le=o.stateNode,Ft=!1;break e;case 3:case 4:Le=o.stateNode.containerInfo,Ft=!0;break e}o=o.return}if(Le===null)throw Error(j(160));n0(s,i,r),Le=null,Ft=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)s0(t,e),t=t.sibling}var Na=null;function s0(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:Gt(t,e),Yt(e),n&4&&(nr(3,e,e.return),zo(3,e),nr(5,e,e.return));break;case 1:Gt(t,e),Yt(e),n&512&&(Ve||a===null||qa(a,a.return)),n&64&&un&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=Na;if(Gt(t,e),Yt(e),n&512&&(Ve||a===null||qa(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[Mo]||s[yt]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),vt(s,n,a),s[yt]=e,ut(s),n=s;break e;case"link":var i=ny("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),vt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=ny("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),vt(s,n,a),r.head.appendChild(s);break;default:throw Error(j(468,n))}s[yt]=e,ut(s),n=s}e.stateNode=n}else ry(r,e.type,e.stateNode);else e.stateNode=ay(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?ry(r,e.type,e.stateNode):ay(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&cm(e,e.memoizedProps,a.memoizedProps)}break;case 27:Gt(t,e),Yt(e),n&512&&(Ve||a===null||qa(a,a.return)),a!==null&&n&4&&cm(e,e.memoizedProps,a.memoizedProps);break;case 5:if(Gt(t,e),Yt(e),n&512&&(Ve||a===null||qa(a,a.return)),e.flags&32){r=e.stateNode;try{Ls(r,"")}catch(h){Re(e,e.return,h)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,cm(e,r,a!==null?a.memoizedProps:r)),n&1024&&(mm=!0);break;case 6:if(Gt(t,e),Yt(e),n&4){if(e.stateNode===null)throw Error(j(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(h){Re(e,e.return,h)}}break;case 3:if(pu=null,r=Na,Na=zu(t.containerInfo),Gt(t,e),Na=r,Yt(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{Co(t.containerInfo)}catch(h){Re(e,e.return,h)}mm&&(mm=!1,i0(e));break;case 4:n=Na,Na=zu(e.stateNode.containerInfo),Gt(t,e),Yt(e),Na=n;break;case 12:Gt(t,e),Yt(e);break;case 13:Gt(t,e),Yt(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(ep=Ha()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,fm(e,n)));break;case 22:r=e.memoizedState!==null;var l=a!==null&&a.memoizedState!==null,c=un,d=Ve;if(un=c||r,Ve=d||l,Gt(t,e),Ve=d,un=c,Yt(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||l||un||Ve||$r(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){l=a=t;try{if(s=l.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=l.stateNode;var m=l.memoizedProps.style,f=m!=null&&m.hasOwnProperty("display")?m.display:null;o.style.display=f==null||typeof f=="boolean"?"":(""+f).trim()}}catch(h){Re(l,l.return,h)}}}else if(t.tag===6){if(a===null){l=t;try{l.stateNode.nodeValue=r?"":l.memoizedProps}catch(h){Re(l,l.return,h)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,fm(e,a))));break;case 19:Gt(t,e),Yt(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,fm(e,n)));break;case 30:break;case 21:break;default:Gt(t,e),Yt(e)}}function Yt(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(Zb(n)){a=n;break}n=n.return}if(a==null)throw Error(j(160));switch(a.tag){case 27:var r=a.stateNode,s=dm(e);Ou(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(Ls(i,""),a.flags&=-33);var o=dm(e);Ou(e,o,i);break;case 3:case 4:var l=a.stateNode.containerInfo,c=dm(e);Wm(e,c,l);break;default:throw Error(j(161))}}catch(d){Re(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function i0(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;i0(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function Pn(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)t0(e,t.alternate,t),t=t.sibling}function $r(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:nr(4,t,t.return),$r(t);break;case 1:qa(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&Xb(t,t.return,a),$r(t);break;case 27:po(t.stateNode);case 26:case 5:qa(t,t.return),$r(t);break;case 22:t.memoizedState===null&&$r(t);break;case 30:$r(t);break;default:$r(t)}e=e.sibling}}function jn(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:jn(r,s,a),zo(4,s);break;case 1:if(jn(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){Re(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var l=r.shared.hiddenCallbacks;if(l!==null)for(r.shared.hiddenCallbacks=null,r=0;r<l.length;r++)tb(l[r],o)}catch(c){Re(n,n.return,c)}}a&&i&64&&Jb(s),uo(s,s.return);break;case 27:e0(s);case 26:case 5:jn(r,s,a),a&&n===null&&i&4&&Wb(s),uo(s,s.return);break;case 12:jn(r,s,a);break;case 13:jn(r,s,a),a&&i&4&&r0(r,s);break;case 22:s.memoizedState===null&&jn(r,s,a),uo(s,s.return);break;case 30:break;default:jn(r,s,a)}t=t.sibling}}function Jf(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&jo(a))}function Xf(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&jo(e))}function Ba(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)o0(e,t,a,n),t=t.sibling}function o0(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:Ba(e,t,a,n),r&2048&&zo(9,t);break;case 1:Ba(e,t,a,n);break;case 3:Ba(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&jo(e)));break;case 12:if(r&2048){Ba(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(l){Re(t,t.return,l)}}else Ba(e,t,a,n);break;case 13:Ba(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?Ba(e,t,a,n):co(e,t):s._visibility&2?Ba(e,t,a,n):(s._visibility|=2,ds(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&Jf(i,t);break;case 24:Ba(e,t,a,n),r&2048&&Xf(t.alternate,t);break;default:Ba(e,t,a,n)}}function ds(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,l=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:ds(s,i,o,l,r),zo(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?ds(s,i,o,l,r):co(s,i):(d._visibility|=2,ds(s,i,o,l,r)),r&&c&2048&&Jf(i.alternate,i);break;case 24:ds(s,i,o,l,r),r&&c&2048&&Xf(i.alternate,i);break;default:ds(s,i,o,l,r)}t=t.sibling}}function co(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:co(a,n),r&2048&&Jf(n.alternate,n);break;case 24:co(a,n),r&2048&&Xf(n.alternate,n);break;default:co(a,n)}t=t.sibling}}var Zi=8192;function ls(e){if(e.subtreeFlags&Zi)for(e=e.child;e!==null;)l0(e),e=e.sibling}function l0(e){switch(e.tag){case 26:ls(e),e.flags&Zi&&e.memoizedState!==null&&m3(Na,e.memoizedState,e.memoizedProps);break;case 5:ls(e);break;case 3:case 4:var t=Na;Na=zu(e.stateNode.containerInfo),ls(e),Na=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=Zi,Zi=16777216,ls(e),Zi=t):ls(e));break;default:ls(e)}}function u0(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function Vi(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];lt=n,d0(n,e)}u0(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)c0(e),e=e.sibling}function c0(e){switch(e.tag){case 0:case 11:case 15:Vi(e),e.flags&2048&&nr(9,e,e.return);break;case 3:Vi(e);break;case 12:Vi(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,mu(e)):Vi(e);break;default:Vi(e)}}function mu(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];lt=n,d0(n,e)}u0(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:nr(8,t,t.return),mu(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,mu(t));break;default:mu(t)}e=e.sibling}}function d0(e,t){for(;lt!==null;){var a=lt;switch(a.tag){case 0:case 11:case 15:nr(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:jo(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,lt=n;else e:for(a=e;lt!==null;){n=lt;var r=n.sibling,s=n.return;if(a0(n),n===a){lt=null;break e}if(r!==null){r.return=s,lt=r;break e}lt=s}}}var TE={getCacheForType:function(e){var t=bt(at),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},AE=typeof WeakMap=="function"?WeakMap:Map,we=0,Ee=null,ue=null,fe=0,$e=0,Jt=null,Qn=!1,Vs=!1,Wf=!1,xn=0,Ie=0,rr=0,kr=0,Zf=0,va=0,Bs=0,mo=null,Bt=null,Zm=!1,ep=0,Lu=1/0,Pu=null,Xn=null,ht=0,Wn=null,zs=null,Ds=0,ef=0,tf=null,m0=null,fo=0,af=null;function ea(){if((we&2)!==0&&fe!==0)return fe&-fe;if(ne.T!==null){var e=Ps;return e!==0?e:ap()}return Sy()}function f0(){va===0&&(va=(fe&536870912)===0||ge?by():536870912);var e=ga.current;return e!==null&&(e.flags|=32),va}function ta(e,t,a){(e===Ee&&($e===2||$e===9)||e.cancelPendingCommit!==null)&&(qs(e,0),Vn(e,fe,va,!1)),Do(e,a),((we&2)===0||e!==Ee)&&(e===Ee&&((we&2)===0&&(kr|=a),Ie===4&&Vn(e,fe,va,!1)),Va(e))}function p0(e,t,a){if((we&6)!==0)throw Error(j(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||Ao(e,t),r=n?OE(e,t):pm(e,t,!0),s=n;do{if(r===0){Vs&&!n&&Vn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!DE(a)){r=pm(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=mo;var l=o.current.memoizedState.isDehydrated;if(l&&(qs(o,i).flags|=256),i=pm(o,i,!1),i!==2){if(Wf&&!l){o.errorRecoveryDisabledLanes|=s,kr|=s,r=4;break e}s=Bt,Bt=r,s!==null&&(Bt===null?Bt=s:Bt.push.apply(Bt,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){qs(e,0),Vn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(j(345));case 4:if((t&4194048)!==t)break;case 6:Vn(n,t,va,!Qn);break e;case 2:Bt=null;break;case 3:case 5:break;default:throw Error(j(329))}if((t&62914560)===t&&(r=ep+300-Ha(),10<r)){if(Vn(n,t,va,!Qn),Qu(n,0,!0)!==0)break e;n.timeoutHandle=M0(Fg.bind(null,n,a,Bt,Pu,Zm,t,va,kr,Bs,Qn,s,2,-0,0),r);break e}Fg(n,a,Bt,Pu,Zm,t,va,kr,Bs,Qn,s,0,-0,0)}}break}while(!0);Va(e)}function Fg(e,t,a,n,r,s,i,o,l,c,d,m,f,h){if(e.timeoutHandle=-1,m=t.subtreeFlags,(m&8192||(m&16785408)===16785408)&&(No={stylesheets:null,count:0,unsuspend:d3},l0(t),m=f3(),m!==null)){e.cancelPendingCommit=m(zg.bind(null,e,t,s,a,n,r,i,o,l,d,1,f,h)),Vn(e,s,i,!c);return}zg(e,t,s,a,n,r,i,o,l)}function DE(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!aa(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function Vn(e,t,a,n){t&=~Zf,t&=~kr,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Zt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&$y(e,a,t)}function ac(){return(we&6)===0?(qo(0,!1),!1):!0}function tp(){if(ue!==null){if($e===0)var e=ue.return;else e=ue,fn=Pr=null,zf(e),As=null,$o=0,e=ue;for(;e!==null;)Yb(e.alternate,e),e=e.return;ue=null}}function qs(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,GE(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),tp(),Ee=e,ue=a=hn(e.current,null),fe=t,$e=0,Jt=null,Qn=!1,Vs=Ao(e,t),Wf=!1,Bs=va=Zf=kr=rr=Ie=0,Bt=mo=null,Zm=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Zt(n),s=1<<r;t|=e[r],n&=~s}return xn=t,Ju(),a}function h0(e,t){oe=null,ne.H=Eu,t===Uo||t===Wu?(t=gg(),$e=3):t===Zy?(t=gg(),$e=4):$e=t===Ib?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Jt=t,ue===null&&(Ie=1,Du(e,ha(t,e.current)))}function v0(){var e=ne.H;return ne.H=Eu,e===null?Eu:e}function g0(){var e=ne.A;return ne.A=TE,e}function nf(){Ie=4,Qn||(fe&4194048)!==fe&&ga.current!==null||(Vs=!0),(rr&134217727)===0&&(kr&134217727)===0||Ee===null||Vn(Ee,fe,va,!1)}function pm(e,t,a){var n=we;we|=2;var r=v0(),s=g0();(Ee!==e||fe!==t)&&(Pu=null,qs(e,t)),t=!1;var i=Ie;e:do try{if($e!==0&&ue!==null){var o=ue,l=Jt;switch($e){case 8:tp(),i=6;break e;case 3:case 2:case 9:case 6:ga.current===null&&(t=!0);var c=$e;if($e=0,Jt=null,Ns(e,o,l,c),a&&Vs){i=0;break e}break;default:c=$e,$e=0,Jt=null,Ns(e,o,l,c)}}ME(),i=Ie;break}catch(d){h0(e,d)}while(!0);return t&&e.shellSuspendCounter++,fn=Pr=null,we=n,ne.H=r,ne.A=s,ue===null&&(Ee=null,fe=0,Ju()),i}function ME(){for(;ue!==null;)y0(ue)}function OE(e,t){var a=we;we|=2;var n=v0(),r=g0();Ee!==e||fe!==t?(Pu=null,Lu=Ha()+500,qs(e,t)):Vs=Ao(e,t);e:do try{if($e!==0&&ue!==null){t=ue;var s=Jt;t:switch($e){case 1:$e=0,Jt=null,Ns(e,t,s,1);break;case 2:case 9:if(vg(s)){$e=0,Jt=null,Bg(t);break}t=function(){$e!==2&&$e!==9||Ee!==e||($e=7),Va(e)},s.then(t,t);break e;case 3:$e=7;break e;case 4:$e=5;break e;case 7:vg(s)?($e=0,Jt=null,Bg(t)):($e=0,Jt=null,Ns(e,t,s,7));break;case 5:var i=null;switch(ue.tag){case 26:i=ue.memoizedState;case 5:case 27:var o=ue;if(!i||j0(i)){$e=0,Jt=null;var l=o.sibling;if(l!==null)ue=l;else{var c=o.return;c!==null?(ue=c,nc(c)):ue=null}break t}}$e=0,Jt=null,Ns(e,t,s,5);break;case 6:$e=0,Jt=null,Ns(e,t,s,6);break;case 8:tp(),Ie=6;break e;default:throw Error(j(462))}}LE();break}catch(d){h0(e,d)}while(!0);return fn=Pr=null,ne.H=n,ne.A=r,we=a,ue!==null?0:(Ee=null,fe=0,Ju(),Ie)}function LE(){for(;ue!==null&&!nC();)y0(ue)}function y0(e){var t=Gb(e.alternate,e,xn);e.memoizedProps=e.pendingProps,t===null?nc(e):ue=t}function Bg(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=Mg(a,t,t.pendingProps,t.type,void 0,fe);break;case 11:t=Mg(a,t,t.pendingProps,t.type.render,t.ref,fe);break;case 5:zf(t);default:Yb(a,t),t=ue=Yy(t,xn),t=Gb(a,t,xn)}e.memoizedProps=e.pendingProps,t===null?nc(e):ue=t}function Ns(e,t,a,n){fn=Pr=null,zf(t),As=null,$o=0;var r=t.return;try{if(NE(e,r,t,a,fe)){Ie=1,Du(e,ha(a,e.current)),ue=null;return}}catch(s){if(r!==null)throw ue=r,s;Ie=1,Du(e,ha(a,e.current)),ue=null;return}t.flags&32768?(ge||n===1?e=!0:Vs||(fe&536870912)!==0?e=!1:(Qn=e=!0,(n===2||n===9||n===3||n===6)&&(n=ga.current,n!==null&&n.tag===13&&(n.flags|=16384))),b0(t,e)):nc(t)}function nc(e){var t=e;do{if((t.flags&32768)!==0){b0(t,Qn);return}e=t.return;var a=RE(t.alternate,t,xn);if(a!==null){ue=a;return}if(t=t.sibling,t!==null){ue=t;return}ue=t=e}while(t!==null);Ie===0&&(Ie=5)}function b0(e,t){do{var a=kE(e.alternate,e);if(a!==null){a.flags&=32767,ue=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){ue=e;return}ue=e=a}while(e!==null);Ie=6,ue=null}function zg(e,t,a,n,r,s,i,o,l){e.cancelPendingCommit=null;do rc();while(ht!==0);if((we&6)!==0)throw Error(j(327));if(t!==null){if(t===e.current)throw Error(j(177));if(s=t.lanes|t.childLanes,s|=Cf,fC(e,a,s,i,o,l),e===Ee&&(ue=Ee=null,fe=0),zs=t,Wn=e,Ds=a,ef=s,tf=r,m0=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,FE(bu,function(){return N0(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=ne.T,ne.T=null,r=ye.p,ye.p=2,i=we,we|=4;try{CE(e,t,a)}finally{we=i,ye.p=r,ne.T=n}}ht=1,x0(),$0(),w0()}}function x0(){if(ht===1){ht=0;var e=Wn,t=zs,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=ne.T,ne.T=null;var n=ye.p;ye.p=2;var r=we;we|=4;try{s0(t,e);var s=lf,i=zy(e.containerInfo),o=s.focusedElem,l=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&By(o.ownerDocument.documentElement,o)){if(l!==null&&kf(o)){var c=l.start,d=l.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var m=o.ownerDocument||document,f=m&&m.defaultView||window;if(f.getSelection){var h=f.getSelection(),x=o.textContent.length,y=Math.min(l.start,x),$=l.end===void 0?y:Math.min(l.end,x);!h.extend&&y>$&&(i=$,$=y,y=i);var g=lg(o,y),v=lg(o,$);if(g&&v&&(h.rangeCount!==1||h.anchorNode!==g.node||h.anchorOffset!==g.offset||h.focusNode!==v.node||h.focusOffset!==v.offset)){var b=m.createRange();b.setStart(g.node,g.offset),h.removeAllRanges(),y>$?(h.addRange(b),h.extend(v.node,v.offset)):(b.setEnd(v.node,v.offset),h.addRange(b))}}}}for(m=[],h=o;h=h.parentNode;)h.nodeType===1&&m.push({element:h,left:h.scrollLeft,top:h.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<m.length;o++){var w=m[o];w.element.scrollLeft=w.left,w.element.scrollTop=w.top}}Hu=!!of,lf=of=null}finally{we=r,ye.p=n,ne.T=a}}e.current=t,ht=2}}function $0(){if(ht===2){ht=0;var e=Wn,t=zs,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=ne.T,ne.T=null;var n=ye.p;ye.p=2;var r=we;we|=4;try{t0(e,t.alternate,t)}finally{we=r,ye.p=n,ne.T=a}}ht=3}}function w0(){if(ht===4||ht===3){ht=0,rC();var e=Wn,t=zs,a=Ds,n=m0;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?ht=5:(ht=0,zs=Wn=null,S0(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(Xn=null),xf(a),t=t.stateNode,Wt&&typeof Wt.onCommitFiberRoot=="function")try{Wt.onCommitFiberRoot(To,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=ne.T,r=ye.p,ye.p=2,ne.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{ne.T=t,ye.p=r}}(Ds&3)!==0&&rc(),Va(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===af?fo++:(fo=0,af=e):fo=0,qo(0,!1)}}function S0(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,jo(t)))}function rc(e){return x0(),$0(),w0(),N0(e)}function N0(){if(ht!==5)return!1;var e=Wn,t=ef;ef=0;var a=xf(Ds),n=ne.T,r=ye.p;try{ye.p=32>a?32:a,ne.T=null,a=tf,tf=null;var s=Wn,i=Ds;if(ht=0,zs=Wn=null,Ds=0,(we&6)!==0)throw Error(j(331));var o=we;if(we|=4,c0(s.current),o0(s,s.current,i,a),we=o,qo(0,!1),Wt&&typeof Wt.onPostCommitFiberRoot=="function")try{Wt.onPostCommitFiberRoot(To,s)}catch{}return!0}finally{ye.p=r,ne.T=n,S0(e,t)}}function qg(e,t,a){t=ha(a,t),t=Ym(e.stateNode,t,2),e=Jn(e,t,2),e!==null&&(Do(e,2),Va(e))}function Re(e,t,a){if(e.tag===3)qg(e,e,a);else for(;t!==null;){if(t.tag===3){qg(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(Xn===null||!Xn.has(n))){e=ha(a,e),a=zb(2),n=Jn(t,a,2),n!==null&&(qb(a,n,t,e),Do(n,2),Va(n));break}}t=t.return}}function hm(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new AE;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(Wf=!0,r.add(a),e=PE.bind(null,e,t,a),t.then(e,e))}function PE(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,Ee===e&&(fe&a)===a&&(Ie===4||Ie===3&&(fe&62914560)===fe&&300>Ha()-ep?(we&2)===0&&qs(e,0):Zf|=a,Bs===fe&&(Bs=0)),Va(e)}function _0(e,t){t===0&&(t=xy()),e=Qs(e,t),e!==null&&(Do(e,t),Va(e))}function jE(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),_0(e,a)}function UE(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(j(314))}n!==null&&n.delete(t),_0(e,a)}function FE(e,t){return yf(e,t)}var ju=null,ms=null,rf=!1,Uu=!1,vm=!1,Cr=0;function Va(e){e!==ms&&e.next===null&&(ms===null?ju=ms=e:ms=ms.next=e),Uu=!0,rf||(rf=!0,zE())}function qo(e,t){if(!vm&&Uu){vm=!0;do for(var a=!1,n=ju;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Zt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,Ig(n,s))}else s=fe,s=Qu(n,n===Ee?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||Ao(n,s)||(a=!0,Ig(n,s));n=n.next}while(a);vm=!1}}function BE(){R0()}function R0(){Uu=rf=!1;var e=0;Cr!==0&&(VE()&&(e=Cr),Cr=0);for(var t=Ha(),a=null,n=ju;n!==null;){var r=n.next,s=k0(n,t);s===0?(n.next=null,a===null?ju=r:a.next=r,r===null&&(ms=a)):(a=n,(e!==0||(s&3)!==0)&&(Uu=!0)),n=r}qo(e,!1)}function k0(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Zt(s),o=1<<i,l=r[i];l===-1?((o&a)===0||(o&n)!==0)&&(r[i]=mC(o,t)):l<=t&&(e.expiredLanes|=o),s&=~o}if(t=Ee,a=fe,a=Qu(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&($e===2||$e===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&Id(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||Ao(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&Id(n),xf(a)){case 2:case 8:a=gy;break;case 32:a=bu;break;case 268435456:a=yy;break;default:a=bu}return n=C0.bind(null,e),a=yf(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&Id(n),e.callbackPriority=2,e.callbackNode=null,2}function C0(e,t){if(ht!==0&&ht!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(rc(!0)&&e.callbackNode!==a)return null;var n=fe;return n=Qu(e,e===Ee?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:(p0(e,n,t),k0(e,Ha()),e.callbackNode!=null&&e.callbackNode===a?C0.bind(null,e):null)}function Ig(e,t){if(rc())return null;p0(e,t,!0)}function zE(){YE(function(){(we&6)!==0?yf(vy,BE):R0()})}function ap(){return Cr===0&&(Cr=by()),Cr}function Hg(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:ru(""+e)}function Kg(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function qE(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=Hg((r[zt]||null).action),i=n.submitter;i&&(t=(t=i[zt]||null)?Hg(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Vu("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(Cr!==0){var l=i?Kg(r,i):new FormData(r);Vm(a,{pending:!0,data:l,method:r.method,action:s},null,l)}}else typeof s=="function"&&(o.preventDefault(),l=i?Kg(r,i):new FormData(r),Vm(a,{pending:!0,data:l,method:r.method,action:s},s,l))},currentTarget:r}]})}}for(Wl=0;Wl<Lm.length;Wl++)Zl=Lm[Wl],Qg=Zl.toLowerCase(),Vg=Zl[0].toUpperCase()+Zl.slice(1),Ra(Qg,"on"+Vg);var Zl,Qg,Vg,Wl;Ra(Iy,"onAnimationEnd");Ra(Hy,"onAnimationIteration");Ra(Ky,"onAnimationStart");Ra("dblclick","onDoubleClick");Ra("focusin","onFocus");Ra("focusout","onBlur");Ra(oE,"onTransitionRun");Ra(lE,"onTransitionStart");Ra(uE,"onTransitionCancel");Ra(Qy,"onTransitionEnd");Os("onMouseEnter",["mouseout","mouseover"]);Os("onMouseLeave",["mouseout","mouseover"]);Os("onPointerEnter",["pointerout","pointerover"]);Os("onPointerLeave",["pointerout","pointerover"]);Mr("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Mr("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Mr("onBeforeInput",["compositionend","keypress","textInput","paste"]);Mr("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Mr("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Mr("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var wo="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),IE=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(wo));function E0(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],l=o.instance,c=o.currentTarget;if(o=o.listener,l!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Au(d)}r.currentTarget=null,s=l}else for(i=0;i<n.length;i++){if(o=n[i],l=o.instance,c=o.currentTarget,o=o.listener,l!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Au(d)}r.currentTarget=null,s=l}}}}function le(e,t){var a=t[Cm];a===void 0&&(a=t[Cm]=new Set);var n=e+"__bubble";a.has(n)||(T0(t,e,2,!1),a.add(n))}function gm(e,t,a){var n=0;t&&(n|=4),T0(a,e,n,t)}var eu="_reactListening"+Math.random().toString(36).slice(2);function np(e){if(!e[eu]){e[eu]=!0,Ny.forEach(function(a){a!=="selectionchange"&&(IE.has(a)||gm(a,!1,e),gm(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[eu]||(t[eu]=!0,gm("selectionchange",!1,t))}}function T0(e,t,a,n){switch(q0(t)){case 2:var r=v3;break;case 8:r=g3;break;default:r=op}a=r.bind(null,t,a,e),r=void 0,!Dm||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function ym(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var l=i.tag;if((l===3||l===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=hs(o),i===null)return;if(l=i.tag,l===5||l===6||l===26||l===27){n=s=i;continue e}o=o.parentNode}}n=n.return}Dy(function(){var c=s,d=Sf(a),m=[];e:{var f=Vy.get(e);if(f!==void 0){var h=Vu,x=e;switch(e){case"keypress":if(iu(a)===0)break e;case"keydown":case"keyup":h=FC;break;case"focusin":x="focus",h=Xd;break;case"focusout":x="blur",h=Xd;break;case"beforeblur":case"afterblur":h=Xd;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":h=Zv;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":h=kC;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":h=qC;break;case Iy:case Hy:case Ky:h=TC;break;case Qy:h=HC;break;case"scroll":case"scrollend":h=_C;break;case"wheel":h=QC;break;case"copy":case"cut":case"paste":h=DC;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":h=tg;break;case"toggle":case"beforetoggle":h=GC}var y=(t&4)!==0,$=!y&&(e==="scroll"||e==="scrollend"),g=y?f!==null?f+"Capture":null:f;y=[];for(var v=c,b;v!==null;){var w=v;if(b=w.stateNode,w=w.tag,w!==5&&w!==26&&w!==27||b===null||g===null||(w=vo(v,g),w!=null&&y.push(So(v,w,b))),$)break;v=v.return}0<y.length&&(f=new h(f,x,null,a,d),m.push({event:f,listeners:y}))}}if((t&7)===0){e:{if(f=e==="mouseover"||e==="pointerover",h=e==="mouseout"||e==="pointerout",f&&a!==Am&&(x=a.relatedTarget||a.fromElement)&&(hs(x)||x[Hs]))break e;if((h||f)&&(f=d.window===d?d:(f=d.ownerDocument)?f.defaultView||f.parentWindow:window,h?(x=a.relatedTarget||a.toElement,h=c,x=x?hs(x):null,x!==null&&($=Eo(x),y=x.tag,x!==$||y!==5&&y!==27&&y!==6)&&(x=null)):(h=null,x=c),h!==x)){if(y=Zv,w="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(y=tg,w="onPointerLeave",g="onPointerEnter",v="pointer"),$=h==null?f:Wi(h),b=x==null?f:Wi(x),f=new y(w,v+"leave",h,a,d),f.target=$,f.relatedTarget=b,w=null,hs(d)===c&&(y=new y(g,v+"enter",x,a,d),y.target=b,y.relatedTarget=$,w=y),$=w,h&&x)t:{for(y=h,g=x,v=0,b=y;b;b=us(b))v++;for(b=0,w=g;w;w=us(w))b++;for(;0<v-b;)y=us(y),v--;for(;0<b-v;)g=us(g),b--;for(;v--;){if(y===g||g!==null&&y===g.alternate)break t;y=us(y),g=us(g)}y=null}else y=null;h!==null&&Gg(m,f,h,y,!1),x!==null&&$!==null&&Gg(m,$,x,y,!0)}}e:{if(f=c?Wi(c):window,h=f.nodeName&&f.nodeName.toLowerCase(),h==="select"||h==="input"&&f.type==="file")var S=sg;else if(rg(f))if(Uy)S=rE;else{S=aE;var E=tE}else h=f.nodeName,!h||h.toLowerCase()!=="input"||f.type!=="checkbox"&&f.type!=="radio"?c&&wf(c.elementType)&&(S=sg):S=nE;if(S&&(S=S(e,c))){jy(m,S,a,d);break e}E&&E(e,f,c),e==="focusout"&&c&&f.type==="number"&&c.memoizedProps.value!=null&&Tm(f,"number",f.value)}switch(E=c?Wi(c):window,e){case"focusin":(rg(E)||E.contentEditable==="true")&&(ys=E,Mm=c,ao=null);break;case"focusout":ao=Mm=ys=null;break;case"mousedown":Om=!0;break;case"contextmenu":case"mouseup":case"dragend":Om=!1,ug(m,a,d);break;case"selectionchange":if(iE)break;case"keydown":case"keyup":ug(m,a,d)}var N;if(Rf)e:{switch(e){case"compositionstart":var T="onCompositionStart";break e;case"compositionend":T="onCompositionEnd";break e;case"compositionupdate":T="onCompositionUpdate";break e}T=void 0}else gs?Ly(e,a)&&(T="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(T="onCompositionStart");T&&(Oy&&a.locale!=="ko"&&(gs||T!=="onCompositionStart"?T==="onCompositionEnd"&&gs&&(N=My()):(Kn=d,Nf="value"in Kn?Kn.value:Kn.textContent,gs=!0)),E=Fu(c,T),0<E.length&&(T=new eg(T,e,null,a,d),m.push({event:T,listeners:E}),N?T.data=N:(N=Py(a),N!==null&&(T.data=N)))),(N=JC?XC(e,a):WC(e,a))&&(T=Fu(c,"onBeforeInput"),0<T.length&&(E=new eg("onBeforeInput","beforeinput",null,a,d),m.push({event:E,listeners:T}),E.data=N)),qE(m,e,c,a,d)}E0(m,t)})}function So(e,t,a){return{instance:e,listener:t,currentTarget:a}}function Fu(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=vo(e,a),r!=null&&n.unshift(So(e,r,s)),r=vo(e,t),r!=null&&n.push(So(e,r,s))),e.tag===3)return n;e=e.return}return[]}function us(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function Gg(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,l=o.alternate,c=o.stateNode;if(o=o.tag,l!==null&&l===n)break;o!==5&&o!==26&&o!==27||c===null||(l=c,r?(c=vo(a,s),c!=null&&i.unshift(So(a,c,l))):r||(c=vo(a,s),c!=null&&i.push(So(a,c,l)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var HE=/\r\n?/g,KE=/\u0000|\uFFFD/g;function Yg(e){return(typeof e=="string"?e:""+e).replace(HE,`
`).replace(KE,"")}function A0(e,t){return t=Yg(t),Yg(e)===t}function sc(){}function Ne(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||Ls(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&Ls(e,""+n);break;case"className":Il(e,"class",n);break;case"tabIndex":Il(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":Il(e,a,n);break;case"style":Ay(e,n,s);break;case"data":if(t!=="object"){Il(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=ru(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&Ne(e,t,"name",r.name,r,null),Ne(e,t,"formEncType",r.formEncType,r,null),Ne(e,t,"formMethod",r.formMethod,r,null),Ne(e,t,"formTarget",r.formTarget,r,null)):(Ne(e,t,"encType",r.encType,r,null),Ne(e,t,"method",r.method,r,null),Ne(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=ru(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=sc);break;case"onScroll":n!=null&&le("scroll",e);break;case"onScrollEnd":n!=null&&le("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(j(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(j(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=ru(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":le("beforetoggle",e),le("toggle",e),nu(e,"popover",n);break;case"xlinkActuate":sn(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":sn(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":sn(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":sn(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":sn(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":sn(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":sn(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":sn(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":sn(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":nu(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=SC.get(a)||a,nu(e,a,n))}}function sf(e,t,a,n,r,s){switch(a){case"style":Ay(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(j(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(j(60));e.innerHTML=a}}break;case"children":typeof n=="string"?Ls(e,n):(typeof n=="number"||typeof n=="bigint")&&Ls(e,""+n);break;case"onScroll":n!=null&&le("scroll",e);break;case"onScrollEnd":n!=null&&le("scrollend",e);break;case"onClick":n!=null&&(e.onclick=sc);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!_y.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[zt]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):nu(e,a,n)}}}function vt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":le("error",e),le("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(j(137,t));default:Ne(e,t,s,i,a,null)}}r&&Ne(e,t,"srcSet",a.srcSet,a,null),n&&Ne(e,t,"src",a.src,a,null);return;case"input":le("invalid",e);var o=s=i=r=null,l=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":l=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(j(137,t));break;default:Ne(e,t,n,d,a,null)}}Cy(e,s,o,l,c,i,r,!1),xu(e);return;case"select":le("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:Ne(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?Rs(e,!!n,t,!1):a!=null&&Rs(e,!!n,a,!0);return;case"textarea":le("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(j(91));break;default:Ne(e,t,i,o,a,null)}Ty(e,n,r,s),xu(e);return;case"option":for(l in a)if(a.hasOwnProperty(l)&&(n=a[l],n!=null))switch(l){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:Ne(e,t,l,n,a,null)}return;case"dialog":le("beforetoggle",e),le("toggle",e),le("cancel",e),le("close",e);break;case"iframe":case"object":le("load",e);break;case"video":case"audio":for(n=0;n<wo.length;n++)le(wo[n],e);break;case"image":le("error",e),le("load",e);break;case"details":le("toggle",e);break;case"embed":case"source":case"link":le("error",e),le("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(j(137,t));default:Ne(e,t,c,n,a,null)}return;default:if(wf(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&sf(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&Ne(e,t,o,n,a,null))}function QE(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,l=null,c=null,d=null;for(h in a){var m=a[h];if(a.hasOwnProperty(h)&&m!=null)switch(h){case"checked":break;case"value":break;case"defaultValue":l=m;default:n.hasOwnProperty(h)||Ne(e,t,h,null,n,m)}}for(var f in n){var h=n[f];if(m=a[f],n.hasOwnProperty(f)&&(h!=null||m!=null))switch(f){case"type":s=h;break;case"name":r=h;break;case"checked":c=h;break;case"defaultChecked":d=h;break;case"value":i=h;break;case"defaultValue":o=h;break;case"children":case"dangerouslySetInnerHTML":if(h!=null)throw Error(j(137,t));break;default:h!==m&&Ne(e,t,f,h,n,m)}}Em(e,i,o,l,c,d,s,r);return;case"select":h=i=o=f=null;for(s in a)if(l=a[s],a.hasOwnProperty(s)&&l!=null)switch(s){case"value":break;case"multiple":h=l;default:n.hasOwnProperty(s)||Ne(e,t,s,null,n,l)}for(r in n)if(s=n[r],l=a[r],n.hasOwnProperty(r)&&(s!=null||l!=null))switch(r){case"value":f=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==l&&Ne(e,t,r,s,n,l)}t=o,a=i,n=h,f!=null?Rs(e,!!a,f,!1):!!n!=!!a&&(t!=null?Rs(e,!!a,t,!0):Rs(e,!!a,a?[]:"",!1));return;case"textarea":h=f=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:Ne(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":f=r;break;case"defaultValue":h=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(j(91));break;default:r!==s&&Ne(e,t,i,r,n,s)}Ey(e,f,h);return;case"option":for(var x in a)if(f=a[x],a.hasOwnProperty(x)&&f!=null&&!n.hasOwnProperty(x))switch(x){case"selected":e.selected=!1;break;default:Ne(e,t,x,null,n,f)}for(l in n)if(f=n[l],h=a[l],n.hasOwnProperty(l)&&f!==h&&(f!=null||h!=null))switch(l){case"selected":e.selected=f&&typeof f!="function"&&typeof f!="symbol";break;default:Ne(e,t,l,f,n,h)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var y in a)f=a[y],a.hasOwnProperty(y)&&f!=null&&!n.hasOwnProperty(y)&&Ne(e,t,y,null,n,f);for(c in n)if(f=n[c],h=a[c],n.hasOwnProperty(c)&&f!==h&&(f!=null||h!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(f!=null)throw Error(j(137,t));break;default:Ne(e,t,c,f,n,h)}return;default:if(wf(t)){for(var $ in a)f=a[$],a.hasOwnProperty($)&&f!==void 0&&!n.hasOwnProperty($)&&sf(e,t,$,void 0,n,f);for(d in n)f=n[d],h=a[d],!n.hasOwnProperty(d)||f===h||f===void 0&&h===void 0||sf(e,t,d,f,n,h);return}}for(var g in a)f=a[g],a.hasOwnProperty(g)&&f!=null&&!n.hasOwnProperty(g)&&Ne(e,t,g,null,n,f);for(m in n)f=n[m],h=a[m],!n.hasOwnProperty(m)||f===h||f==null&&h==null||Ne(e,t,m,f,n,h)}var of=null,lf=null;function Bu(e){return e.nodeType===9?e:e.ownerDocument}function Jg(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function D0(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function uf(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var bm=null;function VE(){var e=window.event;return e&&e.type==="popstate"?e===bm?!1:(bm=e,!0):(bm=null,!1)}var M0=typeof setTimeout=="function"?setTimeout:void 0,GE=typeof clearTimeout=="function"?clearTimeout:void 0,Xg=typeof Promise=="function"?Promise:void 0,YE=typeof queueMicrotask=="function"?queueMicrotask:typeof Xg<"u"?function(e){return Xg.resolve(null).then(e).catch(JE)}:M0;function JE(e){setTimeout(function(){throw e})}function ir(e){return e==="head"}function Wg(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&po(i.documentElement),a&2&&po(i.body),a&4)for(a=i.head,po(a),i=a.firstChild;i;){var o=i.nextSibling,l=i.nodeName;i[Mo]||l==="SCRIPT"||l==="STYLE"||l==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),Co(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);Co(t)}function cf(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":cf(a),$f(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function XE(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[Mo])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=_a(e.nextSibling),e===null)break}return null}function WE(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=_a(e.nextSibling),e===null))return null;return e}function df(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function ZE(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function _a(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var mf=null;function Zg(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function O0(e,t,a){switch(t=Bu(a),e){case"html":if(e=t.documentElement,!e)throw Error(j(452));return e;case"head":if(e=t.head,!e)throw Error(j(453));return e;case"body":if(e=t.body,!e)throw Error(j(454));return e;default:throw Error(j(451))}}function po(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);$f(e)}var ya=new Map,ey=new Set;function zu(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var $n=ye.d;ye.d={f:e3,r:t3,D:a3,C:n3,L:r3,m:s3,X:o3,S:i3,M:l3};function e3(){var e=$n.f(),t=ac();return e||t}function t3(e){var t=Ks(e);t!==null&&t.tag===5&&t.type==="form"?kb(t):$n.r(e)}var Gs=typeof document>"u"?null:document;function L0(e,t,a){var n=Gs;if(n&&typeof t=="string"&&t){var r=pa(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),ey.has(r)||(ey.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),vt(t,"link",e),ut(t),n.head.appendChild(t)))}}function a3(e){$n.D(e),L0("dns-prefetch",e,null)}function n3(e,t){$n.C(e,t),L0("preconnect",e,t)}function r3(e,t,a){$n.L(e,t,a);var n=Gs;if(n&&e&&t){var r='link[rel="preload"][as="'+pa(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+pa(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+pa(a.imageSizes)+'"]')):r+='[href="'+pa(e)+'"]';var s=r;switch(t){case"style":s=Is(e);break;case"script":s=Ys(e)}ya.has(s)||(e=De({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),ya.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(Io(s))||t==="script"&&n.querySelector(Ho(s))||(t=n.createElement("link"),vt(t,"link",e),ut(t),n.head.appendChild(t)))}}function s3(e,t){$n.m(e,t);var a=Gs;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+pa(n)+'"][href="'+pa(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=Ys(e)}if(!ya.has(s)&&(e=De({rel:"modulepreload",href:e},t),ya.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(Ho(s)))return}n=a.createElement("link"),vt(n,"link",e),ut(n),a.head.appendChild(n)}}}function i3(e,t,a){$n.S(e,t,a);var n=Gs;if(n&&e){var r=_s(n).hoistableStyles,s=Is(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(Io(s)))o.loading=5;else{e=De({rel:"stylesheet",href:e,"data-precedence":t},a),(a=ya.get(s))&&rp(e,a);var l=i=n.createElement("link");ut(l),vt(l,"link",e),l._p=new Promise(function(c,d){l.onload=c,l.onerror=d}),l.addEventListener("load",function(){o.loading|=1}),l.addEventListener("error",function(){o.loading|=2}),o.loading|=4,fu(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function o3(e,t){$n.X(e,t);var a=Gs;if(a&&e){var n=_s(a).hoistableScripts,r=Ys(e),s=n.get(r);s||(s=a.querySelector(Ho(r)),s||(e=De({src:e,async:!0},t),(t=ya.get(r))&&sp(e,t),s=a.createElement("script"),ut(s),vt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function l3(e,t){$n.M(e,t);var a=Gs;if(a&&e){var n=_s(a).hoistableScripts,r=Ys(e),s=n.get(r);s||(s=a.querySelector(Ho(r)),s||(e=De({src:e,async:!0,type:"module"},t),(t=ya.get(r))&&sp(e,t),s=a.createElement("script"),ut(s),vt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function ty(e,t,a,n){var r=(r=Gn.current)?zu(r):null;if(!r)throw Error(j(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=Is(a.href),a=_s(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=Is(a.href);var s=_s(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(Io(e)))&&!s._p&&(i.instance=s,i.state.loading=5),ya.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},ya.set(e,a),s||u3(r,e,a,i.state))),t&&n===null)throw Error(j(528,""));return i}if(t&&n!==null)throw Error(j(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=Ys(a),a=_s(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(j(444,e))}}function Is(e){return'href="'+pa(e)+'"'}function Io(e){return'link[rel="stylesheet"]['+e+"]"}function P0(e){return De({},e,{"data-precedence":e.precedence,precedence:null})}function u3(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),vt(t,"link",a),ut(t),e.head.appendChild(t))}function Ys(e){return'[src="'+pa(e)+'"]'}function Ho(e){return"script[async]"+e}function ay(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+pa(a.href)+'"]');if(n)return t.instance=n,ut(n),n;var r=De({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),ut(n),vt(n,"style",r),fu(n,a.precedence,e),t.instance=n;case"stylesheet":r=Is(a.href);var s=e.querySelector(Io(r));if(s)return t.state.loading|=4,t.instance=s,ut(s),s;n=P0(a),(r=ya.get(r))&&rp(n,r),s=(e.ownerDocument||e).createElement("link"),ut(s);var i=s;return i._p=new Promise(function(o,l){i.onload=o,i.onerror=l}),vt(s,"link",n),t.state.loading|=4,fu(s,a.precedence,e),t.instance=s;case"script":return s=Ys(a.src),(r=e.querySelector(Ho(s)))?(t.instance=r,ut(r),r):(n=a,(r=ya.get(s))&&(n=De({},a),sp(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),ut(r),vt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(j(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,fu(n,a.precedence,e));return t.instance}function fu(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function rp(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function sp(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var pu=null;function ny(e,t,a){if(pu===null){var n=new Map,r=pu=new Map;r.set(a,n)}else r=pu,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[Mo]||s[yt]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function ry(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function c3(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function j0(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var No=null;function d3(){}function m3(e,t,a){if(No===null)throw Error(j(475));var n=No;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=Is(a.href),s=e.querySelector(Io(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=qu.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,ut(s);return}s=e.ownerDocument||e,a=P0(a),(r=ya.get(r))&&rp(a,r),s=s.createElement("link"),ut(s);var i=s;i._p=new Promise(function(o,l){i.onload=o,i.onerror=l}),vt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=qu.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function f3(){if(No===null)throw Error(j(475));var e=No;return e.stylesheets&&e.count===0&&ff(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&ff(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function qu(){if(this.count--,this.count===0){if(this.stylesheets)ff(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var Iu=null;function ff(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,Iu=new Map,t.forEach(p3,e),Iu=null,qu.call(e))}function p3(e,t){if(!(t.state.loading&4)){var a=Iu.get(e);if(a)var n=a.get(null);else{a=new Map,Iu.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=qu.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var _o={$$typeof:cn,Provider:null,Consumer:null,_currentValue:wr,_currentValue2:wr,_threadCount:0};function h3(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=Hd(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=Hd(0),this.hiddenUpdates=Hd(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function U0(e,t,a,n,r,s,i,o,l,c,d,m){return e=new h3(e,t,a,i,o,l,c,m),t=1,s===!0&&(t|=24),s=Xt(3,null,null,t),e.current=s,s.stateNode=e,t=Mf(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},Lf(s),e}function F0(e){return e?(e=$s,e):$s}function B0(e,t,a,n,r,s){r=F0(r),n.context===null?n.context=r:n.pendingContext=r,n=Yn(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=Jn(e,n,t),a!==null&&(ta(a,e,t),so(a,e,t))}function sy(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function ip(e,t){sy(e,t),(e=e.alternate)&&sy(e,t)}function z0(e){if(e.tag===13){var t=Qs(e,67108864);t!==null&&ta(t,e,67108864),ip(e,67108864)}}var Hu=!0;function v3(e,t,a,n){var r=ne.T;ne.T=null;var s=ye.p;try{ye.p=2,op(e,t,a,n)}finally{ye.p=s,ne.T=r}}function g3(e,t,a,n){var r=ne.T;ne.T=null;var s=ye.p;try{ye.p=8,op(e,t,a,n)}finally{ye.p=s,ne.T=r}}function op(e,t,a,n){if(Hu){var r=pf(n);if(r===null)ym(e,t,n,Ku,a),iy(e,n);else if(b3(r,e,t,a,n))n.stopPropagation();else if(iy(e,n),t&4&&-1<y3.indexOf(e)){for(;r!==null;){var s=Ks(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=br(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var l=1<<31-Zt(i);o.entanglements[1]|=l,i&=~l}Va(s),(we&6)===0&&(Lu=Ha()+500,qo(0,!1))}}break;case 13:o=Qs(s,2),o!==null&&ta(o,s,2),ac(),ip(s,2)}if(s=pf(n),s===null&&ym(e,t,n,Ku,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else ym(e,t,n,null,a)}}function pf(e){return e=Sf(e),lp(e)}var Ku=null;function lp(e){if(Ku=null,e=hs(e),e!==null){var t=Eo(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=my(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return Ku=e,null}function q0(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(sC()){case vy:return 2;case gy:return 8;case bu:case iC:return 32;case yy:return 268435456;default:return 32}default:return 32}}var hf=!1,Zn=null,er=null,tr=null,Ro=new Map,ko=new Map,In=[],y3="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function iy(e,t){switch(e){case"focusin":case"focusout":Zn=null;break;case"dragenter":case"dragleave":er=null;break;case"mouseover":case"mouseout":tr=null;break;case"pointerover":case"pointerout":Ro.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":ko.delete(t.pointerId)}}function Gi(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Ks(t),t!==null&&z0(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function b3(e,t,a,n,r){switch(t){case"focusin":return Zn=Gi(Zn,e,t,a,n,r),!0;case"dragenter":return er=Gi(er,e,t,a,n,r),!0;case"mouseover":return tr=Gi(tr,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return Ro.set(s,Gi(Ro.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,ko.set(s,Gi(ko.get(s)||null,e,t,a,n,r)),!0}return!1}function I0(e){var t=hs(e.target);if(t!==null){var a=Eo(t);if(a!==null){if(t=a.tag,t===13){if(t=my(a),t!==null){e.blockedOn=t,pC(e.priority,function(){if(a.tag===13){var n=ea();n=bf(n);var r=Qs(a,n);r!==null&&ta(r,a,n),ip(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function hu(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=pf(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);Am=n,a.target.dispatchEvent(n),Am=null}else return t=Ks(a),t!==null&&z0(t),e.blockedOn=a,!1;t.shift()}return!0}function oy(e,t,a){hu(e)&&a.delete(t)}function x3(){hf=!1,Zn!==null&&hu(Zn)&&(Zn=null),er!==null&&hu(er)&&(er=null),tr!==null&&hu(tr)&&(tr=null),Ro.forEach(oy),ko.forEach(oy)}function tu(e,t){e.blockedOn===t&&(e.blockedOn=null,hf||(hf=!0,rt.unstable_scheduleCallback(rt.unstable_NormalPriority,x3)))}var au=null;function ly(e){au!==e&&(au=e,rt.unstable_scheduleCallback(rt.unstable_NormalPriority,function(){au===e&&(au=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(lp(n||a)===null)continue;break}var s=Ks(a);s!==null&&(e.splice(t,3),t-=3,Vm(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function Co(e){function t(l){return tu(l,e)}Zn!==null&&tu(Zn,e),er!==null&&tu(er,e),tr!==null&&tu(tr,e),Ro.forEach(t),ko.forEach(t);for(var a=0;a<In.length;a++){var n=In[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<In.length&&(a=In[0],a.blockedOn===null);)I0(a),a.blockedOn===null&&In.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[zt]||null;if(typeof s=="function")i||ly(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[zt]||null)o=i.formAction;else if(lp(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),ly(a)}}}function up(e){this._internalRoot=e}ic.prototype.render=up.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(j(409));var a=t.current,n=ea();B0(a,n,e,t,null,null)};ic.prototype.unmount=up.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;B0(e.current,2,null,e,null,null),ac(),t[Hs]=null}};function ic(e){this._internalRoot=e}ic.prototype.unstable_scheduleHydration=function(e){if(e){var t=Sy();e={blockedOn:null,target:e,priority:t};for(var a=0;a<In.length&&t!==0&&t<In[a].priority;a++);In.splice(a,0,e),a===0&&I0(e)}};var uy=cy.version;if(uy!=="19.1.0")throw Error(j(527,uy,"19.1.0"));ye.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(j(188)):(e=Object.keys(e).join(","),Error(j(268,e)));return e=Wk(t),e=e!==null?fy(e):null,e=e===null?null:e.stateNode,e};var $3={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:ne,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(Yi=__REACT_DEVTOOLS_GLOBAL_HOOK__,!Yi.isDisabled&&Yi.supportsFiber))try{To=Yi.inject($3),Wt=Yi}catch{}var Yi;oc.createRoot=function(e,t){if(!dy(e))throw Error(j(299));var a=!1,n="",r=Ub,s=Fb,i=Bb,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=U0(e,1,!1,null,null,a,n,r,s,i,o,null),e[Hs]=t.current,np(e),new up(t)};oc.hydrateRoot=function(e,t,a){if(!dy(e))throw Error(j(299));var n=!1,r="",s=Ub,i=Fb,o=Bb,l=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(l=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=U0(e,1,!0,t,a??null,n,r,s,i,o,l,c),t.context=F0(null),a=t.current,n=ea(),n=bf(n),r=Yn(n),r.callback=null,Jn(a,r,n),a=n,t.current.lanes=a,Do(t,a),Va(t),e[Hs]=t.current,np(e),new ic(t)};oc.version="19.1.0"});var V0=An((pP,Q0)=>{"use strict";function K0(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(K0)}catch(e){console.error(e)}}K0(),Q0.exports=H0()});var Lt=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var Ak={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},Dk=class{#t=Ak;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},Pa=new Dk;function av(e){setTimeout(e,0)}var Pt=typeof window>"u"||"Deno"in globalThis;function Me(){}function sv(e,t){return typeof e=="function"?e(t):e}function Ti(e){return typeof e=="number"&&e>=0&&e!==1/0}function bl(e,t){return Math.max(e+(t||0)-Date.now(),0)}function Sa(e,t){return typeof e=="function"?e(t):e}function jt(e,t){return typeof e=="function"?e(t):e}function xl(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==Ai(i,t.options))return!1}else if(!vr(t.queryKey,i))return!1}if(a!=="all"){let l=t.isActive();if(a==="active"&&!l||a==="inactive"&&l)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function $l(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(ja(t.options.mutationKey)!==ja(s))return!1}else if(!vr(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function Ai(e,t){return(t?.queryKeyHashFn||ja)(e)}function ja(e){return JSON.stringify(e,(t,a)=>$d(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function vr(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>vr(e[a],t[a])):!1}var Mk=Object.prototype.hasOwnProperty;function Di(e,t){if(e===t)return e;let a=nv(e)&&nv(t);if(!a&&!($d(e)&&$d(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},l=0;for(let c=0;c<i;c++){let d=a?c:s[c],m=e[d],f=t[d];if(m===f){o[d]=m,(a?c<r:Mk.call(e,d))&&l++;continue}if(m===null||f===null||typeof m!="object"||typeof f!="object"){o[d]=f;continue}let h=Di(m,f);o[d]=h,h===m&&l++}return r===i&&l===r?e:o}function Dn(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function nv(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function $d(e){if(!rv(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!rv(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function rv(e){return Object.prototype.toString.call(e)==="[object Object]"}function iv(e){return new Promise(t=>{Pa.setTimeout(t,e)})}function Mi(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?Di(e,t):t}function ov(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function lv(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var es=Symbol();function wl(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===es?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function Oi(e,t){return typeof e=="function"?e(...t):!!e}var Ok=class extends Lt{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!Pt&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},ts=new Ok;function Li(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var uv=av;function Lk(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=uv,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(l=>{a(l)})})})};return{batch:o=>{let l;t++;try{l=o()}finally{t--,t||i()}return l},batchCalls:o=>(...l)=>{s(()=>{o(...l)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var me=Lk();var Pk=class extends Lt{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!Pt&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},as=new Pk;function jk(e){return Math.min(1e3*2**e,3e4)}function wd(e){return(e??"online")==="online"?as.isOnline():!0}var Sl=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function Nl(e){let t=!1,a=0,n,r=Li(),s=()=>r.status!=="pending",i=y=>{if(!s()){let $=new Sl(y);f($),e.onCancel?.($)}},o=()=>{t=!0},l=()=>{t=!1},c=()=>ts.isFocused()&&(e.networkMode==="always"||as.isOnline())&&e.canRun(),d=()=>wd(e.networkMode)&&e.canRun(),m=y=>{s()||(n?.(),r.resolve(y))},f=y=>{s()||(n?.(),r.reject(y))},h=()=>new Promise(y=>{n=$=>{(s()||c())&&y($)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),x=()=>{if(s())return;let y,$=a===0?e.initialPromise:void 0;try{y=$??e.fn()}catch(g){y=Promise.reject(g)}Promise.resolve(y).then(m).catch(g=>{if(s())return;let v=e.retry??(Pt?0:3),b=e.retryDelay??jk,w=typeof b=="function"?b(a,g):b,S=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!S){f(g);return}a++,e.onFail?.(a,g),iv(w).then(()=>c()?void 0:h()).then(()=>{t?f(g):x()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:l,canStart:d,start:()=>(d()?x():h().then(x),r)}}var _l=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),Ti(this.gcTime)&&(this.#t=Pa.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(Pt?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(Pa.clearTimeout(this.#t),this.#t=void 0)}};var dv=class extends _l{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=cv(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=cv(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=Mi(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(Me).catch(Me):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>jt(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===es||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>Sa(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!bl(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(l=>l.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=wl(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=Nl({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof Sl&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,l)=>{this.#i({type:"failed",failureCount:o,error:l})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof Sl){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,...Sd(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),me.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function Sd(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:wd(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function cv(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var gr=class extends Lt{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=Li(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),mv(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return Nd(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return Nd(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof jt(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!Dn(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&fv(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||jt(this.options.enabled,this.#e)!==jt(t.enabled,this.#e)||Sa(this.options.staleTime,this.#e)!==Sa(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||jt(this.options.enabled,this.#e)!==jt(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return Fk(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(Me)),t}#v(){this.#x();let e=Sa(this.options.staleTime,this.#e);if(Pt||this.#n.isStale||!Ti(e))return;let a=bl(this.#n.dataUpdatedAt,e)+1;this.#u=Pa.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(Pt||jt(this.options.enabled,this.#e)===!1||!Ti(this.#l)||this.#l===0)&&(this.#c=Pa.setInterval(()=>{(this.options.refetchIntervalInBackground||ts.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(Pa.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(Pa.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,l=e!==a?e.state:this.#a,{state:c}=e,d={...c},m=!1,f;if(t._optimisticResults){let T=this.hasListeners(),L=!T&&mv(e,t),O=T&&fv(e,a,t,n);(L||O)&&(d={...d,...Sd(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:h,errorUpdatedAt:x,status:y}=d;f=d.data;let $=!1;if(t.placeholderData!==void 0&&f===void 0&&y==="pending"){let T;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(T=r.data,$=!0):T=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,T!==void 0&&(y="success",f=Mi(r?.data,T,t),m=!0)}if(t.select&&f!==void 0&&!$)if(r&&f===s?.data&&t.select===this.#f)f=this.#d;else try{this.#f=t.select,f=t.select(f),f=Mi(r?.data,f,t),this.#d=f,this.#i=null}catch(T){this.#i=T}this.#i&&(h=this.#i,f=this.#d,x=Date.now(),y="error");let g=d.fetchStatus==="fetching",v=y==="pending",b=y==="error",w=v&&g,S=f!==void 0,N={status:y,fetchStatus:d.fetchStatus,isPending:v,isSuccess:y==="success",isError:b,isInitialLoading:w,isLoading:w,data:f,dataUpdatedAt:d.dataUpdatedAt,error:h,errorUpdatedAt:x,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>l.dataUpdateCount||d.errorUpdateCount>l.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:b&&!S,isPaused:d.fetchStatus==="paused",isPlaceholderData:m,isRefetchError:b&&S,isStale:_d(e,t),refetch:this.refetch,promise:this.#o,isEnabled:jt(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let T=P=>{N.status==="error"?P.reject(N.error):N.data!==void 0&&P.resolve(N.data)},L=()=>{let P=this.#o=N.promise=Li();T(P)},O=this.#o;switch(O.status){case"pending":e.queryHash===a.queryHash&&T(O);break;case"fulfilled":(N.status==="error"||N.data!==O.value)&&L();break;case"rejected":(N.status!=="error"||N.error!==O.reason)&&L();break}}return N}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),Dn(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){me.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function Uk(e,t){return jt(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function mv(e,t){return Uk(e,t)||e.state.data!==void 0&&Nd(e,t,t.refetchOnMount)}function Nd(e,t,a){if(jt(t.enabled,e)!==!1&&Sa(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&_d(e,t)}return!1}function fv(e,t,a,n){return(e!==t||jt(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&_d(e,a)}function _d(e,t){return jt(t.enabled,e)!==!1&&e.isStaleByTime(Sa(t.staleTime,e))}function Fk(e,t){return!Dn(e.getCurrentResult(),t)}function Rd(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},l=0,c=async()=>{let d=!1,m=x=>{Object.defineProperty(x,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},f=wl(t.options,t.fetchOptions),h=async(x,y,$)=>{if(d)return Promise.reject();if(y==null&&x.pages.length)return Promise.resolve(x);let v=(()=>{let E={client:t.client,queryKey:t.queryKey,pageParam:y,direction:$?"backward":"forward",meta:t.options.meta};return m(E),E})(),b=await f(v),{maxPages:w}=t.options,S=$?lv:ov;return{pages:S(x.pages,b,w),pageParams:S(x.pageParams,y,w)}};if(r&&s.length){let x=r==="backward",y=x?Bk:pv,$={pages:s,pageParams:i},g=y(n,$);o=await h($,g,x)}else{let x=e??s.length;do{let y=l===0?i[0]??n.initialPageParam:pv(n,o);if(l>0&&y==null)break;o=await h(o,y),l++}while(l<x)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function pv(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function Bk(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var hv=class extends _l{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||kd(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=Nl({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),me.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function kd(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var vv=class extends Lt{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new hv({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=Rl(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=Rl(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=Rl(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=Rl(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){me.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>$l(t,a))}findAll(e={}){return this.getAll().filter(t=>$l(e,t))}notify(e){me.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return me.batch(()=>Promise.all(e.map(t=>t.continue().catch(Me))))}};function Rl(e){return e.options.scope?.id}var Cd=class extends Lt{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),Dn(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&ja(t.mutationKey)!==ja(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??kd();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){me.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function gv(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function zk(e,t,a){let n=e.slice(0);return n[t]=a,n}var Ed=class extends Lt{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,me.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,m)=>d!==a[m]),l=i||o,c=l?!0:s.some((d,m)=>{let f=this.#e[m];return!f||!Dn(d,f)});!l&&!c||(l&&(this.#r=r),this.#e=s,this.hasListeners()&&(l&&(gv(a,r).forEach(d=>{d.destroy()}),gv(r,a).forEach(d=>{d.subscribe(m=>{this.#c(d,m)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=Di(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new gr(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=zk(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&me.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var yv=class extends Lt{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??Ai(n,t),s=this.get(r);return s||(s=new dv({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){me.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>xl(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>xl(e,a)):t}notify(e){me.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){me.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){me.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var Td=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new yv,this.#e=e.mutationCache||new vv,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=ts.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=as.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime(Sa(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=sv(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return me.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;me.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return me.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=me.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(Me).catch(Me)}invalidateQueries(e,t={}){return me.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=me.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(Me)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(Me)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime(Sa(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(Me).catch(Me)}fetchInfiniteQuery(e){return e.behavior=Rd(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(Me).catch(Me)}ensureInfiniteQueryData(e){return e.behavior=Rd(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return as.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(ja(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{vr(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(ja(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{vr(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=Ai(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===es&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var Ua=ze(Ke(),1);var ns=ze(Ke(),1),wv=ze(Ad(),1),Dd=ns.createContext(void 0),W=e=>{let t=ns.useContext(Dd);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},Md=({client:e,children:t})=>(ns.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,wv.jsx)(Dd.Provider,{value:e,children:t}));var Cl=ze(Ke(),1),Sv=Cl.createContext(!1),El=()=>Cl.useContext(Sv),T6=Sv.Provider;var Pi=ze(Ke(),1),Hk=ze(Ad(),1);function Kk(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var Qk=Pi.createContext(Kk()),Tl=()=>Pi.useContext(Qk);var Nv=ze(Ke(),1);var Al=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},Dl=e=>{Nv.useEffect(()=>{e.clearReset()},[e])},Ml=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||Oi(a,[e.error,n]));var Ol=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},Ll=(e,t)=>e.isLoading&&e.isFetching&&!t,ji=(e,t)=>e?.suspense&&t.isPending,rs=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function Od({queries:e,...t},a){let n=W(a),r=El(),s=Tl(),i=Ua.useMemo(()=>e.map(y=>{let $=n.defaultQueryOptions(y);return $._optimisticResults=r?"isRestoring":"optimistic",$}),[e,n,r]);i.forEach(y=>{Ol(y),Al(y,s)}),Dl(s);let[o]=Ua.useState(()=>new Ed(n,i,t)),[l,c,d]=o.getOptimisticResult(i,t.combine),m=!r&&t.subscribed!==!1;Ua.useSyncExternalStore(Ua.useCallback(y=>m?o.subscribe(me.batchCalls(y)):Me,[o,m]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),Ua.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let h=l.some((y,$)=>ji(i[$],y))?l.flatMap((y,$)=>{let g=i[$];if(g){let v=new gr(n,g);if(ji(g,y))return rs(g,v,s);Ll(y,r)&&rs(g,v,s)}return[]}):[];if(h.length>0)throw Promise.all(h);let x=l.find((y,$)=>{let g=i[$];return g&&Ml({result:y,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(x?.error)throw x.error;return c(d())}var Mn=ze(Ke(),1);function _v(e,t,a){let n=El(),r=Tl(),s=W(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",Ol(i),Al(i,r),Dl(r);let o=!s.getQueryCache().get(i.queryHash),[l]=Mn.useState(()=>new t(s,i)),c=l.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(Mn.useSyncExternalStore(Mn.useCallback(m=>{let f=d?l.subscribe(me.batchCalls(m)):Me;return l.updateResult(),f},[l,d]),()=>l.getCurrentResult(),()=>l.getCurrentResult()),Mn.useEffect(()=>{l.setOptions(i)},[i,l]),ji(i,c))throw rs(i,l,r);if(Ml({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!Pt&&Ll(c,n)&&(o?rs(i,l,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(Me).finally(()=>{l.updateResult()}),i.notifyOnChangeProps?c:l.trackResult(c)}function H(e,t){return _v(e,gr,t)}var nn=ze(Ke(),1);function V(e,t){let a=W(t),[n]=nn.useState(()=>new Cd(a,e));nn.useEffect(()=>{n.setOptions(e)},[n,e]);let r=nn.useSyncExternalStore(nn.useCallback(i=>n.subscribe(me.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=nn.useCallback((i,o)=>{n.mutate(i,o).catch(Me)},[n]);if(r.error&&Oi(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var Ck=ze(V0());var It=ze(Ke(),1),J=ze(Ke(),1),ke=ze(Ke(),1),Ep=ze(Ke(),1),Tx=ze(Ke(),1),ce=ze(Ke(),1),PT=ze(Ke(),1),jT=ze(Ke(),1),UT=ze(Ke(),1),Z=ze(Ke(),1),zx=ze(Ke(),1);var gp=/^(?:[a-z][a-z0-9+.-]*:|[\\/]{2})/i,ax=/^[\\/]{2}/;function w3(e,t){return t+e.replace(/\\/g,"/")}var G0="popstate";function Y0(e){return typeof e=="object"&&e!=null&&"pathname"in e&&"search"in e&&"hash"in e&&"state"in e&&"key"in e}function nx(e={}){function t(n,r){let s=r.state?.masked,{pathname:i,search:o,hash:l}=s||n.location;return fp("",{pathname:i,search:o,hash:l},r.state&&r.state.usr||null,r.state&&r.state.key||"default",s?{pathname:n.location.pathname,search:n.location.search,hash:n.location.hash}:void 0)}function a(n,r){return typeof r=="string"?r:Js(r)}return N3(t,a,null,e)}function Te(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function na(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function S3(){return Math.random().toString(36).substring(2,10)}function J0(e,t){return{usr:e.state,key:e.key,idx:t,masked:e.mask?{pathname:e.pathname,search:e.search,hash:e.hash}:void 0}}function fp(e,t,a=null,n,r){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?jr(t):t,state:a,key:t&&t.key||n||S3(),mask:r}}function Js({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function jr(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function N3(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",l=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function m(){o="POP";let $=d(),g=$==null?null:$-c;c=$,l&&l({action:o,location:y.location,delta:g})}function f($,g){o="PUSH";let v=Y0($)?$:fp(y.location,$,g);a&&a(v,$),c=d()+1;let b=J0(v,c),w=y.createHref(v.mask||v);try{i.pushState(b,"",w)}catch(S){if(S instanceof DOMException&&S.name==="DataCloneError")throw S;r.location.assign(w)}s&&l&&l({action:o,location:y.location,delta:1})}function h($,g){o="REPLACE";let v=Y0($)?$:fp(y.location,$,g);a&&a(v,$),c=d();let b=J0(v,c),w=y.createHref(v.mask||v);i.replaceState(b,"",w),s&&l&&l({action:o,location:y.location,delta:0})}function x($){return _3(r,$)}let y={get action(){return o},get location(){return e(r,i)},listen($){if(l)throw new Error("A history only accepts one active listener");return r.addEventListener(G0,m),l=$,()=>{r.removeEventListener(G0,m),l=null}},createHref($){return t(r,$)},createURL:x,encodeLocation($){let g=x($);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:f,replace:h,go($){return i.go($)}};return y}function _3(e,t,a=!1){let n="http://localhost";e&&(n=e.location.origin!=="null"?e.location.origin:e.location.href),Te(n,"No window.location.(origin|href) available to create URL");let r=typeof t=="string"?t:Js(t);return r=r.replace(/ $/,"%20"),!a&&ax.test(r)&&(r=n+r),new URL(r,n)}var R3;R3=new WeakMap;function yp(e,t,a="/"){return k3(e,t,a,!1)}function k3(e,t,a,n,r){let s=typeof t=="string"?jr(t):t,i=Ga(s.pathname||"/",a);if(i==null)return null;let o=r??E3(e),l=null,c=B3(i);for(let d=0;l==null&&d<o.length;++d)l=F3(o[d],c,n);return l}function C3(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function E3(e){let t=rx(e);return T3(t),t}function rx(e,t=[],a=[],n="",r=!1){let s=(i,o,l=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&l)return;Te(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let m=ka([n,d.relativePath]),f=a.concat(d);i.children&&i.children.length>0&&(Te(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${m}".`),rx(i.children,t,f,m,l)),!(i.path==null&&!i.index)&&t.push({path:m,score:j3(m,i.index),routesMeta:f.map((h,x)=>{let[y,$]=ox(h.relativePath,h.caseSensitive,x===f.length-1);return{...h,matcher:y,compiledParams:$}})})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let l of sx(i.path))s(i,o,!0,l)}),t}function sx(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=sx(n.join("/")),o=[];return o.push(...i.map(l=>l===""?s:[s,l].join("/"))),r&&o.push(...i),o.map(l=>e.startsWith("/")&&l===""?"/":l)}function T3(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:U3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var A3=/^:[\w-]+$/,D3=3,M3=2,O3=1,L3=10,P3=-2,X0=e=>e==="*";function j3(e,t){let a=e.split("/"),n=a.length;return a.some(X0)&&(n+=P3),t&&(n+=M3),a.filter(r=>!X0(r)).reduce((r,s)=>r+(A3.test(s)?D3:s===""?O3:L3),n)}function U3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function F3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let l=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",m={path:l.relativePath,caseSensitive:l.caseSensitive,end:c},f=l.matcher&&l.compiledParams?ix(m,d,l.matcher,l.compiledParams):Qo(m,d),h=l.route;if(!f&&c&&a&&!n[n.length-1].route.index&&(f=Qo({path:l.relativePath,caseSensitive:l.caseSensitive,end:!1},d)),!f)return null;Object.assign(r,f.params),i.push({params:r,pathname:ka([s,f.pathname]),pathnameBase:q3(ka([s,f.pathnameBase])),route:h}),f.pathnameBase!=="/"&&(s=ka([s,f.pathnameBase]))}return i}function Qo(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=ox(e.path,e.caseSensitive,e.end);return ix(e,t,a,n)}function ix(e,t,a,n){let r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:m},f)=>{if(d==="*"){let x=o[f]||"";i=s.slice(0,s.length-x.length).replace(/(.)\/+$/,"$1")}let h=o[f];return m&&!h?c[d]=void 0:c[d]=(h||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function ox(e,t=!1,a=!0){na(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,l,c,d)=>{if(n.push({paramName:o,isOptional:l!=null}),l){let m=d.charAt(c+i.length);return m&&m!=="/"?"/([^\\/]*)":"(?:/([^\\/]*))?"}return"/([^\\/]+)"}).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function B3(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return na(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function Ga(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}function lx(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?jr(e):e,s;return a?(a=ux(a),a.startsWith("/")?s=W0(a.substring(1),"/"):s=W0(a,t)):s=t,{pathname:s,search:I3(n),hash:H3(r)}}function W0(e,t){let a=mc(t).split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function cp(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function z3(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function bp(e){let t=z3(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function pc(e,t,a,n=!1){let r;typeof e=="string"?r=jr(e):(r={...e},Te(!r.pathname||!r.pathname.includes("?"),cp("?","pathname","search",r)),Te(!r.pathname||!r.pathname.includes("#"),cp("#","pathname","hash",r)),Te(!r.search||!r.search.includes("#"),cp("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let m=t.length-1;if(!n&&i.startsWith("..")){let f=i.split("/");for(;f[0]==="..";)f.shift(),m-=1;r.pathname=f.join("/")}o=m>=0?t[m]:"/"}let l=lx(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!l.pathname.endsWith("/")&&(c||d)&&(l.pathname+="/"),l}var ux=e=>e.replace(/[\\/]{2,}/g,"/"),ka=e=>ux(e.join("/")),mc=e=>e.replace(/\/+$/,""),q3=e=>mc(e).replace(/^\/*/,"/"),I3=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,H3=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;var cx=class{constructor(e,t,a,n=!1){this.status=e,this.statusText=t||"",this.internal=n,a instanceof Error?(this.data=a.toString(),this.error=a):this.data=a}};function dx(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}function K3(e){let t=e.map(a=>a.route.path).filter(Boolean);return ka(t)||"/"}var mx=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";function fx(e,t){let a=e;if(typeof a!="string"||!gp.test(a))return{absoluteURL:void 0,isExternal:!1,to:a};let n=a,r=!1;if(mx)try{let s=new URL(window.location.href),i=ax.test(a)?new URL(w3(a,s.protocol)):new URL(a),o=Ga(i.pathname,t);i.origin===s.origin&&o!=null?a=o+i.search+i.hash:r=!0}catch{na(!1,`<Link to="${a}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}return{absoluteURL:n,isExternal:r,to:a}}var hP=Symbol("Uninstrumented");var vP=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");var px=["POST","PUT","PATCH","DELETE"],gP=new Set(px),Q3=["GET",...px],yP=new Set(Q3);var bP=Symbol("ResetLoaderData"),V3,G3,Y3,J3;V3=new WeakMap;G3=new WeakMap;Y3=new WeakMap;J3=new WeakMap;var X3=["about:","blob:","chrome:","chrome-untrusted:","content:","data:","devtools:","file:","filesystem:","javascript:"];function W3(e){try{return X3.includes(new URL(e).protocol)}catch{return!1}}var Ur=It.createContext(null);Ur.displayName="DataRouter";var Xs=It.createContext(null);Xs.displayName="DataRouterState";var hx=It.createContext(!1);function Z3(){return It.useContext(hx)}var xp=It.createContext({isTransitioning:!1});xp.displayName="ViewTransition";var vx=It.createContext(new Map);vx.displayName="Fetchers";var eT=It.createContext(null);eT.displayName="Await";var xt=It.createContext(null);xt.displayName="Navigation";var Ws=It.createContext(null);Ws.displayName="Location";var ra=It.createContext({outlet:null,matches:[],isDataRoute:!1});ra.displayName="Route";var $p=It.createContext(null);$p.displayName="RouteError";var pp=!0,gx="REACT_ROUTER_ERROR",tT="REDIRECT",aT="ROUTE_ERROR_RESPONSE";function nT(e){if(e.startsWith(`${gx}:${tT}:{`))try{let t=JSON.parse(e.slice(28));if(typeof t=="object"&&t&&typeof t.status=="number"&&typeof t.statusText=="string"&&typeof t.location=="string"&&typeof t.reloadDocument=="boolean"&&typeof t.replace=="boolean")return t}catch{}}function rT(e){if(e.startsWith(`${gx}:${aT}:{`))try{let t=JSON.parse(e.slice(40));if(typeof t=="object"&&t&&typeof t.status=="number"&&typeof t.statusText=="string")return new cx(t.status,t.statusText,t.data)}catch{}}function yx(e,{relative:t}={}){Te(Fr(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=J.useContext(xt),{hash:r,pathname:s,search:i}=Zs(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:ka([a,s])),n.createHref({pathname:o,search:i,hash:r})}function Fr(){return J.useContext(Ws)!=null}function Ae(){return Te(Fr(),"useLocation() may be used only in the context of a <Router> component."),J.useContext(Ws).location}var bx="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function xx(e){J.useContext(xt).static||J.useLayoutEffect(e)}function he(){let{isDataRoute:e}=J.useContext(ra);return e?pT():sT()}function sT(){Te(Fr(),"useNavigate() may be used only in the context of a <Router> component.");let e=J.useContext(Ur),{basename:t,navigator:a}=J.useContext(xt),{matches:n}=J.useContext(ra),{pathname:r}=Ae(),s=JSON.stringify(bp(n)),i=J.useRef(!1);return xx(()=>{i.current=!0}),J.useCallback((l,c={})=>{if(na(i.current,bx),!i.current)return;if(typeof l=="number"){a.go(l);return}let d=pc(l,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:ka([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var $x=J.createContext(null);function ba(){return J.useContext($x)}function wx(e){let t=J.useContext(ra).outlet;return J.useMemo(()=>t&&J.createElement($x.Provider,{value:e},t),[t,e])}function st(){let{matches:e}=J.useContext(ra);return e[e.length-1]?.params??{}}function Zs(e,{relative:t}={}){let{matches:a}=J.useContext(ra),{pathname:n}=Ae(),r=JSON.stringify(bp(a));return J.useMemo(()=>pc(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function Sx(e,t){return Nx(e,t)}function Nx(e,t,a){Te(Fr(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:n}=J.useContext(xt),{matches:r}=J.useContext(ra),s=r[r.length-1],i=s?s.params:{},o=s?s.pathname:"/",l=s?s.pathnameBase:"/",c=s&&s.route;if(pp){let $=c&&c.path||"";Cx(o,!c||$.endsWith("*")||$.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${o}" (under <Route path="${$}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${$}"> to <Route path="${$==="/"?"*":`${$}/*`}">.`)}let d=Ae(),m;if(t){let $=typeof t=="string"?jr(t):t;Te(l==="/"||$.pathname?.startsWith(l),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${l}" but pathname "${$.pathname}" was given in the \`location\` prop.`),m=$}else m=d;let f=m.pathname||"/",h=f;if(l!=="/"){let $=l.replace(/^\//,"").split("/");h="/"+f.replace(/^\//,"").split("/").slice($.length).join("/")}let x=a&&a.state.matches.length?a.state.matches.map($=>Object.assign($,{route:a.manifest[$.route.id]||$.route})):yp(e,{pathname:h});pp&&(na(c||x!=null,`No routes matched location "${m.pathname}${m.search}${m.hash}" `),na(x==null||x[x.length-1].route.element!==void 0||x[x.length-1].route.Component!==void 0||x[x.length-1].route.lazy!==void 0,`Matched leaf route at location "${m.pathname}${m.search}${m.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let y=cT(x&&x.map($=>Object.assign({},$,{params:Object.assign({},i,$.params),pathname:ka([l,n.encodeLocation?n.encodeLocation($.pathname.replace(/%/g,"%25").replace(/\?/g,"%3F").replace(/#/g,"%23")).pathname:$.pathname]),pathnameBase:$.pathnameBase==="/"?l:ka([l,n.encodeLocation?n.encodeLocation($.pathnameBase.replace(/%/g,"%25").replace(/\?/g,"%3F").replace(/#/g,"%23")).pathname:$.pathnameBase])})),r,a);return t&&y?J.createElement(Ws.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",mask:void 0,...m},navigationType:"POP"}},y):y}function iT(){let e=kx(),t=dx(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return pp&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=J.createElement(J.Fragment,null,J.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),J.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",J.createElement("code",{style:s},"ErrorBoundary")," or"," ",J.createElement("code",{style:s},"errorElement")," prop on your route."))),J.createElement(J.Fragment,null,J.createElement("h2",null,"Unexpected Application Error!"),J.createElement("h3",{style:{fontStyle:"italic"}},t),a?J.createElement("pre",{style:r},a):null,i)}var oT=J.createElement(iT,null),_x=class extends J.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.onError?this.props.onError(e,t):console.error("React Router caught the following error during render",e)}render(){let e=this.state.error;if(this.context&&typeof e=="object"&&e&&"digest"in e&&typeof e.digest=="string"){let a=rT(e.digest);a&&(e=a)}let t=e!==void 0?J.createElement(ra.Provider,{value:this.props.routeContext},J.createElement($p.Provider,{value:e,children:this.props.component})):this.props.children;return this.context?J.createElement(lT,{error:e},t):t}};_x.contextType=hx;var dp=new WeakMap;function lT({children:e,error:t}){let{basename:a}=J.useContext(xt);if(typeof t=="object"&&t&&"digest"in t&&typeof t.digest=="string"){let n=nT(t.digest);if(n){let r=dp.get(t);if(r)throw r;let s=fx(n.location,a),i=s.absoluteURL||s.to;if(W3(i))throw new Error("Invalid redirect location");if(mx&&!dp.get(t))if(s.isExternal||n.reloadDocument)window.location.href=i;else{let o=Promise.resolve().then(()=>window.__reactRouterDataRouter.navigate(s.to,{replace:n.replace}));throw dp.set(t,o),o}return J.createElement("meta",{httpEquiv:"refresh",content:`0;url=${i}`})}}return e}function uT({routeContext:e,match:t,children:a}){let n=J.useContext(Ur);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),J.createElement(ra.Provider,{value:e},a)}function cT(e,t=[],a){let n=a?.state;if(e==null){if(!n)return null;if(n.errors)e=n.matches;else if(t.length===0&&!n.initialized&&n.matches.length>0)e=n.matches;else return null}let r=e,s=n?.errors;if(s!=null){let d=r.findIndex(m=>m.route.id&&s?.[m.route.id]!==void 0);Te(d>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(s).join(",")}`),r=r.slice(0,Math.min(r.length,d+1))}let i=!1,o=-1;if(a&&n){i=n.renderFallback;for(let d=0;d<r.length;d++){let m=r[d];if((m.route.HydrateFallback||m.route.hydrateFallbackElement)&&(o=d),m.route.id){let{loaderData:f,errors:h}=n,x=m.route.loader&&!f.hasOwnProperty(m.route.id)&&(!h||h[m.route.id]===void 0);if(m.route.lazy||x){a.isStatic&&(i=!0),o>=0?r=r.slice(0,o+1):r=[r[0]];break}}}}let l=a?.onError,c=n&&l?(d,m)=>{l(d,{location:n.location,params:n.matches?.[0]?.params??{},pattern:K3(n.matches),errorInfo:m})}:void 0;return r.reduceRight((d,m,f)=>{let h,x=!1,y=null,$=null;n&&(h=s&&m.route.id?s[m.route.id]:void 0,y=m.route.errorElement||oT,i&&(o<0&&f===0?(Cx("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),x=!0,$=null):o===f&&(x=!0,$=m.route.hydrateFallbackElement||null)));let g=t.concat(r.slice(0,f+1)),v=()=>{let b;return h?b=y:x?b=$:m.route.Component?b=J.createElement(m.route.Component,null):m.route.element?b=m.route.element:b=d,J.createElement(uT,{match:m,routeContext:{outlet:d,matches:g,isDataRoute:n!=null},children:b})};return n&&(m.route.ErrorBoundary||m.route.errorElement||f===0)?J.createElement(_x,{location:n.location,revalidation:n.revalidation,component:y,error:h,children:v(),routeContext:{outlet:null,matches:g,isDataRoute:!0},onError:c}):v()},null)}function wp(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function dT(e){let t=J.useContext(Ur);return Te(t,wp(e)),t}function Sp(e){let t=J.useContext(Xs);return Te(t,wp(e)),t}function mT(e){let t=J.useContext(ra);return Te(t,wp(e)),t}function Np(e){let t=mT(e),a=t.matches[t.matches.length-1];return Te(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function fT(){return Np("useRouteId")}function Rx(){let e=Sp("useNavigation");return J.useMemo(()=>{let{matches:t,historyAction:a,...n}=e.navigation;return n},[e.navigation])}function _p(){let{matches:e,loaderData:t}=Sp("useMatches");return J.useMemo(()=>e.map(a=>C3(a,t)),[e,t])}function kx(){let e=J.useContext($p),t=Sp("useRouteError"),a=Np("useRouteError");return e!==void 0?e:t.errors?.[a]}function pT(){let{router:e}=dT("useNavigate"),t=Np("useNavigate"),a=J.useRef(!1);return xx(()=>{a.current=!0}),J.useCallback(async(r,s={})=>{na(a.current,bx),a.current&&(typeof r=="number"?await e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var Z0={};function Cx(e,t,a){!t&&!Z0[e]&&(Z0[e]=!0,na(!1,a))}var hT="useOptimistic",xP=ke[hT];var $P=ke.memo(vT);function vT({routes:e,manifest:t,future:a,state:n,isStatic:r,onError:s}){return Nx(e,void 0,{manifest:t,state:n,isStatic:r,onError:s,future:a})}function it({to:e,replace:t,state:a,relative:n}){Te(Fr(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=ke.useContext(xt);na(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=ke.useContext(ra),{pathname:i}=Ae(),o=he(),l=pc(e,bp(s),i,n==="path"),c=JSON.stringify(l);return ke.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function Rp(e){return wx(e.context)}function be(e){Te(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function kp({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1,useTransitions:i}){Te(!Fr(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let o=e.replace(/^\/*/,"/"),l=ke.useMemo(()=>({basename:o,navigator:r,static:s,useTransitions:i,future:{}}),[o,r,s,i]);typeof a=="string"&&(a=jr(a));let{pathname:c="/",search:d="",hash:m="",state:f=null,key:h="default",mask:x}=a,y=ke.useMemo(()=>{let $=Ga(c,o);return $==null?null:{location:{pathname:$,search:d,hash:m,state:f,key:h,mask:x},navigationType:n}},[o,c,d,m,f,h,n,x]);return na(y!=null,`<Router basename="${o}"> is not able to match the URL "${c}${d}${m}" because it does not start with the basename, so the <Router> won't render anything.`),y==null?null:ke.createElement(xt.Provider,{value:l},ke.createElement(Ws.Provider,{children:t,value:y}))}function Cp({children:e,location:t}){return Sx(fc(e),t)}function fc(e,t=[]){let a=[];return ke.Children.forEach(e,(n,r)=>{if(!ke.isValidElement(n))return;let s=[...t,r];if(n.type===ke.Fragment){a.push.apply(a,fc(n.props.children,s));return}Te(n.type===be,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),Te(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,middleware:n.props.middleware,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=fc(n.props.children,s)),a.push(i)}),a}var cc="get",dc="application/x-www-form-urlencoded";function hc(e){return typeof HTMLElement<"u"&&e instanceof HTMLElement}function gT(e){return hc(e)&&e.tagName.toLowerCase()==="button"}function yT(e){return hc(e)&&e.tagName.toLowerCase()==="form"}function bT(e){return hc(e)&&e.tagName.toLowerCase()==="input"}function xT(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function $T(e,t){return e.button===0&&(!t||t==="_self")&&!xT(e)}var lc=null;function wT(){if(lc===null)try{new FormData(document.createElement("form"),0),lc=!1}catch{lc=!0}return lc}var ST=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function mp(e){return e!=null&&!ST.has(e)?(na(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${dc}"`),null):e}function NT(e,t){let a,n,r,s,i;if(yT(e)){let o=e.getAttribute("action");n=o?Ga(o,t):null,a=e.getAttribute("method")||cc,r=mp(e.getAttribute("enctype"))||dc,s=new FormData(e)}else if(gT(e)||bT(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let l=e.getAttribute("formaction")||o.getAttribute("action");if(n=l?Ga(l,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||cc,r=mp(e.getAttribute("formenctype"))||mp(o.getAttribute("enctype"))||dc,s=new FormData(o,e),!wT()){let{name:c,type:d,value:m}=e;if(d==="image"){let f=c?`${c}.`:"";s.append(`${f}x`,"0"),s.append(`${f}y`,"0")}else c&&s.append(c,m)}}else{if(hc(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=cc,n=null,r=dc,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var wP=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");var _T={"&":"\\u0026",">":"\\u003e","<":"\\u003c","\u2028":"\\u2028","\u2029":"\\u2029"},RT=/[&><\u2028\u2029]/g;function ex(e){return e.replace(RT,t=>_T[t])}function Tp(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var kT=Symbol("SingleFetchRedirect");function Ex(e,t,a,n){let r=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return a?r.pathname.endsWith("/")?r.pathname=`${r.pathname}_.${n}`:r.pathname=`${r.pathname}.${n}`:r.pathname==="/"?r.pathname=`_root.${n}`:t&&Ga(r.pathname,t)==="/"?r.pathname=`${mc(t)}/_root.${n}`:r.pathname=`${mc(r.pathname)}.${n}`,r}async function CT(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function ET(e){return e!=null&&typeof e.page=="string"}function TT(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function AT(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await CT(s,a);return i.links?i.links():[]}return[]}));return LT(n.flat(1).filter(TT).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function tx(e,t,a,n,r,s){let i=(l,c)=>a[c]?l.route.id!==a[c].route.id:!0,o=(l,c)=>a[c].pathname!==l.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==l.params["*"];return s==="assets"?t.filter((l,c)=>i(l,c)||o(l,c)):s==="data"?t.filter((l,c)=>{let d=n.routes[l.route.id];if(!d||!d.hasLoader)return!1;if(i(l,c)||o(l,c))return!0;if(l.route.shouldRevalidate){let m=l.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:l.params,defaultShouldRevalidate:!0});if(typeof m=="boolean")return m}return!0}):[]}function DT(e,t,{includeHydrateFallback:a}={}){return MT(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function MT(e){return[...new Set(e)]}function OT(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function LT(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!ET(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(OT(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function Ap(){let e=ce.useContext(Ur);return Tp(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function FT(){let e=ce.useContext(Xs);return Tp(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var Vo=ce.createContext(void 0);Vo.displayName="FrameworkContext";function vc(){let e=ce.useContext(Vo);return Tp(e,"You must render this element inside a <HydratedRouter> element"),e}function BT(e,t){let a=ce.useContext(Vo),[n,r]=ce.useState(!1),[s,i]=ce.useState(!1),{onFocus:o,onBlur:l,onMouseEnter:c,onMouseLeave:d,onTouchStart:m}=t,f=ce.useRef(null);ce.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let y=g=>{g.forEach(v=>{i(v.isIntersecting)})},$=new IntersectionObserver(y,{threshold:.5});return f.current&&$.observe(f.current),()=>{$.disconnect()}}},[e]),ce.useEffect(()=>{if(n){let y=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(y)}}},[n]);let h=()=>{r(!0)},x=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,f,{}]:[s,f,{onFocus:Ko(o,h),onBlur:Ko(l,x),onMouseEnter:Ko(c,h),onMouseLeave:Ko(d,x),onTouchStart:Ko(m,h)}]:[!1,f,{}]}function Ko(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function Ax({page:e,...t}){let a=Z3(),{nonce:n}=vc(),{router:r}=Ap(),s=ce.useMemo(()=>yp(r.routes,e,r.basename),[r.routes,e,r.basename]);return s?(t.nonce==null&&n&&(t={...t,nonce:n}),a?ce.createElement(qT,{page:e,matches:s,...t}):ce.createElement(IT,{page:e,matches:s,...t})):null}function zT(e){let{manifest:t,routeModules:a}=vc(),[n,r]=ce.useState([]);return ce.useEffect(()=>{let s=!1;return AT(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function qT({page:e,matches:t,...a}){let n=Ae(),{future:r}=vc(),{basename:s}=Ap(),i=ce.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let o=Ex(e,s,r.v8_trailingSlashAwareDataRequests,"rsc"),l=!1,c=[];for(let d of t)typeof d.route.shouldRevalidate=="function"?l=!0:c.push(d.route.id);return l&&c.length>0&&o.searchParams.set("_routes",c.join(",")),[o.pathname+o.search]},[s,r.v8_trailingSlashAwareDataRequests,e,n,t]);return ce.createElement(ce.Fragment,null,i.map(o=>ce.createElement("link",{key:o,rel:"prefetch",as:"fetch",href:o,...a})))}function IT({page:e,matches:t,...a}){let n=Ae(),{future:r,manifest:s,routeModules:i}=vc(),{basename:o}=Ap(),{loaderData:l,matches:c}=FT(),d=ce.useMemo(()=>tx(e,t,c,s,n,"data"),[e,t,c,s,n]),m=ce.useMemo(()=>tx(e,t,c,s,n,"assets"),[e,t,c,s,n]),f=ce.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let y=new Set,$=!1;if(t.forEach(v=>{let b=s.routes[v.route.id];!b||!b.hasLoader||(!d.some(w=>w.route.id===v.route.id)&&v.route.id in l&&i[v.route.id]?.shouldRevalidate||b.hasClientLoader?$=!0:y.add(v.route.id))}),y.size===0)return[];let g=Ex(e,o,r.v8_trailingSlashAwareDataRequests,"data");return $&&y.size>0&&g.searchParams.set("_routes",t.filter(v=>y.has(v.route.id)).map(v=>v.route.id).join(",")),[g.pathname+g.search]},[o,r.v8_trailingSlashAwareDataRequests,l,n,s,d,t,e,i]),h=ce.useMemo(()=>DT(m,s),[m,s]),x=zT(m);return ce.createElement(ce.Fragment,null,f.map(y=>ce.createElement("link",{key:y,rel:"prefetch",as:"fetch",href:y,...a})),h.map(y=>ce.createElement("link",{key:y,rel:"modulepreload",href:y,...a})),x.map(({key:y,link:$})=>ce.createElement("link",{key:y,nonce:a.nonce,...$,crossOrigin:$.crossOrigin??a.crossOrigin})))}function HT(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var KT=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{KT&&(window.__reactRouterVersion="7.18.0")}catch{}function Dp({basename:e,children:t,useTransitions:a,window:n}){let r=Z.useRef();r.current==null&&(r.current=nx({window:n,v5Compat:!0}));let s=r.current,[i,o]=Z.useState({action:s.action,location:s.location}),l=Z.useCallback(c=>{a===!1?o(c):Z.startTransition(()=>o(c))},[a]);return Z.useLayoutEffect(()=>s.listen(l),[s,l]),Z.createElement(kp,{basename:e,children:t,location:i.location,navigationType:i.action,navigator:s,useTransitions:a})}function Dx({basename:e,children:t,history:a,useTransitions:n}){let[r,s]=Z.useState({action:a.action,location:a.location}),i=Z.useCallback(o=>{n===!1?s(o):Z.startTransition(()=>s(o))},[n]);return Z.useLayoutEffect(()=>a.listen(i),[a,i]),Z.createElement(kp,{basename:e,children:t,location:r.location,navigationType:r.action,navigator:a,useTransitions:n})}Dx.displayName="unstable_HistoryRouter";var Br=Z.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,mask:o,state:l,target:c,to:d,preventScrollReset:m,viewTransition:f,defaultShouldRevalidate:h,...x},y){let{basename:$,navigator:g,useTransitions:v}=Z.useContext(xt),b=typeof d=="string"&&gp.test(d),w=fx(d,$);d=w.to;let S=yx(d,{relative:r}),E=Ae(),N=null;if(o){let ae=pc(o,[],E.mask?E.mask.pathname:"/",!0);$!=="/"&&(ae.pathname=ae.pathname==="/"?$:ka([$,ae.pathname])),N=g.createHref(ae)}let[T,L,O]=BT(n,x),P=Px(d,{replace:i,mask:o,state:l,target:c,preventScrollReset:m,relative:r,viewTransition:f,defaultShouldRevalidate:h,useTransitions:v});function k(ae){t&&t(ae),ae.defaultPrevented||P(ae)}let U=!(w.isExternal||s),Y=Z.createElement("a",{...x,...O,href:(U?N:void 0)||w.absoluteURL||S,onClick:U?k:t,ref:HT(y,L),target:c,"data-discover":!b&&a==="render"?"true":void 0});return T&&!b?Z.createElement(Z.Fragment,null,Y,Z.createElement(Ax,{page:S})):Y});Br.displayName="Link";var Ya=Z.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:l,...c},d){let m=Zs(i,{relative:c.relative}),f=Ae(),h=Z.useContext(Xs),{navigator:x,basename:y}=Z.useContext(xt),$=h!=null&&Bx(m)&&o===!0,g=x.encodeLocation?x.encodeLocation(m).pathname:m.pathname,v=f.pathname,b=h&&h.navigation&&h.navigation.location?h.navigation.location.pathname:null;a||(v=v.toLowerCase(),b=b?b.toLowerCase():null,g=g.toLowerCase()),b&&y&&(b=Ga(b,y)||b);let w=g!=="/"&&g.endsWith("/")?g.length-1:g.length,S=v===g||!r&&v.startsWith(g)&&v.charAt(w)==="/",E=b!=null&&(b===g||!r&&b.startsWith(g)&&b.charAt(g.length)==="/"),N={isActive:S,isPending:E,isTransitioning:$},T=S?t:void 0,L;typeof n=="function"?L=n(N):L=[n,S?"active":null,E?"pending":null,$?"transitioning":null].filter(Boolean).join(" ");let O=typeof s=="function"?s(N):s;return Z.createElement(Br,{...c,"aria-current":T,className:L,ref:d,style:O,to:i,viewTransition:o},typeof l=="function"?l(N):l)});Ya.displayName="NavLink";var Mx=Z.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=cc,action:o,onSubmit:l,relative:c,preventScrollReset:d,viewTransition:m,defaultShouldRevalidate:f,...h},x)=>{let{useTransitions:y}=Z.useContext(xt),$=jx(),g=Ux(o,{relative:c}),v=i.toLowerCase()==="get"?"get":"post",b=typeof o=="string"&&gp.test(o);return Z.createElement("form",{ref:x,method:v,action:g,onSubmit:n?l:S=>{if(l&&l(S),S.defaultPrevented)return;S.preventDefault();let E=S.nativeEvent.submitter,N=E?.getAttribute("formmethod")||i,T=()=>$(E||S.currentTarget,{fetcherKey:t,method:N,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:m,defaultShouldRevalidate:f});y&&a!==!1?Z.startTransition(()=>T()):T()},...h,"data-discover":!b&&e==="render"?"true":void 0})});Mx.displayName="Form";function Ox({getKey:e,storageKey:t,...a}){let n=Z.useContext(Vo),{basename:r}=Z.useContext(xt),s=Ae(),i=_p();Fx({getKey:e,storageKey:t});let o=Z.useMemo(()=>{if(!n||!e)return null;let c=vp(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let l=((c,d)=>{if(!window.history.state||!window.history.state.key){let m=Math.random().toString(32).slice(2);window.history.replaceState({key:m},"")}try{let f=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof f=="number"&&window.scrollTo(0,f)}catch(m){console.error(m),sessionStorage.removeItem(c)}}).toString();return a.nonce==null&&n?.nonce&&(a.nonce=n.nonce),Z.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${l})(${ex(JSON.stringify(t||hp))}, ${ex(JSON.stringify(o))})`}})}Ox.displayName="ScrollRestoration";function Lx(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function Mp(e){let t=Z.useContext(Ur);return Te(t,Lx(e)),t}function QT(e){let t=Z.useContext(Xs);return Te(t,Lx(e)),t}function Px(e,{target:t,replace:a,mask:n,state:r,preventScrollReset:s,relative:i,viewTransition:o,defaultShouldRevalidate:l,useTransitions:c}={}){let d=he(),m=Ae(),f=Zs(e,{relative:i});return Z.useCallback(h=>{if($T(h,t)){h.preventDefault();let x=a!==void 0?a:Js(m)===Js(f),y=()=>d(e,{replace:x,mask:n,state:r,preventScrollReset:s,relative:i,viewTransition:o,defaultShouldRevalidate:l});c?Z.startTransition(()=>y()):y()}},[m,d,f,a,n,r,t,e,s,i,o,l,c])}var VT=0,GT=()=>`__${String(++VT)}__`;function jx(){let{router:e}=Mp("useSubmit"),{basename:t}=Z.useContext(xt),a=fT(),n=e.fetch,r=e.navigate;return Z.useCallback(async(s,i={})=>{let{action:o,method:l,encType:c,formData:d,body:m}=NT(s,t);if(i.navigate===!1){let f=i.fetcherKey||GT();await n(f,a,i.action||o,{defaultShouldRevalidate:i.defaultShouldRevalidate,preventScrollReset:i.preventScrollReset,formData:d,body:m,formMethod:i.method||l,formEncType:i.encType||c,flushSync:i.flushSync})}else await r(i.action||o,{defaultShouldRevalidate:i.defaultShouldRevalidate,preventScrollReset:i.preventScrollReset,formData:d,body:m,formMethod:i.method||l,formEncType:i.encType||c,replace:i.replace,state:i.state,fromRouteId:a,flushSync:i.flushSync,viewTransition:i.viewTransition})},[n,r,t,a])}function Ux(e,{relative:t}={}){let{basename:a}=Z.useContext(xt),n=Z.useContext(ra);Te(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...Zs(e||".",{relative:t})},i=Ae();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),l=o.getAll("index");if(l.some(d=>d==="")){o.delete("index"),l.filter(m=>m).forEach(m=>o.append("index",m));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:ka([a,s.pathname])),Js(s)}var hp="react-router-scroll-positions",uc={};function vp(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:Ga(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function Fx({getKey:e,storageKey:t}={}){let{router:a}=Mp("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=QT("useScrollRestoration"),{basename:s}=Z.useContext(xt),i=Ae(),o=_p(),l=Rx();Z.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),YT(Z.useCallback(()=>{if(l.state==="idle"){let c=vp(i,o,s,e);uc[c]=window.scrollY}try{sessionStorage.setItem(t||hp,JSON.stringify(uc))}catch(c){na(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[l.state,e,s,i,o,t])),typeof document<"u"&&(Z.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||hp);c&&(uc=JSON.parse(c))}catch{}},[t]),Z.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(uc,()=>window.scrollY,e?(d,m)=>vp(d,m,s,e):void 0);return()=>c&&c()},[a,s,e]),Z.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{na(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function YT(e,t){let{capture:a}=t||{};Z.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function Bx(e,{relative:t}={}){let a=Z.useContext(xp);Te(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=Mp("useViewTransitionState"),r=Zs(e,{relative:t});if(!a.isTransitioning)return!1;let s=Ga(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=Ga(a.nextLocation.pathname,n)||a.nextLocation.pathname;return Qo(r.pathname,i)!=null||Qo(r.pathname,s)!=null}var At=new Td({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var Op="ironclaw_token",He="/api/webchat/v2",zr=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function xa(){return sessionStorage.getItem(Op)||""}function ei(e){e?sessionStorage.setItem(Op,e):sessionStorage.removeItem(Op)}function gc(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function Ix(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function qx(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function Hx({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=qx(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=qx(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function K(e,t={}){let a=xa(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await Ix(r);throw new zr(Hx({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function yc(){return K(`${He}/session`)}function bc({clientActionId:e,requestedThreadId:t,projectId:a}={}){let n={client_action_id:e||gc()};return t&&(n.requested_thread_id=t),a&&(n.project_id=a),K(`${He}/threads`,{method:"POST",body:JSON.stringify(n)})}function Kx({limit:e,cursor:t}={}){let a=new URL(`${He}/threads`,window.location.origin);return e!=null&&a.searchParams.set("limit",String(e)),t&&a.searchParams.set("cursor",t),K(a.pathname+a.search)}function Qx({threadId:e}={}){return e?K(`${He}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function Lp(e){return`${He}/threads/${encodeURIComponent(e)}/files`}function Vx({threadId:e,path:t}={}){if(!e)return Promise.reject(new Error("threadId is required"));let a=new URL(Lp(e),window.location.origin);return t&&a.searchParams.set("path",t),K(a.pathname+a.search)}function Gx({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${Lp(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),K(a.pathname+a.search)}function xc({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${Lp(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function Yx({limit:e,runLimit:t,includeCompleted:a}={}){let n=new URLSearchParams;e!=null&&n.set("limit",String(e)),t!=null&&n.set("run_limit",String(t)),a===!0&&n.set("include_completed","true");let r=n.toString();return K(`${He}/automations${r?`?${r}`:""}`)}function Jx({automationId:e}={}){return e?K(`${He}/automations/${encodeURIComponent(e)}/pause`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function Xx({automationId:e}={}){return e?K(`${He}/automations/${encodeURIComponent(e)}/resume`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function Wx({automationId:e}={}){return e?K(`${He}/automations/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("automationId is required"))}var Zx=`${He}/projects`;function JT(e){return`${Zx}/${encodeURIComponent(e)}`}function e$({limit:e}={}){let t=new URL(Zx,window.location.origin);return e!=null&&t.searchParams.set("limit",String(e)),K(t.pathname+t.search)}function t$({projectId:e}={}){return e?K(JT(e)):Promise.reject(new Error("projectId is required"))}function a$(){return K(`${He}/outbound/preferences`)}function n$(){return K(`${He}/outbound/targets`)}function r$({finalReplyTargetId:e}={}){return K(`${He}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function Pp({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:l,source:c,tail:d,follow:m}={}){let f=new URL(`${He}/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),l&&f.searchParams.set("tool_name",l),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),K(f.pathname+f.search)}function s$({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:l,source:c,tail:d,follow:m}={}){let f=new URL(`${He}/operator/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),l&&f.searchParams.set("tool_name",l),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),K(f.pathname+f.search)}function i$({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||gc(),content:t};return a.length>0&&(r.attachments=a),K(`${He}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function o$({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${He}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),K(n.pathname+n.search)}function l$({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${He}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function Ca(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new zr("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=xa(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await Ix(r);throw new zr(Hx({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function jp(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function $c(e){return jp(await Ca(e))}function u$({threadId:e,afterCursor:t}={}){let a=new URL(`${He}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=xa();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function c$({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||gc()};return a&&(r.reason=a),K(`${He}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function Up({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let l={client_action_id:i||gc(),resolution:n};return r!=null&&(l.always=r),s&&(l.credential_ref=s),K(`${He}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(l)})}function d$({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return K("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function m$(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),K(`${He}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function ti(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function f$(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function p$(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new zr("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new zr("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function h$(){let e=xa();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var wc="anon",v$=wc;function g$(e){v$=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:wc}function $t(){return v$}var y$="ironclaw:v2-thread-pins:",Fp=new Set,wn=new Set,Bp=null;function zp(){return`${y$}${$t()}`}function XT(){try{let e=window.localStorage.getItem(zp());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function WT(){try{wn.size===0?window.localStorage.removeItem(zp()):window.localStorage.setItem(zp(),JSON.stringify([...wn]))}catch{}}function b$(){let e=$t();if(e!==Bp){wn.clear();for(let t of XT())wn.add(t);Bp=e}}function x$(){return new Set(wn)}function $$(){let e=x$();for(let t of Fp)try{t(e)}catch{}}function w$(e){e&&(b$(),wn.has(e)?wn.delete(e):wn.add(e),WT(),$$())}function S$(){return b$(),x$()}function N$(e){return Fp.add(e),()=>{Fp.delete(e)}}function _$(){wn.clear(),Bp=$t();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(y$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}$$()}var ZT=0,qr={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function qp(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function R$(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":eA(t)?"text":"download"}function eA(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function Go(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function tA(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function aA(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function nA(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function k$(e,{limits:t,existing:a=[],t:n}){let r=t||qr,s=[],i=[],o=a.length,l=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!tA(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:Go(r.maxFileBytes)}));continue}if(l+c.size>r.maxTotalBytes){let y=n("chat.attachmentTotalTooLarge",{max:Go(r.maxTotalBytes)});i.includes(y)||i.push(y);continue}let d;try{d=await aA(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:m,base64:f}=nA(d,c.type),h=m||"application/octet-stream",x=qp(h);s.push({id:`staged-${ZT++}`,filename:c.name||"attachment",mimeType:h,kind:x,sizeBytes:c.size,sizeLabel:Go(c.size),dataBase64:f,previewUrl:x==="image"?d:null}),o+=1,l+=c.size}return{staged:s,errors:i}}function C$(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function E$(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}function rA(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||qp(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?l$({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?Go(n.size_bytes):"",preview_url:null,fetch_url:s}})}function A$(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let c=lA(s);if(!c)continue;let d=`tool-${c.invocationId}`;if(n.has(d))continue;n.add(d),r.push({id:d,role:"tool_activity",...c,timestamp:T$(s)||c.updatedAt||null,sequence:s.sequence,activityOrder:c.activityOrder,activityOrderSource:c.activityOrderSource,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=oA(s),l=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy");r.push({id:i,role:o,content:s.content||"",attachments:rA(s,a),timestamp:T$(s),kind:s.kind,status:l?"error":s.status,...l&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:iA(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=sA(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function sA(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function iA(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function oA(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function T$(e){return e.received_at||e.created_at||null}function lA(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:Ip(t)}var uA="gate_declined";function Ip(e){let t=e.status==="failed"||e.status==="killed",a=e.error_kind||null,n=O$(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Jo(e.title||e.capability_id)||"tool",toolStatus:M$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(D$(a)||e.output_summary||e.output_preview||e.result_ref)||null,toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null,activityOrder:n,activityOrderSource:Number.isFinite(n)?"projection":null}}function Hp(e){let t=O$(e.activity_order),a=e.error_kind||null;return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Jo(e.capability_id)||"tool",toolStatus:M$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:null,toolError:D$(a),toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null,activityOrder:t,activityOrderSource:Number.isFinite(t)?"projection":null}}function D$(e){return e||null}function Yo(e){return e==="success"||e==="error"||e==="declined"}function Jo(e){let t=typeof e=="string"?e.trim():"";if(!t)return"";let a=t.split(".");return a[a.length-1]||t}function M$(e,t=null){if(t===uA)return"declined";switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}function O$(e){let t=Number(e);return Number.isFinite(t)?t:null}var cA=50,Ja=new Map,dA=30;function Xo(e,t){for(Ja.delete(e),Ja.set(e,t);Ja.size>dA;){let a=Ja.keys().next().value;Ja.delete(a)}}function Wo(e){return`${$t()}:${e}`}function P$(){Ja.clear()}function j$(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?Ja.get(Wo(e)):null,[s,i]=p.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),o=p.default.useRef(new Set),l=p.default.useRef(e);l.current=e;let c=p.default.useCallback(async(m,f={})=>{let{preserveClientOnly:h=!1,finalReplyTimestampByRun:x=null}=f;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(o.current.has(e))return;o.current.add(e);let y=$t(),$=Wo(e);i(g=>({...g,isLoading:!0}));try{let g=await o$({threadId:e,limit:cA,cursor:m});if($t()!==y)return;let v=m?[]:a?.()||[],b=A$(g.messages||[],v,e),w=g.next_cursor||null;if(m||n?.([]),!m){let S=Ja.get($)?.messages||[],E=L$(b,S,{preserveClientOnly:h,finalReplyTimestampByRun:x});Xo($,{messages:E,nextCursor:w})}i(S=>{if(l.current!==e)return S;let E;return m?E=mA(b,S.messages):E=L$(b,S.messages,{preserveClientOnly:h,finalReplyTimestampByRun:x}),Xo($,{messages:E,nextCursor:w}),{messages:E,nextCursor:w,isLoading:!1,loadError:null}})}catch(g){if(console.error("Failed to load timeline:",g),$t()!==y)return;i(v=>l.current===e?{...v,isLoading:!1,loadError:"Failed to load conversation history."}:v)}finally{o.current.delete(e)}},[e,a,n]);p.default.useEffect(()=>{let m=e?Ja.get(Wo(e)):null;i({messages:m?.messages||[],nextCursor:m?.nextCursor||null,isLoading:!!e&&!m,loadError:null}),e&&c()},[e,c]);let d=p.default.useCallback((m,f)=>{if(!m)return;let h=Wo(m),x=g=>typeof f=="function"?f(g||[]):f;if(l.current===m){i(g=>{let v=x(g.messages||[]);return Xo(h,{messages:v,nextCursor:g.nextCursor||null}),{...g,messages:v}});return}let y=Ja.get(h)||{messages:[],nextCursor:null},$=x(y.messages||[]);Xo(h,{messages:$,nextCursor:y.nextCursor||null})},[]);return{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:c,seedThreadMessages:d,setMessages:m=>i(f=>{let h=typeof m=="function"?m(f.messages):m;return e&&Xo(Wo(e),{messages:h,nextCursor:f.nextCursor}),{...f,messages:h}})}}function mA(e,t){let a=new Set(t.map(n=>n?.id).filter(Boolean));return[...e.filter(n=>!a.has(n?.id)),...t]}function L$(e,t,a={}){let{preserveClientOnly:n=!1,finalReplyTimestampByRun:r=null}=a,s=yA(e,t,{finalReplyTimestampByRun:r}),i=new Set(s.map(l=>l?.id).filter(Boolean)),o=t.filter(l=>!l||typeof l.id!="string"||i.has(l.id)?!1:U$(l)?!0:typeof l.timelineMessageId=="string"&&i.has(`msg-${l.timelineMessageId}`)?!1:vA(l)||gA(l)?!0:n&&l.id.startsWith("err-"));return fA(s,o)}function fA(e,t){if(t.length===0)return e;let a=new Map;for(let i=0;i<e.length;i+=1){let o=Kp(e[i]);o&&a.set(o,i)}let n=new Map,r=[];for(let i of t){let o=pA(i)?Kp(i):null;if(o&&a.has(o)){let l=n.get(o)||[];l.push(i),n.set(o,l)}else r.push(i)}if(n.size===0)return[...e,...r];let s=[];for(let i=0;i<e.length;i+=1){let o=e[i];s.push(o);let l=Kp(o);l&&a.get(l)===i&&s.push(...n.get(l)||[])}return r.length>0?[...s,...r]:s}function pA(e){return U$(e)||hA(e)}function hA(e){return e?.role==="error"&&typeof e.id=="string"&&e.id.startsWith("err-")}function Kp(e){return typeof e?.turnRunId=="string"&&e.turnRunId?e.turnRunId:null}function vA(e){return e?.isOptimistic===!0&&typeof e.id=="string"&&e.id.startsWith("pending-")&&(e.role==="user"||e.role==="assistant")}function gA(e){return e?.role==="system"&&typeof e.id=="string"&&e.id.startsWith("system-rejected-")}function yA(e,t,a={}){let{finalReplyTimestampByRun:n=null}=a,r=new Map,s=new Map;for(let i of t||[])!i||!i.timestamp||(typeof i.id=="string"&&r.set(i.id,i),typeof i.timelineMessageId=="string"&&r.set(`msg-${i.timelineMessageId}`,i),Qp(i)&&typeof i.turnRunId=="string"&&s.set(i.turnRunId,i));return r.size===0&&s.size===0&&!n?e:e.map(i=>{if(!i||i.timestamp||typeof i.id!="string")return i;let o=typeof i.turnRunId=="string"?i.turnRunId:null,l=r.get(i.id)||(Qp(i)&&o?s.get(o):null),c=Qp(i)&&o?n?.[o]:null,d=l?.timestamp||c;return d?{...i,timestamp:d}:i})}function Qp(e){return e?.role==="assistant"&&e?.isFinalReply===!0}function U$(e){return e?.role==="tool_activity"||e?.role==="thinking"}var el="__new__",F$="ironclaw:v2-draft:";function ai(e){return`${F$}${$t()}:${e||el}`}function Vp(e){try{return window.localStorage.getItem(ai(e))||""}catch{return""}}function Gp(e,t){try{t?window.localStorage.setItem(ai(e),t):window.localStorage.removeItem(ai(e))}catch{}}function B$(e){Gp(e,"")}var Zo=new Map;function Yp(e){return Zo.get(ai(e))||[]}function z$(e,t){let a=ai(e);t&&t.length>0?Zo.set(a,t):Zo.delete(a)}function q$(e){Zo.delete(ai(e))}function I$(){Zo.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(F$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function bA(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function xA(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function $A(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=bA(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?xA(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),xa()?"":(ei(n),n)}function wA(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var SA={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function NA(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),SA[t]||"Could not complete sign-in. Please try again."):""}function H$(){let[e,t]=p.default.useState(()=>$A()||xa()),[a,n]=p.default.useState(()=>NA()),[r]=p.default.useState(()=>wA()),[s,i]=p.default.useState(null),[o,l]=p.default.useState(()=>!!(r&&!xa())),[c,d]=p.default.useState(()=>!!xa());p.default.useEffect(()=>{if(!r||xa()){l(!1);return}let x=!1;return p$(r).then(y=>{x||(ei(y),d(!0),t(y),i(null),n(""),l(!1),At.clear())}).catch(()=>{x||(n("Could not complete sign-in. Please try again."),l(!1))}),()=>{x=!0}},[r]),p.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let x=!1;return d(!0),yc().then(y=>{x||(i(y),d(!1))}).catch(y=>{x||(i(null),d(!1),(y?.status===401||y?.status===403)&&(ei(""),t(""),n("Your session expired. Please sign in again."),At.clear()))}),()=>{x=!0}},[e,o]),g$(s);let m=p.default.useRef(null);p.default.useEffect(()=>{let x=$t();m.current&&m.current!==wc&&m.current!==x&&(P$(),I$(),_$()),m.current=x},[s]);let f=p.default.useCallback(x=>{ei(x),d(!!x),t(x),i(null),n(""),At.clear()},[]),h=p.default.useCallback(()=>{h$().catch(()=>{}),ei(""),d(!1),t(""),i(null),n(""),At.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,rebornProjectsEnabled:!!s?.features?.reborn_projects,signIn:f,signOut:h}}var Ir="/chat",tl=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace"},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"logs",path:"/logs",labelKey:"nav.logs",hidden:!0},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var _A=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"tools",labelKey:"settings.tools",icon:"tool"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],RA=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],kA=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],Sc={settings:_A,extensions:RA,admin:kA};var K$="ironclaw:v2-theme";function CA(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(K$);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function Nc(){let[e,t]=p.default.useState(CA);p.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(K$,e)}catch{}},[e]);let a=p.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function Q$(e){return H({enabled:!!e,queryKey:["gateway-status",e],queryFn:ti,refetchInterval:3e4})}var EA="/api/webchat/v2/operator/config",_c="/api/webchat/v2/settings/tools",ni="agent.auto_approve_tools",V$="tool.",TA=new Set(["always_allow","ask_each_time","disabled"]),AA=new Set(["default","always_allow","ask_each_time","disabled"]);function G$(e){return e==="ask"?"ask_each_time":TA.has(e)?e:"ask_each_time"}function DA(e){return e==="ask"?"ask_each_time":AA.has(e)?e:"default"}function MA(e){return["default","global","override"].includes(e)?e:"default"}function Y$(e){if(!e?.key?.startsWith(V$))return null;let t=e.value||{};return{name:t.name||e.key.slice(V$.length),description:t.description||"",state:G$(t.state),default_state:G$(t.default_state),locked:!!(t.locked||e.mutable===!1),effective_source:MA(t.effective_source||e.source)}}function OA(e){let t={};for(let a of e.entries||[])a?.key===ni&&(t[ni]=!!a.value);return t}async function J$(){let e=await K(_c);return{settings:OA(e),diagnostics:e.diagnostics||[],precedence:e.precedence||[]}}async function Jp(e,t){if(e===ni){let n=await K(_c,{method:"POST",body:JSON.stringify({enabled:!!t})});return{success:!0,entry:n.entry,value:n.entry?.value}}let a=await K(`${EA}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({value:t})});return{success:!0,entry:a.entry,value:a.entry?.value}}async function X$(e){let t=e?.settings||{},a=[];return Object.prototype.hasOwnProperty.call(t,ni)&&a.push(await Jp(ni,!!t[ni])),{success:!0,imported:a.length,results:a}}function Rc(){return K("/api/webchat/v2/llm/providers")}function W$(e){return K("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function Z$(e){return K(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function al(e){return K("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function ew(e){return K("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function tw(e){return K("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function aw(e){return K("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function nw(e){return K("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function rw(){return K("/api/webchat/v2/llm/codex/login",{method:"POST"})}async function sw(){let e=await K(_c);return{tools:(e.entries||[]).map(Y$).filter(Boolean),diagnostics:e.diagnostics||[]}}async function iw(e,t){let a=DA(t),n=await K(`${_c}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({state:a})});return{success:!0,tool:Y$(n.entry),entry:n.entry}}function ow(){return K("/api/webchat/v2/extensions")}function lw(){return K("/api/webchat/v2/extensions/registry")}function uw(){return K("/api/webchat/v2/skills")}function cw(e){return K(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function dw(e){return K("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function mw(e,t){return K(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function fw(e){return K(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function pw(e,t){return K(`/api/webchat/v2/skills/${encodeURIComponent(e)}/auto-activate`,{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:t})})}function hw(e){return K("/api/webchat/v2/skills/auto-activate-learned",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function vw(){return K("/api/webchat/v2/traces/credit")}function gw(e){return K(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function yw(){return Promise.resolve({users:[],todo:!0})}function bw(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function xw(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var Xp="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",Wp=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function nl(e){return Wp.find(t=>t.value===e)?.label||e}function ri(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function $w(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function kc(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function ww(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function Hr(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Xp||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?ri(e,t).trim().length>0:!0:!1}function LA(e,t,a){return e.id===a?"active":Hr(e,t)?"ready":"setup"}function Sw(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=LA(r,t,a);n[s]&&n[s].push(r)}return n}function Cc(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Xp||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!ri(e,t).trim()?"base_url":"ok"}function Zp(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===Xp&&(i.api_key=void 0),i}function Nw(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function _w(e){return/^[a-z0-9_-]+$/.test(e)}function Rw(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var PA=Object.freeze({});function si({settings:e,gatewayStatus:t,enabled:a=!0}){let n=W(),r=H({queryKey:["llm-providers"],queryFn:Rc,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=PA,l=(s.providers||[]).map(w=>({...w,name:w.description,has_api_key:w.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,m=d||"nearai",f=s.active?.model||t?.llm_model||"",h=l.filter(w=>w.builtin),x=l.filter(w=>!w.builtin),y=[...l].sort((w,S)=>w.id===d?-1:S.id===d?1:(w.name||w.id).localeCompare(S.name||S.id)),$=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=V({mutationFn:async w=>{if(!Hr(w,o)){let E=Cc(w,o);throw new Error(E==="base_url"?"base_url":"api_key")}let S=kc(w,o);if(!S)throw new Error("model");return await al({provider_id:w.id,model:S}),w},onSuccess:$}),v=V({mutationFn:async({provider:w,form:S,apiKey:E,editingProvider:N})=>{let T=!!w?.builtin,O={id:(T?w.id:S.id.trim()).trim(),name:T?w.name||w.id:S.name.trim(),adapter:T?w.adapter:S.adapter,base_url:S.baseUrl.trim()||w?.base_url||"",default_model:S.model.trim()||void 0};return E.trim()&&(O.api_key=E.trim()),(N||w)?.id===m&&O.default_model&&(O.set_active=!0,O.model=O.default_model),await W$(O),O},onSuccess:$}),b=V({mutationFn:async w=>(await Z$(w.id),w),onSuccess:$});return{providers:y,builtinProviders:h,customProviders:x,builtinOverrides:o,activeProviderId:d,selectedModel:f,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:w=>g.mutateAsync(w),saveCustomProvider:w=>v.mutateAsync(w),saveBuiltinProvider:w=>v.mutateAsync(w),deleteCustomProvider:w=>b.mutateAsync(w),testConnection:ew,listModels:tw,isBusy:g.isPending||v.isPending||b.isPending}}function kw({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}var Cw="ironclaw:v2-sidebar-open";function Ew(){return typeof window>"u"?null:window}function Tw(){try{return Ew()?.localStorage||null}catch{return null}}function Aw(e=Tw()){try{return e?.getItem(Cw)!=="false"}catch{return!0}}function Dw(e,t=Tw()){try{t?.setItem(Cw,e?"true":"false")}catch{}}function Mw(e=Ew()){try{return e?.matchMedia?.("(min-width: 768px)").matches===!0}catch{return!1}}function Ow(e,t){return t?{...e,desktopOpen:!e.desktopOpen}:{...e,mobileOpen:!e.mobileOpen}}function Lw(e,t){return t?e.desktopOpen:e.mobileOpen}function Pw({onNewChat:e}={}){let t=he(),[a,n]=p.default.useState(()=>({mobileOpen:!1,desktopOpen:Aw()})),[r,s]=p.default.useState(()=>Mw());p.default.useEffect(()=>{let d=window.matchMedia("(min-width: 768px)"),m=()=>s(d.matches);return m(),d.addEventListener?.("change",m),()=>d.removeEventListener?.("change",m)},[]),p.default.useEffect(()=>{Dw(a.desktopOpen)},[a.desktopOpen]);let i=p.default.useCallback(()=>{n(d=>({...d,mobileOpen:!1}))},[]),o=p.default.useCallback(()=>{n(d=>Ow(d,r))},[r]),l=p.default.useCallback(async()=>{let d=await e?.(),m=typeof d=="string"&&d.length>0?d:null;t(m?`/chat/${m}`:"/chat"),i()},[t,i,e]),c=p.default.useCallback(d=>{t(`/chat/${d}`),i()},[t,i]);return{mobileOpen:a.mobileOpen,desktopOpen:a.desktopOpen,currentOpen:Lw(a,r),close:i,toggle:o,newChat:l,selectThread:c}}var eh=new Set,jA=0;function ii(e,t={}){let a={id:++jA,message:e,tone:t.tone||"info",duration:t.duration??2600};return eh.forEach(n=>n(a)),a.id}function jw(e){return eh.add(e),()=>eh.delete(e)}function UA(e){return e?.status===409&&e?.payload?.kind==="busy"}function Uw(e,t){return UA(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function Fw(){let e=H({queryKey:["threads"],queryFn:()=>Kx({})}),[t,a]=p.default.useState(null),[n,r]=p.default.useState(!1),s=p.default.useRef(new Map),i=p.default.useCallback(async c=>{let d=c||"__global__",m=s.current.get(d);if(m)return m;r(!0);let f=(async()=>{try{let h=await bc(c?{projectId:c}:void 0);At.invalidateQueries({queryKey:["threads"]});let x=h?.thread?.thread_id;return x&&a(x),x}finally{s.current.delete(d),r(s.current.size>0)}})();return s.current.set(d,f),f},[]),o=p.default.useCallback(async c=>{await Qx({threadId:c}),t===c&&a(null),At.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:p.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var Bw={attach:u`<path
    d="m21.4 11.1-9.2 9.2a6 6 0 0 1-8.5-8.5l9.2-9.2a4 4 0 0 1 5.7 5.7l-9.2 9.2a2 2 0 0 1-2.8-2.8l8.5-8.5"
  />`,bolt:u`<path d="M13 2.8 5.8 13h5.1L10 21.2 18.2 10h-5.4L13 2.8Z" />`,calendar:u`<path d="M6.5 4.5v3M17.5 4.5v3" /><path
      d="M4.5 7h15v12.5h-15V7Z"
    /><path d="M4.5 10.5h15" /><path d="M8 14h.1M12 14h.1M16 14h.1M8 17h.1M12 17h.1" />`,check:u`<path d="m5 12.5 4.3 4.3L19.2 6.7" />`,chat:u`<path d="M5 5.5h14v10H9.4L5 19.2V5.5Z" /><path
      d="M8.4 9h7.2M8.4 12.2h4.8"
    />`,close:u`<path d="m6.5 6.5 11 11M17.5 6.5l-11 11" />`,clock:u`<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17Z" /><path
      d="M12 7.5v5l3.2 2"
    />`,download:u`<path d="M12 3.8v10" /><path d="m8 10 4 4 4-4" /><path
      d="M5 17.5v2.7h14v-2.7"
    />`,file:u`<path d="M6.5 3.5h7.2L18 7.8v12.7H6.5v-17Z" /><path
      d="M13.7 3.5V8H18"
    />`,flag:u`<path d="M6.5 21V4.5" /><path d="M6.5 5h10.7l-1.4 4 1.4 4H6.5" />`,pin:u`<path d="M9 3.5h6l-1 5 3 3.5H7l3-3.5-1-5Z" /><path d="M12 15.5V21" />`,pause:u`<path d="M8.5 5.5v13" /><path d="M15.5 5.5v13" />`,play:u`<path d="M8 5.5 18.5 12 8 18.5V5.5Z" />`,folder:u`<path
    d="M3.5 7h6.2l1.9 2h8.9v9.2a2.3 2.3 0 0 1-2.3 2.3H5.8a2.3 2.3 0 0 1-2.3-2.3V7Z"
  />`,layers:u`<path d="m12 3.7 8.5 4.2-8.5 4.4-8.5-4.4L12 3.7Z" /><path
      d="m5.2 11.2 6.8 3.5 6.8-3.5"
    /><path d="m5.2 14.8 6.8 3.5 6.8-3.5" />`,list:u`<path d="M8.5 6.5h11M8.5 12h11M8.5 17.5h11" /><path
      d="M4.5 6.5h.1M4.5 12h.1M4.5 17.5h.1"
    />`,lock:u`<path d="M7.5 10V7.2a4.5 4.5 0 0 1 9 0V10" /><path
      d="M5.5 10h13v10.5h-13V10Z"
    /><path d="M12 14.4v2.3" />`,logout:u`<path d="M10 17 15 12l-5-5" /><path d="M15 12H3.5" /><path
      d="M14.5 4.5H19a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2h-4.5"
    />`,moon:u`<path
    d="M20.2 14.7A7.7 7.7 0 0 1 9.3 3.8 8.4 8.4 0 1 0 20.2 14.7Z"
  />`,plug:u`<path d="M9 3.5v5M15 3.5v5" /><path
      d="M7.5 8.5h9v3.2a4.5 4.5 0 0 1-9 0V8.5Z"
    /><path d="M12 16.2v4.3" />`,plus:u`<path d="M12 5.5v13M5.5 12h13" />`,pulse:u`<path d="M3.5 12h4l2-5.5 4.2 11 2.2-5.5h4.6" />`,send:u`<path d="M4 11.8 20 4l-4.8 16-3.2-6.8L4 11.8Z" /><path
      d="m12 13.2 4.5-4.6"
    />`,search:u`<path d="M10.8 5.2a5.6 5.6 0 1 1 0 11.2 5.6 5.6 0 0 1 0-11.2Z" /><path
      d="m15.1 15.1 4 4"
    />`,settings:u`
    <path
      d="m19.14 12.94 2.06-1.44-1.73-3-2.47 1a7.07 7.07 0 0 0-1.47-.86L15.12 6h-3.46l-.42 2.64a7.07 7.07 0 0 0-1.47.86l-2.47-1-1.73 3 2.06 1.44a7.1 7.1 0 0 0 0 1.72l-2.06 1.44 1.73 3 2.47-1a7.07 7.07 0 0 0 1.47.86l.42 2.64h3.46l.42-2.64a7.07 7.07 0 0 0 1.47-.86l2.47 1 1.73-3-2.06-1.44a7.1 7.1 0 0 0 0-1.72Z"
    />`,spark:u`<path
    d="M12 3.5 14 10l6.5 2-6.5 2-2 6.5-2-6.5-6.5-2 6.5-2 2-6.5Z"
  />`,sun:u`<path d="M12 7.6a4.4 4.4 0 1 1 0 8.8 4.4 4.4 0 0 1 0-8.8Z" /><path
      d="M12 2.8v2.2M12 19v2.2M4.9 4.9l1.6 1.6M17.5 17.5l1.6 1.6M2.8 12H5M19 12h2.2M4.9 19.1l1.6-1.6M17.5 6.5l1.6-1.6"
    />`,shield:u`<path
      d="M12 3.2 4 7.1v4.5c0 4.7 3.3 8.9 8 10.2 4.7-1.3 8-5.5 8-10.2V7.1l-8-3.9Z"
    /><path d="m9.3 12 2 2 3.8-3.8" />`,tool:u`<path
    d="M15.3 4.4a4.5 4.5 0 0 0-5.7 5.7L4.8 15a2.7 2.7 0 1 0 3.8 3.8l4.9-4.8a4.5 4.5 0 0 0 5.7-5.7l-3.3 3.3-3.2-3.2 2.6-4Z"
  />`,trash:u`<path d="M5.5 7h13" /><path d="M9.5 7V4.5h5V7" /><path
      d="M7.2 7 8 20h8l.8-13"
    /><path d="M10.5 10.5v6M13.5 10.5v6" />`,upload:u`<path d="M12 14.2v-10" /><path d="m8 8.2 4-4 4 4" /><path
      d="M5 17.5v2.7h14v-2.7"
    />`,chevron:u`<path d="m6 9 6 6 6-6" />`,more:u`<path d="M12 5.6h.01M12 12h.01M12 18.4h.01" />`,copy:u`<path d="M9 9h9a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1Z" /><path
      d="M5 15a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h9a1 1 0 0 1 1 1"
    />`,arrowDown:u`<path d="M12 5v14" /><path d="m6 13 6 6 6-6" />`,retry:u`<path d="M3.5 12a8.5 8.5 0 1 1 2.6 6.1" /><path d="M3.2 18.5v-5h5" />`};function M({name:e,className:t="",strokeWidth:a=1.7}){return u`
    <svg
      aria-hidden="true"
      className=${t}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth=${String(a)}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      ${Bw[e]||Bw.spark}
    </svg>
  `}function G(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=G(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function zw(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function FA(e){return zw(e).trim().charAt(0).toUpperCase()||"I"}function BA(){let[e,t]=p.default.useState(!1),a=p.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function qw({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=C(),s=BA(),i=zw(a),o=a?.email||a?.role||r("common.gatewaySession");return u`
    <div
      className="relative flex items-center gap-2 border-t border-[var(--v2-panel-border)] px-3 py-3"
    >
      ${s.open&&u`
        <div
          className=${G("absolute bottom-full left-3 right-3 mb-2 rounded-[10px] border p-3 shadow-lg","border-[var(--v2-panel-border)] bg-[var(--v2-surface)]")}
        >
          <div className="truncate text-sm font-medium text-[var(--v2-text-strong)]">
            ${i}
          </div>
          ${a?.email&&u`<div className="mt-1 truncate text-xs text-[var(--v2-text-muted)]">
            ${a.email}
          </div>`}
          ${a?.role&&u`<div className="mt-2 text-[11px] uppercase text-[var(--v2-text-faint)]">
            ${a.role}
          </div>`}
        </div>
      `}

      <button
        type="button"
        onClick=${s.toggle}
        className="flex min-w-0 flex-1 items-center gap-2 rounded-[8px] text-left"
        title=${i}
      >
        <div
          className="grid h-8 w-8 shrink-0 overflow-hidden rounded-full bg-[var(--v2-accent-soft)] text-[11px] font-bold text-[var(--v2-accent-text)]"
        >
          ${a?.avatar_url?u`<img
              src=${a.avatar_url}
              alt=""
              referrerPolicy="no-referrer"
              className="h-full w-full object-cover"
            />`:u`<span className="place-self-center">${FA(a)}</span>`}
        </div>
        <span className="min-w-0">
          <span className="block truncate text-[13px] font-medium text-[var(--v2-text-strong)]">
            ${i}
          </span>
          <span className="block truncate text-[11px] text-[var(--v2-text-faint)]">
            ${o}
          </span>
        </span>
      </button>
      <button
        onClick=${t}
        className="grid h-8 w-8 shrink-0 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
        title=${r(e==="dark"?"theme.light":"theme.dark")}
      >
        <${M} name=${e==="dark"?"sun":"moon"} className="h-4 w-4" />
      </button>
      <button
        onClick=${n}
        className="grid h-8 w-8 shrink-0 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
        title=${r("header.signOut")}
      >
        <${M} name="logout" className="h-4 w-4" />
      </button>
    </div>
  `}var Iw={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",logs:"list",settings:"settings",admin:"shield"},zA=tl.filter(e=>e.id!=="chat"&&!e.hidden);function qA({route:e,label:t,onNavigate:a}){return u`
    <${Ya}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>G("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${M} name=${Iw[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function IA({route:e,label:t,subRoutes:a,onNavigate:n}){let r=C(),s=Ae(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return u`
    <div className="flex flex-col">
      <${Ya}
        to=${o}
        onClick=${n}
        className=${()=>G("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${M}
          name=${Iw[e.id]||"bolt"}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1 truncate">${t}</span>
        <${M}
          name="chevron"
          className=${G("h-3.5 w-3.5 shrink-0 transition-transform duration-150",i&&"rotate-180")}
        />
      <//>

      ${i&&u`
        <div className="mt-0.5 flex flex-col gap-0.5 pl-3">
          ${a.map(l=>u`
              <${Ya}
                key=${l.id}
                to=${e.path+"/"+l.id}
                onClick=${n}
                className=${({isActive:c})=>G("flex items-center gap-2.5 rounded-[8px] py-1.5 pl-7 pr-3 text-[12px] font-medium",c?"text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
              >
                <${M} name=${l.icon} className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate">${r(l.labelKey)}</span>
              <//>
            `)}
        </div>
      `}
    </div>
  `}function Hw({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=C(),s=p.default.useMemo(()=>zA.filter(i=>a||i.id!=="admin"),[a]);return u`
    <div className="flex flex-col px-3 py-2">
      <button
        data-testid="new-chat"
        onClick=${e}
        disabled=${t}
        className=${G("flex items-center gap-2.5 rounded-[10px] px-3 py-2","border border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]","bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]","hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)] disabled:opacity-50")}
      >
        <${M} name="plus" className="h-4 w-4 shrink-0" />
        <span className="text-[13px] font-medium">
          ${r(t?"chat.creating":"chat.newThread")}
        </span>
      </button>

      <nav className="mt-2 flex flex-col gap-1">
        ${s.map(i=>{let o=(Sc[i.id]||[]).filter(l=>a||!(i.id==="settings"&&["users","inference"].includes(l.id)));return o.length>0?u`
              <${IA}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:u`
            <${qA}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var Sn=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),rl=new Set([Sn.NEEDS_ATTENTION,Sn.FAILED]),th="ironclaw:v2-thread-attention",ah=new Set,oi=new Map;function HA(){try{let e=window.localStorage.getItem(th);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&rl.has(a[1])):[]}catch{return[]}}function Kw(){let e=[];for(let[t,a]of oi)rl.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(th):window.localStorage.setItem(th,JSON.stringify(e))}catch{}}for(let[e,t]of HA())oi.set(e,t);function Vw(){return new Map(oi)}function Qw(){let e=Vw();for(let t of ah)try{t(e)}catch{}}function Ec(e,t){if(!e)return;let a=oi.get(e);if(t==null){if(!oi.delete(e))return;rl.has(a)&&Kw(),Qw();return}a!==t&&(oi.set(e,t),(rl.has(t)||rl.has(a))&&Kw(),Qw())}function Gw(e){Ec(e,null)}function KA(){return Vw()}function QA(e){return ah.add(e),()=>{ah.delete(e)}}function Yw(){let[e,t]=p.default.useState(KA);return p.default.useEffect(()=>QA(t),[]),e}function Tc(e){return e.updated_at||e.created_at||null}function nh(e,t){let a=Tc(e)||"",n=Tc(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function Jw(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function Xw(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function VA(){let[e,t]=p.default.useState(S$);return p.default.useEffect(()=>N$(t),[]),e}var GA=Object.freeze({[Sn.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-transparent"},[Sn.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[Sn.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function YA(e){return e&&GA[e]||null}function JA({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=C(),o=Tc(e),l=Jw(o),c=Xw(o),d=p.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(h=>{window.alert(h?.message||"Unable to delete chat")})},[s,e.id]),m=p.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),w$(e.id)},[e.id]);return u`
    <div
      className=${G("group flex w-full items-stretch rounded-[8px] border-l-2",n?n.borderClass:t?"border-[var(--v2-accent)]":"border-transparent",t?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <button
        onClick=${()=>r(e.id)}
        className="min-w-0 flex-1 px-3 py-2 text-left"
        title=${c||void 0}
      >
        <div className="flex w-full items-center gap-1.5">
          <span className="min-w-0 flex-1 truncate text-[13px] font-medium leading-snug">
            ${e.title||`Thread ${e.id.slice(0,8)}`}
          </span>
          ${n&&u`<span
            aria-label=${n.label}
            className=${G("h-1.5 w-1.5 shrink-0 rounded-full",n.dotClass)}
          />`}
        </div>
        ${(n||l)&&u`<span
          className=${G("block truncate text-[11px]",n?n.textClass:"text-[var(--v2-text-faint)]")}
        >
          ${n?n.label:l}
        </span>`}
      </button>
      <button
        type="button"
        onClick=${m}
        title=${i(a?"common.unpin":"common.pin")}
        aria-label=${i(a?"common.unpin":"common.pin")}
        aria-pressed=${a?"true":"false"}
        className=${G("my-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] transition",a?"text-[var(--v2-accent-text)]":"opacity-0 text-[var(--v2-text-faint)] group-hover:opacity-100 focus:opacity-100","hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-accent-text)]")}
      >
        <${M} name="pin" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>
      ${s&&u`<button
        type="button"
        onClick=${d}
        title=${i("common.deleteChat")}
        aria-label=${i("common.deleteChat")}
        className=${G("my-1 mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px]","opacity-0 transition group-hover:opacity-100 focus:opacity-100","text-[var(--v2-text-faint)] hover:bg-[var(--v2-danger-soft)] hover:text-[var(--v2-danger-text)]")}
      >
        <${M} name="trash" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>`}
    </div>
  `}function Ww({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:u`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>u`
          <${JA}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${YA(n.get(o.id))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function Zw({threads:e,activeThreadId:t,rebornProjectsEnabled:a=!1,onSelect:n,onDelete:r,onNavigate:s}){let[i,o]=p.default.useState(!1),[l,c]=p.default.useState(""),d=Yw(),m=VA(),f=C(),{pinned:h,recent:x,totalMatches:y}=p.default.useMemo(()=>{let $=l.trim().toLowerCase(),g=$?e.filter(w=>(w.title||w.id||"").toLowerCase().includes($)):e,v=[],b=[];for(let w of g)m.has(w.id)?v.push(w):b.push(w);return v.sort(nh),b.sort(nh),{pinned:v,recent:b,totalMatches:v.length+b.length}},[e,l,m]);return u`
    <div className="flex min-h-0 flex-1 flex-col px-2">
      <button
        onClick=${()=>o($=>!$)}
        className="flex w-full items-center gap-1 rounded-[6px] px-2 py-1.5 hover:bg-[var(--v2-surface-muted)]"
      >
        <span
          className="flex-1 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]"
        >
          ${f("chat.conversations")}
        </span>
        <${M}
          name="chevron"
          className=${G("h-3.5 w-3.5 text-[var(--v2-text-faint)]",i?"-rotate-90":"")}
          strokeWidth=${2.2}
        />
      </button>

      ${!i&&u`
        ${e.length>0&&u`<div className="relative mb-1 mt-1 px-1">
          <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--v2-text-faint)]">
            <${M} name="search" className="h-3.5 w-3.5" />
          </span>
          <input
            type="text"
            value=${l}
            onInput=${$=>c($.currentTarget.value)}
            placeholder=${f("common.searchChats")}
            className="h-8 w-full rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] pl-8 pr-2 text-[12px] text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]"
          />
        </div>`}
        ${a&&u`<div className="mb-1 px-1">
          <${Ya}
            to="/projects"
            onClick=${s}
            className=${({isActive:$})=>G("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",$?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
          >
            <${M} name="folder" className="h-4 w-4 shrink-0" />
            <span className="min-w-0 truncate">${f("nav.projects")}</span>
          <//>
        </div>`}
        <div
          className="mt-1 flex flex-col gap-2 overflow-y-auto [scrollbar-width:thin]"
        >
          ${e.length===0&&u`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            ${f("chat.noConversations")}
          </div>`}
          ${e.length>0&&y===0&&u`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            ${f("common.noChatsMatch").replace("{query}",l)}
          </div>`}

          <${Ww}
            label=${f("common.pinned")}
            items=${h}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${m}
            onSelect=${n}
            onDelete=${r}
          />
          <${Ww}
            label=${f("common.recent")}
            items=${x}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${m}
            onSelect=${n}
            onDelete=${r}
          />
        </div>
      `}
    </div>
  `}function XA(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function e1(e){return!e||!e.enrolled?null:{final:XA(e.final_credit),accepted:e.submissions_accepted||0,submitted:e.submissions_submitted||0,heldCount:e.manual_review_hold_count||0}}function Ac(){let e=W(),t=H({queryKey:["trace-credits"],queryFn:vw,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=V({mutationFn:gw,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function t1(){let e=C(),{credits:t}=Ac(),a=e1(t);if(!a)return null;let{final:n,accepted:r,submitted:s,heldCount:i}=a;return u`
    <div className="px-3 pb-1">
      <${Br}
        to="/settings/traces"
        className="block rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2.5 transition-colors hover:border-[var(--v2-accent-soft)] hover:bg-[var(--v2-surface-muted)]"
      >
        <div className="flex items-center gap-2 text-[var(--v2-accent-text)]">
          <${M} name="layers" className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 truncate font-mono text-[11px] uppercase tracking-[0.14em]">
            ${e("settings.traceCommons")}
          </span>
        </div>
        <div className="mt-2 flex items-center justify-between gap-2">
          <span className="text-xs text-[var(--v2-text-muted)]">${e("traceCommons.finalCredit")}</span>
          <span className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${n}</span>
        </div>
        <div className="mt-0.5 text-[11px] text-[var(--v2-text-muted)]">
          ${e("traceCommons.cardAccepted",{accepted:r,submitted:s})}
        </div>
        ${i>0&&u`
          <div className="mt-1 text-[11px] font-medium text-[var(--v2-accent-text)]">
            ${e("traceCommons.cardHeld",{count:i})}
          </div>
        `}
      <//>
    </div>
  `}function a1({id:e,threadsState:t,theme:a,toggleTheme:n,profile:r,isAdmin:s,rebornProjectsEnabled:i=!1,onSignOut:o,onClose:l,onNewChat:c,onSelectThread:d,onDeleteThread:m}){return u`
    <aside
      id=${e}
      className="flex h-full w-[260px] shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
    >
      <div className="flex items-center gap-2.5 px-4 py-5">
        <${Br}
          to="/chat"
          onClick=${l}
          className="flex items-center gap-2.5 opacity-90 hover:opacity-100"
          aria-label="IronClaw"
        >
          <img src="/v2/assets/logo.jpg" alt="IronClaw" className="h-7 w-auto" />
        <//>
      </div>

      <${Hw}
        onNewChat=${c}
        isCreating=${t.isCreating}
        isAdmin=${s}
        onNavigate=${l}
      />

      <${t1} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${Zw}
          threads=${t.threads}
          activeThreadId=${t.activeThreadId}
          rebornProjectsEnabled=${i}
          onSelect=${d}
          onDelete=${m}
          onNavigate=${l}
        />
      </div>

      <${qw}
        theme=${a}
        toggleTheme=${n}
        profile=${r}
        onSignOut=${o}
      />
    </aside>
  `}var WA="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",ZA="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",n1="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",r1={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},s1={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function A({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=r1[n]??r1.md,l=r?"w-full":"";if(a==="primary")return u`
      <${s}
        style=${{background:WA,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${G(n1,o,l,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:ZA}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=s1[a]??s1.outline;return u`
    <${s}
      className=${G(n1,o,l,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function i1(e){let t=e.hostname;if(!t||t==="localhost"||e4(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function o1({report:e,teeInfo:t}){return JSON.stringify({...e,instance_attestation:t},null,2)}function e4(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}function l1(){let e=p.default.useMemo(()=>i1(window.location),[]),[t,a]=p.default.useState(null),[n,r]=p.default.useState(null),[s,i]=p.default.useState(!1),[o,l]=p.default.useState(""),[c,d]=p.default.useState(!1);p.default.useEffect(()=>{if(!e)return;let h=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:h.signal}).then(x=>{if(!x.ok)throw new Error(String(x.status));return x.json()}).then(a).catch(()=>{h.signal.aborted||a(null)}),()=>h.abort()},[e]);let m=p.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),l("");try{let h=await fetch(`${e.base}/attestation/report`);if(!h.ok)throw new Error(String(h.status));let x=await h.json();return r(x),x}catch(h){return l(h.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),f=p.default.useCallback(async()=>{let h=n||await m();return!h||!navigator.clipboard?!1:(await navigator.clipboard.writeText(o1({report:h,teeInfo:t})),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[m,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:m,copyReport:f}}var t4=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function u1(){let e=C(),t=l1(),[a,n]=p.default.useState(!1),r=p.default.useCallback(()=>{n(o=>{let l=!o;return l&&t.loadReport(),l})},[t]),s=p.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=a4({teeInfo:t.teeInfo,report:t.report,t:e});return u`
    <div className="relative">
      <button
        type="button"
        onClick=${r}
        aria-expanded=${a}
        title=${e("tee.title")}
        className=${G("grid h-8 w-8 place-items-center rounded-[8px]","border border-[color-mix(in_srgb,var(--v2-positive-text)_28%,transparent)]","bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]","hover:border-[color-mix(in_srgb,var(--v2-positive-text)_52%,transparent)]")}
      >
        <${M} name="shield" className="h-4 w-4" />
      </button>

      ${a&&u`
        <div
          className=${G("absolute right-0 top-full z-40 mt-2 w-[min(22rem,calc(100vw-2rem))]","rounded-[14px] border border-[var(--v2-panel-border)]","bg-[var(--v2-surface)] p-3 shadow-[0_18px_48px_rgba(0,0,0,0.35)]")}
        >
          <div className="flex items-center gap-2">
            <span className="grid h-8 w-8 place-items-center rounded-[10px] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]">
              <${M} name="shield" className="h-4 w-4" />
            </span>
            <div className="min-w-0">
              <div className="text-sm font-semibold text-[var(--v2-text-strong)]">
                ${e("tee.title")}
              </div>
              <div className="text-xs text-[var(--v2-text-muted)]">
                ${e("tee.verified")}
              </div>
            </div>
          </div>

          <div className="mt-3 space-y-2">
            ${i.map(o=>u`
                <div className="rounded-[10px] bg-[var(--v2-surface-soft)] px-3 py-2">
                  <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-[var(--v2-text-faint)]">
                    ${o.label}
                  </div>
                  <div className="mt-1 break-all font-mono text-[11px] text-[var(--v2-text)]">
                    ${o.value}
                  </div>
                </div>
              `)}
            ${t.reportLoading&&u`<div className="text-xs text-[var(--v2-text-muted)]">${e("tee.loading")}</div>`}
            ${t.reportError&&u`<div className="text-xs text-[var(--v2-danger-text)]">${e("tee.loadFailed")}</div>`}
          </div>

          <div className="mt-3 flex justify-end">
            <${A}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${t.reportLoading}
              onClick=${s}
            >
              <${M} name="check" className="h-4 w-4" />
              ${t.copied?e("tee.copied"):e("tee.copyReport")}
            <//>
          </div>
        </div>
      `}
    </div>
  `}function a4({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return t4.map(([r,s])=>({label:a(s),value:n4(n[r])||a("common.unknown")}))}function n4(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var r4="https://docs.ironclaw.com";function c1({threadsState:e,onToggleSidebar:t,sidebarOpen:a=!0}){let n=C(),r=Ae(),s=p.default.useMemo(()=>{for(let o of tl){let l=Sc[o.id];if(!l)continue;let c=o.path+"/";if(r.pathname.startsWith(c)){let d=r.pathname.slice(c.length).split("/")[0],m=l.find(f=>f.id===d);if(m)return{parent:n(o.labelKey),current:n(m.labelKey)}}}return null},[r.pathname,n]),i=p.default.useMemo(()=>{if(s)return null;if(r.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(c=>c.id===e.activeThreadId)?.title||n("nav.chat");let o=tl.find(l=>r.pathname.startsWith(l.path));return o?n(o.labelKey):""},[r.pathname,e.activeThreadId,e.threads,n,s]);return u`
    <header
      className=${G("flex h-14 shrink-0 items-center gap-3 px-4","border-b border-[var(--v2-panel-border)]","bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] backdrop-blur-xl")}
    >
      <button
        type="button"
        onClick=${t}
        className="grid h-8 w-8 shrink-0 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)]"
        aria-label="Toggle sidebar"
        aria-controls="gateway-sidebar"
        aria-expanded=${a?"true":"false"}
        title="Toggle sidebar"
      >
        <${M} name="list" className="h-4 w-4" />
      </button>

      ${s?u`
            <div className="flex min-w-0 items-center gap-2 text-[14px] font-semibold">
              <span className="shrink-0 text-[var(--v2-text-muted)]">
                ${s.parent}
              </span>
              <${M}
                name="chevron"
                className="h-3.5 w-3.5 shrink-0 -rotate-90 text-[var(--v2-text-muted)]"
              />
              <span className="truncate text-[var(--v2-text-strong)]">
                ${s.current}
              </span>
            </div>
          `:u`
            <span
              className="truncate text-[14px] font-semibold text-[var(--v2-text-strong)]"
            >
              ${i}
            </span>
          `}

      <div className="ml-auto flex shrink-0 items-center gap-1">
        <${u1} />
        <${Ya}
          to="/logs"
          className=${({isActive:o})=>G("inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",o&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
          title=${n("nav.logs")}
        >
          ${n("nav.logs")}
        <//>
        <a
          href=${r4}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${n("nav.docs")}
        >
          ${n("nav.docs")}
        </a>
      </div>
    </header>
  `}function d1({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=he(),i=C(),[o,l]=p.default.useState(""),[c,d]=p.default.useState(0),m=p.default.useRef(null),f=p.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(b=>({id:`thread-${b.id}`,label:b.title||`Thread ${b.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${b.id}`)}));return[...g,...v]},[a,s,n,r]),h=p.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?f.filter(v=>v.label.toLowerCase().includes(g)):f},[f,o]);p.default.useEffect(()=>{if(!e)return;l(""),d(0);let g=window.requestAnimationFrame(()=>m.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),p.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,h.length-1)))},[h.length]);let x=p.default.useCallback(g=>{g&&(t(),g.run())},[t]),y=p.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,h.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),x(h[c])):g.key==="Escape"&&(g.preventDefault(),t())},[h,c,x,t]);if(!e)return null;let $=null;return u`
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh]" role="dialog" aria-modal="true" aria-label="Command palette">
      <button type="button" aria-label="Close" onClick=${t} className="absolute inset-0 bg-black/50"></button>
      <div className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_30px_60px_-20px_rgba(0,0,0,0.8)]">
        <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-3">
          <${M} name="search" className="h-4 w-4 text-[var(--v2-text-faint)]" />
          <input
            ref=${m}
            value=${o}
            onInput=${g=>l(g.currentTarget.value)}
            onKeyDown=${y}
            placeholder=${i("command.placeholder")}
            className="h-12 w-full border-0 bg-transparent text-sm text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)]"
          />
          <kbd className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]">esc</kbd>
        </div>
        <ul className="max-h-[50vh] overflow-y-auto p-1.5">
          ${h.length===0&&u`<li className="px-3 py-6 text-center text-sm text-[var(--v2-text-faint)]">No matches</li>`}
          ${h.map((g,v)=>{let b=g.group!==$;return $=g.group,u`
              ${b&&u`<li key=${`g-${g.group}`} className="px-2 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">${g.group}</li>`}
              <li key=${g.id}>
                <button
                  type="button"
                  onMouseEnter=${()=>d(v)}
                  onClick=${()=>x(g)}
                  className=${["flex w-full items-center gap-2.5 rounded-[9px] px-2.5 py-2 text-left text-sm",v===c?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)]"].join(" ")}
                >
                  <${M} name=${g.icon} className="h-4 w-4 shrink-0" />
                  <span className="min-w-0 truncate">${g.label}</span>
                </button>
              </li>
            `})}
        </ul>
      </div>
    </div>
  `}var m1={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},s4={info:"bolt",success:"check",error:"close"};function f1(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>jw(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:u`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${e.map(a=>u`
          <div
            key=${a.id}
            role="status"
            className=${["pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",m1[a.tone]||m1.info].join(" ")}
          >
            <${M} name=${s4[a.tone]||"bolt"} className="h-4 w-4 shrink-0" />
            <span>${a.message}</span>
          </div>
        `)}
    </div>
  `}function p1({token:e,profile:t,isChecking:a=!1,isAdmin:n,rebornProjectsEnabled:r=!1,onSignOut:s}){let i=C(),{theme:o,toggleTheme:l}=Nc(),c=Q$(e),d=Fw(),m=Pw({onNewChat:()=>d.setActiveThreadId(null)}),f=c.data,h=Ae(),x=he(),y=si({settings:{},gatewayStatus:f,enabled:n}),$=n&&kw({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),g=h.pathname==="/welcome"||h.pathname.startsWith("/settings"),[v,b]=p.default.useState(!1);p.default.useEffect(()=>{let S=E=>{(E.metaKey||E.ctrlKey)&&E.key.toLowerCase()==="k"&&(E.preventDefault(),b(N=>!N))};return window.addEventListener("keydown",S),()=>window.removeEventListener("keydown",S)},[]);let w=p.default.useCallback(async S=>{let E=d.activeThreadId===S;try{await d.deleteThread(S),E&&x("/chat",{replace:!0})}catch(N){console.error("Failed to delete thread:",N),ii(Uw(N,i),{tone:"error"})}},[x,d,i]);return $&&!g?u`<${it} to="/welcome" replace />`:u`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${m.mobileOpen&&u`<button
        type="button"
        aria-label=${i("nav.close")}
        onClick=${m.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${G("fixed inset-y-0 left-0 z-50 md:relative md:z-auto",m.mobileOpen?"flex":"hidden",m.desktopOpen?"md:flex":"md:hidden")}
      >
        <${a1}
          id="gateway-sidebar"
          threadsState=${d}
          theme=${o}
          toggleTheme=${l}
          profile=${t}
          isAdmin=${n}
          rebornProjectsEnabled=${r}
          onSignOut=${s}
          onClose=${m.close}
          onNewChat=${m.newChat}
          onSelectThread=${m.selectThread}
          onDeleteThread=${w}
        />
      </div>

      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <${c1}
          threadsState=${d}
          onToggleSidebar=${m.toggle}
          sidebarOpen=${m.currentOpen}
        />
        <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
          ${c.error&&u`
            <div
              className=${G("m-4 rounded-[14px] border px-4 py-3 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >
              ${c.error.message||i("error.gatewayConnection")}
            </div>
          `}
          <${Rp}
            context=${{gatewayStatus:f,gatewayStatusQuery:c,currentUser:t,isChecking:a,isAdmin:n,threadsState:d}}
          />
        </main>
      </div>
      <${d1}
        open=${v}
        onClose=${()=>b(!1)}
        threadsState=${d}
        onNewChat=${m.newChat}
        onToggleTheme=${l}
      />
      <${f1} />
    </div>
  `}var Ht=ze(Ke(),1),ul=e=>e.type==="checkbox",Kr=e=>e instanceof Date,Dt=e=>e==null,k1=e=>typeof e=="object",Ge=e=>!Dt(e)&&!Array.isArray(e)&&k1(e)&&!Kr(e),i4=e=>Ge(e)&&e.target?ul(e.target)?e.target.checked:e.target.value:e,o4=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,l4=(e,t)=>e.has(o4(t)),u4=e=>{let t=e.constructor&&e.constructor.prototype;return Ge(t)&&t.hasOwnProperty("isPrototypeOf")},ih=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function mt(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(ih&&(e instanceof Blob||n))&&(a||Ge(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!u4(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=mt(e[r]));else return e;return t}var Pc=e=>/^\w*$/.test(e),We=e=>e===void 0,oh=e=>Array.isArray(e)?e.filter(Boolean):[],lh=e=>oh(e.replace(/["|']|\]/g,"").split(/\.|\[/)),X=(e,t,a)=>{if(!t||!Ge(e))return a;let n=(Pc(t)?[t]:lh(t)).reduce((r,s)=>Dt(r)?r:r[s],e);return We(n)||n===e?We(e[t])?a:e[t]:n},Xa=e=>typeof e=="boolean",je=(e,t,a)=>{let n=-1,r=Pc(t)?[t]:lh(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],l=a;if(n!==i){let c=e[o];l=Ge(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=l,e=e[o]}},h1={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},Ea={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},Nn={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},c4=Ht.default.createContext(null);c4.displayName="HookFormContext";var d4=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==Ea.all&&(t._proxyFormState[i]=!n||Ea.all),a&&(a[i]=!0),e[i]}});return r},m4=typeof window<"u"?Ht.default.useLayoutEffect:Ht.default.useEffect;var Wa=e=>typeof e=="string",f4=(e,t,a,n,r)=>Wa(e)?(n&&t.watch.add(e),X(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),X(a,s))):(n&&(t.watchAll=!0),a),sh=e=>Dt(e)||!k1(e);function or(e,t,a=new WeakSet){if(sh(e)||sh(t))return e===t;if(Kr(e)&&Kr(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(Kr(i)&&Kr(o)||Ge(i)&&Ge(o)||Array.isArray(i)&&Array.isArray(o)?!or(i,o,a):i!==o)return!1}}return!0}var p4=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},ol=e=>Array.isArray(e)?e:[e],v1=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},Kt=e=>Ge(e)&&!Object.keys(e).length,uh=e=>e.type==="file",Ta=e=>typeof e=="function",Mc=e=>{if(!ih)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},C1=e=>e.type==="select-multiple",ch=e=>e.type==="radio",h4=e=>ch(e)||ul(e),rh=e=>Mc(e)&&e.isConnected;function v4(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=We(e)?n++:e[t[n++]];return e}function g4(e){for(let t in e)if(e.hasOwnProperty(t)&&!We(e[t]))return!1;return!0}function Xe(e,t){let a=Array.isArray(t)?t:Pc(t)?[t]:lh(t),n=a.length===1?e:v4(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Ge(n)&&Kt(n)||Array.isArray(n)&&g4(n))&&Xe(e,a.slice(0,-1)),e}var E1=e=>{for(let t in e)if(Ta(e[t]))return!0;return!1};function Oc(e,t={}){let a=Array.isArray(e);if(Ge(e)||a)for(let n in e)Array.isArray(e[n])||Ge(e[n])&&!E1(e[n])?(t[n]=Array.isArray(e[n])?[]:{},Oc(e[n],t[n])):Dt(e[n])||(t[n]=!0);return t}function T1(e,t,a){let n=Array.isArray(e);if(Ge(e)||n)for(let r in e)Array.isArray(e[r])||Ge(e[r])&&!E1(e[r])?We(t)||sh(a[r])?a[r]=Array.isArray(e[r])?Oc(e[r],[]):{...Oc(e[r])}:T1(e[r],Dt(t)?{}:t[r],a[r]):a[r]=!or(e[r],t[r]);return a}var sl=(e,t)=>T1(e,t,Oc(t)),g1={value:!1,isValid:!1},y1={value:!0,isValid:!0},A1=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!We(e[0].attributes.value)?We(e[0].value)||e[0].value===""?y1:{value:e[0].value,isValid:!0}:y1:g1}return g1},D1=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>We(e)?e:t?e===""?NaN:e&&+e:a&&Wa(e)?new Date(e):n?n(e):e,b1={isValid:!1,value:null},M1=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,b1):b1;function x1(e){let t=e.ref;return uh(t)?t.files:ch(t)?M1(e.refs).value:C1(t)?[...t.selectedOptions].map(({value:a})=>a):ul(t)?A1(e.refs).value:D1(We(t.value)?e.ref.value:t.value,e)}var y4=(e,t,a,n)=>{let r={};for(let s of e){let i=X(t,s);i&&je(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},Lc=e=>e instanceof RegExp,il=e=>We(e)?e:Lc(e)?e.source:Ge(e)?Lc(e.value)?e.value.source:e.value:e,$1=e=>({isOnSubmit:!e||e===Ea.onSubmit,isOnBlur:e===Ea.onBlur,isOnChange:e===Ea.onChange,isOnAll:e===Ea.all,isOnTouch:e===Ea.onTouched}),w1="AsyncFunction",b4=e=>!!e&&!!e.validate&&!!(Ta(e.validate)&&e.validate.constructor.name===w1||Ge(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===w1)),x4=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),S1=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),ll=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=X(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(ll(o,t))break}else if(Ge(o)&&ll(o,t))break}}};function N1(e,t,a){let n=X(e,a);if(n||Pc(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=X(t,s),o=X(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var $4=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return Kt(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||Ea.all))},w4=(e,t,a)=>!e||!t||e===t||ol(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),S4=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,N4=(e,t)=>!oh(X(e,t)).length&&Xe(e,t),_4=(e,t,a)=>{let n=ol(X(e,a));return je(n,"root",t[a]),je(e,a,n),e},Dc=e=>Wa(e);function _1(e,t,a="validate"){if(Dc(e)||Array.isArray(e)&&e.every(Dc)||Xa(e)&&!e)return{type:a,message:Dc(e)?e:"",ref:t}}var li=e=>Ge(e)&&!Lc(e)?e:{value:e,message:""},R1=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:l,maxLength:c,minLength:d,min:m,max:f,pattern:h,validate:x,name:y,valueAsNumber:$,mount:g}=e._f,v=X(a,y);if(!g||t.has(y))return{};let b=o?o[0]:i,w=k=>{r&&b.reportValidity&&(b.setCustomValidity(Xa(k)?"":k||""),b.reportValidity())},S={},E=ch(i),N=ul(i),T=E||N,L=($||uh(i))&&We(i.value)&&We(v)||Mc(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,O=p4.bind(null,y,n,S),P=(k,U,Y,ae=Nn.maxLength,se=Nn.minLength)=>{let ie=k?U:Y;S[y]={type:k?ae:se,message:ie,ref:i,...O(k?ae:se,ie)}};if(s?!Array.isArray(v)||!v.length:l&&(!T&&(L||Dt(v))||Xa(v)&&!v||N&&!A1(o).isValid||E&&!M1(o).isValid)){let{value:k,message:U}=Dc(l)?{value:!!l,message:l}:li(l);if(k&&(S[y]={type:Nn.required,message:U,ref:b,...O(Nn.required,U)},!n))return w(U),S}if(!L&&(!Dt(m)||!Dt(f))){let k,U,Y=li(f),ae=li(m);if(!Dt(v)&&!isNaN(v)){let se=i.valueAsNumber||v&&+v;Dt(Y.value)||(k=se>Y.value),Dt(ae.value)||(U=se<ae.value)}else{let se=i.valueAsDate||new Date(v),ie=ft=>new Date(new Date().toDateString()+" "+ft),ot=i.type=="time",St=i.type=="week";Wa(Y.value)&&v&&(k=ot?ie(v)>ie(Y.value):St?v>Y.value:se>new Date(Y.value)),Wa(ae.value)&&v&&(U=ot?ie(v)<ie(ae.value):St?v<ae.value:se<new Date(ae.value))}if((k||U)&&(P(!!k,Y.message,ae.message,Nn.max,Nn.min),!n))return w(S[y].message),S}if((c||d)&&!L&&(Wa(v)||s&&Array.isArray(v))){let k=li(c),U=li(d),Y=!Dt(k.value)&&v.length>+k.value,ae=!Dt(U.value)&&v.length<+U.value;if((Y||ae)&&(P(Y,k.message,U.message),!n))return w(S[y].message),S}if(h&&!L&&Wa(v)){let{value:k,message:U}=li(h);if(Lc(k)&&!v.match(k)&&(S[y]={type:Nn.pattern,message:U,ref:i,...O(Nn.pattern,U)},!n))return w(U),S}if(x){if(Ta(x)){let k=await x(v,a),U=_1(k,b);if(U&&(S[y]={...U,...O(Nn.validate,U.message)},!n))return w(U.message),S}else if(Ge(x)){let k={};for(let U in x){if(!Kt(k)&&!n)break;let Y=_1(await x[U](v,a),b,U);Y&&(k={...Y,...O(U,Y.message)},w(Y.message),n&&(S[y]=k))}if(!Kt(k)&&(S[y]={ref:b,...k},!n))return S}}return w(!0),S},R4={mode:Ea.onSubmit,reValidateMode:Ea.onChange,shouldFocusError:!0};function k4(e={}){let t={...R4,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:Ta(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Ge(t.defaultValues)||Ge(t.values)?mt(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:mt(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},l,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},m={...d},f={array:v1(),state:v1()},h=t.criteriaMode===Ea.all,x=_=>R=>{clearTimeout(c),c=setTimeout(_,R)},y=async _=>{if(!t.disabled&&(d.isValid||m.isValid||_)){let R=t.resolver?Kt((await N()).errors):await L(n,!0);R!==a.isValid&&f.state.next({isValid:R})}},$=(_,R)=>{!t.disabled&&(d.isValidating||d.validatingFields||m.isValidating||m.validatingFields)&&((_||Array.from(o.mount)).forEach(D=>{D&&(R?je(a.validatingFields,D,R):Xe(a.validatingFields,D))}),f.state.next({validatingFields:a.validatingFields,isValidating:!Kt(a.validatingFields)}))},g=(_,R=[],D,I,F=!0,B=!0)=>{if(I&&D&&!t.disabled){if(i.action=!0,B&&Array.isArray(X(n,_))){let Q=D(X(n,_),I.argA,I.argB);F&&je(n,_,Q)}if(B&&Array.isArray(X(a.errors,_))){let Q=D(X(a.errors,_),I.argA,I.argB);F&&je(a.errors,_,Q),N4(a.errors,_)}if((d.touchedFields||m.touchedFields)&&B&&Array.isArray(X(a.touchedFields,_))){let Q=D(X(a.touchedFields,_),I.argA,I.argB);F&&je(a.touchedFields,_,Q)}(d.dirtyFields||m.dirtyFields)&&(a.dirtyFields=sl(r,s)),f.state.next({name:_,isDirty:P(_,R),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else je(s,_,R)},v=(_,R)=>{je(a.errors,_,R),f.state.next({errors:a.errors})},b=_=>{a.errors=_,f.state.next({errors:a.errors,isValid:!1})},w=(_,R,D,I)=>{let F=X(n,_);if(F){let B=X(s,_,We(D)?X(r,_):D);We(B)||I&&I.defaultChecked||R?je(s,_,R?B:x1(F._f)):Y(_,B),i.mount&&y()}},S=(_,R,D,I,F)=>{let B=!1,Q=!1,pe={name:_};if(!t.disabled){if(!D||I){(d.isDirty||m.isDirty)&&(Q=a.isDirty,a.isDirty=pe.isDirty=P(),B=Q!==pe.isDirty);let ve=or(X(r,_),R);Q=!!X(a.dirtyFields,_),ve?Xe(a.dirtyFields,_):je(a.dirtyFields,_,!0),pe.dirtyFields=a.dirtyFields,B=B||(d.dirtyFields||m.dirtyFields)&&Q!==!ve}if(D){let ve=X(a.touchedFields,_);ve||(je(a.touchedFields,_,D),pe.touchedFields=a.touchedFields,B=B||(d.touchedFields||m.touchedFields)&&ve!==D)}B&&F&&f.state.next(pe)}return B?pe:{}},E=(_,R,D,I)=>{let F=X(a.errors,_),B=(d.isValid||m.isValid)&&Xa(R)&&a.isValid!==R;if(t.delayError&&D?(l=x(()=>v(_,D)),l(t.delayError)):(clearTimeout(c),l=null,D?je(a.errors,_,D):Xe(a.errors,_)),(D?!or(F,D):F)||!Kt(I)||B){let Q={...I,...B&&Xa(R)?{isValid:R}:{},errors:a.errors,name:_};a={...a,...Q},f.state.next(Q)}},N=async _=>{$(_,!0);let R=await t.resolver(s,t.context,y4(_||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return $(_),R},T=async _=>{let{errors:R}=await N(_);if(_)for(let D of _){let I=X(R,D);I?je(a.errors,D,I):Xe(a.errors,D)}else a.errors=R;return R},L=async(_,R,D={valid:!0})=>{for(let I in _){let F=_[I];if(F){let{_f:B,...Q}=F;if(B){let pe=o.array.has(B.name),ve=F._f&&b4(F._f);ve&&d.validatingFields&&$([I],!0);let kt=await R1(F,o.disabled,s,h,t.shouldUseNativeValidation&&!R,pe);if(ve&&d.validatingFields&&$([I]),kt[B.name]&&(D.valid=!1,R))break;!R&&(X(kt,B.name)?pe?_4(a.errors,kt,B.name):je(a.errors,B.name,kt[B.name]):Xe(a.errors,B.name))}!Kt(Q)&&await L(Q,R,D)}}return D.valid},O=()=>{for(let _ of o.unMount){let R=X(n,_);R&&(R._f.refs?R._f.refs.every(D=>!rh(D)):!rh(R._f.ref))&&Da(_)}o.unMount=new Set},P=(_,R)=>!t.disabled&&(_&&R&&je(s,_,R),!or(ft(),r)),k=(_,R,D)=>f4(_,o,{...i.mount?s:We(R)?r:Wa(_)?{[_]:R}:R},D,R),U=_=>oh(X(i.mount?s:r,_,t.shouldUnregister?X(r,_,[]):[])),Y=(_,R,D={})=>{let I=X(n,_),F=R;if(I){let B=I._f;B&&(!B.disabled&&je(s,_,D1(R,B)),F=Mc(B.ref)&&Dt(R)?"":R,C1(B.ref)?[...B.ref.options].forEach(Q=>Q.selected=F.includes(Q.value)):B.refs?ul(B.ref)?B.refs.forEach(Q=>{(!Q.defaultChecked||!Q.disabled)&&(Array.isArray(F)?Q.checked=!!F.find(pe=>pe===Q.value):Q.checked=F===Q.value||!!F)}):B.refs.forEach(Q=>Q.checked=Q.value===F):uh(B.ref)?B.ref.value="":(B.ref.value=F,B.ref.type||f.state.next({name:_,values:mt(s)})))}(D.shouldDirty||D.shouldTouch)&&S(_,F,D.shouldTouch,D.shouldDirty,!0),D.shouldValidate&&St(_)},ae=(_,R,D)=>{for(let I in R){if(!R.hasOwnProperty(I))return;let F=R[I],B=_+"."+I,Q=X(n,B);(o.array.has(_)||Ge(F)||Q&&!Q._f)&&!Kr(F)?ae(B,F,D):Y(B,F,D)}},se=(_,R,D={})=>{let I=X(n,_),F=o.array.has(_),B=mt(R);je(s,_,B),F?(f.array.next({name:_,values:mt(s)}),(d.isDirty||d.dirtyFields||m.isDirty||m.dirtyFields)&&D.shouldDirty&&f.state.next({name:_,dirtyFields:sl(r,s),isDirty:P(_,B)})):I&&!I._f&&!Dt(B)?ae(_,B,D):Y(_,B,D),S1(_,o)&&f.state.next({...a,name:_}),f.state.next({name:i.mount?_:void 0,values:mt(s)})},ie=async _=>{i.mount=!0;let R=_.target,D=R.name,I=!0,F=X(n,D),B=ve=>{I=Number.isNaN(ve)||Kr(ve)&&isNaN(ve.getTime())||or(ve,X(s,D,ve))},Q=$1(t.mode),pe=$1(t.reValidateMode);if(F){let ve,kt,La=R.type?x1(F._f):i4(_),Qt=_.type===h1.BLUR||_.type===h1.FOCUS_OUT,Zr=!x4(F._f)&&!t.resolver&&!X(a.errors,D)&&!F._f.deps||S4(Qt,X(a.touchedFields,D),a.isSubmitted,pe,Q),Cn=S1(D,o,Qt);je(s,D,La),Qt?(!R||!R.readOnly)&&(F._f.onBlur&&F._f.onBlur(_),l&&l(0)):F._f.onChange&&F._f.onChange(_);let hr=S(D,La,Qt),de=!Kt(hr)||Cn;if(!Qt&&f.state.next({name:D,type:_.type,values:mt(s)}),Zr)return(d.isValid||m.isValid)&&(t.mode==="onBlur"?Qt&&y():Qt||y()),de&&f.state.next({name:D,...Cn?{}:hr});if(!Qt&&Cn&&f.state.next({...a}),t.resolver){let{errors:En}=await N([D]);if(B(La),I){let an=N1(a.errors,n,D),Vt=N1(En,n,an.name||D);ve=Vt.error,D=Vt.name,kt=Kt(En)}}else $([D],!0),ve=(await R1(F,o.disabled,s,h,t.shouldUseNativeValidation))[D],$([D]),B(La),I&&(ve?kt=!1:(d.isValid||m.isValid)&&(kt=await L(n,!0)));I&&(F._f.deps&&St(F._f.deps),E(D,kt,ve,hr))}},ot=(_,R)=>{if(X(a.errors,R)&&_.focus)return _.focus(),1},St=async(_,R={})=>{let D,I,F=ol(_);if(t.resolver){let B=await T(We(_)?_:F);D=Kt(B),I=_?!F.some(Q=>X(B,Q)):D}else _?(I=(await Promise.all(F.map(async B=>{let Q=X(n,B);return await L(Q&&Q._f?{[B]:Q}:Q)}))).every(Boolean),!(!I&&!a.isValid)&&y()):I=D=await L(n);return f.state.next({...!Wa(_)||(d.isValid||m.isValid)&&D!==a.isValid?{}:{name:_},...t.resolver||!_?{isValid:D}:{},errors:a.errors}),R.shouldFocus&&!I&&ll(n,ot,_?F:o.mount),I},ft=_=>{let R={...i.mount?s:r};return We(_)?R:Wa(_)?X(R,_):_.map(D=>X(R,D))},Ue=(_,R)=>({invalid:!!X((R||a).errors,_),isDirty:!!X((R||a).dirtyFields,_),error:X((R||a).errors,_),isValidating:!!X(a.validatingFields,_),isTouched:!!X((R||a).touchedFields,_)}),Fe=_=>{_&&ol(_).forEach(R=>Xe(a.errors,R)),f.state.next({errors:_?a.errors:{}})},Nt=(_,R,D)=>{let I=(X(n,_,{_f:{}})._f||{}).ref,F=X(a.errors,_)||{},{ref:B,message:Q,type:pe,...ve}=F;je(a.errors,_,{...ve,...R,ref:I}),f.state.next({name:_,errors:a.errors,isValid:!1}),D&&D.shouldFocus&&I&&I.focus&&I.focus()},kn=(_,R)=>Ta(_)?f.state.subscribe({next:D=>"values"in D&&_(k(void 0,R),D)}):k(_,R,!0),$a=_=>f.state.subscribe({next:R=>{w4(_.name,R.name,_.exact)&&$4(R,_.formState||d,re,_.reRenderRoot)&&_.callback({values:{...s},...a,...R,defaultValues:r})}}).unsubscribe,la=_=>(i.mount=!0,m={...m,..._.formState},$a({..._,formState:m})),Da=(_,R={})=>{for(let D of _?ol(_):o.mount)o.mount.delete(D),o.array.delete(D),R.keepValue||(Xe(n,D),Xe(s,D)),!R.keepError&&Xe(a.errors,D),!R.keepDirty&&Xe(a.dirtyFields,D),!R.keepTouched&&Xe(a.touchedFields,D),!R.keepIsValidating&&Xe(a.validatingFields,D),!t.shouldUnregister&&!R.keepDefaultValue&&Xe(r,D);f.state.next({values:mt(s)}),f.state.next({...a,...R.keepDirty?{isDirty:P()}:{}}),!R.keepIsValid&&y()},Ma=({disabled:_,name:R})=>{(Xa(_)&&i.mount||_||o.disabled.has(R))&&(_?o.disabled.add(R):o.disabled.delete(R))},Oa=(_,R={})=>{let D=X(n,_),I=Xa(R.disabled)||Xa(t.disabled);return je(n,_,{...D||{},_f:{...D&&D._f?D._f:{ref:{name:_}},name:_,mount:!0,...R}}),o.mount.add(_),D?Ma({disabled:Xa(R.disabled)?R.disabled:t.disabled,name:_}):w(_,!0,R.value),{...I?{disabled:R.disabled||t.disabled}:{},...t.progressive?{required:!!R.required,min:il(R.min),max:il(R.max),minLength:il(R.minLength),maxLength:il(R.maxLength),pattern:il(R.pattern)}:{},name:_,onChange:ie,onBlur:ie,ref:F=>{if(F){Oa(_,R),D=X(n,_);let B=We(F.value)&&F.querySelectorAll&&F.querySelectorAll("input,select,textarea")[0]||F,Q=h4(B),pe=D._f.refs||[];if(Q?pe.find(ve=>ve===B):B===D._f.ref)return;je(n,_,{_f:{...D._f,...Q?{refs:[...pe.filter(rh),B,...Array.isArray(X(r,_))?[{}]:[]],ref:{type:B.type,name:_}}:{ref:B}}}),w(_,!1,void 0,B)}else D=X(n,_,{}),D._f&&(D._f.mount=!1),(t.shouldUnregister||R.shouldUnregister)&&!(l4(o.array,_)&&i.action)&&o.unMount.add(_)}}},wa=()=>t.shouldFocusError&&ll(n,ot,o.mount),tn=_=>{Xa(_)&&(f.state.next({disabled:_}),ll(n,(R,D)=>{let I=X(n,D);I&&(R.disabled=I._f.disabled||_,Array.isArray(I._f.refs)&&I._f.refs.forEach(F=>{F.disabled=I._f.disabled||_}))},0,!1))},tt=(_,R)=>async D=>{let I;D&&(D.preventDefault&&D.preventDefault(),D.persist&&D.persist());let F=mt(s);if(f.state.next({isSubmitting:!0}),t.resolver){let{errors:B,values:Q}=await N();a.errors=B,F=mt(Q)}else await L(n);if(o.disabled.size)for(let B of o.disabled)Xe(F,B);if(Xe(a.errors,"root"),Kt(a.errors)){f.state.next({errors:{}});try{await _(F,D)}catch(B){I=B}}else R&&await R({...a.errors},D),wa(),setTimeout(wa);if(f.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:Kt(a.errors)&&!I,submitCount:a.submitCount+1,errors:a.errors}),I)throw I},_t=(_,R={})=>{X(n,_)&&(We(R.defaultValue)?se(_,mt(X(r,_))):(se(_,R.defaultValue),je(r,_,mt(R.defaultValue))),R.keepTouched||Xe(a.touchedFields,_),R.keepDirty||(Xe(a.dirtyFields,_),a.isDirty=R.defaultValue?P(_,mt(X(r,_))):P()),R.keepError||(Xe(a.errors,_),d.isValid&&y()),f.state.next({...a}))},Ot=(_,R={})=>{let D=_?mt(_):r,I=mt(D),F=Kt(_),B=F?r:I;if(R.keepDefaultValues||(r=D),!R.keepValues){if(R.keepDirtyValues){let Q=new Set([...o.mount,...Object.keys(sl(r,s))]);for(let pe of Array.from(Q))X(a.dirtyFields,pe)?je(B,pe,X(s,pe)):se(pe,X(B,pe))}else{if(ih&&We(_))for(let Q of o.mount){let pe=X(n,Q);if(pe&&pe._f){let ve=Array.isArray(pe._f.refs)?pe._f.refs[0]:pe._f.ref;if(Mc(ve)){let kt=ve.closest("form");if(kt){kt.reset();break}}}}if(R.keepFieldsRef)for(let Q of o.mount)se(Q,X(B,Q));else n={}}s=t.shouldUnregister?R.keepDefaultValues?mt(r):{}:mt(B),f.array.next({values:{...B}}),f.state.next({values:{...B}})}o={mount:R.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!R.keepIsValid||!!R.keepDirtyValues,i.watch=!!t.shouldUnregister,f.state.next({submitCount:R.keepSubmitCount?a.submitCount:0,isDirty:F?!1:R.keepDirty?a.isDirty:!!(R.keepDefaultValues&&!or(_,r)),isSubmitted:R.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:F?{}:R.keepDirtyValues?R.keepDefaultValues&&s?sl(r,s):a.dirtyFields:R.keepDefaultValues&&_?sl(r,_):R.keepDirty?a.dirtyFields:{},touchedFields:R.keepTouched?a.touchedFields:{},errors:R.keepErrors?a.errors:{},isSubmitSuccessful:R.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},Rt=(_,R)=>Ot(Ta(_)?_(s):_,R),ee=(_,R={})=>{let D=X(n,_),I=D&&D._f;if(I){let F=I.refs?I.refs[0]:I.ref;F.focus&&(F.focus(),R.shouldSelect&&Ta(F.select)&&F.select())}},re=_=>{a={...a,..._}},Ce={control:{register:Oa,unregister:Da,getFieldState:Ue,handleSubmit:tt,setError:Nt,_subscribe:$a,_runSchema:N,_focusError:wa,_getWatch:k,_getDirty:P,_setValid:y,_setFieldArray:g,_setDisabledField:Ma,_setErrors:b,_getFieldArray:U,_reset:Ot,_resetDefaultValues:()=>Ta(t.defaultValues)&&t.defaultValues().then(_=>{Rt(_,t.resetOptions),f.state.next({isLoading:!1})}),_removeUnmounted:O,_disableForm:tn,_subjects:f,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(_){i=_},get _defaultValues(){return r},get _names(){return o},set _names(_){o=_},get _formState(){return a},get _options(){return t},set _options(_){t={...t,..._}}},subscribe:la,trigger:St,register:Oa,handleSubmit:tt,watch:kn,setValue:se,getValues:ft,reset:Rt,resetField:_t,clearErrors:Fe,unregister:Da,setError:Nt,setFocus:ee,getFieldState:Ue};return{...Ce,formControl:Ce}}function O1(e={}){let t=Ht.default.useRef(void 0),a=Ht.default.useRef(void 0),[n,r]=Ht.default.useState({isDirty:!1,isValidating:!1,isLoading:Ta(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:Ta(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!Ta(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=k4(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,m4(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),Ht.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),Ht.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),Ht.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),Ht.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),Ht.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),Ht.default.useEffect(()=>{e.values&&!or(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),Ht.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=d4(n,s),t.current}var L1={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},P1={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},C4={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function te({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return u`
    <${s}
      className=${G(L1[a]??L1.default,P1[n]??P1.md,C4[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var dh="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",jc={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function Mt({className:e="",size:t="md",error:a=!1,...n}){return u`
    <input
      className=${G(dh,jc[t]??jc.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Uc({className:e="",error:t=!1,rows:a=4,...n}){return u`
    <textarea
      rows=${a}
      className=${G(dh,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function mh({children:e,className:t="",size:a="md",error:n=!1,...r}){return u`
    <div className="relative w-full">
      <select
        className=${G(dh,jc[a]??jc.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
        ...${r}
      >
        ${e}
      </select>
      <!-- Caret arrow -->
      <span
        aria-hidden="true"
        className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-[var(--v2-text-faint)]"
      >
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none"
          stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <path d="M2.5 4.5 6 8l3.5-3.5" />
        </svg>
      </span>
    </div>
  `}function E4({children:e,className:t="",required:a=!1,...n}){return u`
    <label
      className=${G("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&u`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function _n({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return u`
    <div className=${G("flex flex-col gap-2",s)}>
      ${e&&u`<${E4} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&u`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&u`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var T4={google:"Google",github:"GitHub",apple:"Apple"};function A4(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function j1({providers:e,redirectAfter:t}){let a=C();return e.length?u`
    <div className="mt-6 space-y-3">
      <div className="flex items-center gap-3 text-[11px] uppercase text-[var(--v2-text-faint)]">
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
        <span>${a("login.oauthDivider")}</span>
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
      </div>
      <div className="grid gap-2">
        ${e.map(n=>u`
            <${A}
              key=${n}
              as="a"
              href=${A4(n,t)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <${M} name="shield" className="h-4 w-4" />
              ${a("login.oauthProvider",{provider:T4[n]||n})}
            <//>
          `)}
      </div>
    </div>
  `:null}var D4=["google","github","apple"];function U1(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>{let a=!1;return f$().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(D4.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function F1({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=C(),{theme:s,toggleTheme:i}=Nc(),o=U1(),{formState:{errors:l,isSubmitting:c},handleSubmit:d,register:m}=O1({defaultValues:{token:e||""}});return u`
    <main
      className="relative flex min-h-[100dvh] items-center justify-center bg-[var(--v2-canvas)] px-4 py-8 sm:px-6 lg:px-12"
    >
      <!-- Theme toggle -->
      <${A}
        variant="secondary"
        size="icon"
        onClick=${i}
        aria-label=${r(s==="dark"?"theme.switchToLight":"theme.switchToDark")}
        title=${r(s==="dark"?"theme.light":"theme.dark")}
        className="absolute right-4 top-4 z-10 sm:right-6 sm:top-6"
      >
        <${M} name=${s==="dark"?"sun":"moon"} className="h-4 w-4" />
      <//>

      <!-- Login form (centered) -->
      <${te}
        as="section"
        radius="lg"
        padding="md"
        className="w-full max-w-md p-6 shadow-none sm:p-8"
      >
        <div className="mb-8">
          <p className="mb-3 font-mono text-xs uppercase tracking-[0.2em] text-[var(--v2-accent-text)]">
            ${r("login.tagline")}
          </p>
          <h1
            className="text-5xl font-semibold leading-none tracking-[-0.04em] text-[var(--v2-text-strong)]"
          >
            ${r("login.console")}
          </h1>
          <p className="mt-4 text-sm leading-6 text-[var(--v2-text-muted)]">
            ${r("login.secureSub")}
          </p>
        </div>

        <form
          className="space-y-4"
          onSubmit=${d(({token:f})=>n(f))}
        >
          <${_n}
            label=${r("login.tokenLabel")}
            htmlFor="v2-token"
            error=${l.token?.message??""}
            hint=${r("login.tokenHint")}
          >
            <${Mt}
              id="v2-token"
              type="password"
              error=${!!l.token}
              ...${m("token",{required:r("login.tokenRequired"),setValueAs:f=>f.trim()})}
              placeholder=${r("login.tokenPlaceholder")}
              autocomplete="current-password"
            />
          <//>

          ${t&&u`<p
              className=${G("rounded-[10px] border px-3 py-2 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >${t}</p>`}

          <${A}
            type="submit"
            variant="primary"
            fullWidth
            disabled=${c}
          >
            ${r("login.connect")}
          <//>
        </form>

        <${j1}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var B1={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},z1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function z({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return u`
    <span
      className=${G("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",z1[n]??z1.md,B1[e]??B1.muted,r)}
    >
      ${a&&u`<span
          className=${G("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var M4=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,q1=/(bash|shell|exec|run|command|terminal|spawn|process)/,I1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function H1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return M4.test(n)?{tone:"danger",key:"tool.riskWrite"}:q1.test(n)?{tone:"warning",key:"tool.riskExec"}:I1.test(n)?{tone:"info",key:"tool.riskNetwork"}:q1.test(r)?{tone:"warning",key:"tool.riskExec"}:I1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}var Fc=480;function O4(e,t){return t&&t.length>0?t.some(a=>typeof a?.value=="string"&&a.value.length>Fc):typeof e=="string"&&e.length>Fc}function K1(e,t){return typeof e!="string"||t||e.length<=Fc?e:`${e.slice(0,Fc).trimEnd()}
...`}function Q1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=C(),{toolName:s,description:i,parameters:o,allowAlways:l,approvalDetails:c=[]}=e,[d,m]=p.default.useState(!1),[f,h]=p.default.useState(!1);p.default.useEffect(()=>{h(!1)},[e]);let x=p.default.useMemo(()=>H1(s,i,o),[s,i,o]),y=s||r("approval.thisTool"),$=O4(o,c),g=f?"max-h-72":"max-h-36",v=p.default.useCallback(()=>{d&&l?n?.():t?.()},[d,l,n,t]);return u`
    <div
      data-testid="approval-card"
      className="mx-auto max-w-lg rounded-xl border border-copper/30 bg-copper/10 p-4"
    >
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-copper/25 bg-copper/10 text-copper">
          <${M} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">${r("approval.title")}</span>
        <${z}
          tone=${x.tone}
          label=${r(x.key)}
          dot=${!1}
          size="sm"
          className="ml-auto"
        />
      </div>
      ${s&&u`<div className="mb-1 break-all font-mono text-sm font-medium text-iron-100">${s}</div>`}
      ${i&&u`<div className="mb-3 break-words text-sm text-iron-200">${i}</div>`}
      ${c.length>0?u`
            <dl className=${`mb-2 ${g} overflow-y-auto rounded-md border border-iron-800 bg-iron-950/80 text-xs`}>
              ${c.map(b=>u`
                  <div className="grid gap-1 border-b border-iron-800/70 px-3 py-2 last:border-b-0 sm:grid-cols-[7rem_1fr]">
                    <dt className="font-medium text-iron-400">${b.label}</dt>
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${K1(b.value,f)}</dd>
                  </div>
                `)}
            </dl>
          `:o&&u`<pre className=${`mb-2 ${g} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${K1(o,f)}</pre>`}

      ${$&&u`
        <${A}
          variant="ghost"
          size="sm"
          className="mb-3 px-0 text-[var(--v2-accent)] hover:bg-transparent"
          onClick=${()=>h(b=>!b)}
          type="button"
        >
          ${r(f?"approval.showCommandPreview":"approval.viewFullCommand")}
        <//>
      `}

      ${l&&u`
        <label className="mb-3 flex items-center gap-2 text-xs text-iron-200">
          <input
            type="checkbox"
            checked=${d}
            onChange=${b=>m(b.currentTarget.checked)}
            className="h-3.5 w-3.5 accent-[var(--v2-accent)]"
          />
          ${r("approval.alwaysAllowToolLabel",{tool:y})}
        </label>
      `}

      <div className="flex flex-wrap gap-2">
        <${A} variant="primary" onClick=${v}>
          ${r(d&&l?"approval.approveAndAlways":"approval.approve")}
        <//>
        <${A} variant="secondary" onClick=${()=>a?.()}>
          ${r("approval.deny")}
        <//>
      </div>
    </div>
  `}function ui({icon:e="lock",headline:t,provider:a,accountLabel:n,body:r,expiresAt:s,pillHint:i,defaultExpanded:o=!0,children:l}){let c=C(),[d,m]=p.default.useState(o),f=p.default.useId(),h=n||a||"";return u`
    <div className="mx-auto w-full max-w-lg rounded-xl border border-[rgba(76,167,230,0.34)] bg-[rgba(76,167,230,0.08)]">
      <button
        type="button"
        onClick=${()=>m(x=>!x)}
        aria-expanded=${d?"true":"false"}
        aria-controls=${f}
        className="flex w-full items-center gap-3 rounded-xl border-0 bg-transparent px-4 py-3 text-left"
      >
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-md border border-[rgba(76,167,230,0.28)] bg-[rgba(76,167,230,0.1)] text-[#8fc8f2]">
          <${M} name=${e} className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold text-white">
            ${t||c("authGate.title")}
          </span>
          ${h&&u`<span className="block truncate text-xs text-iron-300">${h}</span>`}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 text-xs font-medium text-[#8fc8f2]">
          ${i&&u`<span className="hidden sm:inline">${i}</span>`}
          <${M}
            name="chevron"
            className=${["h-4 w-4",d?"rotate-180":""].join(" ")}
          />
        </span>
      </button>

      ${d&&u`
        <div
          id=${f}
          className="border-t border-[rgba(76,167,230,0.2)] px-4 pb-4 pt-3"
        >
          ${r&&u`<div className="mb-3 text-sm text-iron-200">${r}</div>`}
          ${l}
          ${s&&u`
            <p className="mt-2 text-xs text-iron-300">
              ${c("authGate.expiresAt")}: ${new Date(s).toLocaleString()}
            </p>
          `}
        </div>
      `}
    </div>
  `}function V1({gate:e,onCancel:t}){let a=C();return u`
    <${ui}
      icon="lock"
      headline=${e?.headline||a("authGate.title")}
      body=${e?.body||""}
    >
      <form onSubmit=${n=>n.preventDefault()}>
        <div className="mb-3 text-sm text-iron-200">
          ${a("authGate.unsupportedChallenge")}
        </div>
        <div className="flex flex-wrap gap-2">
          <${A} type="button" variant="secondary" onClick=${()=>t?.()}>
            ${a("authGate.cancel")}
          <//>
        </div>
      </form>
    <//>
  `}function G1({gate:e,onCancel:t}){let a=C(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),o=p.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]);p.default.useEffect(()=>{i("")},[e.authorizationUrl,e.gateRef,e.runId]);let l=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),c=p.default.useCallback(()=>{if(!o){i(a("authGate.serviceUnavailable"));return}i(""),window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0)},[e.authorizationUrl,o]),d=n?a("authGate.reopenAuthorization",{provider:l}):a("authGate.openAuthorization",{provider:l});return u`
    <${ui}
      icon="link"
      headline=${e?.headline||a("authGate.oauthTitle")}
      provider=${e?.provider?l:""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      expiresAt=${e?.expiresAt||""}
      pillHint=${a("authGate.pillAuthorize")}
    >
      <div className="flex flex-wrap gap-2">
        <${A}
          as="a"
          href=${o?e.authorizationUrl:void 0}
          target="_blank"
          rel="noopener noreferrer"
          className="auth-oauth"
          variant="primary"
          onClick=${m=>{m.preventDefault(),c()}}
        >
          <${M} name="link" className="h-4 w-4" />
          ${d}
        <//>
        <${A}
          type="button"
          variant="secondary"
          onClick=${()=>t?.()}
        >
          ${a("authGate.cancel")}
        <//>
      </div>

      ${s&&u`
        <div
          className="mt-3 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
          role="alert"
        >
          ${s}
        </div>
      `}
      ${n&&u`
        <p className="mt-2 text-xs text-iron-300">${a("authGate.oauthWaiting")}</p>
      `}
    <//>
  `}function Y1({gate:e,onSubmit:t,onCancel:a}){let n=C(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState(!1),d=p.default.useCallback(async m=>{m.preventDefault();let f=r.trim();if(!f){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(f),s("")}catch(h){o(h?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return u`
    <${ui}
      icon="lock"
      headline=${e?.headline||n("authGate.title")}
      provider=${e?.provider||""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      pillHint=${n("authGate.pillEnterToken")}
    >
      <form onSubmit=${d}>
        <div className="mb-3">
          <${Mt}
            type="password"
            autoComplete="off"
            spellCheck=${!1}
            value=${r}
            disabled=${l}
            placeholder=${n("authGate.tokenPlaceholder")}
            aria-label=${n("authGate.tokenLabel")}
            error=${!!i}
            onInput=${m=>s(m.currentTarget.value)}
          />
          ${i&&u`
            <p className="mt-2 text-xs text-[var(--v2-danger-text)]" role="alert">
              ${i}
            </p>
          `}
        </div>
        <div className="flex flex-wrap gap-2">
          <${A} type="submit" variant="primary" disabled=${l}>
            ${n(l?"authGate.submitting":"authGate.submit")}
          <//>
          <${A}
            type="button"
            variant="secondary"
            disabled=${l}
            onClick=${()=>a?.()}
          >
            ${n("authGate.cancel")}
          <//>
        </div>
      </form>
    <//>
  `}var L4="/api/webchat/v2/extensions/pairing/redeem";function J1(e){return K(L4,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function Bc({action:e}){let t=C(),a=W(),n=V({mutationFn:({code:l})=>J1(l),onSuccess:()=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=p.default.useState(""),i=P4(e,t),o=()=>{let l=r.trim();l&&(n.mutate({code:l}),s(""))};return u`
    <div
      data-testid="slack-pairing-section"
      className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4"
    >
      <h4 className="mb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        ${i.title}
      </h4>
      <p className="mb-4 text-xs leading-5 text-iron-300">
        ${i.instructions}
      </p>

      <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          data-testid="slack-pairing-code-input"
          type="text"
          value=${r}
          onChange=${l=>s(l.target.value)}
          onKeyDown=${l=>l.key==="Enter"&&o()}
          placeholder=${i.codePlaceholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${A}
          data-testid="slack-pairing-submit"
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${o}
          disabled=${n.isPending||!r.trim()}
        >
          ${i.submitLabel}
        <//>
      </div>

      ${n.isSuccess&&u`<p data-testid="slack-pairing-success" className="text-xs text-emerald-300">
        ${n.data?.message||i.successMessage}
      </p>`}
      ${n.isError&&u`<p data-testid="slack-pairing-error" className="text-xs text-red-300">
        ${j4(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function P4(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function j4(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function U4(e,t){return e?.channel==="slack"&&e.strategy===t}function X1({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return u`
    <div className="rounded-[16px] border border-white/[0.06] bg-white/[0.02] p-3">
      <div className="mb-2 flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            Connect ${e.display_name||a}
          </div>
        </div>
        ${t&&u`
          <button
            type="button"
            aria-label="Dismiss connect action"
            onClick=${t}
            className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-iron-400 hover:bg-white/[0.04] hover:text-iron-100"
          >
            <${M} name="close" className="h-4 w-4" />
          </button>
        `}
      </div>

      ${U4(e,"inbound_proof_code")?u`<${Bc} action=${e.action} />`:u`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function F4(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):qr.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:qr.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:qr.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:qr.maxTotalBytes}:qr}function W1(){let e=xa(),t=H({enabled:!!e,queryKey:["session"],queryFn:yc,staleTime:5*6e4});return F4(t.data)}function zc({onSend:e,onCancel:t,disabled:a,sendDisabled:n=a,canCancel:r=!1,initialText:s="",resetKey:i="",draftKey:o=el,variant:l="dock",context:c={},statusText:d=""}){let m=C(),f=l==="hero",h=W1(),[x,y]=p.default.useState(()=>Vp(o)),[$,g]=p.default.useState(()=>Yp(o)),[v,b]=p.default.useState(""),[w,S]=p.default.useState(!1),[E,N]=p.default.useState(!1),[T,L]=p.default.useState(!1),O=p.default.useRef(null),P=p.default.useRef(null),k=p.default.useRef(!1),U=a||n||w;k.current=U;let Y=p.default.useRef([]),ae=p.default.useRef(Promise.resolve());p.default.useEffect(()=>{Y.current=$},[$]);let se=p.default.useRef(null),ie=p.default.useRef(null),ot=p.default.useCallback(()=>{ie.current&&(window.clearTimeout(ie.current),ie.current=null);let R=se.current;se.current=null,R&&R.scope===$t()&&Gp(R.key,R.text)},[]),St=p.default.useCallback(()=>{ie.current&&(window.clearTimeout(ie.current),ie.current=null),se.current=null},[]),ft=p.default.useCallback(()=>{let R=O.current;R&&(R.style.height="auto",R.style.height=`${Math.min(R.scrollHeight,200)}px`)},[]);p.default.useEffect(()=>{ft()},[x,ft]),p.default.useEffect(()=>(y(Vp(o)),window.requestAnimationFrame(()=>{a||O.current?.focus()}),()=>ot()),[o,a,ot]);let Ue=p.default.useRef(o);p.default.useEffect(()=>{if(Ue.current!==o){Ue.current=o,g(Yp(o)),b("");return}z$(o,$)},[o,$]),p.default.useEffect(()=>{s&&(y(s),window.requestAnimationFrame(()=>{O.current&&(O.current.focus(),O.current.setSelectionRange(s.length,s.length))}))},[s,i]);let Fe=p.default.useCallback(R=>{a||!R||R.length===0||(ae.current=ae.current.then(async()=>{let{staged:D,errors:I}=await k$(R,{limits:h,existing:Y.current,t:m});D.length>0&&g(F=>{let B=[...F,...D];return Y.current=B,B}),b(I.length>0?I.join(" "):"")}).catch(()=>{b(m("chat.attachmentStagingFailed"))}))},[a,h,m]),Nt=p.default.useCallback(R=>{g(D=>{let I=D.filter(F=>F.id!==R);return Y.current=I,I}),b("")},[]),kn=p.default.useCallback(()=>{a||P.current?.click()},[a]),$a=p.default.useCallback(R=>{let D=Array.from(R.target.files||[]);Fe(D),R.target.value=""},[Fe]),la=p.default.useCallback(async()=>{if(!(!x.trim()||k.current)){k.current=!0,S(!0);try{if(await e(x.trim(),{attachments:$})===null)return;y(""),g([]),Y.current=[],b(""),St(),B$(o),q$(o),O.current&&(O.current.style.height="auto")}catch{}finally{k.current=a||n,S(!1)}}},[x,$,e,o,St,a,n]),Da=p.default.useCallback(R=>{let D=R.target.value;y(D),se.current={key:o,text:D,scope:$t()},ie.current&&window.clearTimeout(ie.current),ie.current=window.setTimeout(ot,300)},[o,ot]),Ma=p.default.useCallback(async()=>{if(!(!r||E||!t)){N(!0);try{await t()}finally{N(!1)}}},[r,E,t]),Oa=p.default.useCallback(R=>{if(R.key==="Enter"&&!R.shiftKey){if(R.preventDefault(),O.current?.dataset?.sendDisabled==="true"||k.current)return;la()}},[la]),wa=p.default.useCallback(R=>{let D=Array.from(R.clipboardData?.files||[]);D.length>0&&(R.preventDefault(),Fe(D))},[Fe]),tn=p.default.useCallback(R=>{R.preventDefault(),L(!1);let D=Array.from(R.dataTransfer?.files||[]);D.length>0&&Fe(D)},[Fe]),tt=p.default.useCallback(R=>{R.preventDefault(),!a&&L(!0)},[a]),_t=p.default.useCallback(R=>{R.currentTarget.contains(R.relatedTarget)||L(!1)},[]),Ot=x.trim(),Rt=a||n,ee=m(f?"chat.heroPlaceholder":"chat.followUpPlaceholder"),re=h.accept.length>0?h.accept.join(","):void 0,Se=f?"w-full":"px-4 py-3 sm:px-5 lg:px-8",Ce=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",f?"min-h-[120px]":"",a?"opacity-70":""].join(" "),_=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",f?"min-h-[72px]":"min-h-[40px]"].join(" ");return u`
    <div className=${Se}>
      <div
        className=${Ce}
        onDrop=${tn}
        onDragOver=${tt}
        onDragLeave=${_t}
      >
        ${T&&u`
          <div className="pointer-events-none absolute inset-1 z-10 flex items-center justify-center rounded-[16px] border border-dashed border-[color-mix(in_srgb,var(--v2-accent)_55%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-canvas)_82%,transparent)] text-sm font-medium text-[var(--v2-accent-text)]">
            ${m("chat.attachmentDropHint")}
          </div>
        `}
        ${v&&u`
          <div
            role="alert"
            className="mb-3 flex items-start gap-2 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-2 text-xs leading-5 text-[var(--v2-danger-text)]"
          >
            <span className="min-w-0 flex-1">${v}</span>
            <button
              type="button"
              onClick=${()=>b("")}
              aria-label=${m("common.dismiss")}
              title=${m("common.dismiss")}
              className="-mr-1 -mt-0.5 shrink-0 rounded p-0.5 text-[color-mix(in_srgb,var(--v2-danger-text)_80%,transparent)] transition hover:bg-[color-mix(in_srgb,var(--v2-danger-text)_14%,transparent)] hover:text-[var(--v2-danger-text)]"
            >
              <${M} name="close" className="h-3.5 w-3.5" strokeWidth=${2} />
            </button>
          </div>
        `}

        ${$.length>0&&u`
          <div className="mb-2 flex flex-wrap gap-2 px-1">
            ${$.map(R=>u`
                <div
                  key=${R.id}
                  className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
                >
                  ${R.previewUrl?u`<img
                        src=${R.previewUrl}
                        alt=${R.filename}
                        className="h-9 w-9 shrink-0 rounded object-cover"
                      />`:u`<span
                        className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                      >
                        <${M} name="file" className="h-4 w-4" />
                      </span>`}
                  <span className="flex min-w-0 flex-col">
                    <span className="max-w-[12rem] truncate font-medium">
                      ${R.filename}
                    </span>
                    <span className="text-[10px] text-iron-400">${R.sizeLabel}</span>
                  </span>
                  <button
                    type="button"
                    onClick=${()=>Nt(R.id)}
                    aria-label=${m("chat.attachmentRemove")}
                    title=${m("chat.attachmentRemove")}
                    className="absolute right-1 top-1 grid h-5 w-5 place-items-center rounded-full text-iron-400 hover:bg-iron-700 hover:text-white"
                  >
                    <${M} name="close" className="h-3 w-3" />
                  </button>
                </div>
              `)}
          </div>
        `}

        <textarea
          ref=${O}
          data-testid="chat-composer"
          value=${x}
          onChange=${Da}
          onKeyDown=${Oa}
          onPaste=${wa}
          data-send-disabled=${Rt?"true":"false"}
          placeholder=${ee}
          aria-label=${m("chat.composerLabel")}
          rows=${1}
          disabled=${a}
          className=${_}
        />

        <input
          ref=${P}
          type="file"
          multiple
          accept=${re}
          className="hidden"
          onChange=${$a}
        />

        <div className="mt-2 flex items-center gap-2">
          ${Rt&&u`
            <span className="inline-flex items-center gap-2 text-xs text-[var(--v2-text-muted)]">
              <span className="h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
              ${d||m("chat.statusWorking")}
            </span>
          `}
          <div className="ml-auto flex items-center gap-1.5">
            <button
              type="button"
              onClick=${kn}
              disabled=${a}
              aria-label=${m("chat.attachFiles")}
              title=${m("chat.attachFiles")}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <${M} name="plus" className="h-5 w-5" />
            </button>
            ${r?u`
                <${A}
                  type="button"
                  variant="danger"
                  size="icon-sm"
                  onClick=${Ma}
                  disabled=${E}
                  aria-label=${m("common.cancel")}
                  title=${m("common.cancel")}
                  className="rounded-full"
                >
                  <${M} name="close" className="h-5 w-5" />
                <//>
              `:u`
                <${A}
                  type="button"
                  variant="primary"
                  size="icon-sm"
                  onClick=${la}
                  disabled=${Rt||w||!Ot}
                  aria-label=${m("chat.send")}
                  className="rounded-full"
                >
                  <${M} name="send" className="h-5 w-5" />
                <//>
              `}
          </div>
        </div>
      </div>
    </div>
  `}var Z1={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function e2({status:e}){let t=C();if(e==="idle"||e==="connecting"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return u`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",Z1[e]||Z1.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function t2({onSuggestion:e,onSend:t,disabled:a,sendDisabled:n,initialText:r,resetKey:s,draftKey:i,context:o,statusText:l,canCancel:c,onCancel:d}){let m=C(),f=[{icon:"tool",title:m("chat.suggestion1"),detail:m("chat.suggestion1Desc")},{icon:"shield",title:m("chat.suggestion2"),detail:m("chat.suggestion2Desc")},{icon:"plug",title:m("chat.suggestion3"),detail:m("chat.suggestion3Desc")}];return u`
    <div
      className="v2-page-entrance flex min-h-0 flex-1 flex-col items-center justify-center px-4 py-8 sm:px-8 lg:px-12"
    >
      <div className="w-full max-w-5xl text-center">
        <h2
          className="mx-auto max-w-[16ch] text-4xl font-semibold leading-[1.04] text-white sm:text-5xl lg:text-6xl"
        >
          ${m("chat.heroTitle")}
        </h2>
        <p
          className="mx-auto mt-4 max-w-[64ch] text-base leading-relaxed text-iron-300"
        >
          ${m("chat.heroDesc")}
        </p>
      </div>

      <div className="mt-9 w-full max-w-5xl">
        <${zc}
          onSend=${t}
          disabled=${a}
          sendDisabled=${n}
          initialText=${r}
          resetKey=${s}
          draftKey=${i}
          variant="hero"
          context=${o}
          statusText=${l}
          canCancel=${c}
          onCancel=${d}
        />
      </div>

      <div className="mt-8 grid w-full max-w-5xl gap-2">
        ${f.map(h=>u`
            <button
              type="button"
              key=${h.title}
              onClick=${()=>e(h.title)}
              className="v2-button group grid grid-cols-[auto_1fr_auto] items-center gap-3 border-t border-white/10 px-2 py-4 text-left hover:border-signal/35"
            >
              <span
                className="grid h-8 w-8 place-items-center rounded-full border border-white/10 bg-white/[0.035] text-iron-300 group-hover:border-signal/35 group-hover:text-signal"
              >
                <${M} name=${h.icon} className="h-4 w-4" />
              </span>
              <span className="min-w-0">
                <span className="block text-sm font-semibold text-iron-100">
                  ${h.title}
                </span>
                <span className="mt-0.5 block text-sm text-iron-300">
                  ${h.detail}
                </span>
              </span>
            </button>
          `)}
      </div>
    </div>
  `}var B4=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function a2({open:e,onClose:t}){let a=C();return e?u`
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
      aria-label=${a("shortcuts.title")}
    >
      <button
        type="button"
        aria-label=${a("shortcuts.close")}
        onClick=${t}
        className="absolute inset-0 bg-black/50"
      ></button>
      <div
        className="relative w-full max-w-md rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-5 shadow-[0_30px_60px_-20px_rgba(0,0,0,0.8)]"
      >
        <div className="mb-4 flex items-center gap-2">
          <span className="grid h-8 w-8 place-items-center rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]">
            <${M} name="bolt" className="h-4 w-4" />
          </span>
          <h2 className="text-base font-semibold text-[var(--v2-text-strong)]">
            ${a("shortcuts.title")}
          </h2>
          <button
            type="button"
            onClick=${t}
            aria-label=${a("shortcuts.close")}
            className="ml-auto grid h-7 w-7 place-items-center rounded-md text-[var(--v2-text-faint)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]"
          >
            <${M} name="close" className="h-4 w-4" />
          </button>
        </div>
        <ul className="flex flex-col gap-2">
          ${B4.map((n,r)=>u`
              <li
                key=${r}
                className="flex items-center justify-between gap-3 text-sm text-[var(--v2-text)]"
              >
                <span>${a(n.descKey)}</span>
                <span className="flex items-center gap-1">
                  ${n.keys.map((s,i)=>u`<kbd
                      key=${i}
                      className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2 py-0.5 font-mono text-[11px] text-[var(--v2-text-muted)]"
                    >${s}</kbd>`)}
                </span>
              </li>
            `)}
        </ul>
      </div>
    </div>
  `:null}function r2(e){let t=0,a=0,n=0,r=0,s=0;for(let o of e){if(o.role==="thinking"&&(t+=1),o.role==="tool_activity"){let l=n2([o]);a+=l.tools,n+=l.failed,r+=l.declined,s+=l.running}if(z4(o)){let l=n2(o.toolCalls);a+=l.tools,n+=l.failed,r+=l.declined,s+=l.running}}let i=[];return t&&i.push(`${t} reasoning`),a&&i.push(`${a} ${a===1?"tool":"tools"}`),n&&i.push(`${n} failed`),r&&i.push(`${r} declined`),!n&&!r&&s&&i.push("running"),{hasError:n>0,hasDeclined:r>0,label:`Activity${i.length?` - ${i.join(", ")}`:""}`}}function n2(e){let t=0,a=0,n=0;for(let r of e)r.toolStatus==="error"&&(t+=1),r.toolStatus==="declined"&&(a+=1),r.toolStatus==="running"&&(n+=1);return{tools:e.length,failed:t,declined:a,running:n}}function z4(e){return e.toolCalls&&e.toolCalls.length>0}var s2=!1;function q4(){s2||!window.DOMPurify||typeof window.DOMPurify.addHook=="function"&&(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),s2=!0)}function i2(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}q4();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var fh=360;function I4(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let l=s("Copy");if(l.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),l.textContent="Copied",ii("Code copied",{tone:"success"}),setTimeout(()=>l.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(l),n.appendChild(r),t.scrollHeight>fh){t.style.maxHeight=`${fh}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${fh}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function H4({content:e,className:t=""}){let a=p.default.useRef(null),n=p.default.useMemo(()=>i2(e),[e]);return p.default.useEffect(()=>{I4(a.current)},[n]),u`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var sa=p.default.memo(H4);var o2={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",declined:"bg-iron-400",error:"bg-[var(--v2-danger-text)]"},K4={success:"ok",declined:"declined",error:"err",running:"run"},Q4=2;function ci({activity:e}){return e.toolCalls&&e.toolCalls.length>0?u`<${G4} tools=${e.toolCalls} />`:u`<${Y4} activity=${e} />`}function V4(e,t){let a=0,n=0,r=0,s=0;for(let l of t){let c=String(l.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function G4({tools:e}){let t=C(),a=e.some(o=>o.toolStatus==="error"),n=e.some(o=>o.toolStatus==="error"||o.toolStatus==="declined"),[r,s]=p.default.useState(n);if(p.default.useEffect(()=>{n&&s(!0)},[n]),e.length<=Q4)return u`
      <div className="flex flex-col gap-3">
        ${e.map((o,l)=>u`<${ci}
            key=${o.id||o.callId||`${o.toolName}-${l}`}
            activity=${o}
          />`)}
      </div>
    `;let i=V4(t,e);return u`
    <div className="flex flex-col">
      <button
        type="button"
        onClick=${()=>s(o=>!o)}
        aria-expanded=${r?"true":"false"}
        className=${["v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",a?"text-[var(--v2-danger-text)]":"text-iron-400 hover:text-iron-200"].join(" ")}
      >
        <${M} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${i}</span>
        <${M}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0",r?"rotate-180":""].join(" ")}
        />
      </button>

      ${r&&u`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((o,l)=>u`<${ci}
              key=${o.id||o.callId||`${o.toolName}-${l}`}
              activity=${o}
            />`)}
        </div>
      `}
    </div>
  `}function Y4({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:l}=e,[c,d]=p.default.useState(n==="error"||n==="declined");p.default.useEffect(()=>{(n==="error"||n==="declined")&&d(!0)},[n]);let m=o2[n]||o2.running,f=i!=null,h=p.default.useId(),x=u`
    <button
      type="button"
      onClick=${()=>d(y=>!y)}
      aria-expanded=${c?"true":"false"}
      aria-controls=${h}
      className="v2-button flex w-full items-center gap-2.5 border-0 border-b border-iron-700/40 bg-transparent px-1 py-2 text-left text-sm"
    >
      <span className=${["h-2 w-2 shrink-0 rounded-full",m].join(" ")} />
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-wide text-iron-300"
        >${K4[n]||"run"}</span
      >
      <span className="shrink-0 truncate font-mono text-[13px] font-medium text-iron-100"
        >${a}</span
      >
      ${r&&u`<span className="min-w-0 truncate font-mono text-xs text-iron-400"
        >${r}</span
      >`}
      <span className="ml-auto flex shrink-0 items-center gap-2">
        ${f&&u`<span className="font-mono text-[11px] text-iron-300">${i}ms</span>`}
        <${M}
          name="chevron"
          className=${["h-3.5 w-3.5 text-iron-400",c?"rotate-180":""].join(" ")}
        />
      </span>
    </button>
  `;return u`
    <div className=${t?"":"flex gap-3"}>
      ${!t&&u`
        <div
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
        >
          <${M} name="tool" className="h-4 w-4" />
        </div>
      `}
      <div className=${t?"min-w-0 flex-1":"min-w-0 max-w-[85%] flex-1"}>
        ${x}
        ${c&&u`<${J4}
          controlsId=${h}
          toolDetail=${r}
          toolParameters=${o}
          toolResultPreview=${l}
          toolError=${s}
          toolStatus=${n}
          toolDurationMs=${f?i:null}
        />`}
      </div>
    </div>
  `}function J4({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=C(),l=p.default.useMemo(()=>{let f=[];return r&&f.push({id:s==="declined"?"declined":"error",label:o(s==="declined"?"tool.tabDeclined":"tool.tabError")}),t&&f.push({id:"details",label:o("tool.tabDetails")}),a&&f.push({id:"params",label:o("tool.tabParameters")}),n&&f.push({id:"result",label:o("tool.tabResult")}),f},[o,r,t,a,n,s]),[c,d]=p.default.useState(null),m=c&&l.some(f=>f.id===c)?c:l[0]?.id;return p.default.useEffect(()=>{r&&d(s==="declined"?"declined":"error")},[r,s]),l.length===0?u`
      <div
        id=${e}
        className="rounded-b-lg border-x border-b border-iron-700/40 bg-iron-950 px-3 py-2 font-mono text-xs text-iron-400"
      >
        ${o("tool.noDetail")}
      </div>
    `:u`
    <div
      id=${e}
      className="rounded-b-lg border-x border-b border-iron-700/40 bg-iron-950"
    >
      <div className="flex items-center gap-1 border-b border-iron-700/40 px-2 pt-1.5">
        ${l.map(f=>u`
            <button
              type="button"
              key=${f.id}
              onClick=${()=>d(f.id)}
              className=${["v2-button rounded-t-md px-2.5 py-1 font-mono text-[11px]",m===f.id?"bg-iron-900 text-iron-100":"text-iron-400 hover:text-iron-200"].join(" ")}
            >
              ${f.label}
            </button>
          `)}
        <span className="ml-auto px-1 py-1 font-mono text-[10px] text-iron-500">
          ${o(s==="error"?"tool.exitError":s==="declined"?"tool.exitDeclined":s==="running"?"tool.exitRunning":"tool.exitOk")}${i!==null?` \xB7 ${i}ms`:""}
        </span>
      </div>
      <div className="p-3 text-xs">
        ${m==="details"&&u`<div className="whitespace-pre-wrap text-iron-200">${t}</div>`}
        ${m==="params"&&u`<pre className="overflow-x-auto rounded bg-iron-900 p-2 font-mono text-iron-100">${a}</pre>`}
        ${m==="result"&&u`<${X4} text=${n} />`}
        ${(m==="error"||m==="declined")&&u`<pre
          className=${["overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono",m==="declined"?"text-iron-300":"text-[var(--v2-danger-text)]"].join(" ")}
        >${r}</pre>`}
      </div>
    </div>
  `}function X4({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return u`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(W4)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return u`
      <div className="overflow-x-auto rounded border border-iron-700/60">
        <table className="w-full border-collapse text-left font-mono text-[11px]">
          <thead>
            <tr>
              ${n.map(r=>u`<th
                  key=${r}
                  className="border-b border-iron-700/60 bg-iron-900 px-2 py-1 font-semibold text-iron-100"
                >${r}</th>`)}
            </tr>
          </thead>
          <tbody>
            ${a.map((r,s)=>u`<tr key=${s}>
                ${n.map(i=>u`<td
                    key=${i}
                    className="border-b border-iron-700/40 px-2 py-1 text-iron-200"
                  >${Z4(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?u`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:u`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function W4(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function Z4(e){return e==null?"":String(e)}function l2({activity:e}){let t=r2(e),a=a5(e),[n,r]=p.default.useState(a);return p.default.useEffect(()=>{a&&r(!0)},[a]),u`
    <div className="mr-auto flex w-full max-w-[85%] flex-col">
      <button
        type="button"
        onClick=${()=>r(s=>!s)}
        aria-expanded=${n?"true":"false"}
        className=${["v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",t.hasError?"text-[var(--v2-danger-text)]":"text-iron-400 hover:text-iron-200"].join(" ")}
      >
        <${M} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${t.label}</span>
        <${M}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0",n?"rotate-180":""].join(" ")}
        />
      </button>

      ${n&&u`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((s,i)=>u`
            <${e5}
              key=${s.id||`${s.role||"activity"}-${i}`}
              item=${s}
            />
          `)}
        </div>
      `}
    </div>
  `}function e5({item:e}){if(e.role==="thinking")return u`<${t5} content=${e.content} />`;if(e.role==="tool_activity"||ph(e)){let t=ph(e)?{id:e.id,toolCalls:e.toolCalls}:e;return u`<${ci} activity=${t} />`}return null}function t5({content:e}){return e?u`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <${M} name="spark" className="h-4 w-4" />
      </div>
      <div className="min-w-0 max-w-[85%] flex-1 border-l-2 border-white/10 pl-3 text-iron-300">
        <${sa} content=${e} className="text-[13px]" />
      </div>
    </div>
  `:null}function ph(e){return e?.toolCalls&&e.toolCalls.length>0}function a5(e){return(e||[]).some(t=>t?.role==="thinking"||t?.toolStatus==="running"||t?.toolStatus==="error"||t?.toolStatus==="declined"?!0:ph(t)?t.toolCalls.some(a=>a?.toolStatus==="running"||a?.toolStatus==="error"||a?.toolStatus==="declined"):!1)}function di(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function n5({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=p.default.useState(()=>t&&e.preview_url||null);return p.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return $c(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?u`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:u`<${M} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var u2="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",c2="px-3 py-2";function qc({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await Ca(e.fetch_url);di(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),l=u`
    <${n5} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?u`<div
      className=${`${u2} ${c2} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${l}
    </div>`:u`<div className=${`${u2} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${c2} text-left transition-colors hover:bg-iron-900/80`}
    >
      ${l}
    </button>
    ${e.fetch_url&&u`<button
      type="button"
      onClick=${o}
      disabled=${s}
      aria-label=${`Download ${e.filename||"attachment"}`}
      data-testid=${r}
      className="flex shrink-0 items-center border-l border-iron-700 px-2.5 text-iron-200 transition-colors hover:bg-iron-900/80 hover:text-white disabled:opacity-50"
    >
      <${M} name="download" className="h-3.5 w-3.5" />
    </button>`}
  </div>`}var d2={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function mi({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return p.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),p.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?u`
    <!-- Backdrop -->
    <div
      className="fixed inset-0 z-50 flex items-end justify-center p-4 sm:items-center"
      aria-modal="true"
      role="dialog"
    >
      <!-- Dim layer -->
      <div
        className="absolute inset-0 bg-black/55 backdrop-blur-sm"
        onClick=${t}
        aria-hidden="true"
      />

      <!-- Panel -->
      <div
        className=${G("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",d2[n]??d2.md,r)}
      >
        ${a?u`<${hh} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function hh({children:e,onClose:t,className:a=""}){return u`
    <div
      className=${G("flex shrink-0 items-center justify-between gap-4","px-5 py-4 md:px-7 md:py-5","border-b border-[var(--v2-panel-border)]",a)}
    >
      <h2
        className="text-[1.1rem] font-semibold tracking-[-0.02em] text-[var(--v2-text-strong)] md:text-[1.2rem]"
      >
        ${e}
      </h2>
      ${t&&u`
          <button
            type="button"
            onClick=${t}
            aria-label="Close"
            className="grid h-8 w-8 shrink-0 place-items-center rounded-[10px]
              border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]
              text-[var(--v2-text-muted)]
              hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          >
            <${M} name="close" className="h-4 w-4" />
          </button>
        `}
    </div>
  `}function fi({children:e,className:t=""}){return u`
    <div className=${G("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function pi({children:e,className:t=""}){return u`
    <div
      className=${G("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var m2=1e5;function Ic({attachment:e,onClose:t}){let a=!!e,[n,r]=p.default.useState("loading"),[s,i]=p.default.useState({}),o=e?R$(e.mime_type):"download";if(p.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return Ca(e.fetch_url).then(async m=>{d=URL.createObjectURL(m);let f={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")f.dataUrl=await jp(m);else if(o==="pdf")f.frameUrl=d;else if(o==="text"){let h=await m.text();f.truncated=h.length>m2,f.text=f.truncated?h.slice(0,m2):h}if(c){URL.revokeObjectURL(d);return}i(f),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let l=e.filename||"attachment";return u`
    <${mi} open=${a} onClose=${t} size="xl">
      <${hh} onClose=${t}>
        <span className="block truncate">${l}</span>
      <//>
      <${fi} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&u`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&u`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&u`<${r5} mode=${o} view=${s} filename=${l} />`}
      <//>
      <${pi}>
        ${s.downloadUrl&&u`<a
          href=${s.downloadUrl}
          download=${l}
          data-testid="attachment-download"
          className="v2-button inline-flex items-center gap-1.5 rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-200 hover:border-signal/35 hover:text-white"
        >
          <${M} name="download" className="h-3.5 w-3.5" />
          <span>Download</span>
        </a>`}
        <button
          type="button"
          onClick=${t}
          className="v2-button rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-200 hover:border-signal/35 hover:text-white"
        >
          Close
        </button>
      <//>
    <//>
  `}function r5({mode:e,view:t,filename:a}){switch(e){case"image":return u`<img
        src=${t.dataUrl}
        alt=${a}
        className="mx-auto max-h-[70vh] w-auto rounded object-contain"
      />`;case"audio":return u`<audio controls src=${t.dataUrl} className="w-full" />`;case"video":return u`<video controls src=${t.dataUrl} className="max-h-[70vh] w-full rounded" />`;case"pdf":return u`<iframe
        src=${t.frameUrl}
        title=${a}
        className="h-[70vh] w-full rounded border border-iron-700 bg-white"
      />`;case"text":return u`<div className="w-full">
        <pre
          className="max-h-[70vh] w-full overflow-auto whitespace-pre-wrap break-words rounded bg-iron-900/60 p-3 text-xs text-iron-200"
        >${t.text}</pre>
        ${t.truncated&&u`<div className="mt-2 text-xs text-iron-400">
          Preview truncated — download the file to see the rest.
        </div>`}
      </div>`;default:return u`<div className="flex flex-col items-center gap-2 text-iron-400">
        <${M} name="file" className="h-10 w-10 text-signal" />
        <div className="text-sm">This file type can't be previewed.</div>
      </div>`}}var s5=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function i5(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function f2(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of i5(e).matchAll(s5)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function p2(e){return e.split("/").filter(Boolean).pop()||e}function h2(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function o5({threadId:e,path:t,onPreview:a}){let[n,r]=p.default.useState({mime_type:"",size_label:""});p.default.useEffect(()=>{let i=!0;return Gx({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:h2(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:p2(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:xc({threadId:e,path:t})};return u`<${qc}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function v2({threadId:e,content:t}){let a=p.default.useMemo(()=>f2(t),[t]),[n,r]=p.default.useState(null);return!e||a.length===0?null:u`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>u`<${o5}
          key=${s}
          threadId=${e}
          path=${s}
          onPreview=${r}
        />`)}
      <${Ic}
        attachment=${n}
        onClose=${()=>r(null)}
      />
    </div>
  `}var g2={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function l5(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function u5({content:e}){let[t,a]=p.default.useState(!1);return e?u`
    <div className="flex flex-col items-start">
      <button
        type="button"
        onClick=${()=>a(n=>!n)}
        aria-expanded=${t?"true":"false"}
        className="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent px-1 py-1 text-xs font-medium text-iron-400 hover:text-iron-200"
      >
        <${M} name="spark" className="h-3.5 w-3.5" />
        <span>${t?"Hide reasoning":"Reasoning"}</span>
        <${M}
          name="chevron"
          className=${["h-3 w-3",t?"rotate-180":""].join(" ")}
        />
      </button>
      ${t&&u`
        <div className="mt-1 border-l-2 border-white/10 pl-3 text-iron-300">
          <${sa} content=${e} className="text-[13px]" />
        </div>
      `}
    </div>
  `:null}function c5({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:l,status:c,error:d,toolCalls:m,timestamp:f}=e,h=n==="user",[x,y]=p.default.useState(!1),[$,g]=p.default.useState(null),v=p.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),y(!0),ii("Copied to clipboard",{tone:"success"}),setTimeout(()=>y(!1),1400)}catch{}},[r]);if(n==="tool_activity"||m&&m.length>0){let O=m&&m.length>0?{id:e.id,toolCalls:m}:e;return u`<${ci} activity=${O} />`}if(n==="thinking")return u`<${u5} content=${r} />`;if(n==="image")return u`
      <div className="flex">
        <div className="flex flex-wrap gap-2">
          ${(o||[]).map((P,k)=>P.data_url?u`<img key=${k} src=${P.data_url} className="max-h-64 rounded-lg border border-iron-700 object-cover" alt="Generated result" />`:u`
                  <div key=${k} className="rounded-lg border border-iron-700 bg-iron-900/70 px-4 py-3 text-sm text-iron-200">
                    <div>Generated image unavailable in history payload</div>
                    ${P.path&&u`<div className="mt-1 font-mono text-xs text-iron-300">${P.path}</div>`}
                  </div>
                `)}
        </div>
      </div>
    `;let b=l5(f),w=n==="user"||n==="assistant"&&!l,S=n==="system"||n==="error",E=h?"max-w-[85%]":S?"mx-auto max-w-[85%]":"w-full max-w-[85%]",N=h?"":"w-full min-w-0 max-w-full",T=c==="error"&&t,L=w||T||b;return u`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",h?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col",E].join(" ")}>
        <div
          className=${["text-base leading-7",N,g2[n]||g2.assistant,l?"opacity-70":""].join(" ")}
        >
          ${n==="assistant"||n==="system"||n==="error"?u`<${sa} content=${r} />`:u`<div className="whitespace-pre-wrap">${r}</div>`}

          ${c==="error"&&u`
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-300">
              <span>${d}</span>
            </div>
          `}

          ${s&&s.length>0&&u`
            <div className="mt-2 flex flex-wrap gap-2">
              ${s.map((O,P)=>u`<img key=${P} src=${O} className="max-h-48 rounded-lg border border-iron-700 object-cover" alt="Message attachment" />`)}
            </div>
          `}

          ${i&&i.length>0&&u`
            <div className="mt-2 flex flex-col gap-1.5">
              ${i.map((O,P)=>u`<${qc}
                key=${O.id||P}
                att=${O}
                onPreview=${g}
              />`)}
            </div>
            <${Ic}
              attachment=${$}
              onClose=${()=>g(null)}
            />
          `}

          ${n==="assistant"&&u`<${v2}
            threadId=${a}
            content=${typeof r=="string"?r:""}
          />`}
        </div>
      </div>

      ${L&&u`
        <div
          className=${["mt-1 flex min-h-7 w-max max-w-[85%] flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100",h?"self-end justify-end":S?"self-center justify-center":"self-start justify-start"].join(" ")}
        >
          ${b&&u`<time dateTime=${f} className="shrink-0 font-mono text-[11px] text-iron-500">${b}</time>`}
          ${(w||T)&&u`
            <div className="flex shrink-0 items-center gap-1">
            ${w&&u`
              <button
                type="button"
                onClick=${v}
                title=${x?"Copied":"Copy message"}
                aria-label=${x?"Copied":"Copy message"}
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 hover:text-iron-100"
              >
                <${M} name=${x?"check":"copy"} className="h-3.5 w-3.5" />
              </button>
            `}
            ${T&&u`
              <button
                type="button"
                onClick=${()=>t(e)}
                title="Retry message"
                aria-label="Retry message"
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 text-red-300 hover:text-red-200"
              >
                <${M} name="retry" className="h-3.5 w-3.5" />
              </button>
            `}
            </div>
          `}
        </div>
      `}
    </div>
  `}var y2=p.default.memo(c5);function N2(e){let t=d5(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(_2(r)){let s=b2(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){x2(a,s),$2(a,r),n+=s.length;continue}}if(vh(r)){let s=b2(t,n);x2(a,s),n+=s.length-1;continue}$2(a,r)}return a}function d5(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=Hc(i);o&&_2(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!vh(i))continue;let o=Hc(i),l=o?t.get(o):void 0;if(l===void 0||l>=s)continue;let c=a.get(l)||[];c.push(i),a.set(l,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function b2(e,t){let a=t,n=Hc(e[t]);for(;a<e.length&&vh(e[a])&&m5(n,e[a]);)a+=1;return e.slice(t,a)}function m5(e,t){let a=Hc(t);return!e||!a||a===e}function x2(e,t){if(t.length===0)return;let a=f5(t);e.push({type:"activity-run",id:`activity-run-${a[0].id}`,activity:a})}function $2(e,t){e.push({type:"message",id:t.id,message:t})}function _2(e){return e.role==="assistant"&&!R2(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function vh(e){return e.role==="thinking"||e.role==="tool_activity"||R2(e)}function R2(e){return e?.toolCalls&&e.toolCalls.length>0}function Hc(e){return e?.turnRunId||null}function f5(e){return[...e].sort((t,a)=>t?.role!=="tool_activity"||a?.role!=="tool_activity"?0:p5(t,a))}function p5(e,t){if(Number.isFinite(e.activityOrder)&&Number.isFinite(t.activityOrder)){let n=e.activityOrder-t.activityOrder;if(n!==0)return n}let a=w2(S2(e.updatedAt||e.timestamp),S2(t.updatedAt||t.timestamp));return a!==0?a:w2(e.sequence,t.sequence)}function w2(e,t){let a=Number.isFinite(e)?e:null,n=Number.isFinite(t)?t:null;return a===null&&n===null?0:a===null?1:n===null?-1:a-n}function S2(e){if(!e)return null;let t=Date.parse(e);return Number.isFinite(t)?t:null}var h5=100,v5=100;function g5(e){return e?e.scrollHeight-e.scrollTop-e.clientHeight:Number.POSITIVE_INFINITY}function k2(e,t=h5){return g5(e)<=t}function C2(e){e&&(e.scrollTop=Math.max(0,e.scrollHeight-e.clientHeight))}function E2(e){return e?.id?`${e.role||""}:${e.id}`:null}function y5(e,t){let a=E2(t);return!!(a&&t?.role==="user"&&a!==e)}function T2({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let l=C(),c=p.default.useRef(null),d=p.default.useRef(null),m=p.default.useRef(!0),f=p.default.useRef(null),h=p.default.useRef(null),x=p.default.useRef(null),y=p.default.useRef(0),$=p.default.useRef(!1),[g,v]=p.default.useState(!0),b=p.default.useCallback(()=>{h.current!==null&&(window.cancelAnimationFrame(h.current),h.current=null)},[]),w=p.default.useCallback((k=!1)=>{c.current&&(k&&(m.current=!0,$.current=!1),m.current&&(b(),h.current=window.requestAnimationFrame(()=>{h.current=null;let Y=c.current;!Y||!k&&!m.current||(C2(Y),y.current=Y.scrollTop,$.current=!1,v(!0))})))},[b]),S=p.default.useCallback(()=>{x.current!==null&&(window.cancelAnimationFrame(x.current),x.current=null)},[]);p.default.useLayoutEffect(()=>{let k=e.length>0?e[e.length-1]:null,U=E2(k),Y=y5(f.current,k);return f.current=U,w(Y),b},[e,i,w,b]),p.default.useLayoutEffect(()=>{let k=d.current;if(!k||typeof ResizeObserver!="function")return;let U=new ResizeObserver(()=>{w()});return U.observe(k),()=>{U.disconnect(),b()}},[w,b]);let E=p.default.useCallback(()=>{x.current=null;let k=c.current;if(!k)return;let U=k2(k);y.current=k.scrollTop,U?(m.current=!0,$.current=!1,v(!0)):$.current?(m.current=!1,v(!1)):(m.current=!0,v(!0),w()),a&&k.scrollTop<v5&&n&&!t&&n()},[a,n,t,w]),N=p.default.useCallback(()=>{$.current=!0},[]),T=p.default.useCallback(k=>{let U=c.current;if(!U||typeof k?.clientX!="number")return;let Y=U.offsetWidth-U.clientWidth;if(Y<=0)return;let ae=U.getBoundingClientRect().right;k.clientX>=ae-Y-2&&($.current=!0)},[]),L=p.default.useCallback(()=>{let k=c.current;if(!k)return;let U=k2(k),Y=k.scrollTop<y.current;y.current=k.scrollTop,!U&&Y&&($.current=!0),U?(m.current=!0,$.current=!1):$.current?(m.current=!1,b()):m.current=!0,x.current===null&&(x.current=window.requestAnimationFrame(E))},[b,E]),O=p.default.useCallback(()=>{let k=c.current;k&&(C2(k),y.current=k.scrollTop,m.current=!0,$.current=!1,v(!0))},[]);p.default.useEffect(()=>S,[S]);let P=p.default.useMemo(()=>N2(e),[e]);return u`
    <div className="relative flex min-h-0 min-w-0 flex-1">
    <div
      ref=${c}
      onScroll=${L}
      onWheel=${N}
      onTouchMove=${N}
      onPointerDown=${T}
      className="flex min-w-0 flex-1 overflow-y-auto px-4 pt-6 pb-14 sm:px-5 lg:px-8"
    >
      <div ref=${d} className="mx-auto flex w-full min-w-0 max-w-5xl flex-col gap-5">
        ${a&&u`
          <div className="text-center">
            <button
              onClick=${n}
              disabled=${t}
              className="v2-button rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-300 hover:border-signal/35 hover:text-white disabled:opacity-50"
            >
              ${l(t?"chat.history.loading":"chat.history.loadOlder")}
            </button>
          </div>
        `}
        ${P.map(k=>k.type==="activity-run"?u`<${l2} key=${k.id} activity=${k.activity} />`:u`<${y2}
                key=${k.id}
                message=${k.message}
                onRetry=${r}
                threadId=${s}
              />`)}
        ${o}
      </div>
    </div>
    ${!g&&u`
      <button
        type="button"
        onClick=${O}
        aria-label=${l("chat.jumpToLatest")}
        className="absolute bottom-4 left-1/2 inline-flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-3 py-1.5 text-xs font-medium text-[var(--v2-text-strong)] shadow-[0_10px_30px_-12px_rgba(0,0,0,0.7)] hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]"
      >
        <${M} name="arrowDown" className="h-3.5 w-3.5" />
        ${l("chat.jumpToLatest")}
      </button>
    `}
    </div>
  `}function A2({notice:e,onRecover:t}){return u`
    <div className="mx-auto flex max-w-xl flex-wrap items-center justify-center gap-3 rounded-lg border border-copper/30 bg-copper/10 px-4 py-3 text-sm text-copper">
      <span>${e.message}</span>
      ${e.status!=="loading"&&u`
        <button
          type="button"
          onClick=${t}
          className="rounded-md border border-copper/40 px-2.5 py-1 text-xs font-medium hover:bg-copper/10"
        >
          Reload history
        </button>
      `}
    </div>
  `}function D2({suggestions:e,onSelect:t,disabled:a=!1}){return!e||e.length===0?null:u`
    <div className="px-4 pb-3 sm:px-5 lg:px-8">
      <div className="mx-auto flex max-w-5xl flex-wrap gap-2">
        ${e.map(n=>u`
            <button
              key=${n}
              onClick=${()=>{a||t(n)}}
              disabled=${a}
              className="v2-button rounded-full border border-white/10 bg-white/[0.035] px-3 py-1.5 text-xs text-iron-100 hover:border-signal/40 hover:text-signal disabled:cursor-not-allowed disabled:opacity-50"
            >
              ${n}
            </button>
          `)}
      </div>
    </div>
  `}function M2(){return u`
    <div className="flex flex-col items-start">
      <div className="flex min-w-0 max-w-[85%] flex-col gap-2">
        <div
          data-testid="typing-indicator"
          className="w-fit rounded-[18px] border border-white/10 bg-iron-800/60 px-4 py-3"
        >
          <div className="flex gap-1">
            <span className="v2-typing-dot h-2 w-2 rounded-full bg-iron-200" />
            <span className="v2-typing-dot h-2 w-2 rounded-full bg-iron-200" />
            <span className="v2-typing-dot h-2 w-2 rounded-full bg-iron-200" />
          </div>
        </div>
      </div>
    </div>
  `}function Kc(){return K("/api/webchat/v2/channels/connectable")}function O2(e,t){if(!gh(e))return null;let a=Qc(e),n=w5(a),r=null;for(let s of t||[]){if(!$5(s))continue;let i=S5(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function gh(e){let t=Qc(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function b5(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function x5(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>L2(Qc(n))):a}function $5(e){return e?.strategy!=="admin_managed_channels"}function w5(e){return P2(e,"slack")&&L2(e)}function L2(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function Qc(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function S5(e,t,a={}){return(a.commandAliasesOnly?x5(t,{channelManagementOnly:!0}):b5(t)).reduce((r,s)=>{let i=Qc(s);return P2(e,i)?Math.max(r,i.length):r},0)}function P2(e,t){return t?` ${e} `.includes(` ${t} `):!1}function j2(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n={kind:"gate",gateKind:"approval",runId:t.turn_run_id,gateRef:t.gate_ref,invocationId:t.invocation_id||null,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return N5(n,a,t.body)}return e==="auth_required"?{kind:"auth_required",gateKind:"auth",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,invocationId:t.invocation_id||null,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function U2(e){if(!e?.run_id||!e.gate_ref)return null;let t=e.gate_kind||"generic",a={gateKind:t,runId:e.run_id,gateRef:e.gate_ref,invocationId:e.invocation_id||null,headline:e.headline,body:e.body||"",allowAlways:e.allow_always===!0};if(t==="auth"){let n=e.auth_context||{};return{...a,kind:"auth_required",challengeKind:n.challenge_kind||"other",provider:n.provider||null,accountLabel:n.account_label||"",authorizationUrl:n.authorization_url||null,expiresAt:n.expires_at||null}}return{...a,kind:"gate"}}function N5(e,t,a){if(!t)return e;let n=_5(t);return{...e,toolName:t.tool_name||null,description:t.reason||a,actionLabel:t.action?.label||null,destination:t.destination||null,approvalScope:t.scope||null,approvalDetails:n,parameters:n.length?n.map(r=>`${r.label}: ${r.value}`).join(`
`):null}}function _5(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function F2({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function B2(){return{terminalByInvocation:new Map}}function z2(e){e?.current?.terminalByInvocation?.clear()}function bh(e,t,a){let n=I2(t,{toolStatus:"running"});n&&hi(e,n,a)}function q2(e,t,a,n="gate_declined"){let r=I2(t,{toolStatus:"declined",toolError:n,toolErrorKind:"gate_declined"});r&&hi(e,r,a)}function hi(e,t,a){if(!t)return;let n=A5(t);n=T5(n,a),e(r=>{let s=H2(n),i=k5(r,n,s);if(i>=0){let l=[...r];return l[i]=C5(l[i],n),yh(l[i],a),l}let o={id:s,role:"tool_activity",...n};return yh(o,a),[...r,o]})}function I2(e,t={}){let a=e?.kind==="gate",n=e?.kind==="auth_required",r=n&&t.toolStatus==="declined";if(!e?.runId||!e?.gateRef||!e.invocationId&&!r||!a&&!n)return null;let s=e.invocationId||R5(e),i=e.toolName||e.headline||e.gateKind||"gate";return{invocationId:s,callId:s,capabilityId:e.toolName||e.gateKind||null,toolName:Jo(i)||i,toolStatus:t.toolStatus||"running",toolDetail:null,toolParameters:null,toolResultPreview:null,toolError:t.toolError||null,toolErrorKind:t.toolErrorKind||null,toolDurationMs:null,updatedAt:t.updatedAt||new Date().toISOString(),resultRef:null,truncated:!1,outputBytes:null,outputKind:null,turnRunId:e.runId,gateRef:e.gateRef,gateActivity:!0}}function R5(e){return`gate:${e.runId}:${e.kind}:${e.gateRef}`}function H2(e){return`tool-${e.invocationId}`}function k5(e,t,a){let n=e.findIndex(s=>s?.id===a);if(n>=0)return n;let r=t.gateRef||null;if(r){let s=e.findIndex(i=>i?.role==="tool_activity"&&i.turnRunId===t.turnRunId&&i.gateRef===r);if(s>=0)return s}return-1}function C5(e,t){let a=Yo(e.toolStatus),n=Yo(t.toolStatus),r=a&&!n,s=t.gateActivity&&!e.gateActivity,i={...e,...t,id:e.id,role:"tool_activity",invocationId:e.gateActivity&&!t.gateActivity?t.invocationId:e.invocationId||t.invocationId,callId:e.gateActivity&&!t.gateActivity?t.callId:e.callId||t.callId,toolName:s?e.toolName:t.toolName||e.toolName,toolStatus:r?e.toolStatus:t.toolStatus,toolError:t.toolError||e.toolError,toolErrorKind:t.toolErrorKind||e.toolErrorKind||null,updatedAt:r?e.updatedAt||t.updatedAt:t.updatedAt||e.updatedAt,turnRunId:t.turnRunId||e.turnRunId||null,gateRef:t.gateRef||e.gateRef||null,gateActivity:e.gateActivity&&t.gateActivity,capabilityId:s?e.capabilityId||t.capabilityId||null:t.capabilityId||e.capabilityId||null,activityOrder:E5(e,t),activityOrderSource:t.activityOrderSource||e.activityOrderSource||null};return e.gateActivity&&!t.gateActivity&&(i.id=H2(t),i.gateActivity=!1),i}function E5(e,t){return Number.isFinite(t.activityOrder)?t.activityOrder:e.activityOrder}function T5(e,t){if(!e?.invocationId)return e;if(Yo(e.toolStatus))return yh(e,t),e;let a=t?.current?.terminalByInvocation?.get(e.invocationId);return a?Number.isFinite(e.activityOrder)?{...a,activityOrder:e.activityOrder,activityOrderSource:e.activityOrderSource||a.activityOrderSource||null}:a:e}function yh(e,t){!e?.invocationId||!Yo(e.toolStatus)||t?.current?.terminalByInvocation?.set(e.invocationId,e)}function A5(e){let t=Jo(e.toolName||e.capabilityId);return{...e,toolName:t||e.toolName||"tool"}}function Y2({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o,onRunSettled:l}){let c=p.default.useRef(new Set),d=p.default.useRef(null),m=p.default.useRef(null);return p.default.useCallback(f=>{let{type:h,frame:x}=f||{};if(!(!h||!x))switch(h){case"accepted":{let y=x.ack||{};y.run_id&&(d.current=y.run_id),r?.({runId:y.run_id||null,threadId:y.thread_id||e,status:y.status||null}),a(!0);return}case"running":case"capability_progress":{let y=x.progress||{};y.turn_run_id&&(d.current=y.turn_run_id,r?.($=>$&&$.runId===y.turn_run_id?{...$,status:"running"}:{runId:y.turn_run_id,threadId:e,status:"running"}),D5(n,y.turn_run_id,m)),a(!0);return}case"capability_activity":{let y=x.activity;if(!y||!y.invocation_id)return;hi(t,Hp(y),o);return}case"capability_display_preview":{let y=x.preview;if(!y||!y.invocation_id)return;let $=Ip(y);hi(t,$,o);return}case"gate":case"auth_required":{let y=j2(h,x.prompt);y&&(bh(t,y,o),n(y),r?.({runId:y.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let y=x.reply||{};t($=>[...$,{id:`reply-${y.turn_run_id||Date.now()}`,role:"assistant",content:y.text||"",timestamp:y.generated_at||new Date().toISOString(),turnRunId:y.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let y=x.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Yc(c,l,y,!1);return}case"failed":{let y=x.run_state||{},$=y.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),$h(t,{runId:$,status:y.status||"failed",failureCategory:P5(y),failureSummary:null}),Yc(c,l,$,!1);return}case"projection_snapshot":case"projection_update":{let y=x.state?.items||[];O5({items:y,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:l,settledRunsRef:c,latestRunIdRef:d,promptRunIdRef:m,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i,o,l])}function Yc(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var K2=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),Q2=new Set(["completed","succeeded"]),Vc=new Set(["blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]),Gc=new Set(["awaiting_gate","blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]);function V2(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function D5(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function M5(e,t,a,n,r,s){let i=t?.runId||null;if(!i||r?.has(i))return!0;let o=a?.get(i);if(o)return!Gc.has(o);let l=e?.current,c=l?.runId||n?.current||null;if(c&&i!==c)return!0;let d=s?.current===c;return c&&i===c&&!d&&l?.status&&!Gc.has(l.status)?!0:!l?.runId||!l.status?!1:!Gc.has(l.status)}function O5({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:l,promptRunIdRef:c,activeRunRef:d,locallyResolvedGatesRef:m,toolActivityStateRef:f}){let h=new Map,x=new Set,y=d?.current||null,$=y?.runId||l?.current||null;for(let v of e){let b=v.run_status;b?.run_id&&b.status&&(h.set(b.run_id,b.status),$&&$!==b.run_id&&y?.status&&!K2.has(y.status)&&Vc.has(b.status)&&x.add(b.run_id))}let g=l?.current??null;for(let v of e){if(v.run_status){let{run_id:b,status:w,failure_category:S,failure_summary:E}=v.run_status,N=K2.has(w),T=d?.current?.source==="local"?d.current.runId:null,L=!!(b&&T&&T!==b),O=g??l?.current??null,P=!!(N&&b&&O&&O!==b),k=b&&Vc.has(w)?G2(m,b):null;if(b&&x.has(b)||L)continue;if(P){G2(m,d?.current?.runId)?.outcome==="resumed"&&(L5({runId:b,activePromptRunId:d?.current?.runId,success:Q2.has(w),status:w,failureCategory:S,failureSummary:E,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:l,promptRunIdRef:c,locallyResolvedGatesRef:m}),g=null);continue}if(k){V2(r,b,c),k.outcome==="resumed"?(n(!0),s?.(U=>U&&U.runId===b?{...U,status:U.status==="awaiting_gate"?"queued":U.status||"queued"}:{runId:b,threadId:t,status:"queued"}),g=b,l&&(l.current=b)):(n(!1),d?.current?.runId===b&&s?.(null),g=null,l?.current===b&&(l.current=null));continue}b&&(g=b,!N&&l&&(l.current=b),s?.(U=>U&&U.runId===b?{...U,status:w}:{runId:b,threadId:t,status:w})),b&&Vc.has(w)?c&&(c.current=b):b&&c?.current===b&&(c.current=null),N?(n(!1),r(null),s?.(null),xh(m,b),g=null,l&&(l.current=null),b&&c?.current===b&&(c.current=null),Yc(o,i,b,Q2.has(w)),(w==="failed"||w==="recovery_required")&&$h(a,{runId:b,status:w,failureCategory:S,failureSummary:E})):Vc.has(w)||(V2(r,b,c),xh(m,b),n(!0))}if(v.text){let b=`text-${v.text.id}`;a(w=>{let S=w.findIndex(N=>N.id===b),E={id:b,role:"assistant",content:v.text.body||"",timestamp:new Date().toISOString(),isFinalReply:!0};if(S>=0){let N=[...w];return N[S]=E,N}return[...w,E]}),n(!1)}if(v.thinking){let b=`thinking-${v.thinking.id}`;a(w=>{let S=w.findIndex(N=>N.id===b),E={id:b,role:"thinking",content:v.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:v.thinking.run_id||null};if(S>=0){let N=[...w];return N[S]=E,N}return[...w,E]})}if(v.capability_activity){let b=v.capability_activity;b.invocation_id&&hi(a,Hp(b),f)}if(v.gate){let b=U2(v.gate),w=b?.runId||null;w&&!M5(d,b,h,l,x,c)&&!U5(m,w,b.gateRef)&&(bh(a,b,f),r(S=>S||b),s?.(S=>S&&S.runId===w?{...S,status:Gc.has(S.status)?S.status:"awaiting_gate"}:{runId:w,threadId:t,status:"awaiting_gate"}),c&&(c.current=w),n(!1))}if(v.skill_activation){let{id:b,skill_names:w=[],feedback:S=[]}=v.skill_activation;if(w.length||S.length){let E=`skill-${b||w.join("-")||"activation"}`,N=[w.length?`Skill activated: ${w.join(", ")}`:"",...S].filter(Boolean).join(`
`);a(T=>T.some(L=>L.id===E)?T:[...T,{id:E,role:"system",content:N,timestamp:new Date().toISOString()}])}}}l&&g&&(l.current=g)}function L5({runId:e,activePromptRunId:t,success:a,status:n,failureCategory:r,failureSummary:s,setMessages:i,setIsProcessing:o,setPendingGate:l,setActiveRun:c,onRunSettled:d,settledRunsRef:m,latestRunIdRef:f,promptRunIdRef:h,locallyResolvedGatesRef:x}){o(!1),l(null),c?.(null),xh(x,t),f&&(f.current=null),h?.current===t&&(h.current=null),Yc(m,d,e,a),(n==="failed"||n==="recovery_required")&&$h(i,{runId:e,status:n,failureCategory:r,failureSummary:s})}function P5(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function $h(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`,i=typeof t=="string"&&t?t:null;e(o=>{let l=o.findIndex(d=>d.id===s),c=F2({status:a,failureCategory:n,failureSummary:r});if(l>=0){let d=!!(r&&o[l].content!==c),m=!!(i&&o[l].turnRunId!==i);if(!d&&!m)return o;let f=[...o];return f[l]={...f[l],...d&&{content:c},...m&&{turnRunId:i}},f}return[...o,{id:s,role:"error",content:c,timestamp:new Date().toISOString(),...i&&{turnRunId:i}}]})}function G2(e,t){if(!t)return null;let a=e?.current;if(!a)return null;for(let[n,r]of a.entries())if(n.startsWith(`${t}
`))return j5(r);return null}function j5(e){return e&&typeof e=="object"?{resolution:e.resolution||null,outcome:e.outcome||null}:{resolution:e||null,outcome:null}}function xh(e,t){if(!t)return;let a=e?.current;if(a)for(let n of Array.from(a.keys()))n.startsWith(`${t}
`)&&a.delete(n)}function U5(e,t,a){return!t||!a?!1:!!e?.current?.has(`${t}
${a}`)}function J2(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function X2(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function W2(e,t,a,n){let r=wh(n);return r?(F5(e,t,a,{timelineMessageId:r}),r):null}function F5(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function wh(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var B5=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function Z2({threadId:e,onEvent:t,enabled:a}){let[n,r]=p.default.useState("idle"),s=p.default.useRef(t);s.current=t;let i=p.default.useRef(null);return p.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,l=null,c=0,d=3e4;function m(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=u$({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let y=Math.min(1e3*2**c,d);l=setTimeout(m,y)};let x=(y,$)=>{let g=null;try{g=JSON.parse(y.data)}catch{return}!g||typeof g!="object"||(y.lastEventId&&(i.current=y.lastEventId),s.current?.({type:g.type||$,frame:g,lastEventId:y.lastEventId||null}))};o.onmessage=y=>x(y,"message");for(let y of B5)o.addEventListener(y,$=>x($,y))}function f(){l&&(clearTimeout(l),l=null),o&&(o.close(),o=null),r("paused")}function h(){document.visibilityState==="hidden"?f():o||m()}return m(),document.addEventListener("visibilitychange",h),()=>{document.removeEventListener("visibilitychange",h),l&&clearTimeout(l),o&&o.close()}},[a,e]),{status:n}}var z5=3e4,q5="credential_stored_gate_resolution_failed",I5="approval_gate_pending_send_blocked",H5="ironclaw-product-auth",Sh="ironclaw:product-auth:oauth-complete",K5="ironclaw:product-auth:oauth-complete";async function eS(e){let t=new AbortController,a=setTimeout(()=>t.abort(),z5);try{return await e(t.signal)}finally{clearTimeout(a)}}function Q5(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=q5,t.cause=e,t}function V5(){let e=new Error("Resolve the approval request before sending another message.");return e.safeErrorCode=I5,e}function G5(e){let a=At.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function tS(e,t){return!e||!t?.runId||!t?.gateRef?null:`${e}
${t.runId}
${t.gateRef}`}function Y5(e){return e?.continuation?.type==="turn_gate_resume"}function J5(e){if(e?.outcome)return e.outcome;let t=String(e?.status||"").toLowerCase();return t==="queued"||t==="running"?"resumed":t==="cancelled"||e?.already_terminal===!0?"cancelled":e?.already_terminal===!1?"resumed":null}function aS(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function X5(e){return e?.type===K5&&e?.status==="completed"}function W5(e,t,a){if(!X5(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function Nh(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function Z5(e){if(!gh(e))return null;try{let a=(await At.fetchQuery({queryKey:["connectable-channels"],queryFn:Kc}))?.channels||[];return O2(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function nS(e){let t=p.default.useRef(e),a=p.default.useRef(new Map),n=p.default.useRef(1),[r,s]=p.default.useState(0),[i,o]=p.default.useState(Date.now()),[l,c]=p.default.useState(null),d=p.default.useRef(l),m=p.default.useCallback(ee=>{let re=typeof ee=="function"?ee(d.current):ee;d.current=re,c(re)},[]);p.default.useEffect(()=>{d.current=l},[l]);let[f,h]=p.default.useState(null),x=p.default.useCallback(()=>a.current.get(e||"__new__")||[],[e]),y=p.default.useCallback(ee=>{let re=e||"__new__";ee.length>0?a.current.set(re,ee):a.current.delete(re)},[e]),{messages:$,hasMore:g,nextCursor:v,isLoading:b,loadError:w,loadHistory:S,seedThreadMessages:E,setMessages:N}=j$(e,{getPendingMessages:x,setPendingMessages:y}),[T,L]=p.default.useState(!1),O=p.default.useRef(T),P=p.default.useCallback(ee=>{let re=typeof ee=="function"?ee(O.current):ee;O.current=re,L(re)},[]),[k,U]=p.default.useState(null),Y=p.default.useRef(k),[ae,se]=p.default.useState(null),ie=p.default.useCallback(ee=>{let re=Y.current,Se=typeof ee=="function"?ee(re):ee;Object.is(Se,re)||(Y.current=Se,U(Se))},[]),[ot,St]=p.default.useState(e),ft=p.default.useRef(B2()),Ue=p.default.useRef(new Map),Fe=p.default.useRef({gateKey:null,credentialRef:null,inFlight:!1}),Nt=p.default.useRef(!1);ot!==e&&(St(e),L(!1),U(null),se(null),c(null),h(null)),p.default.useEffect(()=>{t.current=e},[e]),p.default.useEffect(()=>{Y.current=k},[k]),p.default.useEffect(()=>{O.current=T},[T]),p.default.useEffect(()=>{let ee=tS(e,k);se(re=>re&&re.gateKey!==ee?null:re)},[k,e]),p.default.useEffect(()=>{z2(ft),Ue.current.clear()},[e]);let kn=Math.max(0,Math.ceil((r-i)/1e3)),$a=k?.runId&&k?.gateRef?`${k.runId}
${k.gateRef}`:null;p.default.useEffect(()=>{if(!r)return;let ee=setInterval(()=>o(Date.now()),250);return()=>clearInterval(ee)},[r]),p.default.useEffect(()=>{Fe.current.gateKey!==$a&&(Fe.current={gateKey:$a,credentialRef:null,inFlight:!1})},[$a]),p.default.useEffect(()=>{if(!aS(k))return;let ee=Date.now(),re=R=>{W5(R,k,ee)&&(ie(D=>aS(D)?null:D),P(!0))},Se=null;typeof window.BroadcastChannel=="function"&&(Se=new window.BroadcastChannel(H5),Se.onmessage=R=>re(R.data));let Ce=R=>{R.key===Sh&&re(Nh(R.newValue))};window.addEventListener("storage",Ce),re(Nh(window.localStorage?.getItem?.(Sh)));let _=window.setInterval(()=>{re(Nh(window.localStorage?.getItem?.(Sh)))},500);return()=>{window.clearInterval(_),Se&&Se.close(),window.removeEventListener("storage",Ce)}},[k]);let la=Y2({threadId:e,setMessages:N,setIsProcessing:P,setPendingGate:ie,setActiveRun:m,activeRunRef:d,locallyResolvedGatesRef:Ue,toolActivityStateRef:ft,onRunSettled:(ee,{success:re})=>{re&&y([]),S(void 0,{preserveClientOnly:!0,finalReplyTimestampByRun:ee&&re?{[ee]:new Date().toISOString()}:null})}}),{status:Da}=Z2({threadId:e,onEvent:la,enabled:!!e}),Ma=p.default.useCallback(async(ee,re={})=>{let{threadId:Se,attachments:Ce=[]}=re,_=Ce.map(C$),R=Ce.map(E$);if(k||Y.current)throw V5();let D=Se||e,I=d.current,F=!!I&&!!D&&I.threadId===D,B=O.current&&!!D&&D===e;if(Nt.current||B||F)return null;if(Ce.length===0){let de=await Z5(ee);if(de)return h(de),{channel_connect_action:de}}h(null);let Q=Se||e;if(!Q){let de=await bc();if(At.invalidateQueries({queryKey:["threads"]}),Q=de?.thread?.thread_id,!Q)throw new Error("createThread returned no thread_id")}let pe=Q,ve={id:`pending-${n.current++}`,role:"user",content:ee,attachments:R,timestamp:new Date().toISOString(),isOptimistic:!0},kt={id:ve.id,role:"user",content:ee,attachments:R,timestamp:ve.timestamp,isOptimistic:!0};J2(a.current,pe,ve);let La=ve.id,Qt=!e||Q===e,Zr=de=>{Qt&&N(de)},Cn=de=>{Q!==e&&E(Q,de)},hr=de=>{Qt&&de()};Nt.current=!0,Zr(de=>[...de,kt]),Cn(de=>[...de,kt]),hr(()=>{P(!0),Y.current||ie(null)});try{let de=await i$({threadId:Q,content:ee,attachments:_});G5(Q)&&At.invalidateQueries({queryKey:["threads"]}),de?.run_id&&Qt&&m({runId:de.run_id,threadId:de.thread_id||Q,status:de.status||null,source:"local"});let En=W2(a.current,pe,La,de?.accepted_message_ref)||wh(de?.accepted_message_ref);if(En){let an=Vt=>Vt.map(Tn=>Tn.id===La?{...Tn,timelineMessageId:En}:Tn);Zr(an),Cn(an)}if(de?.outcome==="rejected_busy"){let an=Vt=>Vt.map(Tn=>Tn.id===La?{...Tn,isOptimistic:!1,status:"error"}:Tn);if(Zr(an),Cn(an),de?.notice){let Vt=(Ei=Qt)=>{let Ek={id:`system-rejected-${n.current++}`,role:"system",content:de.notice,timestamp:new Date().toISOString(),isOptimistic:!1},Zh=Tk=>[...Tk,Ek];Ei&&N(Zh),(!Ei||Q!==e)&&E(Q,Zh)};if(!t.current||t.current===Q){let Ei=tS(Q,Y.current);Ei?se({gateKey:Ei,content:de.notice}):Vt()}else Vt(!1)}hr(()=>P(!1)),Nt.current=!1}else de?.run_id||(Nt.current=!1);return de}catch(de){de.status===429&&s(Date.now()+tD(de));let En=an=>an.map(Vt=>Vt.id===La?{...Vt,isOptimistic:!1,status:"error",error:de.message,retryThreadId:Q}:Vt);throw Zr(En),Cn(En),hr(()=>P(!1)),Nt.current=!1,de}finally{Nt.current=!1,X2(a.current,pe,La)}},[e,k,N,E,P,ie,m]),Oa=p.default.useCallback(async ee=>{let re=typeof ee?.content=="string"?ee.content:"",Se=ee?.retryThreadId||ee?.threadId||e;return!re.trim()||!Se?null:(N(Ce=>Ce.filter(_=>_.id!==ee.id)),await Ma(re,{threadId:Se}))},[Ma,N,e]),wa=p.default.useCallback(async(ee,re={})=>{if(!k)return;let{runId:Se,gateRef:Ce}=k;if(!Se||!Ce)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");let _=await Up({threadId:e,runId:Se,gateRef:Ce,resolution:ee,always:re.always,credentialRef:re.credentialRef}),R=J5(_);if(Ue.current.set(`${Se}
${Ce}`,{resolution:ee,outcome:R}),eD(ee)&&R==="resumed"&&q2(N,k,ft),ie(null),R==="resumed"){P(!0),m({runId:_?.run_id||Se,threadId:_?.thread_id||e,status:_?.status||"queued"});return}P(!1),m(null)},[k,e,N,m]),tn=p.default.useCallback(async ee=>{if(!k)throw new Error("auth gate is no longer pending");let{runId:re,gateRef:Se,provider:Ce}=k;if(!re||!Se||!Ce)throw new Error("auth gate is missing required credential metadata");let _=k.accountLabel||`${Ce} credential`,R=`${re}
${Se}`;if(Fe.current.gateKey!==R&&(Fe.current={gateKey:R,credentialRef:null,inFlight:!1}),Fe.current.inFlight)throw new Error("auth token submission already in progress");Fe.current.inFlight=!0;try{let D=Fe.current.credentialRef,I=null;if(!D){if(I=await eS(F=>d$({provider:Ce,accountLabel:_,token:ee,threadId:e,runId:re,gateRef:Se,signal:F})),D=I?.credential_ref,!D)throw new Error("manual token submit returned no credential_ref");Fe.current.credentialRef=D}if(!Y5(I))try{await eS(F=>Up({threadId:e,runId:re,gateRef:Se,resolution:"credential_provided",credentialRef:D,signal:F}))}catch(F){throw Q5(F)}Fe.current={gateKey:null,credentialRef:null,inFlight:!1},ie(null),P(!0)}catch(D){throw Fe.current.gateKey===R&&(Fe.current.inFlight=!1),D}},[k,e]),tt=p.default.useCallback(async ee=>{let re=l?.runId;!re||!e||(ie(null),P(!1),m(null),Nt.current=!1,await c$({threadId:e,runId:re,reason:ee}))},[l,e]),_t=p.default.useCallback(()=>{g&&v&&S(v)},[g,v,S]),Ot=p.default.useCallback(async(ee,re,Se)=>{let Ce="approved",_=!1;re==="deny"?Ce="denied":re==="cancel"?Ce="cancelled":re==="always"&&(Ce="approved",_=!0),await wa(Ce,{always:_})},[wa]),Rt=p.default.useCallback(()=>{},[]);return{messages:$,isProcessing:T,pendingGate:k,busyGateNotice:ae,channelConnectAction:f,activeRun:l,sseStatus:Da,historyLoading:b,historyLoadError:w,hasMore:g,cooldownSeconds:kn,send:Ma,resolveGate:wa,submitAuthToken:tn,cancelRun:tt,loadMore:_t,dismissChannelConnectAction:()=>h(null),suggestions:[],setSuggestions:Rt,retryMessage:Oa,approve:Ot,recoverHistory:Rt,recoveryNotice:null}}function eD(e){return e==="denied"||e==="cancelled"}function tD(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function rS({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}var aD=1500;function sS({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let o=C(),{messages:l,isProcessing:c,pendingGate:d,busyGateNotice:m,channelConnectAction:f,suggestions:h,sseStatus:x,historyLoading:y,historyLoadError:$,hasMore:g,cooldownSeconds:v,recoveryNotice:b,activeRun:w,send:S,cancelRun:E,retryMessage:N,approve:T,recoverHistory:L,loadMore:O,setSuggestions:P,submitAuthToken:k,dismissChannelConnectAction:U}=nS(t),Y=p.default.useMemo(()=>e.find(tt=>tt.id===t)||null,[e,t]),ae=p.default.useMemo(()=>rS({gatewayStatus:i,activeThread:Y}),[i,Y]),se=!!t&&!!d,ie=!!t&&c,ot=l.length>0||ie||se||!!f,St=!y&&!ot&&!$,ft=se?"Resolve the approval request before sending another message.":"",Ue=se||ie&&!se||v>0,Fe=p.default.useRef(Ue);Fe.current=Ue;let Nt=ft||(v>0?`Retry in ${v}s`:void 0),kn=t||el,$a=!!(t&&w?.runId&&w.threadId===t&&ie&&!se),la=p.default.useCallback(async(tt,{images:_t=[],attachments:Ot=[]}={})=>{if(se)throw new Error(ft);if(Fe.current)return null;let Rt=await S(tt,{images:_t,attachments:Ot,threadId:t}),ee=Rt?.thread_id||t;return!t&&ee&&a&&a(ee,{replace:!0}),Rt},[t,se,ft,Ue,a,S]),Da=p.default.useCallback(async tt=>{Ue||(P([]),await la(tt))},[Ue,la,P]),Ma=p.default.useCallback(async tt=>{let _t=await N(tt),Ot=_t?.thread_id||t;return!t&&Ot&&a&&a(Ot,{replace:!0}),_t},[t,a,N]),Oa=p.default.useCallback(()=>E("user_requested"),[E]);p.default.useEffect(()=>{if(!t)return;if(d){Ec(t,Sn.NEEDS_ATTENTION);return}if(c){Ec(t,Sn.RUNNING);return}let tt=setTimeout(()=>Gw(t),aD);return()=>clearTimeout(tt)},[t,d,c]);let[wa,tn]=p.default.useState(!1);return p.default.useEffect(()=>{let tt=_t=>{if(_t.key==="Escape"){tn(!1);return}if(_t.key!=="?")return;let Ot=_t.target,Rt=Ot?.tagName;Rt==="INPUT"||Rt==="TEXTAREA"||Ot?.isContentEditable||(_t.preventDefault(),tn(ee=>!ee))};return window.addEventListener("keydown",tt),()=>window.removeEventListener("keydown",tt)},[]),u`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${e2} status=${x} />

        ${$&&u`
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            ${$}
          </div>
        `}

        ${St&&u`
          <${t2}
            onSuggestion=${Da}
            onSend=${la}
            disabled=${!1}
            sendDisabled=${Ue}
            initialText=${r}
            resetKey=${s}
            draftKey=${kn}
            context=${ae}
            statusText=${Nt}
            canCancel=${$a}
            onCancel=${Oa}
          />
        `}
        ${!St&&u`
          <${T2}
            messages=${l}
            isLoading=${y}
            hasMore=${g}
            onLoadMore=${O}
            onRetryMessage=${Ma}
            threadId=${t}
            pending=${ie}
          >
            ${b&&u`
              <${A2}
                notice=${b}
                onRecover=${L}
              />
            `}
            ${ie&&!se&&u`<${M2} />`}
            ${f&&u`
              <${X1}
                connectAction=${f}
                onDismiss=${U}
              />
            `}
            ${d&&(d.kind==="auth_required"?d.challengeKind==="oauth_url"?u`
                  <${G1}
                    gate=${d}
                    onCancel=${()=>T(d.requestId,"cancel",d.kind)}
                  />
                `:d.challengeKind==="manual_token"?u`
                  <${Y1}
                    gate=${d}
                    onSubmit=${k}
                    onCancel=${()=>T(d.requestId,"cancel",d.kind)}
                  />
                `:u`
                  <${V1}
                    gate=${d}
                    onCancel=${()=>T(d.requestId,"cancel",d.kind)}
                  />
                `:u`
              <${Q1}
                gate=${d}
                onApprove=${()=>T(d.requestId,"approve",d.kind)}
                onDeny=${()=>T(d.requestId,"deny",d.kind)}
                onAlways=${()=>T(d.requestId,"always",d.kind)}
              />
            `)}
            ${m&&u`
              <div
                data-testid="busy-gate-notice"
                role="status"
                className="mx-auto mt-3 max-w-lg rounded-lg border border-copper/25 bg-copper/10 px-4 py-3 text-center text-sm leading-6 text-copper"
              >
                ${m.content}
              </div>
            `}
          <//>

          <${D2}
            suggestions=${h}
            onSelect=${Da}
            disabled=${Ue}
          />

          <${zc}
            onSend=${la}
            disabled=${!1}
            sendDisabled=${Ue}
            initialText=${r}
            resetKey=${s}
            draftKey=${kn}
            context=${ae}
            statusText=${Nt}
            canCancel=${$a}
            onCancel=${Oa}
          />
        `}
      </div>
      <${a2}
        open=${wa}
        onClose=${()=>tn(!1)}
      />
    </div>
  `}function _h(){let{threadsState:e,gatewayStatus:t}=ba(),{threadId:a}=st(),n=he(),r=Ae(),s=r.state?.composerDraft||"",i=a||null;p.default.useEffect(()=>{i&&i!==e.activeThreadId?e.setActiveThreadId(i):i||e.setActiveThreadId(null)},[i]);let o=p.default.useCallback((l,c={})=>{if(!l){e.setActiveThreadId(null),n("/chat",c);return}e.setActiveThreadId(l),n(`/chat/${l}`,c)},[e,n]);return u`
    <${sS}
      threads=${e.threads}
      activeThreadId=${i}
      onSelectThread=${o}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function iS(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?ri(e,t):"",model:e?kc(e,t):""}}function oS({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:l}){let[c,d]=p.default.useState(()=>iS(e,a)),[m,f]=p.default.useState(""),[h,x]=p.default.useState([]),[y,$]=p.default.useState(null),[g,v]=p.default.useState(""),b=p.default.useRef(!!e);p.default.useEffect(()=>{n&&(d(iS(e,a)),f(""),x([]),$(null),v(""),b.current=!!e)},[n,e,a]);let w=e?.builtin===!0,S=e&&!e.builtin,E=p.default.useCallback((P,k)=>{d(U=>{let Y={...U,[P]:k};return P==="name"&&!b.current&&(Y.id=Nw(k)),Y})},[]),N=p.default.useCallback(()=>!w&&(!c.name.trim()||!c.id.trim())?l("llm.fieldsRequired"):!w&&!_w(c.id.trim())?l("llm.invalidId"):!S&&!w&&t.includes(c.id.trim())?l("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,w,S,l]),T=p.default.useCallback(async()=>{let P=N();if(P){$({tone:"error",text:P});return}v("save");try{await s({form:c,apiKey:m,provider:e}),r()}catch(k){$({tone:"error",text:k.message})}finally{v("")}},[m,c,r,s,e,N]),L=p.default.useCallback(async()=>{if(!c.model.trim()){$({tone:"error",text:l("llm.modelRequired")});return}v("test");try{let P=await i(Zp(e,c,m,a));$({tone:P.ok?"success":"error",text:P.message})}catch(P){$({tone:"error",text:P.message})}finally{v("")}},[m,a,c,i,e,l]),O=p.default.useCallback(async()=>{if((w?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){$({tone:"error",text:l("llm.baseUrlRequired")});return}v("models");try{let k=await o(Zp(e,c,m,a));if(!k.ok||!Array.isArray(k.models)||!k.models.length)$({tone:"error",text:k.message||l("llm.modelsFetchFailed")});else{x(k.models);let U=Rw(c.model,k.models);U!==null&&E("model",U),$({tone:"success",text:l("llm.modelsFetched",{count:k.models.length})})}}catch(k){$({tone:"error",text:k.message})}finally{v("")}},[m,a,c,w,o,e,l,E]);return{form:c,apiKey:m,models:h,message:y,busy:g,isBuiltin:w,isEditing:S,setApiKey:f,update:E,submit:T,runTest:L,fetchModels:O,markIdEdited:()=>{b.current=!0}}}function Jc({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let l=C(),c=oS({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:l});if(!n)return null;let{form:d,apiKey:m,models:f,message:h,busy:x,isBuiltin:y,isEditing:$}=c,g=y?l("llm.configureProvider",{name:e.name||e.id}):l($?"llm.editProvider":"llm.newProvider");return u`
    <${mi} open=${n} onClose=${r} title=${g} size="lg">
      <${fi} className="space-y-4">
        ${!y&&u`
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${l("llm.providerName")}
              <${Mt} value=${d.name} onChange=${v=>c.update("name",v.target.value)} />
            </label>
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${l("llm.providerId")}
              <${Mt}
                value=${d.id}
                disabled=${$}
                onChange=${v=>{c.markIdEdited(),c.update("id",v.target.value)}}
              />
            </label>
          </div>
          <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
            ${l("llm.adapter")}
            <${mh} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${Wp.map(v=>u`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
            <//>
          </label>
        `}

        ${y&&u`
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            ${nl(e.adapter)}
          </div>
        `}

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${l("llm.baseUrl")}
          <${Mt} value=${d.baseUrl} placeholder=${e?.base_url||""} onChange=${v=>c.update("baseUrl",v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${l("llm.apiKey")}
          <${Mt} type="password" value=${m} placeholder=${l("llm.apiKeyPlaceholder")} onChange=${v=>c.setApiKey(v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${l("llm.defaultModel")}
          <div className="flex items-stretch gap-2">
            <${Mt} value=${d.model} onChange=${v=>c.update("model",v.target.value)} />
            <${A} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${x!==""} onClick=${c.fetchModels}>
              ${l(x==="models"?"llm.fetchingModels":"llm.fetchModels")}
            <//>
          </div>
        </label>

        ${f.length>0&&u`
          <${mh} value=${d.model} onChange=${v=>c.update("model",v.target.value)}>
            ${f.map(v=>u`<option key=${v} value=${v}>${v}</option>`)}
          <//>
        `}

        ${h&&u`
          <div className=${h.tone==="error"?"text-sm text-red-200":"text-sm text-mint"} role="status">
            ${h.text}
          </div>
        `}
      <//>
      <${pi}>
        <${A} type="button" variant="secondary" disabled=${x!==""} onClick=${c.runTest}>
          ${l(x==="test"?"llm.testing":"llm.testConnection")}
        <//>
        <${A} type="button" variant="ghost" disabled=${x!==""} onClick=${r}>${l("common.cancel")}<//>
        <${A} type="button" disabled=${x!==""} onClick=${c.submit}>
          ${l(x==="save"?"common.saving":"common.save")}
        <//>
      <//>
    <//>
  `}function Xc({login:e}){let t=C(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return u`
    ${a&&u`<div
      data-testid="llm-provider-nearai-waiting"
      className="text-center text-xs text-[var(--v2-text-muted)]"
    >
      ${t("onboarding.nearaiWaiting")}
    </div>`}
    ${n&&u`<div data-testid="llm-provider-nearai-error" className="text-center text-xs text-red-300">${n}</div>`}

    ${i&&u`<div
      data-testid="llm-provider-codex-code"
      className="mx-auto max-w-md rounded-lg border border-[var(--v2-border)] bg-[var(--v2-surface-raised)] p-4 text-center"
    >
      <div className="text-xs text-[var(--v2-text-muted)]">
        ${t("onboarding.codexEnterCode")}
      </div>
      <div className="mt-2 font-mono text-2xl font-semibold tracking-[0.3em] text-[var(--v2-text-strong)]">
        ${i.userCode}
      </div>
      <a
        className="mt-2 inline-block text-xs underline hover:text-[var(--v2-text-strong)]"
        href=${i.verificationUri}
        target="_blank"
        rel="noopener noreferrer"
      >
        ${i.verificationUri}
      </a>
    </div>`}
    ${r&&u`<div
      data-testid="llm-provider-codex-waiting"
      className="text-center text-xs text-[var(--v2-text-muted)]"
    >
      ${t("onboarding.codexWaiting")}
    </div>`}
    ${s&&u`<div data-testid="llm-provider-codex-error" className="text-center text-xs text-red-300">${s}</div>`}
  `}function nD(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function Wc({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=si({settings:e,gatewayStatus:t}),[s,i]=p.default.useState(null),[o,l]=p.default.useState(!1),[c,d]=p.default.useState(null),m=p.default.useRef(null),f=p.default.useCallback((g,v)=>{m.current&&window.clearTimeout(m.current),d({tone:g,text:v}),m.current=window.setTimeout(()=>d(null),3500)},[]);p.default.useEffect(()=>()=>{m.current&&window.clearTimeout(m.current)},[]);let h=p.default.useCallback((g=null)=>{i(g),l(!0)},[]),x=p.default.useCallback(async g=>{try{await r.setActiveProvider(g),f("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(h(g),f("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):f("error",v.message)}},[h,r,f,n]),y=p.default.useCallback(async({form:g,apiKey:v,provider:b})=>{if(b?.builtin){await r.saveBuiltinProvider({provider:b,form:g,apiKey:v}),f("success",n("llm.providerConfigured",{name:b.name||b.id}));return}let w=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:b});f("success",n(b?"llm.providerUpdated":"llm.providerAdded",{name:w.name||w.id}))},[r,f,n]),$=p.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),f("success",n("llm.providerDeleted"))}catch(v){f("error",v.message)}},[r,f,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>nD(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:h,closeDialog:()=>l(!1),handleUse:x,handleSave:y,handleDelete:$}}var rD=3e5;function sD(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function iD(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function oD(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=l=>{let c=l.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},rD);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var lD=3e5,uD=9e5,cD=2e3;async function lS(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,cD)),(await Rc().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function Zc({onSuccess:e}={}){let t=C(),a=W(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),[o,l]=p.default.useState(!1),[c,d]=p.default.useState(""),[m,f]=p.default.useState(null),h=p.default.useCallback(()=>{i(""),d(""),f(null)},[]),x=p.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),y=p.default.useCallback(async v=>{if(h(),sD()){i(t("onboarding.nearaiLocalSso"));return}let b=window.open("about:blank","_blank");if(!b){i(t("onboarding.nearaiFailed"));return}try{b.opener=null}catch{}r(!0);try{let{auth_url:w}=await aw({provider:v,origin:window.location.origin});b.location.href=w;let S=await lS("nearai",lD,b);if(S==="active"){await x();return}b.close(),i(t(S==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{b.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,h,t]),$=p.default.useCallback(async()=>{h(),r(!0);try{let v=iD(),b=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!b){i(t("onboarding.nearaiFailed"));return}b.opener=null;let w=await oD(b,v);if(!w){i(t("onboarding.nearaiFailed"));return}await nw({account_id:w.accountId,public_key:w.publicKey,signature:w.signature,message:w.message,recipient:w.recipient,nonce:w.nonce}),await x()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,h,t]),g=p.default.useCallback(async()=>{h();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}l(!0);try{let{user_code:b,verification_uri:w}=await rw();f({userCode:b,verificationUri:w}),v&&(v.location.href=w);let S=await lS("openai_codex",uD,v);if(S==="active"){await x();return}v&&v.close(),d(t(S==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{l(!1)}},[x,h,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:m,startNearai:y,startNearaiWallet:$,startCodex:g}}var uS="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",dD="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",mD="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",fD="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",pD={nearai:{color:"#00ec97",path:dD},openai_codex:{color:"#10a37f",path:uS},openai:{color:"#10a37f",path:uS},anthropic:{color:"#d97757",path:mD},ollama:{color:null,path:fD}};function cS({id:e,name:t}){let a=pD[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return u`
      <span
        className=${`${n} bg-[var(--v2-surface-muted)] text-sm font-semibold text-[var(--v2-text-strong)]`}
      >
        ${s}
      </span>
    `}let r=a.color?{background:`color-mix(in srgb, ${a.color} 16%, transparent)`,color:a.color}:{background:"var(--v2-surface-muted)",color:"var(--v2-text-strong)"};return u`
    <span className=${n} style=${r}>
      <svg viewBox="0 0 24 24" className="h-5 w-5" fill="currentColor" aria-hidden="true">
        <path d=${a.path} />
      </svg>
    </span>
  `}var hD=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function vD({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=p.default.useState(!1),o=p.default.useRef(null),l=t||a.nearaiBusy;p.default.useEffect(()=>{if(!s)return;let d=f=>{o.current&&!o.current.contains(f.target)&&i(!1)},m=f=>{f.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",m),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",m)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return u`
    <div ref=${o} className="relative shrink-0">
      <${A}
        type="button"
        variant="primary"
        size="sm"
        className="gap-1.5"
        aria-haspopup="true"
        aria-expanded=${s?"true":"false"}
        disabled=${l}
        onClick=${()=>i(d=>!d)}
      >
        ${n("onboarding.setUp")}
        <${M} name="chevron" className="h-3.5 w-3.5" />
      <//>
      ${s&&u`
        <div
          role="menu"
          className="absolute right-0 top-10 z-20 min-w-[176px] rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-1 shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]"
        >
          ${c.map(d=>u`
              <button
                key=${d.id}
                type="button"
                role="menuitem"
                disabled=${d.disabled}
                onClick=${()=>{i(!1),d.run()}}
                className="flex w-full items-center rounded-[7px] px-2.5 py-1.5 text-left text-[13px] text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)] disabled:cursor-not-allowed disabled:opacity-50"
              >
                ${d.label}
              </button>
            `)}
        </div>
      `}
    </div>
  `}function gD({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let l=s(e.nameKey),c;return e.auth==="nearai"?c=u`<${vD} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=u`
      <${A} type="button" variant="secondary" size="sm" disabled=${r.codexBusy} onClick=${r.startCodex}>
        ${s("onboarding.signIn")}
      <//>
    `:a?c=u`<${A} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>i(t)}>
      ${s("llm.use")}
    <//>`:c=u`<${A} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>o(t)}>
      ${s("onboarding.setUp")}
    <//>`,u`
    <${te} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-4">
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <${cS} id=${e.id} name=${l} />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">${l}</span>
            ${a&&u`<${z} tone="positive" label=${s("onboarding.ready")} size="sm" />`}
          </div>
          <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">${s(e.descKey)}</div>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap gap-2 sm:justify-end">${c}</div>
    <//>
  `}function dS(){let{isAdmin:e=!1,isChecking:t=!1}=ba();return t?null:e?u`<${yD} />`:u`<${it} to="/chat" replace />`}function yD(){let e=C(),t=he(),a=W(),{gatewayStatus:n}=ba(),r=Wc({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=hD.map(m=>({entry:m,provider:s.providers.find(f=>f.id===m.id)})).filter(m=>m.provider),o=p.default.useCallback(()=>t("/chat"),[t]),l=Zc({onSuccess:o}),c=p.default.useCallback(async m=>{let f=m.active_model||m.default_model||"";await al({provider_id:m.id,model:f}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=p.default.useCallback(async({form:m,apiKey:f,provider:h})=>{await r.handleSave({form:m,apiKey:f,provider:h});let x=h?.id||m.id.trim(),y=m.model?.trim()||h?.default_model||"";await al({provider_id:x,model:y}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?u`
      <div className="grid h-full place-items-center text-sm text-[var(--v2-text-muted)]">
        ${e("common.loading")}
      </div>
    `:u`
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex min-h-full max-w-2xl flex-col justify-center gap-6 p-6">
        <div className="text-center">
          <h1 className="text-2xl font-semibold text-[var(--v2-text-strong)]">
            ${e("onboarding.title")}
          </h1>
          <p className="mt-2 text-sm text-[var(--v2-text-muted)]">${e("onboarding.subtitle")}</p>
        </div>

        <div className="flex flex-col gap-3">
          ${i.map(({entry:m,provider:f})=>u`
              <${gD}
                key=${m.id}
                entry=${m}
                provider=${f}
                configured=${Hr(f,s.builtinOverrides)}
                isBusy=${s.isBusy}
                login=${l}
                t=${e}
                onUse=${c}
                onSetUp=${r.openDialog}
              />
            `)}
        </div>

        <${Xc} login=${l} />

        <div className="text-center text-xs text-[var(--v2-text-muted)]">
          ${e("onboarding.moreInSettings")}${" "}
          <button
            type="button"
            className="underline hover:text-[var(--v2-text-strong)]"
            onClick=${()=>t("/settings/inference")}
          >
            ${e("nav.settings")}
          </button>
        </div>
      </div>

      <${Jc}
        open=${r.isDialogOpen}
        provider=${r.dialogProvider}
        allProviderIds=${r.allProviderIds}
        builtinOverrides=${s.builtinOverrides}
        onClose=${r.closeDialog}
        onSave=${d}
        onTest=${s.testConnection}
        onListModels=${s.listModels}
      />
    </div>
  `}function q({children:e,className:t="",...a}){return u`<${te} className=${t} ...${a}>${e}<//>`}function Ze({label:e,value:t,tone:a="muted",badgeLabel:n,detail:r,showDivider:s=!0,className:i="",valueClassName:o="text-[1.75rem] md:text-[2rem]"}){return u`
    <div
      className=${G("px-1 py-4",s&&"border-t border-[var(--v2-panel-border)]",i)}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div
            className="font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]"
          >
            ${e}
          </div>
          <div
            className=${G("mt-3 truncate font-medium tracking-[-0.05em] text-[var(--v2-text-strong)]",o)}
          >
            ${t}
          </div>
          ${r&&u`<div className="mt-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            ${r}
          </div>`}
        </div>
        <${z} tone=${a} label=${n??a} />
      </div>
    </div>
  `}function mS({items:e}){return u`
    <div className="grid gap-3">
      ${e.map((t,a)=>u`
          <div
            key=${t.title}
            className="grid grid-cols-[2.75rem_minmax(0,1fr)] gap-4 border-t border-[var(--v2-panel-border)] py-4"
            style=${{"--index":a}}
          >
            <div className="font-mono text-xs text-[var(--v2-accent-text)]">
              ${String(a+1).padStart(2,"0")}
            </div>
            <div className="min-w-0">
              <div className="text-sm font-semibold text-[var(--v2-text-strong)]">
                ${t.title}
              </div>
              <div className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
                ${t.description}
              </div>
            </div>
          </div>
        `)}
    </div>
  `}function xe({title:e,description:t,children:a,boxed:n=!0}){let r=u`
    <div className="max-w-xl">
      <h2
        className="text-[1.35rem] font-medium tracking-[-0.03em] text-[var(--v2-text-strong)] md:text-[1.6rem]"
      >
        ${e}
      </h2>
      <p className="mt-3 text-[15px] leading-relaxed text-[var(--v2-text-muted)]">
        ${t}
      </p>
      ${a&&u`<div className="mt-5">${a}</div>`}
    </div>
  `;return n?u`<${te} padding="lg">${r}<//>`:u`<div className="py-8">${r}</div>`}var fS={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function Za({result:e,onDismiss:t}){return e?u`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",fS[e.type]||fS.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}var pS="",bD={workspace:"home"};function ed(e){return bD[e]||e}function cl(e){return[...e||[]].sort((t,a)=>t.is_dir!==a.is_dir?t.is_dir?-1:1:t.name.localeCompare(a.name,void 0,{sensitivity:"base"}))}function vi(e){return e?e.split("/").filter(Boolean):[]}function td(e){return e?`/workspace/${vi(e).map(encodeURIComponent).join("/")}`:"/workspace"}function Rh(e){let t=vi(e);return t.pop(),t.join("/")}function hS(e){return/\.mdx?$/i.test(e||"")}function ad({path:e,onNavigate:t}){let a=C(),n=vi(e),r="";return u`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button
        type="button"
        onClick=${()=>t("/workspace")}
        className="text-signal hover:underline"
      >
        ${a("workspace.breadcrumbRoot")}
      </button>
      ${n.map((s,i)=>{r=r?`${r}/${s}`:s;let o=r,l=i===0?ed(s):s;return u`
          <span key=${o} className="text-iron-400">/</span>
          <button
            key=${`${o}-button`}
            type="button"
            onClick=${()=>t(td(o))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${l}
          </button>
        `})}
    </div>
  `}function xD(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function vS({path:e,entries:t,isLoading:a,filter:n,onOpen:r,onNavigate:s}){let i=C();if(a)return u`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;let o=(t||[]).filter(f=>!xD(f.path)),l=String(n||"").trim().toLowerCase(),c=l?o.filter(f=>f.name.toLowerCase().includes(l)):o,d=cl(c),m;return o.length?d.length?m=u`
      <div className="divide-y divide-white/[0.06]">
        ${d.map(f=>u`
          <button
            key=${f.path}
            type="button"
            onClick=${()=>r(f.path)}
            className="flex w-full items-center gap-3 px-4 py-2.5 text-left text-sm text-iron-200 hover:bg-white/[0.05] hover:text-white"
          >
            <span className=${["w-4 text-center text-xs",f.is_dir?"text-signal":"text-iron-400"].join(" ")}>
              ${f.is_dir?"\u25A1":"\xB7"}
            </span>
            <span className="min-w-0 truncate ${f.is_dir?"font-semibold":""}">${f.name}</span>
          </button>
        `)}
      </div>
    `:m=u`<div className="px-4 py-10 text-center text-sm text-iron-300">${i("workspace.noMatches")}</div>`:m=u`<div className="px-4 py-10 text-center text-sm text-iron-300">${i("workspace.emptyDir")}</div>`,u`
    <${q} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${ad} path=${e} onNavigate=${s} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">${m}</div>
    <//>
  `}var nd="/api/webchat/v2/fs",$D=1024*1024,wD=8*1024*1024;function gS(e){let t=String(e||"").split("/").filter(Boolean);return{mount:t.shift()||"",path:t.join("/")}}function SD(e,t){return t?`${e}/${t}`:e}function ND(e){let t=String(e||"").toLowerCase();return t.startsWith("text/")||t==="application/json"||t==="application/javascript"||t==="application/xml"||t.endsWith("+json")||t.endsWith("+xml")}function _D(e){return String(e||"").toLowerCase().startsWith("image/")}function RD(e){let t=String(e||"").toLowerCase();return t.startsWith("audio/")||t.startsWith("video/")||t.startsWith("font/")||t==="application/pdf"||t==="application/zip"||t==="application/gzip"}function kD(e){if(e.subarray(0,Math.min(e.length,8192)).indexOf(0)!==-1)return!0;try{return new TextDecoder("utf-8",{fatal:!0}).decode(e),!1}catch{return!0}}function CD(e,t){let a=new URL(`${nd}/content`,window.location.origin);return a.searchParams.set("mount",e),a.searchParams.set("path",t),a.pathname+a.search}async function ED(){return(await K(`${nd}/mounts`))?.mounts||[]}async function gi(e=""){if(!e)return{entries:(await ED()).map(o=>({name:ed(o.mount),path:o.mount,is_dir:!0}))};let{mount:t,path:a}=gS(e),n=new URL(`${nd}/list`,window.location.origin);return n.searchParams.set("mount",t),a&&n.searchParams.set("path",a),{entries:((await K(n.pathname+n.search))?.entries||[]).map(i=>({name:i.name,path:SD(t,i.path),is_dir:i.kind==="directory"}))}}async function yS(e){let{mount:t,path:a}=gS(e);if(!t||!a)return{kind:"directory",path:e};let n=new URL(`${nd}/stat`,window.location.origin);n.searchParams.set("mount",t),n.searchParams.set("path",a);let s=(await K(n.pathname+n.search))?.stat||{},i=s.mime_type||"application/octet-stream",o=Number(s.size_bytes||0),l=CD(t,a),c={path:e,mime:i,size_bytes:o,download_path:l};if(s.kind&&s.kind!=="file")return{...c,kind:"directory"};if(_D(i)){if(o>wD)return{...c,kind:"binary"};let h=await $c(l);return{...c,kind:"image",image_data_url:h}}if(RD(i)||o>$D)return{...c,kind:"binary"};let d=await Ca(l),m=new Uint8Array(await d.arrayBuffer());if(!ND(i)&&kD(m))return{...c,kind:"binary"};let f=new TextDecoder("utf-8").decode(m);return{...c,kind:"text",content:f}}function bS(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function TD(e,t,a){let n=String(t||"").trim().toLowerCase(),r=(e||[]).filter(s=>!bS(s.path)).filter(s=>!n||s.name.toLowerCase().includes(n)?!0:s.is_dir&&a.has(s.path));return cl(r)}function xS({entry:e,depth:t,selectedPath:a,expandedPaths:n,filter:r,onToggleDirectory:s,onSelectFile:i}){let o=C(),l=n.has(e.path),c=H({queryKey:["workspace-list",e.path],queryFn:()=>gi(e.path),enabled:e.is_dir&&l});if(e.is_dir){let d=TD(c.data?.entries,r,n);return u`
      <div>
        <button
          type="button"
          onClick=${()=>{i(e.path),s(e.path)}}
          className=${["flex min-h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm hover:bg-white/[0.05] hover:text-white",a===e.path?"bg-signal/10 text-signal":"text-iron-200"].join(" ")}
          style=${{paddingLeft:`${8+t*16}px`}}
          aria-expanded=${l}
        >
          <span className=${["w-3 text-[10px]",l?"rotate-90":""].join(" ")}>></span>
          <span className="min-w-0 truncate font-semibold">${e.name}</span>
        </button>
        ${l&&u`
          <div className="space-y-1">
            ${c.isLoading?u`<div className="px-4 py-2 text-xs text-iron-400">${o("workspace.loading")}</div>`:c.isError?u`<div className="px-4 py-2 text-xs text-red-300">${o("workspace.unableOpenDirectory")}</div>`:d.map(m=>u`
                  <${xS}
                    key=${m.path}
                    entry=${m}
                    depth=${t+1}
                    selectedPath=${a}
                    expandedPaths=${n}
                    filter=${r}
                    onToggleDirectory=${s}
                    onSelectFile=${i}
                  />
                `)}
          </div>
        `}
      </div>
    `}return u`
    <button
      type="button"
      onClick=${()=>i(e.path)}
      className=${["flex min-h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm",a===e.path?"bg-signal/10 text-signal":"text-iron-300 hover:bg-white/[0.05] hover:text-white"].join(" ")}
      style=${{paddingLeft:`${24+t*16}px`}}
    >
      <span className="min-w-0 truncate">${e.name}</span>
    </button>
  `}function $S({entries:e,selectedPath:t,expandedPaths:a,filter:n,onToggleDirectory:r,onSelectFile:s,isLoading:i}){let o=C();if(i)return u`<div className="space-y-2 p-3">${[1,2,3,4].map(c=>u`<div key=${c} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let l=cl(e.filter(c=>!bS(c.path)));return l.length?u`
    <div className="space-y-1 p-2">
      ${l.map(c=>u`
        <${xS}
          key=${c.path}
          entry=${c}
          depth=${0}
          selectedPath=${t}
          expandedPaths=${a}
          filter=${n}
          onToggleDirectory=${r}
          onSelectFile=${s}
        />
      `)}
    </div>
  `:u`<div className="px-4 py-8 text-sm text-iron-300">${o("workspace.noFiles")}</div>`}function wS({rootEntries:e,selectedPath:t,expandedPaths:a,filter:n,onFilterChange:r,isLoadingTree:s,onToggleDirectory:i,onSelectFile:o}){let l=C();return u`
    <${q} className="flex min-h-[420px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="border-b border-white/10 p-3">
        <input
          value=${n}
          onInput=${c=>r(c.target.value)}
          placeholder=${l("workspace.filterPlaceholder")}
          className="h-9 w-full rounded-md border border-white/10 bg-iron-950/80 px-3 text-sm text-white outline-none placeholder:text-iron-400 focus:border-signal/45"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <${$S}
          entries=${e}
          selectedPath=${t}
          expandedPaths=${a}
          filter=${n}
          onToggleDirectory=${i}
          onSelectFile=${o}
          isLoading=${s}
        />
      </div>
    <//>
  `}function SS(e){return vi(e).pop()||"download"}function AD({path:e,file:t}){let a=C();return t.kind==="image"?u`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${t.image_data_url}
          alt=${SS(e)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `:t.kind==="text"?u`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${hS(e)?u`<${sa} content=${t.content} className="max-w-4xl text-base leading-7" />`:u`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
      </div>
    `:u`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${a("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `}function NS({path:e,file:t,isLoading:a,onNavigate:n}){let r=C(),[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(t?.download_path){i(!0);try{let c=await Ca(t.download_path);di(c,SS(e))}catch{}finally{i(!1)}}},[t,e]);if(a)return u`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;if(!t||t.kind==="directory")return u`
      <${xe}
        title=${r("workspace.pickFileTitle")}
        description=${r("workspace.pickFileDesc")}
      />
    `;let l=r("workspace.fileMeta",{mime:t.mime||"application/octet-stream",size:Number(t.size_bytes||0)});return u`
    <${q} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${ad} path=${e} onNavigate=${n} />
        <div className="flex items-center gap-2">
          <${z} tone="muted" label=${l} />
          <${A}
            variant="secondary"
            size="sm"
            onClick=${o}
            disabled=${s}
          >${r("workspace.download")}<//>
        </div>
      </div>

      <${AD} path=${e} file=${t} />

      ${Rh(e)&&u`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${r("workspace.parent",{path:Rh(e)})}
        </div>
      `}
    <//>
  `}function _S(e){let t=C(),a=W(),[n,r]=p.default.useState(new Set),[s,i]=p.default.useState(""),[o,l]=p.default.useState(null),c=H({queryKey:["workspace-list",""],queryFn:()=>gi("")}),d=H({queryKey:["workspace-file",e],queryFn:()=>yS(e),enabled:!!e}),m=e===""||d.data?.kind==="directory",f=H({queryKey:["workspace-list",e],queryFn:()=>gi(e),enabled:m});p.default.useEffect(()=>{l(null)},[e]);let h=p.default.useCallback(y=>a.fetchQuery({queryKey:["workspace-list",y],queryFn:()=>gi(y)}),[a]),x=p.default.useCallback(async y=>{let $=new Set(n);if($.has(y)){$.delete(y),r($);return}$.add(y),r($);try{await h(y)}catch(g){l({type:"error",message:g.message||t("workspace.unableOpenDirectory")})}},[n,h,t]);return{rootEntries:c.data?.entries||[],file:d.data||null,selectionIsDirectory:m,currentEntries:f.data?.entries||[],expandedPaths:n,filter:s,setFilter:i,result:o,clearResult:()=>l(null),isLoadingTree:c.isLoading,isLoadingFile:d.isLoading,isLoadingListing:f.isLoading,isFetching:c.isFetching||d.isFetching||f.isFetching,error:c.error||d.error||f.error||null,loadDirectory:h,toggleDirectory:x,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function kh(){let e=C(),t=he(),n=st()["*"]||pS,r=_S(n),s=p.default.useCallback(i=>{t(td(i))},[t]);return u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="flex h-full min-h-0 flex-col space-y-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1 className="text-lg font-semibold text-white">${e("workspace.title")}</h1>
                <${z} tone="muted" label=${e("workspace.readOnly")} />
              </div>
              <p className="mt-0.5 text-sm text-iron-400">${e("workspace.subtitle")}</p>
            </div>
            <${A}
              variant="secondary"
              size="sm"
              onClick=${r.refresh}
              disabled=${r.isFetching}
            >
              ${r.isFetching?e("workspace.refreshing"):e("workspace.refresh")}
            <//>
          </div>

          ${r.error&&u`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${r.error.message}
            </div>
          `}
          <${Za}
            result=${r.result}
            onDismiss=${r.clearResult}
          />

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <${wS}
              rootEntries=${r.rootEntries}
              selectedPath=${n}
              expandedPaths=${r.expandedPaths}
              filter=${r.filter}
              onFilterChange=${r.setFilter}
              isLoadingTree=${r.isLoadingTree}
              onToggleDirectory=${r.toggleDirectory}
              onSelectFile=${s}
            />
            ${r.selectionIsDirectory?u`
                  <${vS}
                    path=${n}
                    entries=${r.currentEntries}
                    isLoading=${r.isLoadingListing}
                    filter=${r.filter}
                    onOpen=${s}
                    onNavigate=${t}
                  />
                `:u`
                  <${NS}
                    path=${n}
                    file=${r.file}
                    isLoading=${r.isLoadingFile}
                    onNavigate=${t}
                  />
                `}
          </div>
        </div>
      </div>
    </div>
  `}function RS(e){if(!e)return null;let t=e.metadata&&typeof e.metadata=="object"&&!Array.isArray(e.metadata)?e.metadata:{};return{id:e.project_id,name:e.name,description:e.description,goals:Array.isArray(t.goals)?t.goals:[],icon:e.icon||null,color:e.color||null,state:e.state,role:e.role,metadata:t,created_at:e.created_at,updated_at:e.updated_at,health:e.state==="archived"?"muted":"green"}}async function kS(){let t=((await e$({limit:200}))?.projects||[]).map(RS);return{attention:[],projects:t}}async function CS(e){if(!e)return null;let t=await t$({projectId:e});return RS(t?.project)}function ES(e){return Promise.resolve({missions:[],todo:!0})}function TS(e){return Promise.resolve({threads:[],todo:!0})}function AS(e){return Promise.resolve({widgets:[],todo:!0})}function DS(e){return Promise.resolve(null)}function MS(e){return Promise.resolve(null)}function OS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function LS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function PS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function jS(){let e=W(),t=H({queryKey:["projects-overview"],queryFn:kS,refetchInterval:5e3}),a=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function US(e){let t=W(),a=!!e,n=H({queryKey:["project-detail",e],queryFn:()=>CS(e),enabled:a,refetchInterval:a?7e3:!1}),r=H({queryKey:["project-missions",e],queryFn:()=>ES(e),enabled:a,refetchInterval:a?5e3:!1}),s=H({queryKey:["project-threads",e],queryFn:()=>TS(e),enabled:a,refetchInterval:a?4e3:!1}),i=H({queryKey:["project-widgets",e],queryFn:()=>AS(e),enabled:a,refetchInterval:a?15e3:!1}),o=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function FS({projectId:e,missionId:t,threadId:a}){let n=W(),[r,s]=p.default.useState(null),i=H({queryKey:["project-mission-detail",t],queryFn:()=>DS(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=H({queryKey:["project-thread-detail",a],queryFn:()=>MS(a),enabled:!!a,refetchInterval:a?4e3:!1}),l=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=V({mutationFn:({targetMissionId:f})=>OS(f),onSuccess:f=>{s({type:"success",message:f?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to fire mission"})}}),d=V({mutationFn:({targetMissionId:f})=>LS(f),onSuccess:()=>{s({type:"success",message:"Mission paused."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to pause mission"})}}),m=V({mutationFn:({targetMissionId:f})=>PS(f),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending}}function rd(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function sd(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function BS(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function zS(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function DD(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function qS(e){let t=DD(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function IS(e){let t=e?.projects||[],a=t.reduce((o,l)=>o+Number(l.cost_today_usd||0),0),n=t.reduce((o,l)=>o+Number(l.active_missions||0),0),r=t.reduce((o,l)=>o+Number(l.threads_today||0),0),s=t.reduce((o,l)=>o+Number(l.pending_gates||0),0),i=t.reduce((o,l)=>o+Number(l.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function dl(e,t){return`${e} ${t}${e===1?"":"s"}`}var MD={projects:"muted",attention:"warning",spend:"success"};function HS({overview:e}){let t=IS(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:sd(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return u`
    <${q} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>u`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${z} tone=${MD[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function OD(e){return e?.type==="failure"?"danger":"warning"}function LD(e){return e?.type==="failure"?"failure":"gate"}function KS({items:e,onOpenItem:t}){return e?.length?u`
    <${q} className="overflow-hidden border-amber-300/10 p-0">
      <div className="border-b border-amber-300/10 px-5 py-4 sm:px-6">
        <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-copper">Needs attention</div>
        <p className="mt-2 max-w-[70ch] text-sm leading-6 text-iron-200">
          Operator-visible gates and recent failures across your project workspace.
        </p>
      </div>
      <div className="grid gap-3 p-4 sm:p-5 xl:grid-cols-2">
        ${e.map(a=>u`
          <button
            key=${`${a.project_id}-${a.thread_id||a.message}`}
            onClick=${()=>t(a)}
            className="group rounded-2xl border border-white/10 bg-iron-950/55 p-4 text-left hover:border-signal/30 hover:bg-white/[0.05]"
          >
            <div className="flex items-start justify-between gap-3">
              <div>
                <div className="text-sm font-semibold text-white">${a.project_name}</div>
                <div className="mt-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  ${a.thread_id?`Thread ${String(a.thread_id).slice(0,8)}`:"Project"}
                </div>
              </div>
              <${z} tone=${OD(a)} label=${LD(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function PD({project:e,onOpen:t,t:a}){return u`
    <article
      data-testid="project-card"
      data-project-id=${e.id}
      onClick=${()=>t(e.id)}
      role="button"
      tabIndex=${0}
      onKeyDown=${n=>{n.currentTarget===n.target&&(n.key==="Enter"||n.key===" ")&&(n.preventDefault(),t(e.id))}}
      className="group cursor-pointer rounded-xl border border-iron-700 bg-iron-800/60 p-5 transition hover:border-signal/30 hover:bg-iron-800/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/40"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="truncate font-serif text-2xl font-semibold tracking-[-0.03em] text-iron-100">${e.name}</h3>
          <p className="mt-2 line-clamp-3 text-sm leading-6 text-iron-300">
            ${e.description||a("projects.noDescription")}
          </p>
        </div>
        <${z} tone=${BS(e.health)} label=${e.health||"unknown"} />
      </div>

      ${e.goals?.length?u`
            <div className="mt-4 flex flex-wrap gap-2">
              ${e.goals.slice(0,3).map((n,r)=>u`
                <span key=${r} className="rounded-full border border-iron-700 px-3 py-1 text-xs text-iron-200">
                  ${n}
                </span>
              `)}
            </div>
          `:null}

      <div className="mt-5 grid gap-3 sm:grid-cols-2">
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${a("projects.card.runtime")}</div>
          <div className="mt-2 text-sm text-iron-100">
            ${a("projects.card.threadsToday",{count:dl(e.threads_today||0,"thread")})}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${a("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">${dl(e.pending_gates||0,"gate")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${a("projects.card.failures24h",{count:dl(e.failures_24h||0,"failure")})}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>${a("projects.card.spendToday",{value:sd(e.cost_today_usd||0)})}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${rd(e.last_activity)}</div>
        </div>
        <${A}
          data-testid="project-open-workspace"
          variant="secondary"
          onClick=${n=>{n.stopPropagation(),t(e.id)}}
        >${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function jD({project:e,onOpen:t,t:a}){return u`
    <${q}
      onClick=${()=>t(e.id)}
      role="button"
      tabIndex=${0}
      onKeyDown=${n=>{n.currentTarget===n.target&&(n.key==="Enter"||n.key===" ")&&(n.preventDefault(),t(e.id))}}
      className="cursor-pointer overflow-hidden p-5 transition hover:border-signal/30 sm:p-6"
    >
      <div className="flex flex-col gap-6 xl:flex-row xl:items-end xl:justify-between">
        <div className="max-w-3xl">
          <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-signal">${a("projects.general.label")}</div>
          <h2 className="mt-3 font-serif text-4xl font-semibold tracking-[-0.04em] text-iron-100">${a("projects.general.title")}</h2>
          <p className="mt-3 text-sm leading-6 text-iron-200">
            ${a("projects.general.desc")}
          </p>
        </div>
        <div className="flex flex-wrap gap-3">
          <div className="rounded-2xl border border-iron-700 bg-iron-950/55 px-4 py-3 text-sm text-iron-200">
            ${dl(e.threads_today||0,"thread")} today
          </div>
          <${A}
            variant="secondary"
            onClick=${n=>{n.stopPropagation(),t(e.id)}}
          >${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function QS({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=C(),l=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return!e.length&&t>0?u`
      <${xe}
        title=${o("projects.empty.noMatchTitle")}
        description=${o("projects.empty.noMatchDesc")}
      />
    `:e.length?u`
    <div data-testid="projects-grid" className="space-y-5">
      ${l&&u`<${jD} project=${l} onOpen=${r} t=${o} />`}

      <${q} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${o("projects.explorer")}</div>
            <h2 className="mt-2 font-serif text-3xl font-semibold tracking-[-0.04em] text-iron-100">${o("projects.scoped.title")}</h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${o("projects.scoped.desc")}
            </p>
          </div>
          <div className="flex gap-2">
            <input
              data-testid="projects-search-input"
              value=${a}
              onInput=${d=>n(d.target.value)}
              placeholder=${o("projects.searchPlaceholder")}
              className="h-11 min-w-[220px] rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            />
            <${A} onClick=${s}>${o(i?"projects.preparingChat":"projects.newProject")}<//>
          </div>
        </div>
      <//>

      ${c.length?u`<div className="grid gap-4 xl:grid-cols-2 2xl:grid-cols-3">
            ${c.map(d=>u`<${PD} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
          </div>`:u`
            <${xe}
              title=${o("projects.scoped.onlyGeneralTitle")}
              description=${o("projects.scoped.onlyGeneralDesc")}
            >
              <${A} onClick=${s}>${o(i?"projects.preparingChat":"projects.startProject")}<//>
            <//>
          `}
    </div>
  `:u`
      <${xe}
        title=${o("projects.empty.noneTitle")}
        description=${o("projects.empty.noneDesc")}
      >
        <${A} onClick=${s}>${o("projects.createFromChat")}<//>
      <//>
    `}function VS({threads:e,selectedThreadId:t,onSelectThread:a,onNewConversation:n,isStartingConversation:r}){let s=[...e].sort((i,o)=>new Date(o.updated_at||o.created_at)-new Date(i.updated_at||i.created_at));return u`
    <${q} className="p-4 sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Conversations</div>
          <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">Project conversations</h2>
        </div>
        ${n&&u`
          <${A} onClick=${n} disabled=${r}>
            ${r?"Starting\u2026":"New conversation"}
          <//>
        `}
      </div>

      <div className="mt-5 space-y-3">
        ${s.length?s.slice(0,18).map(i=>{let o=qS(i);return u`
                <button
                  key=${i.id}
                  onClick=${()=>a(i.id)}
                  className=${["w-full rounded-[20px] border p-4 text-left",t===i.id?"border-signal/35 bg-signal/10":"border-white/10 bg-white/[0.025] hover:border-signal/25 hover:bg-white/[0.045]"].join(" ")}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="truncate text-base font-semibold text-white">${o.title}</div>
                      <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-400">${o.subtitle}</div>
                      ${o.brief?u`<p className="mt-3 line-clamp-2 text-sm leading-6 text-iron-300">${o.brief}</p>`:null}
                    </div>
                    <${z} tone=${zS(i.state)} label=${i.state} />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                    <span>${i.step_count||0} steps</span>
                    <span>${i.total_tokens||0} tokens</span>
                    <span>${rd(i.updated_at||i.created_at)}</span>
                  </div>
                </button>
              `}):u`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                No project threads yet. When an automation runs or scoped chat work happens inside this project, activity will appear here.
              </div>
            `}
      </div>
    <//>
  `}var UD="/workspace";function FD(e){let t=a=>a.kind==="directory"?0:1;return[...e].sort((a,n)=>t(a)-t(n)||a.name.localeCompare(n.name,void 0,{sensitivity:"base"}))}function BD(e){return e?String(e).replace(/^\/workspace\/?/,"").split("/").filter(Boolean):[]}function GS({threadId:e}){let t=C(),[a,n]=p.default.useState(void 0),[r,s]=p.default.useState(null),i=H({queryKey:["project-files",e||"",a||""],queryFn:()=>Vx({threadId:e,path:a}),enabled:!!e}),o=p.default.useMemo(()=>FD(i.data?.entries||[]),[i.data]),l=p.default.useCallback(async m=>{if(m.kind==="directory"){s(null),n(m.path);return}try{s(null);let f=await Ca(xc({threadId:e,path:m.path})),h=URL.createObjectURL(f),x=document.createElement("a");x.href=h,x.download=m.name,document.body.appendChild(x),x.click(),x.remove(),URL.revokeObjectURL(h)}catch(f){s(f?.message||"Unable to download file")}},[e]),c=BD(a),d=u`
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
          ${"Files"}
        </div>
        <${z} tone="muted" label=${t("workspace.readOnly")} />
      </div>
      <${A}
        variant="secondary"
        size="sm"
        onClick=${()=>i.refetch()}
        disabled=${!e||i.isFetching}
      >
        ${i.isFetching?t("workspace.refreshing"):t("workspace.refresh")}
      <//>
    </div>
  `;return e?u`
    <${q} className="p-4 sm:p-5">
      ${d}

      <div className="mt-3 flex min-w-0 flex-wrap items-center gap-1.5 font-mono text-xs text-iron-400">
        <button
          type="button"
          onClick=${()=>n(void 0)}
          className="text-signal hover:underline"
        >
          ${"workspace"}
        </button>
        ${c.map((m,f)=>{let h=`${UD}/${c.slice(0,f+1).join("/")}`;return u`
            <span key=${h} className="text-iron-500">/</span>
            <button
              key=${`${h}-button`}
              type="button"
              onClick=${()=>n(h)}
              className="max-w-[160px] truncate text-signal hover:underline"
            >
              ${m}
            </button>
          `})}
      </div>

      ${r&&u`
        <div className="mt-3 rounded-xl border border-red-400/30 bg-red-500/10 px-3 py-2 text-xs text-red-200">
          ${r}
        </div>
      `}
      ${i.error&&u`
        <div className="mt-3 rounded-xl border border-red-400/30 bg-red-500/10 px-3 py-2 text-xs text-red-200">
          ${i.error.message}
        </div>
      `}

      <div className="mt-3 space-y-1">
        ${i.isLoading?[1,2,3,4].map(m=>u`<div key=${m} className="v2-skeleton h-9 rounded-[12px]" />`):o.length?o.map(m=>u`
                <button
                  key=${m.path}
                  type="button"
                  onClick=${()=>l(m)}
                  className="flex w-full items-center gap-3 rounded-[12px] border border-transparent px-3 py-2 text-left hover:border-white/10 hover:bg-white/[0.04]"
                >
                  <${M}
                    name=${m.kind==="directory"?"folder":"file"}
                    className="h-4 w-4 shrink-0 text-iron-300"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm text-white">${m.name}</span>
                  ${m.kind==="directory"?u`<${M} name="chevron" className="h-3.5 w-3.5 shrink-0 -rotate-90 text-iron-500" />`:u`<${M} name="download" className="h-3.5 w-3.5 shrink-0 text-iron-500" />`}
                </button>
              `):u`
              <div className="rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                ${"This folder is empty."}
              </div>
            `}
      </div>
    <//>
  `:u`
      <${q} className="p-4 sm:p-5">
        ${d}
        <div className="mt-4 rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
          ${"No files yet \u2014 they appear once a thread has run in this project."}
        </div>
      <//>
    `}function zD(e){return[...e||[]].sort((a,n)=>new Date(n.updated_at||n.created_at)-new Date(a.updated_at||a.created_at))[0]?.id||null}function YS({project:e,threads:t,selectedThreadId:a,onSelectThread:n,onNewConversation:r,isStartingConversation:s}){let i=zD(t);return u`
    <div
      data-testid="project-workspace"
      data-project-id=${e.id}
      className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]"
    >
      <div className="space-y-5">
        <div className="min-w-0">
          <h2
            data-testid="project-workspace-title"
            className="text-2xl font-semibold tracking-tight text-white"
          >
            ${e.name}
          </h2>
          ${e.description?u`<p className="mt-1 text-sm leading-6 text-iron-300">${e.description}</p>`:null}
        </div>

        <${VS}
          threads=${t}
          selectedThreadId=${a}
          onSelectThread=${n}
          onNewConversation=${r}
          isStartingConversation=${s}
        />
      </div>

      <${GS} threadId=${i} />
    </div>
  `}function ml(){let e=C(),t=he(),{threadsState:a}=ba(),{projectId:n=null,threadId:r=null}=st(),[s,i]=p.default.useState(""),[o,l]=p.default.useState(null),c=jS(),d=US(n),m=FS({projectId:n,threadId:r}),f=p.default.useMemo(()=>{let N=s.trim().toLowerCase();return N?c.overview.projects.filter(T=>[T.name,T.description,...T.goals||[]].some(L=>String(L||"").toLowerCase().includes(N))):c.overview.projects},[c.overview.projects,s]),h=p.default.useMemo(()=>c.overview.projects.find(N=>N.id===n)||null,[c.overview.projects,n]),x=p.default.useCallback(()=>{c.invalidate(),d.invalidate()},[c,d]),y=p.default.useCallback(N=>{t(`/projects/${N}`)},[t]),$=p.default.useCallback(N=>{if(N.thread_id){t(`/projects/${N.project_id}/threads/${N.thread_id}`);return}t(`/projects/${N.project_id}`)},[t]),g=p.default.useCallback(async()=>{let N=null;l(null);try{N=await a.createThread()}catch(T){l({type:"error",message:T.message||e("projects.chatAutoFail")})}t("/chat",{state:{composerDraft:e("projects.creationDraft"),threadId:N}})},[t,a]),v=p.default.useCallback(N=>{t(`/projects/${n}/threads/${N}`)},[t,n]),b=p.default.useCallback(async()=>{l(null);try{let N=await a.createThread(n);t("/chat",{state:{threadId:N}}),d.invalidate()}catch(N){l({type:"error",message:N.message||e("projects.chatAutoFail")})}},[t,a,n,d,e]),w=p.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),S=u`
    ${n&&u`<${A} variant="ghost" onClick=${()=>t("/projects")}>${e("projects.allProjects")}<//>`}
  `,E=null;return n?d.isLoading?E=u`
        <div className="space-y-4">
          ${[1,2,3].map(N=>u`<div key=${N} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `:d.error||!d.project&&!h?E=u`
        <${xe}
          title=${e("projects.unavailable")}
          description=${d.error?.message||e("projects.unavailableDesc")}
        >
          <${A} variant="secondary" onClick=${()=>t("/projects")}>${e("projects.returnToProjects")}<//>
        <//>
      `:E=u`
        <${YS}
          project=${d.project||h}
          threads=${d.threads}
          selectedThreadId=${r}
          onSelectThread=${v}
          onNewConversation=${b}
          isStartingConversation=${a.isCreating}
        />
      `:E=c.isLoading?u`
          <div className="space-y-4">
            ${[1,2,3].map(N=>u`<div key=${N} className="v2-skeleton h-40 rounded-[20px]" />`)}
          </div>
        `:u`
          <${QS}
            projects=${f}
            totalProjects=${c.overview.projects.length}
            search=${s}
            onSearchChange=${i}
            onOpenProject=${y}
            onCreateProject=${g}
            isPreparingChat=${a.isCreating}
          />
        `,u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <div className="flex flex-wrap justify-end gap-2">
            ${S}
          </div>
          ${c.error&&u`
            <div className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
              ${c.error.message}
            </div>
          `}
          <${Za} result=${o} onDismiss=${()=>l(null)} />
          <${Za} result=${m.actionResult} onDismiss=${m.clearActionResult} />
          ${!n&&u`
            <${HS} overview=${c.overview} />
            <${KS} items=${c.overview.attention} onOpenItem=${$} />
          `}
          ${E}
        </div>
      </div>
    </div>
  `}function fl(e,t={}){if(!e)return"Not scheduled";let a=new Date(e);return Number.isNaN(a.getTime())?"Not scheduled":a.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t})}function pl(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function JS(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function XS(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);if(r!==0)return r;let s=new Date(n.updated_at||0).getTime(),i=new Date(a.updated_at||0).getTime();return(Number.isNaN(s)?0:s)-(Number.isNaN(i)?0:i)})}function id({label:e,value:t}){return u`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function qD({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=C();return e.status==="Active"?u`
      <${A} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.fireNow")}<//>
      <${A} variant="secondary" onClick=${()=>n(e.id)} disabled=${t}>${s("missions.action.pause")}<//>
    `:e.status==="Paused"?u`
      <${A} onClick=${()=>r(e.id)} disabled=${t}>${s("missions.action.resume")}<//>
      <${A} variant="secondary" onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runOnce")}<//>
    `:u`<${A} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runAgain")}<//>`}function WS({mission:e,isLoading:t,error:a,isBusy:n,onFire:r,onPause:s,onResume:i,onOpenProject:o,onOpenThread:l}){let c=C();return t?u`
      <div className="space-y-4">
        ${[1,2,3].map(d=>u`<div key=${d} className="v2-skeleton h-36 rounded-xl" />`)}
      </div>
    `:a||!e?u`
      <${xe}
        title=${c("missions.unavailable")}
        description=${a?.message||c("missions.unavailableDesc")}
      />
    `:u`
    <div className="space-y-4">
      <${q} className="p-4 sm:p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.dossier")}</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
            ${e.project&&u`
              <button
                type="button"
                onClick=${()=>o(e.project.id)}
                className="mt-2 text-sm text-signal underline-offset-4 hover:underline"
              >
                ${e.project.name}
              </button>
            `}
          </div>
          <${z} tone=${pl(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${id} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${id} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${id} label=${c("missions.meta.nextFire")} value=${fl(e.next_fire_at)} />
          <${id} label=${c("missions.meta.updated")} value=${fl(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${qD}
            mission=${e}
            isBusy=${n}
            onFire=${r}
            onPause=${s}
            onResume=${i}
          />
        </div>
      <//>

      <${q} className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.brief")}</div>
        <div className="mt-4 text-sm leading-6 text-iron-200">
          <${sa} content=${e.goal||c("missions.noGoal")} />
        </div>
      <//>

      ${e.current_focus&&u`
        <${q} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${sa} content=${e.current_focus} />
          </div>
        <//>
      `}

      ${e.success_criteria&&u`
        <${q} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${sa} content=${e.success_criteria} />
          </div>
        <//>
      `}

      ${e.threads?.length?u`
        <${q} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.spawnedThreads")}</div>
          <div className="mt-4 space-y-3">
            ${e.threads.map(d=>u`
              <button
                key=${d.id}
                type="button"
                onClick=${()=>l(d)}
                className="w-full rounded-xl border border-white/8 bg-iron-950/60 p-4 text-left hover:border-signal/30 hover:bg-white/[0.05]"
              >
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0 truncate text-sm font-semibold text-white">${d.title||d.goal}</div>
                  <${z} tone=${pl(d.state==="Running"?"Active":d.state==="Failed"?"Failed":"Completed")} label=${d.state} />
                </div>
              </button>
            `)}
          </div>
        <//>
      `:null}
    </div>
  `}function ID(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function ZS({value:e,onChange:t,children:a,label:n}){return u`
    <label className="min-w-[160px] flex-1 sm:flex-none">
      <span className="sr-only">${n}</span>
      <select
        value=${e}
        onChange=${r=>t(r.target.value)}
        className="v2-select h-11 w-full rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none focus:border-signal/40"
      >
        ${a}
      </select>
    </label>
  `}function HD({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=C(),s=t===e.id;return u`
    <div
      className=${["w-full rounded-xl border p-4 text-left",s?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80"].join(" ")}
    >
      <button type="button" onClick=${()=>a(e.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${e.name}</div>
              <${z} tone=${pl(e.status)} label=${e.status} />
            </div>
            <p className="mt-2 line-clamp-2 text-sm leading-6 text-iron-300">${e.goal||r("missions.noGoal")}</p>
          </div>
          <div className="shrink-0 text-right font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
            <div>${e.cadence_description||e.cadence_type||"manual"}</div>
            <div className="mt-1">${r("missions.threadCount",{count:e.thread_count||0})}</div>
          </div>
        </div>
      </button>

      <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-iron-700 pt-3">
        <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
          ${r("missions.updated",{value:fl(e.updated_at)})}
        </span>
        <${A}
          variant="ghost"
          onClick=${i=>{i.stopPropagation(),n(e.project.id)}}
        >
          ${e.project.name}
        <//>
      </div>
    </div>
  `}function Ch({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:l,projectOptions:c,onSelectMission:d,onOpenProject:m}){let f=C(),h=ID(f);return u`
    <${q} className="p-4 sm:p-5">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${f("missions.title")}</div>
          <h1 className="mt-2 text-3xl font-semibold tracking-tight text-iron-100">${f("missions.subtitle")}</h1>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
            ${f("missions.summary",{missions:t,projects:c.length})}
          </p>
        </div>
      </div>

      <div className="mt-5 flex flex-wrap gap-3">
        <input
          value=${n}
          onChange=${x=>r(x.target.value)}
          placeholder=${f("missions.searchPlaceholder")}
          className="h-11 min-w-[220px] flex-1 rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/40"
        />
        <${ZS} value=${s} onChange=${i} label=${f("missions.filter.status")}>
          ${h.map(x=>u`<option key=${x.value} value=${x.value}>${x.label}<//>`)}
        <//>
        <${ZS} value=${o} onChange=${l} label=${f("missions.filter.project")}>
          <option value="all">${f("missions.filter.allProjects")}</option>
          ${c.map(x=>u`<option key=${x.id} value=${x.id}>${x.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(x=>u`
              <${HD}
                key=${x.id}
                mission=${x}
                selectedMissionId=${a}
                onSelectMission=${d}
                onOpenProject=${m}
              />
            `):u`
              <${xe}
                title=${f("missions.emptyTitle")}
                description=${f("missions.emptyDesc")}
                boxed=${!1}
              />
            `}
      </div>
    <//>
  `}function KD(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function eN({summary:e}){let t=C(),a=KD(t);return u`
    <${q} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>u`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${z} tone=${n.tone} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${e[n.key]||0}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">
              ${n.key==="total"?t("missions.summary.completedFailed",{completed:e.completed||0,failed:e.failed||0}):t("missions.summary.acrossProjects")}
            </p>
          </div>
        `)}
      </div>
    <//>
  `}function tN(){return Promise.resolve({projects:[],todo:!0})}function aN({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function nN(e){return Promise.resolve(null)}function rN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function sN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function iN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function oN(e){let t=H({queryKey:["mission-detail",e],queryFn:()=>nN(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function QD(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function lN(){let e=W(),[t,a]=p.default.useState(null),n=H({queryKey:["projects-overview"],queryFn:tN,refetchInterval:7e3}),r=n.data?.projects||[],s=Od({queries:r.map(f=>({queryKey:["missions","project",f.id],queryFn:()=>aN({projectId:f.id}),refetchInterval:5e3,select:h=>h?.missions||[]}))}),i=s.flatMap((f,h)=>{let x=r[h];return(f.data||[]).map(y=>QD(y,x))}),o=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),l=(f,h)=>({mutationFn:({missionId:x})=>f(x),onSuccess:()=>{a({type:"success",message:h}),o()},onError:x=>{a({type:"error",message:x.message||"Unable to update mission"})}}),c=V(l(rN,"Mission fired and a run was queued.")),d=V(l(sN,"Mission paused.")),m=V(l(iN,"Mission resumed."));return{projects:r,missions:i,summary:JS(i),isLoading:n.isLoading||s.some(f=>f.isLoading),isRefreshing:n.isFetching||s.some(f=>f.isFetching),error:n.error||s.find(f=>f.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending,invalidate:o}}function Eh(){let e=C(),t=he(),{missionId:a=null}=st(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,l]=p.default.useState("all"),c=lN(),d=oN(a),m=p.default.useMemo(()=>{let g=n.trim().toLowerCase();return XS(c.missions).filter(v=>{let b=!g||[v.name,v.goal,v.project?.name].some(E=>String(E||"").toLowerCase().includes(g)),w=s==="all"||v.status===s,S=o==="all"||v.project?.id===o;return b&&w&&S})},[c.missions,o,n,s]),f=p.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),h=d.mission?{...f,...d.mission,project:f?.project||null}:f,x=p.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),y=p.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),$=a?u`
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <${Ch}
            missions=${m}
            totalMissions=${c.missions.length}
            selectedMissionId=${a}
            search=${n}
            onSearchChange=${r}
            statusFilter=${s}
            onStatusFilterChange=${i}
            projectFilter=${o}
            onProjectFilterChange=${l}
            projectOptions=${c.projects}
            onSelectMission=${g=>t(`/missions/${g}`)}
            onOpenProject=${g=>t(`/projects/${g}`)}
          />
          <${WS}
            mission=${h}
            isLoading=${d.isLoading}
            error=${d.error}
            isBusy=${c.isBusy}
            onFire=${g=>y(c.fireMission,g)}
            onPause=${g=>y(c.pauseMission,g)}
            onResume=${g=>y(c.resumeMission,g)}
            onOpenProject=${g=>t(`/projects/${g}`)}
            onOpenThread=${x}
          />
        </div>
      `:u`
        <${Ch}
          missions=${m}
          totalMissions=${c.missions.length}
          selectedMissionId=${a}
          search=${n}
          onSearchChange=${r}
          statusFilter=${s}
          onStatusFilterChange=${i}
          projectFilter=${o}
          onProjectFilterChange=${l}
          projectOptions=${c.projects}
          onSelectMission=${g=>t(`/missions/${g}`)}
          onOpenProject=${g=>t(`/projects/${g}`)}
        />
      `;return u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          ${a&&u`<div className="flex flex-wrap justify-end gap-2">
            <${A}
              variant="ghost"
              onClick=${()=>t("/missions")}
              >${e("missions.allMissions")}<//
            >
          </div>`}

          ${c.error&&u`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${c.error.message}
            </div>
          `}

          <${Za}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${eN} summary=${c.summary} />

          ${c.isLoading?u`
                <div className="space-y-4">
                  ${[1,2,3].map(g=>u`<div
                        key=${g}
                        className="v2-skeleton h-32 rounded-xl"
                      />`)}
                </div>
              `:$}
        </div>
      </div>
    </div>
  `}var uN=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],VD=new Set(["pending","in_progress"]),cN=new Set(["failed","interrupted","stuck","cancelled"]);function lr(e){return e?String(e).replace(/_/g," "):"unknown"}function yi(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":cN.has(e)?"danger":"muted":"muted"}function GD(e){return VD.has(e)}function od(e){return GD(e?.state)}function dN(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":cN.has(e.state):!1}function Qr(e,t=8){return e?String(e).slice(0,t):"unknown"}function ia(e,t={}){if(!e)return"Not available";let a=new Date(e);return Number.isNaN(a.getTime())?"Not available":a.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t})}function mN(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function Th(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${ia(e.started_at)}`:null].filter(Boolean).join(" / ")}var YD=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function fN(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function JD({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?u`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${t==="tool_use"?a.tool_name||"Tool call":a.tool_name||"Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${fN(t==="tool_use"?a.input:a.output||a.error||a)}</pre>
      </details>
    `:t==="message"?u`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a.role||"assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${a.content||""}</div>
      </div>
    `:u`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t.replace(/_/g," ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${a.message||a.status||fN(a)}</div>
    </div>
  `}function pN({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=C(),[s,i]=p.default.useState("all"),[o,l]=p.default.useState(""),[c,d]=p.default.useState(!0),m=p.default.useRef(null),f=p.default.useMemo(()=>s==="all"?t:t.filter(x=>x.event_type===s),[t,s]);p.default.useEffect(()=>{c&&m.current&&(m.current.scrollTop=m.current.scrollHeight)},[c,f.length]);let h=p.default.useCallback(async(x=!1)=>{let y=o.trim();if(!(!y&&!x))try{await a({content:y||"(done)",done:x}),l("")}catch{}},[o,a]);return u`
    <${q} className="p-5 sm:p-6">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Event stream</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Job activity</h3>
          <p className="mt-2 text-sm leading-6 text-iron-300">Persisted events are refreshed automatically so operators can follow tool calls, prompts, and worker output.</p>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <select
            value=${s}
            onChange=${x=>i(x.target.value)}
            className="v2-select h-10 rounded-md border border-white/10 bg-iron-950/90 px-3 text-sm text-white outline-none focus:border-signal/45"
          >
            ${YD.map(x=>u`<option key=${x.value} value=${x.value}>${x.label}</option>`)}
          </select>
          <label className="flex items-center gap-2 text-sm text-iron-300">
            <input type="checkbox" checked=${c} onChange=${x=>d(x.target.checked)} />
            Auto-scroll
          </label>
        </div>
      </div>

      <div ref=${m} className="mt-5 max-h-[56vh] space-y-3 overflow-y-auto rounded-[18px] border border-white/10 bg-iron-950/78 p-4">
        ${f.length?f.map(x=>u`
              <div key=${x.id||`${x.event_type}-${x.created_at}`}>
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${ia(x.created_at)}</div>
                <${JD} event=${x} />
              </div>
            `):u`
              <${xe}
                title=${r("job.noActivityTitle")}
                description=${r("job.noActivityDesc")}
              />
            `}
      </div>

      ${e.can_prompt&&u`
        <div className="mt-5 grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto_auto]">
          <input
            value=${o}
            onInput=${x=>l(x.target.value)}
            onKeyDown=${x=>{x.key==="Enter"&&!x.shiftKey&&(x.preventDefault(),h(!1))}}
            placeholder=${r("job.followupPlaceholder")}
            className="h-11 rounded-md border border-white/10 bg-iron-950/90 px-3 text-sm text-white outline-none focus:border-signal/45"
          />
          <${A} variant="secondary" disabled=${n} onClick=${()=>h(!0)}>${r("common.done")}<//>
          <${A} variant="primary" disabled=${n} onClick=${()=>h(!1)}>${r("common.send")}<//>
        </div>
      `}
    <//>
  `}function hN({job:e,activeTab:t,onTabChange:a,onBack:n,onCancel:r,onRestart:s,isBusy:i,children:o}){return u`
    <div className="space-y-5">
      <${q} className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick=${n} className="text-sm text-signal hover:text-white">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-semibold tracking-tight text-white">${e.title||"Untitled job"}</h2>
              <${z} tone=${yi(e.state)} label=${lr(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${Qr(e.id)}</span>
              <span>created ${ia(e.created_at)}</span>
              ${Th(e)&&u`<span>${Th(e)}</span>`}
            </div>
          </div>

          <div className="flex flex-wrap gap-2">
            ${e.browse_url&&u`
              <a
                href=${e.browse_url}
                target="_blank"
                rel="noreferrer noopener"
                className="v2-button inline-flex h-10 items-center rounded-md border border-white/12 bg-white/[0.04] px-4 text-sm font-semibold text-iron-100 hover:border-signal/45 hover:bg-signal/10"
              >
                Browse files
              </a>
            `}
            ${od(e)&&u`
              <${A} variant="secondary" disabled=${i} onClick=${()=>r(e.id)}>Cancel<//>
            `}
            ${dN(e)&&u`
              <${A} variant="primary" disabled=${i} onClick=${()=>s(e.id)}>Restart<//>
            `}
          </div>
        </div>
      <//>

      <div className="flex flex-wrap gap-2">
        ${uN.map(l=>u`
          <button
            key=${l.id}
            onClick=${()=>a(l.id)}
            className=${["v2-button rounded-full border px-4 py-2 text-sm",t===l.id?"border-signal/35 bg-signal/12 text-white":"border-white/10 bg-white/[0.03] text-iron-300 hover:border-signal/25 hover:text-white"].join(" ")}
          >
            ${l.label}
          </button>
        `)}
      </div>

      ${o}
    </div>
  `}function vN({nodes:e,depth:t=0,selectedPath:a,expandingPath:n,onToggleDirectory:r,onSelectPath:s}){return u`
    ${e.map(i=>u`
      <div key=${i.path}>
        <button
          onClick=${()=>i.isDir?r(i.path):s(i.path)}
          className=${["flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm",a===i.path?"bg-signal/10 text-white":"text-iron-200 hover:bg-white/[0.05]"].join(" ")}
          style=${{paddingLeft:`${t*18+12}px`}}
        >
          <span className="w-4 text-center text-iron-300">
            ${i.isDir?n===i.path?"...":i.expanded?"v":">":"\xB7"}
          </span>
          <span className=${i.isDir?"font-medium":""}>${i.name}</span>
        </button>
        ${i.isDir&&i.expanded&&i.children?.length?u`<${vN}
              nodes=${i.children}
              depth=${t+1}
              selectedPath=${a}
              expandingPath=${n}
              onToggleDirectory=${r}
              onSelectPath=${s}
            />`:null}
      </div>
    `)}
  `}function gN({canBrowse:e,tree:t,selectedPath:a,selectedFile:n,fileError:r,isLoadingTree:s,isLoadingFile:i,expandingPath:o,treeError:l,onToggleDirectory:c,onSelectPath:d}){return e?u`
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <${q} className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          ${l&&u`<div className="mx-2 mb-3 rounded-md border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${l}</div>`}
          ${s?u`<div className="space-y-2 px-2">${[1,2,3,4].map(m=>u`<div key=${m} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?u`
                  <${vN}
                    nodes=${t}
                    selectedPath=${a}
                    expandingPath=${o}
                    onToggleDirectory=${c}
                    onSelectPath=${d}
                  />
                `:u`<div className="px-2 py-6 text-sm text-iron-300">No files were recorded for this workspace.</div>`}
        </div>
      <//>

      <${q} className="min-h-[440px] p-5 sm:p-6">
        <div className="border-b border-white/10 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">File preview</div>
          <p className="mt-2 break-all text-sm leading-6 text-iron-300">${n?.path||a||"Select a file from the tree to inspect its contents."}</p>
        </div>

        ${r&&!i?u`<div className="mt-5 rounded-md border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">${r}</div>`:i?u`<div className="mt-5 space-y-3">${[1,2,3,4,5].map(m=>u`<div key=${m} className="v2-skeleton h-4 rounded" />`)}</div>`:n?u`<pre className="mt-5 max-h-[60vh] overflow-auto whitespace-pre-wrap rounded-[18px] border border-white/10 bg-iron-950/90 p-4 font-mono text-xs leading-6 text-iron-100">${n.content}</pre>`:u`
                <${xe}
                  title="No file selected"
                  description="Pick a concrete file from the workspace tree to render it here."
                />
              `}
      <//>
    </div>
  `:u`
      <${xe}
        title="No project workspace"
        description="File browsing is only available for sandbox jobs that produced a mounted project directory."
      />
    `}function bi({label:e,value:t}){return u`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function yN({job:e}){let t=(e.transitions||[]).map(a=>({title:`${lr(a.from)} -> ${lr(a.to)}`,description:[ia(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return u`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${q} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${z} tone=${yi(e.state)} label=${lr(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${bi} label="Created" value=${ia(e.created_at)} />
          <${bi} label="Started" value=${ia(e.started_at)} />
          <${bi} label="Completed" value=${ia(e.completed_at)} />
          <${bi} label="Duration" value=${mN(e.elapsed_secs)} />
          <${bi} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${bi} label="Mode" value=${e.job_mode||"Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${q} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${e.description?u`<${sa} content=${e.description} className="mt-4 text-sm leading-7 text-iron-200" />`:u`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
        <//>

        ${t.length?u`
              <${q} className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Transitions</div>
                <h3 className="mt-2 text-xl font-semibold text-white">State timeline</h3>
                <div className="mt-3">
                  <${mS} items=${t} />
                </div>
              <//>
            `:u`
              <${xe}
                title="No state history yet"
                description="Transitions appear here once the job advances or records a recovery event."
              />
            `}
      </div>
    </div>
  `}function bN({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:l,isBusy:c,isRefreshing:d}){let m=C(),f=[{value:"all",label:m("jobs.list.filter.all")},{value:"pending",label:m("jobs.list.filter.pending")},{value:"in_progress",label:m("jobs.list.filter.inProgress")},{value:"completed",label:m("jobs.list.filter.completed")},{value:"failed",label:m("jobs.list.filter.failed")},{value:"stuck",label:m("jobs.list.filter.stuck")}];if(!e.length){let h=!!n.trim()||s!=="all";return u`
      <${xe}
        title=${m(t&&h?"jobs.list.empty.noMatchTitle":"jobs.list.empty.noJobsTitle")}
        description=${m(t&&h?"jobs.list.empty.noMatchDesc":"jobs.list.empty.noJobsDesc")}
      />
    `}return u`
    <div className="space-y-5">
      <${q} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${m("jobs.list.explorer")}</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-iron-100">${m("jobs.list.queueTitle")}</h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${m("jobs.list.queueDesc")}
            </p>
          </div>
          <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
            <span>${m("jobs.list.visible",{count:e.length})}</span>
            <span>/</span>
            <span>${m(d?"jobs.list.state.refreshing":"jobs.list.state.live")}</span>
          </div>
        </div>

        <div className="mt-5 grid gap-3 md:grid-cols-[minmax(0,1fr)_220px]">
          <input
            value=${n}
            onInput=${h=>r(h.target.value)}
            placeholder=${m("jobs.list.searchPlaceholder")}
            className="h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          />
          <select
            value=${s}
            onChange=${h=>i(h.target.value)}
            className="v2-select h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          >
            ${f.map(h=>u`<option key=${h.value} value=${h.value}>${h.label}</option>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(h=>u`
          <article
            key=${h.id}
            className=${["group flex flex-col gap-4 rounded-[18px] border p-5",a===h.id?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
          >
            <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
              <button onClick=${()=>o(h.id)} className="min-w-0 text-left">
                <div className="flex flex-wrap items-center gap-2">
                  <h3 className="truncate text-lg font-semibold text-iron-100">${h.title||m("jobs.list.untitled")}</h3>
                  <${z} tone=${yi(h.state)} label=${lr(h.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${Qr(h.id)}</span>
                  <span>${m("jobs.list.created",{value:ia(h.created_at)})}</span>
                  ${h.started_at&&u`<span>${m("jobs.list.started",{value:ia(h.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${od(h)&&u`
                  <${A}
                    variant="secondary"
                    className="h-9 px-3 text-xs"
                    disabled=${c}
                    onClick=${()=>l(h.id)}
                  >
                    ${m("jobs.action.cancel")}
                  <//>
                `}
                <${A} variant="ghost" className="h-9 px-3 text-xs" onClick=${()=>o(h.id)}>${m("jobs.action.open")}<//>
              </div>
            </div>
          </article>
        `)}
      </div>
    </div>
  `}var XD=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function xN({summary:e}){return u`
    <${q} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${XD.map(t=>u`
          <div
            key=${t.key}
            className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
          >
            <${Ze}
              label=${t.label}
              value=${e?.[t.key]??0}
              tone=${t.tone}
              detail=${t.detail}
              showDivider=${!1}
              className="px-0 py-0"
            />
          </div>
        `)}
      </div>
    <//>
  `}function $N(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function wN(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function SN(e){return Promise.resolve(null)}function NN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function _N(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function RN(e){return Promise.resolve({events:[],todo:!0})}function kN(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function Ah(e,t=""){return Promise.resolve({entries:[],todo:!0})}function CN(e,t){return Promise.resolve({content:"",todo:!0})}function EN(e){let t=W(),[a,n]=p.default.useState(null),r=H({queryKey:["job-detail",e],queryFn:()=>SN(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=H({queryKey:["job-events",e],queryFn:()=>RN(e),enabled:!!e,refetchInterval:e?2500:!1}),i=V({mutationFn:({content:o,done:l})=>kN(e,{content:o,done:l}),onSuccess:(o,{done:l})=>{n({type:"success",message:l?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return p.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function TN(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function AN(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=AN(a.children,t);if(n)return n}}return null}function ld(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:ld(n.children,t,a)}:n)}function DN(e){let[t,a]=p.default.useState([]),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState(""),c=!!(e?.project_dir&&e?.id),d=H({queryKey:["job-files-root",e?.id],queryFn:()=>Ah(e.id,""),enabled:c}),m=H({queryKey:["job-file",e?.id,n],queryFn:()=>CN(e.id,n),enabled:!!(c&&n)});p.default.useEffect(()=>{a([]),r(""),i(""),l("")},[e?.id]),p.default.useEffect(()=>{d.data?.entries?(a(TN(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let f=p.default.useCallback(async h=>{let x=AN(t,h);if(!(!x||!e?.id)){if(x.expanded){a(y=>ld(y,h,$=>({...$,expanded:!1})));return}if(x.loaded){a(y=>ld(y,h,$=>({...$,expanded:!0})));return}l(h);try{let y=await Ah(e.id,h);a($=>ld($,h,g=>({...g,expanded:!0,loaded:!0,children:TN(y.entries)}))),i("")}catch(y){i(y.message||"Unable to open folder")}finally{l("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:m.data||null,fileError:m.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:m.isLoading||m.isFetching,expandingPath:o,treeError:s,toggleDirectory:f}}function MN(){let e=W(),[t,a]=p.default.useState(null),n=H({queryKey:["jobs-summary"],queryFn:wN,refetchInterval:5e3}),r=H({queryKey:["jobs"],queryFn:$N,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=V({mutationFn:({jobId:l})=>NN(l),onSuccess:(l,{jobId:c})=>{a({type:"success",message:`Job ${Qr(c)} cancelled`}),s()},onError:l=>{a({type:"error",message:l.message||"Unable to cancel job"})}}),o=V({mutationFn:({jobId:l})=>_N(l),onSuccess:l=>{a({type:"success",message:`Restart queued as ${Qr(l?.new_job_id)}`}),s()},onError:l=>{a({type:"error",message:l.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function ON({result:e,onDismiss:t}){let a=C();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return u`
    <div
      className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",n[e.type]||n.info].join(" ")}
    >
      <span className="min-w-0 flex-1">${e.message}</span>
      <button
        onClick=${t}
        className="shrink-0 opacity-70 hover:opacity-100"
      >
        ${a("jobs.dismiss")}
      </button>
    </div>
  `}function Dh(){let e=C(),t=he(),{jobId:a=null}=st(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,l]=p.default.useState(a?"activity":"overview"),c=MN(),d=EN(a),m=DN(d.job);p.default.useEffect(()=>{l(a?"activity":"overview")},[a]);let f=p.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(b=>{let w=!v||b.title.toLowerCase().includes(v)||b.id.toLowerCase().includes(v),S=s==="all"||b.state===s;return w&&S})},[c.jobs,n,s]),h=p.default.useCallback(v=>t(`/jobs/${v}`),[t]),x=p.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),y=p.default.useCallback(async v=>{try{let b=await c.restartJob({jobId:v});b?.new_job_id&&t(`/jobs/${b.new_job_id}`)}catch{}},[c,t]),$=u`
    ${a&&u`<${A} variant="ghost" onClick=${()=>t("/jobs")}
      >${e("jobs.allJobs")}<//
    >`}
  `,g=null;if(a)if(d.isLoading)g=u`
        <div className="space-y-4">
          ${[1,2,3].map(v=>u`<div key=${v} className="v2-skeleton h-32 rounded-[18px]" />`)}
        </div>
      `;else if(d.error||!d.job)g=u`
        <${xe}
          title=${e("jobs.unavailable")}
          description=${d.error?.message||e("jobs.unavailableDesc")}
        >
          <${A} variant="secondary" onClick=${()=>t("/jobs")}
            >${e("jobs.returnToJobs")}<//
          >
        <//>
      `;else{let v={overview:u`<${yN} job=${d.job} />`,activity:u`
          <${pN}
            job=${d.job}
            events=${d.events}
            onSendPrompt=${d.sendPrompt}
            isSendingPrompt=${d.isSendingPrompt}
          />
        `,files:u`
          <${gN}
            canBrowse=${m.canBrowse}
            tree=${m.tree}
            selectedPath=${m.selectedPath}
            selectedFile=${m.selectedFile}
            fileError=${m.fileError}
            isLoadingTree=${m.isLoadingTree}
            isLoadingFile=${m.isLoadingFile}
            expandingPath=${m.expandingPath}
            treeError=${m.treeError}
            onToggleDirectory=${m.toggleDirectory}
            onSelectPath=${m.selectPath}
          />
        `};g=u`
        <${hN}
          job=${d.job}
          activeTab=${o}
          onTabChange=${l}
          onBack=${()=>t("/jobs")}
          onCancel=${x}
          onRestart=${y}
          isBusy=${c.isBusy}
        >
          ${v[o]||v.overview}
        <//>
      `}else g=c.isLoading?u`
          <div className="space-y-4">
            ${[1,2,3].map(v=>u`<div
                  key=${v}
                  className="v2-skeleton h-28 rounded-[18px]"
                />`)}
          </div>
        `:u`
          <${bN}
            jobs=${f}
            totalJobs=${c.jobs.length}
            selectedJobId=${a}
            search=${n}
            onSearchChange=${r}
            stateFilter=${s}
            onStateFilterChange=${i}
            onSelectJob=${h}
            onCancelJob=${x}
            isBusy=${c.isBusy}
            isRefreshing=${c.isRefreshing}
          />
        `;return u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          ${a&&u`<div className="flex flex-wrap justify-end gap-2">
            ${$}
          </div>`}
          ${c.error&&u`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${c.error.message}
            </div>
          `}
          <${ON}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${ON}
            result=${d.promptResult}
            onDismiss=${d.clearPromptResult}
          />
          <${xN} summary=${c.summary} />
          ${g}
        </div>
      </div>
    </div>
  `}function ur(e){if(!e)return"Not scheduled";let t=new Date(e);return Number.isNaN(t.getTime())?"Not scheduled":t.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}function ud(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function cd(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function LN(e=[]){return[...e].sort((t,a)=>{if(t.enabled!==a.enabled)return t.enabled?-1:1;let n=new Date(a.next_fire_at||a.last_run_at||0).getTime(),r=new Date(t.next_fire_at||t.last_run_at||0).getTime();return(Number.isNaN(n)?0:n)-(Number.isNaN(r)?0:r)})}function PN(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function WD(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function jN({runs:e}){return e?.length?u`
    <div className="space-y-3">
      ${e.map(t=>u`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${z} tone=${WD(t.status)} label=${t.status} />
              <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                ${ur(t.started_at)}
              </span>
            </div>
            ${t.result_summary&&u`<p className="mt-3 text-sm leading-6 text-iron-300">${t.result_summary}</p>`}
          </div>
        `)}
    </div>
  `:u`
      <div className="rounded-xl border border-iron-700 bg-iron-950/40 p-4 text-sm text-iron-300">
        No runs recorded yet.
      </div>
    `}function cr({label:e,value:t}){return u`
    <div className="rounded-xl border border-iron-700 bg-iron-950/50 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div className="mt-2 min-w-0 break-words text-sm text-iron-100">
        ${t||"\u2014"}
      </div>
    </div>
  `}function UN({title:e,value:t}){return u`
    <div>
      <h3 className="text-sm font-semibold text-iron-100">${e}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >${JSON.stringify(t||{},null,2)}</pre>
    </div>
  `}function FN({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=he(),l=C();return t?u`
      <div className="space-y-4">
        ${[1,2,3].map(c=>u`<div key=${c} className="v2-skeleton h-32 rounded-xl" />`)}
      </div>
    `:a||!e?u`
      <${xe}
        title=${l("routine.unavailable")}
        description=${a?.message||l("routine.unavailableDesc")}
      />
    `:u`
    <${q} className="p-4 sm:p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="truncate text-2xl font-semibold tracking-tight text-iron-100">
              ${e.name}
            </h2>
            <${z}
              tone=${ud(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${z}
              tone=${cd(e.verification_status)}
              label=${e.verification_status||"unknown"}
            />
          </div>
          <p className="mt-2 max-w-3xl text-sm leading-6 text-iron-300">
            ${e.description||e.trigger_summary||"No description"}
          </p>
        </div>

        <div className="flex flex-wrap gap-2">
          <${A} variant="secondary" disabled=${n} onClick=${r}>Run<//>
          <${A} variant="ghost" disabled=${n} onClick=${s}>
            ${e.enabled?"Disable":"Enable"}
          <//>
          <${A} variant="ghost" onClick=${i}>Delete<//>
        </div>
      </div>

      <div className="mt-5 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <${cr} label="Trigger" value=${e.trigger_summary||e.trigger_type} />
        <${cr} label="Action" value=${PN(e.action)} />
        <${cr} label="Next fire" value=${ur(e.next_fire_at)} />
        <${cr} label="Last run" value=${ur(e.last_run_at)} />
        <${cr} label="Run count" value=${e.run_count} />
        <${cr} label="Failures" value=${e.consecutive_failures} />
        <${cr} label="Created" value=${ur(e.created_at)} />
        <${cr} label="Routine ID" value=${e.id} />
      </div>

      ${e.conversation_id&&u`
        <div className="mt-5">
          <${A} variant="secondary" onClick=${()=>o(`/chat/${e.conversation_id}`)}>
            Open routine thread
          <//>
        </div>
      `}

      <div className="mt-6 grid gap-6 xl:grid-cols-2">
        <${UN} title=${l("routine.triggerPayload")} value=${e.trigger} />
        <${UN} title=${l("routine.actionPayload")} value=${e.action} />
      </div>

      <div className="mt-6">
        <h3 className="mb-3 text-sm font-semibold text-iron-100">Recent runs</h3>
        <${jN} runs=${e.recent_runs} />
      </div>
    <//>
  `}function BN({routine:e,selectedRoutineId:t,onSelectRoutine:a,onTriggerRoutine:n,onToggleRoutine:r,isBusy:s}){let i=t===e.id;return u`
    <article
      className=${["group flex flex-col gap-4 rounded-[18px] border p-5",i?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick=${()=>a(e.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">${e.name}</h3>
            <${z}
              tone=${ud(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${z}
              tone=${cd(e.verification_status)}
              label=${e.verification_status||"unknown"}
            />
          </div>
          <p className="mt-2 line-clamp-2 text-sm leading-6 text-iron-300">
            ${e.description||e.trigger_summary||"No description"}
          </p>
          <div className="mt-3 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
            <span>${e.trigger_type}</span>
            <span>${e.action_type}</span>
            <span>runs ${e.run_count||0}</span>
            <span>next ${ur(e.next_fire_at)}</span>
          </div>
        </button>

        <div className="flex shrink-0 flex-wrap gap-2">
          <${A}
            variant="secondary"
            className="h-9 px-3 text-xs"
            disabled=${s}
            onClick=${()=>n(e.id)}
          >
            Run
          <//>
          <${A}
            variant="ghost"
            className="h-9 px-3 text-xs"
            disabled=${s}
            onClick=${()=>r(e.id)}
          >
            ${e.enabled?"Disable":"Enable"}
          <//>
          <${A}
            variant="ghost"
            className="h-9 px-3 text-xs"
            onClick=${()=>a(e.id)}
          >
            Open
          <//>
        </div>
      </div>
    </article>
  `}var ZD=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function Mh({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:l,onToggleRoutine:c,isBusy:d,isRefreshing:m}){let f=C();if(!e.length){let h=!!n.trim()||s!=="all";return u`
      <${xe}
        title=${t&&h?"No routines match":"No routines yet"}
        description=${t&&h?"Adjust the search or status filter to find a saved routine.":"Routines created from chat will appear here after they are saved."}
      />
    `}return u`
    <div className="space-y-5">
      <${q} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
              ${f("routines.explorer")}
            </div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-iron-100">
              ${f("routines.title")}
            </h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${f("routines.description")}
            </p>
          </div>
          <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
            <span>${e.length} visible</span>
            <span>/</span>
            <span>${m?"refreshing":"live"}</span>
          </div>
        </div>

        <div className="mt-5 grid gap-3 md:grid-cols-[minmax(0,1fr)_220px]">
          <input
            value=${n}
            onInput=${h=>r(h.target.value)}
            placeholder="Search routine name, trigger, or action"
            className="h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          />
          <select
            value=${s}
            onChange=${h=>i(h.target.value)}
            className="v2-select h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          >
            ${ZD.map(h=>u`<option key=${h.value} value=${h.value}>${h.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(h=>u`
            <${BN}
              key=${h.id}
              routine=${h}
              selectedRoutineId=${a}
              onSelectRoutine=${o}
              onTriggerRoutine=${l}
              onToggleRoutine=${c}
              isBusy=${d}
            />
          `)}
      </div>
    </div>
  `}var eM=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function zN({summary:e}){return u`
    <${q} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${eM.map(t=>u`
            <div
              key=${t.key}
              className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
            >
              <${Ze}
                label=${t.label}
                value=${e?.[t.key]??0}
                tone=${t.tone}
                detail=${t.detail}
                showDivider=${!1}
                className="px-0 py-0"
              />
            </div>
          `)}
      </div>
    <//>
  `}function qN(e){let[t,a]=p.default.useState(""),[n,r]=p.default.useState("all");return{filteredRoutines:p.default.useMemo(()=>{let i=t.trim().toLowerCase();return LN(e).filter(o=>{let l=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||l.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function IN(){return Promise.resolve({routines:[],todo:!0})}function HN(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function KN(e){return Promise.resolve(null)}function dd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function md(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function QN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function VN(e){let t=W(),[a,n]=p.default.useState(null),r=H({queryKey:["routine-detail",e],queryFn:()=>KN(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:m=>{n({type:"error",message:m.message||"Unable to update routine"})}}),o=V(i(dd,"Routine run queued.")),l=V(i(md,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:l.mutateAsync,isBusy:o.isPending||l.isPending}}function GN(){let e=W(),[t,a]=p.default.useState(null),n=H({queryKey:["routines-summary"],queryFn:HN,refetchInterval:5e3}),r=H({queryKey:["routines"],queryFn:IN,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,m)=>({mutationFn:({routineId:f})=>d(f),onSuccess:()=>{a({type:"success",message:m}),s()},onError:f=>{a({type:"error",message:f.message||"Unable to update routine"})}}),o=V(i(dd,"Routine run queued.")),l=V(i(md,"Routine status updated.")),c=V(i(QN,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:l.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||l.isPending||c.isPending,invalidate:s}}function Oh(){let e=he(),{routineId:t=null}=st(),a=GN(),n=VN(t),r=qN(a.routines),s=p.default.useCallback(async(l,c)=>{try{await l({routineId:c})}catch{}},[]),i=p.default.useCallback(async(l,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:l}),e("/routines")}catch{}},[e,a]),o=t?u`
        <div className="grid gap-5 xl:grid-cols-[minmax(0,0.9fr)_minmax(440px,1.1fr)]">
          <${Mh}
            routines=${r.filteredRoutines}
            totalRoutines=${a.routines.length}
            selectedRoutineId=${t}
            search=${r.search}
            onSearchChange=${r.setSearch}
            statusFilter=${r.statusFilter}
            onStatusFilterChange=${r.setStatusFilter}
            onSelectRoutine=${l=>e(`/routines/${l}`)}
            onTriggerRoutine=${l=>s(a.triggerRoutine,l)}
            onToggleRoutine=${l=>s(a.toggleRoutine,l)}
            isBusy=${a.isBusy}
            isRefreshing=${a.isRefreshing}
          />
          <${FN}
            routine=${n.routine}
            isLoading=${n.isLoading}
            error=${n.error}
            isBusy=${n.isBusy}
            onTriggerRoutine=${n.triggerRoutine}
            onToggleRoutine=${n.toggleRoutine}
            onDeleteRoutine=${()=>i(t,n.routine?.name||t)}
          />
        </div>
      `:u`
        <${Mh}
          routines=${r.filteredRoutines}
          totalRoutines=${a.routines.length}
          selectedRoutineId=${t}
          search=${r.search}
          onSearchChange=${r.setSearch}
          statusFilter=${r.statusFilter}
          onStatusFilterChange=${r.setStatusFilter}
          onSelectRoutine=${l=>e(`/routines/${l}`)}
          onTriggerRoutine=${l=>s(a.triggerRoutine,l)}
          onToggleRoutine=${l=>s(a.toggleRoutine,l)}
          isBusy=${a.isBusy}
          isRefreshing=${a.isRefreshing}
        />
      `;return u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          ${t&&u`<div className="flex flex-wrap justify-end gap-2">
            <${A} variant="ghost" onClick=${()=>e("/routines")}>
              All routines
            <//>
          </div>`}

          ${a.error&&u`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${a.error.message}
            </div>
          `}

          <${Za}
            result=${a.actionResult}
            onDismiss=${a.clearActionResult}
          />
          <${Za}
            result=${n.actionResult}
            onDismiss=${n.clearActionResult}
          />
          <${zN} summary=${a.summary} />

          ${a.isLoading?u`
                <div className="space-y-4">
                  ${[1,2,3].map(l=>u`<div key=${l} className="v2-skeleton h-32 rounded-xl" />`)}
                </div>
              `:o}
        </div>
      </div>
    </div>
  `}function tM(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function aM(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function YN({deliveryState:e}){let t=C(),a=e.currentTarget?.target_id||"",[n,r]=p.default.useState(a),[s,i]=p.default.useState(!1),o=p.default.useRef(null);p.default.useEffect(()=>{r(a)},[a]),p.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let l=n!==a,c=e.isLoading||e.isSaving,d=l&&!c,m=!!a&&!c,f=e.finalReplyTargets.length>0,h=e.targets.some(L=>L?.capabilities?.final_replies&&L?.target?.status==="unavailable"),x=f||h,y=L=>(o.current&&clearTimeout(o.current),i(!1),L.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),$=()=>{d&&y(e.saveFinalReplyTarget(n||null))},g=()=>{m&&(r(""),y(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),b=e.currentStatus,w=b==="available"?"success":b==="unavailable"?"warning":"muted",S=t(b==="available"?"automations.delivery.pill.ready":b==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),E=!!e.currentTarget,N=t(E?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),T=aM(t("automations.delivery.footnote"),{command:u`<code
        key="cmd"
        className="rounded px-1.5 py-0.5 font-mono text-[0.6875rem] bg-[var(--v2-surface-muted)] text-[var(--v2-accent-text)]"
      >
        approve &lt;code&gt;
      </code>`});return u`
    <${q} data-testid="automation-delivery-defaults" className="p-5 sm:p-6">
      <div className="flex flex-col gap-5">

        <!-- ── Header ──────────────────────────────────────────────── -->
        <div className="flex flex-col gap-1">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">
            ${t("automations.delivery.eyebrow")}
          </div>
          <h2 className="mt-1 text-xl font-semibold tracking-[-0.02em] text-[var(--v2-text-strong)]">
            ${t("automations.delivery.title")}
          </h2>
          <p className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("automations.delivery.explainer")}
          </p>
        </div>

        <hr className="border-t border-[var(--v2-panel-border)]" />

        <!-- ── Current default row (only when a target is configured) ── -->
        ${E&&u`
          <div>
            <span className="mb-1.5 block font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
              ${t("automations.delivery.currentDefault")}
            </span>
            <div
              data-testid="automation-delivery-current-default"
              className="flex items-center gap-3 rounded-xl border px-4 py-3 bg-[var(--v2-positive-soft)] border-[color-mix(in_srgb,var(--v2-positive-text)_25%,var(--v2-panel-border))]"
            >
              <span className="flex-1 min-w-0 text-sm font-semibold text-[var(--v2-text-strong)] truncate">
                ${v}
              </span>
              <${z} tone=${w} label=${S} />
            </div>
          </div>
        `}

        <!-- ── Radio option rows ────────────────────────────────────── -->
        <div>
          <span className="mb-1.5 block font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
            ${N}
          </span>
          <div
            className="flex flex-col gap-3"
            role="radiogroup"
            aria-label=${t("automations.delivery.title")}
          >

            <!-- Available external targets -->
            ${e.finalReplyTargets.map(L=>{let O=L?.target?.target_id??"",P=L?.target?.display_name||L?.target?.target_id||"",k=L?.target?.description||"",U=L?.target?.status??"available",Y=n===O;return u`
                <label
                  key=${O}
                  data-testid="automation-delivery-target"
                  data-target-id=${O}
                  className=${G("flex items-start gap-3.5 rounded-xl border px-4 py-3.5 cursor-pointer","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]","hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",Y&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
                >
                  <input
                    data-testid="automation-delivery-target-radio"
                    type="radio"
                    name="delivery-target"
                    value=${O}
                    checked=${Y}
                    disabled=${c}
                    onChange=${()=>r(O)}
                    className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-semibold text-[var(--v2-text-strong)] leading-snug">
                      ${P}
                    </div>
                    ${k&&u`<div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                      ${k}
                    </div>`}
                  </div>
                  <${z}
                    tone=${tM(U)}
                    label=${t(U==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.ready")}
                    className="self-center shrink-0"
                  />
                </label>
              `})}

            <!-- Unpaired notice rows (targets present but status=unavailable
                 and NOT already shown above because they lack final_replies) -->
            ${h&&u`
              <div
                className="flex items-center gap-3 rounded-xl border border-dashed border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3.5 text-sm text-[var(--v2-text-muted)]"
              >
                <span className="text-base shrink-0 opacity-70">📎</span>
                <div className="flex-1 min-w-0">
                  <span className="text-sm font-semibold text-[var(--v2-text-muted)]">
                    ${t("automations.delivery.unpairedNotice")}
                  </span>
                  <div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-faint)]">
                    ${t("automations.delivery.unpairedDesc")}
                  </div>
                </div>
                <${z}
                  tone="warning"
                  label=${t("automations.delivery.pill.notPaired")}
                  className="shrink-0"
                />
              </div>
            `}

            <!-- Web app only / fallback row -->
            <label
              data-testid="automation-delivery-web-target"
              className=${G("flex items-start gap-3.5 rounded-xl border px-4 py-3.5","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]",f?"cursor-pointer hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]":"cursor-default",n===""&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
            >
              <input
                data-testid="automation-delivery-target-radio"
                type="radio"
                name="delivery-target"
                value=""
                checked=${n===""}
                disabled=${c||!f}
                onChange=${()=>r("")}
                className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
              />
              <div className="flex-1 min-w-0">
                <div className="text-sm font-semibold text-[var(--v2-text-strong)] leading-snug">
                  ${t("automations.delivery.webOption")}
                </div>
                <div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                  ${t("automations.delivery.webOptionDesc")}
                </div>
              </div>
              <${z}
                tone="muted"
                label=${t("automations.delivery.pill.fallback")}
                className="self-center shrink-0"
              />
            </label>

          </div>
        </div>

        <!-- ── Save row ─────────────────────────────────────────────── -->
        <div className="flex flex-wrap items-center gap-3">
          <${A}
            data-testid="automation-delivery-save"
            variant="primary"
            size="sm"
            disabled=${!d}
            onClick=${$}
          >
            <${M} name="check" className="h-3.5 w-3.5" />
            ${t("automations.delivery.save")}
          <//>
          <${A}
            variant="secondary"
            size="sm"
            disabled=${!m}
            onClick=${g}
          >
            ${t("automations.delivery.clear")}
          <//>
          ${s&&u`
            <span
              role="status"
              className="flex items-center gap-1.5 text-xs font-semibold text-[var(--v2-positive-text)]"
            >
              <${M} name="check" className="h-3 w-3" />
              ${t("automations.delivery.saved")}
            </span>
          `}
          ${e.saveError&&!s&&u`
            <span
              role="alert"
              className="flex items-center gap-1.5 text-xs font-semibold text-red-300"
            >
              <${M} name="close" className="h-3 w-3" />
              ${t("automations.delivery.saveFailed")}
            </span>
          `}
        </div>

        <!-- ── Footnote (only when an external Slack-style target exists) ── -->
        ${x&&u`
          <div
            className="rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-faint)]"
          >
            ${T}
          </div>
        `}

      </div>
    <//>
  `}var nM=["schedule","once"],XN={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},WN={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},ZN={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function oa(e){return typeof e=="function"?e:t=>t}var Ph=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:Rn},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:yM},{value:"completed",labelKey:"automations.filter.completed",predicate:bM}];function e_(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>nM.includes(r?.source?.type)).map(r=>fM(r,t,a)).sort(gM)}function t_(e,t){let a=Ph.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function a_(e){let t=e.filter(i=>i.state!=="completed"),a=t.filter(i=>Rn(i)).length,n=t.filter(i=>i.has_running_run).length,r=t.filter(i=>i.has_failed_runs).length,s=t.filter(i=>Rn(i)&&Lh(i)!=null).sort((i,o)=>(i.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(o.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:t.length,active:a,running:n,failures:r,nextRun:s?.next_run_label||null}}function rM(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=SM(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:l,month:c,dayOfWeek:d,year:m}=s,f=t&&typeof t=="string"?t:null,h=f?` (${f})`:"",x=m==="*"&&l==="*"&&c==="*"&&d==="*";if(x&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=NM(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(dr(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let y=xM(o,i,n);if(!y)return r("automations.schedule.custom");if(x)return r("automations.schedule.everyDayAt",{time:y})+h;let $=_M(d);if(m==="*"&&l==="*"&&c==="*"&&$==="1-5")return r("automations.schedule.weekdaysAt",{time:y})+h;if(m==="*"&&l==="*"&&c==="*"&&dr($,0,7)){let g=$M(Number($)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:y})+h}if(m==="*"&&dr(l,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(l),time:y})+h;if(dr(l,1,31)&&dr(c,1,12)&&d==="*"&&(m==="*"||dr(m,1970,9999))){let g=wM(Number(c),Number(l),m==="*"?null:Number(m),n);return r("automations.schedule.dateAt",{date:g,time:y})+h}return r("automations.schedule.custom")}function Vr(e,t="Unknown",a,n){if(!e)return t;let r=new Date(e);if(Number.isNaN(r.getTime()))return t;let s=n&&typeof n=="string"?{timeZone:n}:{};try{return r.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...s})}catch{return r.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function n_(e,t){let a=XN[e]?.labelKey||"automations.state.unknown";return oa(t)(a)}function r_(e){return XN[e]?.tone||"muted"}function sM(e,t){return Rn(e)&&e?.has_running_run?oa(t)("automations.status.running"):Rn(e)&&e?.has_failed_runs?oa(t)("automations.status.needsReview"):n_(e?.state,t)}function iM(e){return Rn(e)&&e?.has_running_run?"info":Rn(e)&&e?.has_failed_runs?"danger":r_(e?.state)}function oM(e,t){let a=WN[e]?.labelKey||"automations.lastStatus.none";return oa(t)(a)}function lM(e){return WN[e]?.tone||"muted"}function uM(e,t){let a=ZN[fd(e)]?.labelKey||"automations.runStatus.unknown";return oa(t)(a)}function cM(e){return ZN[fd(e)]?.tone||"muted"}function dM(e,t,a,n){if(!e)return oa(a)("automations.schedule.custom");let r=Vr(e,null,n,t);if(!r)return oa(a)("automations.schedule.custom");let s=t&&typeof t=="string"?` (${t})`:"";return oa(a)("automations.schedule.onceAt",{datetime:r})+s}function mM(e,t,a){return e?.type==="once"?dM(e.at,e.timezone,t,a):e?.type==="schedule"?rM(e.cron,e.timezone||"UTC",t,a):oa(t)("automations.schedule.custom")}function fM(e,t,a){let n=oa(t),r=pM(e.recent_runs,t,a),s=r[0]||null,i=r.find(m=>m.status==="running")||null,o=r.find(m=>m.status==="ok"||m.status==="error")||null,l=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null,d={...e,recent_runs:r,has_running_run:r.some(m=>m.status==="running"),has_failed_runs:r.some(m=>m.status==="error")};return{...d,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:mM(e.source,t,a),state_label:n_(e.state,t),state_tone:r_(e.state),primary_status_label:sM(d,t),primary_status_tone:iM(d),next_run_timestamp:jh(e.next_run_at),next_run_label:Vr(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:Vr(c,n("automations.date.noRuns"),a),last_status_label:oM(l,t),last_status_tone:lM(l),created_label:Vr(e.created_at,n("automations.date.unknown"),a),latest_run:s,current_run:i,success_rate_label:vM(r,t)}}function pM(e,t,a){let n=oa(t);return Array.isArray(e)?e.map(r=>{let s=fd(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=jh(i);return{...r,status:s,status_label:uM(s,t),status_tone:cM(s),timestamp:o,timestamp_source:i,fired_label:Vr(i,n("automations.date.unscheduled"),a),submitted_label:Vr(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:Vr(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function fd(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function s_(e){let t=Array.isArray(e)?e:[],a={total:t.length,ok:0,error:0,running:0,unknown:0};for(let n of t){let r=fd(n?.status);Object.prototype.hasOwnProperty.call(a,r)?a[r]+=1:a.unknown+=1}return a}function hM(e){let t=s_(e);return[{key:"ok",tone:"text-emerald-300",count:t.ok},{key:"error",tone:"text-red-300",count:t.error},{key:"running",tone:"text-sky-300",count:t.running},{key:"unknown",tone:"text-iron-400",count:t.unknown}].filter(a=>a.count>0)}function i_(e,t){let a=oa(t),n=s_(e),r=hM(e).map(s=>({...s,text:a(`automations.runs.${s.key}`,{count:s.count})}));return{total:n.total,totalText:a("automations.runs.total",{count:n.total}),chips:r}}function vM(e,t){let a=oa(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function gM(e,t){let a=Rn(e),n=Rn(t);return a!==n?a?-1:1:(Lh(e)??Number.MAX_SAFE_INTEGER)-(Lh(t)??Number.MAX_SAFE_INTEGER)}function jh(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function Rn(e){return e?.state==="active"||e?.state==="scheduled"}function yM(e){return["paused","disabled","inactive"].includes(e?.state)}function bM(e){return e?.state==="completed"}function Lh(e){return e?.next_run_timestamp??jh(e?.next_run_at)}function Uh(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function xM(e,t,a){return!dr(e,0,23)||!dr(t,0,59)?null:Uh(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function $M(e,t){return Uh(t,{weekday:"long"},new Date(2001,0,7+e))}function wM(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return Uh(n,r,new Date(a??2e3,e-1,t))}function SM(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&JN(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&JN(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function JN(e){return/^0+$/.test(e)}function dr(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function NM(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function _M(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}function RM(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function o_({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let l=o.toString(),c=`/logs${l?`?${l}`:""}`;return i?`/v2${c}`:c}function l_(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(RM),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}var kM=8;function Fh(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function pd({runs:e=[]}){let t=C(),a=Array.isArray(e)?e:[],n=a.slice(0,kM);if(!n.length)return u`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;let r=a.length-n.length,s=`+${Math.min(r,999)}`;return u`
    <div
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
    >
      ${n.map(i=>u`
        <span
          key=${Fh(i)}
          title=${`${i.status_label} \xB7 ${i.fired_label}`}
          className=${G("h-3 w-3 rounded-full border",i.status==="ok"&&"border-emerald-300/50 bg-emerald-400",i.status==="error"&&"border-red-300/50 bg-red-400",i.status==="running"&&"border-sky-300/60 bg-sky-400",i.status==="unknown"&&"border-iron-500 bg-iron-600")}
        />
      `)}
      ${r>0&&u`<span
        className="ml-0.5 font-mono text-[11px] text-iron-400"
        title=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
      >
        ${s}
      </span>`}
    </div>
  `}function hd({runs:e=[],className:t=""}){let a=C(),n=i_(e,a);return n.total?u`
    <div className=${G("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]",t)}>
      <span className="text-iron-300">${n.totalText}</span>
      ${n.chips.map(r=>u`<span key=${r.key} className=${r.tone}>${r.text}</span>`)}
    </div>
  `:u`<span className=${G("text-[11px] text-iron-400",t)}>
      ${a("automations.table.noRuns")}
    </span>`}function u_({run:e,onOpenRun:t,onOpenLogs:a}){let n=C(),r=!!e.chat_path,s=o_({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return u`
    <div className="grid gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto] sm:items-center">
      <div>
        <${z} tone=${e.status_tone} label=${e.status_label} />
      </div>
      <div className="min-w-0">
        <div className="text-sm font-semibold text-iron-100">${e.fired_label}</div>
        <div className="mt-1 truncate font-mono text-[11px] text-iron-400">
          ${e.thread_id?`${n("automations.detail.thread")} ${e.thread_id}`:n("automations.detail.noThread")}
        </div>
        ${e.run_id&&u`
          <div className="mt-1 truncate font-mono text-[11px] text-iron-500">
            ${n("automations.detail.run")} ${e.run_id}
          </div>
        `}
      </div>
      <div className="flex flex-wrap items-center gap-2 sm:justify-end">
        <${A}
          variant="secondary"
          size="sm"
          disabled=${!r}
          onClick=${r?()=>t(e.chat_path):void 0}
        >
          <${M} name="chat" className="mr-1.5 h-4 w-4" />
          ${n("automations.detail.openRun")}
        <//>
        <${A}
          variant="ghost"
          size="sm"
          disabled=${!i}
          onClick=${i?()=>a(s):void 0}
        >
          <${M} name="file" className="mr-1.5 h-4 w-4" />
          ${n("nav.logs")}
        <//>
      </div>
    </div>
  `}function vd({label:e,value:t,tone:a}){return u`
    <div className="min-w-0 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div
        className=${G("mt-2 min-w-0 break-words text-sm text-iron-100",a==="success"&&"text-emerald-200",a==="danger"&&"text-red-200",a==="info"&&"text-sky-200")}
      >
        ${t||"\u2014"}
      </div>
    </div>
  `}function c_({automation:e,isMutating:t=!1,onPauseAutomation:a,onResumeAutomation:n,onDeleteAutomation:r}){let s=C(),i=he();if(!e)return u`
      <${q} className="p-4 sm:p-5">
        <${xe}
          boxed=${!1}
          title=${s("automations.detail.emptyTitle")}
          description=${s("automations.detail.emptyDescription")}
        />
      <//>
    `;let o=e.current_run,l=e.state==="paused",c=e.state==="active"||e.state==="scheduled",m=`${s(l?"missions.action.resume":"missions.action.pause")}: ${e.display_name}`,f=()=>{if(l){n?.(e.automation_id);return}c&&a?.(e.automation_id)},h=`${s("common.delete")}: ${e.display_name}`,x=()=>{window.confirm(h)&&r?.(e.automation_id)};return u`
    <${q} className="overflow-hidden">
      <div className="border-b border-[var(--v2-panel-border)] p-4 sm:p-5">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            <h3 className="truncate text-xl font-semibold tracking-tight text-iron-100">
              ${e.display_name}
            </h3>
            <div className="mt-2 truncate font-mono text-[11px] uppercase tracking-[0.12em] text-iron-400">
              ${e.automation_id}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <${z}
              tone=${e.primary_status_tone}
              label=${e.primary_status_label}
            />
            ${(c||l)&&u`
              <${A}
                type="button"
                variant=${l?"primary":"secondary"}
                size="icon-sm"
                aria-label=${m}
                title=${m}
                disabled=${t}
                onClick=${f}
              >
                <${M} name=${l?"play":"pause"} className="h-4 w-4" />
              <//>
            `}
            <${A}
              type="button"
              variant="danger"
              size="icon-sm"
              aria-label=${h}
              title=${h}
              disabled=${t}
              onClick=${x}
            >
              <${M} name="trash" className="h-4 w-4" />
            <//>
          </div>
        </div>
      </div>

      <div className="space-y-5 p-4 sm:p-5">
        <div className="grid gap-3 sm:grid-cols-2">
          <${vd} label=${s("automations.detail.schedule")} value=${e.schedule_label} />
          <${vd}
            label=${s("automations.detail.successRate")}
            value=${e.success_rate_label}
            tone=${e.has_failed_runs?"danger":"success"}
          />
          <${vd} label=${s("automations.detail.lastCompleted")} value=${e.last_run_label} />
          <${vd}
            label=${s("automations.detail.currentRun")}
            value=${o?.run_id||o?.thread_id||s("automations.detail.noCurrentRun")}
            tone=${e.has_running_run?"info":null}
          />
        </div>

        <div>
          <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
            <h4 className="text-sm font-semibold text-iron-100">
              ${s("automations.detail.recentRuns")}
            </h4>
            <div className="flex flex-col items-end gap-1">
              <${pd} runs=${e.recent_runs} />
              <${hd} runs=${e.recent_runs} />
            </div>
          </div>

          ${e.recent_runs.length?u`
                <div>
                  ${e.recent_runs.map(y=>u`
                    <${u_}
                      key=${Fh(y)}
                      run=${y}
                      onOpenRun=${i}
                      onOpenLogs=${i}
                    />
                  `)}
                </div>
              `:u`
                <div className="rounded-xl border border-dashed border-[var(--v2-panel-border)] p-4 text-sm text-iron-300">
                  ${s("automations.detail.noRuns")}
                </div>
              `}
        </div>
      </div>
    <//>
  `}var CM=["automations.empty.example1","automations.empty.example2","automations.empty.example3"];function EM({promptKey:e}){let t=C(),a=t(e),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>()=>clearTimeout(s.current),[]),u`
    <li
      className="flex items-center gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3"
    >
      <span className="min-w-0 flex-1 text-sm leading-6 text-iron-200">${a}</span>
      <button
        type="button"
        onClick=${async()=>{let o=typeof navigator>"u"?null:navigator.clipboard;if(o?.writeText)try{await o.writeText(a),r(!0),clearTimeout(s.current),s.current=setTimeout(()=>r(!1),1500)}catch{}}}
        aria-label=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        title=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        className=${G("inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-[var(--v2-panel-border)] text-iron-300 hover:text-iron-100 hover:border-white/20","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",n&&"text-emerald-300")}
      >
        <${M} name=${n?"check":"copy"} className="h-4 w-4" />
      </button>
    </li>
  `}function d_(){let e=C(),t=he();return u`
    <${q} className="p-6 sm:p-8">
      <div className="max-w-2xl">
        <h2 className="mt-4 text-2xl font-semibold tracking-tight text-iron-100 flex items-center gap-3">
          ${e("automations.empty.onboardingTitle")}
        </h2>
        <p className="mt-3 text-sm leading-6 text-iron-300">
          ${e("automations.empty.onboardingDescription")}
        </p>

        <div className="mt-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-400">
            ${e("automations.empty.examplesTitle")}
          </div>
          <ul className="mt-3 space-y-2">
            ${CM.map(a=>u`<${EM} key=${a} promptKey=${a} />`)}
          </ul>
        </div>

        <div className="mt-6">
          <${A} variant="primary" size="sm" onClick=${()=>t("/chat")}>
            <${M} name="chat" className="mr-1.5 h-4 w-4" />
            ${e("automations.empty.startInChat")}
          <//>
        </div>
      </div>
    <//>
  `}function m_({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,isMutating:s,selectedAutomationId:i,onSelectAutomation:o,onPauseAutomation:l,onResumeAutomation:c,onDeleteAutomation:d}){let m=C(),f=t_(e,t),h=e.length>0,x=f.find(y=>y.automation_id===i)||f[0]||null;return u`
    <div className="space-y-5">
      <${q} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
              ${m("automations.eyebrow")}
            </div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-iron-100">
              ${m("automations.title")}
            </h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${m("automations.description")}
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <div
              className="inline-flex max-w-full overflow-x-auto rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]"
              role="group"
              aria-label=${m("automations.filterLabel")}
            >
              ${Ph.map(y=>u`
                <button
                  key=${y.value}
                  type="button"
                  aria-pressed=${t===y.value}
                  onClick=${()=>a(y.value)}
                  className=${G("min-h-9 shrink-0 whitespace-nowrap px-3 py-2 text-xs font-semibold leading-tight",t===y.value?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
                >
                  ${m(y.labelKey)}
                </button>
              `)}
            </div>
            <${A}
              variant="secondary"
              size="icon-sm"
              aria-label=${m("automations.refresh")}
              title=${m(r?"automations.refreshing":"automations.refresh")}
              disabled=${r}
              onClick=${n}
            >
              <${M}
                name="retry"
                className=${G("h-4 w-4",r&&"v2-spin")}
              />
            <//>
          </div>
        </div>
      <//>

      ${f.length?u`
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1.12fr)_minmax(22rem,0.88fr)]">
              <${q} className="overflow-hidden">
                <div className="overflow-x-auto">
                  <table className="w-full min-w-[900px] border-collapse">
                    <thead>
                      <tr className="border-b border-[var(--v2-panel-border)] text-left">
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${m("automations.table.name")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${m("automations.table.schedule")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${m("automations.table.nextRun")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${m("automations.table.recentRuns")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${m("automations.table.status")}
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      ${f.map(y=>{let $=y.automation_id===x?.automation_id;return u`
                          <tr
                            key=${y.automation_id}
                            className=${G("border-b border-[var(--v2-panel-border)] last:border-0 hover:bg-white/[0.03]",$&&"bg-[var(--v2-accent-soft)]/30")}
                          >
                            <td className="max-w-[280px] px-5 py-4 align-top">
                              <button
                                type="button"
                                aria-pressed=${$}
                                onClick=${()=>o(y.automation_id)}
                                className="block w-full min-w-0 rounded text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]"
                              >
                                <div className="truncate text-sm font-semibold text-iron-100">
                                  ${y.display_name}
                                </div>
                                <div className="mt-1 truncate font-mono text-[11px] uppercase tracking-[0.12em] text-iron-400">
                                  ${y.automation_id}
                                </div>
                              </button>
                            </td>
                            <td className="px-5 py-4 align-top text-sm text-iron-200">
                              ${y.schedule_label}
                            </td>
                            <td className="px-5 py-4 align-top text-sm text-iron-200">
                              ${y.next_run_label}
                            </td>
                            <td className="px-5 py-4 align-top">
                              <div className="space-y-2">
                                <${pd} runs=${y.recent_runs} />
                                <${hd} runs=${y.recent_runs} />
                              </div>
                            </td>
                            <td className="px-5 py-4 align-top">
                              <${z}
                                tone=${y.primary_status_tone}
                                label=${y.primary_status_label}
                              />
                            </td>
                          </tr>
                        `})}
                    </tbody>
                  </table>
                </div>
              <//>

              <${c_}
                automation=${x}
                isMutating=${s}
                onPauseAutomation=${l}
                onResumeAutomation=${c}
                onDeleteAutomation=${d}
              />
            </div>
          `:h?u`
              <${xe}
                title=${m("automations.empty.matchingTitle")}
                description=${m("automations.empty.matchingDescription")}
              />
            `:u`<${d_} />`}
    </div>
  `}function f_({summary:e,activeFilter:t,onSelectFilter:a}){let n=C(),r=[{key:"scheduled",label:n("automations.summary.scheduled"),value:e?.scheduled??0,tone:"muted",detail:n("automations.summary.scheduledDetail"),filter:"all"},{key:"active",label:n("automations.summary.active"),value:e?.active??0,tone:"signal",detail:n("automations.summary.activeDetail"),filter:"active"},{key:"running",label:n("automations.summary.running"),value:e?.running??0,tone:"info",detail:n("automations.summary.runningDetail"),filter:"running"},{key:"failures",label:n("automations.summary.failures"),value:e?.failures??0,tone:(e?.failures??0)>0?"danger":"success",detail:n("automations.summary.failuresDetail"),filter:(e?.failures??0)>0?"failures":null},{key:"nextRun",label:n("automations.summary.nextRun"),value:e?.nextRun||n("automations.summary.none"),tone:"info",detail:n("automations.summary.nextRunDetail"),valueClassName:"text-lg md:text-xl"}];return u`
    <${q} className="p-4 sm:p-5">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        ${r.map(s=>{let i=!!(s.filter&&a),o=i&&t===s.filter,l=u`
            <${Ze}
              label=${s.label}
              value=${s.value}
              tone=${s.tone}
              badgeLabel=${n(`automations.badge.${s.tone}`)}
              detail=${s.detail}
              valueClassName=${s.valueClassName}
              showDivider=${!1}
              className="px-0 py-0"
            />
          `,c="rounded-[14px] border border-white/8 bg-white/[0.03] p-4 text-left";return i?u`
            <button
              key=${s.key}
              type="button"
              aria-pressed=${o}
              title=${n("automations.summary.filterAction",{label:s.label})}
              onClick=${()=>a(s.filter)}
              className=${G(c,"transition-colors hover:border-white/20 hover:bg-white/[0.05]","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",o&&"border-[var(--v2-accent)]/60 bg-[var(--v2-accent-soft)]/30")}
            >
              ${l}
            </button>
          `:u`<div key=${s.key} className=${c}>${l}</div>`})}
      </div>
    <//>
  `}function TM(e){return e==="active"||e==="scheduled"}function AM(e){return Number.isFinite(e)?e:null}function p_(e,t=Date.now()){let a=Array.isArray(e)?e:[],n=null;for(let r of a){if(!r||(r.has_running_run&&(n=n==null?5e3:Math.min(n,5e3)),!TM(r.state)))continue;let s=AM(r.next_run_timestamp);if(s==null)continue;let i=s-t,o=i<=0?2e3:i<3e4?Math.max(1e3,i+1200):null;o!=null&&(n=n==null?o:Math.min(n,o))}return n}var MM=50,OM=25;function h_(e=!1){let{t,lang:a}=gl(),n=W(),r=H({queryKey:["automations",{includeCompleted:e}],queryFn:()=>Yx({limit:MM,runLimit:OM,includeCompleted:e}),refetchInterval:3e4,refetchIntervalInBackground:!1}),s=p.default.useMemo(()=>e_(r.data,t,a),[r.data,t,a]),i=p.default.useMemo(()=>a_(s),[s]),o=p.default.useMemo(()=>p_(s),[s]);p.default.useEffect(()=>{if(o==null)return;let h=setTimeout(()=>{r.refetch()},o);return()=>clearTimeout(h)},[o,r.refetch]);let l=r.data?.scheduler_enabled!==!1,c=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["automations"]})},[n]),d=V({mutationFn:h=>Jx({automationId:h}),onSuccess:c}),m=V({mutationFn:h=>Xx({automationId:h}),onSuccess:c}),f=V({mutationFn:h=>Wx({automationId:h}),onSuccess:c});return{automations:s,summary:i,schedulerEnabled:l,isLoading:r.isLoading,isRefreshing:r.isFetching,isMutating:d.isPending||m.isPending||f.isPending,error:r.error||null,actionError:d.error||m.error||f.error||null,pauseAutomation:d.mutate,resumeAutomation:m.mutate,deleteAutomation:f.mutate,refetch:r.refetch}}var v_=["outbound-delivery","preferences"],g_=["outbound-delivery","targets"];function y_(){let e=W(),t=H({queryKey:v_,queryFn:a$}),a=H({queryKey:g_,queryFn:n$}),n=V({mutationFn:({finalReplyTargetId:i})=>r$({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(v_,i),e.invalidateQueries({queryKey:g_})}}),r=p.default.useMemo(()=>a.data?.targets??[],[a.data]),s=p.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function b_(){let e=C(),[t,a]=p.default.useState("all"),[n,r]=p.default.useState(null),i=h_(t==="completed"),o=y_(),[l,c]=p.default.useState(!1),d=p.default.useRef(null);p.default.useEffect(()=>()=>clearTimeout(d.current),[]);let m=p.default.useCallback(()=>{c(!0),clearTimeout(d.current),d.current=setTimeout(()=>c(!1),1e3),i.refetch()},[i.refetch]),f=i.isRefreshing||l,h=i.error&&!i.isLoading&&i.automations.length===0;return p.default.useEffect(()=>{if(!i.automations.length){r(null);return}i.automations.some(y=>y.automation_id===n)||r(i.automations[0].automation_id)},[i.automations,n]),u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          ${i.error&&u`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${e("automations.error.loadFailed")}
            </div>
          `}
          ${i.actionError&&u`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${i.actionError.message}
            </div>
          `}

          ${h?null:u`
                ${!i.isLoading&&!i.schedulerEnabled&&u`
                  <div
                    role="status"
                    className="rounded-xl border border-amber-400/30 bg-amber-500/10 px-4 py-3"
                  >
                    <div className="text-sm font-semibold text-amber-200">
                      ${e("automations.schedulerOff.title")}
                    </div>
                    <div className="mt-0.5 text-xs leading-5 text-amber-200/80">
                      ${e("automations.schedulerOff.description")}
                    </div>
                  </div>
                `}
                <${f_}
                  summary=${i.summary}
                  activeFilter=${t}
                  onSelectFilter=${a}
                />
                <${YN} deliveryState=${o} />

                ${i.isLoading?u`
                      <div className="space-y-4">
                        ${[1,2,3].map(x=>u`<div
                              key=${x}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`)}
                      </div>
                    `:u`
                      <${m_}
                        automations=${i.automations}
                        filter=${t}
                        onFilterChange=${a}
                        onRefresh=${m}
                        isRefreshing=${f}
                        isMutating=${i.isMutating}
                        selectedAutomationId=${n}
                        onSelectAutomation=${r}
                        onPauseAutomation=${i.pauseAutomation}
                        onResumeAutomation=${i.resumeAutomation}
                        onDeleteAutomation=${i.deleteAutomation}
                      />
                    `}
              `}
        </div>
      </div>
    </div>
  `}var x_={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function $_({result:e,onDismiss:t}){return p.default.useEffect(()=>{if(!e)return;let a=setTimeout(t,4e3);return()=>clearTimeout(a)},[e,t]),e?u`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",x_[e.type]||x_.info].join(" ")}>
      <${M}
        name=${e.type==="success"?"check":e.type==="error"?"close":"bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">
        <${M} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `:null}var S_="/api/webchat/v2/channels/slack/setup";function N_(){return K(S_)}function __(e){let t={installation_id:String(e.installation_id||"").trim(),team_id:String(e.team_id||"").trim(),api_app_id:String(e.api_app_id||"").trim(),user_id:w_(e.user_id),shared_subject_user_id:w_(e.shared_subject_user_id)},a=String(e.bot_token||"").trim(),n=String(e.signing_secret||"").trim();return a&&(t.bot_token=a),n&&(t.signing_secret=n),K(S_,{method:"PUT",body:JSON.stringify(t)})}function Bh(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function w_(e){let t=String(e||"").trim();return t||null}var R_="/api/webchat/v2/channels/slack/allowed",LM="/api/webchat/v2/channels/slack/subjects";function k_(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function C_(){return K(R_)}function E_(){return K(LM)}function T_(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return K(R_,{method:"PUT",body:JSON.stringify(n)})}function A_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var D_=["slack-allowed-channels"];function O_({action:e}){let t=C(),a=W(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState([]),c=jM(e,t),d=H({queryKey:D_,queryFn:C_}),m=H({queryKey:["slack-routable-subjects"],queryFn:E_}),f=m.data?.subjects||[],h=M_(f),x=m.isSuccess||m.isError,y=f.length>0;p.default.useEffect(()=>{d.data&&l(zh(d.data.channels||[]))},[d.data]);let $=V({mutationFn:({channels:E})=>T_(E),onSuccess:E=>{l(zh(E.channels||[])),a.invalidateQueries({queryKey:D_}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let E=n.trim();!E||!m.isSuccess||(l(N=>zh([...N,{channel_id:E,subject_user_id:s}])),r(""))},v=E=>{l(N=>N.filter(T=>T.channel_id!==E))},b=(E,N)=>{l(T=>T.map(L=>L.channel_id===E?{...L,subject_user_id:N}:L))},w=()=>{$.mutate({channels:PM(o)})},S=m.isError&&o.some(E=>!E.subject_user_id);return u`
    <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h4 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${c.title}
          </h4>
          <p className="mt-2 text-xs leading-5 text-iron-300">
            ${c.instructions}
          </p>
        </div>
        ${d.data?.team_id&&u`<span className="shrink-0 rounded-md border border-white/[0.08] px-2 py-1 font-mono text-[10px] text-iron-500">
          ${d.data.team_id}
        </span>`}
      </div>

      <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          type="text"
          value=${n}
          onChange=${E=>r(E.target.value)}
          onKeyDown=${E=>E.key==="Enter"&&g()}
          placeholder=${c.inputPlaceholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <select
          value=${s}
          onChange=${E=>i(E.target.value)}
          disabled=${!y}
          className="h-9 min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
        >
          ${!y&&u`<option value="">${c.noSubjectsLabel}</option>`}
          ${y&&u`<option value="">${c.autoSubjectLabel}</option>`}
          ${h.map(E=>u`
              <option key=${E.subject_user_id} value=${E.subject_user_id}>
                ${E.display_name}
              </option>
            `)}
        </select>
        <${A}
          variant="secondary"
          size="sm"
          className="shrink-0"
          onClick=${g}
          disabled=${!n.trim()||!m.isSuccess}
        >
          ${c.addLabel}
        <//>
      </div>

      <div className="mb-3 rounded-lg border border-white/[0.06] bg-black/10">
        ${d.isLoading&&u`<div className="px-3 py-2 text-xs text-iron-400">${c.loadingMessage}</div>`}
        ${!d.isLoading&&o.length===0&&u`<div className="px-3 py-2 text-xs text-iron-500">
          ${c.emptyMessage}
        </div>`}
        ${o.map(E=>u`
            <label
              key=${E.channel_id}
              className="flex min-h-10 items-center justify-between gap-3 border-t border-white/[0.05] px-3 first:border-t-0"
            >
              <span className="min-w-0">
                <span className="block truncate font-mono text-xs text-iron-200">
                  ${E.channel_id}
                </span>
              </span>
              <div className="flex shrink-0 items-center gap-2">
                ${y?u`
                    <select
                      value=${E.subject_user_id}
                      onChange=${N=>b(E.channel_id,N.target.value)}
                      className="h-8 rounded-md border border-white/10 bg-white/[0.04] px-2 text-xs text-iron-100 outline-none focus:border-signal/45"
                    >
                      <option value="">${c.autoSubjectLabel}</option>
                      ${M_(f,E).map(N=>u`
                          <option key=${N.subject_user_id} value=${N.subject_user_id}>
                            ${N.display_name}
                          </option>
                        `)}
                    </select>
                  `:u`<span className="max-w-40 truncate text-xs text-iron-500">
                    ${E.subject_user_id?E.subject_display_name||E.subject_user_id:c.autoSubjectLabel}
                  </span>`}
                <input
                  type="checkbox"
                  checked=${!0}
                  aria-label=${c.allowLabel(E.channel_id)}
                  onChange=${()=>v(E.channel_id)}
                  className="h-4 w-4 rounded border-white/20 bg-white/[0.04] text-signal"
                />
              </div>
            </label>
          `)}
      </div>

      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <${A}
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick=${w}
          disabled=${!d.isSuccess||!x||$.isPending||S}
        >
          ${$.isPending?c.savingLabel:c.submitLabel}
        <//>
        ${$.isSuccess&&u`<p className="text-xs text-emerald-300">
          ${c.successMessage}
        </p>`}
        ${(d.isError||m.isError||$.isError)&&u`<p className="text-xs text-red-300">
          ${A_($.error||d.error||m.error,c.errorMessage)}
        </p>`}
      </div>
    </div>
  `}function M_(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function zh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return k_(Array.from(t.keys())).map(a=>t.get(a))}function PM(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function jM(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var qh=["slack-setup"],Gr={installationId:{body:"Local IronClaw name for this Slack install. Choose one and keep it stable.",example:"Example: local-slack"},teamId:{body:"Slack workspace/team ID from the workspace that installed the app.",example:"Example: T0123456789"},appId:{body:"Slack app Basic Information > App Credentials.",example:"Example: A0123456789"},botUser:{body:"Optional Reborn user. Blank uses the current WebUI operator.",example:"Example: user:operator"},sharedSubject:{body:"Optional default team agent for shared channel turns. Usually blank.",example:"Example: user:slack-shared"},botToken:{body:"Slack app OAuth & Permissions > Bot User OAuth Token.",example:"Example: xoxb-..."},signingSecret:{body:"Slack app Basic Information > App Credentials > Signing Secret.",example:""}};function j_({action:e}){let t=H({queryKey:qh,queryFn:N_}),a=t.data?.configured===!0;return u`
    <div className="space-y-3">
      <${UM} action=${e} setupQuery=${t} />
      ${a&&u`<${O_} action=${e} />`}
    </div>
  `}function UM({action:e,setupQuery:t}){let a=W(),[n,r]=p.default.useState(FM()),s=p.default.useRef(!1),i=p.default.useRef(!1),o=t.data,l=BM(e);p.default.useEffect(()=>{!o||s.current||i.current||(r(L_(o)),s.current=!0)},[o]);let c=V({mutationFn:__,onSuccess:h=>{i.current=!1,r(L_(h)),s.current=!0,a.setQueryData(qh,h),a.invalidateQueries({queryKey:qh}),a.invalidateQueries({queryKey:["slack-allowed-channels"]}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["extensions"]})}}),d=h=>x=>{i.current=!0,r(y=>({...y,[h]:x.target.value}))},m=()=>c.mutate(n),f=n.installation_id.trim()&&n.team_id.trim()&&n.api_app_id.trim()&&(o?.bot_token_configured||n.bot_token.trim())&&(o?.signing_secret_configured||n.signing_secret.trim());return u`
    <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h4 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${l.title}
          </h4>
          <p className="mt-2 text-xs leading-5 text-iron-300">
            ${l.instructions}
          </p>
        </div>
        ${o?.configured&&u`<span className="shrink-0 rounded-md border border-emerald-400/20 px-2 py-1 text-[10px] text-emerald-300">
          Configured
        </span>`}
      </div>

      <div className="grid gap-3 sm:grid-cols-3">
        ${hl("Installation ID",n.installation_id,d("installation_id"),"",Gr.installationId)}
        ${hl("Team ID",n.team_id,d("team_id"),"",Gr.teamId)}
        ${hl("App ID",n.api_app_id,d("api_app_id"),"",Gr.appId)}
        ${hl("Bot user",n.user_id,d("user_id"),"default operator",Gr.botUser)}
        ${hl("Shared subject",n.shared_subject_user_id,d("shared_subject_user_id"),"optional",Gr.sharedSubject)}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        ${P_("Bot token",n.bot_token,d("bot_token"),o?.bot_token_configured,Gr.botToken)}
        ${P_("Signing secret",n.signing_secret,d("signing_secret"),o?.signing_secret_configured,Gr.signingSecret)}
      </div>

      <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <${A}
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick=${m}
          disabled=${!f||c.isPending}
        >
          ${c.isPending?"Saving...":l.submitLabel}
        <//>
        ${t.isError&&u`<p className="text-xs text-red-300">
          ${Bh(t.error,l.errorMessage)}
        </p>`}
        ${c.isError&&u`<p className="text-xs text-red-300">
          ${Bh(c.error,l.errorMessage)}
        </p>`}
        ${c.isSuccess&&u`<p className="text-xs text-emerald-300">${l.successMessage}</p>`}
      </div>
    </div>
  `}function L_(e){return{installation_id:e.installation_id||"",team_id:e.team_id||"",api_app_id:e.api_app_id||"",user_id:e.user_id||"",shared_subject_user_id:e.shared_subject_user_id||"",bot_token:"",signing_secret:""}}function FM(){return{installation_id:"",team_id:"",api_app_id:"",user_id:"",shared_subject_user_id:"",bot_token:"",signing_secret:""}}function hl(e,t,a,n="",r=null){return u`
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-iron-500">${e}</span>
      <input
        type="text"
        value=${t}
        onChange=${a}
        placeholder=${n}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <${U_} help=${r} />
    </label>
  `}function P_(e,t,a,n,r=null){return u`
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-iron-500">${e}</span>
      <input
        type="password"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck=${!1}
        value=${t}
        onChange=${a}
        placeholder=${n?"Configured; leave blank to keep":""}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <${U_} help=${r} />
    </label>
  `}function U_({help:e}){return e?u`
    <p className="mt-1.5 min-h-8 text-[11px] leading-4 text-iron-400">
      <span className="block">${e.body}</span>
      ${e.example&&u`<span className="mt-0.5 block font-mono text-iron-300">${e.example}</span>`}
    </p>
  `:null}function BM(e){return{title:"Slack setup",instructions:e?.instructions||"Configure the Slack app before assigning channels.",submitLabel:"Save setup",successMessage:"Slack setup saved.",errorMessage:"Slack setup update failed."}}var Ih={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function Yr(e){return e==="wasm_channel"||e==="channel"}var F_={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},B_={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function z_(e){let t=q_(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||Yr(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function q_(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function Hh(e){let t=q_(e);return t==="active"||t==="ready"}function I_({extension:e,secrets:t=[],fields:a=[]}={}){return Hh(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var H_="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",K_="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",Q_="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",V_="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",G_="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",zM="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function Jr(e){return e.package_ref?.id||""}function qM({actions:e,isBusy:t,packageRefId:a}){let n=C(),[r,s]=p.default.useState(!1),i=p.default.useRef(null);return p.default.useEffect(()=>{if(!r)return;let o=l=>{i.current&&!i.current.contains(l.target)&&s(!1)};return document.addEventListener("mousedown",o),()=>document.removeEventListener("mousedown",o)},[r]),u`
    <div ref=${i} className="relative shrink-0">
      <button
        type="button"
        data-testid="extension-card-actions"
        data-extension-id=${a}
        aria-label=${n("extensions.moreActions")}
        aria-haspopup="true"
        aria-expanded=${r?"true":"false"}
        disabled=${t}
        onClick=${()=>s(o=>!o)}
        className="grid h-7 w-7 place-items-center rounded-md border border-transparent text-[var(--v2-text-faint)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] disabled:cursor-not-allowed disabled:opacity-50"
      >
        <${M} name="more" className="h-4 w-4" strokeWidth=${2.4} />
      </button>
      ${r&&u`
        <div
          role="menu"
          className="absolute right-0 top-8 z-10 min-w-[156px] rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-1 shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]"
        >
          ${e.map(o=>u`
              <button
                key=${o.id}
                type="button"
                data-testid=${`extension-menu-${o.id}`}
                data-extension-id=${a}
                role="menuitem"
                disabled=${t}
                onClick=${()=>{s(!1),o.run()}}
                className=${["flex w-full items-center gap-2.5 rounded-[7px] px-2.5 py-1.5 text-left text-[13px] disabled:cursor-not-allowed disabled:opacity-50",o.danger?"text-[var(--v2-danger-text)] hover:bg-[var(--v2-danger-soft)]":"text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)]"].join(" ")}
              >
                <${M} name=${o.icon||"settings"} className="h-3.5 w-3.5" />
                ${o.label}
              </button>
            `)}
        </div>
      `}
    </div>
  `}function Y_({items:e}){return!e||e.length===0?null:u`
    <div className="mt-3 flex flex-wrap gap-1">
      ${e.map(t=>u`<span key=${t} className=${zM}>${t}</span>`)}
    </div>
  `}function xi({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=C(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=F_[i]||"muted",l=s(`extensions.state.${i}`)||B_[i]||i,c=s(`extensions.kind.${e.kind}`)||Ih[e.kind]||e.kind,d=e.display_name||Jr(e),m=!!e.package_ref,f=e.tools||[],[h,x]=p.default.useState(!1),$=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],b=[],w=z_(e);w==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):w==="activate"&&v.push({id:"activate",label:"Activate",run:()=>t(g)}),m&&(e.needs_setup||e.has_auth)&&w!=="configure"&&b.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)}),m&&Yr(e.kind)&&(i==="setup_required"||i==="failed")&&b.push({id:"setup",label:"Setup",icon:"settings",run:()=>a(g)}),m&&Yr(e.kind)&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&b.push({id:"reconfigure",label:"Reconfigure",icon:"settings",run:()=>a(g)}),m&&b.push({id:"remove",label:s("common.remove")||"Remove",icon:"trash",danger:!0,run:()=>n(g)});let S=v[0];return u`
    <div
      className=${H_}
      data-testid="extension-card"
      data-extension-id=${Jr(e)}
      data-extension-state=${i}
    >
      <div className="flex items-start gap-2">
        <${z} tone=${o} label=${l} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${b.length>0&&u`<${qM}
          actions=${b}
          isBusy=${r}
          packageRefId=${Jr(e)}
        />`}
      </div>

      <div className=${K_}>
        <span>${c}</span>
        ${e.version&&u`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&u`<p className=${Q_}>${e.description}</p>`}

      ${e.activation_error&&u`
        <div
          className="mt-2 rounded-[10px] border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-1.5 text-xs text-[var(--v2-danger-text)]"
        >
          ${e.activation_error}
        </div>
      `}

      ${$&&u`
        <div className="mt-2 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2 text-xs leading-5 text-[var(--v2-text-muted)]">
          ${$}
        </div>
      `}

      <div className=${V_}>
        ${f.length>0?u`
              <button
                type="button"
                aria-expanded=${h?"true":"false"}
                onClick=${()=>x(E=>!E)}
                className=${G_}
              >
                <${M} name="layers" className="h-3.5 w-3.5" />
                <span>${f.length===1?s("extensions.oneCapability"):s("extensions.pluralCapabilities",{count:f.length})}</span>
                <${M}
                  name="chevron"
                  className=${["h-3 w-3",h?"rotate-180":""].join(" ")}
                />
              </button>
            `:u`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">No capabilities</span>`}
        <span className="flex-1"></span>
        ${S&&u`
          <${A}
            variant="secondary"
            size="sm"
            onClick=${S.run}
            disabled=${r}
            data-testid=${`extension-action-${S.id}`}
            data-extension-id=${Jr(e)}
          >
            ${S.label}
          <//>
        `}
      </div>

      ${h&&u`<${Y_} items=${f} />`}
    </div>
  `}function Xr({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=C(),s=r(`extensions.kind.${e.kind}`)||Ih[e.kind]||e.kind,i=e.display_name||Jr(e),o=!!(e.package_ref&&t),l=e.keywords||[],[c,d]=p.default.useState(!1);return u`
    <div
      className=${H_}
      data-testid="extension-registry-card"
      data-extension-id=${Jr(e)}
    >
      <div className="flex items-start gap-2">
        <${z}
          tone="muted"
          label=${n||r("extensions.state.available")||"available"}
          size="sm"
        />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${i}
        </span>
      </div>

      <div className=${K_}>
        <span>${s}</span>
        ${e.version&&u`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&u`<p className=${Q_}>${e.description}</p>`}

      <div className=${V_}>
        ${l.length>0?u`
              <button
                type="button"
                aria-expanded=${c?"true":"false"}
                onClick=${()=>d(m=>!m)}
                className=${G_}
              >
                <${M} name="list" className="h-3.5 w-3.5" />
                <span>${l.length===1?r("extensions.oneKeyword"):r("extensions.pluralKeywords",{count:l.length})}</span>
                <${M}
                  name="chevron"
                  className=${["h-3 w-3",c?"rotate-180":""].join(" ")}
                />
              </button>
            `:u`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]"></span>`}
        <span className="flex-1"></span>
        ${o&&u`
          <${A}
            variant="outline"
            size="sm"
            data-testid="extension-action-install"
            data-extension-id=${Jr(e)}
            onClick=${()=>t({packageRef:e.package_ref,displayName:i})}
            disabled=${a}
          >
            <${M} name="plus" className="mr-1.5 h-3.5 w-3.5" />
            Install
          <//>
        `}
      </div>

      ${c&&u`<${Y_} items=${l} />`}
    </div>
  `}function J_(){return K("/api/webchat/v2/extensions")}function X_(){return K("/api/webchat/v2/extensions/registry")}function W_(e){return K("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function Z_(e){return K(`/api/webchat/v2/extensions/${encodeURIComponent(vl(e))}/activate`,{method:"POST"})}function eR(e){return K(`/api/webchat/v2/extensions/${encodeURIComponent(vl(e))}/remove`,{method:"POST"})}function tR(e){return K(`/api/webchat/v2/extensions/${encodeURIComponent(vl(e))}/setup`)}function aR(e,t,a){return m$(vl(e),{action:"submit",payload:{secrets:t,fields:a}})}function nR(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return K(`/api/webchat/v2/extensions/${encodeURIComponent(vl(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function rR(){return Promise.resolve({requests:[]})}function sR(){return Promise.resolve({success:!1,message:"Pairing requires a v2 pairing endpoint."})}function vl(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var IM=2e3,HM=10*60*1e3;function $i(e){return e?.package_ref?.id||null}function Kh(e){return e?.display_name||$i(e)||""}function iR(e,t,a){return $i(t)||`${e}:${Kh(t)||"unknown"}:${a}`}function KM(e,t){return e.installed!==t.installed?e.installed?-1:1:Kh(e.entry||e.extension).localeCompare(Kh(t.entry||t.extension))}function oR(){let e=W(),t=H({queryKey:["gateway-status-extensions"],queryFn:ti,staleTime:1e4}),a=H({queryKey:["extensions"],queryFn:J_}),n=H({queryKey:["extension-registry"],queryFn:X_}),r=H({queryKey:["connectable-channels"],queryFn:Kc}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["extensions"]}),e.invalidateQueries({queryKey:["extension-registry"]}),e.invalidateQueries({queryKey:["gateway-status-extensions"]}),e.invalidateQueries({queryKey:["connectable-channels"]})},[e]),[i,o]=p.default.useState(null),l=p.default.useCallback(()=>o(null),[]),c=V({mutationFn:({packageRef:k})=>W_(k),onSuccess:(k,{displayName:U})=>{k.success?(o({type:"success",message:k.message||k.instructions||`${U||"Extension"} installed`}),k.auth_url&&window.open(k.auth_url,"_blank","noopener,noreferrer")):o({type:"error",message:k.message||"Install failed"}),s()},onError:k=>{o({type:"error",message:k.message}),s()}}),d=V({mutationFn:({packageRef:k})=>Z_(k),onSuccess:(k,{displayName:U})=>{k.success?(o({type:"success",message:k.message||k.instructions||`${U||"Extension"} activated`}),k.auth_url&&window.open(k.auth_url,"_blank","noopener,noreferrer")):k.auth_url?(window.open(k.auth_url,"_blank","noopener,noreferrer"),o({type:"info",message:"Opening authentication\u2026"})):k.awaiting_token?o({type:"info",message:"Configuration required"}):o({type:"error",message:k.message||"Activation failed"}),s()},onError:k=>{o({type:"error",message:k.message})}}),m=V({mutationFn:({packageRef:k})=>eR(k),onSuccess:(k,{displayName:U})=>{k.success?o({type:"success",message:`${U||"Extension"} removed`}):o({type:"error",message:k.message||"Remove failed"}),s()},onError:k=>{o({type:"error",message:k.message})}}),f=t.data||{},h=a.data?.extensions||[],x=n.data?.entries||[],y=r.data?.channels||[],$=new Map(h.map(k=>[$i(k),k]).filter(([k])=>!!k)),g=new Set(x.map(k=>$i(k)).filter(Boolean)),v=[...x.map((k,U)=>{let Y=$i(k),ae=Y&&$.get(Y)||null;return{id:iR("registry",k,U),installed:!!(ae||k.installed),entry:k,extension:ae}}),...h.filter(k=>{let U=$i(k);return!U||!g.has(U)}).map((k,U)=>({id:iR("installed",k,U),installed:!0,entry:null,extension:k}))].sort(KM),b=k=>Yr(k.kind),w=h.filter(b),S=h.filter(k=>k.kind==="mcp_server"),E=h.filter(k=>!b(k)&&k.kind!=="mcp_server"),N=x.filter(k=>b(k)&&!k.installed),T=x.filter(k=>k.kind==="mcp_server"&&!k.installed),L=x.filter(k=>k.kind!=="mcp_server"&&!b(k)&&!k.installed),O=a.isLoading||n.isLoading,P=c.isPending||d.isPending||m.isPending;return{status:f,extensions:h,channels:w,mcpServers:S,tools:E,channelRegistry:N,mcpRegistry:T,toolRegistry:L,registry:x,catalogEntries:v,connectableChannels:y,isLoading:O,isBusy:P,actionResult:i,clearResult:l,install:c.mutate,activate:d.mutate,remove:m.mutate,invalidate:s}}function lR(e){let t=H({queryKey:["extension-setup",e?.id||e],queryFn:()=>tR(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function uR(e,t){let a=W(),n=e?.id||e;return V({mutationFn:({secrets:r,fields:s})=>aR(e,r,s),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function cR(e){let t=W(),a=e?.id||e,n=p.default.useRef(null),r=p.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=p.default.useCallback(()=>{let l=t.getQueryData(["extension-setup",a]);if(l?.secrets?.length>0&&l.secrets.every(f=>f.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(f=>f.package_ref?.id===a),m=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return m==="active"||m==="ready"},[a,t]),o=p.default.useCallback(l=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||l&&l.closed||Date.now()-c>HM)&&(r(),s())},IM)},[r,s,i]);return p.default.useEffect(()=>r,[r]),V({mutationFn:({secret:l,popup:c})=>nR(e,l).then(d=>({res:d,popup:c})),onSuccess:({res:l,popup:c})=>{let d=c;l.authorization_url&&c&&!c.closed?c.location.href=l.authorization_url:l.authorization_url?d=window.open(l.authorization_url,"_blank","noopener,noreferrer"):c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(l,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function dR(e,t={}){let a=H({queryKey:["pairing",e],queryFn:()=>rR(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=W(),r=V({mutationFn:({code:s})=>sR(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function mR(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var QM={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function fR({channel:e,redeemFn:t,i18nKeys:a=QM,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=C(),o=typeof t=="function",l=dR(e,{enabled:!o}),c=W(),[d,m]=p.default.useState(""),f=VM(i,a,r),h=V({mutationFn:({code:S})=>t(e,S),onSuccess:()=>{m("");for(let S of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:S})}}),x=p.default.useCallback(S=>l.approve({code:S}),[l.approve]),y=p.default.useCallback(()=>{let S=d.trim();S&&(o?h.mutate({code:S}):(l.approve({code:S}),m("")))},[o,d,l.approve,h]),$=o?[]:l.requests,g=o?!1:l.isLoading,v=o?h.isPending:l.isApproving,b=o?h.isSuccess?h.data:null:l.result,w=o?h.isError?h.error:null:l.error;return g?u`
      <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
        <div className="v2-skeleton h-3 w-24 rounded" />
      </div>
    `:u`
    <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
      <h4 className="mb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        ${f.title}
      </h4>
      <p className="mb-4 text-xs leading-5 text-iron-300">${f.instructions}</p>

      <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          type="text"
          value=${d}
          onChange=${S=>m(S.target.value)}
          onKeyDown=${S=>S.key==="Enter"&&y()}
          placeholder=${f.placeholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${A}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${y}
          disabled=${v||!d.trim()}
        >
          ${f.action}
        <//>
      </div>

      ${b?.success&&u`<p className="mb-3 text-xs text-emerald-300">
        ${b.message||f.success}
      </p>`}
      ${b&&!b.success&&u`<p className="mb-3 text-xs text-red-300">
        ${b.message||f.error}
      </p>`}
      ${w&&u`<p className="mb-3 text-xs text-red-300">
        ${mR(w,f.error)}
      </p>`}

      ${s&&$.length>0?u`
            <div className="space-y-2">
              ${$.map(S=>u`
                <div
                  key=${S.code||S.id}
                  className="flex items-center justify-between gap-3 rounded-md border border-white/[0.06] bg-white/[0.02] px-3 py-2"
                >
                  <div className="min-w-0">
                    <span className="font-mono text-sm text-iron-200">${S.code||S.id}</span>
                    ${S.label&&u`
                      <span className="ml-2 text-xs text-iron-300">${S.label}</span>
                    `}
                  </div>
                  <${A}
                    variant="secondary"
                    className="h-7 px-2.5 text-xs"
                    onClick=${()=>x(S.code||S.id)}
                    disabled=${v}
                  >
                    ${f.action}
                  <//>
                </div>
              `)}
            </div>
          `:s&&u`<p className="text-xs text-iron-300">${i(a.empty)}</p>`}
    </div>
  `}function VM(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function gd(e){return e.package_ref?.id||""}function pR(e){return gd(e)==="slack"}function vR(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function gR(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function GM(e){let t=e||[],a=[t.find(vR),t.find(gR)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function hR({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>vR(r)?u`<${j_} action=${r.action} />`:gR(r)?u`<${Bc} action=${r.action} />`:null).filter(Boolean);return n.length>0?u`<div className="space-y-3">${n}</div>`:null}function yR({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:l}){let c=C(),d=t||[],m=e.enabled_channels||[],f=GM(a),h=d.some(pR),x=f.length>0&&!h;return u`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          ${c("channels.builtIn")}
        </h3>
        <${wi}
          name="Web Gateway"
          description=${c("channels.webGatewayDesc")||"Browser-based chat with SSE streaming"}
          enabled=${!0}
          detail=${"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)}
        />
        <${wi}
          name="HTTP Webhook"
          description=${c("channels.httpWebhookDesc")||"Inbound webhook endpoint for external integrations"}
          enabled=${m.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${wi}
          name="CLI"
          description=${c("channels.cliDesc")||"Terminal interface with TUI or simple REPL"}
          enabled=${m.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${wi}
          name="REPL"
          description=${c("channels.replDesc")||"Minimal read-eval-print loop for testing"}
          enabled=${m.includes("repl")}
          detail="ironclaw run --repl"
        />
        ${x&&u`
          <${wi}
            name=${c("channels.slack")||"Slack"}
            description=${c("channels.slackDesc")||"Tenant app channel for DMs and app mentions"}
            enabled=${!1}
            statusLabel="setup"
            statusTone="muted"
            detail=${c("channels.slackDetail")||"Tenant Slack app install"}
          >
            <${hR}
              slackConnectActions=${f}
            />
          </${wi}>
        `}
      </div>

      ${d.length>0&&u`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            ${c("channels.messaging")}
          </h3>
          <div className="grid grid-cols-1 gap-4">
            ${d.map(y=>u`
                <div key=${gd(y)} className="flex flex-col gap-3">
                  <${xi}
                    ext=${y}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${l}
                  />
                  ${pR(y)&&u`<${hR}
                    slackConnectActions=${f}
                  />`}
                  ${(y.onboarding_state==="pairing_required"||y.onboarding_state==="pairing")&&u` <${fR} channel=${gd(y)} /> `}
                </div>
              `)}
          </div>
        </div>
      `}
      ${n.length>0&&u`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            ${c("channels.availableChannels")}
          </h3>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            ${n.map(y=>u`
                <${Xr}
                  key=${gd(y)}
                  entry=${y}
                  onInstall=${o}
                  isBusy=${l}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function wi({name:e,description:t,enabled:a,detail:n,children:r,statusLabel:s=a?"on":"off",statusTone:i=a?"success":"muted"}){return u`
    <div
      className="border-t border-white/[0.06] py-4 first:border-0 first:pt-0"
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-iron-200">${e}</span>
            <${z}
              tone=${i}
              label=${s}
            />
          </div>
          <div className="mt-1 text-xs text-iron-300">${t}</div>
          ${n&&u`<div className="mt-1 font-mono text-[11px] text-iron-700">
            ${n}
          </div>`}
        </div>
      </div>
      ${r}
    </div>
  `}function bR({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=C(),s=e?.displayName||e?.packageRef?.id||"Extension",{secrets:i=[],fields:o=[],onboarding:l,isLoading:c,error:d}=lR(e?.packageRef),[m,f]=p.default.useState({}),[h,x]=p.default.useState({}),y=cR(e?.packageRef),$=uR(e?.packageRef,N=>{N.success!==!1&&(n&&n(N),a())}),g=p.default.useCallback(()=>{let N={};for(let[T,L]of Object.entries(m)){let O=(L||"").trim();O&&(N[T]=O)}$.mutate({secrets:N,fields:h})},[m,h,$]),v=p.default.useCallback(N=>{let T=window.open("about:blank","_blank","width=600,height=600");T&&(T.opener=null),y.mutate({secret:N,popup:T})},[y]),w=i.filter(N=>(N.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,S=Hh(e),E=I_({extension:e,secrets:i,fields:o});return c?u`
      <${yd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(N=>u`<div
                key=${N}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?u`
      <${yd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")||"Failed to load setup:"} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?u`
      <${yd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")||"No configuration required for this extension."}
        </p>
      <//>
    `:u`
    <${yd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
      ${l?.credential_instructions&&u`
        <p className="mb-4 text-sm leading-6 text-iron-300">
          ${l.credential_instructions}
        </p>
      `}
      ${l?.setup_url&&u`
        <a
          href=${l.setup_url}
          target="_blank"
          rel="noopener noreferrer"
          className="mb-4 inline-flex items-center gap-1.5 text-sm text-signal hover:underline"
        >
          Get credentials
          <${M} name="bolt" className="h-3.5 w-3.5" />
        </a>
      `}

      <div className="space-y-4">
        ${i.map(N=>u`
            <div key=${N.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                ${N.prompt||N.name}
                ${N.optional&&u`
                  <span className="font-mono text-[10px] text-iron-700"
                    >${r("common.optional")||"optional"}</span
                  >
                `}
                ${N.provided&&u`
                  <span className="font-mono text-[10px] text-mint"
                    >${r("common.configured")||"configured"}</span
                  >
                `}
              </label>
              ${(N.setup?.kind||"manual_token")==="oauth"?u`
                    <div className="flex items-center justify-between gap-3 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2">
                      <span className="text-xs text-iron-300">
                        ${N.provided?r("extensions.authConfigured")||"Authorization is configured.":r("extensions.authPopup")||"Authorize this provider in a browser popup."}
                      </span>
                      <${A}
                        variant=${N.provided?"secondary":"primary"}
                        onClick=${()=>v(N)}
                        disabled=${y.isPending}
                      >
                        ${y.isPending?r("extensions.opening"):N.provided?r("extensions.reconnect"):r("extensions.authorize")}
                      <//>
                    </div>
                  `:u`
              <input
                type="password"
                placeholder=${N.provided?"\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (leave blank to keep)":""}
                value=${m[N.name]||""}
                onChange=${T=>f(L=>({...L,[N.name]:T.target.value}))}
                onKeyDown=${T=>T.key==="Enter"&&g()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
              ${N.auto_generate&&!N.provided&&u`
                <p className="mt-1 text-xs text-iron-700">
                  ${r("extensions.autoGenerated")||"Auto-generated if left blank"}
                </p>
              `}
                  `}
            </div>
          `)}
        ${o.map(N=>u`
            <div key=${N.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                ${N.prompt||N.name}
                ${N.optional&&u`
                  <span className="font-mono text-[10px] text-iron-700"
                    >${r("common.optional")||"optional"}</span
                  >
                `}
              </label>
              <input
                type="text"
                placeholder=${N.placeholder||""}
                value=${h[N.name]||""}
                onChange=${T=>x(L=>({...L,[N.name]:T.target.value}))}
                onKeyDown=${T=>T.key==="Enter"&&g()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
            </div>
          `)}
      </div>

      ${l?.credential_next_step&&u`
        <p className="mt-4 text-xs leading-5 text-iron-300">
          ${l.credential_next_step}
        </p>
      `}
      ${S&&u`
        <div
          className="mt-4 rounded-md border border-mint/20 bg-mint/10 px-3 py-2 text-xs text-mint"
        >
          ${r("extensions.activeConfigured")}
        </div>
      `}
      ${$.error&&u`
        <div
          className="mt-4 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
        >
          ${$.error.message}
        </div>
      `}
      ${y.error&&u`
        <div
          className="mt-4 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
        >
          ${y.error.message}
        </div>
      `}

      <div className="mt-6 flex items-center justify-end gap-3">
        <${A} variant="ghost" onClick=${a}>${r("common.cancel")||"Cancel"}<//>
        ${E&&u`
        <${A}
          variant="primary"
          onClick=${()=>t?.(e)}
        >
          Activate
        <//>
        `}
        ${w&&u`
        <${A}
          variant=${E?"secondary":"primary"}
          onClick=${g}
          disabled=${$.isPending}
        >
          ${$.isPending?"Saving\u2026":r("common.save")||"Save"}
        <//>
        `}
      </div>
    <//>
  `}function yd({onClose:e,title:t,children:a}){return p.default.useEffect(()=>{let n=r=>{r.key==="Escape"&&e()};return window.addEventListener("keydown",n),()=>window.removeEventListener("keydown",n)},[e]),u`
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick=${n=>{n.target===n.currentTarget&&e()}}
    >
      <div
        className="v2-panel mx-4 w-full max-w-lg rounded-2xl p-6"
        onClick=${n=>n.stopPropagation()}
      >
        <div className="mb-5 flex items-center justify-between">
          <h3 className="text-lg font-semibold text-white">${t}</h3>
          <button
            onClick=${e}
            className="grid h-8 w-8 place-items-center rounded-md text-iron-300 hover:bg-white/[0.06] hover:text-white"
          >
            <${M} name="close" className="h-4 w-4" />
          </button>
        </div>
        ${a}
      </div>
    </div>
  `}function xR(e){return e.package_ref?.id||""}function $R({mcpServers:e,mcpRegistry:t,onActivate:a,onConfigure:n,onRemove:r,onInstall:s,isBusy:i}){let o=C();return e.length===0&&t.length===0?u`
      <div className="v2-panel rounded-[18px] p-6 sm:p-8">
        <h3 className="text-lg font-semibold text-white">${o("extensions.emptyMcpTitle")}</h3>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${o("extensions.emptyMcpDesc")}
        </p>
      </div>
    `:u`
    <div className="space-y-5">
      ${e.length>0&&u`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            ${o("mcp.installed")}
          </h3>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            ${e.map(l=>u`
                <${xi}
                  key=${xR(l)}
                  ext=${l}
                  onActivate=${a}
                  onConfigure=${n}
                  onRemove=${r}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
      ${t.length>0&&u`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            Available MCP servers
          </h3>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            ${t.map(l=>u`
                <${Xr}
                  key=${xR(l)}
                  entry=${l}
                  onInstall=${s}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function YM(e){return e?.package_ref?.id||""}function JM(e){return e.entry||e.extension||{}}function wR({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=C(),[o,l]=p.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(y=>{let $=JM(y);return($.display_name||YM($)).toLowerCase().includes(c)||($.description||"").toLowerCase().includes(c)||($.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,m=d.filter(y=>y.installed&&y.extension),f=d.filter(y=>y.installed&&!y.extension&&y.entry),h=m.length+f.length,x=d.filter(y=>!y.installed&&y.entry);return e.length===0?u`
      <div className="v2-panel rounded-[18px] p-6 sm:p-8">
        <h3 className="text-lg font-semibold text-white">
          ${i("ext.registry.emptyTitle")}
        </h3>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${i("ext.registry.emptyDesc")}
        </p>
      </div>
    `:u`
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <input
          type="text"
          value=${o}
          onChange=${y=>l(y.target.value)}
          placeholder=${i("ext.registry.searchPlaceholder")}
          className="h-9 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <span className="font-mono text-[11px] text-iron-700">
          ${d.length} / ${e.length}
        </span>
      </div>

      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        ${d.length===0?u`<p className="py-4 text-sm text-iron-300">
              ${i("ext.registry.noMatch")}
            </p>`:u`
              ${h>0&&u`
                <h3
                  className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
                >
                  ${i("extensions.installed")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${m.map(y=>u`
                      <${xi}
                        key=${y.id}
                        ext=${y.extension||y.entry}
                        onActivate=${a}
                        onConfigure=${n}
                        onRemove=${r}
                        isBusy=${s}
                      />
                    `)}
                  ${f.map(y=>u`
                      <${Xr}
                        key=${y.id}
                        entry=${y.entry}
                        statusLabel=${i("extensions.installed")}
                        isBusy=${s}
                      />
                    `)}
                </div>
              `}

              ${x.length>0&&u`
                <h3
                  className=${["mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal",h>0?"mt-6":""].join(" ")}
                >
                  ${i("ext.registry.availableTitle")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${x.map(y=>u`
                      <${Xr}
                        key=${y.id}
                        entry=${y.entry}
                        onInstall=${t}
                        isBusy=${s}
                      />
                    `)}
                </div>
              `}
            `}
      </div>
    </div>
  `}function Qh(){let{tab:e="registry"}=st(),[t,a]=p.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:l,connectableChannels:c,isLoading:d,isBusy:m,actionResult:f,clearResult:h,install:x,activate:y,remove:$,invalidate:g}=oR(),v=p.default.useCallback(N=>a(N),[]),b=p.default.useCallback(()=>a(null),[]),w=p.default.useCallback(()=>g(),[g]),S=p.default.useCallback(N=>{N&&(y(N),a(null))},[y]);if(d)return u`
      <div className="flex h-full flex-col overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${[1,2,3].map(N=>u`
                <div
                  key=${N}
                  className="flex items-center justify-between border-t border-white/[0.06] py-4 first:border-0"
                >
                  <div>
                    <div className="v2-skeleton h-4 w-40 rounded" />
                    <div className="v2-skeleton mt-2 h-3 w-56 rounded" />
                  </div>
                  <div className="v2-skeleton h-7 w-16 rounded-full" />
                </div>
              `)}
          </div>
        </div>
      </div>
    `;if(e==="installed")return u`<${it} to="/extensions/registry" replace />`;let E={channels:u`<${yR}
      status=${n}
      channels=${r}
      connectableChannels=${c}
      channelRegistry=${i}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${x}
      isBusy=${m}
    />`,mcp:u`<${$R}
      mcpServers=${s}
      mcpRegistry=${o}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${x}
      isBusy=${m}
    />`,registry:u`<${wR}
      catalogEntries=${l}
      onInstall=${x}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      isBusy=${m}
    />`};return E[e]?u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <${$_} result=${f} onDismiss=${h} />
          ${E[e]}
        </div>
      </div>

      ${t&&u`
        <${bR}
          extension=${t}
          onActivate=${S}
          onClose=${b}
          onSaved=${w}
        />
      `}
    </div>
  `:u`<${it} to="/extensions/registry" replace />`}var SR=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],NR=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],_R=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],Vh=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function RR(e){return String(e||"").trim().toLowerCase()}function kR(e){if(e==null)return"";if(Array.isArray(e))return e.map(kR).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function et(e,t){let a=RR(e);return a?t.map(kR).join(" ").toLowerCase().includes(a):!0}function Si(e,t,a,n){let r=RR(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(l=>et(r,[i,l.key,l.labelKey?n(l.labelKey):l.label,l.descKey?n(l.descKey):l.description,t[l.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function XM({visible:e}){let t=C();return e?u`
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      ${t("tools.saved")}
    </span>
  `:null}function WM({checked:e,onChange:t,label:a}){return u`
    <button
      type="button"
      role="switch"
      aria-checked=${e}
      aria-label=${a}
      onClick=${()=>t(!e)}
      className=${["relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border",e?"border-signal/40 bg-signal/30":"border-white/15 bg-white/[0.06]"].join(" ")}
    >
      <span
        className=${["pointer-events-none inline-block h-5 w-5 rounded-full",e?"translate-x-5 bg-signal":"translate-x-0 bg-iron-300"].join(" ")}
      />
    </button>
  `}function ZM({field:e,value:t,onSave:a,isSaved:n}){let r=C(),[s,i]=p.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",l=e.descKey?r(e.descKey):e.description||"";p.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=p.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let m=parseInt(d,10);isNaN(m)||a(e.key,m)}else if(e.type==="float"){let m=parseFloat(d);isNaN(m)||a(e.key,m)}else a(e.key,d)},[e.key,e.type,a]);return u`
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">${o}</div>
        ${l&&u`<div className="mt-1 text-xs leading-5 text-iron-300">${l}</div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${e.type==="boolean"?u`
              <${WM}
                checked=${t===!0||t==="true"}
                onChange=${d=>a(e.key,d?"true":"false")}
                label=${o}
              />
            `:e.type==="select"?u`
              <select
                value=${s}
                onChange=${d=>{i(d.target.value),c(d.target.value)}}
                aria-label=${o}
                className="v2-select h-9 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
              >
                <option value="">${r("tools.default")}</option>
                ${e.options.map(d=>u`<option key=${d} value=${d}>${d}</option>`)}
              </select>
            `:u`
              <input
                type=${e.type==="float"||e.type==="number"?"number":"text"}
                value=${s}
                onChange=${d=>i(d.target.value)}
                onBlur=${d=>c(d.target.value)}
                onKeyDown=${d=>d.key==="Enter"&&c(d.target.value)}
                step=${e.step!==void 0?String(e.step):e.type==="float"?"any":"1"}
                min=${e.min!==void 0?String(e.min):void 0}
                max=${e.max!==void 0?String(e.max):void 0}
                placeholder=${r("tools.default")}
                aria-label=${o}
                className="h-9 w-36 rounded-md border border-white/12 bg-white/[0.04] px-3 text-right font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
            `}
        <${XM} visible=${n} />
      </div>
    </div>
  `}function Ni({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=C(),o=t?i(t):e||"";return u`
    <${te} className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${o}</h3>
      <div>
        ${a.map(l=>u`
              <${ZM}
                key=${l.key}
                field=${l}
                value=${n[l.key]}
                onSave=${r}
                isSaved=${s[l.key]}
              />
            `)}
      </div>
    <//>
  `}function wt({query:e}){let t=C();return u`
    <${te} padding="lg">
      <div className="flex items-center gap-3">
        <span
          className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-faint)]"
        >
          <${M} name="search" className="h-4 w-4" />
        </span>
        <div className="min-w-0">
          <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
            ${t("settings.noMatchingSettings",{query:e})}
          </h3>
        </div>
      </div>
    <//>
  `}function CR({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=C();if(n)return u`<${eO} />`;let i=Si(NR,e,r,s);return i.length===0?u`<${wt} query=${r} />`:u`
    <div className="space-y-5">
      ${i.map(o=>u`
            <${Ni}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function eO(){return u`
    <div className="space-y-5">
      ${[1,2,3].map(e=>u`
            <${te} key=${e} padding="md">
              <div className="mb-4 h-3 w-20 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              ${[1,2,3,4].map(t=>u`
                    <div
                      key=${t}
                      className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0"
                    >
                      <div>
                        <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                        <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                      </div>
                      <div className="h-9 w-36 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                    </div>
                  `)}
            <//>
          `)}
    </div>
  `}function ER(){let e=H({queryKey:["gateway-status-settings"],queryFn:ti,staleTime:1e4}),t=H({queryKey:["extensions"],queryFn:ow}),a=H({queryKey:["extension-registry"],queryFn:lw}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(m=>m.kind==="wasm_channel"||m.kind==="channel"),o=s.filter(m=>(m.kind==="wasm_channel"||m.kind==="channel")&&!m.installed),l=r.filter(m=>m.kind==="mcp_server"),c=s.filter(m=>m.kind==="mcp_server"&&!m.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:l,mcpRegistry:c,extensions:r,isLoading:d}}function tO({name:e,description:t,enabled:a,detail:n}){let r=C();return u`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${e}</span>
          <${z}
            tone=${a?"positive":"muted"}
            label=${r(a?"channels.statusOn":"channels.statusOff")}
            size="sm"
          />
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${t}</div>
        ${n&&u`<div className="mt-1 font-mono text-[11px] text-[var(--v2-text-faint)]">
          ${n}
        </div>`}
      </div>
    </div>
  `}function TR({channel:e,registryEntry:t}){let a=C(),n=t?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},l={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return u`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${n}</span>
          ${s?u`<${z}
                tone=${o[i]||"muted"}
                label=${l[i]||i}
                size="sm"
              />`:u`<${z}
                tone="muted"
                label=${a("channels.available")}
                size="sm"
              />`}
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${r}</div>
      </div>
    </div>
  `}function aO(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function nO({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=aO(e,i).filter(x=>et(s,[i("channels.builtIn"),x.id,x.name,x.description,x.detail])),l=new Set(t.map(x=>x.name)),c=t.filter(x=>et(s,[i("channels.messaging"),x.name,x.display_name,x.description,x.onboarding_state])),d=a.filter(x=>!l.has(x.name)).filter(x=>et(s,[i("channels.messaging"),x.name,x.display_name,x.description])),m=new Set(n.map(x=>x.name)),f=n.filter(x=>et(s,[i("channels.mcpServers"),x.name,x.display_name,x.description,x.active?i("channels.active"):i("channels.inactive")])),h=r.filter(x=>!m.has(x.name)).filter(x=>et(s,[i("channels.mcpServers"),x.name,x.display_name,x.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:h}}function AR({searchQuery:e=""}){let t=C(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=ER();if(o)return u`
      <div className="space-y-5">
        <${te} padding="md">
          <div className="mb-4 h-3 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(h=>u`
              <div
                key=${h}
                className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0"
              >
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="h-6 w-16 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
              </div>
            `)}
        <//>
      </div>
    `;let{builtInChannels:l,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:f}=nO({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return l.length===0&&c.length===0&&d.length===0&&m.length===0&&f.length===0?u`<${wt} query=${e} />`:u`
    <div className="space-y-5">
      ${l.length>0&&u`
      <${te} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${l.map(h=>u`
            <${tO}
              key=${h.id}
              name=${h.name}
              description=${h.description}
              enabled=${h.enabled}
              detail=${h.detail}
            />
          `)}
      <//>
      `}

      ${(c.length>0||d.length>0)&&u`
        <${te} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.messaging")}
          </h3>
          ${c.map(h=>u`
              <${TR}
                key=${h.name}
                channel=${h}
                registryEntry=${r.find(x=>x.name===h.name)}
              />
            `)}
          ${d.map(h=>u`
              <${TR} key=${h.name} registryEntry=${h} />
            `)}
        <//>
      `}
      ${(m.length>0||f.length>0)&&u`
        <${te} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.mcpServers")}
          </h3>
          ${m.map(h=>u`
                <div
                  key=${h.name}
                  className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-[var(--v2-text)]"
                        >${h.display_name||h.name}</span
                      >
                      <${z}
                        tone=${h.active?"positive":"muted"}
                        label=${h.active?t("channels.active"):t("channels.inactive")}
                        size="sm"
                      />
                    </div>
                    <div className="mt-1 text-xs text-[var(--v2-text-muted)]">
                      ${h.description||""}
                    </div>
                  </div>
                </div>
              `)}
          ${f.map(h=>u`
                <div
                  key=${h.name}
                  className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-[var(--v2-text)]"
                        >${h.display_name||h.name}</span
                      >
                      <${z}
                        tone="muted"
                        label=${t("channels.available")}
                        size="sm"
                      />
                    </div>
                    <div className="mt-1 text-xs text-[var(--v2-text-muted)]">
                      ${h.description||""}
                    </div>
                  </div>
                </div>
              `)}
        <//>
      `}
    </div>
  `}function DR({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:l,onNearaiWallet:c,onCodexLogin:d,loginBusy:m}){let f=C(),h=e.id===t,x=Hr(e,n),y=ri(e,n),$=$w(e,n,t,a),g=Cc(e,n),v=ww(e),b=f(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[w,S]=p.default.useState(h),E=p.default.useCallback(()=>S(ot=>!ot),[]);p.default.useEffect(()=>{S(h)},[h]);let N=x?u`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${nl(e.adapter)} · ${$||e.default_model||f("llm.none")}
      </span>`:u`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${b}
      </span>`,T=e.id==="nearai"||e.id==="openai_codex",L=e.api_key_set===!0||e.has_api_key===!0,O=e.builtin?e.id==="nearai"&&v&&!L?f("llm.addApiKey"):f("llm.configure"):f("common.edit"),P=v&&e.builtin?u`
          <${A}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${r}
            onClick=${()=>i(e)}
          >
            ${O}
          <//>
        `:null,k=!h&&e.id==="nearai"?u`
          ${P}
          <${A}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${m}
            onClick=${c}
            data-testid="llm-provider-nearai-wallet-login"
            data-provider-id=${e.id}
          >
            ${f("onboarding.nearWallet")}
          <//>
          <${A}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${m}
            onClick=${()=>l("github")}
            data-testid="llm-provider-nearai-github-login"
            data-provider-id=${e.id}
          >
            GitHub
          <//>
          <${A}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${m}
            onClick=${()=>l("google")}
            data-testid="llm-provider-nearai-google-login"
            data-provider-id=${e.id}
          >
            Google
          <//>
        `:!h&&e.id==="openai_codex"?u`
          <${A}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${m}
            onClick=${d}
            data-testid="llm-provider-codex-login"
            data-provider-id=${e.id}
          >
            ${f("onboarding.codexSignIn")}
          <//>
        `:null,Y=!h&&x&&(!T||e.id==="nearai"&&e.has_api_key===!0)?u`
        <${A}
          type="button"
          variant="primary"
          size="sm"
          disabled=${r}
          onClick=${()=>s(e)}
        >
          ${f("llm.use")}
        <//>
      `:null,ae=x?null:u`
        <${A}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${r}
          onClick=${()=>i(e)}
        >
          ${f(g==="api_key"?"llm.addApiKey":"llm.configure")}
        <//>
      `,se=h?null:Y||(T?k:ae),ie=!T&&(e.builtin&&e.id!=="bedrock"||!e.builtin)||e.id==="nearai"&&v;return u`
    <${te}
      padding="none"
      data-testid="llm-provider-card"
      data-provider-id=${e.id}
      className=${["transition-colors",h?"border-[color-mix(in_srgb,var(--v2-positive-text)_36%,var(--v2-panel-border))]":w?"border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))]":""].join(" ")}
    >
      <div className="flex w-full items-stretch hover:bg-[var(--v2-surface-soft)]">
        <button
          type="button"
          aria-expanded=${w?"true":"false"}
          aria-label=${f(w?"llm.collapseDetails":"llm.expandDetails")}
          data-testid="llm-provider-disclosure"
          onClick=${E}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 px-4 py-3 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)] sm:pl-5 sm:pr-3"
        >
          <span
            className=${["h-2 w-2 shrink-0 rounded-full",h?"bg-[var(--v2-positive-text)]":x?"bg-[var(--v2-accent)]":"bg-[var(--v2-warning-text)]"].join(" ")}
          />
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
            <span className="min-w-0 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
              ${e.name||e.id}
            </span>
            <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">${e.id}</span>
            ${h&&u`<${z} tone="positive" label=${f("llm.active")} size="sm" />`}
            ${e.builtin&&!h&&u`<${z} tone="muted" label=${f("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${N}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 py-3 pr-4 sm:pr-5">
          ${se}
          <button
            type="button"
            onClick=${E}
            data-testid="llm-provider-chevron"
            aria-label=${f(w?"llm.collapseDetails":"llm.expandDetails")}
            className=${["grid h-7 w-7 place-items-center rounded-md text-[var(--v2-text-faint)] transition-transform hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]",w?"rotate-180":""].join(" ")}
          >
            <${M} name="chevron" className="h-4 w-4" />
          </button>
        </div>
      </div>

      ${w&&u`
        <div data-testid="llm-provider-details" className="border-t border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-4 sm:px-5">
          <div className="grid gap-3 text-xs text-[var(--v2-text-muted)] sm:grid-cols-3">
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${f("llm.adapter")}</div>
              <div className="mt-1 truncate">${nl(e.adapter)}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${f("llm.baseUrl")}</div>
              <div className="mt-1 truncate font-mono">${y||f("llm.none")}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${f("llm.model")}</div>
              <div className="mt-1 truncate font-mono">${$||f("llm.none")}</div>
            </div>
          </div>

          <div className="mt-4 flex flex-wrap justify-end gap-2 border-t border-[var(--v2-panel-border)] pt-3">
            ${ie&&u`
              <${A}
                type="button"
                variant="secondary"
                size="sm"
                disabled=${r}
                onClick=${()=>i(e)}
              >
                ${O}
              <//>
            `}
            ${!e.builtin&&!h&&u`
              <${A}
                type="button"
                variant="danger"
                size="sm"
                disabled=${r}
                onClick=${()=>o(e)}
              >
                ${f("common.delete")}
              <//>
            `}
          </div>
        </div>
      `}
    <//>
  `}var rO=[{key:"active",labelKey:"llm.groupActive",dotClass:"bg-[var(--v2-positive-text)]"},{key:"ready",labelKey:"llm.groupReady",dotClass:"bg-[var(--v2-accent)]"},{key:"setup",labelKey:"llm.groupSetup",dotClass:"bg-[var(--v2-warning-text)]"}];function sO({label:e,count:t,dotClass:a}){return u`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full "+a} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${t}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `}function MR({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=C(),r=Wc({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=Zc(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return u`<${wt} query=${a} />`;let l=Sw(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return u`
    <${te} className="p-4 sm:p-6">
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${n("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">${n("llm.providersDesc")}</p>
        </div>
        <${A} type="button" variant="secondary" size="sm" className="gap-2" onClick=${()=>r.openDialog(null)}>
          <${M} name="plus" className="h-3.5 w-3.5" />
          ${n("llm.addProvider")}
        <//>
      </div>

      ${r.message&&u`
        <div
          className=${["mb-4 rounded-md border px-3 py-2 text-sm",r.message.tone==="error"?"border-red-400/30 bg-red-500/10 text-red-200":"border-mint/30 bg-mint/10 text-mint"].join(" ")}
          role="status"
        >
          ${r.message.text}
        </div>
      `}

      <${Xc} login=${i} />

      ${s.isLoading?u`<div className="text-sm text-[var(--v2-text-muted)]">${n("common.loading")}</div>`:s.error?u`<div className="text-sm text-red-200">${n("error.loadFailed",{what:n("llm.providers"),message:s.error.message})}</div>`:u`
            <div className="space-y-1">
              ${rO.flatMap(c=>{let d=l[c.key];return d.length?[u`
                    <section
                      key=${c.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${c.key}
                      className="mb-3"
                    >
                      <${sO}
                        label=${n(c.labelKey)}
                        count=${d.length}
                        dotClass=${c.dotClass}
                      />
                      <div className="space-y-2">
                      ${d.map(m=>u`
                          <${DR}
                            key=${m.id}
                            provider=${m}
                            activeProviderId=${s.activeProviderId}
                            selectedModel=${s.selectedModel}
                            builtinOverrides=${s.builtinOverrides}
                            isBusy=${s.isBusy}
                            onUse=${r.handleUse}
                            onConfigure=${r.openDialog}
                            onDelete=${r.handleDelete}
                            onNearaiLogin=${i.startNearai}
                            onNearaiWallet=${i.startNearaiWallet}
                            onCodexLogin=${i.startCodex}
                            loginBusy=${o}
                          />
                        `)}
                      </div>
                    </section>
                  `]:[]})}
            </div>
          `}

      <${Jc}
        open=${r.isDialogOpen}
        provider=${r.dialogProvider}
        allProviderIds=${r.allProviderIds}
        builtinOverrides=${s.builtinOverrides}
        onClose=${r.closeDialog}
        onSave=${r.handleSave}
        onTest=${s.testConnection}
        onListModels=${s.listModels}
      />
    <//>
  `}function OR({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=C(),{activeProviderId:o,selectedModel:l,providers:c,hasActiveProvider:d}=si({settings:e,gatewayStatus:t});if(r)return u`<${iO} />`;let m=d?o:"",f=c.find(g=>g.id===o),h=d&&(l||f?.default_model||e.selected_model)||"",x=Si(SR,e,s,i),y=et(s,[i("inference.provider"),i("inference.backend"),m,i("inference.model"),h]),$=et(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!y&&!$&&x.length===0?u`<${wt} query=${s} />`:u`
    <div className="space-y-5">
      ${y&&u`
      <${te} padding="none" className="p-4 sm:p-5">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${i("inference.provider")}</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${i("inference.backend")}</div>
            <div className="mt-1 flex items-center gap-2">
              <span className="font-mono text-lg font-semibold text-[var(--v2-text-strong)]">${m||i("inference.none")}</span>
              ${d?u`<${z} tone="positive" label=${i("inference.active")} size="sm" />`:u`<${z} tone="muted" label=${i("llm.notConfigured")} size="sm" />`}
            </div>
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${i("inference.model")}</div>
            <div className="mt-1 font-mono text-lg font-semibold text-[var(--v2-text-strong)]">
              ${h||i("inference.none")}
            </div>
          </div>
        </div>
      <//>
      `}

      ${$&&u`
        <${MR}
          settings=${e}
          gatewayStatus=${t}
          searchQuery=${s}
        />
      `}

      ${x.map(g=>u`
            <${Ni}
              key=${g.groupKey}
              groupKey=${g.groupKey}
              fields=${g.fields}
              settings=${e}
              onSave=${a}
              savedKeys=${n}
            />
          `)}
    </div>
  `}function mr({className:e=""}){return u`
    <div
      className=${"rounded animate-pulse bg-[var(--v2-surface-muted)] "+e}
    />
  `}function iO(){return u`
    <div className="space-y-5">
      <${te} padding="md">
        <${mr} className="mb-4 h-3 w-24" />
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${mr} className="h-3 w-16" />
            <${mr} className="mt-2 h-6 w-28" />
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${mr} className="h-3 w-16" />
            <${mr} className="mt-2 h-6 w-40" />
          </div>
        </div>
      <//>
      ${[1,2].map(e=>u`
            <${te} key=${e} padding="md">
              <${mr} className="mb-4 h-3 w-20" />
              ${[1,2,3].map(t=>u`
                    <div key=${t} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                      <${mr} className="h-4 w-32" />
                      <${mr} className="h-9 w-36" />
                    </div>
                  `)}
            <//>
          `)}
    </div>
  `}function LR({searchQuery:e=""}){let t=C(),{lang:a,setLang:n}=gl(),r=yl.find(i=>i.code===a)||yl[0],s=yl.filter(i=>et(e,[i.code,i.name,i.native]));return s.length===0?u`<${wt} query=${e} />`:u`
    <${te} padding="md">
      <h3 className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${t("lang.title")}
      </h3>
      <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
        ${t("lang.description")}
      </p>

      <div className="mt-5 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
        <div className="text-xs text-[var(--v2-text-muted)]">${t("lang.current")}</div>
        <div className="mt-1 flex items-baseline gap-2">
          <span className="text-lg font-semibold text-[var(--v2-text-strong)]">${r.native}</span>
          <span className="font-mono text-xs text-[var(--v2-text-faint)]">${r.name}</span>
        </div>
      </div>

      <div className="mt-4 grid gap-2 sm:grid-cols-2">
        ${s.map(i=>u`
            <button
              key=${i.code}
              type="button"
              onClick=${()=>n(i.code)}
              className=${["flex items-center justify-between gap-3 rounded-xl border px-4 py-3 text-left",i.code===a?"border-[color-mix(in_srgb,var(--v2-accent)_35%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]":"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_20%,var(--v2-panel-border))] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"].join(" ")}
            >
              <div className="min-w-0">
                <div className="truncate text-sm font-medium">${i.native}</div>
                <div className="truncate font-mono text-[11px] text-[var(--v2-text-faint)]">${i.name}</div>
              </div>
              <div className="shrink-0 font-mono text-[11px] text-[var(--v2-text-faint)]">${i.code}</div>
            </button>
          `)}
      </div>
    <//>
  `}function PR({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=C();if(n)return u`
      <div className="space-y-5">
        ${[1,2].map(o=>u`
              <${te} key=${o} padding="md">
                <div className="mb-4 h-3 w-20 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                ${[1,2].map(l=>u`
                      <div key=${l} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                        <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                        <div className="h-9 w-36 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                      </div>
                    `)}
              <//>
            `)}
      </div>
    `;let i=Si(_R,e,r,s);return i.length===0?u`<${wt} query=${r} />`:u`
    <div className="space-y-5">
      ${i.map(o=>u`
            <${Ni}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function jR(){let e=C(),[t,a]=p.default.useState(!1),n=p.default.useCallback(()=>a(!0),[]),r=p.default.useCallback(()=>a(!1),[]),s=p.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function UR({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=C(),r=jR({gatewayStatus:t,gatewayStatusQuery:a});return e?u`
    <div className="space-y-3">
      <div
        role="alert"
        className="flex flex-col gap-3 rounded-xl border border-copper/30 bg-copper/10 px-4 py-3 sm:flex-row sm:items-center"
      >
        <div className="flex min-w-0 flex-1 items-start gap-3">
          <${M} name="bolt" className="mt-0.5 h-4 w-4 shrink-0 text-copper" />
          <div className="min-w-0">
            <p className="text-sm text-copper">
              ${n("settings.restartRequired")}
            </p>
            ${!r.restartEnabled&&u`
              <p className="mt-1 text-xs text-[var(--v2-text-muted)]">
                ${r.unavailableReason}
              </p>
            `}
            ${r.isRestarting&&u`
              <p className="mt-1 text-xs text-[var(--v2-text-muted)]">
                ${r.progressLabel}
              </p>
            `}
          </div>
        </div>

        <${A}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${!r.restartEnabled||r.isRestarting}
          onClick=${r.openConfirm}
          title=${r.restartEnabled?void 0:r.unavailableReason}
          className="w-full sm:w-auto"
        >
          <${M} name=${r.isRestarting?"pulse":"bolt"} className="h-4 w-4" />
          ${r.isRestarting?n("settings.restartStarting"):n("settings.restartNow")}
        <//>
      </div>

      ${r.error&&u`
        <div className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
          ${r.error}
        </div>
      `}

      ${r.message&&u`
        <div className="rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200">
          ${r.message}
        </div>
      `}
    </div>

    <${mi}
      open=${r.confirmOpen}
      onClose=${r.closeConfirm}
      title=${n("restart.title")}
      size="sm"
    >
      <${fi} className="space-y-3">
        <p className="text-sm text-[var(--v2-text)]">
          ${n("restart.description")}
        </p>
        <div className="rounded-xl border border-copper/25 bg-copper/10 px-3 py-2 text-xs text-copper">
          ${n("restart.warning")}
        </div>
      <//>
      <${pi}>
        <${A}
          type="button"
          variant="ghost"
          size="sm"
          disabled=${r.isRestarting}
          onClick=${r.closeConfirm}
        >
          ${n("restart.cancel")}
        <//>
        <${A}
          type="button"
          variant="danger"
          size="sm"
          disabled=${r.isRestarting}
          onClick=${r.confirmRestart}
        >
          <${M} name="bolt" className="h-4 w-4" />
          ${n("restart.confirm")}
        <//>
      <//>
    <//>

    ${r.isRestarting&&u`
      <div
        className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-sm"
        role="status"
        aria-live="polite"
      >
        <div className="w-full max-w-sm rounded-[1.5rem] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-6 text-center shadow-[0_24px_60px_rgba(0,0,0,0.35)]">
          <div className="mx-auto grid h-12 w-12 place-items-center rounded-full border border-copper/30 bg-copper/10 text-copper">
            <${M} name="pulse" className="h-5 w-5 animate-pulse" />
          </div>
          <p className="mt-4 text-base font-semibold text-[var(--v2-text-strong)]">
            ${n("restart.progressTitle")}
          </p>
          <p className="mt-2 text-sm text-[var(--v2-text-muted)]">
            ${r.progressLabel}
          </p>
        </div>
      </div>
    `}
  `:null}function oO(e,t){di(new Blob([JSON.stringify(t,null,2)],{type:"application/json"}),e)}function lO(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{try{t(JSON.parse(n.result))}catch(r){a(r)}},n.onerror=()=>a(n.error||new Error("Unable to read file")),n.readAsText(e)})}function FR({settingsExport:e,onImport:t,isImporting:a,searchQuery:n,onSearchChange:r,onSearchClear:s,onBack:i,canGoBack:o}){let l=C(),c=p.default.useRef(null),d=p.default.useRef(null),[m,f]=p.default.useState(null),h=p.default.useCallback(($,g)=>{d.current&&window.clearTimeout(d.current),f({tone:$,text:g}),d.current=window.setTimeout(()=>f(null),3500)},[]);p.default.useEffect(()=>()=>{d.current&&window.clearTimeout(d.current)},[]);let x=p.default.useCallback(()=>{e&&(oO("ironclaw-settings.json",e),h("success",l("settings.exportSuccess")))},[e,h,l]),y=p.default.useCallback(async $=>{let g=$.target.files?.[0];if($.target.value="",!!g)try{let v=await lO(g);if(!v||typeof v!="object"||!v.settings||typeof v.settings!="object"||Array.isArray(v.settings))throw new Error(l("settings.importInvalid"));await t(v),h("success",l("settings.importSuccess"))}catch(v){h("error",l("settings.importFailed",{message:v.message}))}},[t,h,l]);return u`
    <div className="rounded-md border border-white/10 bg-white/[0.03] px-3 py-3">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center">
        <div className="flex min-w-0 flex-1 flex-col gap-3 sm:flex-row sm:items-center">
          ${o&&u`
            <${A}
              type="button"
              variant="ghost"
              size="sm"
              onClick=${i}
              className="w-fit gap-2"
            >
              <${M} name="chevron" className="h-3.5 w-3.5 rotate-90" />
              ${l("settings.back")}
            <//>
          `}

          <label className="relative min-w-0 flex-1">
            <span className="sr-only">${l("settings.searchPlaceholder")}</span>
            <${M}
              name="search"
              className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--v2-text-faint)]"
            />
            <input
              type="search"
              value=${n}
              onChange=${$=>r($.target.value)}
              placeholder=${l("settings.searchPlaceholder")}
              className="h-9 w-full rounded-md border border-white/12 bg-white/[0.04] pl-9 pr-9 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
            />
            ${n&&u`
              <button
                type="button"
                onClick=${s}
                aria-label=${l("settings.clearSearch")}
                className="absolute right-2 top-1/2 grid h-6 w-6 -translate-y-1/2 place-items-center rounded-md text-[var(--v2-text-faint)] hover:bg-white/[0.07] hover:text-[var(--v2-text-strong)]"
              >
                <${M} name="close" className="h-3.5 w-3.5" />
              </button>
            `}
          </label>
        </div>

        <div className="flex shrink-0 flex-wrap gap-2">
          <${A}
            type="button"
            variant="secondary"
            size="sm"
            onClick=${x}
            disabled=${!e||a}
            className="gap-2"
          >
            <${M} name="download" className="h-3.5 w-3.5" />
            ${l("settings.export")}
          <//>
          <${A}
            type="button"
            variant="secondary"
            size="sm"
            onClick=${()=>c.current?.click()}
            disabled=${a}
            className="gap-2"
          >
            <${M} name="upload" className="h-3.5 w-3.5" />
            ${l(a?"settings.importing":"settings.import")}
          <//>
          <input
            ref=${c}
            type="file"
            accept=".json,application/json"
            className="hidden"
            onChange=${y}
          />
        </div>
      </div>

      <div className="mt-2 min-w-0">
        <div className="text-xs font-medium text-iron-400">${l("settings.manageJson")}</div>
        ${m&&u`
          <div
            role="status"
            className=${["mt-1 text-xs",m.tone==="error"?"text-red-200":"text-mint"].join(" ")}
          >
            ${m.text}
          </div>
        `}
      </div>
    </div>
  `}function BR(){let e=W(),t=H({queryKey:["skills"],queryFn:uw}),a=V({mutationFn:dw,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),n=V({mutationFn:fw,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),r=V({mutationFn:({name:c,content:d})=>mw(c,{content:d}),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),s=V({mutationFn:({name:c,enabled:d})=>pw(c,d),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),i=V({mutationFn:c=>hw(c),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),o=t.data?.skills||[],l=t.data?.auto_activate_learned!==!1;return{skills:o,query:t,autoActivateLearned:l,fetchSkillContent:cw,installSkill:a.mutateAsync,removeSkill:n.mutateAsync,updateSkill:r.mutateAsync,setSkillAutoActivate:s.mutateAsync,setAutoActivateLearned:i.mutateAsync,isInstalling:a.isPending,isRemoving:n.isPending,isUpdating:r.isPending,isSettingAutoActivate:s.isPending,isSettingAutoActivateLearned:i.isPending}}function zR({skill:e,onEdit:t,onRemove:a,onUpdate:n,onSetAutoActivate:r,isRemoving:s,isUpdating:i,isSettingAutoActivate:o}){let l=C(),c=e.name||e.id,d=e.trust||e.trust_level||"installed",m=e.source_kind||"installed",f=!!e.can_edit,h=!!e.can_delete,x=e.auto_activate!==!1,[y,$]=p.default.useState(!1),[g,v]=p.default.useState(""),[b,w]=p.default.useState(""),[S,E]=p.default.useState(!1);p.default.useEffect(()=>{y||(v(""),w(""))},[y]);let N=p.default.useCallback(async()=>{E(!0),w("");try{let L=await t(c);v(L?.content||""),$(!0)}catch(L){w(L.message||l("skills.contentLoadFailed"))}finally{E(!1)}},[c,t,l]),T=p.default.useCallback(async()=>{(await n(c,g))?.success&&$(!1)},[g,c,n]);return u`
    <div className="ext-card border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text)]">${c}</span>
            <${z}
              tone=${String(d).toLowerCase()==="trusted"?"positive":"muted"}
              label=${d}
              size="sm"
            />
            <${z}
              tone=${m==="system"?"positive":"muted"}
              label=${l(`skills.source.${m}`)}
              size="sm"
            />
            ${e.version&&u`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v${e.version}</span>`}
          </div>

          ${e.description&&u`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${e.description}</div>`}

          ${y?u`
                <div className="mt-3">
                  <${Uc}
                    rows=${12}
                    value=${g}
                    className="font-mono text-xs leading-5"
                    onInput=${L=>v(L.currentTarget.value)}
                  />
                </div>
              `:u`<${uO} skill=${e} />`}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          ${f&&!y&&u`
            <${A}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${i||S}
              title=${l("skills.edit")}
              onClick=${N}
            >
              <${M} name="file" className="h-4 w-4" />
              ${l(S?"skills.loading":"skills.edit")}
            <//>
          `}
          ${y&&u`
            <${A}
              type="button"
              variant="ghost"
              size="sm"
              disabled=${i}
              onClick=${()=>{v(""),$(!1)}}
            >
              <${M} name="close" className="h-4 w-4" />
              ${l("skills.cancel")}
            <//>
            <${A}
              type="button"
              variant="primary"
              size="sm"
              disabled=${i}
              onClick=${T}
            >
              <${M} name="check" className="h-4 w-4" />
              ${l(i?"skills.saving":"skills.save")}
            <//>
          `}
          ${f&&!y&&u`
            <${A}
              type="button"
              variant=${x?"secondary":"ghost"}
              size="sm"
              disabled=${o}
              title=${l(x?"skills.autoActivateOnTitle":"skills.autoActivateOffTitle")}
              onClick=${()=>r(c,!x)}
            >
              <${M} name=${x?"check":"close"} className="h-4 w-4" />
              ${l(x?"skills.autoActivateOnLabel":"skills.autoActivateOffLabel")}
            <//>
          `}
          ${h&&!y&&u`
            <${A}
              type="button"
              variant="danger"
              size="sm"
              disabled=${s}
              title=${l("skills.delete")}
              onClick=${()=>a(c)}
            >
              <${M} name="trash" className="h-4 w-4" />
              ${l("skills.delete")}
            <//>
          `}
        </div>
      </div>
      ${b&&u`<p className="mt-2 text-xs text-[var(--v2-danger-text)]">${b}</p>`}
    </div>
  `}function uO({skill:e}){let t=C();return u`
    ${e.keywords?.length>0&&u`
      <div className="mt-2 text-xs text-[var(--v2-text-muted)]">
        <span className="text-[var(--v2-text-faint)]">${t("skills.activatesOn")}:</span>
        ${e.keywords.join(", ")}
      </div>
    `}
    ${e.usage_hint&&u`<div className="mt-2 text-xs text-[var(--v2-text-muted)]">${e.usage_hint}</div>`}
    ${e.setup_hint&&u`<div className="mt-2 text-xs text-[var(--v2-warning-text)]">${e.setup_hint}</div>`}
    ${(e.has_requirements||e.has_scripts||e.install_source_url)&&u`
      <div className="mt-2 flex flex-wrap gap-1.5">
        ${e.has_requirements&&u`<${Gh}>requirements.txt<//>`}
        ${e.has_scripts&&u`<${Gh}>scripts/<//>`}
        ${e.install_source_url&&u`<${Gh}>${t("skills.imported")}<//>`}
      </div>
    `}
  `}function Gh({children:e}){return u`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${e}
    </span>
  `}function qR({onInstall:e,isInstalling:t}){let a=C(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState({name:"",content:""}),[c,d]=p.default.useState(""),[m,f]=p.default.useState(""),h=p.default.useCallback((y,$)=>{l(g=>!g[y]||!$.trim()?g:{...g,[y]:""})},[]),x=p.default.useCallback(async()=>{let y=cO({name:n,content:s}),$=dO(y,a);if($.name||$.content){l($),d(""),f("");return}l({name:"",content:""}),d(""),f("");try{let g=await e(y);if(!g?.success){d(g?.message||a("skills.installFailed"));return}r(""),i(""),f(g.message||a("skills.installedSuccess",{name:y.name}))}catch(g){d(g.message||a("skills.installFailed"))}},[s,n,e,a]);return u`
    <${te} padding="md">
      <div className="mb-4 flex items-start justify-between gap-4">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${a("skills.import")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">
            ${a("skills.importDesc")}
          </p>
        </div>
      </div>

      <${_n} label=${a("skills.name")} error=${o.name} required>
        <${Mt}
          size="sm"
          error=${!!o.name}
          aria-invalid=${o.name?"true":void 0}
          value=${n}
          placeholder=${a("skills.namePlaceholder")}
          onInput=${y=>{let $=y.currentTarget.value;r($),h("name",$)}}
        />
      <//>

      <${_n}
        className="mt-3"
        label=${a("skills.content")}
        error=${o.content}
        hint=${a("skills.contentHint")}
        required
      >
        <${Uc}
          rows=${5}
          error=${!!o.content}
          aria-invalid=${o.content?"true":void 0}
          value=${s}
          placeholder=${a("skills.contentPlaceholder")}
          onInput=${y=>{let $=y.currentTarget.value;i($),h("content",$)}}
        />
      <//>

      ${c&&u`<p className="mt-3 text-sm text-[var(--v2-danger-text)]">${c}</p>`}
      ${m&&u`<p className="mt-3 text-sm text-[var(--v2-positive-text)]">${m}</p>`}

      <div className="mt-4 flex justify-end">
        <${A} type="button" size="sm" disabled=${t} onClick=${x}>
          <${M} name="upload" className="h-4 w-4" />
          ${a(t?"skills.installing":"skills.install")}
        <//>
      </div>
    <//>
  `}function cO({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function dO(e,t){return{name:e.name?"":t("skills.nameRequired"),content:e.content?"":t("skills.contentRequired")}}function IR({searchQuery:e=""}){let t=C(),{skills:a,query:n,autoActivateLearned:r,fetchSkillContent:s,installSkill:i,removeSkill:o,updateSkill:l,setSkillAutoActivate:c,setAutoActivateLearned:d,isInstalling:m,isRemoving:f,isUpdating:h,isSettingAutoActivate:x,isSettingAutoActivateLearned:y}=BR(),[$,g]=p.default.useState(""),[v,b]=p.default.useState(""),w=p.default.useCallback(async L=>{if(window.confirm(t("skills.confirmDelete",{name:L}))){g(""),b("");try{let O=await o(L);if(!O?.success){g(O?.message||t("skills.removeFailed"));return}b(O.message||t("skills.removed",{name:L}))}catch(O){g(O.message||t("skills.removeFailed"))}}},[o,t]),S=p.default.useCallback(async(L,O)=>{if(!O.trim())return g(t("skills.contentRequired")),b(""),{success:!1,message:t("skills.contentRequired")};g(""),b("");try{let P=await l({name:L,content:O});return P?.success?(b(P.message||t("skills.updated",{name:L})),P):(g(P?.message||t("skills.updateFailed")),P)}catch(P){let k=P.message||t("skills.updateFailed");return g(k),{success:!1,message:k}}},[t,l]),E=p.default.useCallback(async(L,O)=>{g(""),b("");try{let P=await c({name:L,enabled:O});if(!P?.success){g(P?.message||t("skills.updateFailed"));return}b(P.message)}catch(P){g(P.message||t("skills.updateFailed"))}},[c,t]),N=p.default.useCallback(async L=>{g(""),b("");try{let O=await d(L);if(!O?.success){g(O?.message||t("skills.updateFailed"));return}b(O.message)}catch(O){g(O.message||t("skills.updateFailed"))}},[d,t]),T;if(n.isLoading)T=u`
      <${te} padding="md">
          <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(L=>u`
            <div key=${L} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
              <div>
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              </div>
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
        <//>
    `;else if(n.error)T=u`
      <${te} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">${t("skills.failedLoad",{message:n.error.message})}</p>
        <//>
    `;else{let L=a.filter(P=>et(e,[P.name,P.id,P.description,P.keywords,P.trust_level,P.source_kind,P.version])),O=pO(L);a.length===0?T=u`
        <${te} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:L.length===0?T=u`<${wt} query=${e} />`:T=u`
        <div id="skills-list">
          ${O.map(P=>u`
              <${fO}
                key=${P.id}
                title=${t(P.labelKey)}
                skills=${P.skills}
                onEdit=${s}
                onRemove=${w}
                onUpdate=${S}
                onSetAutoActivate=${E}
                isRemoving=${f}
                isUpdating=${h}
                isSettingAutoActivate=${x}
              />
            `)}
        </div>
      `}return u`
    <div className="space-y-4">
      <${mO}
        enabled=${r}
        isSaving=${y}
        onToggle=${N}
      />
      <${qR} onInstall=${i} isInstalling=${m} />
      <${hO} error=${$} result=${v} />
      ${T}
    </div>
  `}function mO({enabled:e,isSaving:t,onToggle:a}){let n=C();return u`
    <${te} padding="md" style=${e?void 0:{background:"var(--v2-danger-soft)"}}>
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="text-sm font-medium text-[var(--v2-text-strong)]">
            ${n(e?"skills.defaultAutoActivationEnabled":"skills.defaultAutoActivationDisabled")}
          </div>
          <div className="mt-1 text-xs text-[var(--v2-text-muted)]">
            ${n(e?"skills.defaultAutoActivationOnDesc":"skills.defaultAutoActivationOffDesc")}
          </div>
        </div>
        <div className="shrink-0">
          <${A}
            type="button"
            variant=${e?"secondary":"ghost"}
            size="sm"
            disabled=${t}
            onClick=${()=>a(!e)}
          >
            ${n(e?"skills.defaultAutoActivationOnButton":"skills.defaultAutoActivationOffButton")}
          <//>
        </div>
      </div>
    <//>
  `}function fO({title:e,skills:t,onEdit:a,onRemove:n,onUpdate:r,onSetAutoActivate:s,isRemoving:i,isUpdating:o,isSettingAutoActivate:l}){return t.length===0?null:u`
    <${te} padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${e}
      </h3>
      ${t.map(c=>u`
          <${zR}
            key=${`${c.source_kind||"skill"}:${c.name||c.id}`}
            skill=${c}
            onEdit=${a}
            onRemove=${n}
            onUpdate=${r}
            onSetAutoActivate=${s}
            isRemoving=${i}
            isUpdating=${o}
            isSettingAutoActivate=${l}
          />
        `)}
    <//>
  `}function pO(e){let t=[{id:"user",labelKey:"skills.group.user",skills:[]},{id:"system",labelKey:"skills.group.system",skills:[]},{id:"workspace",labelKey:"skills.group.workspace",skills:[]}],a=t[0];for(let n of e){let r=n.source_kind||"";(r==="system"?t[1]:r==="workspace"?t[2]:a).skills.push(n)}return t.filter(n=>n.skills.length>0)}function hO({error:e,result:t}){return!e&&!t?null:u`
    <div
      className=${e?"rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200":"rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200"}
    >
      ${e||t}
    </div>
  `}function bd(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function HR(){let e=W(),t=H({queryKey:["settings-tools"],queryFn:sw}),a=t.data?.tools||[],[n,r]=p.default.useState({}),s=V({mutationFn:async({name:o,state:l})=>bd(await iw(o,l),"Save failed"),onSuccess:(o,{name:l,state:c})=>{e.setQueryData(["settings-tools"],d=>{if(!d)return d;let m=o?.tool;return{...d,tools:d.tools.map(f=>f.name===l?{...f,state:c,...m||{}}:f)}}),r(d=>({...d,[l]:!0})),setTimeout(()=>r(d=>({...d,[l]:!1})),2e3)}}),i=p.default.useCallback((o,l)=>s.mutate({name:o,state:l}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}var Yh="agent.auto_approve_tools";function vO({visible:e}){let t=C();return e?u`
    <span className="font-mono text-[11px] text-[var(--v2-accent-text)]" role="status">
      ${t("tools.saved")}
    </span>
  `:null}function gO({checked:e,disabled:t=!1,label:a,onChange:n}){return u`
    <button
      type="button"
      role="switch"
      aria-checked=${e}
      aria-label=${a}
      disabled=${t}
      onClick=${()=>!t&&n(!e)}
      className=${["relative inline-flex h-7 w-12 shrink-0 items-center rounded-full border transition",t?"cursor-not-allowed opacity-60":"cursor-pointer",e?"border-[color-mix(in_srgb,var(--v2-accent)_45%,transparent)] bg-[color-mix(in_srgb,var(--v2-accent)_22%,transparent)]":"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]"].join(" ")}
    >
      <span
        className=${["pointer-events-none inline-block h-5 w-5 rounded-full transition",e?"translate-x-5 bg-[var(--v2-accent-text)]":"translate-x-1 bg-[var(--v2-text-muted)]"].join(" ")}
      />
    </button>
  `}function Jh({settings:e,onSave:t,savedKeys:a,isLoading:n}){let r=C(),s=r("settings.field.autoApproveEligibleTools"),i=e?.[Yh],o=i==null?!0:i===!0||i==="true";return u`
    <${te} padding="md" className="flex items-center justify-between gap-6">
      <div className="min-w-0">
        <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
          ${s}
        </h3>
        <p className="mt-1 text-sm text-[var(--v2-text-muted)]">
          ${r("settings.field.autoApproveEligibleToolsDesc")}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-3">
        <${vO} visible=${a?.[Yh]} />
        <${gO}
          checked=${o}
          disabled=${n}
          label=${s}
          onChange=${l=>t(Yh,l)}
        />
      </div>
    <//>
  `}function yO({tool:e,onPermissionChange:t,isSaved:a}){let n=C(),r=[{value:"default",label:n("tools.followDefault"),tone:"neutral"},{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask_each_time",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],s={default:n("tools.sourceDefault"),global:n("tools.sourceGlobal"),override:n("tools.sourceOverride")},i=e.locked,o=r.find(m=>m.value===e.state)||r[1],l=e.effective_source||"default",c=l==="override"?e.state:"default",d=l==="default"&&e.state===e.default_state;return u`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="flex min-w-0 items-center gap-3">
        ${i&&u`<${M}
          name="lock"
          className="h-3.5 w-3.5 shrink-0 text-[var(--v2-text-faint)]"
        />`}
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate font-mono text-sm text-[var(--v2-text)]"
              >${e.name}</span
            >
            ${d&&u`
              <span
                className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]"
              >
                ${n("tools.default")}
              </span>
            `}
            <span
              className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]"
            >
              ${s[l]||s.default}
            </span>
          </div>
          ${e.description&&u`
            <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">
              ${e.description}
            </div>
          `}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${i?u`<${z} tone=${o.tone} label=${o.label} size="sm" />`:u`
              <select
                value=${c}
                onChange=${m=>t(e.name,m.target.value)}
                aria-label=${n("tools.permissionFor",{name:e.name})}
                className="v2-select h-8 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2.5 font-mono text-xs text-[var(--v2-text-strong)] outline-none focus:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]"
              >
                ${r.map(m=>u`<option key=${m.value} value=${m.value}>
                      ${m.label}
                    </option>`)}
              </select>
            `}
        ${a&&u`
          <span className="font-mono text-[11px] text-[var(--v2-accent-text)]"
            >${n("tools.saved")}</span
          >
        `}
      </div>
    </div>
  `}function KR({settings:e={},onSave:t=()=>{},savedKeys:a={},isLoading:n=!1,searchQuery:r=""}){let s=C(),{tools:i,query:o,setPermission:l,savedTools:c}=HR();if(o.isLoading)return u`
      <div className="space-y-4">
        <${Jh}
          settings=${e}
          onSave=${t}
          savedKeys=${a}
          isLoading=${n}
        />
        <${te} padding="md">
          <div className="mb-4 h-3 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3,4,5].map(m=>u`
              <div
                key=${m}
                className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-3.5 first:border-0"
              >
                <div className="h-4 w-36 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="h-8 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              </div>
            `)}
        <//>
      </div>
    `;if(o.error)return u`
      <div className="space-y-4">
        <${Jh}
          settings=${e}
          onSave=${t}
          savedKeys=${a}
          isLoading=${n}
        />
        <${te} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">
            ${s("tools.failedLoad",{message:o.error.message})}
          </p>
        <//>
      </div>
    `;let d=i.filter(m=>et(r,[m.name,m.description,m.state,m.default_state,m.effective_source,m.locked?s("tools.disabled"):""]));return u`
    <div className="space-y-4">
      <${Jh}
        settings=${e}
        onSave=${t}
        savedKeys=${a}
        isLoading=${n}
      />

      ${r&&u`
        <div className="flex justify-end">
          <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">
            ${d.length} / ${i.length}
          </span>
        </div>
      `}

      <${te} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${s("tools.permissions")}
        </h3>
        ${d.length===0?u`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${s("tools.noMatch")}
            </p>`:d.map(m=>u`
                  <${yO}
                    key=${m.name}
                    tool=${m}
                    onPermissionChange=${l}
                    isSaved=${c[m.name]}
                  />
                `)}
      <//>
    </div>
  `}function QR(e){return(Number(e)||0).toFixed(2)}function bO(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function VR(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function Wr({label:e,value:t,description:a}){return u`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&u`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function GR({searchQuery:e=""}){let t=C(),{credits:a,query:n,authorize:r}=Ac();if(!et(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return u`<${wt} query=${e} />`;let s;if(n.isLoading)s=u`
      <div className="mt-4">
        ${[1,2,3].map(i=>u`
            <div
              key=${i}
              className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-3 first:border-0"
            >
              <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              <div className="h-4 w-16 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
      </div>
    `;else if(n.isError)s=u`
      <div
        className="mt-4 rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
      >
        ${t("traceCommons.loadFailed")}
      </div>
    `;else if(!a||!a.enrolled&&!(a.submissions_total>0))s=u`
      <div
        className="mt-4 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-6 text-center text-sm text-[var(--v2-text-muted)]"
      >
        ${t("traceCommons.emptyState")}
      </div>
    `;else{let i=a.recent_explanations||[],o=a.holds||[];s=u`
      <div className="mt-4">
        <${Wr}
          label=${t("traceCommons.enrollment")}
          value=${a.enrolled?t("traceCommons.enrolled"):t("traceCommons.notEnrolled")}
        />
        <${Wr}
          label=${t("traceCommons.pendingCredit")}
          description=${t("traceCommons.pendingCreditDesc")}
          value=${QR(a.pending_credit)}
        />
        <${Wr}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${QR(a.final_credit)}
        />
        <${Wr}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${bO(a.delayed_credit_delta)}
        />
        <${Wr}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${Wr}
          label=${t("traceCommons.lastSubmission")}
          value=${VR(a.last_submission_at,t)}
        />
        <${Wr}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${VR(a.last_credit_sync_at,t)}
        />
      </div>
      ${i.length>0&&u`
        <div className="mt-5">
          <h4
            className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("traceCommons.recentExplanations")}
          </h4>
          <ul className="ml-4 list-disc space-y-1 text-xs text-[var(--v2-text-muted)]">
            ${i.map((l,c)=>u`<li key=${c}>${l}</li>`)}
          </ul>
        </div>
      `}
      ${o.length>0&&u`
        <div className="mt-5">
          <h4
            className="mb-1 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("traceCommons.heldTitle")}
          </h4>
          <p className="mb-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            ${t("traceCommons.heldDescription")}
          </p>
          <ul className="space-y-2">
            ${o.map(l=>u`
                <li
                  key=${l.submission_id}
                  className="flex items-start justify-between gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2"
                >
                  <div className="min-w-0">
                    <div className="text-xs text-[var(--v2-text-strong)]">${l.reason}</div>
                    <div className="mt-0.5 truncate font-mono text-[10px] text-[var(--v2-text-faint)]">
                      ${l.submission_id}
                    </div>
                  </div>
                  <button
                    type="button"
                    onClick=${()=>r.mutate(l.submission_id)}
                    disabled=${r.isPending}
                    className="shrink-0 rounded-lg border border-[var(--v2-accent-soft)] px-2.5 py-1 text-xs font-medium text-[var(--v2-accent-text)] transition-colors hover:bg-[var(--v2-accent-soft)] disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    ${r.isPending?t("traceCommons.authorizing"):t("traceCommons.authorize")}
                  </button>
                </li>
              `)}
          </ul>
        </div>
      `}
    `}return u`
    <${te} padding="md">
      <h3
        className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${t("traceCommons.title")}
      </h3>
      <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
        ${t("traceCommons.description")}
      </p>

      ${s}

      <p className="mt-5 text-xs leading-5 text-[var(--v2-text-faint)]">
        ${t("traceCommons.note")}
      </p>
    <//>
  `}function YR(){let e=W(),t=H({queryKey:["admin-users"],queryFn:yw,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=V({mutationFn:bw,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=V({mutationFn:({id:i,payload:o})=>xw(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function xO({onCreate:e,isCreating:t,error:a}){let n=C(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState("member"),[d,m]=p.default.useState(!1),f=h=>{h.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:l},{onSuccess:()=>{s(""),o(""),m(!1)}})};return d?u`
    <${te} padding="md">
      <h3
        className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${n("users.newUser")}
      </h3>
      <form onSubmit=${f} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <${_n} label=${n("users.displayName")} htmlFor="user-name">
            <${Mt}
              id="user-name"
              type="text"
              value=${r}
              onChange=${h=>s(h.target.value)}
              required
            />
          <//>
          <${_n} label=${n("users.email")} htmlFor="user-email">
            <${Mt}
              id="user-email"
              type="email"
              value=${i}
              onChange=${h=>o(h.target.value)}
            />
          <//>
        </div>
        <${_n} label=${n("users.role")} htmlFor="user-role">
          <select
            id="user-role"
            value=${l}
            onChange=${h=>c(h.target.value)}
            className="v2-select h-9 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 text-sm text-[var(--v2-text-strong)] outline-none focus:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]"
          >
            <option value="member">${n("users.member")}</option>
            <option value="admin">${n("users.admin")}</option>
          </select>
        <//>
        ${a&&u` <p className="text-sm text-[var(--v2-danger-text)]">${a.message}</p> `}
        <div className="flex gap-2">
          <${A} type="submit" disabled=${t}>
            ${n(t?"users.creating":"users.createUser")}
          <//>
          <${A}
            variant="ghost"
            type="button"
            onClick=${()=>m(!1)}
            >${n("users.cancel")}<//
          >
        </div>
      </form>
    <//>
  `:u`
      <${A} variant="secondary" onClick=${()=>m(!0)}>
        <${M} name="plus" className="mr-2 h-4 w-4" />
        ${n("users.addUser")}
      <//>
    `}function $O({user:e}){let t=C(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return u`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]"
            >${e.display_name||e.id}</span
          >
          <${z}
            tone=${n}
            label=${e.role==="admin"?t("users.admin"):t("users.member")}
            size="sm"
          />
          <${z} tone=${a} label=${e.status||"active"} size="sm" />
        </div>
        ${e.email&&u`
          <div className="mt-0.5 font-mono text-xs text-[var(--v2-text-muted)]">
            ${e.email}
          </div>
        `}
      </div>
      <div
        className="flex shrink-0 items-center gap-4 font-mono text-[11px] text-[var(--v2-text-faint)]"
      >
        ${e.last_active&&u`<span>${new Date(e.last_active).toLocaleDateString()}</span>`}
      </div>
    </div>
  `}function JR({searchQuery:e=""}){let t=C(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=YR();if(n.isLoading)return u`
      <${te} padding="md">
        <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
        ${[1,2,3].map(c=>u`
            <div
              key=${c}
              className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-3.5 first:border-0"
            >
              <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
      <//>
    `;if(r)return u`
      <${te} padding="lg">
        <div className="flex items-center gap-3">
          <${M} name="lock" className="h-5 w-5 text-[var(--v2-text-faint)]" />
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">
            ${t("users.adminRequired")}
          </h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
          ${t("users.adminRequiredDesc")}
        </p>
      <//>
    `;if(n.error)return u`
      <${te} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("users.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let l=a.filter(c=>et(e,[c.id,c.display_name,c.email,c.role,c.status,c.last_active]));return u`
    <div className="space-y-5">
      <${xO}
        onCreate=${s}
        isCreating=${o}
        error=${i}
      />

      <${te} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("users.title",{count:l.length})}
        </h3>
        ${a.length===0?u`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("users.noUsers")}
            </p>`:l.length===0?u`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("settings.noMatchingSettings",{query:e})}
            </p>`:l.map(c=>u`<${$O} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function XR(){let e=W(),t=H({queryKey:["settings-export"],queryFn:J$,staleTime:3e4}),a=t.data?.settings||{},[n,r]=p.default.useState({}),[s,i]=p.default.useState(!1),o=V({mutationFn:async({key:m,value:f})=>bd(await Jp(m,f),"Save failed"),onSuccess:(m,{key:f,value:h})=>{e.setQueryData(["settings-export"],x=>{if(!x)return x;let y={...x,settings:{...x.settings}};return h==null?delete y.settings[f]:y.settings[f]=h,y}),r(x=>({...x,[f]:!0})),setTimeout(()=>r(x=>({...x,[f]:!1})),2e3),Vh.has(f)&&i(!0),f==="agent.auto_approve_tools"&&e.invalidateQueries({queryKey:["settings-tools"]})}}),l=p.default.useCallback((m,f)=>o.mutate({key:m,value:f}),[o]),c=V({mutationFn:X$,onSuccess:(m,f)=>{e.invalidateQueries({queryKey:["settings-export"]});let h=Object.keys(f?.settings||{});h.includes("agent.auto_approve_tools")&&e.invalidateQueries({queryKey:["settings-tools"]}),h.some(x=>Vh.has(x))&&i(!0)}}),d=p.default.useCallback(m=>c.mutateAsync(m),[c]);return{settings:a,query:t,save:l,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error||c.error}}function Xh(){let e=C(),{tab:t}=st(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=ba(),s=r?"inference":"language",i=t||s,{settings:o,query:l,save:c,savedKeys:d,needsRestart:m,importSettings:f,isImporting:h,saveError:x}=XR(),[y,$]=p.default.useState("");p.default.useEffect(()=>{$("")},[i]);let g=l.isLoading,v={inference:u`<${OR}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${g}
      searchQuery=${y}
    />`,agent:u`<${CR}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${g}
      searchQuery=${y}
    />`,channels:u`<${AR} searchQuery=${y} />`,networking:u`<${PR}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${g}
      searchQuery=${y}
    />`,tools:u`<${KR}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${g}
      searchQuery=${y}
    />`,skills:u`<${IR} searchQuery=${y} />`,traces:u`<${GR} searchQuery=${y} />`,users:u`<${JR} searchQuery=${y} />`,language:u`<${LR} searchQuery=${y} />`},b=T=>T==="users"||T==="inference",w=T=>Object.prototype.hasOwnProperty.call(v,T),S=Object.keys(v).filter(T=>r||!b(T)),N=w(s)&&S.includes(s)?s:S[0]||"language";return!w(i)||!r&&b(i)?u`<${it} to=${`/settings/${N}`} replace />`:u`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${m&&u`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${UR}
                visible=${!0}
                gatewayStatus=${a}
                gatewayStatusQuery=${n}
              />
            </div>`}

            ${x&&u`
              <div
                className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
              >
                ${e("error.saveFailed",{message:x.message})}
              </div>
            `}

            <${FR}
              settingsExport=${l.data||null}
              onImport=${f}
              isImporting=${h}
              searchQuery=${y}
              onSearchChange=${$}
              onSearchClear=${()=>$("")}
              canGoBack=${!1}
            />

            ${v[i]}
          </div>
        </div>
      </div>
    </div>
  `}var Wh=Object.freeze({todo:!0});function WR(){return Promise.resolve({users:[],total:0,...Wh})}function ZR(e){return Promise.resolve(null)}function ek(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function tk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function ak(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function nk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function rk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function sk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function ik(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...Wh})}function ok(e="day",t){return Promise.resolve({entries:[],...Wh})}function lk(){return H({queryKey:["admin","usage-summary"],queryFn:ik,refetchInterval:3e4})}function xd(e="day",t){return H({queryKey:["admin","usage",e,t],queryFn:()=>ok(e,t),refetchInterval:3e4})}function _i(){let e=W(),t=H({queryKey:["admin","users"],queryFn:WR,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=V({mutationFn:ek,onSuccess:s}),o=V({mutationFn:({id:f,payload:h})=>tk(f,h),onSuccess:s}),l=V({mutationFn:f=>ak(f),onSuccess:s}),c=V({mutationFn:f=>nk(f),onSuccess:s}),d=V({mutationFn:f=>rk(f),onSuccess:s}),m=V({mutationFn:({userId:f,name:h})=>sk(f,h)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(f,h)=>o.mutateAsync({id:f,payload:h}),deleteUser:l.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(f,h)=>m.mutateAsync({userId:f,name:h}),newToken:m.data,clearToken:()=>m.reset()}}function uk(e){return H({queryKey:["admin","user",e],queryFn:()=>ZR(e),enabled:!!e,refetchInterval:1e4})}function en(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function Aa(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function ck(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function fr(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function Ri(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function ki(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function Ci(e){return e==="admin"?"signal":"muted"}function dk(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function mk(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function fk(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function pk(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function hk(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function wO({users:e,onSelectUser:t}){let a=C(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?u`
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-white/10 text-left">
            <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a("admin.dashboard.name")}</th>
            <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a("admin.dashboard.role")}</th>
            <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a("admin.dashboard.status")}</th>
            <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">${a("admin.dashboard.jobs")}</th>
            <th className="pb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a("admin.dashboard.lastActive")}</th>
          </tr>
        </thead>
        <tbody>
          ${n.map(r=>u`
              <tr key=${r.id} className="border-b border-white/[0.06] last:border-0">
                <td className="py-3 pr-4">
                  <button
                    onClick=${()=>t(r.id)}
                    className="text-sm font-medium text-signal hover:underline"
                  >
                    ${r.display_name||r.id}
                  </button>
                </td>
                <td className="py-3 pr-4"><${z} tone=${Ci(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${z} tone=${ki(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${fr(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:u`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function vk({onSelectUser:e,onNavigateTab:t}){let a=C(),n=lk(),{users:r,query:s}=_i(),i=n.data||{},o=dk(r),l=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?u`
      <div className="space-y-5">
        <${q} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            ${[1,2,3,4].map(m=>u`<div key=${m} className="v2-skeleton h-28 rounded-lg" />`)}
          </div>
        <//>
      </div>
    `:u`
    <div className="space-y-5">
      <${q} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.systemOverview")}</h3>
          ${i.uptime_seconds!=null&&u`
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:ck(i.uptime_seconds)})}</span>
          `}
        </div>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <${Ze}
            label=${a("admin.dashboard.totalUsers")}
            value=${String(o.total)}
            tone=${o.total>0?"success":"muted"}
          />
          <${Ze}
            label=${a("admin.dashboard.activeUsers")}
            value=${String(o.active)}
            tone="success"
          />
          <${Ze}
            label=${a("admin.dashboard.suspended")}
            value=${String(o.suspended)}
            tone=${o.suspended>0?"danger":"muted"}
          />
          <${Ze}
            label=${a("admin.dashboard.admins")}
            value=${String(o.admins)}
            tone="signal"
          />
        </div>
      <//>

      <${q} className="p-5 sm:p-6">
        <h3 className="mb-5 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.usage30d")}</h3>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <${Ze}
            label=${a("admin.dashboard.totalJobs")}
            value=${String(c.total||0)}
            tone="muted"
          />
          <${Ze}
            label=${a("admin.dashboard.llmCalls")}
            value=${String(l.llm_calls||0)}
            tone="muted"
          />
          <${Ze}
            label=${a("admin.dashboard.totalCost")}
            value=${Aa(l.total_cost)}
            tone="signal"
          />
          <${Ze}
            label=${a("admin.dashboard.activeJobs")}
            value=${String(c.in_progress||0)}
            tone=${(c.in_progress||0)>0?"success":"muted"}
          />
        </div>
      <//>

      <${q} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.recentUsers")}</h3>
          <button
            onClick=${()=>t("users")}
            className="text-xs text-signal hover:underline"
          >
            ${a("admin.dashboard.viewAll")}
          </button>
        </div>
        <${wO} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var SO=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function NO({value:e,max:t}){let a=t>0?e/t*100:0;return u`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function gk({onSelectUser:e}){let t=C(),[a,n]=p.default.useState("day"),r=xd(a),s=r.data?.usage||[],i=fk(s),o=pk(s),l=hk(i),c=i.length>0?i[0].cost:0;return r.isLoading?u`
      <${q} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
        <div className="grid gap-4 sm:grid-cols-4">
          ${[1,2,3,4].map(d=>u`<div key=${d} className="v2-skeleton h-28 rounded-lg" />`)}
        </div>
      <//>
    `:u`
    <div className="space-y-5">
      <${q} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.overview")}</h3>
          <div className="flex gap-1">
            ${SO.map(d=>u`
                <button
                  key=${d.value}
                  onClick=${()=>n(d.value)}
                  className=${["rounded-md px-3 py-1.5 text-[11px] font-medium",a===d.value?"border border-signal/35 bg-signal/10 text-white":"border border-transparent text-iron-300 hover:text-white"].join(" ")}
                >
                  ${d.label}
                </button>
              `)}
          </div>
        </div>

        ${s.length===0?u`<p className="py-4 text-sm text-iron-300">${t("admin.usage.noData")}</p>`:u`
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
                <${Ze} label=${t("admin.usage.totalCalls")} value=${l.calls.toLocaleString()} tone="muted" />
                <${Ze} label=${t("admin.usage.inputTokens")} value=${en(l.input_tokens)} tone="muted" />
                <${Ze} label=${t("admin.usage.outputTokens")} value=${en(l.output_tokens)} tone="muted" />
                <${Ze} label=${t("admin.usage.totalCost")} value=${Aa(l.cost.toFixed(2))} tone="signal" />
              </div>
            `}
      <//>

      ${i.length>0&&u`
        <${q} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.perUser")}</h3>
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/10 text-left">
                  <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t("admin.usage.user")}</th>
                  <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t("admin.usage.calls")}</th>
                  <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">${t("admin.usage.input")}</th>
                  <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">${t("admin.usage.output")}</th>
                  <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t("admin.usage.cost")}</th>
                  <th className="hidden pb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 md:table-cell" />
                </tr>
              </thead>
              <tbody>
                ${i.map(d=>u`
                    <tr key=${d.user_id} className="border-b border-white/[0.06] last:border-0">
                      <td className="py-3 pr-4">
                        <button
                          onClick=${()=>e(d.user_id)}
                          className="font-mono text-xs text-signal hover:underline"
                        >
                          ${Ri(d.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${en(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${en(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${Aa(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${NO} value=${d.cost} max=${c} />
                      </td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}

      ${o.length>0&&u`
        <${q} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.perModel")}</h3>
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/10 text-left">
                  <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t("admin.usage.model")}</th>
                  <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t("admin.usage.calls")}</th>
                  <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">${t("admin.usage.input")}</th>
                  <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">${t("admin.usage.output")}</th>
                  <th className="pb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t("admin.usage.cost")}</th>
                </tr>
              </thead>
              <tbody>
                ${o.map(d=>u`
                    <tr key=${d.model} className="border-b border-white/[0.06] last:border-0">
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${d.model}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${en(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${en(d.output_tokens)}</td>
                      <td className="py-3 font-mono text-xs text-iron-100">${Aa(d.cost.toFixed(2))}</td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}
    </div>
  `}function pr({label:e,children:t}){return u`
    <div className="flex items-start justify-between gap-4 border-t border-white/[0.06] py-3 first:border-0 first:pt-0">
      <span className="text-xs text-iron-300">${e}</span>
      <span className="text-right text-sm text-iron-100">${t}</span>
    </div>
  `}function yk({userId:e,onBack:t}){let a=C(),n=uk(e),r=xd("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:l,createToken:c,newToken:d,clearToken:m}=_i(),[f,h]=p.default.useState(null),[x,y]=p.default.useState(!1),$=n.data,g=r.data?.usage||[];if(p.default.useEffect(()=>{$&&f===null&&h($.role)},[$]),n.isLoading)return u`
      <div className="space-y-5">
        <${q} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-2 h-6 w-48 rounded" />
          <div className="v2-skeleton h-4 w-32 rounded" />
        <//>
      </div>
    `;if(n.error)return u`
      <${q} className="p-5 sm:p-6">
        <p className="text-sm text-red-200">${a("error.loadFailed",{what:a("admin.users.user"),message:n.error.message})}</p>
      <//>
    `;if(!$)return null;let v=async()=>{f&&f!==$.role&&await o($.id,{role:f})},b=async()=>{await l($.id),t()},w=async()=>{let S=window.prompt(a("admin.users.tokenNamePrompt",{name:$.display_name||a("admin.users.userFallback")}));S&&await c($.id,S)};return u`
    <div className="space-y-5">
      <button
        onClick=${t}
        className="flex items-center gap-1.5 text-xs text-iron-300 hover:text-white"
      >
        <span>←</span>
        <span>${a("admin.users.backToUsers")}</span>
      </button>

      <${q} className="p-5 sm:p-6">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <h2 className="text-2xl font-semibold tracking-tight text-white">${$.display_name||$.id}</h2>
            <div className="mt-2 flex items-center gap-2">
              <${z} tone=${Ci($.role)} label=${$.role||"member"} />
              <${z} tone=${ki($.status)} label=${$.status||"active"} />
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            ${$.status==="active"?u`<${A} variant="secondary" onClick=${()=>s($.id)}>${a("admin.users.suspend")}<//>`:u`<${A} variant="secondary" onClick=${()=>i($.id)}>${a("admin.users.activate")}<//>`}
            <${A} variant="secondary" onClick=${w}>${a("admin.users.createToken")}<//>
            <button
              onClick=${()=>y(!0)}
              className="v2-button inline-flex h-10 items-center justify-center rounded-md border border-red-400/30 bg-red-500/10 px-4 text-sm font-semibold text-red-200 hover:bg-red-500/20"
            >
              ${a("admin.users.delete")}
            </button>
          </div>
        </div>
      <//>

      ${(d?.token||d?.plaintext_token)&&u`
        <div className="rounded-xl border border-signal/30 bg-signal/10 p-4 sm:p-5">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 flex-1">
              <p className="text-sm font-semibold text-white">${a("admin.users.tokenCreated")}</p>
              <p className="mt-1 text-xs text-iron-300">${a("admin.users.tokenCreatedDesc")}</p>
              <code className="mt-2 block truncate rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 font-mono text-xs text-iron-100">
                ${d.token||d.plaintext_token}
              </code>
            </div>
            <button onClick=${m} className="text-iron-300 hover:text-white">
              <${M} name="close" className="h-4 w-4" />
            </button>
          </div>
        </div>
      `}

      <div className="grid gap-5 lg:grid-cols-2">
        <${q} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.profile")}</h3>
          <${pr} label=${a("admin.user.id")}>
            <span className="font-mono text-xs">${$.id}</span>
          <//>
          <${pr} label=${a("admin.user.email")}>${$.email||a("admin.user.notSet")}<//>
          <${pr} label=${a("admin.user.created")}>${fr($.created_at)}<//>
          <${pr} label=${a("admin.user.lastLogin")}>${fr($.last_login_at)}<//>
          ${$.created_by&&u`
            <${pr} label=${a("admin.user.createdBy")}>
              <span className="font-mono text-xs">${Ri($.created_by)}</span>
            <//>
          `}
        <//>

        <${q} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${pr} label=${a("admin.user.jobs")}>${$.job_count??0}<//>
          <${pr} label=${a("admin.user.totalCost")}>${Aa($.total_cost)}<//>
          <${pr} label=${a("admin.user.lastActive")}>${fr($.last_active_at)}<//>
        <//>
      </div>

      <${q} className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.roleManagement")}</h3>
        <div className="flex items-end gap-3">
          <div>
            <label className="mb-1 block text-xs text-iron-300">${a("admin.user.currentRole")}</label>
            <select
              value=${f||$.role}
              onChange=${S=>h(S.target.value)}
              className="v2-select h-9 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            >
              <option value="member">${a("admin.users.member")}</option>
              <option value="admin">${a("admin.users.admin")}</option>
            </select>
          </div>
          <${A} onClick=${v} disabled=${!f||f===$.role}>
            ${a("admin.user.saveRole")}
          <//>
        </div>
      <//>

      <${q} className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.usage30Days")}</h3>
        ${g.length===0?u`<p className="py-4 text-sm text-iron-300">${a("admin.user.noUsage")}</p>`:u`
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-white/10 text-left">
                      <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a("admin.usage.model")}</th>
                      <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a("admin.usage.calls")}</th>
                      <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">${a("admin.usage.input")}</th>
                      <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">${a("admin.usage.output")}</th>
                      <th className="pb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a("admin.usage.cost")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    ${g.map((S,E)=>u`
                        <tr key=${E} className="border-b border-white/[0.06] last:border-0">
                          <td className="py-3 pr-4 font-mono text-xs text-iron-100">${S.model}</td>
                          <td className="py-3 pr-4 font-mono text-xs text-iron-300">${(S.call_count||0).toLocaleString()}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${en(S.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${en(S.output_tokens)}</td>
                          <td className="py-3 font-mono text-xs text-iron-100">${Aa(S.total_cost)}</td>
                        </tr>
                      `)}
                  </tbody>
                </table>
              </div>
            `}
      <//>

      ${x&&u`
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick=${()=>y(!1)}>
          <div className="w-full max-w-md rounded-xl border border-white/10 bg-iron-900 p-6" onClick=${S=>S.stopPropagation()}>
            <h3 className="text-lg font-semibold text-white">${a("admin.users.deleteUserTitle")}</h3>
            <p className="mt-2 text-sm text-iron-300">
              ${a("admin.users.deleteUserDesc",{name:$.display_name})}
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <${A} variant="ghost" onClick=${()=>y(!1)}>${a("admin.users.cancel")}<//>
              <button
                onClick=${b}
                className="v2-button inline-flex h-10 items-center justify-center rounded-md bg-red-500/20 px-4 text-sm font-semibold text-red-200 hover:bg-red-500/30"
              >
                ${a("admin.users.delete")}
              </button>
            </div>
          </div>
        </div>
      `}
    </div>
  `}function _O(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function RO({token:e,onDismiss:t}){let a=C(),[n,r]=p.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return u`
    <div className="rounded-xl border border-signal/30 bg-signal/10 p-4 sm:p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-iron-100">${a("admin.users.tokenCreated")}</p>
          <p className="mt-1 text-xs text-iron-300">${a("admin.users.tokenCreatedDesc")}</p>
          <div className="mt-3 flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-md border border-iron-700 bg-iron-800/70 px-3 py-2 font-mono text-xs text-iron-100">
              ${e}
            </code>
            <${A} variant="secondary" onClick=${s}>
              ${a(n?"admin.users.copied":"admin.users.copy")}
            <//>
          </div>
        </div>
        <button onClick=${t} className="text-iron-300 hover:text-iron-100">
          <${M} name="close" className="h-4 w-4" />
        </button>
      </div>
    </div>
  `}function kO({onCreate:e,isCreating:t,error:a}){let n=C(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState("member"),[d,m]=p.default.useState(!1),f=async h=>{h.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:l}),s(""),o(""),m(!1))};return d?u`
    <${q} className="p-5 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${n("admin.users.createUser")}</h3>
      <form onSubmit=${f} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-3">
          <div>
            <label className="mb-1 block text-xs text-iron-300">${n("admin.users.displayName")}</label>
            <input
              type="text"
              value=${r}
              onChange=${h=>s(h.target.value)}
              required
              className="h-9 w-full rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
              placeholder=${n("admin.users.displayNamePlaceholder")}
            />
          </div>
          <div>
            <label className="mb-1 block text-xs text-iron-300">${n("admin.users.email")}</label>
            <input
              type="email"
              value=${i}
              onChange=${h=>o(h.target.value)}
              className="h-9 w-full rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
              placeholder=${n("admin.users.emailPlaceholder")}
            />
          </div>
          <div>
            <label className="mb-1 block text-xs text-iron-300">${n("admin.users.role")}</label>
            <select
              value=${l}
              onChange=${h=>c(h.target.value)}
              className="v2-select h-9 w-full rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            >
              <option value="member">${n("admin.users.member")}</option>
              <option value="admin">${n("admin.users.admin")}</option>
            </select>
          </div>
        </div>
        ${a&&u`<p className="text-sm text-[var(--v2-danger-text)]">${a.message}</p>`}
        <div className="flex gap-2">
          <${A} type="submit" disabled=${t}>
            ${n(t?"admin.users.creating":"admin.users.createUser")}
          <//>
          <${A} variant="ghost" type="button" onClick=${()=>m(!1)}>${n("admin.users.cancel")}<//>
        </div>
      </form>
    <//>
  `:u`
      <${A} variant="secondary" onClick=${()=>m(!0)}>
        <${M} name="plus" className="mr-2 h-4 w-4" />
        ${n("admin.users.newUser")}
      <//>
    `}function CO({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=C();return u`
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick=${r}>
      <div className="w-full max-w-md rounded-xl border border-iron-700 bg-iron-900 p-6" onClick=${i=>i.stopPropagation()}>
        <h3 className="text-lg font-semibold text-iron-100">${e}</h3>
        <p className="mt-2 text-sm text-iron-300">${t}</p>
        <div className="mt-5 flex justify-end gap-2">
          <${A} variant="ghost" onClick=${r}>${s("admin.users.cancel")}<//>
          <button
            onClick=${n}
            className="v2-button inline-flex h-10 items-center justify-center rounded-md bg-[var(--v2-danger-soft)] px-4 text-sm font-semibold text-[var(--v2-danger-text)] hover:bg-[color-mix(in_srgb,var(--v2-danger-soft)_65%,var(--v2-danger-text))]"
          >
            ${a}
          </button>
        </div>
      </div>
    </div>
  `}function EO({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=C();return u`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${z} tone=${Ci(e.role)} label=${e.role||"member"} />
          <${z} tone=${ki(e.status)} label=${e.status||"active"} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          ${e.email&&u`<span className="font-mono text-xs text-iron-300">${e.email}</span>`}
          <span className="font-mono text-xs text-iron-700">${Ri(e.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-iron-300 sm:inline">
          ${e.job_count!=null?i("admin.users.jobsCount",{count:e.job_count}):""}
          ${e.total_cost!=null?` \xB7 ${Aa(e.total_cost)}`:""}
        </span>
        <span className="hidden text-xs text-iron-700 lg:inline">${fr(e.last_active_at)}</span>
        <div className="flex gap-1">
          ${e.status==="active"?u`<button onClick=${()=>a(e.id)} className="rounded-md border border-iron-700 px-2.5 py-1.5 text-[11px] font-medium text-iron-300 hover:border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] hover:text-[var(--v2-danger-text)]">${i("admin.users.suspend")}</button>`:u`<button onClick=${()=>n(e.id)} className="rounded-md border border-iron-700 px-2.5 py-1.5 text-[11px] font-medium text-iron-300 hover:border-signal/30 hover:text-signal">${i("admin.users.activate")}</button>`}
          <button
            onClick=${()=>r(e.id,e.role==="admin"?"member":"admin")}
            className="rounded-md border border-iron-700 px-2.5 py-1.5 text-[11px] font-medium text-iron-300 hover:border-iron-700 hover:text-iron-100"
          >
            ${e.role==="admin"?i("admin.users.demote"):i("admin.users.promote")}
          </button>
          <button
            onClick=${()=>s(e.id,e.display_name)}
            className="rounded-md border border-iron-700 px-2.5 py-1.5 text-[11px] font-medium text-iron-300 hover:border-signal/30 hover:text-signal"
          >
            ${i("admin.users.token")}
          </button>
        </div>
      </div>
    </div>
  `}function bk({selectedUserId:e,onSelectUser:t}){let a=C(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:l,updateUser:c,deleteUser:d,suspendUser:m,activateUser:f,createToken:h,newToken:x,clearToken:y}=_i(),[$,g]=p.default.useState(""),[v,b]=p.default.useState("all"),[w,S]=p.default.useState(null),E=mk(n,{search:$,filter:v}),N=_O(a),T=O=>{S({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{m(O),S(null)}})},L=async(O,P)=>{let k=window.prompt(a("admin.users.tokenNamePrompt",{name:P||a("admin.users.userFallback")}));k&&await h(O,k)};return r.isLoading?u`
      <${q} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-3 w-24 rounded" />
        ${[1,2,3].map(O=>u`
          <div key=${O} className="flex items-center justify-between border-t border-iron-700 py-3.5 first:border-0">
            <div className="v2-skeleton h-4 w-32 rounded" />
            <div className="v2-skeleton h-6 w-20 rounded-full" />
          </div>
        `)}
      <//>
    `:s?u`
      <${q} className="p-6 sm:p-8">
        <div className="flex items-center gap-3">
          <${M} name="lock" className="h-5 w-5 text-iron-700" />
          <h3 className="text-lg font-semibold text-iron-100">${a("users.adminRequired")}</h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${a("users.adminRequiredDesc")}
        </p>
      <//>
    `:u`
    <div className="space-y-5">
      ${x&&u`
        <${RO}
          token=${x.token||x.plaintext_token}
          onDismiss=${y}
        />
      `}

      <${kO} onCreate=${i} isCreating=${o} error=${l} />

      <${q} className="p-5 sm:p-6">
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${a("admin.users.title",{count:E.length,total:n.length})}
          </h3>
          <div className="flex items-center gap-2">
            <input
              type="text"
              placeholder=${a("admin.users.searchPlaceholder")}
              value=${$}
              onChange=${O=>g(O.target.value)}
              className="h-8 w-48 rounded-md border border-iron-700 bg-iron-800/70 px-3 text-xs text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
            />
            <div className="flex gap-1">
              ${N.map(O=>u`
                  <button
                    key=${O.value}
                    onClick=${()=>b(O.value)}
                    className=${["rounded-md px-2.5 py-1.5 text-[11px] font-medium",v===O.value?"border border-signal/35 bg-signal/10 text-iron-100":"border border-transparent text-iron-300 hover:text-iron-100"].join(" ")}
                  >
                    ${O.label}
                  </button>
                `)}
            </div>
          </div>
        </div>

        ${E.length===0?u`<p className="py-4 text-sm text-iron-300">${a("admin.users.noMatch")}</p>`:E.map(O=>u`
                <${EO}
                  key=${O.id}
                  user=${O}
                  onSelect=${t}
                  onSuspend=${T}
                  onActivate=${f}
                  onChangeRole=${(P,k)=>c(P,{role:k})}
                  onCreateToken=${L}
                />
              `)}
      <//>

      ${w&&u`
        <${CO}
          title=${w.title}
          message=${w.message}
          confirmLabel=${w.confirmLabel}
          onConfirm=${w.onConfirm}
          onCancel=${()=>S(null)}
        />
      `}
    </div>
  `}function xk(){let{tab:e="dashboard"}=st(),t=he(),[a,n]=p.default.useState(null),r=p.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=p.default.useCallback(()=>{n(null)},[]),i={dashboard:u`<${vk}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?u`<${yk} userId=${a} onBack=${s} />`:u`<${bk}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:u`<${gk} onSelectUser=${r} />`};return i[e]?u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:u`<${it} to="/admin/dashboard" replace />`}var TO=2e3,AO=500,DO=2e3,MO=new Set([403,404]),OO=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function LO(e=globalThis.location,t=null){let a=new URLSearchParams(e?.search||""),n={active:[]};for(let[r,s,i]of OO){let o=a.get(s)?.trim();o?(n[r]=o,n.active.push({key:r,param:s,labelKey:i,value:o})):n[r]=null}return!n.threadId&&t&&(n.threadId=t),n}function $k({isAdmin:e=!1,defaultThreadId:t=null}={}){let a=Ae(),n=a?.search||"",r=p.default.useMemo(()=>LO(a,t),[t,n]),{runId:s,source:i,threadId:o,toolCallId:l,toolName:c,turnId:d}=r,[m,f]=p.default.useState([]),[h,x]=p.default.useState("all"),[y,$]=p.default.useState(""),[g,v]=p.default.useState(!1),[b,w]=p.default.useState(!0),[S,E]=p.default.useState(!0),[N,T]=p.default.useState(null),L=p.default.useRef(new Set),O=p.default.useRef(0),P=!e&&!o;p.default.useEffect(()=>{O.current+=1,f([]),T(null)},[e,s,i,o,l,c,d]);let k=p.default.useCallback(async()=>{if(P){E(!1);return}let ae=++O.current;E(!0);try{let se={limit:AO,level:h==="all"?null:h,target:y.trim()||null,threadId:o,runId:s,turnId:d,toolCallId:l,toolName:c,source:i},ie;try{ie=await(e?s$(se):Pp(se))}catch(Ue){if(!e||!MO.has(Ue?.status))throw Ue;ie=await Pp(se)}if(ae!==O.current)return;let ot=L.current,ft=l_(ie).entries.filter(Ue=>!ot.has(Ue.id));f(ft),T(null)}catch(se){if(ae!==O.current)return;T(se)}finally{ae===O.current&&E(!1)}},[e,h,P,s,i,y,o,l,c,d]);p.default.useEffect(()=>{k()},[k]),p.default.useEffect(()=>{if(g||P)return;let ae=setInterval(k,TO);return()=>clearInterval(ae)},[k,P,g]);let U=p.default.useCallback(()=>{v(ae=>!ae)},[]),Y=p.default.useCallback(()=>{let ae=[...L.current,...m.map(se=>se.id)].slice(-DO);L.current=new Set(ae),f([])},[m]);return{entries:m,totalCount:m.length,paused:g,togglePause:U,clearEntries:Y,levelFilter:h,setLevelFilter:x,targetFilter:y,setTargetFilter:$,autoScroll:b,setAutoScroll:w,serverLevel:null,changeServerLevel:async()=>{},scope:r,needsThreadScope:P,status:P?"needs_scope":N?"error":S?"loading":"ready",isLoading:S,error:N}}var PO=["all","trace","debug","info","warn","error"],jO=["trace","debug","info","warn","error"],wk={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},UO={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function FO({entry:e}){let t=C(),[a,n]=p.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=wk[e.level]||wk.info,i=UO[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(l=>!!l.value);return u`
    <div data-testid="logs-entry" className=${i}>
      <div
        data-testid="logs-entry-row"
        onClick=${()=>n(l=>!l)}
        className=${["grid cursor-pointer select-none gap-x-3 px-4 py-1 font-mono text-xs hover:bg-[var(--v2-surface-muted)]","grid-cols-[7rem_3rem_minmax(10rem,18rem)_1fr]"].join(" ")}
      >
        <span className="text-[var(--v2-text-muted)] tabular-nums">${r}</span>
        <span className=${["font-semibold uppercase",s].join(" ")}>
          ${e.level}
        </span>
        <span className="truncate text-[var(--v2-text-muted)]">${e.target}</span>
        <span
          data-testid="logs-entry-message"
          className=${["min-w-0 text-[var(--v2-text-base)]",a?"whitespace-pre-wrap break-all":"truncate"].join(" ")}
        >
          ${e.message}
        </span>
      </div>
      ${a&&o.length>0&&u`
        <div
          data-testid="logs-entry-context"
          className="flex flex-wrap gap-1.5 px-4 pb-2 pl-[calc(7rem+3rem+2.5rem)] font-mono text-[11px] text-[var(--v2-text-muted)]"
        >
          ${o.map(l=>u`
              <span
                key=${l.key}
                data-testid="logs-context-chip"
                data-context-key=${l.key}
                className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-0.5"
              >
                <span>${t(l.labelKey)}</span>
                <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${l.value}</span>
              </span>
            `)}
        </div>
      `}
    </div>
  `}function Sk({value:e,onChange:t,options:a,labelKey:n,t:r}){return u`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>u`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function BO({label:e,value:t,scopeKey:a}){return u`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function Nk(){let e=C(),{isAdmin:t=!1,threadsState:a}=ba()||{},{entries:n,totalCount:r,paused:s,togglePause:i,clearEntries:o,levelFilter:l,setLevelFilter:c,targetFilter:d,setTargetFilter:m,autoScroll:f,setAutoScroll:h,serverLevel:x,changeServerLevel:y,scope:$,isLoading:g,error:v,needsThreadScope:b}=$k({isAdmin:t,defaultThreadId:t?null:a?.activeThreadId||null}),w=p.default.useRef(null),S=p.default.useRef(!0);p.default.useEffect(()=>{f&&S.current&&w.current&&(w.current.scrollTop=0)},[n,f]);let E=p.default.useCallback(L=>{S.current=L.currentTarget.scrollTop<=48},[]),N=n.length>0,T=$?.active||[];return u`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${Sk}
          value=${l}
          onChange=${c}
          options=${PO}
          labelKey=${L=>L==="all"?"logs.levelAll":`logs.level.${L}`}
          t=${e}
        />

        <!-- Target filter -->
        <input
          type="text"
          value=${d}
          onInput=${L=>m(L.target.value)}
          placeholder=${e("logs.filterTarget")}
          className="h-8 min-w-[10rem] flex-1 rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-3 text-xs text-[var(--v2-text-base)] placeholder:text-[var(--v2-text-muted)] focus:outline-none focus:ring-1 focus:ring-[var(--v2-accent)]"
        />

        <div className="flex items-center gap-2 ml-auto">
          <span className="hidden tabular-nums text-xs text-[var(--v2-text-muted)] sm:inline">
            ${e("logs.entryCount",{count:r})}
          </span>

          <!-- Auto-scroll toggle -->
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-[var(--v2-text-muted)]">
            <input
              type="checkbox"
              checked=${f}
              onChange=${L=>h(L.target.checked)}
              className="h-3.5 w-3.5 accent-[var(--v2-accent)]"
            />
            ${e("logs.autoScroll")}
          </label>

          <!-- Pause/Resume -->
          <button
            onClick=${i}
            className=${["h-8 rounded-[8px] px-3 text-xs font-medium",s?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)] hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)]":"border border-[var(--v2-panel-border)] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"].join(" ")}
          >
            ${e(s?"logs.resume":"logs.pause")}
          </button>

          <!-- Clear -->
          <button
            onClick=${()=>{confirm(e("logs.confirmClear"))&&o()}}
            className="h-8 rounded-[8px] border border-[var(--v2-panel-border)] px-3 text-xs text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          >
            ${e("logs.clear")}
          </button>
        </div>

        ${T.length>0&&u`
          <div
            data-testid="logs-scope-toolbar"
            className="flex w-full flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]"
          >
            <span className="font-medium text-[var(--v2-text-strong)]">${e("logs.scoped")}</span>
            ${T.map(L=>u`<${BO} key=${L.param} scopeKey=${L.param} label=${e(L.labelKey)} value=${L.value} />`)}
            <a
              href="/v2/logs"
              className="ml-auto rounded-[6px] px-2 py-1 text-xs text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
            >
              ${e("logs.clearScope")}
            </a>
          </div>
        `}

        <!-- Server log level -->
        ${x!=null&&u`
          <div className="flex w-full items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]">
            <span>${e("logs.serverLevel")}</span>
            <${Sk}
              value=${x}
              onChange=${y}
              options=${jO}
              labelKey=${L=>`logs.level.${L}`}
              t=${e}
            />
            <span className="ml-auto tabular-nums">
              ${e("logs.entryCount",{count:r})}
              ${s?u`<span className="ml-1 text-yellow-400">${e("logs.pausedBadge")}</span>`:null}
            </span>
          </div>
        `}
      </div>

      <!-- Log output -->
      <div
        ref=${w}
        onScroll=${E}
        className="min-h-0 flex-1 overflow-y-auto bg-[var(--v2-canvas)]"
      >
        ${v&&N?u`
              <div
                className="sticky top-0 z-10 border-b border-red-500/25 bg-red-950/70 px-4 py-2 text-xs text-red-100 backdrop-blur"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:v.message||v.statusText||"Request failed"})}
              </div>
            `:null}
        ${b?u`
              <div
                data-testid="logs-select-thread-state"
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("chat.selectConversation")}
              </div>
            `:v&&!N?u`
              <div
                className="flex h-full items-center justify-center px-6 text-center text-sm text-red-300"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:v.message||v.statusText||"Request failed"})}
              </div>
            `:g&&!N?u`
                <div
                  className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
                >
                  ${e("common.loading")}
                </div>
              `:N?n.map(L=>u`<${FO} key=${L.id} entry=${L} />`):u`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function Rk(){return u`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function zO({auth:e}){let t=he(),n=Ae().state?.from,r=n?`${n.pathname||Ir}${n.search||""}${n.hash||""}`:Ir,s=`/v2${r==="/"?"":r}`,i=p.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?u`<${Rk} />`:e.isAuthenticated?u`<${it} to=${r} replace />`:u`<${F1}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function qO({auth:e,children:t}){let a=Ae();return e.isChecking?u`<${Rk} />`:e.isAuthenticated?t:u`<${it} to="/login" replace state=${{from:a}} />`}function IO({auth:e}){return u`
    <${qO} auth=${e}>
      <${p1}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        rebornProjectsEnabled=${e.rebornProjectsEnabled}
        onSignOut=${e.signOut}
      />
    <//>
  `}function _k({auth:e}){return e.isAdmin?u`<${xk} />`:u`<${it} to=${Ir} replace />`}function kk(){let e=H$();return u`
    <${Dp} basename="/v2">
      <${Cp}>
        <${be} path="/login" element=${u`<${zO} auth=${e} />`} />
        <${be} path="/" element=${u`<${IO} auth=${e} />`}>
          <${be} index element=${u`<${it} to=${Ir} replace />`} />
          <${be} path="overview" element=${u`<${it} to=${Ir} replace />`} />
          <${be} path="welcome" element=${u`<${dS} />`} />
          <${be} path="chat" element=${u`<${_h} />`} />
          <${be} path="chat/:threadId" element=${u`<${_h} />`} />
          <${be} path="workspace" element=${u`<${kh} />`} />
          <${be} path="workspace/*" element=${u`<${kh} />`} />
          <${be} path="projects" element=${u`<${ml} />`} />
          <${be} path="projects/:projectId" element=${u`<${ml} />`} />
          <${be} path="projects/:projectId/missions/:missionId" element=${u`<${ml} />`} />
          <${be} path="projects/:projectId/threads/:threadId" element=${u`<${ml} />`} />
          <${be} path="missions" element=${u`<${Eh} />`} />
          <${be} path="missions/:missionId" element=${u`<${Eh} />`} />
          <${be} path="jobs" element=${u`<${Dh} />`} />
          <${be} path="jobs/:jobId" element=${u`<${Dh} />`} />
          <${be} path="routines" element=${u`<${Oh} />`} />
          <${be} path="routines/:routineId" element=${u`<${Oh} />`} />
          <${be} path="automations" element=${u`<${b_} />`} />
          <${be} path="extensions" element=${u`<${Qh} />`} />
          <${be} path="extensions/:tab" element=${u`<${Qh} />`} />
          <${be} path="logs" element=${u`<${Nk} />`} />
          <${be} path="settings" element=${u`<${Xh} />`} />
          <${be} path="settings/:tab" element=${u`<${Xh} />`} />
          <${be} path="admin" element=${u`<${_k} auth=${e} />`} />
          <${be} path="admin/:tab" element=${u`<${_k} auth=${e} />`} />
        <//>
        <${be} path="*" element=${u`<${it} to=${Ir} replace />`} />
      <//>
    <//>
  `}ev("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.composerLabel":"Message IronClaw","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","approval.viewFullCommand":"View full command","approval.showCommandPreview":"Show preview","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.tabDeclined":"Declined","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitDeclined":"declined","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.serviceUnavailable":"Service unavailable","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveEligibleTools":"Always allow eligible tools","settings.field.autoApproveEligibleToolsDesc":"Applies to tools set to \u201CFollow global\u201D. Per-tool settings override this; hard-floor approval gates still ask.","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.followDefault":"Follow global","tools.sourceDefault":"default permission","tools.sourceGlobal":"global setting","tools.sourceOverride":"per-tool override","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","skills.defaultAutoActivationEnabled":"Default skill auto-activation enabled","skills.defaultAutoActivationDisabled":"Default skill auto-activation disabled","skills.defaultAutoActivationOnDesc":"Skills auto-activate by keyword on matching requests. Turn off to require an explicit /name.","skills.defaultAutoActivationOffDesc":"Skills run only when you type /name. Turn on to let them auto-activate by keyword.","skills.defaultAutoActivationOnButton":"Default: On","skills.defaultAutoActivationOffButton":"Default: Off","skills.autoActivateOnTitle":"Auto-activation on \u2014 runs on matching requests. Click to make it explicit-only (/name).","skills.autoActivateOffTitle":"Explicit-only \u2014 runs only when you type /name. Click to enable auto-activation.","skills.autoActivateOnLabel":"Auto-activate: On","skills.autoActivateOffLabel":"Auto-activate: Off","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Memory, files & attachments","workspace.readOnly":"Read-only","workspace.filterPlaceholder":"Filter by name\u2026","workspace.emptyDir":"This folder is empty.","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.noFiles":"No files here.","workspace.noMatches":"Nothing matches that filter.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a file","workspace.pickFileDesc":"Choose a file from the tree to preview or download it. This viewer is read-only.","workspace.parent":"Parent: {path}","workspace.download":"Download","workspace.binaryPreviewUnavailable":"No inline preview for this file type. Download it to view the contents.","workspace.fileMeta":"{mime} \xB7 {size} bytes","workspace.unableOpenDirectory":"Unable to open directory","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.filter.completed":"Completed","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.schedule.onceAt":"Once on {datetime}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.filterAction":"Show {label}","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.empty.onboardingTitle":"No automations yet","automations.empty.onboardingDescription":"Automations are created by chatting with your agent \u2014 there's no form to fill out. Ask it to do something on a schedule and it will set up a recurring automation for you.","automations.empty.examplesTitle":"Try asking your agent","automations.empty.example1":"Check the nearai/ironclaw repo every 10 minutes and summarize new issues, PRs, and commits.","automations.empty.example2":"Every weekday at 9am, send me a summary of my unread email.","automations.empty.example3":"Remind me to review open pull requests every afternoon at 3pm.","automations.empty.startInChat":"Start in chat","automations.empty.copyPrompt":"Copy prompt","automations.empty.copied":"Copied","automations.refreshing":"Refreshing\u2026","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.runs.total":"Recent runs: {count}","automations.runs.ok":"OK: {count}","automations.runs.error":"Failed: {count}","automations.runs.running":"Running: {count}","automations.runs.unknown":"Unknown: {count}","automations.runs.showingOf":"Showing {shown} of {total} recent runs","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up a project for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open project","projects.openGeneralWorkspace":"Open project","projects.noDescription":"No project description yet. The project is still being shaped by recent activity and thread history.","projects.general.label":"General project","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,Ck.createRoot)(document.getElementById("v2-root")).render(u`
  <${tv}>
    <${Md} client=${At}>
      <${kk} />
    <//>
  <//>
`);
