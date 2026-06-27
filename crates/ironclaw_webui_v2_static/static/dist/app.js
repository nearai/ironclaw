import{a as _n,b as Fe,c as Ie,d as p,e as u,f as ev,g as tv,h as $l,i as R,j as wl}from"./chunks/chunk-IGTNS7XG.js";var xv=_n(Al=>{"use strict";var ER=Symbol.for("react.transitional.element"),TR=Symbol.for("react.fragment");function bv(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:ER,type:e,key:n,ref:t!==void 0?t:null,props:a}}Al.Fragment=TR;Al.jsx=bv;Al.jsxs=bv});var Md=_n((XL,$v)=>{"use strict";$v.exports=xv()});var Lv=_n(De=>{"use strict";function Bd(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<Bl(r,t))e[n]=t,e[a]=r,a=n;else break e}}function Oa(e){return e.length===0?null:e[0]}function ql(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],l=i+1,c=e[l];if(0>Bl(o,a))l<r&&0>Bl(c,o)?(e[n]=c,e[l]=a,n=l):(e[n]=o,e[i]=a,n=i);else if(l<r&&0>Bl(c,a))e[n]=c,e[l]=a,n=l;else break e}}return t}function Bl(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}De.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(kv=performance,De.unstable_now=function(){return kv.now()}):(Ud=Date,Rv=Ud.now(),De.unstable_now=function(){return Ud.now()-Rv});var kv,Ud,Rv,Wa=[],Cn=[],OR=1,ua=null,bt=3,zd=!1,zi=!1,qi=!1,qd=!1,Tv=typeof setTimeout=="function"?setTimeout:null,Av=typeof clearTimeout=="function"?clearTimeout:null,Cv=typeof setImmediate<"u"?setImmediate:null;function zl(e){for(var t=Oa(Cn);t!==null;){if(t.callback===null)ql(Cn);else if(t.startTime<=e)ql(Cn),t.sortIndex=t.expirationTime,Bd(Wa,t);else break;t=Oa(Cn)}}function Id(e){if(qi=!1,zl(e),!zi)if(Oa(Wa)!==null)zi=!0,us||(us=!0,ls());else{var t=Oa(Cn);t!==null&&Kd(Id,t.startTime-e)}}var us=!1,Ii=-1,Dv=5,Mv=-1;function Ov(){return qd?!0:!(De.unstable_now()-Mv<Dv)}function jd(){if(qd=!1,us){var e=De.unstable_now();Mv=e;var t=!0;try{e:{zi=!1,qi&&(qi=!1,Av(Ii),Ii=-1),zd=!0;var a=bt;try{t:{for(zl(e),ua=Oa(Wa);ua!==null&&!(ua.expirationTime>e&&Ov());){var n=ua.callback;if(typeof n=="function"){ua.callback=null,bt=ua.priorityLevel;var r=n(ua.expirationTime<=e);if(e=De.unstable_now(),typeof r=="function"){ua.callback=r,zl(e),t=!0;break t}ua===Oa(Wa)&&ql(Wa),zl(e)}else ql(Wa);ua=Oa(Wa)}if(ua!==null)t=!0;else{var s=Oa(Cn);s!==null&&Kd(Id,s.startTime-e),t=!1}}break e}finally{ua=null,bt=a,zd=!1}t=void 0}}finally{t?ls():us=!1}}}var ls;typeof Cv=="function"?ls=function(){Cv(jd)}:typeof MessageChannel<"u"?(Fd=new MessageChannel,Ev=Fd.port2,Fd.port1.onmessage=jd,ls=function(){Ev.postMessage(null)}):ls=function(){Tv(jd,0)};var Fd,Ev;function Kd(e,t){Ii=Tv(function(){e(De.unstable_now())},t)}De.unstable_IdlePriority=5;De.unstable_ImmediatePriority=1;De.unstable_LowPriority=4;De.unstable_NormalPriority=3;De.unstable_Profiling=null;De.unstable_UserBlockingPriority=2;De.unstable_cancelCallback=function(e){e.callback=null};De.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):Dv=0<e?Math.floor(1e3/e):5};De.unstable_getCurrentPriorityLevel=function(){return bt};De.unstable_next=function(e){switch(bt){case 1:case 2:case 3:var t=3;break;default:t=bt}var a=bt;bt=t;try{return e()}finally{bt=a}};De.unstable_requestPaint=function(){qd=!0};De.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=bt;bt=e;try{return t()}finally{bt=a}};De.unstable_scheduleCallback=function(e,t,a){var n=De.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:OR++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,Bd(Cn,e),Oa(Wa)===null&&e===Oa(Cn)&&(qi?(Av(Ii),Ii=-1):qi=!0,Kd(Id,a-n))):(e.sortIndex=r,Bd(Wa,e),zi||zd||(zi=!0,us||(us=!0,ls()))),e};De.unstable_shouldYield=Ov;De.unstable_wrapCallback=function(e){var t=bt;return function(){var a=bt;bt=t;try{return e.apply(this,arguments)}finally{bt=a}}}});var Uv=_n((M6,Pv)=>{"use strict";Pv.exports=Lv()});var Fv=_n(_t=>{"use strict";var LR=Ie();function jv(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function En(){}var Nt={d:{f:En,r:function(){throw Error(jv(522))},D:En,C:En,L:En,m:En,X:En,S:En,M:En},p:0,findDOMNode:null},PR=Symbol.for("react.portal");function UR(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:PR,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var Ki=LR.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function Il(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}_t.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=Nt;_t.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(jv(299));return UR(e,t,null,a)};_t.flushSync=function(e){var t=Ki.T,a=Nt.p;try{if(Ki.T=null,Nt.p=2,e)return e()}finally{Ki.T=t,Nt.p=a,Nt.d.f()}};_t.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,Nt.d.C(e,t))};_t.prefetchDNS=function(e){typeof e=="string"&&Nt.d.D(e)};_t.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=Il(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?Nt.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&Nt.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};_t.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=Il(t.as,t.crossOrigin);Nt.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&Nt.d.M(e)};_t.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=Il(a,t.crossOrigin);Nt.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};_t.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=Il(t.as,t.crossOrigin);Nt.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else Nt.d.m(e)};_t.requestFormReset=function(e){Nt.d.r(e)};_t.unstable_batchedUpdates=function(e,t){return e(t)};_t.useFormState=function(e,t,a){return Ki.H.useFormState(e,t,a)};_t.useFormStatus=function(){return Ki.H.useHostTransitionStatus()};_t.version="19.1.0"});var qv=_n((L6,zv)=>{"use strict";function Bv(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(Bv)}catch(e){console.error(e)}}Bv(),zv.exports=Fv()});var K0=_n(dc=>{"use strict";var rt=Uv(),cy=Ie(),jR=qv();function U(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function dy(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function Do(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function my(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function Iv(e){if(Do(e)!==e)throw Error(U(188))}function FR(e){var t=e.alternate;if(!t){if(t=Do(e),t===null)throw Error(U(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return Iv(r),e;if(s===n)return Iv(r),t;s=s.sibling}throw Error(U(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(U(189))}}if(a.alternate!==n)throw Error(U(190))}if(a.tag!==3)throw Error(U(188));return a.stateNode.current===a?e:t}function fy(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=fy(e),t!==null)return t;e=e.sibling}return null}var Te=Object.assign,BR=Symbol.for("react.element"),Kl=Symbol.for("react.transitional.element"),Wi=Symbol.for("react.portal"),vs=Symbol.for("react.fragment"),py=Symbol.for("react.strict_mode"),wm=Symbol.for("react.profiler"),zR=Symbol.for("react.provider"),hy=Symbol.for("react.consumer"),rn=Symbol.for("react.context"),yf=Symbol.for("react.forward_ref"),Sm=Symbol.for("react.suspense"),Nm=Symbol.for("react.suspense_list"),bf=Symbol.for("react.memo"),Dn=Symbol.for("react.lazy");Symbol.for("react.scope");var _m=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var qR=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var Kv=Symbol.iterator;function Hi(e){return e===null||typeof e!="object"?null:(e=Kv&&e[Kv]||e["@@iterator"],typeof e=="function"?e:null)}var IR=Symbol.for("react.client.reference");function km(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===IR?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case vs:return"Fragment";case wm:return"Profiler";case py:return"StrictMode";case Sm:return"Suspense";case Nm:return"SuspenseList";case _m:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case Wi:return"Portal";case rn:return(e.displayName||"Context")+".Provider";case hy:return(e._context.displayName||"Context")+".Consumer";case yf:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case bf:return t=e.displayName||null,t!==null?t:km(e.type)||"Memo";case Dn:t=e._payload,e=e._init;try{return km(e(t))}catch{}}return null}var eo=Array.isArray,te=cy.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,he=jR.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,yr={pending:!1,data:null,method:null,action:null},Rm=[],gs=-1;function za(e){return{current:e}}function ct(e){0>gs||(e.current=Rm[gs],Rm[gs]=null,gs--)}function Oe(e,t){gs++,Rm[gs]=e.current,e.current=t}var ja=za(null),yo=za(null),qn=za(null),xu=za(null);function $u(e,t){switch(Oe(qn,t),Oe(yo,e),Oe(ja,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?Jg(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=Jg(t),e=D0(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}ct(ja),Oe(ja,e)}function Ps(){ct(ja),ct(yo),ct(qn)}function Cm(e){e.memoizedState!==null&&Oe(xu,e);var t=ja.current,a=D0(t,e.type);t!==a&&(Oe(yo,e),Oe(ja,a))}function wu(e){yo.current===e&&(ct(ja),ct(yo)),xu.current===e&&(ct(xu),Co._currentValue=yr)}var Em=Object.prototype.hasOwnProperty,xf=rt.unstable_scheduleCallback,Hd=rt.unstable_cancelCallback,KR=rt.unstable_shouldYield,HR=rt.unstable_requestPaint,Fa=rt.unstable_now,QR=rt.unstable_getCurrentPriorityLevel,vy=rt.unstable_ImmediatePriority,gy=rt.unstable_UserBlockingPriority,Su=rt.unstable_NormalPriority,VR=rt.unstable_LowPriority,yy=rt.unstable_IdlePriority,GR=rt.log,YR=rt.unstable_setDisableYieldValue,Mo=null,Gt=null;function jn(e){if(typeof GR=="function"&&YR(e),Gt&&typeof Gt.setStrictMode=="function")try{Gt.setStrictMode(Mo,e)}catch{}}var Yt=Math.clz32?Math.clz32:ZR,JR=Math.log,XR=Math.LN2;function ZR(e){return e>>>=0,e===0?32:31-(JR(e)/XR|0)|0}var Hl=256,Ql=4194304;function hr(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Ju(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=hr(n):(i&=o,i!==0?r=hr(i):a||(a=o&~e,a!==0&&(r=hr(a))))):(o=n&~s,o!==0?r=hr(o):i!==0?r=hr(i):a||(a=n&~e,a!==0&&(r=hr(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function Oo(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function WR(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function by(){var e=Hl;return Hl<<=1,(Hl&4194048)===0&&(Hl=256),e}function xy(){var e=Ql;return Ql<<=1,(Ql&62914560)===0&&(Ql=4194304),e}function Qd(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function Lo(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function eC(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,l=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Yt(a),m=1<<d;o[d]=0,l[d]=-1;var f=c[d];if(f!==null)for(c[d]=null,d=0;d<f.length;d++){var h=f[d];h!==null&&(h.lane&=-536870913)}a&=~m}n!==0&&$y(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function $y(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Yt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function wy(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Yt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function $f(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function wf(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function Sy(){var e=he.p;return e!==0?e:(e=window.event,e===void 0?32:q0(e.type))}function tC(e,t){var a=he.p;try{return he.p=e,t()}finally{he.p=a}}var Wn=Math.random().toString(36).slice(2),xt="__reactFiber$"+Wn,Ft="__reactProps$"+Wn,Vs="__reactContainer$"+Wn,Tm="__reactEvents$"+Wn,aC="__reactListeners$"+Wn,nC="__reactHandles$"+Wn,Hv="__reactResources$"+Wn,Po="__reactMarker$"+Wn;function Sf(e){delete e[xt],delete e[Ft],delete e[Tm],delete e[aC],delete e[nC]}function ys(e){var t=e[xt];if(t)return t;for(var a=e.parentNode;a;){if(t=a[Vs]||a[xt]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=Wg(e);e!==null;){if(a=e[xt])return a;e=Wg(e)}return t}e=a,a=e.parentNode}return null}function Gs(e){if(e=e[xt]||e[Vs]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function to(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(U(33))}function Cs(e){var t=e[Hv];return t||(t=e[Hv]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function lt(e){e[Po]=!0}var Ny=new Set,_y={};function Er(e,t){Us(e,t),Us(e+"Capture",t)}function Us(e,t){for(_y[e]=t,e=0;e<t.length;e++)Ny.add(t[e])}var rC=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),Qv={},Vv={};function sC(e){return Em.call(Vv,e)?!0:Em.call(Qv,e)?!1:rC.test(e)?Vv[e]=!0:(Qv[e]=!0,!1)}function ou(e,t,a){if(sC(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function Vl(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function en(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var Vd,Gv;function fs(e){if(Vd===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);Vd=t&&t[1]||"",Gv=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+Vd+e+Gv}var Gd=!1;function Yd(e,t){if(!e||Gd)return"";Gd=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var m=function(){throw Error()};if(Object.defineProperty(m.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(m,[])}catch(h){var f=h}Reflect.construct(e,[],m)}else{try{m.call()}catch(h){f=h}e.call(m.prototype)}}else{try{throw Error()}catch(h){f=h}(m=e())&&typeof m.catch=="function"&&m.catch(function(){})}}catch(h){if(h&&f&&typeof h.stack=="string")return[h.stack,f.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var l=i.split(`
`),c=o.split(`
`);for(r=n=0;n<l.length&&!l[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===l.length||r===c.length)for(n=l.length-1,r=c.length-1;1<=n&&0<=r&&l[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(l[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||l[n]!==c[r]){var d=`
`+l[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{Gd=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?fs(a):""}function iC(e){switch(e.tag){case 26:case 27:case 5:return fs(e.type);case 16:return fs("Lazy");case 13:return fs("Suspense");case 19:return fs("SuspenseList");case 0:case 15:return Yd(e.type,!1);case 11:return Yd(e.type.render,!1);case 1:return Yd(e.type,!0);case 31:return fs("Activity");default:return""}}function Yv(e){try{var t="";do t+=iC(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function da(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function ky(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function oC(e){var t=ky(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function Nu(e){e._valueTracker||(e._valueTracker=oC(e))}function Ry(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=ky(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function _u(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var lC=/[\n"\\]/g;function pa(e){return e.replace(lC,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function Am(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+da(t)):e.value!==""+da(t)&&(e.value=""+da(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?Dm(e,i,da(t)):a!=null?Dm(e,i,da(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+da(o):e.removeAttribute("name")}function Cy(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+da(a):"",t=t!=null?""+da(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function Dm(e,t,a){t==="number"&&_u(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function Es(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+da(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function Ey(e,t,a){if(t!=null&&(t=""+da(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+da(a):""}function Ty(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(U(92));if(eo(n)){if(1<n.length)throw Error(U(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=da(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function js(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var uC=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function Jv(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||uC.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function Ay(e,t,a){if(t!=null&&typeof t!="object")throw Error(U(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&Jv(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&Jv(e,s,t[s])}function Nf(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var cC=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),dC=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function lu(e){return dC.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var Mm=null;function _f(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var bs=null,Ts=null;function Xv(e){var t=Gs(e);if(t&&(e=t.stateNode)){var a=e[Ft]||null;e:switch(e=t.stateNode,t.type){case"input":if(Am(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+pa(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[Ft]||null;if(!r)throw Error(U(90));Am(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&Ry(n)}break e;case"textarea":Ey(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&Es(e,!!a.multiple,t,!1)}}}var Jd=!1;function Dy(e,t,a){if(Jd)return e(t,a);Jd=!0;try{var n=e(t);return n}finally{if(Jd=!1,(bs!==null||Ts!==null)&&(ic(),bs&&(t=bs,e=Ts,Ts=bs=null,Xv(t),e)))for(t=0;t<e.length;t++)Xv(e[t])}}function bo(e,t){var a=e.stateNode;if(a===null)return null;var n=a[Ft]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(U(231,t,typeof a));return a}var mn=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),Om=!1;if(mn)try{cs={},Object.defineProperty(cs,"passive",{get:function(){Om=!0}}),window.addEventListener("test",cs,cs),window.removeEventListener("test",cs,cs)}catch{Om=!1}var cs,Fn=null,kf=null,uu=null;function My(){if(uu)return uu;var e,t=kf,a=t.length,n,r="value"in Fn?Fn.value:Fn.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return uu=r.slice(e,1<n?1-n:void 0)}function cu(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function Gl(){return!0}function Zv(){return!1}function Bt(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?Gl:Zv,this.isPropagationStopped=Zv,this}return Te(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=Gl)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=Gl)},persist:function(){},isPersistent:Gl}),t}var Tr={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Xu=Bt(Tr),Uo=Te({},Tr,{view:0,detail:0}),mC=Bt(Uo),Xd,Zd,Qi,Zu=Te({},Uo,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:Rf,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==Qi&&(Qi&&e.type==="mousemove"?(Xd=e.screenX-Qi.screenX,Zd=e.screenY-Qi.screenY):Zd=Xd=0,Qi=e),Xd)},movementY:function(e){return"movementY"in e?e.movementY:Zd}}),Wv=Bt(Zu),fC=Te({},Zu,{dataTransfer:0}),pC=Bt(fC),hC=Te({},Uo,{relatedTarget:0}),Wd=Bt(hC),vC=Te({},Tr,{animationName:0,elapsedTime:0,pseudoElement:0}),gC=Bt(vC),yC=Te({},Tr,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),bC=Bt(yC),xC=Te({},Tr,{data:0}),eg=Bt(xC),$C={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},wC={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},SC={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function NC(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=SC[e])?!!t[e]:!1}function Rf(){return NC}var _C=Te({},Uo,{key:function(e){if(e.key){var t=$C[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=cu(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?wC[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:Rf,charCode:function(e){return e.type==="keypress"?cu(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?cu(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),kC=Bt(_C),RC=Te({},Zu,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),tg=Bt(RC),CC=Te({},Uo,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:Rf}),EC=Bt(CC),TC=Te({},Tr,{propertyName:0,elapsedTime:0,pseudoElement:0}),AC=Bt(TC),DC=Te({},Zu,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),MC=Bt(DC),OC=Te({},Tr,{newState:0,oldState:0}),LC=Bt(OC),PC=[9,13,27,32],Cf=mn&&"CompositionEvent"in window,no=null;mn&&"documentMode"in document&&(no=document.documentMode);var UC=mn&&"TextEvent"in window&&!no,Oy=mn&&(!Cf||no&&8<no&&11>=no),ag=" ",ng=!1;function Ly(e,t){switch(e){case"keyup":return PC.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function Py(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var xs=!1;function jC(e,t){switch(e){case"compositionend":return Py(t);case"keypress":return t.which!==32?null:(ng=!0,ag);case"textInput":return e=t.data,e===ag&&ng?null:e;default:return null}}function FC(e,t){if(xs)return e==="compositionend"||!Cf&&Ly(e,t)?(e=My(),uu=kf=Fn=null,xs=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return Oy&&t.locale!=="ko"?null:t.data;default:return null}}var BC={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function rg(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!BC[e.type]:t==="textarea"}function Uy(e,t,a,n){bs?Ts?Ts.push(n):Ts=[n]:bs=n,t=Iu(t,"onChange"),0<t.length&&(a=new Xu("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var ro=null,xo=null;function zC(e){E0(e,0)}function Wu(e){var t=to(e);if(Ry(t))return e}function sg(e,t){if(e==="change")return t}var jy=!1;mn&&(mn?(Jl="oninput"in document,Jl||(em=document.createElement("div"),em.setAttribute("oninput","return;"),Jl=typeof em.oninput=="function"),Yl=Jl):Yl=!1,jy=Yl&&(!document.documentMode||9<document.documentMode));var Yl,Jl,em;function ig(){ro&&(ro.detachEvent("onpropertychange",Fy),xo=ro=null)}function Fy(e){if(e.propertyName==="value"&&Wu(xo)){var t=[];Uy(t,xo,e,_f(e)),Dy(zC,t)}}function qC(e,t,a){e==="focusin"?(ig(),ro=t,xo=a,ro.attachEvent("onpropertychange",Fy)):e==="focusout"&&ig()}function IC(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return Wu(xo)}function KC(e,t){if(e==="click")return Wu(t)}function HC(e,t){if(e==="input"||e==="change")return Wu(t)}function QC(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var Zt=typeof Object.is=="function"?Object.is:QC;function $o(e,t){if(Zt(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!Em.call(t,r)||!Zt(e[r],t[r]))return!1}return!0}function og(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function lg(e,t){var a=og(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=og(a)}}function By(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?By(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function zy(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=_u(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=_u(e.document)}return t}function Ef(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var VC=mn&&"documentMode"in document&&11>=document.documentMode,$s=null,Lm=null,so=null,Pm=!1;function ug(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;Pm||$s==null||$s!==_u(n)||(n=$s,"selectionStart"in n&&Ef(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),so&&$o(so,n)||(so=n,n=Iu(Lm,"onSelect"),0<n.length&&(t=new Xu("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=$s)))}function pr(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var ws={animationend:pr("Animation","AnimationEnd"),animationiteration:pr("Animation","AnimationIteration"),animationstart:pr("Animation","AnimationStart"),transitionrun:pr("Transition","TransitionRun"),transitionstart:pr("Transition","TransitionStart"),transitioncancel:pr("Transition","TransitionCancel"),transitionend:pr("Transition","TransitionEnd")},tm={},qy={};mn&&(qy=document.createElement("div").style,"AnimationEvent"in window||(delete ws.animationend.animation,delete ws.animationiteration.animation,delete ws.animationstart.animation),"TransitionEvent"in window||delete ws.transitionend.transition);function Ar(e){if(tm[e])return tm[e];if(!ws[e])return e;var t=ws[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in qy)return tm[e]=t[a];return e}var Iy=Ar("animationend"),Ky=Ar("animationiteration"),Hy=Ar("animationstart"),GC=Ar("transitionrun"),YC=Ar("transitionstart"),JC=Ar("transitioncancel"),Qy=Ar("transitionend"),Vy=new Map,Um="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");Um.push("scrollEnd");function Na(e,t){Vy.set(e,t),Er(t,[e])}var cg=new WeakMap;function ha(e,t){if(typeof e=="object"&&e!==null){var a=cg.get(e);return a!==void 0?a:(t={value:e,source:t,stack:Yv(t)},cg.set(e,t),t)}return{value:e,source:t,stack:Yv(t)}}var ca=[],Ss=0,Tf=0;function ec(){for(var e=Ss,t=Tf=Ss=0;t<e;){var a=ca[t];ca[t++]=null;var n=ca[t];ca[t++]=null;var r=ca[t];ca[t++]=null;var s=ca[t];if(ca[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&Gy(a,r,s)}}function tc(e,t,a,n){ca[Ss++]=e,ca[Ss++]=t,ca[Ss++]=a,ca[Ss++]=n,Tf|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function Af(e,t,a,n){return tc(e,t,a,n),ku(e)}function Ys(e,t){return tc(e,null,null,t),ku(e)}function Gy(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Yt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function ku(e){if(50<vo)throw vo=0,rf=null,Error(U(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var Ns={};function XC(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Vt(e,t,a,n){return new XC(e,t,a,n)}function Df(e){return e=e.prototype,!(!e||!e.isReactComponent)}function cn(e,t){var a=e.alternate;return a===null?(a=Vt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function Yy(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function du(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")Df(e)&&(i=1);else if(typeof e=="string")i=XE(e,a,ja.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case _m:return e=Vt(31,a,t,r),e.elementType=_m,e.lanes=s,e;case vs:return br(a.children,r,s,t);case py:i=8,r|=24;break;case wm:return e=Vt(12,a,t,r|2),e.elementType=wm,e.lanes=s,e;case Sm:return e=Vt(13,a,t,r),e.elementType=Sm,e.lanes=s,e;case Nm:return e=Vt(19,a,t,r),e.elementType=Nm,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case zR:case rn:i=10;break e;case hy:i=9;break e;case yf:i=11;break e;case bf:i=14;break e;case Dn:i=16,n=null;break e}i=29,a=Error(U(130,e===null?"null":typeof e,"")),n=null}return t=Vt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function br(e,t,a,n){return e=Vt(7,e,n,t),e.lanes=a,e}function am(e,t,a){return e=Vt(6,e,null,t),e.lanes=a,e}function nm(e,t,a){return t=Vt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var _s=[],ks=0,Ru=null,Cu=0,ma=[],fa=0,xr=null,sn=1,on="";function vr(e,t){_s[ks++]=Cu,_s[ks++]=Ru,Ru=e,Cu=t}function Jy(e,t,a){ma[fa++]=sn,ma[fa++]=on,ma[fa++]=xr,xr=e;var n=sn;e=on;var r=32-Yt(n)-1;n&=~(1<<r),a+=1;var s=32-Yt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,sn=1<<32-Yt(t)+r|a<<r|n,on=s+e}else sn=1<<s|a<<r|n,on=e}function Mf(e){e.return!==null&&(vr(e,1),Jy(e,1,0))}function Of(e){for(;e===Ru;)Ru=_s[--ks],_s[ks]=null,Cu=_s[--ks],_s[ks]=null;for(;e===xr;)xr=ma[--fa],ma[fa]=null,on=ma[--fa],ma[fa]=null,sn=ma[--fa],ma[fa]=null}var kt=null,Be=null,pe=!1,$r=null,Pa=!1,jm=Error(U(519));function _r(e){var t=Error(U(418,""));throw wo(ha(t,e)),jm}function dg(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[xt]=e,t[Ft]=n,a){case"dialog":oe("cancel",t),oe("close",t);break;case"iframe":case"object":case"embed":oe("load",t);break;case"video":case"audio":for(a=0;a<_o.length;a++)oe(_o[a],t);break;case"source":oe("error",t);break;case"img":case"image":case"link":oe("error",t),oe("load",t);break;case"details":oe("toggle",t);break;case"input":oe("invalid",t),Cy(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),Nu(t);break;case"select":oe("invalid",t);break;case"textarea":oe("invalid",t),Ty(t,n.value,n.defaultValue,n.children),Nu(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||A0(t.textContent,a)?(n.popover!=null&&(oe("beforetoggle",t),oe("toggle",t)),n.onScroll!=null&&oe("scroll",t),n.onScrollEnd!=null&&oe("scrollend",t),n.onClick!=null&&(t.onclick=uc),t=!0):t=!1,t||_r(e)}function mg(e){for(kt=e.return;kt;)switch(kt.tag){case 5:case 13:Pa=!1;return;case 27:case 3:Pa=!0;return;default:kt=kt.return}}function Vi(e){if(e!==kt)return!1;if(!pe)return mg(e),pe=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||df(e.type,e.memoizedProps)),a=!a),a&&Be&&_r(e),mg(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(U(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){Be=Sa(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}Be=null}}else t===27?(t=Be,er(e.type)?(e=pf,pf=null,Be=e):Be=t):Be=kt?Sa(e.stateNode.nextSibling):null;return!0}function jo(){Be=kt=null,pe=!1}function fg(){var e=$r;return e!==null&&(jt===null?jt=e:jt.push.apply(jt,e),$r=null),e}function wo(e){$r===null?$r=[e]:$r.push(e)}var Fm=za(null),Dr=null,ln=null;function On(e,t,a){Oe(Fm,t._currentValue),t._currentValue=a}function dn(e){e._currentValue=Fm.current,ct(Fm)}function Bm(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function zm(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var l=0;l<t.length;l++)if(o.context===t[l]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),Bm(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(U(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),Bm(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function Fo(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(U(387));if(i=i.memoizedProps,i!==null){var o=r.type;Zt(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===xu.current){if(i=r.alternate,i===null)throw Error(U(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(Co):e=[Co])}r=r.return}e!==null&&zm(t,e,a,n),t.flags|=262144}function Eu(e){for(e=e.firstContext;e!==null;){if(!Zt(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function kr(e){Dr=e,ln=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function $t(e){return Xy(Dr,e)}function Xl(e,t){return Dr===null&&kr(e),Xy(e,t)}function Xy(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},ln===null){if(e===null)throw Error(U(308));ln=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else ln=ln.next=t;return a}var ZC=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},WC=rt.unstable_scheduleCallback,eE=rt.unstable_NormalPriority,at={$$typeof:rn,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function Lf(){return{controller:new ZC,data:new Map,refCount:0}}function Bo(e){e.refCount--,e.refCount===0&&WC(eE,function(){e.controller.abort()})}var io=null,qm=0,Fs=0,As=null;function tE(e,t){if(io===null){var a=io=[];qm=0,Fs=rp(),As={status:"pending",value:void 0,then:function(n){a.push(n)}}}return qm++,t.then(pg,pg),t}function pg(){if(--qm===0&&io!==null){As!==null&&(As.status="fulfilled");var e=io;io=null,Fs=0,As=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function aE(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var hg=te.S;te.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&tE(e,t),hg!==null&&hg(e,t)};var wr=za(null);function Pf(){var e=wr.current;return e!==null?e:_e.pooledCache}function mu(e,t){t===null?Oe(wr,wr.current):Oe(wr,t.pool)}function Zy(){var e=Pf();return e===null?null:{parent:at._currentValue,pool:e}}var zo=Error(U(460)),Wy=Error(U(474)),ac=Error(U(542)),Im={then:function(){}};function vg(e){return e=e.status,e==="fulfilled"||e==="rejected"}function Zl(){}function eb(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(Zl,Zl),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,yg(e),e;default:if(typeof t.status=="string")t.then(Zl,Zl);else{if(e=_e,e!==null&&100<e.shellSuspendCounter)throw Error(U(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,yg(e),e}throw oo=t,zo}}var oo=null;function gg(){if(oo===null)throw Error(U(459));var e=oo;return oo=null,e}function yg(e){if(e===zo||e===ac)throw Error(U(483))}var Mn=!1;function Uf(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function Km(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function In(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function Kn(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,(xe&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=ku(e),Gy(e,null,a),t}return tc(e,n,t,a),ku(e)}function lo(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,wy(e,a)}}function rm(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var Hm=!1;function uo(){if(Hm){var e=As;if(e!==null)throw e}}function co(e,t,a,n){Hm=!1;var r=e.updateQueue;Mn=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var l=o,c=l.next;l.next=null,i===null?s=c:i.next=c,i=l;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=l))}if(s!==null){var m=r.baseState;i=0,d=c=l=null,o=s;do{var f=o.lane&-536870913,h=f!==o.lane;if(h?(ce&f)===f:(n&f)===f){f!==0&&f===Fs&&(Hm=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var x=e,y=o;f=t;var $=a;switch(y.tag){case 1:if(x=y.payload,typeof x=="function"){m=x.call($,m,f);break e}m=x;break e;case 3:x.flags=x.flags&-65537|128;case 0:if(x=y.payload,f=typeof x=="function"?x.call($,m,f):x,f==null)break e;m=Te({},m,f);break e;case 2:Mn=!0}}f=o.callback,f!==null&&(e.flags|=64,h&&(e.flags|=8192),h=r.callbacks,h===null?r.callbacks=[f]:h.push(f))}else h={lane:f,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=h,l=m):d=d.next=h,i|=f;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;h=o,o=h.next,h.next=null,r.lastBaseUpdate=h,r.shared.pending=null}}while(!0);d===null&&(l=m),r.baseState=l,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),Zn|=i,e.lanes=i,e.memoizedState=m}}function tb(e,t){if(typeof e!="function")throw Error(U(191,e));e.call(t)}function ab(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)tb(a[e],t)}var Bs=za(null),Tu=za(0);function bg(e,t){e=hn,Oe(Tu,e),Oe(Bs,t),hn=e|t.baseLanes}function Qm(){Oe(Tu,hn),Oe(Bs,Bs.current)}function jf(){hn=Tu.current,ct(Bs),ct(Tu)}var Jn=0,re=null,Se=null,Ye=null,Au=!1,Ds=!1,Rr=!1,Du=0,So=0,Ms=null,nE=0;function Ke(){throw Error(U(321))}function Ff(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!Zt(e[a],t[a]))return!1;return!0}function Bf(e,t,a,n,r,s){return Jn=s,re=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,te.H=e===null||e.memoizedState===null?Mb:Ob,Rr=!1,s=a(n,r),Rr=!1,Ds&&(s=rb(t,a,n,r)),nb(e),s}function nb(e){te.H=Mu;var t=Se!==null&&Se.next!==null;if(Jn=0,Ye=Se=re=null,Au=!1,So=0,Ms=null,t)throw Error(U(300));e===null||ut||(e=e.dependencies,e!==null&&Eu(e)&&(ut=!0))}function rb(e,t,a,n){re=e;var r=0;do{if(Ds&&(Ms=null),So=0,Ds=!1,25<=r)throw Error(U(301));if(r+=1,Ye=Se=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}te.H=cE,s=t(a,n)}while(Ds);return s}function rE(){var e=te.H,t=e.useState()[0];return t=typeof t.then=="function"?qo(t):t,e=e.useState()[0],(Se!==null?Se.memoizedState:null)!==e&&(re.flags|=1024),t}function zf(){var e=Du!==0;return Du=0,e}function qf(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function If(e){if(Au){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}Au=!1}Jn=0,Ye=Se=re=null,Ds=!1,So=Du=0,Ms=null}function Pt(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return Ye===null?re.memoizedState=Ye=e:Ye=Ye.next=e,Ye}function Je(){if(Se===null){var e=re.alternate;e=e!==null?e.memoizedState:null}else e=Se.next;var t=Ye===null?re.memoizedState:Ye.next;if(t!==null)Ye=t,Se=e;else{if(e===null)throw re.alternate===null?Error(U(467)):Error(U(310));Se=e,e={memoizedState:Se.memoizedState,baseState:Se.baseState,baseQueue:Se.baseQueue,queue:Se.queue,next:null},Ye===null?re.memoizedState=Ye=e:Ye=Ye.next=e}return Ye}function Kf(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function qo(e){var t=So;return So+=1,Ms===null&&(Ms=[]),e=eb(Ms,e,t),t=re,(Ye===null?t.memoizedState:Ye.next)===null&&(t=t.alternate,te.H=t===null||t.memoizedState===null?Mb:Ob),e}function nc(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return qo(e);if(e.$$typeof===rn)return $t(e)}throw Error(U(438,String(e)))}function Hf(e){var t=null,a=re.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=re.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=Kf(),re.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=qR;return t.index++,a}function fn(e,t){return typeof t=="function"?t(e):t}function fu(e){var t=Je();return Qf(t,Se,e)}function Qf(e,t,a){var n=e.queue;if(n===null)throw Error(U(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,l=null,c=t,d=!1;do{var m=c.lane&-536870913;if(m!==c.lane?(ce&m)===m:(Jn&m)===m){var f=c.revertLane;if(f===0)l!==null&&(l=l.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),m===Fs&&(d=!0);else if((Jn&f)===f){c=c.next,f===Fs&&(d=!0);continue}else m={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},l===null?(o=l=m,i=s):l=l.next=m,re.lanes|=f,Zn|=f;m=c.action,Rr&&a(s,m),s=c.hasEagerState?c.eagerState:a(s,m)}else f={lane:m,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},l===null?(o=l=f,i=s):l=l.next=f,re.lanes|=m,Zn|=m;c=c.next}while(c!==null&&c!==t);if(l===null?i=s:l.next=o,!Zt(s,e.memoizedState)&&(ut=!0,d&&(a=As,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=l,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function sm(e){var t=Je(),a=t.queue;if(a===null)throw Error(U(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);Zt(s,t.memoizedState)||(ut=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function sb(e,t,a){var n=re,r=Je(),s=pe;if(s){if(a===void 0)throw Error(U(407));a=a()}else a=t();var i=!Zt((Se||r).memoizedState,a);i&&(r.memoizedState=a,ut=!0),r=r.queue;var o=lb.bind(null,n,r,e);if(Io(2048,8,o,[e]),r.getSnapshot!==t||i||Ye!==null&&Ye.memoizedState.tag&1){if(n.flags|=2048,zs(9,rc(),ob.bind(null,n,r,a,t),null),_e===null)throw Error(U(349));s||(Jn&124)!==0||ib(n,t,a)}return a}function ib(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=re.updateQueue,t===null?(t=Kf(),re.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function ob(e,t,a,n){t.value=a,t.getSnapshot=n,ub(t)&&cb(e)}function lb(e,t,a){return a(function(){ub(t)&&cb(e)})}function ub(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!Zt(e,a)}catch{return!0}}function cb(e){var t=Ys(e,2);t!==null&&Xt(t,e,2)}function Vm(e){var t=Pt();if(typeof e=="function"){var a=e;if(e=a(),Rr){jn(!0);try{a()}finally{jn(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:fn,lastRenderedState:e},t}function db(e,t,a,n){return e.baseState=a,Qf(e,Se,typeof n=="function"?n:fn)}function sE(e,t,a,n,r){if(sc(e))throw Error(U(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};te.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,mb(t,s)):(s.next=a.next,t.pending=a.next=s)}}function mb(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=te.T,i={};te.T=i;try{var o=a(r,n),l=te.S;l!==null&&l(i,o),xg(e,t,o)}catch(c){Gm(e,t,c)}finally{te.T=s}}else try{s=a(r,n),xg(e,t,s)}catch(c){Gm(e,t,c)}}function xg(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){$g(e,t,n)},function(n){return Gm(e,t,n)}):$g(e,t,a)}function $g(e,t,a){t.status="fulfilled",t.value=a,fb(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,mb(e,a)))}function Gm(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,fb(t),t=t.next;while(t!==n)}e.action=null}function fb(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function pb(e,t){return t}function wg(e,t){if(pe){var a=_e.formState;if(a!==null){e:{var n=re;if(pe){if(Be){t:{for(var r=Be,s=Pa;r.nodeType!==8;){if(!s){r=null;break t}if(r=Sa(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){Be=Sa(r.nextSibling),n=r.data==="F!";break e}}_r(n)}n=!1}n&&(t=a[0])}}return a=Pt(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:pb,lastRenderedState:t},a.queue=n,a=Tb.bind(null,re,n),n.dispatch=a,n=Vm(!1),s=Jf.bind(null,re,!1,n.queue),n=Pt(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=sE.bind(null,re,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function Sg(e){var t=Je();return hb(t,Se,e)}function hb(e,t,a){if(t=Qf(e,t,pb)[0],e=fu(fn)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=qo(t)}catch(i){throw i===zo?ac:i}else n=t;t=Je();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(re.flags|=2048,zs(9,rc(),iE.bind(null,r,a),null)),[n,s,e]}function iE(e,t){e.action=t}function Ng(e){var t=Je(),a=Se;if(a!==null)return hb(t,a,e);Je(),t=t.memoizedState,a=Je();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function zs(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=re.updateQueue,t===null&&(t=Kf(),re.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function rc(){return{destroy:void 0,resource:void 0}}function vb(){return Je().memoizedState}function pu(e,t,a,n){var r=Pt();n=n===void 0?null:n,re.flags|=e,r.memoizedState=zs(1|t,rc(),a,n)}function Io(e,t,a,n){var r=Je();n=n===void 0?null:n;var s=r.memoizedState.inst;Se!==null&&n!==null&&Ff(n,Se.memoizedState.deps)?r.memoizedState=zs(t,s,a,n):(re.flags|=e,r.memoizedState=zs(1|t,s,a,n))}function _g(e,t){pu(8390656,8,e,t)}function gb(e,t){Io(2048,8,e,t)}function yb(e,t){return Io(4,2,e,t)}function bb(e,t){return Io(4,4,e,t)}function xb(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function $b(e,t,a){a=a!=null?a.concat([e]):null,Io(4,4,xb.bind(null,t,e),a)}function Vf(){}function wb(e,t){var a=Je();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&Ff(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function Sb(e,t){var a=Je();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&Ff(t,n[1]))return n[0];if(n=e(),Rr){jn(!0);try{e()}finally{jn(!1)}}return a.memoizedState=[n,t],n}function Gf(e,t,a){return a===void 0||(Jn&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=f0(),re.lanes|=e,Zn|=e,a)}function Nb(e,t,a,n){return Zt(a,t)?a:Bs.current!==null?(e=Gf(e,a,n),Zt(e,t)||(ut=!0),e):(Jn&42)===0?(ut=!0,e.memoizedState=a):(e=f0(),re.lanes|=e,Zn|=e,t)}function _b(e,t,a,n,r){var s=he.p;he.p=s!==0&&8>s?s:8;var i=te.T,o={};te.T=o,Jf(e,!1,t,a);try{var l=r(),c=te.S;if(c!==null&&c(o,l),l!==null&&typeof l=="object"&&typeof l.then=="function"){var d=aE(l,n);mo(e,t,d,Jt(e))}else mo(e,t,n,Jt(e))}catch(m){mo(e,t,{then:function(){},status:"rejected",reason:m},Jt())}finally{he.p=s,te.T=i}}function oE(){}function Ym(e,t,a,n){if(e.tag!==5)throw Error(U(476));var r=kb(e).queue;_b(e,r,t,yr,a===null?oE:function(){return Rb(e),a(n)})}function kb(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:yr,baseState:yr,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:fn,lastRenderedState:yr},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:fn,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function Rb(e){var t=kb(e).next.queue;mo(e,t,{},Jt())}function Yf(){return $t(Co)}function Cb(){return Je().memoizedState}function Eb(){return Je().memoizedState}function lE(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=Jt();e=In(a);var n=Kn(t,e,a);n!==null&&(Xt(n,t,a),lo(n,t,a)),t={cache:Lf()},e.payload=t;return}t=t.return}}function uE(e,t,a){var n=Jt();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},sc(e)?Ab(t,a):(a=Af(e,t,a,n),a!==null&&(Xt(a,e,n),Db(a,t,n)))}function Tb(e,t,a){var n=Jt();mo(e,t,a,n)}function mo(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(sc(e))Ab(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,Zt(o,i))return tc(e,t,r,0),_e===null&&ec(),!1}catch{}finally{}if(a=Af(e,t,r,n),a!==null)return Xt(a,e,n),Db(a,t,n),!0}return!1}function Jf(e,t,a,n){if(n={lane:2,revertLane:rp(),action:n,hasEagerState:!1,eagerState:null,next:null},sc(e)){if(t)throw Error(U(479))}else t=Af(e,a,n,2),t!==null&&Xt(t,e,2)}function sc(e){var t=e.alternate;return e===re||t!==null&&t===re}function Ab(e,t){Ds=Au=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function Db(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,wy(e,a)}}var Mu={readContext:$t,use:nc,useCallback:Ke,useContext:Ke,useEffect:Ke,useImperativeHandle:Ke,useLayoutEffect:Ke,useInsertionEffect:Ke,useMemo:Ke,useReducer:Ke,useRef:Ke,useState:Ke,useDebugValue:Ke,useDeferredValue:Ke,useTransition:Ke,useSyncExternalStore:Ke,useId:Ke,useHostTransitionStatus:Ke,useFormState:Ke,useActionState:Ke,useOptimistic:Ke,useMemoCache:Ke,useCacheRefresh:Ke},Mb={readContext:$t,use:nc,useCallback:function(e,t){return Pt().memoizedState=[e,t===void 0?null:t],e},useContext:$t,useEffect:_g,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,pu(4194308,4,xb.bind(null,t,e),a)},useLayoutEffect:function(e,t){return pu(4194308,4,e,t)},useInsertionEffect:function(e,t){pu(4,2,e,t)},useMemo:function(e,t){var a=Pt();t=t===void 0?null:t;var n=e();if(Rr){jn(!0);try{e()}finally{jn(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=Pt();if(a!==void 0){var r=a(t);if(Rr){jn(!0);try{a(t)}finally{jn(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=uE.bind(null,re,e),[n.memoizedState,e]},useRef:function(e){var t=Pt();return e={current:e},t.memoizedState=e},useState:function(e){e=Vm(e);var t=e.queue,a=Tb.bind(null,re,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:Vf,useDeferredValue:function(e,t){var a=Pt();return Gf(a,e,t)},useTransition:function(){var e=Vm(!1);return e=_b.bind(null,re,e.queue,!0,!1),Pt().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=re,r=Pt();if(pe){if(a===void 0)throw Error(U(407));a=a()}else{if(a=t(),_e===null)throw Error(U(349));(ce&124)!==0||ib(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,_g(lb.bind(null,n,s,e),[e]),n.flags|=2048,zs(9,rc(),ob.bind(null,n,s,a,t),null),a},useId:function(){var e=Pt(),t=_e.identifierPrefix;if(pe){var a=on,n=sn;a=(n&~(1<<32-Yt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=Du++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=nE++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:Yf,useFormState:wg,useActionState:wg,useOptimistic:function(e){var t=Pt();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=Jf.bind(null,re,!0,a),a.dispatch=t,[e,t]},useMemoCache:Hf,useCacheRefresh:function(){return Pt().memoizedState=lE.bind(null,re)}},Ob={readContext:$t,use:nc,useCallback:wb,useContext:$t,useEffect:gb,useImperativeHandle:$b,useInsertionEffect:yb,useLayoutEffect:bb,useMemo:Sb,useReducer:fu,useRef:vb,useState:function(){return fu(fn)},useDebugValue:Vf,useDeferredValue:function(e,t){var a=Je();return Nb(a,Se.memoizedState,e,t)},useTransition:function(){var e=fu(fn)[0],t=Je().memoizedState;return[typeof e=="boolean"?e:qo(e),t]},useSyncExternalStore:sb,useId:Cb,useHostTransitionStatus:Yf,useFormState:Sg,useActionState:Sg,useOptimistic:function(e,t){var a=Je();return db(a,Se,e,t)},useMemoCache:Hf,useCacheRefresh:Eb},cE={readContext:$t,use:nc,useCallback:wb,useContext:$t,useEffect:gb,useImperativeHandle:$b,useInsertionEffect:yb,useLayoutEffect:bb,useMemo:Sb,useReducer:sm,useRef:vb,useState:function(){return sm(fn)},useDebugValue:Vf,useDeferredValue:function(e,t){var a=Je();return Se===null?Gf(a,e,t):Nb(a,Se.memoizedState,e,t)},useTransition:function(){var e=sm(fn)[0],t=Je().memoizedState;return[typeof e=="boolean"?e:qo(e),t]},useSyncExternalStore:sb,useId:Cb,useHostTransitionStatus:Yf,useFormState:Ng,useActionState:Ng,useOptimistic:function(e,t){var a=Je();return Se!==null?db(a,Se,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:Hf,useCacheRefresh:Eb},Os=null,No=0;function Wl(e){var t=No;return No+=1,Os===null&&(Os=[]),eb(Os,e,t)}function Gi(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function eu(e,t){throw t.$$typeof===BR?Error(U(525)):(e=Object.prototype.toString.call(t),Error(U(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function kg(e){var t=e._init;return t(e._payload)}function Lb(e){function t(g,v){if(e){var b=g.deletions;b===null?(g.deletions=[v],g.flags|=16):b.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=cn(g,v),g.index=0,g.sibling=null,g}function s(g,v,b){return g.index=b,e?(b=g.alternate,b!==null?(b=b.index,b<v?(g.flags|=67108866,v):b):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,b,w){return v===null||v.tag!==6?(v=am(b,g.mode,w),v.return=g,v):(v=r(v,b),v.return=g,v)}function l(g,v,b,w){var N=b.type;return N===vs?d(g,v,b.props.children,w,b.key):v!==null&&(v.elementType===N||typeof N=="object"&&N!==null&&N.$$typeof===Dn&&kg(N)===v.type)?(v=r(v,b.props),Gi(v,b),v.return=g,v):(v=du(b.type,b.key,b.props,null,g.mode,w),Gi(v,b),v.return=g,v)}function c(g,v,b,w){return v===null||v.tag!==4||v.stateNode.containerInfo!==b.containerInfo||v.stateNode.implementation!==b.implementation?(v=nm(b,g.mode,w),v.return=g,v):(v=r(v,b.children||[]),v.return=g,v)}function d(g,v,b,w,N){return v===null||v.tag!==7?(v=br(b,g.mode,w,N),v.return=g,v):(v=r(v,b),v.return=g,v)}function m(g,v,b){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=am(""+v,g.mode,b),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case Kl:return b=du(v.type,v.key,v.props,null,g.mode,b),Gi(b,v),b.return=g,b;case Wi:return v=nm(v,g.mode,b),v.return=g,v;case Dn:var w=v._init;return v=w(v._payload),m(g,v,b)}if(eo(v)||Hi(v))return v=br(v,g.mode,b,null),v.return=g,v;if(typeof v.then=="function")return m(g,Wl(v),b);if(v.$$typeof===rn)return m(g,Xl(g,v),b);eu(g,v)}return null}function f(g,v,b,w){var N=v!==null?v.key:null;if(typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint")return N!==null?null:o(g,v,""+b,w);if(typeof b=="object"&&b!==null){switch(b.$$typeof){case Kl:return b.key===N?l(g,v,b,w):null;case Wi:return b.key===N?c(g,v,b,w):null;case Dn:return N=b._init,b=N(b._payload),f(g,v,b,w)}if(eo(b)||Hi(b))return N!==null?null:d(g,v,b,w,null);if(typeof b.then=="function")return f(g,v,Wl(b),w);if(b.$$typeof===rn)return f(g,v,Xl(g,b),w);eu(g,b)}return null}function h(g,v,b,w,N){if(typeof w=="string"&&w!==""||typeof w=="number"||typeof w=="bigint")return g=g.get(b)||null,o(v,g,""+w,N);if(typeof w=="object"&&w!==null){switch(w.$$typeof){case Kl:return g=g.get(w.key===null?b:w.key)||null,l(v,g,w,N);case Wi:return g=g.get(w.key===null?b:w.key)||null,c(v,g,w,N);case Dn:var C=w._init;return w=C(w._payload),h(g,v,b,w,N)}if(eo(w)||Hi(w))return g=g.get(b)||null,d(v,g,w,N,null);if(typeof w.then=="function")return h(g,v,b,Wl(w),N);if(w.$$typeof===rn)return h(g,v,b,Xl(v,w),N);eu(v,w)}return null}function x(g,v,b,w){for(var N=null,C=null,_=v,A=v=0,L=null;_!==null&&A<b.length;A++){_.index>A?(L=_,_=null):L=_.sibling;var M=f(g,_,b[A],w);if(M===null){_===null&&(_=L);break}e&&_&&M.alternate===null&&t(g,_),v=s(M,v,A),C===null?N=M:C.sibling=M,C=M,_=L}if(A===b.length)return a(g,_),pe&&vr(g,A),N;if(_===null){for(;A<b.length;A++)_=m(g,b[A],w),_!==null&&(v=s(_,v,A),C===null?N=_:C.sibling=_,C=_);return pe&&vr(g,A),N}for(_=n(_);A<b.length;A++)L=h(_,g,A,b[A],w),L!==null&&(e&&L.alternate!==null&&_.delete(L.key===null?A:L.key),v=s(L,v,A),C===null?N=L:C.sibling=L,C=L);return e&&_.forEach(function(P){return t(g,P)}),pe&&vr(g,A),N}function y(g,v,b,w){if(b==null)throw Error(U(151));for(var N=null,C=null,_=v,A=v=0,L=null,M=b.next();_!==null&&!M.done;A++,M=b.next()){_.index>A?(L=_,_=null):L=_.sibling;var P=f(g,_,M.value,w);if(P===null){_===null&&(_=L);break}e&&_&&P.alternate===null&&t(g,_),v=s(P,v,A),C===null?N=P:C.sibling=P,C=P,_=L}if(M.done)return a(g,_),pe&&vr(g,A),N;if(_===null){for(;!M.done;A++,M=b.next())M=m(g,M.value,w),M!==null&&(v=s(M,v,A),C===null?N=M:C.sibling=M,C=M);return pe&&vr(g,A),N}for(_=n(_);!M.done;A++,M=b.next())M=h(_,g,A,M.value,w),M!==null&&(e&&M.alternate!==null&&_.delete(M.key===null?A:M.key),v=s(M,v,A),C===null?N=M:C.sibling=M,C=M);return e&&_.forEach(function(k){return t(g,k)}),pe&&vr(g,A),N}function $(g,v,b,w){if(typeof b=="object"&&b!==null&&b.type===vs&&b.key===null&&(b=b.props.children),typeof b=="object"&&b!==null){switch(b.$$typeof){case Kl:e:{for(var N=b.key;v!==null;){if(v.key===N){if(N=b.type,N===vs){if(v.tag===7){a(g,v.sibling),w=r(v,b.props.children),w.return=g,g=w;break e}}else if(v.elementType===N||typeof N=="object"&&N!==null&&N.$$typeof===Dn&&kg(N)===v.type){a(g,v.sibling),w=r(v,b.props),Gi(w,b),w.return=g,g=w;break e}a(g,v);break}else t(g,v);v=v.sibling}b.type===vs?(w=br(b.props.children,g.mode,w,b.key),w.return=g,g=w):(w=du(b.type,b.key,b.props,null,g.mode,w),Gi(w,b),w.return=g,g=w)}return i(g);case Wi:e:{for(N=b.key;v!==null;){if(v.key===N)if(v.tag===4&&v.stateNode.containerInfo===b.containerInfo&&v.stateNode.implementation===b.implementation){a(g,v.sibling),w=r(v,b.children||[]),w.return=g,g=w;break e}else{a(g,v);break}else t(g,v);v=v.sibling}w=nm(b,g.mode,w),w.return=g,g=w}return i(g);case Dn:return N=b._init,b=N(b._payload),$(g,v,b,w)}if(eo(b))return x(g,v,b,w);if(Hi(b)){if(N=Hi(b),typeof N!="function")throw Error(U(150));return b=N.call(b),y(g,v,b,w)}if(typeof b.then=="function")return $(g,v,Wl(b),w);if(b.$$typeof===rn)return $(g,v,Xl(g,b),w);eu(g,b)}return typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint"?(b=""+b,v!==null&&v.tag===6?(a(g,v.sibling),w=r(v,b),w.return=g,g=w):(a(g,v),w=am(b,g.mode,w),w.return=g,g=w),i(g)):a(g,v)}return function(g,v,b,w){try{No=0;var N=$(g,v,b,w);return Os=null,N}catch(_){if(_===zo||_===ac)throw _;var C=Vt(29,_,null,g.mode);return C.lanes=w,C.return=g,C}finally{}}}var qs=Lb(!0),Pb=Lb(!1),ga=za(null),Ba=null;function Ln(e){var t=e.alternate;Oe(nt,nt.current&1),Oe(ga,e),Ba===null&&(t===null||Bs.current!==null||t.memoizedState!==null)&&(Ba=e)}function Ub(e){if(e.tag===22){if(Oe(nt,nt.current),Oe(ga,e),Ba===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(Ba=e)}}else Pn(e)}function Pn(){Oe(nt,nt.current),Oe(ga,ga.current)}function un(e){ct(ga),Ba===e&&(Ba=null),ct(nt)}var nt=za(0);function Ou(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||ff(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function im(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:Te({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var Jm={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=Jt(),r=In(n);r.payload=t,a!=null&&(r.callback=a),t=Kn(e,r,n),t!==null&&(Xt(t,e,n),lo(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=Jt(),r=In(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=Kn(e,r,n),t!==null&&(Xt(t,e,n),lo(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=Jt(),n=In(a);n.tag=2,t!=null&&(n.callback=t),t=Kn(e,n,a),t!==null&&(Xt(t,e,a),lo(t,e,a))}};function Rg(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!$o(a,n)||!$o(r,s):!0}function Cg(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&Jm.enqueueReplaceState(t,t.state,null)}function Cr(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=Te({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var Lu=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function jb(e){Lu(e)}function Fb(e){console.error(e)}function Bb(e){Lu(e)}function Pu(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function Eg(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function Xm(e,t,a){return a=In(a),a.tag=3,a.payload={element:null},a.callback=function(){Pu(e,t)},a}function zb(e){return e=In(e),e.tag=3,e}function qb(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){Eg(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){Eg(t,a,n),typeof r!="function"&&(Hn===null?Hn=new Set([this]):Hn.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function dE(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&Fo(t,a,r,!0),a=ga.current,a!==null){switch(a.tag){case 13:return Ba===null?sf():a.alternate===null&&ze===0&&(ze=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===Im?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),gm(e,n,r)),!1;case 22:return a.flags|=65536,n===Im?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),gm(e,n,r)),!1}throw Error(U(435,a.tag))}return gm(e,n,r),sf(),!1}if(pe)return t=ga.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==jm&&(e=Error(U(422),{cause:n}),wo(ha(e,a)))):(n!==jm&&(t=Error(U(423),{cause:n}),wo(ha(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=ha(n,a),r=Xm(e.stateNode,n,r),rm(e,r),ze!==4&&(ze=2)),!1;var s=Error(U(520),{cause:n});if(s=ha(s,a),ho===null?ho=[s]:ho.push(s),ze!==4&&(ze=2),t===null)return!0;n=ha(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=Xm(a.stateNode,n,e),rm(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(Hn===null||!Hn.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=zb(r),qb(r,e,a,n),rm(a,r),!1}a=a.return}while(a!==null);return!1}var Ib=Error(U(461)),ut=!1;function pt(e,t,a,n){t.child=e===null?Pb(t,null,a,n):qs(t,e.child,a,n)}function Tg(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return kr(t),n=Bf(e,t,a,i,s,r),o=zf(),e!==null&&!ut?(qf(e,t,r),pn(e,t,r)):(pe&&o&&Mf(t),t.flags|=1,pt(e,t,n,r),t.child)}function Ag(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!Df(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,Kb(e,t,s,n,r)):(e=du(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!Xf(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:$o,a(i,n)&&e.ref===t.ref)return pn(e,t,r)}return t.flags|=1,e=cn(s,n),e.ref=t.ref,e.return=t,t.child=e}function Kb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if($o(s,n)&&e.ref===t.ref)if(ut=!1,t.pendingProps=n=s,Xf(e,r))(e.flags&131072)!==0&&(ut=!0);else return t.lanes=e.lanes,pn(e,t,r)}return Zm(e,t,a,n,r)}function Hb(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return Dg(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&mu(t,s!==null?s.cachePool:null),s!==null?bg(t,s):Qm(),Ub(t);else return t.lanes=t.childLanes=536870912,Dg(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(mu(t,s.cachePool),bg(t,s),Pn(t),t.memoizedState=null):(e!==null&&mu(t,null),Qm(),Pn(t));return pt(e,t,r,a),t.child}function Dg(e,t,a,n){var r=Pf();return r=r===null?null:{parent:at._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&mu(t,null),Qm(),Ub(t),e!==null&&Fo(e,t,n,!0),null}function hu(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(U(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function Zm(e,t,a,n,r){return kr(t),a=Bf(e,t,a,n,void 0,r),n=zf(),e!==null&&!ut?(qf(e,t,r),pn(e,t,r)):(pe&&n&&Mf(t),t.flags|=1,pt(e,t,a,r),t.child)}function Mg(e,t,a,n,r,s){return kr(t),t.updateQueue=null,a=rb(t,n,a,r),nb(e),n=zf(),e!==null&&!ut?(qf(e,t,s),pn(e,t,s)):(pe&&n&&Mf(t),t.flags|=1,pt(e,t,a,s),t.child)}function Og(e,t,a,n,r){if(kr(t),t.stateNode===null){var s=Ns,i=a.contextType;typeof i=="object"&&i!==null&&(s=$t(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=Jm,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},Uf(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?$t(i):Ns,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(im(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&Jm.enqueueReplaceState(s,s.state,null),co(t,n,s,r),uo(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,l=Cr(a,o);s.props=l;var c=s.context,d=a.contextType;i=Ns,typeof d=="object"&&d!==null&&(i=$t(d));var m=a.getDerivedStateFromProps;d=typeof m=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&Cg(t,s,n,i),Mn=!1;var f=t.memoizedState;s.state=f,co(t,n,s,r),uo(),c=t.memoizedState,o||f!==c||Mn?(typeof m=="function"&&(im(t,a,m,n),c=t.memoizedState),(l=Mn||Rg(t,a,l,n,f,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=l):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,Km(e,t),i=t.memoizedProps,d=Cr(a,i),s.props=d,m=t.pendingProps,f=s.context,c=a.contextType,l=Ns,typeof c=="object"&&c!==null&&(l=$t(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==m||f!==l)&&Cg(t,s,n,l),Mn=!1,f=t.memoizedState,s.state=f,co(t,n,s,r),uo();var h=t.memoizedState;i!==m||f!==h||Mn||e!==null&&e.dependencies!==null&&Eu(e.dependencies)?(typeof o=="function"&&(im(t,a,o,n),h=t.memoizedState),(d=Mn||Rg(t,a,d,n,f,h,l)||e!==null&&e.dependencies!==null&&Eu(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,h,l),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,h,l)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=h),s.props=n,s.state=h,s.context=l,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,hu(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=qs(t,e.child,null,r),t.child=qs(t,null,a,r)):pt(e,t,a,r),t.memoizedState=s.state,e=t.child):e=pn(e,t,r),e}function Lg(e,t,a,n){return jo(),t.flags|=256,pt(e,t,a,n),t.child}var om={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function lm(e){return{baseLanes:e,cachePool:Zy()}}function um(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=va),e}function Qb(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(nt.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(pe){if(r?Ln(t):Pn(t),pe){var o=Be,l;if(l=o){e:{for(l=o,o=Pa;l.nodeType!==8;){if(!o){o=null;break e}if(l=Sa(l.nextSibling),l===null){o=null;break e}}o=l}o!==null?(t.memoizedState={dehydrated:o,treeContext:xr!==null?{id:sn,overflow:on}:null,retryLane:536870912,hydrationErrors:null},l=Vt(18,null,null,0),l.stateNode=o,l.return=t,t.child=l,kt=t,Be=null,l=!0):l=!1}l||_r(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return ff(o)?t.lanes=32:t.lanes=536870912,null;un(t)}return o=n.children,n=n.fallback,r?(Pn(t),r=t.mode,o=Uu({mode:"hidden",children:o},r),n=br(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=lm(a),r.childLanes=um(e,i,a),t.memoizedState=om,n):(Ln(t),Wm(t,o))}if(l=e.memoizedState,l!==null&&(o=l.dehydrated,o!==null)){if(s)t.flags&256?(Ln(t),t.flags&=-257,t=cm(e,t,a)):t.memoizedState!==null?(Pn(t),t.child=e.child,t.flags|=128,t=null):(Pn(t),r=n.fallback,o=t.mode,n=Uu({mode:"visible",children:n.children},o),r=br(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,qs(t,e.child,null,a),n=t.child,n.memoizedState=lm(a),n.childLanes=um(e,i,a),t.memoizedState=om,t=r);else if(Ln(t),ff(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(U(419)),n.stack="",n.digest=i,wo({value:n,source:null,stack:null}),t=cm(e,t,a)}else if(ut||Fo(e,t,a,!1),i=(a&e.childLanes)!==0,ut||i){if(i=_e,i!==null&&(n=a&-a,n=(n&42)!==0?1:$f(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==l.retryLane))throw l.retryLane=n,Ys(e,n),Xt(i,e,n),Ib;o.data==="$?"||sf(),t=cm(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=l.treeContext,Be=Sa(o.nextSibling),kt=t,pe=!0,$r=null,Pa=!1,e!==null&&(ma[fa++]=sn,ma[fa++]=on,ma[fa++]=xr,sn=e.id,on=e.overflow,xr=t),t=Wm(t,n.children),t.flags|=4096);return t}return r?(Pn(t),r=n.fallback,o=t.mode,l=e.child,c=l.sibling,n=cn(l,{mode:"hidden",children:n.children}),n.subtreeFlags=l.subtreeFlags&65011712,c!==null?r=cn(c,r):(r=br(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=lm(a):(l=o.cachePool,l!==null?(c=at._currentValue,l=l.parent!==c?{parent:c,pool:c}:l):l=Zy(),o={baseLanes:o.baseLanes|a,cachePool:l}),r.memoizedState=o,r.childLanes=um(e,i,a),t.memoizedState=om,n):(Ln(t),a=e.child,e=a.sibling,a=cn(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function Wm(e,t){return t=Uu({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function Uu(e,t){return e=Vt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function cm(e,t,a){return qs(t,e.child,null,a),e=Wm(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function Pg(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),Bm(e.return,t,a)}function dm(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function Vb(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(pt(e,t,n.children,a),n=nt.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&Pg(e,a,t);else if(e.tag===19)Pg(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Oe(nt,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&Ou(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),dm(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&Ou(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}dm(t,!0,a,null,s);break;case"together":dm(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function pn(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),Zn|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(Fo(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(U(153));if(t.child!==null){for(e=t.child,a=cn(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=cn(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function Xf(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&Eu(e)))}function mE(e,t,a){switch(t.tag){case 3:$u(t,t.stateNode.containerInfo),On(t,at,e.memoizedState.cache),jo();break;case 27:case 5:Cm(t);break;case 4:$u(t,t.stateNode.containerInfo);break;case 10:On(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(Ln(t),t.flags|=128,null):(a&t.child.childLanes)!==0?Qb(e,t,a):(Ln(t),e=pn(e,t,a),e!==null?e.sibling:null);Ln(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(Fo(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return Vb(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Oe(nt,nt.current),n)break;return null;case 22:case 23:return t.lanes=0,Hb(e,t,a);case 24:On(t,at,e.memoizedState.cache)}return pn(e,t,a)}function Gb(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)ut=!0;else{if(!Xf(e,a)&&(t.flags&128)===0)return ut=!1,mE(e,t,a);ut=(e.flags&131072)!==0}else ut=!1,pe&&(t.flags&1048576)!==0&&Jy(t,Cu,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")Df(n)?(e=Cr(n,e),t.tag=1,t=Og(null,t,n,e,a)):(t.tag=0,t=Zm(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===yf){t.tag=11,t=Tg(null,t,n,e,a);break e}else if(r===bf){t.tag=14,t=Ag(null,t,n,e,a);break e}}throw t=km(n)||n,Error(U(306,t,""))}}return t;case 0:return Zm(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Cr(n,t.pendingProps),Og(e,t,n,r,a);case 3:e:{if($u(t,t.stateNode.containerInfo),e===null)throw Error(U(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,Km(e,t),co(t,n,null,a);var i=t.memoizedState;if(n=i.cache,On(t,at,n),n!==s.cache&&zm(t,[at],a,!0),uo(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=Lg(e,t,n,a);break e}else if(n!==r){r=ha(Error(U(424)),t),wo(r),t=Lg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(Be=Sa(e.firstChild),kt=t,pe=!0,$r=null,Pa=!0,a=Pb(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if(jo(),n===r){t=pn(e,t,a);break e}pt(e,t,n,a)}t=t.child}return t;case 26:return hu(e,t),e===null?(a=ty(t.type,null,t.pendingProps,null))?t.memoizedState=a:pe||(a=t.type,e=t.pendingProps,n=Ku(qn.current).createElement(a),n[xt]=t,n[Ft]=e,vt(n,a,e),lt(n),t.stateNode=n):t.memoizedState=ty(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return Cm(t),e===null&&pe&&(n=t.stateNode=O0(t.type,t.pendingProps,qn.current),kt=t,Pa=!0,r=Be,er(t.type)?(pf=r,Be=Sa(n.firstChild)):Be=r),pt(e,t,t.pendingProps.children,a),hu(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&pe&&((r=n=Be)&&(n=jE(n,t.type,t.pendingProps,Pa),n!==null?(t.stateNode=n,kt=t,Be=Sa(n.firstChild),Pa=!1,r=!0):r=!1),r||_r(t)),Cm(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,df(r,s)?n=null:i!==null&&df(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=Bf(e,t,rE,null,null,a),Co._currentValue=r),hu(e,t),pt(e,t,n,a),t.child;case 6:return e===null&&pe&&((e=a=Be)&&(a=FE(a,t.pendingProps,Pa),a!==null?(t.stateNode=a,kt=t,Be=null,e=!0):e=!1),e||_r(t)),null;case 13:return Qb(e,t,a);case 4:return $u(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=qs(t,null,n,a):pt(e,t,n,a),t.child;case 11:return Tg(e,t,t.type,t.pendingProps,a);case 7:return pt(e,t,t.pendingProps,a),t.child;case 8:return pt(e,t,t.pendingProps.children,a),t.child;case 12:return pt(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,On(t,t.type,n.value),pt(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,kr(t),r=$t(r),n=n(r),t.flags|=1,pt(e,t,n,a),t.child;case 14:return Ag(e,t,t.type,t.pendingProps,a);case 15:return Kb(e,t,t.type,t.pendingProps,a);case 19:return Vb(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=Uu(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=cn(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return Hb(e,t,a);case 24:return kr(t),n=$t(at),e===null?(r=Pf(),r===null&&(r=_e,s=Lf(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},Uf(t),On(t,at,r)):((e.lanes&a)!==0&&(Km(e,t),co(t,null,null,a),uo()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),On(t,at,n)):(n=s.cache,On(t,at,n),n!==r.cache&&zm(t,[at],a,!0))),pt(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(U(156,t.tag))}function tn(e){e.flags|=4}function Ug(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!U0(t)){if(t=ga.current,t!==null&&((ce&4194048)===ce?Ba!==null:(ce&62914560)!==ce&&(ce&536870912)===0||t!==Ba))throw oo=Im,Wy;e.flags|=8192}}function tu(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?xy():536870912,e.lanes|=t,Is|=t)}function Yi(e,t){if(!pe)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function je(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function fE(e,t,a){var n=t.pendingProps;switch(Of(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return je(t),null;case 1:return je(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),dn(at),Ps(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Vi(t)?tn(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,fg())),je(t),null;case 26:return a=t.memoizedState,e===null?(tn(t),a!==null?(je(t),Ug(t,a)):(je(t),t.flags&=-16777217)):a?a!==e.memoizedState?(tn(t),je(t),Ug(t,a)):(je(t),t.flags&=-16777217):(e.memoizedProps!==n&&tn(t),je(t),t.flags&=-16777217),null;case 27:wu(t),a=qn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&tn(t);else{if(!n){if(t.stateNode===null)throw Error(U(166));return je(t),null}e=ja.current,Vi(t)?dg(t,e):(e=O0(r,n,a),t.stateNode=e,tn(t))}return je(t),null;case 5:if(wu(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&tn(t);else{if(!n){if(t.stateNode===null)throw Error(U(166));return je(t),null}if(e=ja.current,Vi(t))dg(t,e);else{switch(r=Ku(qn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[xt]=t,e[Ft]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(vt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&tn(t)}}return je(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&tn(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(U(166));if(e=qn.current,Vi(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=kt,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[xt]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||A0(e.nodeValue,a)),e||_r(t)}else e=Ku(e).createTextNode(n),e[xt]=t,t.stateNode=e}return je(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Vi(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(U(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(U(317));r[xt]=t}else jo(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;je(t),r=!1}else r=fg(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(un(t),t):(un(t),null)}if(un(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),tu(t,t.updateQueue),je(t),null;case 4:return Ps(),e===null&&sp(t.stateNode.containerInfo),je(t),null;case 10:return dn(t.type),je(t),null;case 19:if(ct(nt),r=t.memoizedState,r===null)return je(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)Yi(r,!1);else{if(ze!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=Ou(e),s!==null){for(t.flags|=128,Yi(r,!1),e=s.updateQueue,t.updateQueue=e,tu(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)Yy(a,e),a=a.sibling;return Oe(nt,nt.current&1|2),t.child}e=e.sibling}r.tail!==null&&Fa()>Fu&&(t.flags|=128,n=!0,Yi(r,!1),t.lanes=4194304)}else{if(!n)if(e=Ou(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,tu(t,e),Yi(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!pe)return je(t),null}else 2*Fa()-r.renderingStartTime>Fu&&a!==536870912&&(t.flags|=128,n=!0,Yi(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=Fa(),t.sibling=null,e=nt.current,Oe(nt,n?e&1|2:e&1),t):(je(t),null);case 22:case 23:return un(t),jf(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(je(t),t.subtreeFlags&6&&(t.flags|=8192)):je(t),a=t.updateQueue,a!==null&&tu(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&ct(wr),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),dn(at),je(t),null;case 25:return null;case 30:return null}throw Error(U(156,t.tag))}function pE(e,t){switch(Of(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return dn(at),Ps(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return wu(t),null;case 13:if(un(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(U(340));jo()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return ct(nt),null;case 4:return Ps(),null;case 10:return dn(t.type),null;case 22:case 23:return un(t),jf(),e!==null&&ct(wr),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return dn(at),null;case 25:return null;default:return null}}function Yb(e,t){switch(Of(t),t.tag){case 3:dn(at),Ps();break;case 26:case 27:case 5:wu(t);break;case 4:Ps();break;case 13:un(t);break;case 19:ct(nt);break;case 10:dn(t.type);break;case 22:case 23:un(t),jf(),e!==null&&ct(wr);break;case 24:dn(at)}}function Ko(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){Ne(t,t.return,o)}}function Xn(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var l=a,c=o;try{c()}catch(d){Ne(r,l,d)}}}n=n.next}while(n!==s)}}catch(d){Ne(t,t.return,d)}}function Jb(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{ab(t,a)}catch(n){Ne(e,e.return,n)}}}function Xb(e,t,a){a.props=Cr(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){Ne(e,t,n)}}function fo(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){Ne(e,t,r)}}function Ua(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){Ne(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){Ne(e,t,r)}else a.current=null}function Zb(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){Ne(e,e.return,r)}}function mm(e,t,a){try{var n=e.stateNode;ME(n,e.type,a,t),n[Ft]=t}catch(r){Ne(e,e.return,r)}}function Wb(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&er(e.type)||e.tag===4}function fm(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||Wb(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&er(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function ef(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=uc));else if(n!==4&&(n===27&&er(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(ef(e,t,a),e=e.sibling;e!==null;)ef(e,t,a),e=e.sibling}function ju(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&er(e.type)&&(a=e.stateNode),e=e.child,e!==null))for(ju(e,t,a),e=e.sibling;e!==null;)ju(e,t,a),e=e.sibling}function e0(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);vt(t,n,a),t[xt]=e,t[Ft]=a}catch(s){Ne(e,e.return,s)}}var nn=!1,He=!1,pm=!1,jg=typeof WeakSet=="function"?WeakSet:Set,ot=null;function hE(e,t){if(e=e.containerInfo,uf=Gu,e=zy(e),Ef(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,l=-1,c=0,d=0,m=e,f=null;t:for(;;){for(var h;m!==a||r!==0&&m.nodeType!==3||(o=i+r),m!==s||n!==0&&m.nodeType!==3||(l=i+n),m.nodeType===3&&(i+=m.nodeValue.length),(h=m.firstChild)!==null;)f=m,m=h;for(;;){if(m===e)break t;if(f===a&&++c===r&&(o=i),f===s&&++d===n&&(l=i),(h=m.nextSibling)!==null)break;m=f,f=m.parentNode}m=h}a=o===-1||l===-1?null:{start:o,end:l}}else a=null}a=a||{start:0,end:0}}else a=null;for(cf={focusedElem:e,selectionRange:a},Gu=!1,ot=t;ot!==null;)if(t=ot,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,ot=e;else for(;ot!==null;){switch(t=ot,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var x=Cr(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(x,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(y){Ne(a,a.return,y)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)mf(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":mf(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(U(163))}if(e=t.sibling,e!==null){e.return=t.return,ot=e;break}ot=t.return}}function t0(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:Tn(e,a),n&4&&Ko(5,a);break;case 1:if(Tn(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){Ne(a,a.return,i)}else{var r=Cr(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){Ne(a,a.return,i)}}n&64&&Jb(a),n&512&&fo(a,a.return);break;case 3:if(Tn(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{ab(e,t)}catch(i){Ne(a,a.return,i)}}break;case 27:t===null&&n&4&&e0(a);case 26:case 5:Tn(e,a),t===null&&n&4&&Zb(a),n&512&&fo(a,a.return);break;case 12:Tn(e,a);break;case 13:Tn(e,a),n&4&&r0(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=NE.bind(null,a),BE(e,a))));break;case 22:if(n=a.memoizedState!==null||nn,!n){t=t!==null&&t.memoizedState!==null||He,r=nn;var s=He;nn=n,(He=t)&&!s?An(e,a,(a.subtreeFlags&8772)!==0):Tn(e,a),nn=r,He=s}break;case 30:break;default:Tn(e,a)}}function a0(e){var t=e.alternate;t!==null&&(e.alternate=null,a0(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&Sf(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var Me=null,Ut=!1;function an(e,t,a){for(a=a.child;a!==null;)n0(e,t,a),a=a.sibling}function n0(e,t,a){if(Gt&&typeof Gt.onCommitFiberUnmount=="function")try{Gt.onCommitFiberUnmount(Mo,a)}catch{}switch(a.tag){case 26:He||Ua(a,t),an(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:He||Ua(a,t);var n=Me,r=Ut;er(a.type)&&(Me=a.stateNode,Ut=!1),an(e,t,a),go(a.stateNode),Me=n,Ut=r;break;case 5:He||Ua(a,t);case 6:if(n=Me,r=Ut,Me=null,an(e,t,a),Me=n,Ut=r,Me!==null)if(Ut)try{(Me.nodeType===9?Me.body:Me.nodeName==="HTML"?Me.ownerDocument.body:Me).removeChild(a.stateNode)}catch(s){Ne(a,t,s)}else try{Me.removeChild(a.stateNode)}catch(s){Ne(a,t,s)}break;case 18:Me!==null&&(Ut?(e=Me,Zg(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),Ao(e)):Zg(Me,a.stateNode));break;case 4:n=Me,r=Ut,Me=a.stateNode.containerInfo,Ut=!0,an(e,t,a),Me=n,Ut=r;break;case 0:case 11:case 14:case 15:He||Xn(2,a,t),He||Xn(4,a,t),an(e,t,a);break;case 1:He||(Ua(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&Xb(a,t,n)),an(e,t,a);break;case 21:an(e,t,a);break;case 22:He=(n=He)||a.memoizedState!==null,an(e,t,a),He=n;break;default:an(e,t,a)}}function r0(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{Ao(e)}catch(a){Ne(t,t.return,a)}}function vE(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new jg),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new jg),t;default:throw Error(U(435,e.tag))}}function hm(e,t){var a=vE(e);t.forEach(function(n){var r=_E.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function Kt(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(er(o.type)){Me=o.stateNode,Ut=!1;break e}break;case 5:Me=o.stateNode,Ut=!1;break e;case 3:case 4:Me=o.stateNode.containerInfo,Ut=!0;break e}o=o.return}if(Me===null)throw Error(U(160));n0(s,i,r),Me=null,Ut=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)s0(t,e),t=t.sibling}var wa=null;function s0(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:Kt(t,e),Ht(e),n&4&&(Xn(3,e,e.return),Ko(3,e),Xn(5,e,e.return));break;case 1:Kt(t,e),Ht(e),n&512&&(He||a===null||Ua(a,a.return)),n&64&&nn&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=wa;if(Kt(t,e),Ht(e),n&512&&(He||a===null||Ua(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[Po]||s[xt]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),vt(s,n,a),s[xt]=e,lt(s),n=s;break e;case"link":var i=ny("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),vt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=ny("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),vt(s,n,a),r.head.appendChild(s);break;default:throw Error(U(468,n))}s[xt]=e,lt(s),n=s}e.stateNode=n}else ry(r,e.type,e.stateNode);else e.stateNode=ay(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?ry(r,e.type,e.stateNode):ay(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&mm(e,e.memoizedProps,a.memoizedProps)}break;case 27:Kt(t,e),Ht(e),n&512&&(He||a===null||Ua(a,a.return)),a!==null&&n&4&&mm(e,e.memoizedProps,a.memoizedProps);break;case 5:if(Kt(t,e),Ht(e),n&512&&(He||a===null||Ua(a,a.return)),e.flags&32){r=e.stateNode;try{js(r,"")}catch(h){Ne(e,e.return,h)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,mm(e,r,a!==null?a.memoizedProps:r)),n&1024&&(pm=!0);break;case 6:if(Kt(t,e),Ht(e),n&4){if(e.stateNode===null)throw Error(U(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(h){Ne(e,e.return,h)}}break;case 3:if(yu=null,r=wa,wa=Hu(t.containerInfo),Kt(t,e),wa=r,Ht(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{Ao(t.containerInfo)}catch(h){Ne(e,e.return,h)}pm&&(pm=!1,i0(e));break;case 4:n=wa,wa=Hu(e.stateNode.containerInfo),Kt(t,e),Ht(e),wa=n;break;case 12:Kt(t,e),Ht(e);break;case 13:Kt(t,e),Ht(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(ap=Fa()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,hm(e,n)));break;case 22:r=e.memoizedState!==null;var l=a!==null&&a.memoizedState!==null,c=nn,d=He;if(nn=c||r,He=d||l,Kt(t,e),He=d,nn=c,Ht(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||l||nn||He||gr(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){l=a=t;try{if(s=l.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=l.stateNode;var m=l.memoizedProps.style,f=m!=null&&m.hasOwnProperty("display")?m.display:null;o.style.display=f==null||typeof f=="boolean"?"":(""+f).trim()}}catch(h){Ne(l,l.return,h)}}}else if(t.tag===6){if(a===null){l=t;try{l.stateNode.nodeValue=r?"":l.memoizedProps}catch(h){Ne(l,l.return,h)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,hm(e,a))));break;case 19:Kt(t,e),Ht(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,hm(e,n)));break;case 30:break;case 21:break;default:Kt(t,e),Ht(e)}}function Ht(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(Wb(n)){a=n;break}n=n.return}if(a==null)throw Error(U(160));switch(a.tag){case 27:var r=a.stateNode,s=fm(e);ju(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(js(i,""),a.flags&=-33);var o=fm(e);ju(e,o,i);break;case 3:case 4:var l=a.stateNode.containerInfo,c=fm(e);ef(e,c,l);break;default:throw Error(U(161))}}catch(d){Ne(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function i0(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;i0(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function Tn(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)t0(e,t.alternate,t),t=t.sibling}function gr(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:Xn(4,t,t.return),gr(t);break;case 1:Ua(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&Xb(t,t.return,a),gr(t);break;case 27:go(t.stateNode);case 26:case 5:Ua(t,t.return),gr(t);break;case 22:t.memoizedState===null&&gr(t);break;case 30:gr(t);break;default:gr(t)}e=e.sibling}}function An(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:An(r,s,a),Ko(4,s);break;case 1:if(An(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){Ne(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var l=r.shared.hiddenCallbacks;if(l!==null)for(r.shared.hiddenCallbacks=null,r=0;r<l.length;r++)tb(l[r],o)}catch(c){Ne(n,n.return,c)}}a&&i&64&&Jb(s),fo(s,s.return);break;case 27:e0(s);case 26:case 5:An(r,s,a),a&&n===null&&i&4&&Zb(s),fo(s,s.return);break;case 12:An(r,s,a);break;case 13:An(r,s,a),a&&i&4&&r0(r,s);break;case 22:s.memoizedState===null&&An(r,s,a),fo(s,s.return);break;case 30:break;default:An(r,s,a)}t=t.sibling}}function Zf(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&Bo(a))}function Wf(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&Bo(e))}function La(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)o0(e,t,a,n),t=t.sibling}function o0(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:La(e,t,a,n),r&2048&&Ko(9,t);break;case 1:La(e,t,a,n);break;case 3:La(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&Bo(e)));break;case 12:if(r&2048){La(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(l){Ne(t,t.return,l)}}else La(e,t,a,n);break;case 13:La(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?La(e,t,a,n):po(e,t):s._visibility&2?La(e,t,a,n):(s._visibility|=2,ps(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&Zf(i,t);break;case 24:La(e,t,a,n),r&2048&&Wf(t.alternate,t);break;default:La(e,t,a,n)}}function ps(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,l=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:ps(s,i,o,l,r),Ko(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?ps(s,i,o,l,r):po(s,i):(d._visibility|=2,ps(s,i,o,l,r)),r&&c&2048&&Zf(i.alternate,i);break;case 24:ps(s,i,o,l,r),r&&c&2048&&Wf(i.alternate,i);break;default:ps(s,i,o,l,r)}t=t.sibling}}function po(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:po(a,n),r&2048&&Zf(n.alternate,n);break;case 24:po(a,n),r&2048&&Wf(n.alternate,n);break;default:po(a,n)}t=t.sibling}}var ao=8192;function ds(e){if(e.subtreeFlags&ao)for(e=e.child;e!==null;)l0(e),e=e.sibling}function l0(e){switch(e.tag){case 26:ds(e),e.flags&ao&&e.memoizedState!==null&&WE(wa,e.memoizedState,e.memoizedProps);break;case 5:ds(e);break;case 3:case 4:var t=wa;wa=Hu(e.stateNode.containerInfo),ds(e),wa=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=ao,ao=16777216,ds(e),ao=t):ds(e));break;default:ds(e)}}function u0(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function Ji(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ot=n,d0(n,e)}u0(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)c0(e),e=e.sibling}function c0(e){switch(e.tag){case 0:case 11:case 15:Ji(e),e.flags&2048&&Xn(9,e,e.return);break;case 3:Ji(e);break;case 12:Ji(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,vu(e)):Ji(e);break;default:Ji(e)}}function vu(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ot=n,d0(n,e)}u0(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:Xn(8,t,t.return),vu(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,vu(t));break;default:vu(t)}e=e.sibling}}function d0(e,t){for(;ot!==null;){var a=ot;switch(a.tag){case 0:case 11:case 15:Xn(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:Bo(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,ot=n;else e:for(a=e;ot!==null;){n=ot;var r=n.sibling,s=n.return;if(a0(n),n===a){ot=null;break e}if(r!==null){r.return=s,ot=r;break e}ot=s}}}var gE={getCacheForType:function(e){var t=$t(at),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},yE=typeof WeakMap=="function"?WeakMap:Map,xe=0,_e=null,le=null,ce=0,be=0,Qt=null,Bn=!1,Js=!1,ep=!1,hn=0,ze=0,Zn=0,Sr=0,tp=0,va=0,Is=0,ho=null,jt=null,tf=!1,ap=0,Fu=1/0,Bu=null,Hn=null,ht=0,Qn=null,Ks=null,Ls=0,af=0,nf=null,m0=null,vo=0,rf=null;function Jt(){if((xe&2)!==0&&ce!==0)return ce&-ce;if(te.T!==null){var e=Fs;return e!==0?e:rp()}return Sy()}function f0(){va===0&&(va=(ce&536870912)===0||pe?by():536870912);var e=ga.current;return e!==null&&(e.flags|=32),va}function Xt(e,t,a){(e===_e&&(be===2||be===9)||e.cancelPendingCommit!==null)&&(Hs(e,0),zn(e,ce,va,!1)),Lo(e,a),((xe&2)===0||e!==_e)&&(e===_e&&((xe&2)===0&&(Sr|=a),ze===4&&zn(e,ce,va,!1)),qa(e))}function p0(e,t,a){if((xe&6)!==0)throw Error(U(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||Oo(e,t),r=n?$E(e,t):vm(e,t,!0),s=n;do{if(r===0){Js&&!n&&zn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!bE(a)){r=vm(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=ho;var l=o.current.memoizedState.isDehydrated;if(l&&(Hs(o,i).flags|=256),i=vm(o,i,!1),i!==2){if(ep&&!l){o.errorRecoveryDisabledLanes|=s,Sr|=s,r=4;break e}s=jt,jt=r,s!==null&&(jt===null?jt=s:jt.push.apply(jt,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){Hs(e,0),zn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(U(345));case 4:if((t&4194048)!==t)break;case 6:zn(n,t,va,!Bn);break e;case 2:jt=null;break;case 3:case 5:break;default:throw Error(U(329))}if((t&62914560)===t&&(r=ap+300-Fa(),10<r)){if(zn(n,t,va,!Bn),Ju(n,0,!0)!==0)break e;n.timeoutHandle=M0(Fg.bind(null,n,a,jt,Bu,tf,t,va,Sr,Is,Bn,s,2,-0,0),r);break e}Fg(n,a,jt,Bu,tf,t,va,Sr,Is,Bn,s,0,-0,0)}}break}while(!0);qa(e)}function Fg(e,t,a,n,r,s,i,o,l,c,d,m,f,h){if(e.timeoutHandle=-1,m=t.subtreeFlags,(m&8192||(m&16785408)===16785408)&&(Ro={stylesheets:null,count:0,unsuspend:ZE},l0(t),m=e3(),m!==null)){e.cancelPendingCommit=m(zg.bind(null,e,t,s,a,n,r,i,o,l,d,1,f,h)),zn(e,s,i,!c);return}zg(e,t,s,a,n,r,i,o,l)}function bE(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!Zt(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function zn(e,t,a,n){t&=~tp,t&=~Sr,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Yt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&$y(e,a,t)}function ic(){return(xe&6)===0?(Ho(0,!1),!1):!0}function np(){if(le!==null){if(be===0)var e=le.return;else e=le,ln=Dr=null,If(e),Os=null,No=0,e=le;for(;e!==null;)Yb(e.alternate,e),e=e.return;le=null}}function Hs(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,LE(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),np(),_e=e,le=a=cn(e.current,null),ce=t,be=0,Qt=null,Bn=!1,Js=Oo(e,t),ep=!1,Is=va=tp=Sr=Zn=ze=0,jt=ho=null,tf=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Yt(n),s=1<<r;t|=e[r],n&=~s}return hn=t,ec(),a}function h0(e,t){re=null,te.H=Mu,t===zo||t===ac?(t=gg(),be=3):t===Wy?(t=gg(),be=4):be=t===Ib?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Qt=t,le===null&&(ze=1,Pu(e,ha(t,e.current)))}function v0(){var e=te.H;return te.H=Mu,e===null?Mu:e}function g0(){var e=te.A;return te.A=gE,e}function sf(){ze=4,Bn||(ce&4194048)!==ce&&ga.current!==null||(Js=!0),(Zn&134217727)===0&&(Sr&134217727)===0||_e===null||zn(_e,ce,va,!1)}function vm(e,t,a){var n=xe;xe|=2;var r=v0(),s=g0();(_e!==e||ce!==t)&&(Bu=null,Hs(e,t)),t=!1;var i=ze;e:do try{if(be!==0&&le!==null){var o=le,l=Qt;switch(be){case 8:np(),i=6;break e;case 3:case 2:case 9:case 6:ga.current===null&&(t=!0);var c=be;if(be=0,Qt=null,Rs(e,o,l,c),a&&Js){i=0;break e}break;default:c=be,be=0,Qt=null,Rs(e,o,l,c)}}xE(),i=ze;break}catch(d){h0(e,d)}while(!0);return t&&e.shellSuspendCounter++,ln=Dr=null,xe=n,te.H=r,te.A=s,le===null&&(_e=null,ce=0,ec()),i}function xE(){for(;le!==null;)y0(le)}function $E(e,t){var a=xe;xe|=2;var n=v0(),r=g0();_e!==e||ce!==t?(Bu=null,Fu=Fa()+500,Hs(e,t)):Js=Oo(e,t);e:do try{if(be!==0&&le!==null){t=le;var s=Qt;t:switch(be){case 1:be=0,Qt=null,Rs(e,t,s,1);break;case 2:case 9:if(vg(s)){be=0,Qt=null,Bg(t);break}t=function(){be!==2&&be!==9||_e!==e||(be=7),qa(e)},s.then(t,t);break e;case 3:be=7;break e;case 4:be=5;break e;case 7:vg(s)?(be=0,Qt=null,Bg(t)):(be=0,Qt=null,Rs(e,t,s,7));break;case 5:var i=null;switch(le.tag){case 26:i=le.memoizedState;case 5:case 27:var o=le;if(!i||U0(i)){be=0,Qt=null;var l=o.sibling;if(l!==null)le=l;else{var c=o.return;c!==null?(le=c,oc(c)):le=null}break t}}be=0,Qt=null,Rs(e,t,s,5);break;case 6:be=0,Qt=null,Rs(e,t,s,6);break;case 8:np(),ze=6;break e;default:throw Error(U(462))}}wE();break}catch(d){h0(e,d)}while(!0);return ln=Dr=null,te.H=n,te.A=r,xe=a,le!==null?0:(_e=null,ce=0,ec(),ze)}function wE(){for(;le!==null&&!KR();)y0(le)}function y0(e){var t=Gb(e.alternate,e,hn);e.memoizedProps=e.pendingProps,t===null?oc(e):le=t}function Bg(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=Mg(a,t,t.pendingProps,t.type,void 0,ce);break;case 11:t=Mg(a,t,t.pendingProps,t.type.render,t.ref,ce);break;case 5:If(t);default:Yb(a,t),t=le=Yy(t,hn),t=Gb(a,t,hn)}e.memoizedProps=e.pendingProps,t===null?oc(e):le=t}function Rs(e,t,a,n){ln=Dr=null,If(t),Os=null,No=0;var r=t.return;try{if(dE(e,r,t,a,ce)){ze=1,Pu(e,ha(a,e.current)),le=null;return}}catch(s){if(r!==null)throw le=r,s;ze=1,Pu(e,ha(a,e.current)),le=null;return}t.flags&32768?(pe||n===1?e=!0:Js||(ce&536870912)!==0?e=!1:(Bn=e=!0,(n===2||n===9||n===3||n===6)&&(n=ga.current,n!==null&&n.tag===13&&(n.flags|=16384))),b0(t,e)):oc(t)}function oc(e){var t=e;do{if((t.flags&32768)!==0){b0(t,Bn);return}e=t.return;var a=fE(t.alternate,t,hn);if(a!==null){le=a;return}if(t=t.sibling,t!==null){le=t;return}le=t=e}while(t!==null);ze===0&&(ze=5)}function b0(e,t){do{var a=pE(e.alternate,e);if(a!==null){a.flags&=32767,le=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){le=e;return}le=e=a}while(e!==null);ze=6,le=null}function zg(e,t,a,n,r,s,i,o,l){e.cancelPendingCommit=null;do lc();while(ht!==0);if((xe&6)!==0)throw Error(U(327));if(t!==null){if(t===e.current)throw Error(U(177));if(s=t.lanes|t.childLanes,s|=Tf,eC(e,a,s,i,o,l),e===_e&&(le=_e=null,ce=0),Ks=t,Qn=e,Ls=a,af=s,nf=r,m0=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,kE(Su,function(){return N0(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=te.T,te.T=null,r=he.p,he.p=2,i=xe,xe|=4;try{hE(e,t,a)}finally{xe=i,he.p=r,te.T=n}}ht=1,x0(),$0(),w0()}}function x0(){if(ht===1){ht=0;var e=Qn,t=Ks,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=te.T,te.T=null;var n=he.p;he.p=2;var r=xe;xe|=4;try{s0(t,e);var s=cf,i=zy(e.containerInfo),o=s.focusedElem,l=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&By(o.ownerDocument.documentElement,o)){if(l!==null&&Ef(o)){var c=l.start,d=l.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var m=o.ownerDocument||document,f=m&&m.defaultView||window;if(f.getSelection){var h=f.getSelection(),x=o.textContent.length,y=Math.min(l.start,x),$=l.end===void 0?y:Math.min(l.end,x);!h.extend&&y>$&&(i=$,$=y,y=i);var g=lg(o,y),v=lg(o,$);if(g&&v&&(h.rangeCount!==1||h.anchorNode!==g.node||h.anchorOffset!==g.offset||h.focusNode!==v.node||h.focusOffset!==v.offset)){var b=m.createRange();b.setStart(g.node,g.offset),h.removeAllRanges(),y>$?(h.addRange(b),h.extend(v.node,v.offset)):(b.setEnd(v.node,v.offset),h.addRange(b))}}}}for(m=[],h=o;h=h.parentNode;)h.nodeType===1&&m.push({element:h,left:h.scrollLeft,top:h.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<m.length;o++){var w=m[o];w.element.scrollLeft=w.left,w.element.scrollTop=w.top}}Gu=!!uf,cf=uf=null}finally{xe=r,he.p=n,te.T=a}}e.current=t,ht=2}}function $0(){if(ht===2){ht=0;var e=Qn,t=Ks,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=te.T,te.T=null;var n=he.p;he.p=2;var r=xe;xe|=4;try{t0(e,t.alternate,t)}finally{xe=r,he.p=n,te.T=a}}ht=3}}function w0(){if(ht===4||ht===3){ht=0,HR();var e=Qn,t=Ks,a=Ls,n=m0;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?ht=5:(ht=0,Ks=Qn=null,S0(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(Hn=null),wf(a),t=t.stateNode,Gt&&typeof Gt.onCommitFiberRoot=="function")try{Gt.onCommitFiberRoot(Mo,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=te.T,r=he.p,he.p=2,te.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{te.T=t,he.p=r}}(Ls&3)!==0&&lc(),qa(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===rf?vo++:(vo=0,rf=e):vo=0,Ho(0,!1)}}function S0(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,Bo(t)))}function lc(e){return x0(),$0(),w0(),N0(e)}function N0(){if(ht!==5)return!1;var e=Qn,t=af;af=0;var a=wf(Ls),n=te.T,r=he.p;try{he.p=32>a?32:a,te.T=null,a=nf,nf=null;var s=Qn,i=Ls;if(ht=0,Ks=Qn=null,Ls=0,(xe&6)!==0)throw Error(U(331));var o=xe;if(xe|=4,c0(s.current),o0(s,s.current,i,a),xe=o,Ho(0,!1),Gt&&typeof Gt.onPostCommitFiberRoot=="function")try{Gt.onPostCommitFiberRoot(Mo,s)}catch{}return!0}finally{he.p=r,te.T=n,S0(e,t)}}function qg(e,t,a){t=ha(a,t),t=Xm(e.stateNode,t,2),e=Kn(e,t,2),e!==null&&(Lo(e,2),qa(e))}function Ne(e,t,a){if(e.tag===3)qg(e,e,a);else for(;t!==null;){if(t.tag===3){qg(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(Hn===null||!Hn.has(n))){e=ha(a,e),a=zb(2),n=Kn(t,a,2),n!==null&&(qb(a,n,t,e),Lo(n,2),qa(n));break}}t=t.return}}function gm(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new yE;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(ep=!0,r.add(a),e=SE.bind(null,e,t,a),t.then(e,e))}function SE(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,_e===e&&(ce&a)===a&&(ze===4||ze===3&&(ce&62914560)===ce&&300>Fa()-ap?(xe&2)===0&&Hs(e,0):tp|=a,Is===ce&&(Is=0)),qa(e)}function _0(e,t){t===0&&(t=xy()),e=Ys(e,t),e!==null&&(Lo(e,t),qa(e))}function NE(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),_0(e,a)}function _E(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(U(314))}n!==null&&n.delete(t),_0(e,a)}function kE(e,t){return xf(e,t)}var zu=null,hs=null,of=!1,qu=!1,ym=!1,Nr=0;function qa(e){e!==hs&&e.next===null&&(hs===null?zu=hs=e:hs=hs.next=e),qu=!0,of||(of=!0,CE())}function Ho(e,t){if(!ym&&qu){ym=!0;do for(var a=!1,n=zu;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Yt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,Ig(n,s))}else s=ce,s=Ju(n,n===_e?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||Oo(n,s)||(a=!0,Ig(n,s));n=n.next}while(a);ym=!1}}function RE(){k0()}function k0(){qu=of=!1;var e=0;Nr!==0&&(OE()&&(e=Nr),Nr=0);for(var t=Fa(),a=null,n=zu;n!==null;){var r=n.next,s=R0(n,t);s===0?(n.next=null,a===null?zu=r:a.next=r,r===null&&(hs=a)):(a=n,(e!==0||(s&3)!==0)&&(qu=!0)),n=r}Ho(e,!1)}function R0(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Yt(s),o=1<<i,l=r[i];l===-1?((o&a)===0||(o&n)!==0)&&(r[i]=WR(o,t)):l<=t&&(e.expiredLanes|=o),s&=~o}if(t=_e,a=ce,a=Ju(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&(be===2||be===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&Hd(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||Oo(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&Hd(n),wf(a)){case 2:case 8:a=gy;break;case 32:a=Su;break;case 268435456:a=yy;break;default:a=Su}return n=C0.bind(null,e),a=xf(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&Hd(n),e.callbackPriority=2,e.callbackNode=null,2}function C0(e,t){if(ht!==0&&ht!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(lc(!0)&&e.callbackNode!==a)return null;var n=ce;return n=Ju(e,e===_e?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:(p0(e,n,t),R0(e,Fa()),e.callbackNode!=null&&e.callbackNode===a?C0.bind(null,e):null)}function Ig(e,t){if(lc())return null;p0(e,t,!0)}function CE(){PE(function(){(xe&6)!==0?xf(vy,RE):k0()})}function rp(){return Nr===0&&(Nr=by()),Nr}function Kg(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:lu(""+e)}function Hg(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function EE(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=Kg((r[Ft]||null).action),i=n.submitter;i&&(t=(t=i[Ft]||null)?Kg(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Xu("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(Nr!==0){var l=i?Hg(r,i):new FormData(r);Ym(a,{pending:!0,data:l,method:r.method,action:s},null,l)}}else typeof s=="function"&&(o.preventDefault(),l=i?Hg(r,i):new FormData(r),Ym(a,{pending:!0,data:l,method:r.method,action:s},s,l))},currentTarget:r}]})}}for(au=0;au<Um.length;au++)nu=Um[au],Qg=nu.toLowerCase(),Vg=nu[0].toUpperCase()+nu.slice(1),Na(Qg,"on"+Vg);var nu,Qg,Vg,au;Na(Iy,"onAnimationEnd");Na(Ky,"onAnimationIteration");Na(Hy,"onAnimationStart");Na("dblclick","onDoubleClick");Na("focusin","onFocus");Na("focusout","onBlur");Na(GC,"onTransitionRun");Na(YC,"onTransitionStart");Na(JC,"onTransitionCancel");Na(Qy,"onTransitionEnd");Us("onMouseEnter",["mouseout","mouseover"]);Us("onMouseLeave",["mouseout","mouseover"]);Us("onPointerEnter",["pointerout","pointerover"]);Us("onPointerLeave",["pointerout","pointerover"]);Er("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Er("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Er("onBeforeInput",["compositionend","keypress","textInput","paste"]);Er("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Er("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Er("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var _o="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),TE=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(_o));function E0(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],l=o.instance,c=o.currentTarget;if(o=o.listener,l!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Lu(d)}r.currentTarget=null,s=l}else for(i=0;i<n.length;i++){if(o=n[i],l=o.instance,c=o.currentTarget,o=o.listener,l!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Lu(d)}r.currentTarget=null,s=l}}}}function oe(e,t){var a=t[Tm];a===void 0&&(a=t[Tm]=new Set);var n=e+"__bubble";a.has(n)||(T0(t,e,2,!1),a.add(n))}function bm(e,t,a){var n=0;t&&(n|=4),T0(a,e,n,t)}var ru="_reactListening"+Math.random().toString(36).slice(2);function sp(e){if(!e[ru]){e[ru]=!0,Ny.forEach(function(a){a!=="selectionchange"&&(TE.has(a)||bm(a,!1,e),bm(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[ru]||(t[ru]=!0,bm("selectionchange",!1,t))}}function T0(e,t,a,n){switch(q0(t)){case 2:var r=n3;break;case 8:r=r3;break;default:r=up}a=r.bind(null,t,a,e),r=void 0,!Om||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function xm(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var l=i.tag;if((l===3||l===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=ys(o),i===null)return;if(l=i.tag,l===5||l===6||l===26||l===27){n=s=i;continue e}o=o.parentNode}}n=n.return}Dy(function(){var c=s,d=_f(a),m=[];e:{var f=Vy.get(e);if(f!==void 0){var h=Xu,x=e;switch(e){case"keypress":if(cu(a)===0)break e;case"keydown":case"keyup":h=kC;break;case"focusin":x="focus",h=Wd;break;case"focusout":x="blur",h=Wd;break;case"beforeblur":case"afterblur":h=Wd;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":h=Wv;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":h=pC;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":h=EC;break;case Iy:case Ky:case Hy:h=gC;break;case Qy:h=AC;break;case"scroll":case"scrollend":h=mC;break;case"wheel":h=MC;break;case"copy":case"cut":case"paste":h=bC;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":h=tg;break;case"toggle":case"beforetoggle":h=LC}var y=(t&4)!==0,$=!y&&(e==="scroll"||e==="scrollend"),g=y?f!==null?f+"Capture":null:f;y=[];for(var v=c,b;v!==null;){var w=v;if(b=w.stateNode,w=w.tag,w!==5&&w!==26&&w!==27||b===null||g===null||(w=bo(v,g),w!=null&&y.push(ko(v,w,b))),$)break;v=v.return}0<y.length&&(f=new h(f,x,null,a,d),m.push({event:f,listeners:y}))}}if((t&7)===0){e:{if(f=e==="mouseover"||e==="pointerover",h=e==="mouseout"||e==="pointerout",f&&a!==Mm&&(x=a.relatedTarget||a.fromElement)&&(ys(x)||x[Vs]))break e;if((h||f)&&(f=d.window===d?d:(f=d.ownerDocument)?f.defaultView||f.parentWindow:window,h?(x=a.relatedTarget||a.toElement,h=c,x=x?ys(x):null,x!==null&&($=Do(x),y=x.tag,x!==$||y!==5&&y!==27&&y!==6)&&(x=null)):(h=null,x=c),h!==x)){if(y=Wv,w="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(y=tg,w="onPointerLeave",g="onPointerEnter",v="pointer"),$=h==null?f:to(h),b=x==null?f:to(x),f=new y(w,v+"leave",h,a,d),f.target=$,f.relatedTarget=b,w=null,ys(d)===c&&(y=new y(g,v+"enter",x,a,d),y.target=b,y.relatedTarget=$,w=y),$=w,h&&x)t:{for(y=h,g=x,v=0,b=y;b;b=ms(b))v++;for(b=0,w=g;w;w=ms(w))b++;for(;0<v-b;)y=ms(y),v--;for(;0<b-v;)g=ms(g),b--;for(;v--;){if(y===g||g!==null&&y===g.alternate)break t;y=ms(y),g=ms(g)}y=null}else y=null;h!==null&&Gg(m,f,h,y,!1),x!==null&&$!==null&&Gg(m,$,x,y,!0)}}e:{if(f=c?to(c):window,h=f.nodeName&&f.nodeName.toLowerCase(),h==="select"||h==="input"&&f.type==="file")var N=sg;else if(rg(f))if(jy)N=HC;else{N=IC;var C=qC}else h=f.nodeName,!h||h.toLowerCase()!=="input"||f.type!=="checkbox"&&f.type!=="radio"?c&&Nf(c.elementType)&&(N=sg):N=KC;if(N&&(N=N(e,c))){Uy(m,N,a,d);break e}C&&C(e,f,c),e==="focusout"&&c&&f.type==="number"&&c.memoizedProps.value!=null&&Dm(f,"number",f.value)}switch(C=c?to(c):window,e){case"focusin":(rg(C)||C.contentEditable==="true")&&($s=C,Lm=c,so=null);break;case"focusout":so=Lm=$s=null;break;case"mousedown":Pm=!0;break;case"contextmenu":case"mouseup":case"dragend":Pm=!1,ug(m,a,d);break;case"selectionchange":if(VC)break;case"keydown":case"keyup":ug(m,a,d)}var _;if(Cf)e:{switch(e){case"compositionstart":var A="onCompositionStart";break e;case"compositionend":A="onCompositionEnd";break e;case"compositionupdate":A="onCompositionUpdate";break e}A=void 0}else xs?Ly(e,a)&&(A="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(A="onCompositionStart");A&&(Oy&&a.locale!=="ko"&&(xs||A!=="onCompositionStart"?A==="onCompositionEnd"&&xs&&(_=My()):(Fn=d,kf="value"in Fn?Fn.value:Fn.textContent,xs=!0)),C=Iu(c,A),0<C.length&&(A=new eg(A,e,null,a,d),m.push({event:A,listeners:C}),_?A.data=_:(_=Py(a),_!==null&&(A.data=_)))),(_=UC?jC(e,a):FC(e,a))&&(A=Iu(c,"onBeforeInput"),0<A.length&&(C=new eg("onBeforeInput","beforeinput",null,a,d),m.push({event:C,listeners:A}),C.data=_)),EE(m,e,c,a,d)}E0(m,t)})}function ko(e,t,a){return{instance:e,listener:t,currentTarget:a}}function Iu(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=bo(e,a),r!=null&&n.unshift(ko(e,r,s)),r=bo(e,t),r!=null&&n.push(ko(e,r,s))),e.tag===3)return n;e=e.return}return[]}function ms(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function Gg(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,l=o.alternate,c=o.stateNode;if(o=o.tag,l!==null&&l===n)break;o!==5&&o!==26&&o!==27||c===null||(l=c,r?(c=bo(a,s),c!=null&&i.unshift(ko(a,c,l))):r||(c=bo(a,s),c!=null&&i.push(ko(a,c,l)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var AE=/\r\n?/g,DE=/\u0000|\uFFFD/g;function Yg(e){return(typeof e=="string"?e:""+e).replace(AE,`
`).replace(DE,"")}function A0(e,t){return t=Yg(t),Yg(e)===t}function uc(){}function we(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||js(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&js(e,""+n);break;case"className":Vl(e,"class",n);break;case"tabIndex":Vl(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":Vl(e,a,n);break;case"style":Ay(e,n,s);break;case"data":if(t!=="object"){Vl(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=lu(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&we(e,t,"name",r.name,r,null),we(e,t,"formEncType",r.formEncType,r,null),we(e,t,"formMethod",r.formMethod,r,null),we(e,t,"formTarget",r.formTarget,r,null)):(we(e,t,"encType",r.encType,r,null),we(e,t,"method",r.method,r,null),we(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=lu(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=uc);break;case"onScroll":n!=null&&oe("scroll",e);break;case"onScrollEnd":n!=null&&oe("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(U(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(U(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=lu(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":oe("beforetoggle",e),oe("toggle",e),ou(e,"popover",n);break;case"xlinkActuate":en(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":en(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":en(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":en(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":en(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":en(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":en(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":en(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":en(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":ou(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=cC.get(a)||a,ou(e,a,n))}}function lf(e,t,a,n,r,s){switch(a){case"style":Ay(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(U(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(U(60));e.innerHTML=a}}break;case"children":typeof n=="string"?js(e,n):(typeof n=="number"||typeof n=="bigint")&&js(e,""+n);break;case"onScroll":n!=null&&oe("scroll",e);break;case"onScrollEnd":n!=null&&oe("scrollend",e);break;case"onClick":n!=null&&(e.onclick=uc);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!_y.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[Ft]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):ou(e,a,n)}}}function vt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":oe("error",e),oe("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(U(137,t));default:we(e,t,s,i,a,null)}}r&&we(e,t,"srcSet",a.srcSet,a,null),n&&we(e,t,"src",a.src,a,null);return;case"input":oe("invalid",e);var o=s=i=r=null,l=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":l=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(U(137,t));break;default:we(e,t,n,d,a,null)}}Cy(e,s,o,l,c,i,r,!1),Nu(e);return;case"select":oe("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:we(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?Es(e,!!n,t,!1):a!=null&&Es(e,!!n,a,!0);return;case"textarea":oe("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(U(91));break;default:we(e,t,i,o,a,null)}Ty(e,n,r,s),Nu(e);return;case"option":for(l in a)if(a.hasOwnProperty(l)&&(n=a[l],n!=null))switch(l){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:we(e,t,l,n,a,null)}return;case"dialog":oe("beforetoggle",e),oe("toggle",e),oe("cancel",e),oe("close",e);break;case"iframe":case"object":oe("load",e);break;case"video":case"audio":for(n=0;n<_o.length;n++)oe(_o[n],e);break;case"image":oe("error",e),oe("load",e);break;case"details":oe("toggle",e);break;case"embed":case"source":case"link":oe("error",e),oe("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(U(137,t));default:we(e,t,c,n,a,null)}return;default:if(Nf(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&lf(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&we(e,t,o,n,a,null))}function ME(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,l=null,c=null,d=null;for(h in a){var m=a[h];if(a.hasOwnProperty(h)&&m!=null)switch(h){case"checked":break;case"value":break;case"defaultValue":l=m;default:n.hasOwnProperty(h)||we(e,t,h,null,n,m)}}for(var f in n){var h=n[f];if(m=a[f],n.hasOwnProperty(f)&&(h!=null||m!=null))switch(f){case"type":s=h;break;case"name":r=h;break;case"checked":c=h;break;case"defaultChecked":d=h;break;case"value":i=h;break;case"defaultValue":o=h;break;case"children":case"dangerouslySetInnerHTML":if(h!=null)throw Error(U(137,t));break;default:h!==m&&we(e,t,f,h,n,m)}}Am(e,i,o,l,c,d,s,r);return;case"select":h=i=o=f=null;for(s in a)if(l=a[s],a.hasOwnProperty(s)&&l!=null)switch(s){case"value":break;case"multiple":h=l;default:n.hasOwnProperty(s)||we(e,t,s,null,n,l)}for(r in n)if(s=n[r],l=a[r],n.hasOwnProperty(r)&&(s!=null||l!=null))switch(r){case"value":f=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==l&&we(e,t,r,s,n,l)}t=o,a=i,n=h,f!=null?Es(e,!!a,f,!1):!!n!=!!a&&(t!=null?Es(e,!!a,t,!0):Es(e,!!a,a?[]:"",!1));return;case"textarea":h=f=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:we(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":f=r;break;case"defaultValue":h=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(U(91));break;default:r!==s&&we(e,t,i,r,n,s)}Ey(e,f,h);return;case"option":for(var x in a)if(f=a[x],a.hasOwnProperty(x)&&f!=null&&!n.hasOwnProperty(x))switch(x){case"selected":e.selected=!1;break;default:we(e,t,x,null,n,f)}for(l in n)if(f=n[l],h=a[l],n.hasOwnProperty(l)&&f!==h&&(f!=null||h!=null))switch(l){case"selected":e.selected=f&&typeof f!="function"&&typeof f!="symbol";break;default:we(e,t,l,f,n,h)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var y in a)f=a[y],a.hasOwnProperty(y)&&f!=null&&!n.hasOwnProperty(y)&&we(e,t,y,null,n,f);for(c in n)if(f=n[c],h=a[c],n.hasOwnProperty(c)&&f!==h&&(f!=null||h!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(f!=null)throw Error(U(137,t));break;default:we(e,t,c,f,n,h)}return;default:if(Nf(t)){for(var $ in a)f=a[$],a.hasOwnProperty($)&&f!==void 0&&!n.hasOwnProperty($)&&lf(e,t,$,void 0,n,f);for(d in n)f=n[d],h=a[d],!n.hasOwnProperty(d)||f===h||f===void 0&&h===void 0||lf(e,t,d,f,n,h);return}}for(var g in a)f=a[g],a.hasOwnProperty(g)&&f!=null&&!n.hasOwnProperty(g)&&we(e,t,g,null,n,f);for(m in n)f=n[m],h=a[m],!n.hasOwnProperty(m)||f===h||f==null&&h==null||we(e,t,m,f,n,h)}var uf=null,cf=null;function Ku(e){return e.nodeType===9?e:e.ownerDocument}function Jg(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function D0(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function df(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var $m=null;function OE(){var e=window.event;return e&&e.type==="popstate"?e===$m?!1:($m=e,!0):($m=null,!1)}var M0=typeof setTimeout=="function"?setTimeout:void 0,LE=typeof clearTimeout=="function"?clearTimeout:void 0,Xg=typeof Promise=="function"?Promise:void 0,PE=typeof queueMicrotask=="function"?queueMicrotask:typeof Xg<"u"?function(e){return Xg.resolve(null).then(e).catch(UE)}:M0;function UE(e){setTimeout(function(){throw e})}function er(e){return e==="head"}function Zg(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&go(i.documentElement),a&2&&go(i.body),a&4)for(a=i.head,go(a),i=a.firstChild;i;){var o=i.nextSibling,l=i.nodeName;i[Po]||l==="SCRIPT"||l==="STYLE"||l==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),Ao(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);Ao(t)}function mf(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":mf(a),Sf(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function jE(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[Po])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=Sa(e.nextSibling),e===null)break}return null}function FE(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=Sa(e.nextSibling),e===null))return null;return e}function ff(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function BE(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function Sa(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var pf=null;function Wg(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function O0(e,t,a){switch(t=Ku(a),e){case"html":if(e=t.documentElement,!e)throw Error(U(452));return e;case"head":if(e=t.head,!e)throw Error(U(453));return e;case"body":if(e=t.body,!e)throw Error(U(454));return e;default:throw Error(U(451))}}function go(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);Sf(e)}var ya=new Map,ey=new Set;function Hu(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var vn=he.d;he.d={f:zE,r:qE,D:IE,C:KE,L:HE,m:QE,X:GE,S:VE,M:YE};function zE(){var e=vn.f(),t=ic();return e||t}function qE(e){var t=Gs(e);t!==null&&t.tag===5&&t.type==="form"?Rb(t):vn.r(e)}var Xs=typeof document>"u"?null:document;function L0(e,t,a){var n=Xs;if(n&&typeof t=="string"&&t){var r=pa(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),ey.has(r)||(ey.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),vt(t,"link",e),lt(t),n.head.appendChild(t)))}}function IE(e){vn.D(e),L0("dns-prefetch",e,null)}function KE(e,t){vn.C(e,t),L0("preconnect",e,t)}function HE(e,t,a){vn.L(e,t,a);var n=Xs;if(n&&e&&t){var r='link[rel="preload"][as="'+pa(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+pa(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+pa(a.imageSizes)+'"]')):r+='[href="'+pa(e)+'"]';var s=r;switch(t){case"style":s=Qs(e);break;case"script":s=Zs(e)}ya.has(s)||(e=Te({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),ya.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(Qo(s))||t==="script"&&n.querySelector(Vo(s))||(t=n.createElement("link"),vt(t,"link",e),lt(t),n.head.appendChild(t)))}}function QE(e,t){vn.m(e,t);var a=Xs;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+pa(n)+'"][href="'+pa(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=Zs(e)}if(!ya.has(s)&&(e=Te({rel:"modulepreload",href:e},t),ya.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(Vo(s)))return}n=a.createElement("link"),vt(n,"link",e),lt(n),a.head.appendChild(n)}}}function VE(e,t,a){vn.S(e,t,a);var n=Xs;if(n&&e){var r=Cs(n).hoistableStyles,s=Qs(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(Qo(s)))o.loading=5;else{e=Te({rel:"stylesheet",href:e,"data-precedence":t},a),(a=ya.get(s))&&ip(e,a);var l=i=n.createElement("link");lt(l),vt(l,"link",e),l._p=new Promise(function(c,d){l.onload=c,l.onerror=d}),l.addEventListener("load",function(){o.loading|=1}),l.addEventListener("error",function(){o.loading|=2}),o.loading|=4,gu(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function GE(e,t){vn.X(e,t);var a=Xs;if(a&&e){var n=Cs(a).hoistableScripts,r=Zs(e),s=n.get(r);s||(s=a.querySelector(Vo(r)),s||(e=Te({src:e,async:!0},t),(t=ya.get(r))&&op(e,t),s=a.createElement("script"),lt(s),vt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function YE(e,t){vn.M(e,t);var a=Xs;if(a&&e){var n=Cs(a).hoistableScripts,r=Zs(e),s=n.get(r);s||(s=a.querySelector(Vo(r)),s||(e=Te({src:e,async:!0,type:"module"},t),(t=ya.get(r))&&op(e,t),s=a.createElement("script"),lt(s),vt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function ty(e,t,a,n){var r=(r=qn.current)?Hu(r):null;if(!r)throw Error(U(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=Qs(a.href),a=Cs(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=Qs(a.href);var s=Cs(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(Qo(e)))&&!s._p&&(i.instance=s,i.state.loading=5),ya.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},ya.set(e,a),s||JE(r,e,a,i.state))),t&&n===null)throw Error(U(528,""));return i}if(t&&n!==null)throw Error(U(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=Zs(a),a=Cs(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(U(444,e))}}function Qs(e){return'href="'+pa(e)+'"'}function Qo(e){return'link[rel="stylesheet"]['+e+"]"}function P0(e){return Te({},e,{"data-precedence":e.precedence,precedence:null})}function JE(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),vt(t,"link",a),lt(t),e.head.appendChild(t))}function Zs(e){return'[src="'+pa(e)+'"]'}function Vo(e){return"script[async]"+e}function ay(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+pa(a.href)+'"]');if(n)return t.instance=n,lt(n),n;var r=Te({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),lt(n),vt(n,"style",r),gu(n,a.precedence,e),t.instance=n;case"stylesheet":r=Qs(a.href);var s=e.querySelector(Qo(r));if(s)return t.state.loading|=4,t.instance=s,lt(s),s;n=P0(a),(r=ya.get(r))&&ip(n,r),s=(e.ownerDocument||e).createElement("link"),lt(s);var i=s;return i._p=new Promise(function(o,l){i.onload=o,i.onerror=l}),vt(s,"link",n),t.state.loading|=4,gu(s,a.precedence,e),t.instance=s;case"script":return s=Zs(a.src),(r=e.querySelector(Vo(s)))?(t.instance=r,lt(r),r):(n=a,(r=ya.get(s))&&(n=Te({},a),op(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),lt(r),vt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(U(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,gu(n,a.precedence,e));return t.instance}function gu(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function ip(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function op(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var yu=null;function ny(e,t,a){if(yu===null){var n=new Map,r=yu=new Map;r.set(a,n)}else r=yu,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[Po]||s[xt]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function ry(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function XE(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function U0(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var Ro=null;function ZE(){}function WE(e,t,a){if(Ro===null)throw Error(U(475));var n=Ro;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=Qs(a.href),s=e.querySelector(Qo(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=Qu.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,lt(s);return}s=e.ownerDocument||e,a=P0(a),(r=ya.get(r))&&ip(a,r),s=s.createElement("link"),lt(s);var i=s;i._p=new Promise(function(o,l){i.onload=o,i.onerror=l}),vt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=Qu.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function e3(){if(Ro===null)throw Error(U(475));var e=Ro;return e.stylesheets&&e.count===0&&hf(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&hf(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function Qu(){if(this.count--,this.count===0){if(this.stylesheets)hf(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var Vu=null;function hf(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,Vu=new Map,t.forEach(t3,e),Vu=null,Qu.call(e))}function t3(e,t){if(!(t.state.loading&4)){var a=Vu.get(e);if(a)var n=a.get(null);else{a=new Map,Vu.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=Qu.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var Co={$$typeof:rn,Provider:null,Consumer:null,_currentValue:yr,_currentValue2:yr,_threadCount:0};function a3(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=Qd(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=Qd(0),this.hiddenUpdates=Qd(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function j0(e,t,a,n,r,s,i,o,l,c,d,m){return e=new a3(e,t,a,i,o,l,c,m),t=1,s===!0&&(t|=24),s=Vt(3,null,null,t),e.current=s,s.stateNode=e,t=Lf(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},Uf(s),e}function F0(e){return e?(e=Ns,e):Ns}function B0(e,t,a,n,r,s){r=F0(r),n.context===null?n.context=r:n.pendingContext=r,n=In(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=Kn(e,n,t),a!==null&&(Xt(a,e,t),lo(a,e,t))}function sy(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function lp(e,t){sy(e,t),(e=e.alternate)&&sy(e,t)}function z0(e){if(e.tag===13){var t=Ys(e,67108864);t!==null&&Xt(t,e,67108864),lp(e,67108864)}}var Gu=!0;function n3(e,t,a,n){var r=te.T;te.T=null;var s=he.p;try{he.p=2,up(e,t,a,n)}finally{he.p=s,te.T=r}}function r3(e,t,a,n){var r=te.T;te.T=null;var s=he.p;try{he.p=8,up(e,t,a,n)}finally{he.p=s,te.T=r}}function up(e,t,a,n){if(Gu){var r=vf(n);if(r===null)xm(e,t,n,Yu,a),iy(e,n);else if(i3(r,e,t,a,n))n.stopPropagation();else if(iy(e,n),t&4&&-1<s3.indexOf(e)){for(;r!==null;){var s=Gs(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=hr(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var l=1<<31-Yt(i);o.entanglements[1]|=l,i&=~l}qa(s),(xe&6)===0&&(Fu=Fa()+500,Ho(0,!1))}}break;case 13:o=Ys(s,2),o!==null&&Xt(o,s,2),ic(),lp(s,2)}if(s=vf(n),s===null&&xm(e,t,n,Yu,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else xm(e,t,n,null,a)}}function vf(e){return e=_f(e),cp(e)}var Yu=null;function cp(e){if(Yu=null,e=ys(e),e!==null){var t=Do(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=my(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return Yu=e,null}function q0(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(QR()){case vy:return 2;case gy:return 8;case Su:case VR:return 32;case yy:return 268435456;default:return 32}default:return 32}}var gf=!1,Vn=null,Gn=null,Yn=null,Eo=new Map,To=new Map,Un=[],s3="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function iy(e,t){switch(e){case"focusin":case"focusout":Vn=null;break;case"dragenter":case"dragleave":Gn=null;break;case"mouseover":case"mouseout":Yn=null;break;case"pointerover":case"pointerout":Eo.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":To.delete(t.pointerId)}}function Xi(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Gs(t),t!==null&&z0(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function i3(e,t,a,n,r){switch(t){case"focusin":return Vn=Xi(Vn,e,t,a,n,r),!0;case"dragenter":return Gn=Xi(Gn,e,t,a,n,r),!0;case"mouseover":return Yn=Xi(Yn,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return Eo.set(s,Xi(Eo.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,To.set(s,Xi(To.get(s)||null,e,t,a,n,r)),!0}return!1}function I0(e){var t=ys(e.target);if(t!==null){var a=Do(t);if(a!==null){if(t=a.tag,t===13){if(t=my(a),t!==null){e.blockedOn=t,tC(e.priority,function(){if(a.tag===13){var n=Jt();n=$f(n);var r=Ys(a,n);r!==null&&Xt(r,a,n),lp(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function bu(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=vf(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);Mm=n,a.target.dispatchEvent(n),Mm=null}else return t=Gs(a),t!==null&&z0(t),e.blockedOn=a,!1;t.shift()}return!0}function oy(e,t,a){bu(e)&&a.delete(t)}function o3(){gf=!1,Vn!==null&&bu(Vn)&&(Vn=null),Gn!==null&&bu(Gn)&&(Gn=null),Yn!==null&&bu(Yn)&&(Yn=null),Eo.forEach(oy),To.forEach(oy)}function su(e,t){e.blockedOn===t&&(e.blockedOn=null,gf||(gf=!0,rt.unstable_scheduleCallback(rt.unstable_NormalPriority,o3)))}var iu=null;function ly(e){iu!==e&&(iu=e,rt.unstable_scheduleCallback(rt.unstable_NormalPriority,function(){iu===e&&(iu=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(cp(n||a)===null)continue;break}var s=Gs(a);s!==null&&(e.splice(t,3),t-=3,Ym(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function Ao(e){function t(l){return su(l,e)}Vn!==null&&su(Vn,e),Gn!==null&&su(Gn,e),Yn!==null&&su(Yn,e),Eo.forEach(t),To.forEach(t);for(var a=0;a<Un.length;a++){var n=Un[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<Un.length&&(a=Un[0],a.blockedOn===null);)I0(a),a.blockedOn===null&&Un.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[Ft]||null;if(typeof s=="function")i||ly(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[Ft]||null)o=i.formAction;else if(cp(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),ly(a)}}}function dp(e){this._internalRoot=e}cc.prototype.render=dp.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(U(409));var a=t.current,n=Jt();B0(a,n,e,t,null,null)};cc.prototype.unmount=dp.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;B0(e.current,2,null,e,null,null),ic(),t[Vs]=null}};function cc(e){this._internalRoot=e}cc.prototype.unstable_scheduleHydration=function(e){if(e){var t=Sy();e={blockedOn:null,target:e,priority:t};for(var a=0;a<Un.length&&t!==0&&t<Un[a].priority;a++);Un.splice(a,0,e),a===0&&I0(e)}};var uy=cy.version;if(uy!=="19.1.0")throw Error(U(527,uy,"19.1.0"));he.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(U(188)):(e=Object.keys(e).join(","),Error(U(268,e)));return e=FR(t),e=e!==null?fy(e):null,e=e===null?null:e.stateNode,e};var l3={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:te,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(Zi=__REACT_DEVTOOLS_GLOBAL_HOOK__,!Zi.isDisabled&&Zi.supportsFiber))try{Mo=Zi.inject(l3),Gt=Zi}catch{}var Zi;dc.createRoot=function(e,t){if(!dy(e))throw Error(U(299));var a=!1,n="",r=jb,s=Fb,i=Bb,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=j0(e,1,!1,null,null,a,n,r,s,i,o,null),e[Vs]=t.current,sp(e),new dp(t)};dc.hydrateRoot=function(e,t,a){if(!dy(e))throw Error(U(299));var n=!1,r="",s=jb,i=Fb,o=Bb,l=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(l=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=j0(e,1,!0,t,a??null,n,r,s,i,o,l,c),t.context=F0(null),a=t.current,n=Jt(),n=$f(n),r=In(n),r.callback=null,Kn(a,r,n),a=n,t.current.lanes=a,Lo(t,a),qa(t),e[Vs]=t.current,sp(e),new cc(t)};dc.version="19.1.0"});var V0=_n((U6,Q0)=>{"use strict";function H0(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(H0)}catch(e){console.error(e)}}H0(),Q0.exports=K0()});var Mt=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var yR={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},bR=class{#t=yR;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},Aa=new bR;function av(e){setTimeout(e,0)}var Ot=typeof window>"u"||"Deno"in globalThis;function Ae(){}function sv(e,t){return typeof e=="function"?e(t):e}function Mi(e){return typeof e=="number"&&e>=0&&e!==1/0}function Sl(e,t){return Math.max(e+(t||0)-Date.now(),0)}function $a(e,t){return typeof e=="function"?e(t):e}function Lt(e,t){return typeof e=="function"?e(t):e}function Nl(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==Oi(i,t.options))return!1}else if(!mr(t.queryKey,i))return!1}if(a!=="all"){let l=t.isActive();if(a==="active"&&!l||a==="inactive"&&l)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function _l(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(Da(t.options.mutationKey)!==Da(s))return!1}else if(!mr(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function Oi(e,t){return(t?.queryKeyHashFn||Da)(e)}function Da(e){return JSON.stringify(e,(t,a)=>Sd(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function mr(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>mr(e[a],t[a])):!1}var xR=Object.prototype.hasOwnProperty;function Li(e,t){if(e===t)return e;let a=nv(e)&&nv(t);if(!a&&!(Sd(e)&&Sd(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},l=0;for(let c=0;c<i;c++){let d=a?c:s[c],m=e[d],f=t[d];if(m===f){o[d]=m,(a?c<r:xR.call(e,d))&&l++;continue}if(m===null||f===null||typeof m!="object"||typeof f!="object"){o[d]=f;continue}let h=Li(m,f);o[d]=h,h===m&&l++}return r===i&&l===r?e:o}function kn(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function nv(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function Sd(e){if(!rv(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!rv(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function rv(e){return Object.prototype.toString.call(e)==="[object Object]"}function iv(e){return new Promise(t=>{Aa.setTimeout(t,e)})}function Pi(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?Li(e,t):t}function ov(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function lv(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var ns=Symbol();function kl(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===ns?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function Ui(e,t){return typeof e=="function"?e(...t):!!e}var $R=class extends Mt{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!Ot&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},rs=new $R;function ji(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var uv=av;function wR(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=uv,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(l=>{a(l)})})})};return{batch:o=>{let l;t++;try{l=o()}finally{t--,t||i()}return l},batchCalls:o=>(...l)=>{s(()=>{o(...l)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var ue=wR();var SR=class extends Mt{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!Ot&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},ss=new SR;function NR(e){return Math.min(1e3*2**e,3e4)}function Nd(e){return(e??"online")==="online"?ss.isOnline():!0}var Rl=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function Cl(e){let t=!1,a=0,n,r=ji(),s=()=>r.status!=="pending",i=y=>{if(!s()){let $=new Rl(y);f($),e.onCancel?.($)}},o=()=>{t=!0},l=()=>{t=!1},c=()=>rs.isFocused()&&(e.networkMode==="always"||ss.isOnline())&&e.canRun(),d=()=>Nd(e.networkMode)&&e.canRun(),m=y=>{s()||(n?.(),r.resolve(y))},f=y=>{s()||(n?.(),r.reject(y))},h=()=>new Promise(y=>{n=$=>{(s()||c())&&y($)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),x=()=>{if(s())return;let y,$=a===0?e.initialPromise:void 0;try{y=$??e.fn()}catch(g){y=Promise.reject(g)}Promise.resolve(y).then(m).catch(g=>{if(s())return;let v=e.retry??(Ot?0:3),b=e.retryDelay??NR,w=typeof b=="function"?b(a,g):b,N=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!N){f(g);return}a++,e.onFail?.(a,g),iv(w).then(()=>c()?void 0:h()).then(()=>{t?f(g):x()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:l,canStart:d,start:()=>(d()?x():h().then(x),r)}}var El=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),Mi(this.gcTime)&&(this.#t=Aa.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(Ot?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(Aa.clearTimeout(this.#t),this.#t=void 0)}};var dv=class extends El{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=cv(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=cv(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=Pi(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(Ae).catch(Ae):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>Lt(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===ns||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>$a(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!Sl(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(l=>l.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=kl(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=Cl({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof Rl&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,l)=>{this.#i({type:"failed",failureCount:o,error:l})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof Rl){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,..._d(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),ue.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function _d(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:Nd(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function cv(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var fr=class extends Mt{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=ji(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),mv(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return kd(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return kd(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof Lt(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!kn(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&fv(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||Lt(this.options.enabled,this.#e)!==Lt(t.enabled,this.#e)||$a(this.options.staleTime,this.#e)!==$a(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||Lt(this.options.enabled,this.#e)!==Lt(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return kR(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(Ae)),t}#v(){this.#x();let e=$a(this.options.staleTime,this.#e);if(Ot||this.#n.isStale||!Mi(e))return;let a=Sl(this.#n.dataUpdatedAt,e)+1;this.#u=Aa.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(Ot||Lt(this.options.enabled,this.#e)===!1||!Mi(this.#l)||this.#l===0)&&(this.#c=Aa.setInterval(()=>{(this.options.refetchIntervalInBackground||rs.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(Aa.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(Aa.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,l=e!==a?e.state:this.#a,{state:c}=e,d={...c},m=!1,f;if(t._optimisticResults){let A=this.hasListeners(),L=!A&&mv(e,t),M=A&&fv(e,a,t,n);(L||M)&&(d={...d,..._d(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:h,errorUpdatedAt:x,status:y}=d;f=d.data;let $=!1;if(t.placeholderData!==void 0&&f===void 0&&y==="pending"){let A;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(A=r.data,$=!0):A=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,A!==void 0&&(y="success",f=Pi(r?.data,A,t),m=!0)}if(t.select&&f!==void 0&&!$)if(r&&f===s?.data&&t.select===this.#f)f=this.#d;else try{this.#f=t.select,f=t.select(f),f=Pi(r?.data,f,t),this.#d=f,this.#i=null}catch(A){this.#i=A}this.#i&&(h=this.#i,f=this.#d,x=Date.now(),y="error");let g=d.fetchStatus==="fetching",v=y==="pending",b=y==="error",w=v&&g,N=f!==void 0,_={status:y,fetchStatus:d.fetchStatus,isPending:v,isSuccess:y==="success",isError:b,isInitialLoading:w,isLoading:w,data:f,dataUpdatedAt:d.dataUpdatedAt,error:h,errorUpdatedAt:x,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>l.dataUpdateCount||d.errorUpdateCount>l.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:b&&!N,isPaused:d.fetchStatus==="paused",isPlaceholderData:m,isRefetchError:b&&N,isStale:Rd(e,t),refetch:this.refetch,promise:this.#o,isEnabled:Lt(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let A=P=>{_.status==="error"?P.reject(_.error):_.data!==void 0&&P.resolve(_.data)},L=()=>{let P=this.#o=_.promise=ji();A(P)},M=this.#o;switch(M.status){case"pending":e.queryHash===a.queryHash&&A(M);break;case"fulfilled":(_.status==="error"||_.data!==M.value)&&L();break;case"rejected":(_.status!=="error"||_.error!==M.reason)&&L();break}}return _}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),kn(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){ue.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function _R(e,t){return Lt(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function mv(e,t){return _R(e,t)||e.state.data!==void 0&&kd(e,t,t.refetchOnMount)}function kd(e,t,a){if(Lt(t.enabled,e)!==!1&&$a(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&Rd(e,t)}return!1}function fv(e,t,a,n){return(e!==t||Lt(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&Rd(e,a)}function Rd(e,t){return Lt(t.enabled,e)!==!1&&e.isStaleByTime($a(t.staleTime,e))}function kR(e,t){return!kn(e.getCurrentResult(),t)}function Cd(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},l=0,c=async()=>{let d=!1,m=x=>{Object.defineProperty(x,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},f=kl(t.options,t.fetchOptions),h=async(x,y,$)=>{if(d)return Promise.reject();if(y==null&&x.pages.length)return Promise.resolve(x);let v=(()=>{let C={client:t.client,queryKey:t.queryKey,pageParam:y,direction:$?"backward":"forward",meta:t.options.meta};return m(C),C})(),b=await f(v),{maxPages:w}=t.options,N=$?lv:ov;return{pages:N(x.pages,b,w),pageParams:N(x.pageParams,y,w)}};if(r&&s.length){let x=r==="backward",y=x?RR:pv,$={pages:s,pageParams:i},g=y(n,$);o=await h($,g,x)}else{let x=e??s.length;do{let y=l===0?i[0]??n.initialPageParam:pv(n,o);if(l>0&&y==null)break;o=await h(o,y),l++}while(l<x)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function pv(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function RR(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var hv=class extends El{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||Ed(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=Cl({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),ue.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function Ed(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var vv=class extends Mt{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new hv({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=Tl(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=Tl(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=Tl(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=Tl(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){ue.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>_l(t,a))}findAll(e={}){return this.getAll().filter(t=>_l(e,t))}notify(e){ue.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return ue.batch(()=>Promise.all(e.map(t=>t.continue().catch(Ae))))}};function Tl(e){return e.options.scope?.id}var Td=class extends Mt{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),kn(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&Da(t.mutationKey)!==Da(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??Ed();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){ue.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function gv(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function CR(e,t,a){let n=e.slice(0);return n[t]=a,n}var Ad=class extends Mt{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,ue.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,m)=>d!==a[m]),l=i||o,c=l?!0:s.some((d,m)=>{let f=this.#e[m];return!f||!kn(d,f)});!l&&!c||(l&&(this.#r=r),this.#e=s,this.hasListeners()&&(l&&(gv(a,r).forEach(d=>{d.destroy()}),gv(r,a).forEach(d=>{d.subscribe(m=>{this.#c(d,m)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=Li(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new fr(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=CR(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&ue.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var yv=class extends Mt{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??Oi(n,t),s=this.get(r);return s||(s=new dv({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){ue.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>Nl(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>Nl(e,a)):t}notify(e){ue.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){ue.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){ue.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var Dd=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new yv,this.#e=e.mutationCache||new vv,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=rs.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=ss.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime($a(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=sv(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return ue.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;ue.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return ue.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=ue.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(Ae).catch(Ae)}invalidateQueries(e,t={}){return ue.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=ue.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(Ae)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(Ae)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime($a(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(Ae).catch(Ae)}fetchInfiniteQuery(e){return e.behavior=Cd(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(Ae).catch(Ae)}ensureInfiniteQueryData(e){return e.behavior=Cd(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return ss.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(Da(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{mr(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(Da(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{mr(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=Oi(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===ns&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var Ma=Fe(Ie(),1);var is=Fe(Ie(),1),wv=Fe(Md(),1),Od=is.createContext(void 0),J=e=>{let t=is.useContext(Od);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},Ld=({client:e,children:t})=>(is.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,wv.jsx)(Od.Provider,{value:e,children:t}));var Dl=Fe(Ie(),1),Sv=Dl.createContext(!1),Ml=()=>Dl.useContext(Sv),WL=Sv.Provider;var Fi=Fe(Ie(),1),AR=Fe(Md(),1);function DR(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var MR=Fi.createContext(DR()),Ol=()=>Fi.useContext(MR);var Nv=Fe(Ie(),1);var Ll=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},Pl=e=>{Nv.useEffect(()=>{e.clearReset()},[e])},Ul=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||Ui(a,[e.error,n]));var jl=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},Fl=(e,t)=>e.isLoading&&e.isFetching&&!t,Bi=(e,t)=>e?.suspense&&t.isPending,os=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function Pd({queries:e,...t},a){let n=J(a),r=Ml(),s=Ol(),i=Ma.useMemo(()=>e.map(y=>{let $=n.defaultQueryOptions(y);return $._optimisticResults=r?"isRestoring":"optimistic",$}),[e,n,r]);i.forEach(y=>{jl(y),Ll(y,s)}),Pl(s);let[o]=Ma.useState(()=>new Ad(n,i,t)),[l,c,d]=o.getOptimisticResult(i,t.combine),m=!r&&t.subscribed!==!1;Ma.useSyncExternalStore(Ma.useCallback(y=>m?o.subscribe(ue.batchCalls(y)):Ae,[o,m]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),Ma.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let h=l.some((y,$)=>Bi(i[$],y))?l.flatMap((y,$)=>{let g=i[$];if(g){let v=new fr(n,g);if(Bi(g,y))return os(g,v,s);Fl(y,r)&&os(g,v,s)}return[]}):[];if(h.length>0)throw Promise.all(h);let x=l.find((y,$)=>{let g=i[$];return g&&Ul({result:y,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(x?.error)throw x.error;return c(d())}var Rn=Fe(Ie(),1);function _v(e,t,a){let n=Ml(),r=Ol(),s=J(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",jl(i),Ll(i,r),Pl(r);let o=!s.getQueryCache().get(i.queryHash),[l]=Rn.useState(()=>new t(s,i)),c=l.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(Rn.useSyncExternalStore(Rn.useCallback(m=>{let f=d?l.subscribe(ue.batchCalls(m)):Ae;return l.updateResult(),f},[l,d]),()=>l.getCurrentResult(),()=>l.getCurrentResult()),Rn.useEffect(()=>{l.setOptions(i)},[i,l]),Bi(i,c))throw os(i,l,r);if(Ul({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!Ot&&Fl(c,n)&&(o?os(i,l,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(Ae).finally(()=>{l.updateResult()}),i.notifyOnChangeProps?c:l.trackResult(c)}function K(e,t){return _v(e,fr,t)}var Za=Fe(Ie(),1);function Q(e,t){let a=J(t),[n]=Za.useState(()=>new Td(a,e));Za.useEffect(()=>{n.setOptions(e)},[n,e]);let r=Za.useSyncExternalStore(Za.useCallback(i=>n.subscribe(ue.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=Za.useCallback((i,o)=>{n.mutate(i,o).catch(Ae)},[n]);if(r.error&&Ui(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var hR=Fe(V0());var ea=Fe(Ie(),1),X=Fe(Ie(),1),Re=Fe(Ie(),1),Tp=Fe(Ie(),1),vx=Fe(Ie(),1),ve=Fe(Ie(),1),uT=Fe(Ie(),1),cT=Fe(Ie(),1),dT=Fe(Ie(),1),W=Fe(Ie(),1),Ax=Fe(Ie(),1);var G0="popstate";function W0(e={}){function t(n,r){let{pathname:s,search:i,hash:o}=n.location;return pp("",{pathname:s,search:i,hash:o},r.state&&r.state.usr||null,r.state&&r.state.key||"default")}function a(n,r){return typeof r=="string"?r:Ws(r)}return c3(t,a,null,e)}function ke(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function Wt(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function u3(){return Math.random().toString(36).substring(2,10)}function Y0(e,t){return{usr:e.state,key:e.key,idx:t}}function pp(e,t,a=null,n){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?Mr(t):t,state:a,key:t&&t.key||n||u3()}}function Ws({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function Mr(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function c3(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",l=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function m(){o="POP";let $=d(),g=$==null?null:$-c;c=$,l&&l({action:o,location:y.location,delta:g})}function f($,g){o="PUSH";let v=pp(y.location,$,g);a&&a(v,$),c=d()+1;let b=Y0(v,c),w=y.createHref(v);try{i.pushState(b,"",w)}catch(N){if(N instanceof DOMException&&N.name==="DataCloneError")throw N;r.location.assign(w)}s&&l&&l({action:o,location:y.location,delta:1})}function h($,g){o="REPLACE";let v=pp(y.location,$,g);a&&a(v,$),c=d();let b=Y0(v,c),w=y.createHref(v);i.replaceState(b,"",w),s&&l&&l({action:o,location:y.location,delta:0})}function x($){return d3($)}let y={get action(){return o},get location(){return e(r,i)},listen($){if(l)throw new Error("A history only accepts one active listener");return r.addEventListener(G0,m),l=$,()=>{r.removeEventListener(G0,m),l=null}},createHref($){return t(r,$)},createURL:x,encodeLocation($){let g=x($);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:f,replace:h,go($){return i.go($)}};return y}function d3(e,t=!1){let a="http://localhost";typeof window<"u"&&(a=window.location.origin!=="null"?window.location.origin:window.location.href),ke(a,"No window.location.(origin|href) available to create URL");let n=typeof e=="string"?e:Ws(e);return n=n.replace(/ $/,"%20"),!t&&n.startsWith("//")&&(n=a+n),new URL(n,a)}var m3;m3=new WeakMap;function yp(e,t,a="/"){return f3(e,t,a,!1)}function f3(e,t,a,n){let r=typeof t=="string"?Mr(t):t,s=Ia(r.pathname||"/",a);if(s==null)return null;let i=ex(e);h3(i);let o=null;for(let l=0;o==null&&l<i.length;++l){let c=k3(s);o=N3(i[l],c,n)}return o}function p3(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function ex(e,t=[],a=[],n="",r=!1){let s=(i,o,l=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&l)return;ke(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let m=gn([n,d.relativePath]),f=a.concat(d);i.children&&i.children.length>0&&(ke(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${m}".`),ex(i.children,t,f,m,l)),!(i.path==null&&!i.index)&&t.push({path:m,score:w3(m,i.index),routesMeta:f})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let l of tx(i.path))s(i,o,!0,l)}),t}function tx(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=tx(n.join("/")),o=[];return o.push(...i.map(l=>l===""?s:[s,l].join("/"))),r&&o.push(...i),o.map(l=>e.startsWith("/")&&l===""?"/":l)}function h3(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:S3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var v3=/^:[\w-]+$/,g3=3,y3=2,b3=1,x3=10,$3=-2,J0=e=>e==="*";function w3(e,t){let a=e.split("/"),n=a.length;return a.some(J0)&&(n+=$3),t&&(n+=y3),a.filter(r=>!J0(r)).reduce((r,s)=>r+(v3.test(s)?g3:s===""?b3:x3),n)}function S3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function N3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let l=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",m=Yo({path:l.relativePath,caseSensitive:l.caseSensitive,end:c},d),f=l.route;if(!m&&c&&a&&!n[n.length-1].route.index&&(m=Yo({path:l.relativePath,caseSensitive:l.caseSensitive,end:!1},d)),!m)return null;Object.assign(r,m.params),i.push({params:r,pathname:gn([s,m.pathname]),pathnameBase:E3(gn([s,m.pathnameBase])),route:f}),m.pathnameBase!=="/"&&(s=gn([s,m.pathnameBase]))}return i}function Yo(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=_3(e.path,e.caseSensitive,e.end),r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:m},f)=>{if(d==="*"){let x=o[f]||"";i=s.slice(0,s.length-x.length).replace(/(.)\/+$/,"$1")}let h=o[f];return m&&!h?c[d]=void 0:c[d]=(h||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function _3(e,t=!1,a=!0){Wt(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,l)=>(n.push({paramName:o,isOptional:l!=null}),l?"/?([^\\/]+)?":"/([^\\/]+)")).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function k3(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return Wt(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function Ia(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}function ax(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?Mr(e):e;return{pathname:a?a.startsWith("/")?a:R3(a,t):t,search:T3(n),hash:A3(r)}}function R3(e,t){let a=t.replace(/\/+$/,"").split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function mp(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function C3(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function bp(e){let t=C3(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function xp(e,t,a,n=!1){let r;typeof e=="string"?r=Mr(e):(r={...e},ke(!r.pathname||!r.pathname.includes("?"),mp("?","pathname","search",r)),ke(!r.pathname||!r.pathname.includes("#"),mp("#","pathname","hash",r)),ke(!r.search||!r.search.includes("#"),mp("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let m=t.length-1;if(!n&&i.startsWith("..")){let f=i.split("/");for(;f[0]==="..";)f.shift(),m-=1;r.pathname=f.join("/")}o=m>=0?t[m]:"/"}let l=ax(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!l.pathname.endsWith("/")&&(c||d)&&(l.pathname+="/"),l}var gn=e=>e.join("/").replace(/\/\/+/g,"/"),E3=e=>e.replace(/\/+$/,"").replace(/^\/*/,"/"),T3=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,A3=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;function nx(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}var rx=["POST","PUT","PATCH","DELETE"],j6=new Set(rx),D3=["GET",...rx],F6=new Set(D3);var B6=Symbol("ResetLoaderData");var Or=ea.createContext(null);Or.displayName="DataRouter";var ei=ea.createContext(null);ei.displayName="DataRouterState";var z6=ea.createContext(!1);var $p=ea.createContext({isTransitioning:!1});$p.displayName="ViewTransition";var sx=ea.createContext(new Map);sx.displayName="Fetchers";var M3=ea.createContext(null);M3.displayName="Await";var zt=ea.createContext(null);zt.displayName="Navigation";var ti=ea.createContext(null);ti.displayName="Location";var ta=ea.createContext({outlet:null,matches:[],isDataRoute:!1});ta.displayName="Route";var wp=ea.createContext(null);wp.displayName="RouteError";var hp=!0;function ix(e,{relative:t}={}){ke(Lr(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=X.useContext(zt),{hash:r,pathname:s,search:i}=ai(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:gn([a,s])),n.createHref({pathname:o,search:i,hash:r})}function Lr(){return X.useContext(ti)!=null}function Le(){return ke(Lr(),"useLocation() may be used only in the context of a <Router> component."),X.useContext(ti).location}var ox="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function lx(e){X.useContext(zt).static||X.useLayoutEffect(e)}function me(){let{isDataRoute:e}=X.useContext(ta);return e?I3():O3()}function O3(){ke(Lr(),"useNavigate() may be used only in the context of a <Router> component.");let e=X.useContext(Or),{basename:t,navigator:a}=X.useContext(zt),{matches:n}=X.useContext(ta),{pathname:r}=Le(),s=JSON.stringify(bp(n)),i=X.useRef(!1);return lx(()=>{i.current=!0}),X.useCallback((l,c={})=>{if(Wt(i.current,ox),!i.current)return;if(typeof l=="number"){a.go(l);return}let d=xp(l,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:gn([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var ux=X.createContext(null);function ba(){return X.useContext(ux)}function cx(e){let t=X.useContext(ta).outlet;return t&&X.createElement(ux.Provider,{value:e},t)}function st(){let{matches:e}=X.useContext(ta),t=e[e.length-1];return t?t.params:{}}function ai(e,{relative:t}={}){let{matches:a}=X.useContext(ta),{pathname:n}=Le(),r=JSON.stringify(bp(a));return X.useMemo(()=>xp(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function dx(e,t){return mx(e,t)}function mx(e,t,a,n,r){ke(Lr(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:s}=X.useContext(zt),{matches:i}=X.useContext(ta),o=i[i.length-1],l=o?o.params:{},c=o?o.pathname:"/",d=o?o.pathnameBase:"/",m=o&&o.route;if(hp){let v=m&&m.path||"";hx(c,!m||v.endsWith("*")||v.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${c}" (under <Route path="${v}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${v}"> to <Route path="${v==="/"?"*":`${v}/*`}">.`)}let f=Le(),h;if(t){let v=typeof t=="string"?Mr(t):t;ke(d==="/"||v.pathname?.startsWith(d),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${d}" but pathname "${v.pathname}" was given in the \`location\` prop.`),h=v}else h=f;let x=h.pathname||"/",y=x;if(d!=="/"){let v=d.replace(/^\//,"").split("/");y="/"+x.replace(/^\//,"").split("/").slice(v.length).join("/")}let $=yp(e,{pathname:y});hp&&(Wt(m||$!=null,`No routes matched location "${h.pathname}${h.search}${h.hash}" `),Wt($==null||$[$.length-1].route.element!==void 0||$[$.length-1].route.Component!==void 0||$[$.length-1].route.lazy!==void 0,`Matched leaf route at location "${h.pathname}${h.search}${h.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let g=F3($&&$.map(v=>Object.assign({},v,{params:Object.assign({},l,v.params),pathname:gn([d,s.encodeLocation?s.encodeLocation(v.pathname).pathname:v.pathname]),pathnameBase:v.pathnameBase==="/"?d:gn([d,s.encodeLocation?s.encodeLocation(v.pathnameBase).pathname:v.pathnameBase])})),i,a,n,r);return t&&g?X.createElement(ti.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",...h},navigationType:"POP"}},g):g}function L3(){let e=px(),t=nx(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return hp&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=X.createElement(X.Fragment,null,X.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),X.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",X.createElement("code",{style:s},"ErrorBoundary")," or"," ",X.createElement("code",{style:s},"errorElement")," prop on your route."))),X.createElement(X.Fragment,null,X.createElement("h2",null,"Unexpected Application Error!"),X.createElement("h3",{style:{fontStyle:"italic"}},t),a?X.createElement("pre",{style:r},a):null,i)}var P3=X.createElement(L3,null),U3=class extends X.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.unstable_onError?this.props.unstable_onError(e,t):console.error("React Router caught the following error during render",e)}render(){return this.state.error!==void 0?X.createElement(ta.Provider,{value:this.props.routeContext},X.createElement(wp.Provider,{value:this.state.error,children:this.props.component})):this.props.children}};function j3({routeContext:e,match:t,children:a}){let n=X.useContext(Or);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),X.createElement(ta.Provider,{value:e},a)}function F3(e,t=[],a=null,n=null,r=null){if(e==null){if(!a)return null;if(a.errors)e=a.matches;else if(t.length===0&&!a.initialized&&a.matches.length>0)e=a.matches;else return null}let s=e,i=a?.errors;if(i!=null){let c=s.findIndex(d=>d.route.id&&i?.[d.route.id]!==void 0);ke(c>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(i).join(",")}`),s=s.slice(0,Math.min(s.length,c+1))}let o=!1,l=-1;if(a)for(let c=0;c<s.length;c++){let d=s[c];if((d.route.HydrateFallback||d.route.hydrateFallbackElement)&&(l=c),d.route.id){let{loaderData:m,errors:f}=a,h=d.route.loader&&!m.hasOwnProperty(d.route.id)&&(!f||f[d.route.id]===void 0);if(d.route.lazy||h){o=!0,l>=0?s=s.slice(0,l+1):s=[s[0]];break}}}return s.reduceRight((c,d,m)=>{let f,h=!1,x=null,y=null;a&&(f=i&&d.route.id?i[d.route.id]:void 0,x=d.route.errorElement||P3,o&&(l<0&&m===0?(hx("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),h=!0,y=null):l===m&&(h=!0,y=d.route.hydrateFallbackElement||null)));let $=t.concat(s.slice(0,m+1)),g=()=>{let v;return f?v=x:h?v=y:d.route.Component?v=X.createElement(d.route.Component,null):d.route.element?v=d.route.element:v=c,X.createElement(j3,{match:d,routeContext:{outlet:c,matches:$,isDataRoute:a!=null},children:v})};return a&&(d.route.ErrorBoundary||d.route.errorElement||m===0)?X.createElement(U3,{location:a.location,revalidation:a.revalidation,component:x,error:f,children:g(),routeContext:{outlet:null,matches:$,isDataRoute:!0},unstable_onError:n}):g()},null)}function Sp(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function B3(e){let t=X.useContext(Or);return ke(t,Sp(e)),t}function Np(e){let t=X.useContext(ei);return ke(t,Sp(e)),t}function z3(e){let t=X.useContext(ta);return ke(t,Sp(e)),t}function _p(e){let t=z3(e),a=t.matches[t.matches.length-1];return ke(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function q3(){return _p("useRouteId")}function fx(){return Np("useNavigation").navigation}function kp(){let{matches:e,loaderData:t}=Np("useMatches");return X.useMemo(()=>e.map(a=>p3(a,t)),[e,t])}function px(){let e=X.useContext(wp),t=Np("useRouteError"),a=_p("useRouteError");return e!==void 0?e:t.errors?.[a]}function I3(){let{router:e}=B3("useNavigate"),t=_p("useNavigate"),a=X.useRef(!1);return lx(()=>{a.current=!0}),X.useCallback(async(r,s={})=>{Wt(a.current,ox),a.current&&(typeof r=="number"?e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var X0={};function hx(e,t,a){!t&&!X0[e]&&(X0[e]=!0,Wt(!1,a))}var q6=Re.memo(K3);function K3({routes:e,future:t,state:a,unstable_onError:n}){return mx(e,void 0,a,n,t)}function it({to:e,replace:t,state:a,relative:n}){ke(Lr(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=Re.useContext(zt);Wt(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=Re.useContext(ta),{pathname:i}=Le(),o=me(),l=xp(e,bp(s),i,n==="path"),c=JSON.stringify(l);return Re.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function Rp(e){return cx(e.context)}function ge(e){ke(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function Cp({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1}){ke(!Lr(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let i=e.replace(/^\/*/,"/"),o=Re.useMemo(()=>({basename:i,navigator:r,static:s,future:{}}),[i,r,s]);typeof a=="string"&&(a=Mr(a));let{pathname:l="/",search:c="",hash:d="",state:m=null,key:f="default"}=a,h=Re.useMemo(()=>{let x=Ia(l,i);return x==null?null:{location:{pathname:x,search:c,hash:d,state:m,key:f},navigationType:n}},[i,l,c,d,m,f,n]);return Wt(h!=null,`<Router basename="${i}"> is not able to match the URL "${l}${c}${d}" because it does not start with the basename, so the <Router> won't render anything.`),h==null?null:Re.createElement(zt.Provider,{value:o},Re.createElement(ti.Provider,{children:t,value:h}))}function Ep({children:e,location:t}){return dx(vc(e),t)}function vc(e,t=[]){let a=[];return Re.Children.forEach(e,(n,r)=>{if(!Re.isValidElement(n))return;let s=[...t,r];if(n.type===Re.Fragment){a.push.apply(a,vc(n.props.children,s));return}ke(n.type===ge,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),ke(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=vc(n.props.children,s)),a.push(i)}),a}var pc="get",hc="application/x-www-form-urlencoded";function gc(e){return e!=null&&typeof e.tagName=="string"}function H3(e){return gc(e)&&e.tagName.toLowerCase()==="button"}function Q3(e){return gc(e)&&e.tagName.toLowerCase()==="form"}function V3(e){return gc(e)&&e.tagName.toLowerCase()==="input"}function G3(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function Y3(e,t){return e.button===0&&(!t||t==="_self")&&!G3(e)}var mc=null;function J3(){if(mc===null)try{new FormData(document.createElement("form"),0),mc=!1}catch{mc=!0}return mc}var X3=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function fp(e){return e!=null&&!X3.has(e)?(Wt(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${hc}"`),null):e}function Z3(e,t){let a,n,r,s,i;if(Q3(e)){let o=e.getAttribute("action");n=o?Ia(o,t):null,a=e.getAttribute("method")||pc,r=fp(e.getAttribute("enctype"))||hc,s=new FormData(e)}else if(H3(e)||V3(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let l=e.getAttribute("formaction")||o.getAttribute("action");if(n=l?Ia(l,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||pc,r=fp(e.getAttribute("formenctype"))||fp(o.getAttribute("enctype"))||hc,s=new FormData(o,e),!J3()){let{name:c,type:d,value:m}=e;if(d==="image"){let f=c?`${c}.`:"";s.append(`${f}x`,"0"),s.append(`${f}y`,"0")}else c&&s.append(c,m)}}else{if(gc(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=pc,n=null,r=hc,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var I6=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");function Ap(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var W3=Symbol("SingleFetchRedirect");function eT(e,t,a){let n=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return n.pathname==="/"?n.pathname=`_root.${a}`:t&&Ia(n.pathname,t)==="/"?n.pathname=`${t.replace(/\/$/,"")}/_root.${a}`:n.pathname=`${n.pathname.replace(/\/$/,"")}.${a}`,n}async function tT(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function aT(e){return e!=null&&typeof e.page=="string"}function nT(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function rT(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await tT(s,a);return i.links?i.links():[]}return[]}));return lT(n.flat(1).filter(nT).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function Z0(e,t,a,n,r,s){let i=(l,c)=>a[c]?l.route.id!==a[c].route.id:!0,o=(l,c)=>a[c].pathname!==l.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==l.params["*"];return s==="assets"?t.filter((l,c)=>i(l,c)||o(l,c)):s==="data"?t.filter((l,c)=>{let d=n.routes[l.route.id];if(!d||!d.hasLoader)return!1;if(i(l,c)||o(l,c))return!0;if(l.route.shouldRevalidate){let m=l.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:l.params,defaultShouldRevalidate:!0});if(typeof m=="boolean")return m}return!0}):[]}function sT(e,t,{includeHydrateFallback:a}={}){return iT(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function iT(e){return[...new Set(e)]}function oT(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function lT(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!aT(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(oT(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function gx(){let e=ve.useContext(Or);return Ap(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function mT(){let e=ve.useContext(ei);return Ap(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var Jo=ve.createContext(void 0);Jo.displayName="FrameworkContext";function yx(){let e=ve.useContext(Jo);return Ap(e,"You must render this element inside a <HydratedRouter> element"),e}function fT(e,t){let a=ve.useContext(Jo),[n,r]=ve.useState(!1),[s,i]=ve.useState(!1),{onFocus:o,onBlur:l,onMouseEnter:c,onMouseLeave:d,onTouchStart:m}=t,f=ve.useRef(null);ve.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let y=g=>{g.forEach(v=>{i(v.isIntersecting)})},$=new IntersectionObserver(y,{threshold:.5});return f.current&&$.observe(f.current),()=>{$.disconnect()}}},[e]),ve.useEffect(()=>{if(n){let y=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(y)}}},[n]);let h=()=>{r(!0)},x=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,f,{}]:[s,f,{onFocus:Go(o,h),onBlur:Go(l,x),onMouseEnter:Go(c,h),onMouseLeave:Go(d,x),onTouchStart:Go(m,h)}]:[!1,f,{}]}function Go(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function bx({page:e,...t}){let{router:a}=gx(),n=ve.useMemo(()=>yp(a.routes,e,a.basename),[a.routes,e,a.basename]);return n?ve.createElement(hT,{page:e,matches:n,...t}):null}function pT(e){let{manifest:t,routeModules:a}=yx(),[n,r]=ve.useState([]);return ve.useEffect(()=>{let s=!1;return rT(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function hT({page:e,matches:t,...a}){let n=Le(),{manifest:r,routeModules:s}=yx(),{basename:i}=gx(),{loaderData:o,matches:l}=mT(),c=ve.useMemo(()=>Z0(e,t,l,r,n,"data"),[e,t,l,r,n]),d=ve.useMemo(()=>Z0(e,t,l,r,n,"assets"),[e,t,l,r,n]),m=ve.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let x=new Set,y=!1;if(t.forEach(g=>{let v=r.routes[g.route.id];!v||!v.hasLoader||(!c.some(b=>b.route.id===g.route.id)&&g.route.id in o&&s[g.route.id]?.shouldRevalidate||v.hasClientLoader?y=!0:x.add(g.route.id))}),x.size===0)return[];let $=eT(e,i,"data");return y&&x.size>0&&$.searchParams.set("_routes",t.filter(g=>x.has(g.route.id)).map(g=>g.route.id).join(",")),[$.pathname+$.search]},[i,o,n,r,c,t,e,s]),f=ve.useMemo(()=>sT(d,r),[d,r]),h=pT(d);return ve.createElement(ve.Fragment,null,m.map(x=>ve.createElement("link",{key:x,rel:"prefetch",as:"fetch",href:x,...a})),f.map(x=>ve.createElement("link",{key:x,rel:"modulepreload",href:x,...a})),h.map(({key:x,link:y})=>ve.createElement("link",{key:x,nonce:a.nonce,...y})))}function vT(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var xx=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{xx&&(window.__reactRouterVersion="7.9.1")}catch{}function Dp({basename:e,children:t,window:a}){let n=W.useRef();n.current==null&&(n.current=W0({window:a,v5Compat:!0}));let r=n.current,[s,i]=W.useState({action:r.action,location:r.location}),o=W.useCallback(l=>{W.startTransition(()=>i(l))},[i]);return W.useLayoutEffect(()=>r.listen(o),[r,o]),W.createElement(Cp,{basename:e,children:t,location:s.location,navigationType:s.action,navigator:r})}function $x({basename:e,children:t,history:a}){let[n,r]=W.useState({action:a.action,location:a.location}),s=W.useCallback(i=>{W.startTransition(()=>r(i))},[r]);return W.useLayoutEffect(()=>a.listen(s),[a,s]),W.createElement(Cp,{basename:e,children:t,location:n.location,navigationType:n.action,navigator:a})}$x.displayName="unstable_HistoryRouter";var wx=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i,Pr=W.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,state:o,target:l,to:c,preventScrollReset:d,viewTransition:m,...f},h){let{basename:x}=W.useContext(zt),y=typeof c=="string"&&wx.test(c),$,g=!1;if(typeof c=="string"&&y&&($=c,xx))try{let L=new URL(window.location.href),M=c.startsWith("//")?new URL(L.protocol+c):new URL(c),P=Ia(M.pathname,x);M.origin===L.origin&&P!=null?c=P+M.search+M.hash:g=!0}catch{Wt(!1,`<Link to="${c}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}let v=ix(c,{relative:r}),[b,w,N]=fT(n,f),C=kx(c,{replace:i,state:o,target:l,preventScrollReset:d,relative:r,viewTransition:m});function _(L){t&&t(L),L.defaultPrevented||C(L)}let A=W.createElement("a",{...f,...N,href:$||v,onClick:g||s?t:_,ref:vT(h,w),target:l,"data-discover":!y&&a==="render"?"true":void 0});return b&&!y?W.createElement(W.Fragment,null,A,W.createElement(bx,{page:v})):A});Pr.displayName="Link";var Ka=W.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:l,...c},d){let m=ai(i,{relative:c.relative}),f=Le(),h=W.useContext(ei),{navigator:x,basename:y}=W.useContext(zt),$=h!=null&&Tx(m)&&o===!0,g=x.encodeLocation?x.encodeLocation(m).pathname:m.pathname,v=f.pathname,b=h&&h.navigation&&h.navigation.location?h.navigation.location.pathname:null;a||(v=v.toLowerCase(),b=b?b.toLowerCase():null,g=g.toLowerCase()),b&&y&&(b=Ia(b,y)||b);let w=g!=="/"&&g.endsWith("/")?g.length-1:g.length,N=v===g||!r&&v.startsWith(g)&&v.charAt(w)==="/",C=b!=null&&(b===g||!r&&b.startsWith(g)&&b.charAt(g.length)==="/"),_={isActive:N,isPending:C,isTransitioning:$},A=N?t:void 0,L;typeof n=="function"?L=n(_):L=[n,N?"active":null,C?"pending":null,$?"transitioning":null].filter(Boolean).join(" ");let M=typeof s=="function"?s(_):s;return W.createElement(Pr,{...c,"aria-current":A,className:L,ref:d,style:M,to:i,viewTransition:o},typeof l=="function"?l(_):l)});Ka.displayName="NavLink";var Sx=W.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=pc,action:o,onSubmit:l,relative:c,preventScrollReset:d,viewTransition:m,...f},h)=>{let x=Rx(),y=Cx(o,{relative:c}),$=i.toLowerCase()==="get"?"get":"post",g=typeof o=="string"&&wx.test(o);return W.createElement("form",{ref:h,method:$,action:y,onSubmit:n?l:b=>{if(l&&l(b),b.defaultPrevented)return;b.preventDefault();let w=b.nativeEvent.submitter,N=w?.getAttribute("formmethod")||i;x(w||b.currentTarget,{fetcherKey:t,method:N,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:m})},...f,"data-discover":!g&&e==="render"?"true":void 0})});Sx.displayName="Form";function Nx({getKey:e,storageKey:t,...a}){let n=W.useContext(Jo),{basename:r}=W.useContext(zt),s=Le(),i=kp();Ex({getKey:e,storageKey:t});let o=W.useMemo(()=>{if(!n||!e)return null;let c=gp(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let l=((c,d)=>{if(!window.history.state||!window.history.state.key){let m=Math.random().toString(32).slice(2);window.history.replaceState({key:m},"")}try{let f=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof f=="number"&&window.scrollTo(0,f)}catch(m){console.error(m),sessionStorage.removeItem(c)}}).toString();return W.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${l})(${JSON.stringify(t||vp)}, ${JSON.stringify(o)})`}})}Nx.displayName="ScrollRestoration";function _x(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function Mp(e){let t=W.useContext(Or);return ke(t,_x(e)),t}function gT(e){let t=W.useContext(ei);return ke(t,_x(e)),t}function kx(e,{target:t,replace:a,state:n,preventScrollReset:r,relative:s,viewTransition:i}={}){let o=me(),l=Le(),c=ai(e,{relative:s});return W.useCallback(d=>{if(Y3(d,t)){d.preventDefault();let m=a!==void 0?a:Ws(l)===Ws(c);o(e,{replace:m,state:n,preventScrollReset:r,relative:s,viewTransition:i})}},[l,o,c,a,n,t,e,r,s,i])}var yT=0,bT=()=>`__${String(++yT)}__`;function Rx(){let{router:e}=Mp("useSubmit"),{basename:t}=W.useContext(zt),a=q3();return W.useCallback(async(n,r={})=>{let{action:s,method:i,encType:o,formData:l,body:c}=Z3(n,t);if(r.navigate===!1){let d=r.fetcherKey||bT();await e.fetch(d,a,r.action||s,{preventScrollReset:r.preventScrollReset,formData:l,body:c,formMethod:r.method||i,formEncType:r.encType||o,flushSync:r.flushSync})}else await e.navigate(r.action||s,{preventScrollReset:r.preventScrollReset,formData:l,body:c,formMethod:r.method||i,formEncType:r.encType||o,replace:r.replace,state:r.state,fromRouteId:a,flushSync:r.flushSync,viewTransition:r.viewTransition})},[e,t,a])}function Cx(e,{relative:t}={}){let{basename:a}=W.useContext(zt),n=W.useContext(ta);ke(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...ai(e||".",{relative:t})},i=Le();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),l=o.getAll("index");if(l.some(d=>d==="")){o.delete("index"),l.filter(m=>m).forEach(m=>o.append("index",m));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:gn([a,s.pathname])),Ws(s)}var vp="react-router-scroll-positions",fc={};function gp(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:Ia(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function Ex({getKey:e,storageKey:t}={}){let{router:a}=Mp("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=gT("useScrollRestoration"),{basename:s}=W.useContext(zt),i=Le(),o=kp(),l=fx();W.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),xT(W.useCallback(()=>{if(l.state==="idle"){let c=gp(i,o,s,e);fc[c]=window.scrollY}try{sessionStorage.setItem(t||vp,JSON.stringify(fc))}catch(c){Wt(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[l.state,e,s,i,o,t])),typeof document<"u"&&(W.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||vp);c&&(fc=JSON.parse(c))}catch{}},[t]),W.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(fc,()=>window.scrollY,e?(d,m)=>gp(d,m,s,e):void 0);return()=>c&&c()},[a,s,e]),W.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{Wt(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function xT(e,t){let{capture:a}=t||{};W.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function Tx(e,{relative:t}={}){let a=W.useContext($p);ke(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=Mp("useViewTransitionState"),r=ai(e,{relative:t});if(!a.isTransitioning)return!1;let s=Ia(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=Ia(a.nextLocation.pathname,n)||a.nextLocation.pathname;return Yo(r.pathname,i)!=null||Yo(r.pathname,s)!=null}var Rt=new Dd({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var Op="ironclaw_token",qe="/api/webchat/v2",Ur=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function xa(){return sessionStorage.getItem(Op)||""}function ni(e){e?sessionStorage.setItem(Op,e):sessionStorage.removeItem(Op)}function yc(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function Mx(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function Dx(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function Ox({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=Dx(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=Dx(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function H(e,t={}){let a=xa(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await Mx(r);throw new Ur(Ox({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function bc(){return H(`${qe}/session`)}function xc({clientActionId:e,requestedThreadId:t,projectId:a}={}){let n={client_action_id:e||yc()};return t&&(n.requested_thread_id=t),a&&(n.project_id=a),H(`${qe}/threads`,{method:"POST",body:JSON.stringify(n)})}function Lx({limit:e,cursor:t}={}){let a=new URL(`${qe}/threads`,window.location.origin);return e!=null&&a.searchParams.set("limit",String(e)),t&&a.searchParams.set("cursor",t),H(a.pathname+a.search)}function Px({threadId:e}={}){return e?H(`${qe}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function Lp(e){return`${qe}/threads/${encodeURIComponent(e)}/files`}function Ux({threadId:e,path:t}={}){if(!e)return Promise.reject(new Error("threadId is required"));let a=new URL(Lp(e),window.location.origin);return t&&a.searchParams.set("path",t),H(a.pathname+a.search)}function jx({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${Lp(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),H(a.pathname+a.search)}function $c({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${Lp(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function Fx({limit:e,runLimit:t,includeCompleted:a}={}){let n=new URLSearchParams;e!=null&&n.set("limit",String(e)),t!=null&&n.set("run_limit",String(t)),a===!0&&n.set("include_completed","true");let r=n.toString();return H(`${qe}/automations${r?`?${r}`:""}`)}function Bx({automationId:e}={}){return e?H(`${qe}/automations/${encodeURIComponent(e)}/pause`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function zx({automationId:e}={}){return e?H(`${qe}/automations/${encodeURIComponent(e)}/resume`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function qx({automationId:e}={}){return e?H(`${qe}/automations/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("automationId is required"))}var Ix=`${qe}/projects`;function $T(e){return`${Ix}/${encodeURIComponent(e)}`}function Kx({limit:e}={}){let t=new URL(Ix,window.location.origin);return e!=null&&t.searchParams.set("limit",String(e)),H(t.pathname+t.search)}function Hx({projectId:e}={}){return e?H($T(e)):Promise.reject(new Error("projectId is required"))}function Qx(){return H(`${qe}/outbound/preferences`)}function Vx(){return H(`${qe}/outbound/targets`)}function Gx({finalReplyTargetId:e}={}){return H(`${qe}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function Pp({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:l,source:c,tail:d,follow:m}={}){let f=new URL(`${qe}/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),l&&f.searchParams.set("tool_name",l),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),H(f.pathname+f.search)}function Yx({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:l,source:c,tail:d,follow:m}={}){let f=new URL(`${qe}/operator/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),l&&f.searchParams.set("tool_name",l),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),H(f.pathname+f.search)}function Jx({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||yc(),content:t};return a.length>0&&(r.attachments=a),H(`${qe}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function Xx({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${qe}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),H(n.pathname+n.search)}function Zx({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${qe}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function _a(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new Ur("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=xa(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await Mx(r);throw new Ur(Ox({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function Up(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function wc(e){return Up(await _a(e))}function Wx({threadId:e,afterCursor:t}={}){let a=new URL(`${qe}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=xa();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function e$({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||yc()};return a&&(r.reason=a),H(`${qe}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function jp({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let l={client_action_id:i||yc(),resolution:n};return r!=null&&(l.always=r),s&&(l.credential_ref=s),H(`${qe}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(l)})}function t$({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return H("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function a$(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),H(`${qe}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function ri(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function n$(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function r$(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new Ur("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new Ur("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function s$(){let e=xa();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var Sc="anon",i$=Sc;function o$(e){i$=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:Sc}function wt(){return i$}var l$="ironclaw:v2-thread-pins:",Fp=new Set,yn=new Set,Bp=null;function zp(){return`${l$}${wt()}`}function wT(){try{let e=window.localStorage.getItem(zp());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function ST(){try{yn.size===0?window.localStorage.removeItem(zp()):window.localStorage.setItem(zp(),JSON.stringify([...yn]))}catch{}}function u$(){let e=wt();if(e!==Bp){yn.clear();for(let t of wT())yn.add(t);Bp=e}}function c$(){return new Set(yn)}function d$(){let e=c$();for(let t of Fp)try{t(e)}catch{}}function m$(e){e&&(u$(),yn.has(e)?yn.delete(e):yn.add(e),ST(),d$())}function f$(){return u$(),c$()}function p$(e){return Fp.add(e),()=>{Fp.delete(e)}}function h$(){yn.clear(),Bp=wt();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(l$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}d$()}var NT=0,jr={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function qp(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function v$(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":_T(t)?"text":"download"}function _T(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function Xo(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function kT(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function RT(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function CT(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function g$(e,{limits:t,existing:a=[],t:n}){let r=t||jr,s=[],i=[],o=a.length,l=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!kT(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:Xo(r.maxFileBytes)}));continue}if(l+c.size>r.maxTotalBytes){let y=n("chat.attachmentTotalTooLarge",{max:Xo(r.maxTotalBytes)});i.includes(y)||i.push(y);continue}let d;try{d=await RT(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:m,base64:f}=CT(d,c.type),h=m||"application/octet-stream",x=qp(h);s.push({id:`staged-${NT++}`,filename:c.name||"attachment",mimeType:h,kind:x,sizeBytes:c.size,sizeLabel:Xo(c.size),dataBase64:f,previewUrl:x==="image"?d:null}),o+=1,l+=c.size}return{staged:s,errors:i}}function y$(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function b$(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}function ET(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||qp(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?Zx({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?Xo(n.size_bytes):"",preview_url:null,fetch_url:s}})}function $$(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let c=MT(s);if(!c)continue;let d=`tool-${c.invocationId}`;if(n.has(d))continue;n.add(d),r.push({id:d,role:"tool_activity",...c,timestamp:x$(s)||c.updatedAt||null,sequence:s.sequence,activityOrder:c.activityOrder,activityOrderSource:c.activityOrderSource,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=DT(s),l=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy");r.push({id:i,role:o,content:s.content||"",attachments:ET(s,a),timestamp:x$(s),kind:s.kind,status:l?"error":s.status,...l&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:AT(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=TT(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function TT(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function AT(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function DT(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function x$(e){return e.received_at||e.created_at||null}function MT(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:Ip(t)}var OT="gate_declined";function Ip(e){let t=e.status==="failed"||e.status==="killed",a=e.error_kind||null,n=N$(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Wo(e.title||e.capability_id)||"tool",toolStatus:S$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(w$(a)||e.output_summary||e.output_preview||e.result_ref)||null,toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null,activityOrder:n,activityOrderSource:Number.isFinite(n)?"projection":null}}function Kp(e){let t=N$(e.activity_order),a=e.error_kind||null;return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Wo(e.capability_id)||"tool",toolStatus:S$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:null,toolError:w$(a),toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null,activityOrder:t,activityOrderSource:Number.isFinite(t)?"projection":null}}function w$(e){return e||null}function Zo(e){return e==="success"||e==="error"||e==="declined"}function Wo(e){let t=typeof e=="string"?e.trim():"";if(!t)return"";let a=t.split(".");return a[a.length-1]||t}function S$(e,t=null){if(t===OT)return"declined";switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}function N$(e){let t=Number(e);return Number.isFinite(t)?t:null}var LT=50,Ha=new Map,PT=30;function el(e,t){for(Ha.delete(e),Ha.set(e,t);Ha.size>PT;){let a=Ha.keys().next().value;Ha.delete(a)}}function tl(e){return`${wt()}:${e}`}function k$(){Ha.clear()}function R$(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?Ha.get(tl(e)):null,[s,i]=p.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),o=p.default.useRef(new Set),l=p.default.useRef(e);l.current=e;let c=p.default.useCallback(async(m,f={})=>{let{preserveClientOnly:h=!1,finalReplyTimestampByRun:x=null}=f;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(o.current.has(e))return;o.current.add(e);let y=wt(),$=tl(e);i(g=>({...g,isLoading:!0}));try{let g=await Xx({threadId:e,limit:LT,cursor:m});if(wt()!==y)return;let v=m?[]:a?.()||[],b=$$(g.messages||[],v,e),w=g.next_cursor||null;if(m||n?.([]),!m){let N=Ha.get($)?.messages||[],C=_$(b,N,{preserveClientOnly:h,finalReplyTimestampByRun:x});el($,{messages:C,nextCursor:w})}i(N=>{if(l.current!==e)return N;let C;return m?C=UT(b,N.messages):C=_$(b,N.messages,{preserveClientOnly:h,finalReplyTimestampByRun:x}),el($,{messages:C,nextCursor:w}),{messages:C,nextCursor:w,isLoading:!1,loadError:null}})}catch(g){if(console.error("Failed to load timeline:",g),wt()!==y)return;i(v=>l.current===e?{...v,isLoading:!1,loadError:"Failed to load conversation history."}:v)}finally{o.current.delete(e)}},[e,a,n]);p.default.useEffect(()=>{let m=e?Ha.get(tl(e)):null;i({messages:m?.messages||[],nextCursor:m?.nextCursor||null,isLoading:!!e&&!m,loadError:null}),e&&c()},[e,c]);let d=p.default.useCallback((m,f)=>{if(!m)return;let h=tl(m),x=g=>typeof f=="function"?f(g||[]):f;if(l.current===m){i(g=>{let v=x(g.messages||[]);return el(h,{messages:v,nextCursor:g.nextCursor||null}),{...g,messages:v}});return}let y=Ha.get(h)||{messages:[],nextCursor:null},$=x(y.messages||[]);el(h,{messages:$,nextCursor:y.nextCursor||null})},[]);return{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:c,seedThreadMessages:d,setMessages:m=>i(f=>{let h=typeof m=="function"?m(f.messages):m;return e&&el(tl(e),{messages:h,nextCursor:f.nextCursor}),{...f,messages:h}})}}function UT(e,t){let a=new Set(t.map(n=>n?.id).filter(Boolean));return[...e.filter(n=>!a.has(n?.id)),...t]}function _$(e,t,a={}){let{preserveClientOnly:n=!1,finalReplyTimestampByRun:r=null}=a,s=qT(e,t,{finalReplyTimestampByRun:r}),i=new Set(s.map(l=>l?.id).filter(Boolean)),o=t.filter(l=>!l||typeof l.id!="string"||i.has(l.id)?!1:C$(l)?!0:typeof l.timelineMessageId=="string"&&i.has(`msg-${l.timelineMessageId}`)?!1:zT(l)?!0:n&&l.id.startsWith("err-"));return jT(s,o)}function jT(e,t){if(t.length===0)return e;let a=new Map;for(let i=0;i<e.length;i+=1){let o=Hp(e[i]);o&&a.set(o,i)}let n=new Map,r=[];for(let i of t){let o=FT(i)?Hp(i):null;if(o&&a.has(o)){let l=n.get(o)||[];l.push(i),n.set(o,l)}else r.push(i)}if(n.size===0)return[...e,...r];let s=[];for(let i=0;i<e.length;i+=1){let o=e[i];s.push(o);let l=Hp(o);l&&a.get(l)===i&&s.push(...n.get(l)||[])}return r.length>0?[...s,...r]:s}function FT(e){return C$(e)||BT(e)}function BT(e){return e?.role==="error"&&typeof e.id=="string"&&e.id.startsWith("err-")}function Hp(e){return typeof e?.turnRunId=="string"&&e.turnRunId?e.turnRunId:null}function zT(e){return e?.isOptimistic===!0&&typeof e.id=="string"&&e.id.startsWith("pending-")&&(e.role==="user"||e.role==="assistant")}function qT(e,t,a={}){let{finalReplyTimestampByRun:n=null}=a,r=new Map,s=new Map;for(let i of t||[])!i||!i.timestamp||(typeof i.id=="string"&&r.set(i.id,i),typeof i.timelineMessageId=="string"&&r.set(`msg-${i.timelineMessageId}`,i),Qp(i)&&typeof i.turnRunId=="string"&&s.set(i.turnRunId,i));return r.size===0&&s.size===0&&!n?e:e.map(i=>{if(!i||i.timestamp||typeof i.id!="string")return i;let o=typeof i.turnRunId=="string"?i.turnRunId:null,l=r.get(i.id)||(Qp(i)&&o?s.get(o):null),c=Qp(i)&&o?n?.[o]:null,d=l?.timestamp||c;return d?{...i,timestamp:d}:i})}function Qp(e){return e?.role==="assistant"&&e?.isFinalReply===!0}function C$(e){return e?.role==="tool_activity"||e?.role==="thinking"}var nl="__new__",E$="ironclaw:v2-draft:";function si(e){return`${E$}${wt()}:${e||nl}`}function Vp(e){try{return window.localStorage.getItem(si(e))||""}catch{return""}}function Gp(e,t){try{t?window.localStorage.setItem(si(e),t):window.localStorage.removeItem(si(e))}catch{}}function T$(e){Gp(e,"")}var al=new Map;function Yp(e){return al.get(si(e))||[]}function A$(e,t){let a=si(e);t&&t.length>0?al.set(a,t):al.delete(a)}function D$(e){al.delete(si(e))}function M$(){al.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(E$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function IT(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function KT(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function HT(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=IT(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?KT(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),xa()?"":(ni(n),n)}function QT(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var VT={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function GT(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),VT[t]||"Could not complete sign-in. Please try again."):""}function O$(){let[e,t]=p.default.useState(()=>HT()||xa()),[a,n]=p.default.useState(()=>GT()),[r]=p.default.useState(()=>QT()),[s,i]=p.default.useState(null),[o,l]=p.default.useState(()=>!!(r&&!xa())),[c,d]=p.default.useState(()=>!!xa());p.default.useEffect(()=>{if(!r||xa()){l(!1);return}let x=!1;return r$(r).then(y=>{x||(ni(y),d(!0),t(y),i(null),n(""),l(!1),Rt.clear())}).catch(()=>{x||(n("Could not complete sign-in. Please try again."),l(!1))}),()=>{x=!0}},[r]),p.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let x=!1;return d(!0),bc().then(y=>{x||(i(y),d(!1))}).catch(y=>{x||(i(null),d(!1),(y?.status===401||y?.status===403)&&(ni(""),t(""),n("Your session expired. Please sign in again."),Rt.clear()))}),()=>{x=!0}},[e,o]),o$(s);let m=p.default.useRef(null);p.default.useEffect(()=>{let x=wt();m.current&&m.current!==Sc&&m.current!==x&&(k$(),M$(),h$()),m.current=x},[s]);let f=p.default.useCallback(x=>{ni(x),d(!!x),t(x),i(null),n(""),Rt.clear()},[]),h=p.default.useCallback(()=>{s$().catch(()=>{}),ni(""),d(!1),t(""),i(null),n(""),Rt.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,rebornProjectsEnabled:!!s?.features?.reborn_projects,signIn:f,signOut:h}}var Fr="/chat",rl=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace"},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"logs",path:"/logs",labelKey:"nav.logs",hidden:!0},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var YT=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"tools",labelKey:"settings.tools",icon:"tool"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],JT=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],XT=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],Nc={settings:YT,extensions:JT,admin:XT};var L$="ironclaw:v2-theme";function ZT(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(L$);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function _c(){let[e,t]=p.default.useState(ZT);p.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(L$,e)}catch{}},[e]);let a=p.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function P$(e){return K({enabled:!!e,queryKey:["gateway-status",e],queryFn:ri,refetchInterval:3e4})}var WT="/api/webchat/v2/operator/config",kc="/api/webchat/v2/settings/tools",ii="agent.auto_approve_tools",U$="tool.",eA=new Set(["always_allow","ask_each_time","disabled"]),tA=new Set(["default","always_allow","ask_each_time","disabled"]);function j$(e){return e==="ask"?"ask_each_time":eA.has(e)?e:"ask_each_time"}function aA(e){return e==="ask"?"ask_each_time":tA.has(e)?e:"default"}function nA(e){return["default","global","override"].includes(e)?e:"default"}function F$(e){if(!e?.key?.startsWith(U$))return null;let t=e.value||{};return{name:t.name||e.key.slice(U$.length),description:t.description||"",state:j$(t.state),default_state:j$(t.default_state),locked:!!(t.locked||e.mutable===!1),effective_source:nA(t.effective_source||e.source)}}function rA(e){let t={};for(let a of e.entries||[])a?.key===ii&&(t[ii]=!!a.value);return t}async function B$(){let e=await H(kc);return{settings:rA(e),diagnostics:e.diagnostics||[],precedence:e.precedence||[]}}async function Jp(e,t){if(e===ii){let n=await H(kc,{method:"POST",body:JSON.stringify({enabled:!!t})});return{success:!0,entry:n.entry,value:n.entry?.value}}let a=await H(`${WT}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({value:t})});return{success:!0,entry:a.entry,value:a.entry?.value}}async function z$(e){let t=e?.settings||{},a=[];return Object.prototype.hasOwnProperty.call(t,ii)&&a.push(await Jp(ii,!!t[ii])),{success:!0,imported:a.length,results:a}}function Rc(){return H("/api/webchat/v2/llm/providers")}function q$(e){return H("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function I$(e){return H(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function sl(e){return H("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function K$(e){return H("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function H$(e){return H("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function Q$(e){return H("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function V$(e){return H("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function G$(){return H("/api/webchat/v2/llm/codex/login",{method:"POST"})}async function Y$(){let e=await H(kc);return{tools:(e.entries||[]).map(F$).filter(Boolean),diagnostics:e.diagnostics||[]}}async function J$(e,t){let a=aA(t),n=await H(`${kc}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({state:a})});return{success:!0,tool:F$(n.entry),entry:n.entry}}function X$(){return H("/api/webchat/v2/extensions")}function Z$(){return H("/api/webchat/v2/extensions/registry")}function W$(){return H("/api/webchat/v2/skills")}function ew(e){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function tw(e){return H("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function aw(e,t){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function nw(e){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function rw(e,t){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}/auto-activate`,{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:t})})}function sw(e){return H("/api/webchat/v2/skills/auto-activate-learned",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function iw(){return H("/api/webchat/v2/traces/credit")}function ow(e){return H(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function lw(){return Promise.resolve({users:[],todo:!0})}function uw(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function cw(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var Xp="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",Zp=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function il(e){return Zp.find(t=>t.value===e)?.label||e}function oi(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function dw(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function Cc(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function mw(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function Br(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Xp||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?oi(e,t).trim().length>0:!0:!1}function sA(e,t,a){return e.id===a?"active":Br(e,t)?"ready":"setup"}function fw(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=sA(r,t,a);n[s]&&n[s].push(r)}return n}function Ec(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Xp||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!oi(e,t).trim()?"base_url":"ok"}function Wp(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===Xp&&(i.api_key=void 0),i}function pw(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function hw(e){return/^[a-z0-9_-]+$/.test(e)}function vw(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var iA=Object.freeze({});function li({settings:e,gatewayStatus:t,enabled:a=!0}){let n=J(),r=K({queryKey:["llm-providers"],queryFn:Rc,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=iA,l=(s.providers||[]).map(w=>({...w,name:w.description,has_api_key:w.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,m=d||"nearai",f=s.active?.model||t?.llm_model||"",h=l.filter(w=>w.builtin),x=l.filter(w=>!w.builtin),y=[...l].sort((w,N)=>w.id===d?-1:N.id===d?1:(w.name||w.id).localeCompare(N.name||N.id)),$=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=Q({mutationFn:async w=>{if(!Br(w,o)){let C=Ec(w,o);throw new Error(C==="base_url"?"base_url":"api_key")}let N=Cc(w,o);if(!N)throw new Error("model");return await sl({provider_id:w.id,model:N}),w},onSuccess:$}),v=Q({mutationFn:async({provider:w,form:N,apiKey:C,editingProvider:_})=>{let A=!!w?.builtin,M={id:(A?w.id:N.id.trim()).trim(),name:A?w.name||w.id:N.name.trim(),adapter:A?w.adapter:N.adapter,base_url:N.baseUrl.trim()||w?.base_url||"",default_model:N.model.trim()||void 0};return C.trim()&&(M.api_key=C.trim()),(_||w)?.id===m&&M.default_model&&(M.set_active=!0,M.model=M.default_model),await q$(M),M},onSuccess:$}),b=Q({mutationFn:async w=>(await I$(w.id),w),onSuccess:$});return{providers:y,builtinProviders:h,customProviders:x,builtinOverrides:o,activeProviderId:d,selectedModel:f,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:w=>g.mutateAsync(w),saveCustomProvider:w=>v.mutateAsync(w),saveBuiltinProvider:w=>v.mutateAsync(w),deleteCustomProvider:w=>b.mutateAsync(w),testConnection:K$,listModels:H$,isBusy:g.isPending||v.isPending||b.isPending}}function gw({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}var yw="ironclaw:v2-sidebar-open";function bw(){return typeof window>"u"?null:window}function xw(){try{return bw()?.localStorage||null}catch{return null}}function $w(e=xw()){try{return e?.getItem(yw)!=="false"}catch{return!0}}function ww(e,t=xw()){try{t?.setItem(yw,e?"true":"false")}catch{}}function Sw(e=bw()){try{return e?.matchMedia?.("(min-width: 768px)").matches===!0}catch{return!1}}function Nw(e,t){return t?{...e,desktopOpen:!e.desktopOpen}:{...e,mobileOpen:!e.mobileOpen}}function _w(e,t){return t?e.desktopOpen:e.mobileOpen}function kw({onNewChat:e}={}){let t=me(),[a,n]=p.default.useState(()=>({mobileOpen:!1,desktopOpen:$w()})),[r,s]=p.default.useState(()=>Sw());p.default.useEffect(()=>{let d=window.matchMedia("(min-width: 768px)"),m=()=>s(d.matches);return m(),d.addEventListener?.("change",m),()=>d.removeEventListener?.("change",m)},[]),p.default.useEffect(()=>{ww(a.desktopOpen)},[a.desktopOpen]);let i=p.default.useCallback(()=>{n(d=>({...d,mobileOpen:!1}))},[]),o=p.default.useCallback(()=>{n(d=>Nw(d,r))},[r]),l=p.default.useCallback(async()=>{let d=await e?.(),m=typeof d=="string"&&d.length>0?d:null;t(m?`/chat/${m}`:"/chat"),i()},[t,i,e]),c=p.default.useCallback(d=>{t(`/chat/${d}`),i()},[t,i]);return{mobileOpen:a.mobileOpen,desktopOpen:a.desktopOpen,currentOpen:_w(a,r),close:i,toggle:o,newChat:l,selectThread:c}}var eh=new Set,oA=0;function ui(e,t={}){let a={id:++oA,message:e,tone:t.tone||"info",duration:t.duration??2600};return eh.forEach(n=>n(a)),a.id}function Rw(e){return eh.add(e),()=>eh.delete(e)}function lA(e){return e?.status===409&&e?.payload?.kind==="busy"}function Cw(e,t){return lA(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function Ew(){let e=K({queryKey:["threads"],queryFn:()=>Lx({})}),[t,a]=p.default.useState(null),[n,r]=p.default.useState(!1),s=p.default.useRef(new Map),i=p.default.useCallback(async c=>{let d=c||"__global__",m=s.current.get(d);if(m)return m;r(!0);let f=(async()=>{try{let h=await xc(c?{projectId:c}:void 0);Rt.invalidateQueries({queryKey:["threads"]});let x=h?.thread?.thread_id;return x&&a(x),x}finally{s.current.delete(d),r(s.current.size>0)}})();return s.current.set(d,f),f},[]),o=p.default.useCallback(async c=>{await Px({threadId:c}),t===c&&a(null),Rt.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:p.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var Tw={attach:u`<path
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
    />`,arrowDown:u`<path d="M12 5v14" /><path d="m6 13 6 6 6-6" />`,retry:u`<path d="M3.5 12a8.5 8.5 0 1 1 2.6 6.1" /><path d="M3.2 18.5v-5h5" />`};function O({name:e,className:t="",strokeWidth:a=1.7}){return u`
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
      ${Tw[e]||Tw.spark}
    </svg>
  `}function V(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=V(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function Aw(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function uA(e){return Aw(e).trim().charAt(0).toUpperCase()||"I"}function cA(){let[e,t]=p.default.useState(!1),a=p.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function Dw({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=R(),s=cA(),i=Aw(a),o=a?.email||a?.role||r("common.gatewaySession");return u`
    <div
      className="relative flex items-center gap-2 border-t border-[var(--v2-panel-border)] px-3 py-3"
    >
      ${s.open&&u`
        <div
          className=${V("absolute bottom-full left-3 right-3 mb-2 rounded-[10px] border p-3 shadow-lg","border-[var(--v2-panel-border)] bg-[var(--v2-surface)]")}
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
            />`:u`<span className="place-self-center">${uA(a)}</span>`}
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
        <${O} name=${e==="dark"?"sun":"moon"} className="h-4 w-4" />
      </button>
      <button
        onClick=${n}
        className="grid h-8 w-8 shrink-0 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
        title=${r("header.signOut")}
      >
        <${O} name="logout" className="h-4 w-4" />
      </button>
    </div>
  `}var Mw={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",logs:"list",settings:"settings",admin:"shield"},dA=rl.filter(e=>e.id!=="chat"&&!e.hidden);function mA({route:e,label:t,onNavigate:a}){return u`
    <${Ka}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${O} name=${Mw[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function fA({route:e,label:t,subRoutes:a,onNavigate:n}){let r=R(),s=Le(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return u`
    <div className="flex flex-col">
      <${Ka}
        to=${o}
        onClick=${n}
        className=${()=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${O}
          name=${Mw[e.id]||"bolt"}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1 truncate">${t}</span>
        <${O}
          name="chevron"
          className=${V("h-3.5 w-3.5 shrink-0 transition-transform duration-150",i&&"rotate-180")}
        />
      <//>

      ${i&&u`
        <div className="mt-0.5 flex flex-col gap-0.5 pl-3">
          ${a.map(l=>u`
              <${Ka}
                key=${l.id}
                to=${e.path+"/"+l.id}
                onClick=${n}
                className=${({isActive:c})=>V("flex items-center gap-2.5 rounded-[8px] py-1.5 pl-7 pr-3 text-[12px] font-medium",c?"text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
              >
                <${O} name=${l.icon} className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate">${r(l.labelKey)}</span>
              <//>
            `)}
        </div>
      `}
    </div>
  `}function Ow({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=R(),s=p.default.useMemo(()=>dA.filter(i=>a||i.id!=="admin"),[a]);return u`
    <div className="flex flex-col px-3 py-2">
      <button
        data-testid="new-chat"
        onClick=${e}
        disabled=${t}
        className=${V("flex items-center gap-2.5 rounded-[10px] px-3 py-2","border border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]","bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]","hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)] disabled:opacity-50")}
      >
        <${O} name="plus" className="h-4 w-4 shrink-0" />
        <span className="text-[13px] font-medium">
          ${r(t?"chat.creating":"chat.newThread")}
        </span>
      </button>

      <nav className="mt-2 flex flex-col gap-1">
        ${s.map(i=>{let o=(Nc[i.id]||[]).filter(l=>a||!(i.id==="settings"&&["users","inference"].includes(l.id)));return o.length>0?u`
              <${fA}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:u`
            <${mA}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var bn=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),ol=new Set([bn.NEEDS_ATTENTION,bn.FAILED]),th="ironclaw:v2-thread-attention",ah=new Set,ci=new Map;function pA(){try{let e=window.localStorage.getItem(th);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&ol.has(a[1])):[]}catch{return[]}}function Lw(){let e=[];for(let[t,a]of ci)ol.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(th):window.localStorage.setItem(th,JSON.stringify(e))}catch{}}for(let[e,t]of pA())ci.set(e,t);function Uw(){return new Map(ci)}function Pw(){let e=Uw();for(let t of ah)try{t(e)}catch{}}function Tc(e,t){if(!e)return;let a=ci.get(e);if(t==null){if(!ci.delete(e))return;ol.has(a)&&Lw(),Pw();return}a!==t&&(ci.set(e,t),(ol.has(t)||ol.has(a))&&Lw(),Pw())}function jw(e){Tc(e,null)}function hA(){return Uw()}function vA(e){return ah.add(e),()=>{ah.delete(e)}}function Fw(){let[e,t]=p.default.useState(hA);return p.default.useEffect(()=>vA(t),[]),e}function Ac(e){return e.updated_at||e.created_at||null}function nh(e,t){let a=Ac(e)||"",n=Ac(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function Bw(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function zw(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function gA(){let[e,t]=p.default.useState(f$);return p.default.useEffect(()=>p$(t),[]),e}var yA=Object.freeze({[bn.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-transparent"},[bn.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[bn.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function bA(e){return e&&yA[e]||null}function xA({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=R(),o=Ac(e),l=Bw(o),c=zw(o),d=p.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(h=>{window.alert(h?.message||"Unable to delete chat")})},[s,e.id]),m=p.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),m$(e.id)},[e.id]);return u`
    <div
      className=${V("group flex w-full items-stretch rounded-[8px] border-l-2",n?n.borderClass:t?"border-[var(--v2-accent)]":"border-transparent",t?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
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
            className=${V("h-1.5 w-1.5 shrink-0 rounded-full",n.dotClass)}
          />`}
        </div>
        ${(n||l)&&u`<span
          className=${V("block truncate text-[11px]",n?n.textClass:"text-[var(--v2-text-faint)]")}
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
        className=${V("my-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] transition",a?"text-[var(--v2-accent-text)]":"opacity-0 text-[var(--v2-text-faint)] group-hover:opacity-100 focus:opacity-100","hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-accent-text)]")}
      >
        <${O} name="pin" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>
      ${s&&u`<button
        type="button"
        onClick=${d}
        title=${i("common.deleteChat")}
        aria-label=${i("common.deleteChat")}
        className=${V("my-1 mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px]","opacity-0 transition group-hover:opacity-100 focus:opacity-100","text-[var(--v2-text-faint)] hover:bg-[var(--v2-danger-soft)] hover:text-[var(--v2-danger-text)]")}
      >
        <${O} name="trash" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>`}
    </div>
  `}function qw({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:u`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>u`
          <${xA}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${bA(n.get(o.id))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function Iw({threads:e,activeThreadId:t,rebornProjectsEnabled:a=!1,onSelect:n,onDelete:r,onNavigate:s}){let[i,o]=p.default.useState(!1),[l,c]=p.default.useState(""),d=Fw(),m=gA(),f=R(),{pinned:h,recent:x,totalMatches:y}=p.default.useMemo(()=>{let $=l.trim().toLowerCase(),g=$?e.filter(w=>(w.title||w.id||"").toLowerCase().includes($)):e,v=[],b=[];for(let w of g)m.has(w.id)?v.push(w):b.push(w);return v.sort(nh),b.sort(nh),{pinned:v,recent:b,totalMatches:v.length+b.length}},[e,l,m]);return u`
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
        <${O}
          name="chevron"
          className=${V("h-3.5 w-3.5 text-[var(--v2-text-faint)]",i?"-rotate-90":"")}
          strokeWidth=${2.2}
        />
      </button>

      ${!i&&u`
        ${e.length>0&&u`<div className="relative mb-1 mt-1 px-1">
          <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--v2-text-faint)]">
            <${O} name="search" className="h-3.5 w-3.5" />
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
          <${Ka}
            to="/projects"
            onClick=${s}
            className=${({isActive:$})=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",$?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
          >
            <${O} name="folder" className="h-4 w-4 shrink-0" />
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

          <${qw}
            label=${f("common.pinned")}
            items=${h}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${m}
            onSelect=${n}
            onDelete=${r}
          />
          <${qw}
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
  `}function Dc(){let e=J(),t=K({queryKey:["trace-credits"],queryFn:iw,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=Q({mutationFn:ow,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function $A(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function Kw(){let e=R(),{credits:t}=Dc();if(!t||!t.enrolled)return null;let a=$A(t.final_credit),n=t.submissions_accepted||0,r=t.submissions_submitted||0,s=t.manual_review_hold_count||0;return u`
    <div className="px-3 pb-1">
      <${Pr}
        to="/settings/traces"
        className="block rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2.5 transition-colors hover:border-[var(--v2-accent-soft)] hover:bg-[var(--v2-surface-muted)]"
      >
        <div className="flex items-center gap-2 text-[var(--v2-accent-text)]">
          <${O} name="layers" className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 truncate font-mono text-[11px] uppercase tracking-[0.14em]">
            ${e("settings.traceCommons")}
          </span>
        </div>
        <div className="mt-2 flex items-center justify-between gap-2">
          <span className="text-xs text-[var(--v2-text-muted)]">${e("traceCommons.finalCredit")}</span>
          <span className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${a}</span>
        </div>
        <div className="mt-0.5 text-[11px] text-[var(--v2-text-muted)]">
          ${e("traceCommons.cardAccepted",{accepted:n,submitted:r})}
        </div>
        ${s>0&&u`
          <div className="mt-1 text-[11px] font-medium text-[var(--v2-accent-text)]">
            ${e("traceCommons.cardHeld",{count:s})}
          </div>
        `}
      <//>
    </div>
  `}function Hw({id:e,threadsState:t,theme:a,toggleTheme:n,profile:r,isAdmin:s,rebornProjectsEnabled:i=!1,onSignOut:o,onClose:l,onNewChat:c,onSelectThread:d,onDeleteThread:m}){return u`
    <aside
      id=${e}
      className="flex h-full w-[260px] shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
    >
      <div className="flex items-center gap-2.5 px-4 py-5">
        <${Pr}
          to="/chat"
          onClick=${l}
          className="flex items-center gap-2.5 opacity-90 hover:opacity-100"
          aria-label="IronClaw"
        >
          <img src="/v2/assets/logo.jpg" alt="IronClaw" className="h-7 w-auto" />
        <//>
      </div>

      <${Ow}
        onNewChat=${c}
        isCreating=${t.isCreating}
        isAdmin=${s}
        onNavigate=${l}
      />

      <${Kw} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${Iw}
          threads=${t.threads}
          activeThreadId=${t.activeThreadId}
          rebornProjectsEnabled=${i}
          onSelect=${d}
          onDelete=${m}
          onNavigate=${l}
        />
      </div>

      <${Dw}
        theme=${a}
        toggleTheme=${n}
        profile=${r}
        onSignOut=${o}
      />
    </aside>
  `}var wA="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",SA="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",Qw="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",Vw={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},Gw={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function D({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=Vw[n]??Vw.md,l=r?"w-full":"";if(a==="primary")return u`
      <${s}
        style=${{background:wA,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${V(Qw,o,l,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:SA}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=Gw[a]??Gw.outline;return u`
    <${s}
      className=${V(Qw,o,l,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function Yw(){let e=p.default.useMemo(()=>NA(window.location),[]),[t,a]=p.default.useState(null),[n,r]=p.default.useState(null),[s,i]=p.default.useState(!1),[o,l]=p.default.useState(""),[c,d]=p.default.useState(!1);p.default.useEffect(()=>{if(!e)return;let h=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:h.signal}).then(x=>{if(!x.ok)throw new Error(String(x.status));return x.json()}).then(a).catch(()=>{h.signal.aborted||a(null)}),()=>h.abort()},[e]);let m=p.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),l("");try{let h=await fetch(`${e.base}/attestation/report`);if(!h.ok)throw new Error(String(h.status));let x=await h.json();return r(x),x}catch(h){return l(h.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),f=p.default.useCallback(async()=>{let h=n||await m();return!h||!navigator.clipboard?!1:(await navigator.clipboard.writeText(JSON.stringify({...h,instance_attestation:t},null,2)),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[m,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:m,copyReport:f}}function NA(e){let t=e.hostname;if(!t||t==="localhost"||_A(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function _A(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}var kA=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function Jw(){let e=R(),t=Yw(),[a,n]=p.default.useState(!1),r=p.default.useCallback(()=>{n(o=>{let l=!o;return l&&t.loadReport(),l})},[t]),s=p.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=RA({teeInfo:t.teeInfo,report:t.report,t:e});return u`
    <div className="relative">
      <button
        type="button"
        onClick=${r}
        aria-expanded=${a}
        title=${e("tee.title")}
        className=${V("grid h-8 w-8 place-items-center rounded-[8px]","border border-[color-mix(in_srgb,var(--v2-positive-text)_28%,transparent)]","bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]","hover:border-[color-mix(in_srgb,var(--v2-positive-text)_52%,transparent)]")}
      >
        <${O} name="shield" className="h-4 w-4" />
      </button>

      ${a&&u`
        <div
          className=${V("absolute right-0 top-full z-40 mt-2 w-[min(22rem,calc(100vw-2rem))]","rounded-[14px] border border-[var(--v2-panel-border)]","bg-[var(--v2-surface)] p-3 shadow-[0_18px_48px_rgba(0,0,0,0.35)]")}
        >
          <div className="flex items-center gap-2">
            <span className="grid h-8 w-8 place-items-center rounded-[10px] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]">
              <${O} name="shield" className="h-4 w-4" />
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
            <${D}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${t.reportLoading}
              onClick=${s}
            >
              <${O} name="check" className="h-4 w-4" />
              ${t.copied?e("tee.copied"):e("tee.copyReport")}
            <//>
          </div>
        </div>
      `}
    </div>
  `}function RA({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return kA.map(([r,s])=>({label:a(s),value:CA(n[r])||a("common.unknown")}))}function CA(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var EA="https://docs.ironclaw.com";function Xw({threadsState:e,onToggleSidebar:t,sidebarOpen:a=!0}){let n=R(),r=Le(),s=p.default.useMemo(()=>{for(let o of rl){let l=Nc[o.id];if(!l)continue;let c=o.path+"/";if(r.pathname.startsWith(c)){let d=r.pathname.slice(c.length).split("/")[0],m=l.find(f=>f.id===d);if(m)return{parent:n(o.labelKey),current:n(m.labelKey)}}}return null},[r.pathname,n]),i=p.default.useMemo(()=>{if(s)return null;if(r.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(c=>c.id===e.activeThreadId)?.title||n("nav.chat");let o=rl.find(l=>r.pathname.startsWith(l.path));return o?n(o.labelKey):""},[r.pathname,e.activeThreadId,e.threads,n,s]);return u`
    <header
      className=${V("flex h-14 shrink-0 items-center gap-3 px-4","border-b border-[var(--v2-panel-border)]","bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] backdrop-blur-xl")}
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
        <${O} name="list" className="h-4 w-4" />
      </button>

      ${s?u`
            <div className="flex min-w-0 items-center gap-2 text-[14px] font-semibold">
              <span className="shrink-0 text-[var(--v2-text-muted)]">
                ${s.parent}
              </span>
              <${O}
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
        <${Jw} />
        <${Ka}
          to="/logs"
          className=${({isActive:o})=>V("inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",o&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
          title=${n("nav.logs")}
        >
          ${n("nav.logs")}
        <//>
        <a
          href=${EA}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${n("nav.docs")}
        >
          ${n("nav.docs")}
        </a>
      </div>
    </header>
  `}function Zw({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=me(),i=R(),[o,l]=p.default.useState(""),[c,d]=p.default.useState(0),m=p.default.useRef(null),f=p.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(b=>({id:`thread-${b.id}`,label:b.title||`Thread ${b.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${b.id}`)}));return[...g,...v]},[a,s,n,r]),h=p.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?f.filter(v=>v.label.toLowerCase().includes(g)):f},[f,o]);p.default.useEffect(()=>{if(!e)return;l(""),d(0);let g=window.requestAnimationFrame(()=>m.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),p.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,h.length-1)))},[h.length]);let x=p.default.useCallback(g=>{g&&(t(),g.run())},[t]),y=p.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,h.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),x(h[c])):g.key==="Escape"&&(g.preventDefault(),t())},[h,c,x,t]);if(!e)return null;let $=null;return u`
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh]" role="dialog" aria-modal="true" aria-label="Command palette">
      <button type="button" aria-label="Close" onClick=${t} className="absolute inset-0 bg-black/50"></button>
      <div className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_30px_60px_-20px_rgba(0,0,0,0.8)]">
        <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-3">
          <${O} name="search" className="h-4 w-4 text-[var(--v2-text-faint)]" />
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
                  <${O} name=${g.icon} className="h-4 w-4 shrink-0" />
                  <span className="min-w-0 truncate">${g.label}</span>
                </button>
              </li>
            `})}
        </ul>
      </div>
    </div>
  `}var Ww={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},TA={info:"bolt",success:"check",error:"close"};function e1(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>Rw(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:u`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${e.map(a=>u`
          <div
            key=${a.id}
            role="status"
            className=${["pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",Ww[a.tone]||Ww.info].join(" ")}
          >
            <${O} name=${TA[a.tone]||"bolt"} className="h-4 w-4 shrink-0" />
            <span>${a.message}</span>
          </div>
        `)}
    </div>
  `}function t1({token:e,profile:t,isChecking:a=!1,isAdmin:n,rebornProjectsEnabled:r=!1,onSignOut:s}){let i=R(),{theme:o,toggleTheme:l}=_c(),c=P$(e),d=Ew(),m=kw({onNewChat:()=>d.setActiveThreadId(null)}),f=c.data,h=Le(),x=me(),y=li({settings:{},gatewayStatus:f,enabled:n}),$=n&&gw({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),g=h.pathname==="/welcome"||h.pathname.startsWith("/settings"),[v,b]=p.default.useState(!1);p.default.useEffect(()=>{let N=C=>{(C.metaKey||C.ctrlKey)&&C.key.toLowerCase()==="k"&&(C.preventDefault(),b(_=>!_))};return window.addEventListener("keydown",N),()=>window.removeEventListener("keydown",N)},[]);let w=p.default.useCallback(async N=>{let C=d.activeThreadId===N;try{await d.deleteThread(N),C&&x("/chat",{replace:!0})}catch(_){console.error("Failed to delete thread:",_),ui(Cw(_,i),{tone:"error"})}},[x,d,i]);return $&&!g?u`<${it} to="/welcome" replace />`:u`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${m.mobileOpen&&u`<button
        type="button"
        aria-label=${i("nav.close")}
        onClick=${m.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${V("fixed inset-y-0 left-0 z-50 md:relative md:z-auto",m.mobileOpen?"flex":"hidden",m.desktopOpen?"md:flex":"md:hidden")}
      >
        <${Hw}
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
        <${Xw}
          threadsState=${d}
          onToggleSidebar=${m.toggle}
          sidebarOpen=${m.currentOpen}
        />
        <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
          ${c.error&&u`
            <div
              className=${V("m-4 rounded-[14px] border px-4 py-3 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >
              ${c.error.message||i("error.gatewayConnection")}
            </div>
          `}
          <${Rp}
            context=${{gatewayStatus:f,gatewayStatusQuery:c,currentUser:t,isChecking:a,isAdmin:n,threadsState:d}}
          />
        </main>
      </div>
      <${Zw}
        open=${v}
        onClose=${()=>b(!1)}
        threadsState=${d}
        onNewChat=${m.newChat}
        onToggleTheme=${l}
      />
      <${e1} />
    </div>
  `}var qt=Fe(Ie(),1),ml=e=>e.type==="checkbox",zr=e=>e instanceof Date,Ct=e=>e==null,p1=e=>typeof e=="object",Qe=e=>!Ct(e)&&!Array.isArray(e)&&p1(e)&&!zr(e),AA=e=>Qe(e)&&e.target?ml(e.target)?e.target.checked:e.target.value:e,DA=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,MA=(e,t)=>e.has(DA(t)),OA=e=>{let t=e.constructor&&e.constructor.prototype;return Qe(t)&&t.hasOwnProperty("isPrototypeOf")},ih=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function dt(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(ih&&(e instanceof Blob||n))&&(a||Qe(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!OA(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=dt(e[r]));else return e;return t}var Uc=e=>/^\w*$/.test(e),Ze=e=>e===void 0,oh=e=>Array.isArray(e)?e.filter(Boolean):[],lh=e=>oh(e.replace(/["|']|\]/g,"").split(/\.|\[/)),Y=(e,t,a)=>{if(!t||!Qe(e))return a;let n=(Uc(t)?[t]:lh(t)).reduce((r,s)=>Ct(r)?r:r[s],e);return Ze(n)||n===e?Ze(e[t])?a:e[t]:n},Qa=e=>typeof e=="boolean",Pe=(e,t,a)=>{let n=-1,r=Uc(t)?[t]:lh(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],l=a;if(n!==i){let c=e[o];l=Qe(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=l,e=e[o]}},a1={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},ka={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},xn={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},LA=qt.default.createContext(null);LA.displayName="HookFormContext";var PA=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==ka.all&&(t._proxyFormState[i]=!n||ka.all),a&&(a[i]=!0),e[i]}});return r},UA=typeof window<"u"?qt.default.useLayoutEffect:qt.default.useEffect;var Va=e=>typeof e=="string",jA=(e,t,a,n,r)=>Va(e)?(n&&t.watch.add(e),Y(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),Y(a,s))):(n&&(t.watchAll=!0),a),sh=e=>Ct(e)||!p1(e);function tr(e,t,a=new WeakSet){if(sh(e)||sh(t))return e===t;if(zr(e)&&zr(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(zr(i)&&zr(o)||Qe(i)&&Qe(o)||Array.isArray(i)&&Array.isArray(o)?!tr(i,o,a):i!==o)return!1}}return!0}var FA=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},cl=e=>Array.isArray(e)?e:[e],n1=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},It=e=>Qe(e)&&!Object.keys(e).length,uh=e=>e.type==="file",Ra=e=>typeof e=="function",Oc=e=>{if(!ih)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},h1=e=>e.type==="select-multiple",ch=e=>e.type==="radio",BA=e=>ch(e)||ml(e),rh=e=>Oc(e)&&e.isConnected;function zA(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=Ze(e)?n++:e[t[n++]];return e}function qA(e){for(let t in e)if(e.hasOwnProperty(t)&&!Ze(e[t]))return!1;return!0}function Xe(e,t){let a=Array.isArray(t)?t:Uc(t)?[t]:lh(t),n=a.length===1?e:zA(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Qe(n)&&It(n)||Array.isArray(n)&&qA(n))&&Xe(e,a.slice(0,-1)),e}var v1=e=>{for(let t in e)if(Ra(e[t]))return!0;return!1};function Lc(e,t={}){let a=Array.isArray(e);if(Qe(e)||a)for(let n in e)Array.isArray(e[n])||Qe(e[n])&&!v1(e[n])?(t[n]=Array.isArray(e[n])?[]:{},Lc(e[n],t[n])):Ct(e[n])||(t[n]=!0);return t}function g1(e,t,a){let n=Array.isArray(e);if(Qe(e)||n)for(let r in e)Array.isArray(e[r])||Qe(e[r])&&!v1(e[r])?Ze(t)||sh(a[r])?a[r]=Array.isArray(e[r])?Lc(e[r],[]):{...Lc(e[r])}:g1(e[r],Ct(t)?{}:t[r],a[r]):a[r]=!tr(e[r],t[r]);return a}var ll=(e,t)=>g1(e,t,Lc(t)),r1={value:!1,isValid:!1},s1={value:!0,isValid:!0},y1=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!Ze(e[0].attributes.value)?Ze(e[0].value)||e[0].value===""?s1:{value:e[0].value,isValid:!0}:s1:r1}return r1},b1=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>Ze(e)?e:t?e===""?NaN:e&&+e:a&&Va(e)?new Date(e):n?n(e):e,i1={isValid:!1,value:null},x1=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,i1):i1;function o1(e){let t=e.ref;return uh(t)?t.files:ch(t)?x1(e.refs).value:h1(t)?[...t.selectedOptions].map(({value:a})=>a):ml(t)?y1(e.refs).value:b1(Ze(t.value)?e.ref.value:t.value,e)}var IA=(e,t,a,n)=>{let r={};for(let s of e){let i=Y(t,s);i&&Pe(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},Pc=e=>e instanceof RegExp,ul=e=>Ze(e)?e:Pc(e)?e.source:Qe(e)?Pc(e.value)?e.value.source:e.value:e,l1=e=>({isOnSubmit:!e||e===ka.onSubmit,isOnBlur:e===ka.onBlur,isOnChange:e===ka.onChange,isOnAll:e===ka.all,isOnTouch:e===ka.onTouched}),u1="AsyncFunction",KA=e=>!!e&&!!e.validate&&!!(Ra(e.validate)&&e.validate.constructor.name===u1||Qe(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===u1)),HA=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),c1=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),dl=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=Y(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(dl(o,t))break}else if(Qe(o)&&dl(o,t))break}}};function d1(e,t,a){let n=Y(e,a);if(n||Uc(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=Y(t,s),o=Y(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var QA=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return It(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||ka.all))},VA=(e,t,a)=>!e||!t||e===t||cl(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),GA=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,YA=(e,t)=>!oh(Y(e,t)).length&&Xe(e,t),JA=(e,t,a)=>{let n=cl(Y(e,a));return Pe(n,"root",t[a]),Pe(e,a,n),e},Mc=e=>Va(e);function m1(e,t,a="validate"){if(Mc(e)||Array.isArray(e)&&e.every(Mc)||Qa(e)&&!e)return{type:a,message:Mc(e)?e:"",ref:t}}var di=e=>Qe(e)&&!Pc(e)?e:{value:e,message:""},f1=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:l,maxLength:c,minLength:d,min:m,max:f,pattern:h,validate:x,name:y,valueAsNumber:$,mount:g}=e._f,v=Y(a,y);if(!g||t.has(y))return{};let b=o?o[0]:i,w=k=>{r&&b.reportValidity&&(b.setCustomValidity(Qa(k)?"":k||""),b.reportValidity())},N={},C=ch(i),_=ml(i),A=C||_,L=($||uh(i))&&Ze(i.value)&&Ze(v)||Oc(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,M=FA.bind(null,y,n,N),P=(k,z,Z,ne=xn.maxLength,de=xn.minLength)=>{let fe=k?z:Z;N[y]={type:k?ne:de,message:fe,ref:i,...M(k?ne:de,fe)}};if(s?!Array.isArray(v)||!v.length:l&&(!A&&(L||Ct(v))||Qa(v)&&!v||_&&!y1(o).isValid||C&&!x1(o).isValid)){let{value:k,message:z}=Mc(l)?{value:!!l,message:l}:di(l);if(k&&(N[y]={type:xn.required,message:z,ref:b,...M(xn.required,z)},!n))return w(z),N}if(!L&&(!Ct(m)||!Ct(f))){let k,z,Z=di(f),ne=di(m);if(!Ct(v)&&!isNaN(v)){let de=i.valueAsNumber||v&&+v;Ct(Z.value)||(k=de>Z.value),Ct(ne.value)||(z=de<ne.value)}else{let de=i.valueAsDate||new Date(v),fe=gt=>new Date(new Date().toDateString()+" "+gt),Ce=i.type=="time",Ue=i.type=="week";Va(Z.value)&&v&&(k=Ce?fe(v)>fe(Z.value):Ue?v>Z.value:de>new Date(Z.value)),Va(ne.value)&&v&&(z=Ce?fe(v)<fe(ne.value):Ue?v<ne.value:de<new Date(ne.value))}if((k||z)&&(P(!!k,Z.message,ne.message,xn.max,xn.min),!n))return w(N[y].message),N}if((c||d)&&!L&&(Va(v)||s&&Array.isArray(v))){let k=di(c),z=di(d),Z=!Ct(k.value)&&v.length>+k.value,ne=!Ct(z.value)&&v.length<+z.value;if((Z||ne)&&(P(Z,k.message,z.message),!n))return w(N[y].message),N}if(h&&!L&&Va(v)){let{value:k,message:z}=di(h);if(Pc(k)&&!v.match(k)&&(N[y]={type:xn.pattern,message:z,ref:i,...M(xn.pattern,z)},!n))return w(z),N}if(x){if(Ra(x)){let k=await x(v,a),z=m1(k,b);if(z&&(N[y]={...z,...M(xn.validate,z.message)},!n))return w(z.message),N}else if(Qe(x)){let k={};for(let z in x){if(!It(k)&&!n)break;let Z=m1(await x[z](v,a),b,z);Z&&(k={...Z,...M(z,Z.message)},w(Z.message),n&&(N[y]=k))}if(!It(k)&&(N[y]={ref:b,...k},!n))return N}}return w(!0),N},XA={mode:ka.onSubmit,reValidateMode:ka.onChange,shouldFocusError:!0};function ZA(e={}){let t={...XA,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:Ra(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Qe(t.defaultValues)||Qe(t.values)?dt(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:dt(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},l,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},m={...d},f={array:n1(),state:n1()},h=t.criteriaMode===ka.all,x=S=>T=>{clearTimeout(c),c=setTimeout(S,T)},y=async S=>{if(!t.disabled&&(d.isValid||m.isValid||S)){let T=t.resolver?It((await _()).errors):await L(n,!0);T!==a.isValid&&f.state.next({isValid:T})}},$=(S,T)=>{!t.disabled&&(d.isValidating||d.validatingFields||m.isValidating||m.validatingFields)&&((S||Array.from(o.mount)).forEach(E=>{E&&(T?Pe(a.validatingFields,E,T):Xe(a.validatingFields,E))}),f.state.next({validatingFields:a.validatingFields,isValidating:!It(a.validatingFields)}))},g=(S,T=[],E,F,B=!0,j=!0)=>{if(F&&E&&!t.disabled){if(i.action=!0,j&&Array.isArray(Y(n,S))){let G=E(Y(n,S),F.argA,F.argB);B&&Pe(n,S,G)}if(j&&Array.isArray(Y(a.errors,S))){let G=E(Y(a.errors,S),F.argA,F.argB);B&&Pe(a.errors,S,G),YA(a.errors,S)}if((d.touchedFields||m.touchedFields)&&j&&Array.isArray(Y(a.touchedFields,S))){let G=E(Y(a.touchedFields,S),F.argA,F.argB);B&&Pe(a.touchedFields,S,G)}(d.dirtyFields||m.dirtyFields)&&(a.dirtyFields=ll(r,s)),f.state.next({name:S,isDirty:P(S,T),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else Pe(s,S,T)},v=(S,T)=>{Pe(a.errors,S,T),f.state.next({errors:a.errors})},b=S=>{a.errors=S,f.state.next({errors:a.errors,isValid:!1})},w=(S,T,E,F)=>{let B=Y(n,S);if(B){let j=Y(s,S,Ze(E)?Y(r,S):E);Ze(j)||F&&F.defaultChecked||T?Pe(s,S,T?j:o1(B._f)):Z(S,j),i.mount&&y()}},N=(S,T,E,F,B)=>{let j=!1,G=!1,ie={name:S};if(!t.disabled){if(!E||F){(d.isDirty||m.isDirty)&&(G=a.isDirty,a.isDirty=ie.isDirty=P(),j=G!==ie.isDirty);let $e=tr(Y(r,S),T);G=!!Y(a.dirtyFields,S),$e?Xe(a.dirtyFields,S):Pe(a.dirtyFields,S,!0),ie.dirtyFields=a.dirtyFields,j=j||(d.dirtyFields||m.dirtyFields)&&G!==!$e}if(E){let $e=Y(a.touchedFields,S);$e||(Pe(a.touchedFields,S,E),ie.touchedFields=a.touchedFields,j=j||(d.touchedFields||m.touchedFields)&&$e!==E)}j&&B&&f.state.next(ie)}return j?ie:{}},C=(S,T,E,F)=>{let B=Y(a.errors,S),j=(d.isValid||m.isValid)&&Qa(T)&&a.isValid!==T;if(t.delayError&&E?(l=x(()=>v(S,E)),l(t.delayError)):(clearTimeout(c),l=null,E?Pe(a.errors,S,E):Xe(a.errors,S)),(E?!tr(B,E):B)||!It(F)||j){let G={...F,...j&&Qa(T)?{isValid:T}:{},errors:a.errors,name:S};a={...a,...G},f.state.next(G)}},_=async S=>{$(S,!0);let T=await t.resolver(s,t.context,IA(S||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return $(S),T},A=async S=>{let{errors:T}=await _(S);if(S)for(let E of S){let F=Y(T,E);F?Pe(a.errors,E,F):Xe(a.errors,E)}else a.errors=T;return T},L=async(S,T,E={valid:!0})=>{for(let F in S){let B=S[F];if(B){let{_f:j,...G}=B;if(j){let ie=o.array.has(j.name),$e=B._f&&KA(B._f);$e&&d.validatingFields&&$([F],!0);let Dt=await f1(B,o.disabled,s,h,t.shouldUseNativeValidation&&!T,ie);if($e&&d.validatingFields&&$([F]),Dt[j.name]&&(E.valid=!1,T))break;!T&&(Y(Dt,j.name)?ie?JA(a.errors,Dt,j.name):Pe(a.errors,j.name,Dt[j.name]):Xe(a.errors,j.name))}!It(G)&&await L(G,T,E)}}return E.valid},M=()=>{for(let S of o.unMount){let T=Y(n,S);T&&(T._f.refs?T._f.refs.every(E=>!rh(E)):!rh(T._f.ref))&&yt(S)}o.unMount=new Set},P=(S,T)=>!t.disabled&&(S&&T&&Pe(s,S,T),!tr(gt(),r)),k=(S,T,E)=>jA(S,o,{...i.mount?s:Ze(T)?r:Va(S)?{[S]:T}:T},E,T),z=S=>oh(Y(i.mount?s:r,S,t.shouldUnregister?Y(r,S,[]):[])),Z=(S,T,E={})=>{let F=Y(n,S),B=T;if(F){let j=F._f;j&&(!j.disabled&&Pe(s,S,b1(T,j)),B=Oc(j.ref)&&Ct(T)?"":T,h1(j.ref)?[...j.ref.options].forEach(G=>G.selected=B.includes(G.value)):j.refs?ml(j.ref)?j.refs.forEach(G=>{(!G.defaultChecked||!G.disabled)&&(Array.isArray(B)?G.checked=!!B.find(ie=>ie===G.value):G.checked=B===G.value||!!B)}):j.refs.forEach(G=>G.checked=G.value===B):uh(j.ref)?j.ref.value="":(j.ref.value=B,j.ref.type||f.state.next({name:S,values:dt(s)})))}(E.shouldDirty||E.shouldTouch)&&N(S,B,E.shouldTouch,E.shouldDirty,!0),E.shouldValidate&&Ue(S)},ne=(S,T,E)=>{for(let F in T){if(!T.hasOwnProperty(F))return;let B=T[F],j=S+"."+F,G=Y(n,j);(o.array.has(S)||Qe(B)||G&&!G._f)&&!zr(B)?ne(j,B,E):Z(j,B,E)}},de=(S,T,E={})=>{let F=Y(n,S),B=o.array.has(S),j=dt(T);Pe(s,S,j),B?(f.array.next({name:S,values:dt(s)}),(d.isDirty||d.dirtyFields||m.isDirty||m.dirtyFields)&&E.shouldDirty&&f.state.next({name:S,dirtyFields:ll(r,s),isDirty:P(S,j)})):F&&!F._f&&!Ct(j)?ne(S,j,E):Z(S,j,E),c1(S,o)&&f.state.next({...a,name:S}),f.state.next({name:i.mount?S:void 0,values:dt(s)})},fe=async S=>{i.mount=!0;let T=S.target,E=T.name,F=!0,B=Y(n,E),j=$e=>{F=Number.isNaN($e)||zr($e)&&isNaN($e.getTime())||tr($e,Y(s,E,$e))},G=l1(t.mode),ie=l1(t.reValidateMode);if(B){let $e,Dt,Ee=T.type?o1(B._f):AA(S),la=S.type===a1.BLUR||S.type===a1.FOCUS_OUT,Xr=!HA(B._f)&&!t.resolver&&!Y(a.errors,E)&&!B._f.deps||GA(la,Y(a.touchedFields,E),a.isSubmitted,ie,G),Zr=c1(E,o,la);Pe(s,E,Ee),la?(!T||!T.readOnly)&&(B._f.onBlur&&B._f.onBlur(S),l&&l(0)):B._f.onChange&&B._f.onChange(S);let Ja=N(E,Ee,la),cr=!It(Ja)||Zr;if(!la&&f.state.next({name:E,type:S.type,values:dt(s)}),Xr)return(d.isValid||m.isValid)&&(t.mode==="onBlur"?la&&y():la||y()),cr&&f.state.next({name:E,...Zr?{}:Ja});if(!la&&Zr&&f.state.next({...a}),t.resolver){let{errors:dr}=await _([E]);if(j(Ee),F){let Wr=d1(a.errors,n,E),es=d1(dr,n,Wr.name||E);$e=es.error,E=es.name,Dt=It(dr)}}else $([E],!0),$e=(await f1(B,o.disabled,s,h,t.shouldUseNativeValidation))[E],$([E]),j(Ee),F&&($e?Dt=!1:(d.isValid||m.isValid)&&(Dt=await L(n,!0)));F&&(B._f.deps&&Ue(B._f.deps),C(E,Dt,$e,Ja))}},Ce=(S,T)=>{if(Y(a.errors,T)&&S.focus)return S.focus(),1},Ue=async(S,T={})=>{let E,F,B=cl(S);if(t.resolver){let j=await A(Ze(S)?S:B);E=It(j),F=S?!B.some(G=>Y(j,G)):E}else S?(F=(await Promise.all(B.map(async j=>{let G=Y(n,j);return await L(G&&G._f?{[j]:G}:G)}))).every(Boolean),!(!F&&!a.isValid)&&y()):F=E=await L(n);return f.state.next({...!Va(S)||(d.isValid||m.isValid)&&E!==a.isValid?{}:{name:S},...t.resolver||!S?{isValid:E}:{},errors:a.errors}),T.shouldFocus&&!F&&dl(n,Ce,S?B:o.mount),F},gt=S=>{let T={...i.mount?s:r};return Ze(S)?T:Va(S)?Y(T,S):S.map(E=>Y(T,E))},mt=(S,T)=>({invalid:!!Y((T||a).errors,S),isDirty:!!Y((T||a).dirtyFields,S),error:Y((T||a).errors,S),isValidating:!!Y(a.validatingFields,S),isTouched:!!Y((T||a).touchedFields,S)}),tt=S=>{S&&cl(S).forEach(T=>Xe(a.errors,T)),f.state.next({errors:S?a.errors:{}})},Ve=(S,T,E)=>{let F=(Y(n,S,{_f:{}})._f||{}).ref,B=Y(a.errors,S)||{},{ref:j,message:G,type:ie,...$e}=B;Pe(a.errors,S,{...$e,...T,ref:F}),f.state.next({name:S,errors:a.errors,isValid:!1}),E&&E.shouldFocus&&F&&F.focus&&F.focus()},ft=(S,T)=>Ra(S)?f.state.subscribe({next:E=>"values"in E&&S(k(void 0,T),E)}):k(S,T,!0),Tt=S=>f.state.subscribe({next:T=>{VA(S.name,T.name,S.exact)&&QA(T,S.formState||d,Ai,S.reRenderRoot)&&S.callback({values:{...s},...a,...T,defaultValues:r})}}).unsubscribe,sa=S=>(i.mount=!0,m={...m,...S.formState},Tt({...S,formState:m})),yt=(S,T={})=>{for(let E of S?cl(S):o.mount)o.mount.delete(E),o.array.delete(E),T.keepValue||(Xe(n,E),Xe(s,E)),!T.keepError&&Xe(a.errors,E),!T.keepDirty&&Xe(a.dirtyFields,E),!T.keepTouched&&Xe(a.touchedFields,E),!T.keepIsValidating&&Xe(a.validatingFields,E),!t.shouldUnregister&&!T.keepDefaultValue&&Xe(r,E);f.state.next({values:dt(s)}),f.state.next({...a,...T.keepDirty?{isDirty:P()}:{}}),!T.keepIsValid&&y()},Ea=({disabled:S,name:T})=>{(Qa(S)&&i.mount||S||o.disabled.has(T))&&(S?o.disabled.add(T):o.disabled.delete(T))},Ge=(S,T={})=>{let E=Y(n,S),F=Qa(T.disabled)||Qa(t.disabled);return Pe(n,S,{...E||{},_f:{...E&&E._f?E._f:{ref:{name:S}},name:S,mount:!0,...T}}),o.mount.add(S),E?Ea({disabled:Qa(T.disabled)?T.disabled:t.disabled,name:S}):w(S,!0,T.value),{...F?{disabled:T.disabled||t.disabled}:{},...t.progressive?{required:!!T.required,min:ul(T.min),max:ul(T.max),minLength:ul(T.minLength),maxLength:ul(T.maxLength),pattern:ul(T.pattern)}:{},name:S,onChange:fe,onBlur:fe,ref:B=>{if(B){Ge(S,T),E=Y(n,S);let j=Ze(B.value)&&B.querySelectorAll&&B.querySelectorAll("input,select,textarea")[0]||B,G=BA(j),ie=E._f.refs||[];if(G?ie.find($e=>$e===j):j===E._f.ref)return;Pe(n,S,{_f:{...E._f,...G?{refs:[...ie.filter(rh),j,...Array.isArray(Y(r,S))?[{}]:[]],ref:{type:j.type,name:S}}:{ref:j}}}),w(S,!1,void 0,j)}else E=Y(n,S,{}),E._f&&(E._f.mount=!1),(t.shouldUnregister||T.shouldUnregister)&&!(MA(o.array,S)&&i.action)&&o.unMount.add(S)}}},At=()=>t.shouldFocusError&&dl(n,Ce,o.mount),Ta=S=>{Qa(S)&&(f.state.next({disabled:S}),dl(n,(T,E)=>{let F=Y(n,E);F&&(T.disabled=F._f.disabled||S,Array.isArray(F._f.refs)&&F._f.refs.forEach(B=>{B.disabled=F._f.disabled||S}))},0,!1))},ia=(S,T)=>async E=>{let F;E&&(E.preventDefault&&E.preventDefault(),E.persist&&E.persist());let B=dt(s);if(f.state.next({isSubmitting:!0}),t.resolver){let{errors:j,values:G}=await _();a.errors=j,B=dt(G)}else await L(n);if(o.disabled.size)for(let j of o.disabled)Xe(B,j);if(Xe(a.errors,"root"),It(a.errors)){f.state.next({errors:{}});try{await S(B,E)}catch(j){F=j}}else T&&await T({...a.errors},E),At(),setTimeout(At);if(f.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:It(a.errors)&&!F,submitCount:a.submitCount+1,errors:a.errors}),F)throw F},oa=(S,T={})=>{Y(n,S)&&(Ze(T.defaultValue)?de(S,dt(Y(r,S))):(de(S,T.defaultValue),Pe(r,S,dt(T.defaultValue))),T.keepTouched||Xe(a.touchedFields,S),T.keepDirty||(Xe(a.dirtyFields,S),a.isDirty=T.defaultValue?P(S,dt(Y(r,S))):P()),T.keepError||(Xe(a.errors,S),d.isValid&&y()),f.state.next({...a}))},Gr=(S,T={})=>{let E=S?dt(S):r,F=dt(E),B=It(S),j=B?r:F;if(T.keepDefaultValues||(r=E),!T.keepValues){if(T.keepDirtyValues){let G=new Set([...o.mount,...Object.keys(ll(r,s))]);for(let ie of Array.from(G))Y(a.dirtyFields,ie)?Pe(j,ie,Y(s,ie)):de(ie,Y(j,ie))}else{if(ih&&Ze(S))for(let G of o.mount){let ie=Y(n,G);if(ie&&ie._f){let $e=Array.isArray(ie._f.refs)?ie._f.refs[0]:ie._f.ref;if(Oc($e)){let Dt=$e.closest("form");if(Dt){Dt.reset();break}}}}if(T.keepFieldsRef)for(let G of o.mount)de(G,Y(j,G));else n={}}s=t.shouldUnregister?T.keepDefaultValues?dt(r):{}:dt(j),f.array.next({values:{...j}}),f.state.next({values:{...j}})}o={mount:T.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!T.keepIsValid||!!T.keepDirtyValues,i.watch=!!t.shouldUnregister,f.state.next({submitCount:T.keepSubmitCount?a.submitCount:0,isDirty:B?!1:T.keepDirty?a.isDirty:!!(T.keepDefaultValues&&!tr(S,r)),isSubmitted:T.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:B?{}:T.keepDirtyValues?T.keepDefaultValues&&s?ll(r,s):a.dirtyFields:T.keepDefaultValues&&S?ll(r,S):T.keepDirty?a.dirtyFields:{},touchedFields:T.keepTouched?a.touchedFields:{},errors:T.keepErrors?a.errors:{},isSubmitSuccessful:T.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},Yr=(S,T)=>Gr(Ra(S)?S(s):S,T),ur=(S,T={})=>{let E=Y(n,S),F=E&&E._f;if(F){let B=F.refs?F.refs[0]:F.ref;B.focus&&(B.focus(),T.shouldSelect&&Ra(B.select)&&B.select())}},Ai=S=>{a={...a,...S}},ae={control:{register:Ge,unregister:yt,getFieldState:mt,handleSubmit:ia,setError:Ve,_subscribe:Tt,_runSchema:_,_focusError:At,_getWatch:k,_getDirty:P,_setValid:y,_setFieldArray:g,_setDisabledField:Ea,_setErrors:b,_getFieldArray:z,_reset:Gr,_resetDefaultValues:()=>Ra(t.defaultValues)&&t.defaultValues().then(S=>{Yr(S,t.resetOptions),f.state.next({isLoading:!1})}),_removeUnmounted:M,_disableForm:Ta,_subjects:f,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(S){i=S},get _defaultValues(){return r},get _names(){return o},set _names(S){o=S},get _formState(){return a},get _options(){return t},set _options(S){t={...t,...S}}},subscribe:sa,trigger:Ue,register:Ge,handleSubmit:ia,watch:ft,setValue:de,getValues:gt,reset:Yr,resetField:oa,clearErrors:tt,unregister:yt,setError:Ve,setFocus:ur,getFieldState:mt};return{...ae,formControl:ae}}function $1(e={}){let t=qt.default.useRef(void 0),a=qt.default.useRef(void 0),[n,r]=qt.default.useState({isDirty:!1,isValidating:!1,isLoading:Ra(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:Ra(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!Ra(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=ZA(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,UA(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),qt.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),qt.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),qt.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),qt.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),qt.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),qt.default.useEffect(()=>{e.values&&!tr(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),qt.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=PA(n,s),t.current}var w1={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},S1={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},WA={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function ee({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return u`
    <${s}
      className=${V(w1[a]??w1.default,S1[n]??S1.md,WA[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var dh="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",jc={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function Et({className:e="",size:t="md",error:a=!1,...n}){return u`
    <input
      className=${V(dh,jc[t]??jc.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Fc({className:e="",error:t=!1,rows:a=4,...n}){return u`
    <textarea
      rows=${a}
      className=${V(dh,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function mh({children:e,className:t="",size:a="md",error:n=!1,...r}){return u`
    <div className="relative w-full">
      <select
        className=${V(dh,jc[a]??jc.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
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
  `}function e4({children:e,className:t="",required:a=!1,...n}){return u`
    <label
      className=${V("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&u`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function $n({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return u`
    <div className=${V("flex flex-col gap-2",s)}>
      ${e&&u`<${e4} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&u`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&u`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var t4={google:"Google",github:"GitHub",apple:"Apple"};function a4(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function N1({providers:e,redirectAfter:t}){let a=R();return e.length?u`
    <div className="mt-6 space-y-3">
      <div className="flex items-center gap-3 text-[11px] uppercase text-[var(--v2-text-faint)]">
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
        <span>${a("login.oauthDivider")}</span>
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
      </div>
      <div className="grid gap-2">
        ${e.map(n=>u`
            <${D}
              key=${n}
              as="a"
              href=${a4(n,t)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <${O} name="shield" className="h-4 w-4" />
              ${a("login.oauthProvider",{provider:t4[n]||n})}
            <//>
          `)}
      </div>
    </div>
  `:null}var n4=["google","github","apple"];function _1(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>{let a=!1;return n$().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(n4.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function k1({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=R(),{theme:s,toggleTheme:i}=_c(),o=_1(),{formState:{errors:l,isSubmitting:c},handleSubmit:d,register:m}=$1({defaultValues:{token:e||""}});return u`
    <main
      className="relative flex min-h-[100dvh] items-center justify-center bg-[var(--v2-canvas)] px-4 py-8 sm:px-6 lg:px-12"
    >
      <!-- Theme toggle -->
      <${D}
        variant="secondary"
        size="icon"
        onClick=${i}
        aria-label=${r(s==="dark"?"theme.switchToLight":"theme.switchToDark")}
        title=${r(s==="dark"?"theme.light":"theme.dark")}
        className="absolute right-4 top-4 z-10 sm:right-6 sm:top-6"
      >
        <${O} name=${s==="dark"?"sun":"moon"} className="h-4 w-4" />
      <//>

      <!-- Login form (centered) -->
      <${ee}
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
          <${$n}
            label=${r("login.tokenLabel")}
            htmlFor="v2-token"
            error=${l.token?.message??""}
            hint=${r("login.tokenHint")}
          >
            <${Et}
              id="v2-token"
              type="password"
              error=${!!l.token}
              ...${m("token",{required:r("login.tokenRequired"),setValueAs:f=>f.trim()})}
              placeholder=${r("login.tokenPlaceholder")}
              autocomplete="current-password"
            />
          <//>

          ${t&&u`<p
              className=${V("rounded-[10px] border px-3 py-2 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >${t}</p>`}

          <${D}
            type="submit"
            variant="primary"
            fullWidth
            disabled=${c}
          >
            ${r("login.connect")}
          <//>
        </form>

        <${N1}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var R1={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},C1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function q({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return u`
    <span
      className=${V("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",C1[n]??C1.md,R1[e]??R1.muted,r)}
    >
      ${a&&u`<span
          className=${V("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var r4=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,E1=/(bash|shell|exec|run|command|terminal|spawn|process)/,T1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function A1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return r4.test(n)?{tone:"danger",key:"tool.riskWrite"}:E1.test(n)?{tone:"warning",key:"tool.riskExec"}:T1.test(n)?{tone:"info",key:"tool.riskNetwork"}:E1.test(r)?{tone:"warning",key:"tool.riskExec"}:T1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}var Bc=480;function s4(e,t){return t&&t.length>0?t.some(a=>typeof a?.value=="string"&&a.value.length>Bc):typeof e=="string"&&e.length>Bc}function D1(e,t){return typeof e!="string"||t||e.length<=Bc?e:`${e.slice(0,Bc).trimEnd()}
...`}function M1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=R(),{toolName:s,description:i,parameters:o,allowAlways:l,approvalDetails:c=[]}=e,[d,m]=p.default.useState(!1),[f,h]=p.default.useState(!1);p.default.useEffect(()=>{h(!1)},[e]);let x=p.default.useMemo(()=>A1(s,i,o),[s,i,o]),y=s||r("approval.thisTool"),$=s4(o,c),g=f?"max-h-72":"max-h-36",v=p.default.useCallback(()=>{d&&l?n?.():t?.()},[d,l,n,t]);return u`
    <div
      data-testid="approval-card"
      className="mx-auto max-w-lg rounded-xl border border-copper/30 bg-copper/10 p-4"
    >
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-copper/25 bg-copper/10 text-copper">
          <${O} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">${r("approval.title")}</span>
        <${q}
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
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${D1(b.value,f)}</dd>
                  </div>
                `)}
            </dl>
          `:o&&u`<pre className=${`mb-2 ${g} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${D1(o,f)}</pre>`}

      ${$&&u`
        <${D}
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
        <${D} variant="primary" onClick=${v}>
          ${r(d&&l?"approval.approveAndAlways":"approval.approve")}
        <//>
        <${D} variant="secondary" onClick=${()=>a?.()}>
          ${r("approval.deny")}
        <//>
      </div>
    </div>
  `}function mi({icon:e="lock",headline:t,provider:a,accountLabel:n,body:r,expiresAt:s,pillHint:i,defaultExpanded:o=!0,children:l}){let c=R(),[d,m]=p.default.useState(o),f=p.default.useId(),h=n||a||"";return u`
    <div className="mx-auto w-full max-w-lg rounded-xl border border-[rgba(76,167,230,0.34)] bg-[rgba(76,167,230,0.08)]">
      <button
        type="button"
        onClick=${()=>m(x=>!x)}
        aria-expanded=${d?"true":"false"}
        aria-controls=${f}
        className="flex w-full items-center gap-3 rounded-xl border-0 bg-transparent px-4 py-3 text-left"
      >
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-md border border-[rgba(76,167,230,0.28)] bg-[rgba(76,167,230,0.1)] text-[#8fc8f2]">
          <${O} name=${e} className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold text-white">
            ${t||c("authGate.title")}
          </span>
          ${h&&u`<span className="block truncate text-xs text-iron-300">${h}</span>`}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 text-xs font-medium text-[#8fc8f2]">
          ${i&&u`<span className="hidden sm:inline">${i}</span>`}
          <${O}
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
  `}function O1({gate:e,onCancel:t}){let a=R();return u`
    <${mi}
      icon="lock"
      headline=${e?.headline||a("authGate.title")}
      body=${e?.body||""}
    >
      <form onSubmit=${n=>n.preventDefault()}>
        <div className="mb-3 text-sm text-iron-200">
          ${a("authGate.unsupportedChallenge")}
        </div>
        <div className="flex flex-wrap gap-2">
          <${D} type="button" variant="secondary" onClick=${()=>t?.()}>
            ${a("authGate.cancel")}
          <//>
        </div>
      </form>
    <//>
  `}function L1({gate:e,onCancel:t}){let a=R(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),o=p.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]);p.default.useEffect(()=>{i("")},[e.authorizationUrl,e.gateRef,e.runId]);let l=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),c=p.default.useCallback(()=>{if(!o){i(a("authGate.serviceUnavailable"));return}i(""),window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0)},[e.authorizationUrl,o]),d=n?a("authGate.reopenAuthorization",{provider:l}):a("authGate.openAuthorization",{provider:l});return u`
    <${mi}
      icon="link"
      headline=${e?.headline||a("authGate.oauthTitle")}
      provider=${e?.provider?l:""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      expiresAt=${e?.expiresAt||""}
      pillHint=${a("authGate.pillAuthorize")}
    >
      <div className="flex flex-wrap gap-2">
        <${D}
          as="a"
          href=${o?e.authorizationUrl:void 0}
          target="_blank"
          rel="noopener noreferrer"
          className="auth-oauth"
          variant="primary"
          onClick=${m=>{m.preventDefault(),c()}}
        >
          <${O} name="link" className="h-4 w-4" />
          ${d}
        <//>
        <${D}
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
  `}function P1({gate:e,onSubmit:t,onCancel:a}){let n=R(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState(!1),d=p.default.useCallback(async m=>{m.preventDefault();let f=r.trim();if(!f){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(f),s("")}catch(h){o(h?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return u`
    <${mi}
      icon="lock"
      headline=${e?.headline||n("authGate.title")}
      provider=${e?.provider||""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      pillHint=${n("authGate.pillEnterToken")}
    >
      <form onSubmit=${d}>
        <div className="mb-3">
          <${Et}
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
          <${D} type="submit" variant="primary" disabled=${l}>
            ${n(l?"authGate.submitting":"authGate.submit")}
          <//>
          <${D}
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
  `}var i4="/api/webchat/v2/extensions/pairing/redeem";function U1(e){return H(i4,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function zc({action:e}){let t=R(),a=J(),n=Q({mutationFn:({code:l})=>U1(l),onSuccess:()=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=p.default.useState(""),i=o4(e,t),o=()=>{let l=r.trim();l&&(n.mutate({code:l}),s(""))};return u`
    <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
      <h4 className="mb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        ${i.title}
      </h4>
      <p className="mb-4 text-xs leading-5 text-iron-300">
        ${i.instructions}
      </p>

      <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          type="text"
          value=${r}
          onChange=${l=>s(l.target.value)}
          onKeyDown=${l=>l.key==="Enter"&&o()}
          placeholder=${i.codePlaceholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${D}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${o}
          disabled=${n.isPending||!r.trim()}
        >
          ${i.submitLabel}
        <//>
      </div>

      ${n.isSuccess&&u`<p className="text-xs text-emerald-300">
        ${n.data?.message||i.successMessage}
      </p>`}
      ${n.isError&&u`<p className="text-xs text-red-300">
        ${l4(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function o4(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function l4(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function u4(e,t){return e?.channel==="slack"&&e.strategy===t}function j1({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return u`
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
            <${O} name="close" className="h-4 w-4" />
          </button>
        `}
      </div>

      ${u4(e,"inbound_proof_code")?u`<${zc} action=${e.action} />`:u`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function c4(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):jr.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:jr.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:jr.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:jr.maxTotalBytes}:jr}function F1(){let e=xa(),t=K({enabled:!!e,queryKey:["session"],queryFn:bc,staleTime:5*6e4});return c4(t.data)}function qc({onSend:e,onCancel:t,disabled:a,sendDisabled:n=a,canCancel:r=!1,initialText:s="",resetKey:i="",draftKey:o=nl,variant:l="dock",context:c={},statusText:d=""}){let m=R(),f=l==="hero",h=F1(),[x,y]=p.default.useState(()=>Vp(o)),[$,g]=p.default.useState(()=>Yp(o)),[v,b]=p.default.useState(""),[w,N]=p.default.useState(!1),[C,_]=p.default.useState(!1),[A,L]=p.default.useState(!1),M=p.default.useRef(null),P=p.default.useRef(null),k=p.default.useRef(!1),z=a||n||w,Z=p.default.useRef(a||n);Z.current=a||n,k.current=z;let ne=p.default.useRef([]),de=p.default.useRef(Promise.resolve());p.default.useEffect(()=>{ne.current=$},[$]);let fe=p.default.useRef(null),Ce=p.default.useRef(null),Ue=p.default.useCallback(()=>{Ce.current&&(window.clearTimeout(Ce.current),Ce.current=null);let E=fe.current;fe.current=null,E&&E.scope===wt()&&Gp(E.key,E.text)},[]),gt=p.default.useCallback(()=>{Ce.current&&(window.clearTimeout(Ce.current),Ce.current=null),fe.current=null},[]),mt=p.default.useCallback(()=>{let E=M.current;E&&(E.style.height="auto",E.style.height=`${Math.min(E.scrollHeight,200)}px`)},[]);p.default.useEffect(()=>{mt()},[x,mt]),p.default.useEffect(()=>(y(Vp(o)),()=>Ue()),[o,Ue]);let tt=p.default.useRef(o);p.default.useEffect(()=>{if(tt.current!==o){tt.current=o,g(Yp(o)),b("");return}A$(o,$)},[o,$]),p.default.useEffect(()=>{s&&(y(s),window.requestAnimationFrame(()=>{M.current&&(M.current.focus(),M.current.setSelectionRange(s.length,s.length))}))},[s,i]);let Ve=p.default.useCallback(E=>{a||!E||E.length===0||(de.current=de.current.then(async()=>{let{staged:F,errors:B}=await g$(E,{limits:h,existing:ne.current,t:m});F.length>0&&g(j=>{let G=[...j,...F];return ne.current=G,G}),b(B.length>0?B.join(" "):"")}).catch(()=>{b(m("chat.attachmentStagingFailed"))}))},[a,h,m]),ft=p.default.useCallback(E=>{g(F=>{let B=F.filter(j=>j.id!==E);return ne.current=B,B}),b("")},[]),Tt=p.default.useCallback(()=>{a||P.current?.click()},[a]),sa=p.default.useCallback(E=>{let F=Array.from(E.target.files||[]);Ve(F),E.target.value=""},[Ve]),yt=p.default.useCallback(async()=>{if(!(!x.trim()||k.current)){k.current=!0,N(!0);try{if(await e(x.trim(),{attachments:$})===null)return;y(""),g([]),ne.current=[],b(""),gt(),T$(o),D$(o),M.current&&(M.current.style.height="auto")}catch{}finally{k.current=Z.current,N(!1)}}},[x,$,e,o,gt]),Ea=p.default.useCallback(E=>{let F=E.target.value;y(F),fe.current={key:o,text:F,scope:wt()},Ce.current&&window.clearTimeout(Ce.current),Ce.current=window.setTimeout(Ue,300)},[o,Ue]),Ge=p.default.useCallback(async()=>{if(!(!r||C||!t)){_(!0);try{await t()}finally{_(!1)}}},[r,C,t]),At=p.default.useCallback(E=>{if(E.key==="Enter"&&!E.shiftKey){if(E.preventDefault(),M.current?.dataset?.sendDisabled==="true"||k.current)return;yt()}},[yt]),Ta=p.default.useCallback(E=>{let F=Array.from(E.clipboardData?.files||[]);F.length>0&&(E.preventDefault(),Ve(F))},[Ve]),ia=p.default.useCallback(E=>{E.preventDefault(),L(!1);let F=Array.from(E.dataTransfer?.files||[]);F.length>0&&Ve(F)},[Ve]),oa=p.default.useCallback(E=>{E.preventDefault(),!a&&L(!0)},[a]),Gr=p.default.useCallback(E=>{E.currentTarget.contains(E.relatedTarget)||L(!1)},[]),Yr=x.trim(),ur=a||n,Ai=m(f?"chat.heroPlaceholder":"chat.followUpPlaceholder"),Jr=h.accept.length>0?h.accept.join(","):void 0,ae=f?"w-full":"px-4 py-3 sm:px-5 lg:px-8",S=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",f?"min-h-[120px]":"",a?"opacity-70":""].join(" "),T=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",f?"min-h-[72px]":"min-h-[40px]"].join(" ");return u`
    <div className=${ae}>
      <div
        className=${S}
        onDrop=${ia}
        onDragOver=${oa}
        onDragLeave=${Gr}
      >
        ${A&&u`
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
              <${O} name="close" className="h-3.5 w-3.5" strokeWidth=${2} />
            </button>
          </div>
        `}

        ${$.length>0&&u`
          <div className="mb-2 flex flex-wrap gap-2 px-1">
            ${$.map(E=>u`
                <div
                  key=${E.id}
                  className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
                >
                  ${E.previewUrl?u`<img
                        src=${E.previewUrl}
                        alt=${E.filename}
                        className="h-9 w-9 shrink-0 rounded object-cover"
                      />`:u`<span
                        className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                      >
                        <${O} name="file" className="h-4 w-4" />
                      </span>`}
                  <span className="flex min-w-0 flex-col">
                    <span className="max-w-[12rem] truncate font-medium">
                      ${E.filename}
                    </span>
                    <span className="text-[10px] text-iron-400">${E.sizeLabel}</span>
                  </span>
                  <button
                    type="button"
                    onClick=${()=>ft(E.id)}
                    aria-label=${m("chat.attachmentRemove")}
                    title=${m("chat.attachmentRemove")}
                    className="absolute right-1 top-1 grid h-5 w-5 place-items-center rounded-full text-iron-400 hover:bg-iron-700 hover:text-white"
                  >
                    <${O} name="close" className="h-3 w-3" />
                  </button>
                </div>
              `)}
          </div>
        `}

        <textarea
          ref=${M}
          data-testid="chat-composer"
          value=${x}
          onChange=${Ea}
          onKeyDown=${At}
          onPaste=${Ta}
          data-send-disabled=${ur?"true":"false"}
          placeholder=${Ai}
          rows=${1}
          disabled=${a}
          className=${T}
        />

        <input
          ref=${P}
          type="file"
          multiple
          accept=${Jr}
          className="hidden"
          onChange=${sa}
        />

        <div className="mt-2 flex items-center gap-2">
          ${ur&&u`
            <span className="inline-flex items-center gap-2 text-xs text-[var(--v2-text-muted)]">
              <span className="h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
              ${d||m("chat.statusWorking")}
            </span>
          `}
          <div className="ml-auto flex items-center gap-1.5">
            <button
              type="button"
              onClick=${Tt}
              disabled=${a}
              aria-label=${m("chat.attachFiles")}
              title=${m("chat.attachFiles")}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <${O} name="plus" className="h-5 w-5" />
            </button>
            ${r?u`
                <${D}
                  type="button"
                  variant="danger"
                  size="icon-sm"
                  onClick=${Ge}
                  disabled=${C}
                  aria-label=${m("common.cancel")}
                  title=${m("common.cancel")}
                  className="rounded-full"
                >
                  <${O} name="close" className="h-5 w-5" />
                <//>
              `:u`
                <${D}
                  type="button"
                  variant="primary"
                  size="icon-sm"
                  onClick=${yt}
                  disabled=${ur||w||!Yr}
                  aria-label=${m("chat.send")}
                  className="rounded-full"
                >
                  <${O} name="send" className="h-5 w-5" />
                <//>
              `}
          </div>
        </div>
      </div>
    </div>
  `}var B1={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function z1({status:e}){let t=R();if(e==="idle"||e==="connecting"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return u`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",B1[e]||B1.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function q1({onSuggestion:e,onSend:t,disabled:a,sendDisabled:n,initialText:r,resetKey:s,draftKey:i,context:o,statusText:l,canCancel:c,onCancel:d}){let m=R(),f=[{icon:"tool",title:m("chat.suggestion1"),detail:m("chat.suggestion1Desc")},{icon:"shield",title:m("chat.suggestion2"),detail:m("chat.suggestion2Desc")},{icon:"plug",title:m("chat.suggestion3"),detail:m("chat.suggestion3Desc")}];return u`
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
        <${qc}
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
                <${O} name=${h.icon} className="h-4 w-4" />
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
  `}var d4=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function I1({open:e,onClose:t}){let a=R();return e?u`
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
            <${O} name="bolt" className="h-4 w-4" />
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
            <${O} name="close" className="h-4 w-4" />
          </button>
        </div>
        <ul className="flex flex-col gap-2">
          ${d4.map((n,r)=>u`
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
  `:null}function H1(e){let t=0,a=0,n=0,r=0,s=0;for(let o of e){if(o.role==="thinking"&&(t+=1),o.role==="tool_activity"){let l=K1([o]);a+=l.tools,n+=l.failed,r+=l.declined,s+=l.running}if(m4(o)){let l=K1(o.toolCalls);a+=l.tools,n+=l.failed,r+=l.declined,s+=l.running}}let i=[];return t&&i.push(`${t} reasoning`),a&&i.push(`${a} ${a===1?"tool":"tools"}`),n&&i.push(`${n} failed`),r&&i.push(`${r} declined`),!n&&!r&&s&&i.push("running"),{hasError:n>0,hasDeclined:r>0,label:`Activity${i.length?` - ${i.join(", ")}`:""}`}}function K1(e){let t=0,a=0,n=0;for(let r of e)r.toolStatus==="error"&&(t+=1),r.toolStatus==="declined"&&(a+=1),r.toolStatus==="running"&&(n+=1);return{tools:e.length,failed:t,declined:a,running:n}}function m4(e){return e.toolCalls&&e.toolCalls.length>0}var Q1=!1;function f4(){Q1||!window.DOMPurify||(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),Q1=!0)}function V1(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}f4();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var fh=360;function p4(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let l=s("Copy");if(l.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),l.textContent="Copied",ui("Code copied",{tone:"success"}),setTimeout(()=>l.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(l),n.appendChild(r),t.scrollHeight>fh){t.style.maxHeight=`${fh}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${fh}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function h4({content:e,className:t=""}){let a=p.default.useRef(null),n=p.default.useMemo(()=>V1(e),[e]);return p.default.useEffect(()=>{p4(a.current)},[n]),u`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var aa=p.default.memo(h4);var G1={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",declined:"bg-iron-400",error:"bg-[var(--v2-danger-text)]"},v4={success:"ok",declined:"declined",error:"err",running:"run"},g4=2;function fi({activity:e}){return e.toolCalls&&e.toolCalls.length>0?u`<${b4} tools=${e.toolCalls} />`:u`<${x4} activity=${e} />`}function y4(e,t){let a=0,n=0,r=0,s=0;for(let l of t){let c=String(l.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function b4({tools:e}){let t=R(),a=e.some(o=>o.toolStatus==="error"),n=e.some(o=>o.toolStatus==="error"||o.toolStatus==="declined"),[r,s]=p.default.useState(n);if(p.default.useEffect(()=>{n&&s(!0)},[n]),e.length<=g4)return u`
      <div className="flex flex-col gap-3">
        ${e.map((o,l)=>u`<${fi}
            key=${o.id||o.callId||`${o.toolName}-${l}`}
            activity=${o}
          />`)}
      </div>
    `;let i=y4(t,e);return u`
    <div className="flex flex-col">
      <button
        type="button"
        onClick=${()=>s(o=>!o)}
        aria-expanded=${r?"true":"false"}
        className=${["v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",a?"text-[var(--v2-danger-text)]":"text-iron-400 hover:text-iron-200"].join(" ")}
      >
        <${O} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${i}</span>
        <${O}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0",r?"rotate-180":""].join(" ")}
        />
      </button>

      ${r&&u`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((o,l)=>u`<${fi}
              key=${o.id||o.callId||`${o.toolName}-${l}`}
              activity=${o}
            />`)}
        </div>
      `}
    </div>
  `}function x4({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:l}=e,[c,d]=p.default.useState(n==="error"||n==="declined");p.default.useEffect(()=>{(n==="error"||n==="declined")&&d(!0)},[n]);let m=G1[n]||G1.running,f=i!=null,h=p.default.useId(),x=u`
    <button
      type="button"
      onClick=${()=>d(y=>!y)}
      aria-expanded=${c?"true":"false"}
      aria-controls=${h}
      className="v2-button flex w-full items-center gap-2.5 border-0 border-b border-iron-700/40 bg-transparent px-1 py-2 text-left text-sm"
    >
      <span className=${["h-2 w-2 shrink-0 rounded-full",m].join(" ")} />
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-wide text-iron-300"
        >${v4[n]||"run"}</span
      >
      <span className="shrink-0 truncate font-mono text-[13px] font-medium text-iron-100"
        >${a}</span
      >
      ${r&&u`<span className="min-w-0 truncate font-mono text-xs text-iron-400"
        >${r}</span
      >`}
      <span className="ml-auto flex shrink-0 items-center gap-2">
        ${f&&u`<span className="font-mono text-[11px] text-iron-300">${i}ms</span>`}
        <${O}
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
          <${O} name="tool" className="h-4 w-4" />
        </div>
      `}
      <div className=${t?"min-w-0 flex-1":"min-w-0 max-w-[85%] flex-1"}>
        ${x}
        ${c&&u`<${$4}
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
  `}function $4({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=R(),l=p.default.useMemo(()=>{let f=[];return r&&f.push({id:s==="declined"?"declined":"error",label:o(s==="declined"?"tool.tabDeclined":"tool.tabError")}),t&&f.push({id:"details",label:o("tool.tabDetails")}),a&&f.push({id:"params",label:o("tool.tabParameters")}),n&&f.push({id:"result",label:o("tool.tabResult")}),f},[o,r,t,a,n,s]),[c,d]=p.default.useState(null),m=c&&l.some(f=>f.id===c)?c:l[0]?.id;return p.default.useEffect(()=>{r&&d(s==="declined"?"declined":"error")},[r,s]),l.length===0?u`
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
        ${m==="result"&&u`<${w4} text=${n} />`}
        ${(m==="error"||m==="declined")&&u`<pre
          className=${["overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono",m==="declined"?"text-iron-300":"text-[var(--v2-danger-text)]"].join(" ")}
        >${r}</pre>`}
      </div>
    </div>
  `}function w4({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return u`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(S4)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return u`
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
                  >${N4(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?u`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:u`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function S4(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function N4(e){return e==null?"":String(e)}function Y1({activity:e}){let t=H1(e),a=R4(e),[n,r]=p.default.useState(a);return p.default.useEffect(()=>{a&&r(!0)},[a]),u`
    <div className="mr-auto flex w-full max-w-[85%] flex-col">
      <button
        type="button"
        onClick=${()=>r(s=>!s)}
        aria-expanded=${n?"true":"false"}
        className=${["v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",t.hasError?"text-[var(--v2-danger-text)]":"text-iron-400 hover:text-iron-200"].join(" ")}
      >
        <${O} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${t.label}</span>
        <${O}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0",n?"rotate-180":""].join(" ")}
        />
      </button>

      ${n&&u`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((s,i)=>u`
            <${_4}
              key=${s.id||`${s.role||"activity"}-${i}`}
              item=${s}
            />
          `)}
        </div>
      `}
    </div>
  `}function _4({item:e}){if(e.role==="thinking")return u`<${k4} content=${e.content} />`;if(e.role==="tool_activity"||ph(e)){let t=ph(e)?{id:e.id,toolCalls:e.toolCalls}:e;return u`<${fi} activity=${t} />`}return null}function k4({content:e}){return e?u`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <${O} name="spark" className="h-4 w-4" />
      </div>
      <div className="min-w-0 max-w-[85%] flex-1 border-l-2 border-white/10 pl-3 text-iron-300">
        <${aa} content=${e} className="text-[13px]" />
      </div>
    </div>
  `:null}function ph(e){return e?.toolCalls&&e.toolCalls.length>0}function R4(e){return(e||[]).some(t=>t?.role==="thinking"||t?.toolStatus==="running"||t?.toolStatus==="error"||t?.toolStatus==="declined"?!0:ph(t)?t.toolCalls.some(a=>a?.toolStatus==="running"||a?.toolStatus==="error"||a?.toolStatus==="declined"):!1)}function Ic(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function C4({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=p.default.useState(()=>t&&e.preview_url||null);return p.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return wc(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?u`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:u`<${O} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var J1="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",X1="px-3 py-2";function Kc({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await _a(e.fetch_url);Ic(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),l=u`
    <${C4} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?u`<div
      className=${`${J1} ${X1} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${l}
    </div>`:u`<div className=${`${J1} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${X1} text-left transition-colors hover:bg-iron-900/80`}
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
      <${O} name="download" className="h-3.5 w-3.5" />
    </button>`}
  </div>`}var Z1={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function pi({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return p.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),p.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?u`
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
        className=${V("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",Z1[n]??Z1.md,r)}
      >
        ${a?u`<${hh} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function hh({children:e,onClose:t,className:a=""}){return u`
    <div
      className=${V("flex shrink-0 items-center justify-between gap-4","px-5 py-4 md:px-7 md:py-5","border-b border-[var(--v2-panel-border)]",a)}
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
            <${O} name="close" className="h-4 w-4" />
          </button>
        `}
    </div>
  `}function hi({children:e,className:t=""}){return u`
    <div className=${V("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function vi({children:e,className:t=""}){return u`
    <div
      className=${V("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var W1=1e5;function Hc({attachment:e,onClose:t}){let a=!!e,[n,r]=p.default.useState("loading"),[s,i]=p.default.useState({}),o=e?v$(e.mime_type):"download";if(p.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return _a(e.fetch_url).then(async m=>{d=URL.createObjectURL(m);let f={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")f.dataUrl=await Up(m);else if(o==="pdf")f.frameUrl=d;else if(o==="text"){let h=await m.text();f.truncated=h.length>W1,f.text=f.truncated?h.slice(0,W1):h}if(c){URL.revokeObjectURL(d);return}i(f),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let l=e.filename||"attachment";return u`
    <${pi} open=${a} onClose=${t} size="xl">
      <${hh} onClose=${t}>
        <span className="block truncate">${l}</span>
      <//>
      <${hi} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&u`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&u`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&u`<${E4} mode=${o} view=${s} filename=${l} />`}
      <//>
      <${vi}>
        ${s.downloadUrl&&u`<a
          href=${s.downloadUrl}
          download=${l}
          data-testid="attachment-download"
          className="v2-button inline-flex items-center gap-1.5 rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-200 hover:border-signal/35 hover:text-white"
        >
          <${O} name="download" className="h-3.5 w-3.5" />
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
  `}function E4({mode:e,view:t,filename:a}){switch(e){case"image":return u`<img
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
        <${O} name="file" className="h-10 w-10 text-signal" />
        <div className="text-sm">This file type can't be previewed.</div>
      </div>`}}var T4=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function A4(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function e2(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of A4(e).matchAll(T4)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function t2(e){return e.split("/").filter(Boolean).pop()||e}function a2(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function D4({threadId:e,path:t,onPreview:a}){let[n,r]=p.default.useState({mime_type:"",size_label:""});p.default.useEffect(()=>{let i=!0;return jx({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:a2(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:t2(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:$c({threadId:e,path:t})};return u`<${Kc}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function n2({threadId:e,content:t}){let a=p.default.useMemo(()=>e2(t),[t]),[n,r]=p.default.useState(null);return!e||a.length===0?null:u`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>u`<${D4}
          key=${s}
          threadId=${e}
          path=${s}
          onPreview=${r}
        />`)}
      <${Hc}
        attachment=${n}
        onClose=${()=>r(null)}
      />
    </div>
  `}var r2={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function M4(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function O4({content:e}){let[t,a]=p.default.useState(!1);return e?u`
    <div className="flex flex-col items-start">
      <button
        type="button"
        onClick=${()=>a(n=>!n)}
        aria-expanded=${t?"true":"false"}
        className="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent px-1 py-1 text-xs font-medium text-iron-400 hover:text-iron-200"
      >
        <${O} name="spark" className="h-3.5 w-3.5" />
        <span>${t?"Hide reasoning":"Reasoning"}</span>
        <${O}
          name="chevron"
          className=${["h-3 w-3",t?"rotate-180":""].join(" ")}
        />
      </button>
      ${t&&u`
        <div className="mt-1 border-l-2 border-white/10 pl-3 text-iron-300">
          <${aa} content=${e} className="text-[13px]" />
        </div>
      `}
    </div>
  `:null}function L4({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:l,status:c,error:d,toolCalls:m,timestamp:f}=e,h=n==="user",[x,y]=p.default.useState(!1),[$,g]=p.default.useState(null),v=p.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),y(!0),ui("Copied to clipboard",{tone:"success"}),setTimeout(()=>y(!1),1400)}catch{}},[r]);if(n==="tool_activity"||m&&m.length>0){let M=m&&m.length>0?{id:e.id,toolCalls:m}:e;return u`<${fi} activity=${M} />`}if(n==="thinking")return u`<${O4} content=${r} />`;if(n==="image")return u`
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
    `;let b=M4(f),w=n==="user"||n==="assistant"&&!l,N=n==="system"||n==="error",C=h?"max-w-[85%]":N?"mx-auto max-w-[85%]":"w-full max-w-[85%]",_=h?"":"w-full min-w-0 max-w-full",A=c==="error"&&t,L=w||A||b;return u`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",h?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col",C].join(" ")}>
        <div
          className=${["text-base leading-7",_,r2[n]||r2.assistant,l?"opacity-70":""].join(" ")}
        >
          ${n==="assistant"||n==="system"||n==="error"?u`<${aa} content=${r} />`:u`<div className="whitespace-pre-wrap">${r}</div>`}

          ${c==="error"&&u`
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-300">
              <span>${d}</span>
            </div>
          `}

          ${s&&s.length>0&&u`
            <div className="mt-2 flex flex-wrap gap-2">
              ${s.map((M,P)=>u`<img key=${P} src=${M} className="max-h-48 rounded-lg border border-iron-700 object-cover" alt="Message attachment" />`)}
            </div>
          `}

          ${i&&i.length>0&&u`
            <div className="mt-2 flex flex-col gap-1.5">
              ${i.map((M,P)=>u`<${Kc}
                key=${M.id||P}
                att=${M}
                onPreview=${g}
              />`)}
            </div>
            <${Hc}
              attachment=${$}
              onClose=${()=>g(null)}
            />
          `}

          ${n==="assistant"&&u`<${n2}
            threadId=${a}
            content=${typeof r=="string"?r:""}
          />`}
        </div>
      </div>

      ${L&&u`
        <div
          className=${["mt-1 flex min-h-7 w-max max-w-[85%] flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100",h?"self-end justify-end":N?"self-center justify-center":"self-start justify-start"].join(" ")}
        >
          ${b&&u`<time dateTime=${f} className="shrink-0 font-mono text-[11px] text-iron-500">${b}</time>`}
          ${(w||A)&&u`
            <div className="flex shrink-0 items-center gap-1">
            ${w&&u`
              <button
                type="button"
                onClick=${v}
                title=${x?"Copied":"Copy message"}
                aria-label=${x?"Copied":"Copy message"}
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 hover:text-iron-100"
              >
                <${O} name=${x?"check":"copy"} className="h-3.5 w-3.5" />
              </button>
            `}
            ${A&&u`
              <button
                type="button"
                onClick=${()=>t(e)}
                title="Retry message"
                aria-label="Retry message"
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 text-red-300 hover:text-red-200"
              >
                <${O} name="retry" className="h-3.5 w-3.5" />
              </button>
            `}
            </div>
          `}
        </div>
      `}
    </div>
  `}var s2=p.default.memo(L4);function d2(e){let t=P4(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(m2(r)){let s=i2(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){o2(a,s),l2(a,r),n+=s.length;continue}}if(vh(r)){let s=i2(t,n);o2(a,s),n+=s.length-1;continue}l2(a,r)}return a}function P4(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=Qc(i);o&&m2(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!vh(i))continue;let o=Qc(i),l=o?t.get(o):void 0;if(l===void 0||l>=s)continue;let c=a.get(l)||[];c.push(i),a.set(l,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function i2(e,t){let a=t,n=Qc(e[t]);for(;a<e.length&&vh(e[a])&&U4(n,e[a]);)a+=1;return e.slice(t,a)}function U4(e,t){let a=Qc(t);return!e||!a||a===e}function o2(e,t){if(t.length===0)return;let a=j4(t);e.push({type:"activity-run",id:`activity-run-${a[0].id}`,activity:a})}function l2(e,t){e.push({type:"message",id:t.id,message:t})}function m2(e){return e.role==="assistant"&&!f2(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function vh(e){return e.role==="thinking"||e.role==="tool_activity"||f2(e)}function f2(e){return e?.toolCalls&&e.toolCalls.length>0}function Qc(e){return e?.turnRunId||null}function j4(e){return[...e].sort((t,a)=>t?.role!=="tool_activity"||a?.role!=="tool_activity"?0:F4(t,a))}function F4(e,t){if(Number.isFinite(e.activityOrder)&&Number.isFinite(t.activityOrder)){let n=e.activityOrder-t.activityOrder;if(n!==0)return n}let a=u2(c2(e.updatedAt||e.timestamp),c2(t.updatedAt||t.timestamp));return a!==0?a:u2(e.sequence,t.sequence)}function u2(e,t){let a=Number.isFinite(e)?e:null,n=Number.isFinite(t)?t:null;return a===null&&n===null?0:a===null?1:n===null?-1:a-n}function c2(e){if(!e)return null;let t=Date.parse(e);return Number.isFinite(t)?t:null}var B4=100,z4=100;function q4(e){return e?e.scrollHeight-e.scrollTop-e.clientHeight:Number.POSITIVE_INFINITY}function p2(e,t=B4){return q4(e)<=t}function h2(e){e&&(e.scrollTop=Math.max(0,e.scrollHeight-e.clientHeight))}function v2(e){return e?.id?`${e.role||""}:${e.id}`:null}function I4(e,t){let a=v2(t);return!!(a&&t?.role==="user"&&a!==e)}function g2({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let l=R(),c=p.default.useRef(null),d=p.default.useRef(null),m=p.default.useRef(!0),f=p.default.useRef(null),h=p.default.useRef(null),x=p.default.useRef(null),y=p.default.useRef(0),$=p.default.useRef(!1),[g,v]=p.default.useState(!0),b=p.default.useCallback(()=>{h.current!==null&&(window.cancelAnimationFrame(h.current),h.current=null)},[]),w=p.default.useCallback((k=!1)=>{c.current&&(k&&(m.current=!0,$.current=!1),m.current&&(b(),h.current=window.requestAnimationFrame(()=>{h.current=null;let Z=c.current;!Z||!k&&!m.current||(h2(Z),y.current=Z.scrollTop,$.current=!1,v(!0))})))},[b]),N=p.default.useCallback(()=>{x.current!==null&&(window.cancelAnimationFrame(x.current),x.current=null)},[]);p.default.useLayoutEffect(()=>{let k=e.length>0?e[e.length-1]:null,z=v2(k),Z=I4(f.current,k);return f.current=z,w(Z),b},[e,i,w,b]),p.default.useLayoutEffect(()=>{let k=d.current;if(!k||typeof ResizeObserver!="function")return;let z=new ResizeObserver(()=>{w()});return z.observe(k),()=>{z.disconnect(),b()}},[w,b]);let C=p.default.useCallback(()=>{x.current=null;let k=c.current;if(!k)return;let z=p2(k);y.current=k.scrollTop,z?(m.current=!0,$.current=!1,v(!0)):$.current?(m.current=!1,v(!1)):(m.current=!0,v(!0),w()),a&&k.scrollTop<z4&&n&&!t&&n()},[a,n,t,w]),_=p.default.useCallback(()=>{$.current=!0},[]),A=p.default.useCallback(k=>{let z=c.current;if(!z||typeof k?.clientX!="number")return;let Z=z.offsetWidth-z.clientWidth;if(Z<=0)return;let ne=z.getBoundingClientRect().right;k.clientX>=ne-Z-2&&($.current=!0)},[]),L=p.default.useCallback(()=>{let k=c.current;if(!k)return;let z=p2(k),Z=k.scrollTop<y.current;y.current=k.scrollTop,!z&&Z&&($.current=!0),z?(m.current=!0,$.current=!1):$.current?(m.current=!1,b()):m.current=!0,x.current===null&&(x.current=window.requestAnimationFrame(C))},[b,C]),M=p.default.useCallback(()=>{let k=c.current;k&&(h2(k),y.current=k.scrollTop,m.current=!0,$.current=!1,v(!0))},[]);p.default.useEffect(()=>N,[N]);let P=p.default.useMemo(()=>d2(e),[e]);return u`
    <div className="relative flex min-h-0 min-w-0 flex-1">
    <div
      ref=${c}
      onScroll=${L}
      onWheel=${_}
      onTouchMove=${_}
      onPointerDown=${A}
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
        ${P.map(k=>k.type==="activity-run"?u`<${Y1} key=${k.id} activity=${k.activity} />`:u`<${s2}
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
        onClick=${M}
        aria-label=${l("chat.jumpToLatest")}
        className="absolute bottom-4 left-1/2 inline-flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-3 py-1.5 text-xs font-medium text-[var(--v2-text-strong)] shadow-[0_10px_30px_-12px_rgba(0,0,0,0.7)] hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]"
      >
        <${O} name="arrowDown" className="h-3.5 w-3.5" />
        ${l("chat.jumpToLatest")}
      </button>
    `}
    </div>
  `}function y2({notice:e,onRecover:t}){return u`
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
  `}function b2({suggestions:e,onSelect:t,disabled:a=!1}){return!e||e.length===0?null:u`
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
  `}function x2(){return u`
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
  `}function Vc(){return H("/api/webchat/v2/channels/connectable")}function $2(e,t){if(!gh(e))return null;let a=Gc(e),n=V4(a),r=null;for(let s of t||[]){if(!Q4(s))continue;let i=G4(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function gh(e){let t=Gc(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function K4(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function H4(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>w2(Gc(n))):a}function Q4(e){return e?.strategy!=="admin_managed_channels"}function V4(e){return S2(e,"slack")&&w2(e)}function w2(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function Gc(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function G4(e,t,a={}){return(a.commandAliasesOnly?H4(t,{channelManagementOnly:!0}):K4(t)).reduce((r,s)=>{let i=Gc(s);return S2(e,i)?Math.max(r,i.length):r},0)}function S2(e,t){return t?` ${e} `.includes(` ${t} `):!1}function N2(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n={kind:"gate",gateKind:"approval",runId:t.turn_run_id,gateRef:t.gate_ref,invocationId:t.invocation_id||null,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return Y4(n,a,t.body)}return e==="auth_required"?{kind:"auth_required",gateKind:"auth",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,invocationId:t.invocation_id||null,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function _2(e){if(!e?.run_id||!e.gate_ref)return null;let t=e.gate_kind||"generic",a={gateKind:t,runId:e.run_id,gateRef:e.gate_ref,invocationId:e.invocation_id||null,headline:e.headline,body:e.body||"",allowAlways:e.allow_always===!0};if(t==="auth"){let n=e.auth_context||{};return{...a,kind:"auth_required",challengeKind:n.challenge_kind||"other",provider:n.provider||null,accountLabel:n.account_label||"",authorizationUrl:n.authorization_url||null,expiresAt:n.expires_at||null}}return{...a,kind:"gate"}}function Y4(e,t,a){if(!t)return e;let n=J4(t);return{...e,toolName:t.tool_name||null,description:t.reason||a,actionLabel:t.action?.label||null,destination:t.destination||null,approvalScope:t.scope||null,approvalDetails:n,parameters:n.length?n.map(r=>`${r.label}: ${r.value}`).join(`
`):null}}function J4(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function k2({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function R2(){return{terminalByInvocation:new Map}}function C2(e){e?.current?.terminalByInvocation?.clear()}function bh(e,t,a){let n=T2(t,{toolStatus:"running"});n&&gi(e,n,a)}function E2(e,t,a,n="gate_declined"){let r=T2(t,{toolStatus:"declined",toolError:n,toolErrorKind:"gate_declined"});r&&gi(e,r,a)}function gi(e,t,a){if(!t)return;let n=a5(t);n=t5(n,a),e(r=>{let s=A2(n),i=Z4(r,n,s);if(i>=0){let l=[...r];return l[i]=W4(l[i],n),yh(l[i],a),l}let o={id:s,role:"tool_activity",...n};return yh(o,a),[...r,o]})}function T2(e,t={}){let a=e?.kind==="gate",n=e?.kind==="auth_required",r=n&&t.toolStatus==="declined";if(!e?.runId||!e?.gateRef||!e.invocationId&&!r||!a&&!n)return null;let s=e.invocationId||X4(e),i=e.toolName||e.headline||e.gateKind||"gate";return{invocationId:s,callId:s,capabilityId:e.toolName||e.gateKind||null,toolName:Wo(i)||i,toolStatus:t.toolStatus||"running",toolDetail:null,toolParameters:null,toolResultPreview:null,toolError:t.toolError||null,toolErrorKind:t.toolErrorKind||null,toolDurationMs:null,updatedAt:t.updatedAt||new Date().toISOString(),resultRef:null,truncated:!1,outputBytes:null,outputKind:null,turnRunId:e.runId,gateRef:e.gateRef,gateActivity:!0}}function X4(e){return`gate:${e.runId}:${e.kind}:${e.gateRef}`}function A2(e){return`tool-${e.invocationId}`}function Z4(e,t,a){let n=e.findIndex(s=>s?.id===a);if(n>=0)return n;let r=t.gateRef||null;if(r){let s=e.findIndex(i=>i?.role==="tool_activity"&&i.turnRunId===t.turnRunId&&i.gateRef===r);if(s>=0)return s}return-1}function W4(e,t){let a=Zo(e.toolStatus),n=Zo(t.toolStatus),r=a&&!n,s=t.gateActivity&&!e.gateActivity,i={...e,...t,id:e.id,role:"tool_activity",invocationId:e.gateActivity&&!t.gateActivity?t.invocationId:e.invocationId||t.invocationId,callId:e.gateActivity&&!t.gateActivity?t.callId:e.callId||t.callId,toolName:s?e.toolName:t.toolName||e.toolName,toolStatus:r?e.toolStatus:t.toolStatus,toolError:t.toolError||e.toolError,toolErrorKind:t.toolErrorKind||e.toolErrorKind||null,updatedAt:r?e.updatedAt||t.updatedAt:t.updatedAt||e.updatedAt,turnRunId:t.turnRunId||e.turnRunId||null,gateRef:t.gateRef||e.gateRef||null,gateActivity:e.gateActivity&&t.gateActivity,capabilityId:s?e.capabilityId||t.capabilityId||null:t.capabilityId||e.capabilityId||null,activityOrder:e5(e,t),activityOrderSource:t.activityOrderSource||e.activityOrderSource||null};return e.gateActivity&&!t.gateActivity&&(i.id=A2(t),i.gateActivity=!1),i}function e5(e,t){return Number.isFinite(t.activityOrder)?t.activityOrder:e.activityOrder}function t5(e,t){if(!e?.invocationId)return e;if(Zo(e.toolStatus))return yh(e,t),e;let a=t?.current?.terminalByInvocation?.get(e.invocationId);return a?Number.isFinite(e.activityOrder)?{...a,activityOrder:e.activityOrder,activityOrderSource:e.activityOrderSource||a.activityOrderSource||null}:a:e}function yh(e,t){!e?.invocationId||!Zo(e.toolStatus)||t?.current?.terminalByInvocation?.set(e.invocationId,e)}function a5(e){let t=Wo(e.toolName||e.capabilityId);return{...e,toolName:t||e.toolName||"tool"}}function P2({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o,onRunSettled:l}){let c=p.default.useRef(new Set),d=p.default.useRef(null),m=p.default.useRef(null);return p.default.useCallback(f=>{let{type:h,frame:x}=f||{};if(!(!h||!x))switch(h){case"accepted":{let y=x.ack||{};y.run_id&&(d.current=y.run_id),r?.({runId:y.run_id||null,threadId:y.thread_id||e,status:y.status||null}),a(!0);return}case"running":case"capability_progress":{let y=x.progress||{};y.turn_run_id&&(d.current=y.turn_run_id,r?.($=>$&&$.runId===y.turn_run_id?{...$,status:"running"}:{runId:y.turn_run_id,threadId:e,status:"running"}),n5(n,y.turn_run_id,m)),a(!0);return}case"capability_activity":{let y=x.activity;if(!y||!y.invocation_id)return;gi(t,Kp(y),o);return}case"capability_display_preview":{let y=x.preview;if(!y||!y.invocation_id)return;let $=Ip(y);gi(t,$,o);return}case"gate":case"auth_required":{let y=N2(h,x.prompt);y&&(bh(t,y,o),n(y),r?.({runId:y.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let y=x.reply||{};t($=>[...$,{id:`reply-${y.turn_run_id||Date.now()}`,role:"assistant",content:y.text||"",timestamp:y.generated_at||new Date().toISOString(),turnRunId:y.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let y=x.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Xc(c,l,y,!1);return}case"failed":{let y=x.run_state||{},$=y.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),$h(t,{runId:$,status:y.status||"failed",failureCategory:o5(y),failureSummary:null}),Xc(c,l,$,!1);return}case"projection_snapshot":case"projection_update":{let y=x.state?.items||[];s5({items:y,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:l,settledRunsRef:c,latestRunIdRef:d,promptRunIdRef:m,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i,o,l])}function Xc(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var D2=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),M2=new Set(["completed","succeeded"]),Yc=new Set(["blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]),Jc=new Set(["awaiting_gate","blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]);function O2(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function n5(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function r5(e,t,a,n,r,s){let i=t?.runId||null;if(!i||r?.has(i))return!0;let o=a?.get(i);if(o)return!Jc.has(o);let l=e?.current,c=l?.runId||n?.current||null;if(c&&i!==c)return!0;let d=s?.current===c;return c&&i===c&&!d&&l?.status&&!Jc.has(l.status)?!0:!l?.runId||!l.status?!1:!Jc.has(l.status)}function s5({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:l,promptRunIdRef:c,activeRunRef:d,locallyResolvedGatesRef:m,toolActivityStateRef:f}){let h=new Map,x=new Set,y=d?.current||null,$=y?.runId||l?.current||null;for(let v of e){let b=v.run_status;b?.run_id&&b.status&&(h.set(b.run_id,b.status),$&&$!==b.run_id&&y?.status&&!D2.has(y.status)&&Yc.has(b.status)&&x.add(b.run_id))}let g=l?.current??null;for(let v of e){if(v.run_status){let{run_id:b,status:w,failure_category:N,failure_summary:C}=v.run_status,_=D2.has(w),A=d?.current?.source==="local"?d.current.runId:null,L=!!(b&&A&&A!==b),M=g??l?.current??null,P=!!(_&&b&&M&&M!==b),k=b&&Yc.has(w)?L2(m,b):null;if(b&&x.has(b)||L)continue;if(P){L2(m,d?.current?.runId)?.outcome==="resumed"&&(i5({runId:b,activePromptRunId:d?.current?.runId,success:M2.has(w),status:w,failureCategory:N,failureSummary:C,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:l,promptRunIdRef:c,locallyResolvedGatesRef:m}),g=null);continue}if(k){O2(r,b,c),k.outcome==="resumed"?(n(!0),s?.(z=>z&&z.runId===b?{...z,status:z.status==="awaiting_gate"?"queued":z.status||"queued"}:{runId:b,threadId:t,status:"queued"}),g=b,l&&(l.current=b)):(n(!1),d?.current?.runId===b&&s?.(null),g=null,l?.current===b&&(l.current=null));continue}b&&(g=b,!_&&l&&(l.current=b),s?.(z=>z&&z.runId===b?{...z,status:w}:{runId:b,threadId:t,status:w})),b&&Yc.has(w)?c&&(c.current=b):b&&c?.current===b&&(c.current=null),_?(n(!1),r(null),s?.(null),xh(m,b),g=null,l&&(l.current=null),b&&c?.current===b&&(c.current=null),Xc(o,i,b,M2.has(w)),(w==="failed"||w==="recovery_required")&&$h(a,{runId:b,status:w,failureCategory:N,failureSummary:C})):Yc.has(w)||(O2(r,b,c),xh(m,b),n(!0))}if(v.text){let b=`text-${v.text.id}`;a(w=>{let N=w.findIndex(_=>_.id===b),C={id:b,role:"assistant",content:v.text.body||"",timestamp:new Date().toISOString(),isFinalReply:!0};if(N>=0){let _=[...w];return _[N]=C,_}return[...w,C]}),n(!1)}if(v.thinking){let b=`thinking-${v.thinking.id}`;a(w=>{let N=w.findIndex(_=>_.id===b),C={id:b,role:"thinking",content:v.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:v.thinking.run_id||null};if(N>=0){let _=[...w];return _[N]=C,_}return[...w,C]})}if(v.capability_activity){let b=v.capability_activity;b.invocation_id&&gi(a,Kp(b),f)}if(v.gate){let b=_2(v.gate),w=b?.runId||null;w&&!r5(d,b,h,l,x,c)&&!u5(m,w,b.gateRef)&&(bh(a,b,f),r(N=>N||b),s?.(N=>N&&N.runId===w?{...N,status:Jc.has(N.status)?N.status:"awaiting_gate"}:{runId:w,threadId:t,status:"awaiting_gate"}),c&&(c.current=w),n(!1))}if(v.skill_activation){let{id:b,skill_names:w=[],feedback:N=[]}=v.skill_activation;if(w.length||N.length){let C=`skill-${b||w.join("-")||"activation"}`,_=[w.length?`Skill activated: ${w.join(", ")}`:"",...N].filter(Boolean).join(`
`);a(A=>A.some(L=>L.id===C)?A:[...A,{id:C,role:"system",content:_,timestamp:new Date().toISOString()}])}}}l&&g&&(l.current=g)}function i5({runId:e,activePromptRunId:t,success:a,status:n,failureCategory:r,failureSummary:s,setMessages:i,setIsProcessing:o,setPendingGate:l,setActiveRun:c,onRunSettled:d,settledRunsRef:m,latestRunIdRef:f,promptRunIdRef:h,locallyResolvedGatesRef:x}){o(!1),l(null),c?.(null),xh(x,t),f&&(f.current=null),h?.current===t&&(h.current=null),Xc(m,d,e,a),(n==="failed"||n==="recovery_required")&&$h(i,{runId:e,status:n,failureCategory:r,failureSummary:s})}function o5(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function $h(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`,i=typeof t=="string"&&t?t:null;e(o=>{let l=o.findIndex(d=>d.id===s),c=k2({status:a,failureCategory:n,failureSummary:r});if(l>=0){let d=!!(r&&o[l].content!==c),m=!!(i&&o[l].turnRunId!==i);if(!d&&!m)return o;let f=[...o];return f[l]={...f[l],...d&&{content:c},...m&&{turnRunId:i}},f}return[...o,{id:s,role:"error",content:c,timestamp:new Date().toISOString(),...i&&{turnRunId:i}}]})}function L2(e,t){if(!t)return null;let a=e?.current;if(!a)return null;for(let[n,r]of a.entries())if(n.startsWith(`${t}
`))return l5(r);return null}function l5(e){return e&&typeof e=="object"?{resolution:e.resolution||null,outcome:e.outcome||null}:{resolution:e||null,outcome:null}}function xh(e,t){if(!t)return;let a=e?.current;if(a)for(let n of Array.from(a.keys()))n.startsWith(`${t}
`)&&a.delete(n)}function u5(e,t,a){return!t||!a?!1:!!e?.current?.has(`${t}
${a}`)}function U2(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function j2(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function F2(e,t,a,n){let r=wh(n);return r?(c5(e,t,a,{timelineMessageId:r}),r):null}function c5(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function wh(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var d5=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function B2({threadId:e,onEvent:t,enabled:a}){let[n,r]=p.default.useState("idle"),s=p.default.useRef(t);s.current=t;let i=p.default.useRef(null);return p.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,l=null,c=0,d=3e4;function m(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=Wx({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let y=Math.min(1e3*2**c,d);l=setTimeout(m,y)};let x=(y,$)=>{let g=null;try{g=JSON.parse(y.data)}catch{return}!g||typeof g!="object"||(y.lastEventId&&(i.current=y.lastEventId),s.current?.({type:g.type||$,frame:g,lastEventId:y.lastEventId||null}))};o.onmessage=y=>x(y,"message");for(let y of d5)o.addEventListener(y,$=>x($,y))}function f(){l&&(clearTimeout(l),l=null),o&&(o.close(),o=null),r("paused")}function h(){document.visibilityState==="hidden"?f():o||m()}return m(),document.addEventListener("visibilitychange",h),()=>{document.removeEventListener("visibilitychange",h),l&&clearTimeout(l),o&&o.close()}},[a,e]),{status:n}}var m5=3e4,f5="credential_stored_gate_resolution_failed",p5="approval_gate_pending_send_blocked",h5="ironclaw-product-auth",Sh="ironclaw:product-auth:oauth-complete",v5="ironclaw:product-auth:oauth-complete";async function z2(e){let t=new AbortController,a=setTimeout(()=>t.abort(),m5);try{return await e(t.signal)}finally{clearTimeout(a)}}function g5(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=f5,t.cause=e,t}function y5(){let e=new Error("Resolve the approval request before sending another message.");return e.safeErrorCode=p5,e}function b5(e){let a=Rt.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function q2(e,t){return!e||!t?.runId||!t?.gateRef?null:`${e}
${t.runId}
${t.gateRef}`}function x5(e){return e?.continuation?.type==="turn_gate_resume"}function $5(e){if(e?.outcome)return e.outcome;let t=String(e?.status||"").toLowerCase();return t==="queued"||t==="running"?"resumed":t==="cancelled"||e?.already_terminal===!0?"cancelled":e?.already_terminal===!1?"resumed":null}function I2(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function w5(e){return e?.type===v5&&e?.status==="completed"}function S5(e,t,a){if(!w5(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function Nh(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function N5(e){if(!gh(e))return null;try{let a=(await Rt.fetchQuery({queryKey:["connectable-channels"],queryFn:Vc}))?.channels||[];return $2(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function K2(e){let t=p.default.useRef(e),a=p.default.useRef(new Map),n=p.default.useRef(1),[r,s]=p.default.useState(0),[i,o]=p.default.useState(Date.now()),[l,c]=p.default.useState(null),d=p.default.useRef(l),m=p.default.useCallback(ae=>{let S=typeof ae=="function"?ae(d.current):ae;d.current=S,c(S)},[]);p.default.useEffect(()=>{d.current=l},[l]);let[f,h]=p.default.useState(null),x=p.default.useCallback(()=>a.current.get(e||"__new__")||[],[e]),y=p.default.useCallback(ae=>{let S=e||"__new__";ae.length>0?a.current.set(S,ae):a.current.delete(S)},[e]),{messages:$,hasMore:g,nextCursor:v,isLoading:b,loadError:w,loadHistory:N,seedThreadMessages:C,setMessages:_}=R$(e,{getPendingMessages:x,setPendingMessages:y}),[A,L]=p.default.useState(!1),M=p.default.useRef(A),P=p.default.useCallback(ae=>{let S=typeof ae=="function"?ae(M.current):ae;M.current=S,L(S)},[]),[k,z]=p.default.useState(null),Z=p.default.useRef(k),[ne,de]=p.default.useState(null),fe=p.default.useCallback(ae=>{let S=Z.current,T=typeof ae=="function"?ae(S):ae;Object.is(T,S)||(Z.current=T,z(T))},[]),[Ce,Ue]=p.default.useState(e),gt=p.default.useRef(R2()),mt=p.default.useRef(new Map),tt=p.default.useRef({gateKey:null,credentialRef:null,inFlight:!1}),Ve=p.default.useRef(!1),ft=p.default.useRef(new Set),Tt=p.default.useRef(new Map),sa=p.default.useRef(!1),yt=p.default.useRef(new Set);Ce!==e&&(Ue(e),L(!1),z(null),de(null),c(null),h(null)),p.default.useEffect(()=>{t.current=e},[e]),p.default.useEffect(()=>{Z.current=k},[k]),p.default.useEffect(()=>{M.current=A},[A]),p.default.useEffect(()=>{let ae=q2(e,k);de(S=>S&&S.gateKey!==ae?null:S)},[k,e]),p.default.useEffect(()=>{C2(gt),mt.current.clear()},[e]);let Ea=Math.max(0,Math.ceil((r-i)/1e3)),Ge=k?.runId&&k?.gateRef?`${k.runId}
${k.gateRef}`:null;p.default.useEffect(()=>{if(!r)return;let ae=setInterval(()=>o(Date.now()),250);return()=>clearInterval(ae)},[r]),p.default.useEffect(()=>{tt.current.gateKey!==Ge&&(tt.current={gateKey:Ge,credentialRef:null,inFlight:!1})},[Ge]),p.default.useEffect(()=>{if(!I2(k))return;let ae=Date.now(),S=B=>{S5(B,k,ae)&&(fe(j=>I2(j)?null:j),P(!0))},T=null;typeof window.BroadcastChannel=="function"&&(T=new window.BroadcastChannel(h5),T.onmessage=B=>S(B.data));let E=B=>{B.key===Sh&&S(Nh(B.newValue))};window.addEventListener("storage",E),S(Nh(window.localStorage?.getItem?.(Sh)));let F=window.setInterval(()=>{S(Nh(window.localStorage?.getItem?.(Sh)))},500);return()=>{window.clearInterval(F),T&&T.close(),window.removeEventListener("storage",E)}},[k]);let At=P2({threadId:e,setMessages:_,setIsProcessing:P,setPendingGate:fe,setActiveRun:m,activeRunRef:d,locallyResolvedGatesRef:mt,toolActivityStateRef:gt,onRunSettled:(ae,{success:S})=>{let T=Tt.current.get(ae);T?(Tt.current.delete(ae),ft.current.delete(T)):ae&&sa.current&&yt.current.add(ae),S&&y([]),N(void 0,{preserveClientOnly:!0,finalReplyTimestampByRun:ae&&S?{[ae]:new Date().toISOString()}:null})}}),{status:Ta}=B2({threadId:e,onEvent:At,enabled:!!e}),ia=p.default.useCallback(async(ae,S={})=>{let{threadId:T,attachments:E=[]}=S,F=E.map(y$),B=E.map(b$);if(k||Z.current)throw y5();let j=T||e,G=d.current,ie=!!G&&!!j&&G.threadId===j,$e=M.current&&!!j&&j===e,Dt=!!j&&ft.current.has(j);if(Ve.current||$e||ie||Dt)return null;if(E.length===0){let se=await N5(ae);if(se)return h(se),{channel_connect_action:se}}h(null);let Ee=T||e;if(!Ee){let se=await xc();if(Rt.invalidateQueries({queryKey:["threads"]}),Ee=se?.thread?.thread_id,!Ee)throw new Error("createThread returned no thread_id")}let la=Ee,Xr={id:`pending-${n.current++}`,role:"user",content:ae,attachments:B,timestamp:new Date().toISOString(),isOptimistic:!0},Zr={id:Xr.id,role:"user",content:ae,attachments:B,timestamp:Xr.timestamp,isOptimistic:!0};U2(a.current,la,Xr);let Ja=Xr.id,cr=!e||Ee===e,dr=se=>{cr&&_(se)},Wr=se=>{Ee!==e&&C(Ee,se)},es=se=>{cr&&se()},Sn=cr;Sn&&(ft.current.add(Ee),sa.current=!0),Ve.current=!0,dr(se=>[...se,Zr]),Wr(se=>[...se,Zr]),es(()=>{P(!0),Z.current||fe(null)});try{let se=await Jx({threadId:Ee,content:ae,attachments:F});b5(Ee)&&Rt.invalidateQueries({queryKey:["threads"]});let ts=!1;Sn&&(sa.current=!1),se?.run_id&&Sn?(ts=yt.current.delete(se.run_id),yt.current.clear(),ts?ft.current.delete(Ee):Tt.current.set(se.run_id,Ee)):Sn&&yt.current.clear(),se?.run_id&&cr&&!ts&&m({runId:se.run_id,threadId:se.thread_id||Ee,status:se.status||null,source:"local"});let xl=F2(a.current,la,Ja,se?.accepted_message_ref)||wh(se?.accepted_message_ref);if(xl){let Xa=as=>as.map(Nn=>Nn.id===Ja?{...Nn,timelineMessageId:xl}:Nn);dr(Xa),Wr(Xa)}if(se?.outcome==="rejected_busy"){se?.run_id&&Sn&&Tt.current.delete(se.run_id),Sn&&ft.current.delete(Ee);let Xa=as=>as.map(Nn=>Nn.id===Ja?{...Nn,isOptimistic:!1,status:"error"}:Nn);if(dr(Xa),Wr(Xa),se?.notice){let as=(Di=cr)=>{let vR={id:`system-rejected-${n.current++}`,role:"system",content:se.notice,timestamp:new Date().toISOString(),isOptimistic:!1},Wh=gR=>[...gR,vR];Di&&_(Wh),(!Di||Ee!==e)&&C(Ee,Wh)};if(!t.current||t.current===Ee){let Di=q2(Ee,Z.current);Di?de({gateKey:Di,content:se.notice}):as()}else as(!1)}es(()=>P(!1)),Ve.current=!1}else se?.run_id||(Sn&&ft.current.delete(Ee),Ve.current=!1);return se}catch(se){Sn&&(sa.current=!1,yt.current.clear(),ft.current.delete(Ee)),se.status===429&&s(Date.now()+k5(se));let ts=xl=>xl.map(Xa=>Xa.id===Ja?{...Xa,isOptimistic:!1,status:"error",error:se.message}:Xa);throw dr(ts),Wr(ts),es(()=>P(!1)),Ve.current=!1,se}finally{Ve.current=!1,j2(a.current,la,Ja)}},[e,k,_,C,P,fe,m]),oa=p.default.useCallback(async(ae,S={})=>{if(!k)return;let{runId:T,gateRef:E}=k;if(!T||!E)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");let F=await jp({threadId:e,runId:T,gateRef:E,resolution:ae,always:S.always,credentialRef:S.credentialRef}),B=$5(F);if(mt.current.set(`${T}
${E}`,{resolution:ae,outcome:B}),_5(ae)&&B==="resumed"&&E2(_,k,gt),fe(null),B==="resumed"){P(!0),m({runId:F?.run_id||T,threadId:F?.thread_id||e,status:F?.status||"queued"});return}P(!1),m(null)},[k,e,_,m]),Gr=p.default.useCallback(async ae=>{if(!k)throw new Error("auth gate is no longer pending");let{runId:S,gateRef:T,provider:E}=k;if(!S||!T||!E)throw new Error("auth gate is missing required credential metadata");let F=k.accountLabel||`${E} credential`,B=`${S}
${T}`;if(tt.current.gateKey!==B&&(tt.current={gateKey:B,credentialRef:null,inFlight:!1}),tt.current.inFlight)throw new Error("auth token submission already in progress");tt.current.inFlight=!0;try{let j=tt.current.credentialRef,G=null;if(!j){if(G=await z2(ie=>t$({provider:E,accountLabel:F,token:ae,threadId:e,runId:S,gateRef:T,signal:ie})),j=G?.credential_ref,!j)throw new Error("manual token submit returned no credential_ref");tt.current.credentialRef=j}if(!x5(G))try{await z2(ie=>jp({threadId:e,runId:S,gateRef:T,resolution:"credential_provided",credentialRef:j,signal:ie}))}catch(ie){throw g5(ie)}tt.current={gateKey:null,credentialRef:null,inFlight:!1},fe(null),P(!0)}catch(j){throw tt.current.gateKey===B&&(tt.current.inFlight=!1),j}},[k,e]),Yr=p.default.useCallback(async ae=>{let S=l?.runId;if(!S||!e)return;fe(null),P(!1),m(null),Ve.current=!1,sa.current=!1,yt.current.clear();let T=Tt.current.get(S)||l?.threadId||e;Tt.current.delete(S),T&&ft.current.delete(T),await e$({threadId:e,runId:S,reason:ae})},[l,e]),ur=p.default.useCallback(()=>{g&&v&&N(v)},[g,v,N]),Ai=p.default.useCallback(async(ae,S,T)=>{let E="approved",F=!1;S==="deny"?E="denied":S==="cancel"?E="cancelled":S==="always"&&(E="approved",F=!0),await oa(E,{always:F})},[oa]),Jr=p.default.useCallback(()=>{},[]);return{messages:$,isProcessing:A,pendingGate:k,busyGateNotice:ne,channelConnectAction:f,activeRun:l,sseStatus:Ta,historyLoading:b,historyLoadError:w,hasMore:g,cooldownSeconds:Ea,send:ia,resolveGate:oa,submitAuthToken:Gr,cancelRun:Yr,loadMore:ur,dismissChannelConnectAction:()=>h(null),suggestions:[],setSuggestions:Jr,retryMessage:Jr,approve:Ai,recoverHistory:Jr,recoveryNotice:null}}function _5(e){return e==="denied"||e==="cancelled"}function k5(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function H2({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}var R5=1500;function Q2({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let o=R(),{messages:l,isProcessing:c,pendingGate:d,busyGateNotice:m,channelConnectAction:f,suggestions:h,sseStatus:x,historyLoading:y,historyLoadError:$,hasMore:g,cooldownSeconds:v,recoveryNotice:b,activeRun:w,send:N,cancelRun:C,retryMessage:_,approve:A,recoverHistory:L,loadMore:M,setSuggestions:P,submitAuthToken:k,dismissChannelConnectAction:z}=K2(t),Z=p.default.useMemo(()=>e.find(Ge=>Ge.id===t)||null,[e,t]),ne=p.default.useMemo(()=>H2({gatewayStatus:i,activeThread:Z}),[i,Z]),de=l.length>0||c||!!d||!!f,fe=!y&&!de&&!$,Ce=d?"Resolve the approval request before sending another message.":"",Ue=!!d||c&&!d||v>0,gt=p.default.useRef(Ue);gt.current=Ue;let mt=Ce||(v>0?`Retry in ${v}s`:void 0),tt=t||nl,Ve=!!(t&&w?.runId&&w.threadId===t&&c&&!d),ft=p.default.useCallback(async(Ge,{images:At=[],attachments:Ta=[]}={})=>{if(d)throw new Error(Ce);if(gt.current)return null;let ia=await N(Ge,{images:At,attachments:Ta,threadId:t}),oa=ia?.thread_id||t;return!t&&oa&&a&&a(oa,{replace:!0}),ia},[t,Ce,Ue,a,d,N]),Tt=p.default.useCallback(async Ge=>{Ue||(P([]),await ft(Ge))},[Ue,ft,P]),sa=p.default.useCallback(()=>C("user_requested"),[C]);p.default.useEffect(()=>{if(!t)return;if(d){Tc(t,bn.NEEDS_ATTENTION);return}if(c){Tc(t,bn.RUNNING);return}let Ge=setTimeout(()=>jw(t),R5);return()=>clearTimeout(Ge)},[t,d,c]);let[yt,Ea]=p.default.useState(!1);return p.default.useEffect(()=>{let Ge=At=>{if(At.key==="Escape"){Ea(!1);return}if(At.key!=="?")return;let Ta=At.target,ia=Ta?.tagName;ia==="INPUT"||ia==="TEXTAREA"||Ta?.isContentEditable||(At.preventDefault(),Ea(oa=>!oa))};return window.addEventListener("keydown",Ge),()=>window.removeEventListener("keydown",Ge)},[]),u`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${z1} status=${x} />

        ${$&&u`
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            ${$}
          </div>
        `}

        ${fe&&u`
          <${q1}
            onSuggestion=${Tt}
            onSend=${ft}
            disabled=${!1}
            sendDisabled=${Ue}
            initialText=${r}
            resetKey=${s}
            draftKey=${tt}
            context=${ne}
            statusText=${mt}
            canCancel=${Ve}
            onCancel=${sa}
          />
        `}
        ${!fe&&u`
          <${g2}
            messages=${l}
            isLoading=${y}
            hasMore=${g}
            onLoadMore=${M}
            onRetryMessage=${_}
            threadId=${t}
            pending=${c}
          >
            ${b&&u`
              <${y2}
                notice=${b}
                onRecover=${L}
              />
            `}
            ${c&&!d&&u`<${x2} />`}
            ${f&&u`
              <${j1}
                connectAction=${f}
                onDismiss=${z}
              />
            `}
            ${d&&(d.kind==="auth_required"?d.challengeKind==="oauth_url"?u`
                  <${L1}
                    gate=${d}
                    onCancel=${()=>A(d.requestId,"cancel",d.kind)}
                  />
                `:d.challengeKind==="manual_token"?u`
                  <${P1}
                    gate=${d}
                    onSubmit=${k}
                    onCancel=${()=>A(d.requestId,"cancel",d.kind)}
                  />
                `:u`
                  <${O1}
                    gate=${d}
                    onCancel=${()=>A(d.requestId,"cancel",d.kind)}
                  />
                `:u`
              <${M1}
                gate=${d}
                onApprove=${()=>A(d.requestId,"approve",d.kind)}
                onDeny=${()=>A(d.requestId,"deny",d.kind)}
                onAlways=${()=>A(d.requestId,"always",d.kind)}
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

          <${b2}
            suggestions=${h}
            onSelect=${Tt}
            disabled=${Ue}
          />

          <${qc}
            onSend=${ft}
            disabled=${!1}
            sendDisabled=${Ue}
            initialText=${r}
            resetKey=${s}
            draftKey=${tt}
            context=${ne}
            statusText=${mt}
            canCancel=${Ve}
            onCancel=${sa}
          />
        `}
      </div>
      <${I1}
        open=${yt}
        onClose=${()=>Ea(!1)}
      />
    </div>
  `}function _h(){let{threadsState:e,gatewayStatus:t}=ba(),{threadId:a}=st(),n=me(),r=Le(),s=r.state?.composerDraft||"",i=a||null;p.default.useEffect(()=>{i&&i!==e.activeThreadId?e.setActiveThreadId(i):i||e.setActiveThreadId(null)},[i]);let o=p.default.useCallback((l,c={})=>{if(!l){e.setActiveThreadId(null),n("/chat",c);return}e.setActiveThreadId(l),n(`/chat/${l}`,c)},[e,n]);return u`
    <${Q2}
      threads=${e.threads}
      activeThreadId=${i}
      onSelectThread=${o}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function V2(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?oi(e,t):"",model:e?Cc(e,t):""}}function G2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:l}){let[c,d]=p.default.useState(()=>V2(e,a)),[m,f]=p.default.useState(""),[h,x]=p.default.useState([]),[y,$]=p.default.useState(null),[g,v]=p.default.useState(""),b=p.default.useRef(!!e);p.default.useEffect(()=>{n&&(d(V2(e,a)),f(""),x([]),$(null),v(""),b.current=!!e)},[n,e,a]);let w=e?.builtin===!0,N=e&&!e.builtin,C=p.default.useCallback((P,k)=>{d(z=>{let Z={...z,[P]:k};return P==="name"&&!b.current&&(Z.id=pw(k)),Z})},[]),_=p.default.useCallback(()=>!w&&(!c.name.trim()||!c.id.trim())?l("llm.fieldsRequired"):!w&&!hw(c.id.trim())?l("llm.invalidId"):!N&&!w&&t.includes(c.id.trim())?l("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,w,N,l]),A=p.default.useCallback(async()=>{let P=_();if(P){$({tone:"error",text:P});return}v("save");try{await s({form:c,apiKey:m,provider:e}),r()}catch(k){$({tone:"error",text:k.message})}finally{v("")}},[m,c,r,s,e,_]),L=p.default.useCallback(async()=>{if(!c.model.trim()){$({tone:"error",text:l("llm.modelRequired")});return}v("test");try{let P=await i(Wp(e,c,m,a));$({tone:P.ok?"success":"error",text:P.message})}catch(P){$({tone:"error",text:P.message})}finally{v("")}},[m,a,c,i,e,l]),M=p.default.useCallback(async()=>{if((w?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){$({tone:"error",text:l("llm.baseUrlRequired")});return}v("models");try{let k=await o(Wp(e,c,m,a));if(!k.ok||!Array.isArray(k.models)||!k.models.length)$({tone:"error",text:k.message||l("llm.modelsFetchFailed")});else{x(k.models);let z=vw(c.model,k.models);z!==null&&C("model",z),$({tone:"success",text:l("llm.modelsFetched",{count:k.models.length})})}}catch(k){$({tone:"error",text:k.message})}finally{v("")}},[m,a,c,w,o,e,l,C]);return{form:c,apiKey:m,models:h,message:y,busy:g,isBuiltin:w,isEditing:N,setApiKey:f,update:C,submit:A,runTest:L,fetchModels:M,markIdEdited:()=>{b.current=!0}}}function Zc({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let l=R(),c=G2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:l});if(!n)return null;let{form:d,apiKey:m,models:f,message:h,busy:x,isBuiltin:y,isEditing:$}=c,g=y?l("llm.configureProvider",{name:e.name||e.id}):l($?"llm.editProvider":"llm.newProvider");return u`
    <${pi} open=${n} onClose=${r} title=${g} size="lg">
      <${hi} className="space-y-4">
        ${!y&&u`
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${l("llm.providerName")}
              <${Et} value=${d.name} onChange=${v=>c.update("name",v.target.value)} />
            </label>
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${l("llm.providerId")}
              <${Et}
                value=${d.id}
                disabled=${$}
                onChange=${v=>{c.markIdEdited(),c.update("id",v.target.value)}}
              />
            </label>
          </div>
          <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
            ${l("llm.adapter")}
            <${mh} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${Zp.map(v=>u`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
            <//>
          </label>
        `}

        ${y&&u`
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            ${il(e.adapter)}
          </div>
        `}

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${l("llm.baseUrl")}
          <${Et} value=${d.baseUrl} placeholder=${e?.base_url||""} onChange=${v=>c.update("baseUrl",v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${l("llm.apiKey")}
          <${Et} type="password" value=${m} placeholder=${l("llm.apiKeyPlaceholder")} onChange=${v=>c.setApiKey(v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${l("llm.defaultModel")}
          <div className="flex items-stretch gap-2">
            <${Et} value=${d.model} onChange=${v=>c.update("model",v.target.value)} />
            <${D} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${x!==""} onClick=${c.fetchModels}>
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
      <${vi}>
        <${D} type="button" variant="secondary" disabled=${x!==""} onClick=${c.runTest}>
          ${l(x==="test"?"llm.testing":"llm.testConnection")}
        <//>
        <${D} type="button" variant="ghost" disabled=${x!==""} onClick=${r}>${l("common.cancel")}<//>
        <${D} type="button" disabled=${x!==""} onClick=${c.submit}>
          ${l(x==="save"?"common.saving":"common.save")}
        <//>
      <//>
    <//>
  `}function Wc({login:e}){let t=R(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return u`
    ${a&&u`<div className="text-center text-xs text-[var(--v2-text-muted)]">
      ${t("onboarding.nearaiWaiting")}
    </div>`}
    ${n&&u`<div className="text-center text-xs text-red-300">${n}</div>`}

    ${i&&u`<div
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
    ${r&&u`<div className="text-center text-xs text-[var(--v2-text-muted)]">
      ${t("onboarding.codexWaiting")}
    </div>`}
    ${s&&u`<div className="text-center text-xs text-red-300">${s}</div>`}
  `}function C5(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function ed({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=li({settings:e,gatewayStatus:t}),[s,i]=p.default.useState(null),[o,l]=p.default.useState(!1),[c,d]=p.default.useState(null),m=p.default.useRef(null),f=p.default.useCallback((g,v)=>{m.current&&window.clearTimeout(m.current),d({tone:g,text:v}),m.current=window.setTimeout(()=>d(null),3500)},[]);p.default.useEffect(()=>()=>{m.current&&window.clearTimeout(m.current)},[]);let h=p.default.useCallback((g=null)=>{i(g),l(!0)},[]),x=p.default.useCallback(async g=>{try{await r.setActiveProvider(g),f("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(h(g),f("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):f("error",v.message)}},[h,r,f,n]),y=p.default.useCallback(async({form:g,apiKey:v,provider:b})=>{if(b?.builtin){await r.saveBuiltinProvider({provider:b,form:g,apiKey:v}),f("success",n("llm.providerConfigured",{name:b.name||b.id}));return}let w=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:b});f("success",n(b?"llm.providerUpdated":"llm.providerAdded",{name:w.name||w.id}))},[r,f,n]),$=p.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),f("success",n("llm.providerDeleted"))}catch(v){f("error",v.message)}},[r,f,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>C5(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:h,closeDialog:()=>l(!1),handleUse:x,handleSave:y,handleDelete:$}}var E5=3e5;function T5(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function A5(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function D5(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=l=>{let c=l.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},E5);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var M5=3e5,O5=9e5,L5=2e3;async function Y2(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,L5)),(await Rc().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function td({onSuccess:e}={}){let t=R(),a=J(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),[o,l]=p.default.useState(!1),[c,d]=p.default.useState(""),[m,f]=p.default.useState(null),h=p.default.useCallback(()=>{i(""),d(""),f(null)},[]),x=p.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),y=p.default.useCallback(async v=>{if(h(),T5()){i(t("onboarding.nearaiLocalSso"));return}let b=window.open("about:blank","_blank");if(!b){i(t("onboarding.nearaiFailed"));return}try{b.opener=null}catch{}r(!0);try{let{auth_url:w}=await Q$({provider:v,origin:window.location.origin});b.location.href=w;let N=await Y2("nearai",M5,b);if(N==="active"){await x();return}b.close(),i(t(N==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{b.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,h,t]),$=p.default.useCallback(async()=>{h(),r(!0);try{let v=A5(),b=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!b){i(t("onboarding.nearaiFailed"));return}b.opener=null;let w=await D5(b,v);if(!w){i(t("onboarding.nearaiFailed"));return}await V$({account_id:w.accountId,public_key:w.publicKey,signature:w.signature,message:w.message,recipient:w.recipient,nonce:w.nonce}),await x()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,h,t]),g=p.default.useCallback(async()=>{h();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}l(!0);try{let{user_code:b,verification_uri:w}=await G$();f({userCode:b,verificationUri:w}),v&&(v.location.href=w);let N=await Y2("openai_codex",O5,v);if(N==="active"){await x();return}v&&v.close(),d(t(N==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{l(!1)}},[x,h,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:m,startNearai:y,startNearaiWallet:$,startCodex:g}}var J2="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",P5="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",U5="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",j5="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",F5={nearai:{color:"#00ec97",path:P5},openai_codex:{color:"#10a37f",path:J2},openai:{color:"#10a37f",path:J2},anthropic:{color:"#d97757",path:U5},ollama:{color:null,path:j5}};function X2({id:e,name:t}){let a=F5[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return u`
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
  `}var B5=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function z5({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=p.default.useState(!1),o=p.default.useRef(null),l=t||a.nearaiBusy;p.default.useEffect(()=>{if(!s)return;let d=f=>{o.current&&!o.current.contains(f.target)&&i(!1)},m=f=>{f.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",m),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",m)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return u`
    <div ref=${o} className="relative shrink-0">
      <${D}
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
        <${O} name="chevron" className="h-3.5 w-3.5" />
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
  `}function q5({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let l=s(e.nameKey),c;return e.auth==="nearai"?c=u`<${z5} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=u`
      <${D} type="button" variant="secondary" size="sm" disabled=${r.codexBusy} onClick=${r.startCodex}>
        ${s("onboarding.signIn")}
      <//>
    `:a?c=u`<${D} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>i(t)}>
      ${s("llm.use")}
    <//>`:c=u`<${D} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>o(t)}>
      ${s("onboarding.setUp")}
    <//>`,u`
    <${ee} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-4">
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <${X2} id=${e.id} name=${l} />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">${l}</span>
            ${a&&u`<${q} tone="positive" label=${s("onboarding.ready")} size="sm" />`}
          </div>
          <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">${s(e.descKey)}</div>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap gap-2 sm:justify-end">${c}</div>
    <//>
  `}function Z2(){let{isAdmin:e=!1,isChecking:t=!1}=ba();return t?null:e?u`<${I5} />`:u`<${it} to="/chat" replace />`}function I5(){let e=R(),t=me(),a=J(),{gatewayStatus:n}=ba(),r=ed({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=B5.map(m=>({entry:m,provider:s.providers.find(f=>f.id===m.id)})).filter(m=>m.provider),o=p.default.useCallback(()=>t("/chat"),[t]),l=td({onSuccess:o}),c=p.default.useCallback(async m=>{let f=m.active_model||m.default_model||"";await sl({provider_id:m.id,model:f}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=p.default.useCallback(async({form:m,apiKey:f,provider:h})=>{await r.handleSave({form:m,apiKey:f,provider:h});let x=h?.id||m.id.trim(),y=m.model?.trim()||h?.default_model||"";await sl({provider_id:x,model:y}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?u`
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
              <${q5}
                key=${m.id}
                entry=${m}
                provider=${f}
                configured=${Br(f,s.builtinOverrides)}
                isBusy=${s.isBusy}
                login=${l}
                t=${e}
                onUse=${c}
                onSetUp=${r.openDialog}
              />
            `)}
        </div>

        <${Wc} login=${l} />

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

      <${Zc}
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
  `}function I({children:e,className:t="",...a}){return u`<${ee} className=${t} ...${a}>${e}<//>`}function We({label:e,value:t,tone:a="muted",badgeLabel:n,detail:r,showDivider:s=!0,className:i="",valueClassName:o="text-[1.75rem] md:text-[2rem]"}){return u`
    <div
      className=${V("px-1 py-4",s&&"border-t border-[var(--v2-panel-border)]",i)}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div
            className="font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]"
          >
            ${e}
          </div>
          <div
            className=${V("mt-3 truncate font-medium tracking-[-0.05em] text-[var(--v2-text-strong)]",o)}
          >
            ${t}
          </div>
          ${r&&u`<div className="mt-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            ${r}
          </div>`}
        </div>
        <${q} tone=${a} label=${n??a} />
      </div>
    </div>
  `}function W2({items:e}){return u`
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
  `}function ye({title:e,description:t,children:a,boxed:n=!0}){let r=u`
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
  `;return n?u`<${ee} padding="lg">${r}<//>`:u`<div className="py-8">${r}</div>`}var eS={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function Ga({result:e,onDismiss:t}){return e?u`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",eS[e.type]||eS.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}var tS="",K5={workspace:"home"};function ad(e){return K5[e]||e}function fl(e){return[...e||[]].sort((t,a)=>t.is_dir!==a.is_dir?t.is_dir?-1:1:t.name.localeCompare(a.name,void 0,{sensitivity:"base"}))}function yi(e){return e?e.split("/").filter(Boolean):[]}function nd(e){return e?`/workspace/${yi(e).map(encodeURIComponent).join("/")}`:"/workspace"}function kh(e){let t=yi(e);return t.pop(),t.join("/")}function aS(e){return/\.mdx?$/i.test(e||"")}function rd({path:e,onNavigate:t}){let a=R(),n=yi(e),r="";return u`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button
        type="button"
        onClick=${()=>t("/workspace")}
        className="text-signal hover:underline"
      >
        ${a("workspace.breadcrumbRoot")}
      </button>
      ${n.map((s,i)=>{r=r?`${r}/${s}`:s;let o=r,l=i===0?ad(s):s;return u`
          <span key=${o} className="text-iron-400">/</span>
          <button
            key=${`${o}-button`}
            type="button"
            onClick=${()=>t(nd(o))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${l}
          </button>
        `})}
    </div>
  `}function H5(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function nS({path:e,entries:t,isLoading:a,filter:n,onOpen:r,onNavigate:s}){let i=R();if(a)return u`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;let o=(t||[]).filter(f=>!H5(f.path)),l=String(n||"").trim().toLowerCase(),c=l?o.filter(f=>f.name.toLowerCase().includes(l)):o,d=fl(c),m;return o.length?d.length?m=u`
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
    <${I} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${rd} path=${e} onNavigate=${s} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">${m}</div>
    <//>
  `}var sd="/api/webchat/v2/fs",Q5=1024*1024,V5=8*1024*1024;function rS(e){let t=String(e||"").split("/").filter(Boolean);return{mount:t.shift()||"",path:t.join("/")}}function G5(e,t){return t?`${e}/${t}`:e}function Y5(e){let t=String(e||"").toLowerCase();return t.startsWith("text/")||t==="application/json"||t==="application/javascript"||t==="application/xml"||t.endsWith("+json")||t.endsWith("+xml")}function J5(e){return String(e||"").toLowerCase().startsWith("image/")}function X5(e){let t=String(e||"").toLowerCase();return t.startsWith("audio/")||t.startsWith("video/")||t.startsWith("font/")||t==="application/pdf"||t==="application/zip"||t==="application/gzip"}function Z5(e){if(e.subarray(0,Math.min(e.length,8192)).indexOf(0)!==-1)return!0;try{return new TextDecoder("utf-8",{fatal:!0}).decode(e),!1}catch{return!0}}function W5(e,t){let a=new URL(`${sd}/content`,window.location.origin);return a.searchParams.set("mount",e),a.searchParams.set("path",t),a.pathname+a.search}async function eD(){return(await H(`${sd}/mounts`))?.mounts||[]}async function bi(e=""){if(!e)return{entries:(await eD()).map(o=>({name:ad(o.mount),path:o.mount,is_dir:!0}))};let{mount:t,path:a}=rS(e),n=new URL(`${sd}/list`,window.location.origin);return n.searchParams.set("mount",t),a&&n.searchParams.set("path",a),{entries:((await H(n.pathname+n.search))?.entries||[]).map(i=>({name:i.name,path:G5(t,i.path),is_dir:i.kind==="directory"}))}}async function sS(e){let{mount:t,path:a}=rS(e);if(!t||!a)return{kind:"directory",path:e};let n=new URL(`${sd}/stat`,window.location.origin);n.searchParams.set("mount",t),n.searchParams.set("path",a);let s=(await H(n.pathname+n.search))?.stat||{},i=s.mime_type||"application/octet-stream",o=Number(s.size_bytes||0),l=W5(t,a),c={path:e,mime:i,size_bytes:o,download_path:l};if(s.kind&&s.kind!=="file")return{...c,kind:"directory"};if(J5(i)){if(o>V5)return{...c,kind:"binary"};let h=await wc(l);return{...c,kind:"image",image_data_url:h}}if(X5(i)||o>Q5)return{...c,kind:"binary"};let d=await _a(l),m=new Uint8Array(await d.arrayBuffer());if(!Y5(i)&&Z5(m))return{...c,kind:"binary"};let f=new TextDecoder("utf-8").decode(m);return{...c,kind:"text",content:f}}function iS(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function tD(e,t,a){let n=String(t||"").trim().toLowerCase(),r=(e||[]).filter(s=>!iS(s.path)).filter(s=>!n||s.name.toLowerCase().includes(n)?!0:s.is_dir&&a.has(s.path));return fl(r)}function oS({entry:e,depth:t,selectedPath:a,expandedPaths:n,filter:r,onToggleDirectory:s,onSelectFile:i}){let o=R(),l=n.has(e.path),c=K({queryKey:["workspace-list",e.path],queryFn:()=>bi(e.path),enabled:e.is_dir&&l});if(e.is_dir){let d=tD(c.data?.entries,r,n);return u`
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
                  <${oS}
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
  `}function lS({entries:e,selectedPath:t,expandedPaths:a,filter:n,onToggleDirectory:r,onSelectFile:s,isLoading:i}){let o=R();if(i)return u`<div className="space-y-2 p-3">${[1,2,3,4].map(c=>u`<div key=${c} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let l=fl(e.filter(c=>!iS(c.path)));return l.length?u`
    <div className="space-y-1 p-2">
      ${l.map(c=>u`
        <${oS}
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
  `:u`<div className="px-4 py-8 text-sm text-iron-300">${o("workspace.noFiles")}</div>`}function uS({rootEntries:e,selectedPath:t,expandedPaths:a,filter:n,onFilterChange:r,isLoadingTree:s,onToggleDirectory:i,onSelectFile:o}){let l=R();return u`
    <${I} className="flex min-h-[420px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="border-b border-white/10 p-3">
        <input
          value=${n}
          onInput=${c=>r(c.target.value)}
          placeholder=${l("workspace.filterPlaceholder")}
          className="h-9 w-full rounded-md border border-white/10 bg-iron-950/80 px-3 text-sm text-white outline-none placeholder:text-iron-400 focus:border-signal/45"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <${lS}
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
  `}function cS(e){return yi(e).pop()||"download"}function aD({path:e,file:t}){let a=R();return t.kind==="image"?u`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${t.image_data_url}
          alt=${cS(e)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `:t.kind==="text"?u`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${aS(e)?u`<${aa} content=${t.content} className="max-w-4xl text-base leading-7" />`:u`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
      </div>
    `:u`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${a("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `}function dS({path:e,file:t,isLoading:a,onNavigate:n}){let r=R(),[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(t?.download_path){i(!0);try{let c=await _a(t.download_path);Ic(c,cS(e))}catch{}finally{i(!1)}}},[t,e]);if(a)return u`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;if(!t||t.kind==="directory")return u`
      <${ye}
        title=${r("workspace.pickFileTitle")}
        description=${r("workspace.pickFileDesc")}
      />
    `;let l=r("workspace.fileMeta",{mime:t.mime||"application/octet-stream",size:Number(t.size_bytes||0)});return u`
    <${I} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${rd} path=${e} onNavigate=${n} />
        <div className="flex items-center gap-2">
          <${q} tone="muted" label=${l} />
          <${D}
            variant="secondary"
            size="sm"
            onClick=${o}
            disabled=${s}
          >${r("workspace.download")}<//>
        </div>
      </div>

      <${aD} path=${e} file=${t} />

      ${kh(e)&&u`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${r("workspace.parent",{path:kh(e)})}
        </div>
      `}
    <//>
  `}function mS(e){let t=R(),a=J(),[n,r]=p.default.useState(new Set),[s,i]=p.default.useState(""),[o,l]=p.default.useState(null),c=K({queryKey:["workspace-list",""],queryFn:()=>bi("")}),d=K({queryKey:["workspace-file",e],queryFn:()=>sS(e),enabled:!!e}),m=e===""||d.data?.kind==="directory",f=K({queryKey:["workspace-list",e],queryFn:()=>bi(e),enabled:m});p.default.useEffect(()=>{l(null)},[e]);let h=p.default.useCallback(y=>a.fetchQuery({queryKey:["workspace-list",y],queryFn:()=>bi(y)}),[a]),x=p.default.useCallback(async y=>{let $=new Set(n);if($.has(y)){$.delete(y),r($);return}$.add(y),r($);try{await h(y)}catch(g){l({type:"error",message:g.message||t("workspace.unableOpenDirectory")})}},[n,h,t]);return{rootEntries:c.data?.entries||[],file:d.data||null,selectionIsDirectory:m,currentEntries:f.data?.entries||[],expandedPaths:n,filter:s,setFilter:i,result:o,clearResult:()=>l(null),isLoadingTree:c.isLoading,isLoadingFile:d.isLoading,isLoadingListing:f.isLoading,isFetching:c.isFetching||d.isFetching||f.isFetching,error:c.error||d.error||f.error||null,loadDirectory:h,toggleDirectory:x,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function Rh(){let e=R(),t=me(),n=st()["*"]||tS,r=mS(n),s=p.default.useCallback(i=>{t(nd(i))},[t]);return u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="flex h-full min-h-0 flex-col space-y-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1 className="text-lg font-semibold text-white">${e("workspace.title")}</h1>
                <${q} tone="muted" label=${e("workspace.readOnly")} />
              </div>
              <p className="mt-0.5 text-sm text-iron-400">${e("workspace.subtitle")}</p>
            </div>
            <${D}
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
          <${Ga}
            result=${r.result}
            onDismiss=${r.clearResult}
          />

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <${uS}
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
                  <${nS}
                    path=${n}
                    entries=${r.currentEntries}
                    isLoading=${r.isLoadingListing}
                    filter=${r.filter}
                    onOpen=${s}
                    onNavigate=${t}
                  />
                `:u`
                  <${dS}
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
  `}function fS(e){if(!e)return null;let t=e.metadata&&typeof e.metadata=="object"&&!Array.isArray(e.metadata)?e.metadata:{};return{id:e.project_id,name:e.name,description:e.description,goals:Array.isArray(t.goals)?t.goals:[],icon:e.icon||null,color:e.color||null,state:e.state,role:e.role,metadata:t,created_at:e.created_at,updated_at:e.updated_at,health:e.state==="archived"?"muted":"green"}}async function pS(){let t=((await Kx({limit:200}))?.projects||[]).map(fS);return{attention:[],projects:t}}async function hS(e){if(!e)return null;let t=await Hx({projectId:e});return fS(t?.project)}function vS(e){return Promise.resolve({missions:[],todo:!0})}function gS(e){return Promise.resolve({threads:[],todo:!0})}function yS(e){return Promise.resolve({widgets:[],todo:!0})}function bS(e){return Promise.resolve(null)}function xS(e){return Promise.resolve(null)}function $S(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function wS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function SS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function NS(){let e=J(),t=K({queryKey:["projects-overview"],queryFn:pS,refetchInterval:5e3}),a=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function _S(e){let t=J(),a=!!e,n=K({queryKey:["project-detail",e],queryFn:()=>hS(e),enabled:a,refetchInterval:a?7e3:!1}),r=K({queryKey:["project-missions",e],queryFn:()=>vS(e),enabled:a,refetchInterval:a?5e3:!1}),s=K({queryKey:["project-threads",e],queryFn:()=>gS(e),enabled:a,refetchInterval:a?4e3:!1}),i=K({queryKey:["project-widgets",e],queryFn:()=>yS(e),enabled:a,refetchInterval:a?15e3:!1}),o=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function kS({projectId:e,missionId:t,threadId:a}){let n=J(),[r,s]=p.default.useState(null),i=K({queryKey:["project-mission-detail",t],queryFn:()=>bS(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=K({queryKey:["project-thread-detail",a],queryFn:()=>xS(a),enabled:!!a,refetchInterval:a?4e3:!1}),l=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=Q({mutationFn:({targetMissionId:f})=>$S(f),onSuccess:f=>{s({type:"success",message:f?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to fire mission"})}}),d=Q({mutationFn:({targetMissionId:f})=>wS(f),onSuccess:()=>{s({type:"success",message:"Mission paused."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to pause mission"})}}),m=Q({mutationFn:({targetMissionId:f})=>SS(f),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending}}function id(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function od(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function RS(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function CS(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function nD(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function ES(e){let t=nD(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function TS(e){let t=e?.projects||[],a=t.reduce((o,l)=>o+Number(l.cost_today_usd||0),0),n=t.reduce((o,l)=>o+Number(l.active_missions||0),0),r=t.reduce((o,l)=>o+Number(l.threads_today||0),0),s=t.reduce((o,l)=>o+Number(l.pending_gates||0),0),i=t.reduce((o,l)=>o+Number(l.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function pl(e,t){return`${e} ${t}${e===1?"":"s"}`}var rD={projects:"muted",attention:"warning",spend:"success"};function AS({overview:e}){let t=TS(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:od(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return u`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>u`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${q} tone=${rD[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function sD(e){return e?.type==="failure"?"danger":"warning"}function iD(e){return e?.type==="failure"?"failure":"gate"}function DS({items:e,onOpenItem:t}){return e?.length?u`
    <${I} className="overflow-hidden border-amber-300/10 p-0">
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
              <${q} tone=${sD(a)} label=${iD(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function oD({project:e,onOpen:t,t:a}){return u`
    <article
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
        <${q} tone=${RS(e.health)} label=${e.health||"unknown"} />
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
            ${a("projects.card.threadsToday",{count:pl(e.threads_today||0,"thread")})}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${a("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">${pl(e.pending_gates||0,"gate")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${a("projects.card.failures24h",{count:pl(e.failures_24h||0,"failure")})}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>${a("projects.card.spendToday",{value:od(e.cost_today_usd||0)})}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${id(e.last_activity)}</div>
        </div>
        <${D}
          variant="secondary"
          onClick=${n=>{n.stopPropagation(),t(e.id)}}
        >${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function lD({project:e,onOpen:t,t:a}){return u`
    <${I}
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
            ${pl(e.threads_today||0,"thread")} today
          </div>
          <${D}
            variant="secondary"
            onClick=${n=>{n.stopPropagation(),t(e.id)}}
          >${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function MS({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=R(),l=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return!e.length&&t>0?u`
      <${ye}
        title=${o("projects.empty.noMatchTitle")}
        description=${o("projects.empty.noMatchDesc")}
      />
    `:e.length?u`
    <div className="space-y-5">
      ${l&&u`<${lD} project=${l} onOpen=${r} t=${o} />`}

      <${I} className="p-4 sm:p-5">
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
              value=${a}
              onInput=${d=>n(d.target.value)}
              placeholder=${o("projects.searchPlaceholder")}
              className="h-11 min-w-[220px] rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            />
            <${D} onClick=${s}>${o(i?"projects.preparingChat":"projects.newProject")}<//>
          </div>
        </div>
      <//>

      ${c.length?u`<div className="grid gap-4 xl:grid-cols-2 2xl:grid-cols-3">
            ${c.map(d=>u`<${oD} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
          </div>`:u`
            <${ye}
              title=${o("projects.scoped.onlyGeneralTitle")}
              description=${o("projects.scoped.onlyGeneralDesc")}
            >
              <${D} onClick=${s}>${o(i?"projects.preparingChat":"projects.startProject")}<//>
            <//>
          `}
    </div>
  `:u`
      <${ye}
        title=${o("projects.empty.noneTitle")}
        description=${o("projects.empty.noneDesc")}
      >
        <${D} onClick=${s}>${o("projects.createFromChat")}<//>
      <//>
    `}function OS({threads:e,selectedThreadId:t,onSelectThread:a,onNewConversation:n,isStartingConversation:r}){let s=[...e].sort((i,o)=>new Date(o.updated_at||o.created_at)-new Date(i.updated_at||i.created_at));return u`
    <${I} className="p-4 sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Conversations</div>
          <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">Project conversations</h2>
        </div>
        ${n&&u`
          <${D} onClick=${n} disabled=${r}>
            ${r?"Starting\u2026":"New conversation"}
          <//>
        `}
      </div>

      <div className="mt-5 space-y-3">
        ${s.length?s.slice(0,18).map(i=>{let o=ES(i);return u`
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
                    <${q} tone=${CS(i.state)} label=${i.state} />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                    <span>${i.step_count||0} steps</span>
                    <span>${i.total_tokens||0} tokens</span>
                    <span>${id(i.updated_at||i.created_at)}</span>
                  </div>
                </button>
              `}):u`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                No project threads yet. When an automation runs or scoped chat work happens inside this project, activity will appear here.
              </div>
            `}
      </div>
    <//>
  `}var uD="/workspace";function cD(e){let t=a=>a.kind==="directory"?0:1;return[...e].sort((a,n)=>t(a)-t(n)||a.name.localeCompare(n.name,void 0,{sensitivity:"base"}))}function dD(e){return e?String(e).replace(/^\/workspace\/?/,"").split("/").filter(Boolean):[]}function LS({threadId:e}){let t=R(),[a,n]=p.default.useState(void 0),[r,s]=p.default.useState(null),i=K({queryKey:["project-files",e||"",a||""],queryFn:()=>Ux({threadId:e,path:a}),enabled:!!e}),o=p.default.useMemo(()=>cD(i.data?.entries||[]),[i.data]),l=p.default.useCallback(async m=>{if(m.kind==="directory"){s(null),n(m.path);return}try{s(null);let f=await _a($c({threadId:e,path:m.path})),h=URL.createObjectURL(f),x=document.createElement("a");x.href=h,x.download=m.name,document.body.appendChild(x),x.click(),x.remove(),URL.revokeObjectURL(h)}catch(f){s(f?.message||"Unable to download file")}},[e]),c=dD(a),d=u`
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
          ${"Files"}
        </div>
        <${q} tone="muted" label=${t("workspace.readOnly")} />
      </div>
      <${D}
        variant="secondary"
        size="sm"
        onClick=${()=>i.refetch()}
        disabled=${!e||i.isFetching}
      >
        ${i.isFetching?t("workspace.refreshing"):t("workspace.refresh")}
      <//>
    </div>
  `;return e?u`
    <${I} className="p-4 sm:p-5">
      ${d}

      <div className="mt-3 flex min-w-0 flex-wrap items-center gap-1.5 font-mono text-xs text-iron-400">
        <button
          type="button"
          onClick=${()=>n(void 0)}
          className="text-signal hover:underline"
        >
          ${"workspace"}
        </button>
        ${c.map((m,f)=>{let h=`${uD}/${c.slice(0,f+1).join("/")}`;return u`
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
                  <${O}
                    name=${m.kind==="directory"?"folder":"file"}
                    className="h-4 w-4 shrink-0 text-iron-300"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm text-white">${m.name}</span>
                  ${m.kind==="directory"?u`<${O} name="chevron" className="h-3.5 w-3.5 shrink-0 -rotate-90 text-iron-500" />`:u`<${O} name="download" className="h-3.5 w-3.5 shrink-0 text-iron-500" />`}
                </button>
              `):u`
              <div className="rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                ${"This folder is empty."}
              </div>
            `}
      </div>
    <//>
  `:u`
      <${I} className="p-4 sm:p-5">
        ${d}
        <div className="mt-4 rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
          ${"No files yet \u2014 they appear once a thread has run in this project."}
        </div>
      <//>
    `}function mD(e){return[...e||[]].sort((a,n)=>new Date(n.updated_at||n.created_at)-new Date(a.updated_at||a.created_at))[0]?.id||null}function PS({project:e,threads:t,selectedThreadId:a,onSelectThread:n,onNewConversation:r,isStartingConversation:s}){let i=mD(t);return u`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]">
      <div className="space-y-5">
        <div className="min-w-0">
          <h2 className="text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
          ${e.description?u`<p className="mt-1 text-sm leading-6 text-iron-300">${e.description}</p>`:null}
        </div>

        <${OS}
          threads=${t}
          selectedThreadId=${a}
          onSelectThread=${n}
          onNewConversation=${r}
          isStartingConversation=${s}
        />
      </div>

      <${LS} threadId=${i} />
    </div>
  `}function hl(){let e=R(),t=me(),{threadsState:a}=ba(),{projectId:n=null,threadId:r=null}=st(),[s,i]=p.default.useState(""),[o,l]=p.default.useState(null),c=NS(),d=_S(n),m=kS({projectId:n,threadId:r}),f=p.default.useMemo(()=>{let _=s.trim().toLowerCase();return _?c.overview.projects.filter(A=>[A.name,A.description,...A.goals||[]].some(L=>String(L||"").toLowerCase().includes(_))):c.overview.projects},[c.overview.projects,s]),h=p.default.useMemo(()=>c.overview.projects.find(_=>_.id===n)||null,[c.overview.projects,n]),x=p.default.useCallback(()=>{c.invalidate(),d.invalidate()},[c,d]),y=p.default.useCallback(_=>{t(`/projects/${_}`)},[t]),$=p.default.useCallback(_=>{if(_.thread_id){t(`/projects/${_.project_id}/threads/${_.thread_id}`);return}t(`/projects/${_.project_id}`)},[t]),g=p.default.useCallback(async()=>{let _=null;l(null);try{_=await a.createThread()}catch(A){l({type:"error",message:A.message||e("projects.chatAutoFail")})}t("/chat",{state:{composerDraft:e("projects.creationDraft"),threadId:_}})},[t,a]),v=p.default.useCallback(_=>{t(`/projects/${n}/threads/${_}`)},[t,n]),b=p.default.useCallback(async()=>{l(null);try{let _=await a.createThread(n);t("/chat",{state:{threadId:_}}),d.invalidate()}catch(_){l({type:"error",message:_.message||e("projects.chatAutoFail")})}},[t,a,n,d,e]),w=p.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),N=u`
    ${n&&u`<${D} variant="ghost" onClick=${()=>t("/projects")}>${e("projects.allProjects")}<//>`}
  `,C=null;return n?d.isLoading?C=u`
        <div className="space-y-4">
          ${[1,2,3].map(_=>u`<div key=${_} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `:d.error||!d.project&&!h?C=u`
        <${ye}
          title=${e("projects.unavailable")}
          description=${d.error?.message||e("projects.unavailableDesc")}
        >
          <${D} variant="secondary" onClick=${()=>t("/projects")}>${e("projects.returnToProjects")}<//>
        <//>
      `:C=u`
        <${PS}
          project=${d.project||h}
          threads=${d.threads}
          selectedThreadId=${r}
          onSelectThread=${v}
          onNewConversation=${b}
          isStartingConversation=${a.isCreating}
        />
      `:C=c.isLoading?u`
          <div className="space-y-4">
            ${[1,2,3].map(_=>u`<div key=${_} className="v2-skeleton h-40 rounded-[20px]" />`)}
          </div>
        `:u`
          <${MS}
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
            ${N}
          </div>
          ${c.error&&u`
            <div className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
              ${c.error.message}
            </div>
          `}
          <${Ga} result=${o} onDismiss=${()=>l(null)} />
          <${Ga} result=${m.actionResult} onDismiss=${m.clearActionResult} />
          ${!n&&u`
            <${AS} overview=${c.overview} />
            <${DS} items=${c.overview.attention} onOpenItem=${$} />
          `}
          ${C}
        </div>
      </div>
    </div>
  `}function vl(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not scheduled"}function gl(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function US(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function jS(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);return r!==0?r:new Date(n.updated_at||0).getTime()-new Date(a.updated_at||0).getTime()})}function ld({label:e,value:t}){return u`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function fD({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=R();return e.status==="Active"?u`
      <${D} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.fireNow")}<//>
      <${D} variant="secondary" onClick=${()=>n(e.id)} disabled=${t}>${s("missions.action.pause")}<//>
    `:e.status==="Paused"?u`
      <${D} onClick=${()=>r(e.id)} disabled=${t}>${s("missions.action.resume")}<//>
      <${D} variant="secondary" onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runOnce")}<//>
    `:u`<${D} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runAgain")}<//>`}function FS({mission:e,isLoading:t,error:a,isBusy:n,onFire:r,onPause:s,onResume:i,onOpenProject:o,onOpenThread:l}){let c=R();return t?u`
      <div className="space-y-4">
        ${[1,2,3].map(d=>u`<div key=${d} className="v2-skeleton h-36 rounded-xl" />`)}
      </div>
    `:a||!e?u`
      <${ye}
        title=${c("missions.unavailable")}
        description=${a?.message||c("missions.unavailableDesc")}
      />
    `:u`
    <div className="space-y-4">
      <${I} className="p-4 sm:p-5">
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
          <${q} tone=${gl(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${ld} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${ld} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${ld} label=${c("missions.meta.nextFire")} value=${vl(e.next_fire_at)} />
          <${ld} label=${c("missions.meta.updated")} value=${vl(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${fD}
            mission=${e}
            isBusy=${n}
            onFire=${r}
            onPause=${s}
            onResume=${i}
          />
        </div>
      <//>

      <${I} className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.brief")}</div>
        <div className="mt-4 text-sm leading-6 text-iron-200">
          <${aa} content=${e.goal||c("missions.noGoal")} />
        </div>
      <//>

      ${e.current_focus&&u`
        <${I} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${aa} content=${e.current_focus} />
          </div>
        <//>
      `}

      ${e.success_criteria&&u`
        <${I} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${aa} content=${e.success_criteria} />
          </div>
        <//>
      `}

      ${e.threads?.length?u`
        <${I} className="p-4 sm:p-5">
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
                  <${q} tone=${gl(d.state==="Running"?"Active":d.state==="Failed"?"Failed":"Completed")} label=${d.state} />
                </div>
              </button>
            `)}
          </div>
        <//>
      `:null}
    </div>
  `}function pD(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function BS({value:e,onChange:t,children:a,label:n}){return u`
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
  `}function hD({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=R(),s=t===e.id;return u`
    <div
      className=${["w-full rounded-xl border p-4 text-left",s?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80"].join(" ")}
    >
      <button type="button" onClick=${()=>a(e.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${e.name}</div>
              <${q} tone=${gl(e.status)} label=${e.status} />
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
          ${r("missions.updated",{value:vl(e.updated_at)})}
        </span>
        <${D}
          variant="ghost"
          onClick=${i=>{i.stopPropagation(),n(e.project.id)}}
        >
          ${e.project.name}
        <//>
      </div>
    </div>
  `}function Ch({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:l,projectOptions:c,onSelectMission:d,onOpenProject:m}){let f=R(),h=pD(f);return u`
    <${I} className="p-4 sm:p-5">
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
        <${BS} value=${s} onChange=${i} label=${f("missions.filter.status")}>
          ${h.map(x=>u`<option key=${x.value} value=${x.value}>${x.label}<//>`)}
        <//>
        <${BS} value=${o} onChange=${l} label=${f("missions.filter.project")}>
          <option value="all">${f("missions.filter.allProjects")}</option>
          ${c.map(x=>u`<option key=${x.id} value=${x.id}>${x.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(x=>u`
              <${hD}
                key=${x.id}
                mission=${x}
                selectedMissionId=${a}
                onSelectMission=${d}
                onOpenProject=${m}
              />
            `):u`
              <${ye}
                title=${f("missions.emptyTitle")}
                description=${f("missions.emptyDesc")}
                boxed=${!1}
              />
            `}
      </div>
    <//>
  `}function vD(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function zS({summary:e}){let t=R(),a=vD(t);return u`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>u`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${q} tone=${n.tone} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${e[n.key]||0}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">
              ${n.key==="total"?t("missions.summary.completedFailed",{completed:e.completed||0,failed:e.failed||0}):t("missions.summary.acrossProjects")}
            </p>
          </div>
        `)}
      </div>
    <//>
  `}function qS(){return Promise.resolve({projects:[],todo:!0})}function IS({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function KS(e){return Promise.resolve(null)}function HS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function QS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function VS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function GS(e){let t=K({queryKey:["mission-detail",e],queryFn:()=>KS(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function gD(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function YS(){let e=J(),[t,a]=p.default.useState(null),n=K({queryKey:["projects-overview"],queryFn:qS,refetchInterval:7e3}),r=n.data?.projects||[],s=Pd({queries:r.map(f=>({queryKey:["missions","project",f.id],queryFn:()=>IS({projectId:f.id}),refetchInterval:5e3,select:h=>h?.missions||[]}))}),i=s.flatMap((f,h)=>{let x=r[h];return(f.data||[]).map(y=>gD(y,x))}),o=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),l=(f,h)=>({mutationFn:({missionId:x})=>f(x),onSuccess:()=>{a({type:"success",message:h}),o()},onError:x=>{a({type:"error",message:x.message||"Unable to update mission"})}}),c=Q(l(HS,"Mission fired and a run was queued.")),d=Q(l(QS,"Mission paused.")),m=Q(l(VS,"Mission resumed."));return{projects:r,missions:i,summary:US(i),isLoading:n.isLoading||s.some(f=>f.isLoading),isRefreshing:n.isFetching||s.some(f=>f.isFetching),error:n.error||s.find(f=>f.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending,invalidate:o}}function Eh(){let e=R(),t=me(),{missionId:a=null}=st(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,l]=p.default.useState("all"),c=YS(),d=GS(a),m=p.default.useMemo(()=>{let g=n.trim().toLowerCase();return jS(c.missions).filter(v=>{let b=!g||[v.name,v.goal,v.project?.name].some(C=>String(C||"").toLowerCase().includes(g)),w=s==="all"||v.status===s,N=o==="all"||v.project?.id===o;return b&&w&&N})},[c.missions,o,n,s]),f=p.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),h=d.mission?{...f,...d.mission,project:f?.project||null}:f,x=p.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),y=p.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),$=a?u`
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
          <${FS}
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
            <${D}
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

          <${Ga}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${zS} summary=${c.summary} />

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
  `}var JS=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],yD=new Set(["pending","in_progress"]),XS=new Set(["failed","interrupted","stuck","cancelled"]);function ar(e){return e?String(e).replace(/_/g," "):"unknown"}function xi(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":XS.has(e)?"danger":"muted":"muted"}function bD(e){return yD.has(e)}function ud(e){return bD(e?.state)}function ZS(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":XS.has(e.state):!1}function qr(e,t=8){return e?String(e).slice(0,t):"unknown"}function na(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function WS(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function Th(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${na(e.started_at)}`:null].filter(Boolean).join(" / ")}var xD=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function eN(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function $D({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?u`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${t==="tool_use"?a.tool_name||"Tool call":a.tool_name||"Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${eN(t==="tool_use"?a.input:a.output||a.error||a)}</pre>
      </details>
    `:t==="message"?u`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a.role||"assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${a.content||""}</div>
      </div>
    `:u`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t.replace(/_/g," ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${a.message||a.status||eN(a)}</div>
    </div>
  `}function tN({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=R(),[s,i]=p.default.useState("all"),[o,l]=p.default.useState(""),[c,d]=p.default.useState(!0),m=p.default.useRef(null),f=p.default.useMemo(()=>s==="all"?t:t.filter(x=>x.event_type===s),[t,s]);p.default.useEffect(()=>{c&&m.current&&(m.current.scrollTop=m.current.scrollHeight)},[c,f.length]);let h=p.default.useCallback(async(x=!1)=>{let y=o.trim();if(!(!y&&!x))try{await a({content:y||"(done)",done:x}),l("")}catch{}},[o,a]);return u`
    <${I} className="p-5 sm:p-6">
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
            ${xD.map(x=>u`<option key=${x.value} value=${x.value}>${x.label}</option>`)}
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
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${na(x.created_at)}</div>
                <${$D} event=${x} />
              </div>
            `):u`
              <${ye}
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
          <${D} variant="secondary" disabled=${n} onClick=${()=>h(!0)}>${r("common.done")}<//>
          <${D} variant="primary" disabled=${n} onClick=${()=>h(!1)}>${r("common.send")}<//>
        </div>
      `}
    <//>
  `}function aN({job:e,activeTab:t,onTabChange:a,onBack:n,onCancel:r,onRestart:s,isBusy:i,children:o}){return u`
    <div className="space-y-5">
      <${I} className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick=${n} className="text-sm text-signal hover:text-white">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-semibold tracking-tight text-white">${e.title||"Untitled job"}</h2>
              <${q} tone=${xi(e.state)} label=${ar(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${qr(e.id)}</span>
              <span>created ${na(e.created_at)}</span>
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
            ${ud(e)&&u`
              <${D} variant="secondary" disabled=${i} onClick=${()=>r(e.id)}>Cancel<//>
            `}
            ${ZS(e)&&u`
              <${D} variant="primary" disabled=${i} onClick=${()=>s(e.id)}>Restart<//>
            `}
          </div>
        </div>
      <//>

      <div className="flex flex-wrap gap-2">
        ${JS.map(l=>u`
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
  `}function nN({nodes:e,depth:t=0,selectedPath:a,expandingPath:n,onToggleDirectory:r,onSelectPath:s}){return u`
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
        ${i.isDir&&i.expanded&&i.children?.length?u`<${nN}
              nodes=${i.children}
              depth=${t+1}
              selectedPath=${a}
              expandingPath=${n}
              onToggleDirectory=${r}
              onSelectPath=${s}
            />`:null}
      </div>
    `)}
  `}function rN({canBrowse:e,tree:t,selectedPath:a,selectedFile:n,fileError:r,isLoadingTree:s,isLoadingFile:i,expandingPath:o,treeError:l,onToggleDirectory:c,onSelectPath:d}){return e?u`
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <${I} className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          ${l&&u`<div className="mx-2 mb-3 rounded-md border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${l}</div>`}
          ${s?u`<div className="space-y-2 px-2">${[1,2,3,4].map(m=>u`<div key=${m} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?u`
                  <${nN}
                    nodes=${t}
                    selectedPath=${a}
                    expandingPath=${o}
                    onToggleDirectory=${c}
                    onSelectPath=${d}
                  />
                `:u`<div className="px-2 py-6 text-sm text-iron-300">No files were recorded for this workspace.</div>`}
        </div>
      <//>

      <${I} className="min-h-[440px] p-5 sm:p-6">
        <div className="border-b border-white/10 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">File preview</div>
          <p className="mt-2 break-all text-sm leading-6 text-iron-300">${n?.path||a||"Select a file from the tree to inspect its contents."}</p>
        </div>

        ${r&&!i?u`<div className="mt-5 rounded-md border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">${r}</div>`:i?u`<div className="mt-5 space-y-3">${[1,2,3,4,5].map(m=>u`<div key=${m} className="v2-skeleton h-4 rounded" />`)}</div>`:n?u`<pre className="mt-5 max-h-[60vh] overflow-auto whitespace-pre-wrap rounded-[18px] border border-white/10 bg-iron-950/90 p-4 font-mono text-xs leading-6 text-iron-100">${n.content}</pre>`:u`
                <${ye}
                  title="No file selected"
                  description="Pick a concrete file from the workspace tree to render it here."
                />
              `}
      <//>
    </div>
  `:u`
      <${ye}
        title="No project workspace"
        description="File browsing is only available for sandbox jobs that produced a mounted project directory."
      />
    `}function $i({label:e,value:t}){return u`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function sN({job:e}){let t=(e.transitions||[]).map(a=>({title:`${ar(a.from)} -> ${ar(a.to)}`,description:[na(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return u`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${I} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${q} tone=${xi(e.state)} label=${ar(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${$i} label="Created" value=${na(e.created_at)} />
          <${$i} label="Started" value=${na(e.started_at)} />
          <${$i} label="Completed" value=${na(e.completed_at)} />
          <${$i} label="Duration" value=${WS(e.elapsed_secs)} />
          <${$i} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${$i} label="Mode" value=${e.job_mode||"Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${I} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${e.description?u`<${aa} content=${e.description} className="mt-4 text-sm leading-7 text-iron-200" />`:u`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
        <//>

        ${t.length?u`
              <${I} className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Transitions</div>
                <h3 className="mt-2 text-xl font-semibold text-white">State timeline</h3>
                <div className="mt-3">
                  <${W2} items=${t} />
                </div>
              <//>
            `:u`
              <${ye}
                title="No state history yet"
                description="Transitions appear here once the job advances or records a recovery event."
              />
            `}
      </div>
    </div>
  `}function iN({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:l,isBusy:c,isRefreshing:d}){let m=R(),f=[{value:"all",label:m("jobs.list.filter.all")},{value:"pending",label:m("jobs.list.filter.pending")},{value:"in_progress",label:m("jobs.list.filter.inProgress")},{value:"completed",label:m("jobs.list.filter.completed")},{value:"failed",label:m("jobs.list.filter.failed")},{value:"stuck",label:m("jobs.list.filter.stuck")}];if(!e.length){let h=!!n.trim()||s!=="all";return u`
      <${ye}
        title=${m(t&&h?"jobs.list.empty.noMatchTitle":"jobs.list.empty.noJobsTitle")}
        description=${m(t&&h?"jobs.list.empty.noMatchDesc":"jobs.list.empty.noJobsDesc")}
      />
    `}return u`
    <div className="space-y-5">
      <${I} className="p-4 sm:p-5">
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
                  <${q} tone=${xi(h.state)} label=${ar(h.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${qr(h.id)}</span>
                  <span>${m("jobs.list.created",{value:na(h.created_at)})}</span>
                  ${h.started_at&&u`<span>${m("jobs.list.started",{value:na(h.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${ud(h)&&u`
                  <${D}
                    variant="secondary"
                    className="h-9 px-3 text-xs"
                    disabled=${c}
                    onClick=${()=>l(h.id)}
                  >
                    ${m("jobs.action.cancel")}
                  <//>
                `}
                <${D} variant="ghost" className="h-9 px-3 text-xs" onClick=${()=>o(h.id)}>${m("jobs.action.open")}<//>
              </div>
            </div>
          </article>
        `)}
      </div>
    </div>
  `}var wD=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function oN({summary:e}){return u`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${wD.map(t=>u`
          <div
            key=${t.key}
            className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
          >
            <${We}
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
  `}function lN(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function uN(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function cN(e){return Promise.resolve(null)}function dN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function mN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function fN(e){return Promise.resolve({events:[],todo:!0})}function pN(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function Ah(e,t=""){return Promise.resolve({entries:[],todo:!0})}function hN(e,t){return Promise.resolve({content:"",todo:!0})}function vN(e){let t=J(),[a,n]=p.default.useState(null),r=K({queryKey:["job-detail",e],queryFn:()=>cN(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=K({queryKey:["job-events",e],queryFn:()=>fN(e),enabled:!!e,refetchInterval:e?2500:!1}),i=Q({mutationFn:({content:o,done:l})=>pN(e,{content:o,done:l}),onSuccess:(o,{done:l})=>{n({type:"success",message:l?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return p.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function gN(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function yN(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=yN(a.children,t);if(n)return n}}return null}function cd(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:cd(n.children,t,a)}:n)}function bN(e){let[t,a]=p.default.useState([]),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState(""),c=!!(e?.project_dir&&e?.id),d=K({queryKey:["job-files-root",e?.id],queryFn:()=>Ah(e.id,""),enabled:c}),m=K({queryKey:["job-file",e?.id,n],queryFn:()=>hN(e.id,n),enabled:!!(c&&n)});p.default.useEffect(()=>{a([]),r(""),i(""),l("")},[e?.id]),p.default.useEffect(()=>{d.data?.entries?(a(gN(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let f=p.default.useCallback(async h=>{let x=yN(t,h);if(!(!x||!e?.id)){if(x.expanded){a(y=>cd(y,h,$=>({...$,expanded:!1})));return}if(x.loaded){a(y=>cd(y,h,$=>({...$,expanded:!0})));return}l(h);try{let y=await Ah(e.id,h);a($=>cd($,h,g=>({...g,expanded:!0,loaded:!0,children:gN(y.entries)}))),i("")}catch(y){i(y.message||"Unable to open folder")}finally{l("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:m.data||null,fileError:m.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:m.isLoading||m.isFetching,expandingPath:o,treeError:s,toggleDirectory:f}}function xN(){let e=J(),[t,a]=p.default.useState(null),n=K({queryKey:["jobs-summary"],queryFn:uN,refetchInterval:5e3}),r=K({queryKey:["jobs"],queryFn:lN,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=Q({mutationFn:({jobId:l})=>dN(l),onSuccess:(l,{jobId:c})=>{a({type:"success",message:`Job ${qr(c)} cancelled`}),s()},onError:l=>{a({type:"error",message:l.message||"Unable to cancel job"})}}),o=Q({mutationFn:({jobId:l})=>mN(l),onSuccess:l=>{a({type:"success",message:`Restart queued as ${qr(l?.new_job_id)}`}),s()},onError:l=>{a({type:"error",message:l.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function $N({result:e,onDismiss:t}){let a=R();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return u`
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
  `}function Dh(){let e=R(),t=me(),{jobId:a=null}=st(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,l]=p.default.useState(a?"activity":"overview"),c=xN(),d=vN(a),m=bN(d.job);p.default.useEffect(()=>{l(a?"activity":"overview")},[a]);let f=p.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(b=>{let w=!v||b.title.toLowerCase().includes(v)||b.id.toLowerCase().includes(v),N=s==="all"||b.state===s;return w&&N})},[c.jobs,n,s]),h=p.default.useCallback(v=>t(`/jobs/${v}`),[t]),x=p.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),y=p.default.useCallback(async v=>{try{let b=await c.restartJob({jobId:v});b?.new_job_id&&t(`/jobs/${b.new_job_id}`)}catch{}},[c,t]),$=u`
    ${a&&u`<${D} variant="ghost" onClick=${()=>t("/jobs")}
      >${e("jobs.allJobs")}<//
    >`}
  `,g=null;if(a)if(d.isLoading)g=u`
        <div className="space-y-4">
          ${[1,2,3].map(v=>u`<div key=${v} className="v2-skeleton h-32 rounded-[18px]" />`)}
        </div>
      `;else if(d.error||!d.job)g=u`
        <${ye}
          title=${e("jobs.unavailable")}
          description=${d.error?.message||e("jobs.unavailableDesc")}
        >
          <${D} variant="secondary" onClick=${()=>t("/jobs")}
            >${e("jobs.returnToJobs")}<//
          >
        <//>
      `;else{let v={overview:u`<${sN} job=${d.job} />`,activity:u`
          <${tN}
            job=${d.job}
            events=${d.events}
            onSendPrompt=${d.sendPrompt}
            isSendingPrompt=${d.isSendingPrompt}
          />
        `,files:u`
          <${rN}
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
        <${aN}
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
          <${iN}
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
          <${$N}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${$N}
            result=${d.promptResult}
            onDismiss=${d.clearPromptResult}
          />
          <${oN} summary=${c.summary} />
          ${g}
        </div>
      </div>
    </div>
  `}function nr(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not scheduled"}function dd(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function md(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function wN(e=[]){return[...e].sort((t,a)=>t.enabled!==a.enabled?t.enabled?-1:1:new Date(a.next_fire_at||a.last_run_at||0).getTime()-new Date(t.next_fire_at||t.last_run_at||0).getTime())}function SN(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function SD(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function NN({runs:e}){return e?.length?u`
    <div className="space-y-3">
      ${e.map(t=>u`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${q} tone=${SD(t.status)} label=${t.status} />
              <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                ${nr(t.started_at)}
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
    `}function rr({label:e,value:t}){return u`
    <div className="rounded-xl border border-iron-700 bg-iron-950/50 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div className="mt-2 min-w-0 break-words text-sm text-iron-100">
        ${t||"\u2014"}
      </div>
    </div>
  `}function _N({title:e,value:t}){return u`
    <div>
      <h3 className="text-sm font-semibold text-iron-100">${e}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >${JSON.stringify(t||{},null,2)}</pre>
    </div>
  `}function kN({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=me(),l=R();return t?u`
      <div className="space-y-4">
        ${[1,2,3].map(c=>u`<div key=${c} className="v2-skeleton h-32 rounded-xl" />`)}
      </div>
    `:a||!e?u`
      <${ye}
        title=${l("routine.unavailable")}
        description=${a?.message||l("routine.unavailableDesc")}
      />
    `:u`
    <${I} className="p-4 sm:p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="truncate text-2xl font-semibold tracking-tight text-iron-100">
              ${e.name}
            </h2>
            <${q}
              tone=${dd(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${q}
              tone=${md(e.verification_status)}
              label=${e.verification_status||"unknown"}
            />
          </div>
          <p className="mt-2 max-w-3xl text-sm leading-6 text-iron-300">
            ${e.description||e.trigger_summary||"No description"}
          </p>
        </div>

        <div className="flex flex-wrap gap-2">
          <${D} variant="secondary" disabled=${n} onClick=${r}>Run<//>
          <${D} variant="ghost" disabled=${n} onClick=${s}>
            ${e.enabled?"Disable":"Enable"}
          <//>
          <${D} variant="ghost" onClick=${i}>Delete<//>
        </div>
      </div>

      <div className="mt-5 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <${rr} label="Trigger" value=${e.trigger_summary||e.trigger_type} />
        <${rr} label="Action" value=${SN(e.action)} />
        <${rr} label="Next fire" value=${nr(e.next_fire_at)} />
        <${rr} label="Last run" value=${nr(e.last_run_at)} />
        <${rr} label="Run count" value=${e.run_count} />
        <${rr} label="Failures" value=${e.consecutive_failures} />
        <${rr} label="Created" value=${nr(e.created_at)} />
        <${rr} label="Routine ID" value=${e.id} />
      </div>

      ${e.conversation_id&&u`
        <div className="mt-5">
          <${D} variant="secondary" onClick=${()=>o(`/chat/${e.conversation_id}`)}>
            Open routine thread
          <//>
        </div>
      `}

      <div className="mt-6 grid gap-6 xl:grid-cols-2">
        <${_N} title=${l("routine.triggerPayload")} value=${e.trigger} />
        <${_N} title=${l("routine.actionPayload")} value=${e.action} />
      </div>

      <div className="mt-6">
        <h3 className="mb-3 text-sm font-semibold text-iron-100">Recent runs</h3>
        <${NN} runs=${e.recent_runs} />
      </div>
    <//>
  `}function RN({routine:e,selectedRoutineId:t,onSelectRoutine:a,onTriggerRoutine:n,onToggleRoutine:r,isBusy:s}){let i=t===e.id;return u`
    <article
      className=${["group flex flex-col gap-4 rounded-[18px] border p-5",i?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick=${()=>a(e.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">${e.name}</h3>
            <${q}
              tone=${dd(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${q}
              tone=${md(e.verification_status)}
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
            <span>next ${nr(e.next_fire_at)}</span>
          </div>
        </button>

        <div className="flex shrink-0 flex-wrap gap-2">
          <${D}
            variant="secondary"
            className="h-9 px-3 text-xs"
            disabled=${s}
            onClick=${()=>n(e.id)}
          >
            Run
          <//>
          <${D}
            variant="ghost"
            className="h-9 px-3 text-xs"
            disabled=${s}
            onClick=${()=>r(e.id)}
          >
            ${e.enabled?"Disable":"Enable"}
          <//>
          <${D}
            variant="ghost"
            className="h-9 px-3 text-xs"
            onClick=${()=>a(e.id)}
          >
            Open
          <//>
        </div>
      </div>
    </article>
  `}var ND=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function Mh({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:l,onToggleRoutine:c,isBusy:d,isRefreshing:m}){let f=R();if(!e.length){let h=!!n.trim()||s!=="all";return u`
      <${ye}
        title=${t&&h?"No routines match":"No routines yet"}
        description=${t&&h?"Adjust the search or status filter to find a saved routine.":"Routines created from chat will appear here after they are saved."}
      />
    `}return u`
    <div className="space-y-5">
      <${I} className="p-4 sm:p-5">
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
            ${ND.map(h=>u`<option key=${h.value} value=${h.value}>${h.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(h=>u`
            <${RN}
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
  `}var _D=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function CN({summary:e}){return u`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${_D.map(t=>u`
            <div
              key=${t.key}
              className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
            >
              <${We}
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
  `}function EN(e){let[t,a]=p.default.useState(""),[n,r]=p.default.useState("all");return{filteredRoutines:p.default.useMemo(()=>{let i=t.trim().toLowerCase();return wN(e).filter(o=>{let l=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||l.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function TN(){return Promise.resolve({routines:[],todo:!0})}function AN(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function DN(e){return Promise.resolve(null)}function fd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function pd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function MN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function ON(e){let t=J(),[a,n]=p.default.useState(null),r=K({queryKey:["routine-detail",e],queryFn:()=>DN(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:m=>{n({type:"error",message:m.message||"Unable to update routine"})}}),o=Q(i(fd,"Routine run queued.")),l=Q(i(pd,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:l.mutateAsync,isBusy:o.isPending||l.isPending}}function LN(){let e=J(),[t,a]=p.default.useState(null),n=K({queryKey:["routines-summary"],queryFn:AN,refetchInterval:5e3}),r=K({queryKey:["routines"],queryFn:TN,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,m)=>({mutationFn:({routineId:f})=>d(f),onSuccess:()=>{a({type:"success",message:m}),s()},onError:f=>{a({type:"error",message:f.message||"Unable to update routine"})}}),o=Q(i(fd,"Routine run queued.")),l=Q(i(pd,"Routine status updated.")),c=Q(i(MN,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:l.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||l.isPending||c.isPending,invalidate:s}}function Oh(){let e=me(),{routineId:t=null}=st(),a=LN(),n=ON(t),r=EN(a.routines),s=p.default.useCallback(async(l,c)=>{try{await l({routineId:c})}catch{}},[]),i=p.default.useCallback(async(l,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:l}),e("/routines")}catch{}},[e,a]),o=t?u`
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
          <${kN}
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
            <${D} variant="ghost" onClick=${()=>e("/routines")}>
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

          <${Ga}
            result=${a.actionResult}
            onDismiss=${a.clearActionResult}
          />
          <${Ga}
            result=${n.actionResult}
            onDismiss=${n.clearActionResult}
          />
          <${CN} summary=${a.summary} />

          ${a.isLoading?u`
                <div className="space-y-4">
                  ${[1,2,3].map(l=>u`<div key=${l} className="v2-skeleton h-32 rounded-xl" />`)}
                </div>
              `:o}
        </div>
      </div>
    </div>
  `}function kD(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function RD(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function PN({deliveryState:e}){let t=R(),a=e.currentTarget?.target_id||"",[n,r]=p.default.useState(a),[s,i]=p.default.useState(!1),o=p.default.useRef(null);p.default.useEffect(()=>{r(a)},[a]),p.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let l=n!==a,c=e.isLoading||e.isSaving,d=l&&!c,m=!!a&&!c,f=e.finalReplyTargets.length>0,h=e.targets.some(L=>L?.capabilities?.final_replies&&L?.target?.status==="unavailable"),x=f||h,y=L=>(o.current&&clearTimeout(o.current),i(!1),L.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),$=()=>{d&&y(e.saveFinalReplyTarget(n||null))},g=()=>{m&&(r(""),y(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),b=e.currentStatus,w=b==="available"?"success":b==="unavailable"?"warning":"muted",N=t(b==="available"?"automations.delivery.pill.ready":b==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),C=!!e.currentTarget,_=t(C?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),A=RD(t("automations.delivery.footnote"),{command:u`<code
        key="cmd"
        className="rounded px-1.5 py-0.5 font-mono text-[0.6875rem] bg-[var(--v2-surface-muted)] text-[var(--v2-accent-text)]"
      >
        approve &lt;code&gt;
      </code>`});return u`
    <${I} className="p-5 sm:p-6">
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
        ${C&&u`
          <div>
            <span className="mb-1.5 block font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
              ${t("automations.delivery.currentDefault")}
            </span>
            <div
              className="flex items-center gap-3 rounded-xl border px-4 py-3 bg-[var(--v2-positive-soft)] border-[color-mix(in_srgb,var(--v2-positive-text)_25%,var(--v2-panel-border))]"
            >
              <span className="flex-1 min-w-0 text-sm font-semibold text-[var(--v2-text-strong)] truncate">
                ${v}
              </span>
              <${q} tone=${w} label=${N} />
            </div>
          </div>
        `}

        <!-- ── Radio option rows ────────────────────────────────────── -->
        <div>
          <span className="mb-1.5 block font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
            ${_}
          </span>
          <div
            className="flex flex-col gap-3"
            role="radiogroup"
            aria-label=${t("automations.delivery.title")}
          >

            <!-- Available external targets -->
            ${e.finalReplyTargets.map(L=>{let M=L?.target?.target_id??"",P=L?.target?.display_name||L?.target?.target_id||"",k=L?.target?.description||"",z=L?.target?.status??"available",Z=n===M;return u`
                <label
                  key=${M}
                  className=${V("flex items-start gap-3.5 rounded-xl border px-4 py-3.5 cursor-pointer","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]","hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",Z&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
                >
                  <input
                    type="radio"
                    name="delivery-target"
                    value=${M}
                    checked=${Z}
                    disabled=${c}
                    onChange=${()=>r(M)}
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
                  <${q}
                    tone=${kD(z)}
                    label=${t(z==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.ready")}
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
                <${q}
                  tone="warning"
                  label=${t("automations.delivery.pill.notPaired")}
                  className="shrink-0"
                />
              </div>
            `}

            <!-- Web app only / fallback row -->
            <label
              className=${V("flex items-start gap-3.5 rounded-xl border px-4 py-3.5","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]",f?"cursor-pointer hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]":"cursor-default",n===""&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
            >
              <input
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
              <${q}
                tone="muted"
                label=${t("automations.delivery.pill.fallback")}
                className="self-center shrink-0"
              />
            </label>

          </div>
        </div>

        <!-- ── Save row ─────────────────────────────────────────────── -->
        <div className="flex flex-wrap items-center gap-3">
          <${D}
            variant="primary"
            size="sm"
            disabled=${!d}
            onClick=${$}
          >
            <${O} name="check" className="h-3.5 w-3.5" />
            ${t("automations.delivery.save")}
          <//>
          <${D}
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
              <${O} name="check" className="h-3 w-3" />
              ${t("automations.delivery.saved")}
            </span>
          `}
          ${e.saveError&&!s&&u`
            <span
              role="alert"
              className="flex items-center gap-1.5 text-xs font-semibold text-red-300"
            >
              <${O} name="close" className="h-3 w-3" />
              ${t("automations.delivery.saveFailed")}
            </span>
          `}
        </div>

        <!-- ── Footnote (only when an external Slack-style target exists) ── -->
        ${x&&u`
          <div
            className="rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-faint)]"
          >
            ${A}
          </div>
        `}

      </div>
    <//>
  `}var CD=["schedule","once"],jN={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},FN={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},BN={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function ra(e){return typeof e=="function"?e:t=>t}var Ph=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:wn},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:ID},{value:"completed",labelKey:"automations.filter.completed",predicate:KD}];function zN(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>CD.includes(r?.source?.type)).map(r=>jD(r,t,a)).sort(qD)}function qN(e,t){let a=Ph.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function IN(e){let t=e.filter(i=>i.state!=="completed"),a=t.filter(i=>wn(i)).length,n=t.filter(i=>i.has_running_run).length,r=t.filter(i=>i.has_failed_runs).length,s=t.filter(i=>wn(i)&&Lh(i)!=null).sort((i,o)=>(i.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(o.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:t.length,active:a,running:n,failures:r,nextRun:s?.next_run_label||null}}function ED(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=GD(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:l,month:c,dayOfWeek:d,year:m}=s,f=t&&typeof t=="string"?t:null,h=f?` (${f})`:"",x=m==="*"&&l==="*"&&c==="*"&&d==="*";if(x&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=YD(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(sr(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let y=HD(o,i,n);if(!y)return r("automations.schedule.custom");if(x)return r("automations.schedule.everyDayAt",{time:y})+h;let $=JD(d);if(m==="*"&&l==="*"&&c==="*"&&$==="1-5")return r("automations.schedule.weekdaysAt",{time:y})+h;if(m==="*"&&l==="*"&&c==="*"&&sr($,0,7)){let g=QD(Number($)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:y})+h}if(m==="*"&&sr(l,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(l),time:y})+h;if(sr(l,1,31)&&sr(c,1,12)&&d==="*"&&(m==="*"||sr(m,1970,9999))){let g=VD(Number(c),Number(l),m==="*"?null:Number(m),n);return r("automations.schedule.dateAt",{date:g,time:y})+h}return r("automations.schedule.custom")}function Ir(e,t="Unknown",a,n){if(!e)return t;let r=new Date(e);if(Number.isNaN(r.getTime()))return t;let s=n&&typeof n=="string"?{timeZone:n}:{};try{return r.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...s})}catch{return r.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function KN(e,t){let a=jN[e]?.labelKey||"automations.state.unknown";return ra(t)(a)}function HN(e){return jN[e]?.tone||"muted"}function TD(e,t){return wn(e)&&e?.has_running_run?ra(t)("automations.status.running"):wn(e)&&e?.has_failed_runs?ra(t)("automations.status.needsReview"):KN(e?.state,t)}function AD(e){return wn(e)&&e?.has_running_run?"info":wn(e)&&e?.has_failed_runs?"danger":HN(e?.state)}function DD(e,t){let a=FN[e]?.labelKey||"automations.lastStatus.none";return ra(t)(a)}function MD(e){return FN[e]?.tone||"muted"}function OD(e,t){let a=BN[hd(e)]?.labelKey||"automations.runStatus.unknown";return ra(t)(a)}function LD(e){return BN[hd(e)]?.tone||"muted"}function PD(e,t,a,n){if(!e)return ra(a)("automations.schedule.custom");let r=Ir(e,null,n,t);if(!r)return ra(a)("automations.schedule.custom");let s=t&&typeof t=="string"?` (${t})`:"";return ra(a)("automations.schedule.onceAt",{datetime:r})+s}function UD(e,t,a){return e?.type==="once"?PD(e.at,e.timezone,t,a):e?.type==="schedule"?ED(e.cron,e.timezone||"UTC",t,a):ra(t)("automations.schedule.custom")}function jD(e,t,a){let n=ra(t),r=FD(e.recent_runs,t,a),s=r[0]||null,i=r.find(m=>m.status==="running")||null,o=r.find(m=>m.status==="ok"||m.status==="error")||null,l=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null,d={...e,recent_runs:r,has_running_run:r.some(m=>m.status==="running"),has_failed_runs:r.some(m=>m.status==="error")};return{...d,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:UD(e.source,t,a),state_label:KN(e.state,t),state_tone:HN(e.state),primary_status_label:TD(d,t),primary_status_tone:AD(d),next_run_timestamp:Uh(e.next_run_at),next_run_label:Ir(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:Ir(c,n("automations.date.noRuns"),a),last_status_label:DD(l,t),last_status_tone:MD(l),created_label:Ir(e.created_at,n("automations.date.unknown"),a),latest_run:s,current_run:i,success_rate_label:zD(r,t)}}function FD(e,t,a){let n=ra(t);return Array.isArray(e)?e.map(r=>{let s=hd(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=Uh(i);return{...r,status:s,status_label:OD(s,t),status_tone:LD(s),timestamp:o,timestamp_source:i,fired_label:Ir(i,n("automations.date.unscheduled"),a),submitted_label:Ir(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:Ir(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function hd(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function QN(e){let t=Array.isArray(e)?e:[],a={total:t.length,ok:0,error:0,running:0,unknown:0};for(let n of t){let r=hd(n?.status);Object.prototype.hasOwnProperty.call(a,r)?a[r]+=1:a.unknown+=1}return a}function BD(e){let t=QN(e);return[{key:"ok",tone:"text-emerald-300",count:t.ok},{key:"error",tone:"text-red-300",count:t.error},{key:"running",tone:"text-sky-300",count:t.running},{key:"unknown",tone:"text-iron-400",count:t.unknown}].filter(a=>a.count>0)}function VN(e,t){let a=ra(t),n=QN(e),r=BD(e).map(s=>({...s,text:a(`automations.runs.${s.key}`,{count:s.count})}));return{total:n.total,totalText:a("automations.runs.total",{count:n.total}),chips:r}}function zD(e,t){let a=ra(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function qD(e,t){let a=wn(e),n=wn(t);return a!==n?a?-1:1:(Lh(e)??Number.MAX_SAFE_INTEGER)-(Lh(t)??Number.MAX_SAFE_INTEGER)}function Uh(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function wn(e){return e?.state==="active"||e?.state==="scheduled"}function ID(e){return["paused","disabled","inactive"].includes(e?.state)}function KD(e){return e?.state==="completed"}function Lh(e){return e?.next_run_timestamp??Uh(e?.next_run_at)}function jh(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function HD(e,t,a){return!sr(e,0,23)||!sr(t,0,59)?null:jh(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function QD(e,t){return jh(t,{weekday:"long"},new Date(2001,0,7+e))}function VD(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return jh(n,r,new Date(a??2e3,e-1,t))}function GD(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&UN(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&UN(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function UN(e){return/^0+$/.test(e)}function sr(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function YD(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function JD(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}function XD(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function GN({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let l=o.toString(),c=`/logs${l?`?${l}`:""}`;return i?`/v2${c}`:c}function YN(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(XD),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}var ZD=8;function Fh(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function vd({runs:e=[]}){let t=R(),a=Array.isArray(e)?e:[],n=a.slice(0,ZD);if(!n.length)return u`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;let r=a.length-n.length,s=`+${Math.min(r,999)}`;return u`
    <div
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
    >
      ${n.map(i=>u`
        <span
          key=${Fh(i)}
          title=${`${i.status_label} \xB7 ${i.fired_label}`}
          className=${V("h-3 w-3 rounded-full border",i.status==="ok"&&"border-emerald-300/50 bg-emerald-400",i.status==="error"&&"border-red-300/50 bg-red-400",i.status==="running"&&"border-sky-300/60 bg-sky-400",i.status==="unknown"&&"border-iron-500 bg-iron-600")}
        />
      `)}
      ${r>0&&u`<span
        className="ml-0.5 font-mono text-[11px] text-iron-400"
        title=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
      >
        ${s}
      </span>`}
    </div>
  `}function gd({runs:e=[],className:t=""}){let a=R(),n=VN(e,a);return n.total?u`
    <div className=${V("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]",t)}>
      <span className="text-iron-300">${n.totalText}</span>
      ${n.chips.map(r=>u`<span key=${r.key} className=${r.tone}>${r.text}</span>`)}
    </div>
  `:u`<span className=${V("text-[11px] text-iron-400",t)}>
      ${a("automations.table.noRuns")}
    </span>`}function JN({run:e,onOpenRun:t,onOpenLogs:a}){let n=R(),r=!!e.chat_path,s=GN({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return u`
    <div className="grid gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto] sm:items-center">
      <div>
        <${q} tone=${e.status_tone} label=${e.status_label} />
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
        <${D}
          variant="secondary"
          size="sm"
          disabled=${!r}
          onClick=${r?()=>t(e.chat_path):void 0}
        >
          <${O} name="chat" className="mr-1.5 h-4 w-4" />
          ${n("automations.detail.openRun")}
        <//>
        <${D}
          variant="ghost"
          size="sm"
          disabled=${!i}
          onClick=${i?()=>a(s):void 0}
        >
          <${O} name="file" className="mr-1.5 h-4 w-4" />
          ${n("nav.logs")}
        <//>
      </div>
    </div>
  `}function yd({label:e,value:t,tone:a}){return u`
    <div className="min-w-0 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div
        className=${V("mt-2 min-w-0 break-words text-sm text-iron-100",a==="success"&&"text-emerald-200",a==="danger"&&"text-red-200",a==="info"&&"text-sky-200")}
      >
        ${t||"\u2014"}
      </div>
    </div>
  `}function XN({automation:e,isMutating:t=!1,onPauseAutomation:a,onResumeAutomation:n,onDeleteAutomation:r}){let s=R(),i=me();if(!e)return u`
      <${I} className="p-4 sm:p-5">
        <${ye}
          boxed=${!1}
          title=${s("automations.detail.emptyTitle")}
          description=${s("automations.detail.emptyDescription")}
        />
      <//>
    `;let o=e.current_run,l=e.state==="paused",c=e.state==="active"||e.state==="scheduled",m=`${s(l?"missions.action.resume":"missions.action.pause")}: ${e.display_name}`,f=()=>{if(l){n?.(e.automation_id);return}c&&a?.(e.automation_id)},h=`${s("common.delete")}: ${e.display_name}`,x=()=>{window.confirm(h)&&r?.(e.automation_id)};return u`
    <${I} className="overflow-hidden">
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
            <${q}
              tone=${e.primary_status_tone}
              label=${e.primary_status_label}
            />
            ${(c||l)&&u`
              <${D}
                type="button"
                variant=${l?"primary":"secondary"}
                size="icon-sm"
                aria-label=${m}
                title=${m}
                disabled=${t}
                onClick=${f}
              >
                <${O} name=${l?"play":"pause"} className="h-4 w-4" />
              <//>
            `}
            <${D}
              type="button"
              variant="danger"
              size="icon-sm"
              aria-label=${h}
              title=${h}
              disabled=${t}
              onClick=${x}
            >
              <${O} name="trash" className="h-4 w-4" />
            <//>
          </div>
        </div>
      </div>

      <div className="space-y-5 p-4 sm:p-5">
        <div className="grid gap-3 sm:grid-cols-2">
          <${yd} label=${s("automations.detail.schedule")} value=${e.schedule_label} />
          <${yd}
            label=${s("automations.detail.successRate")}
            value=${e.success_rate_label}
            tone=${e.has_failed_runs?"danger":"success"}
          />
          <${yd} label=${s("automations.detail.lastCompleted")} value=${e.last_run_label} />
          <${yd}
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
              <${vd} runs=${e.recent_runs} />
              <${gd} runs=${e.recent_runs} />
            </div>
          </div>

          ${e.recent_runs.length?u`
                <div>
                  ${e.recent_runs.map(y=>u`
                    <${JN}
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
  `}var WD=["automations.empty.example1","automations.empty.example2","automations.empty.example3"];function eM({promptKey:e}){let t=R(),a=t(e),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>()=>clearTimeout(s.current),[]),u`
    <li
      className="flex items-center gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3"
    >
      <span className="min-w-0 flex-1 text-sm leading-6 text-iron-200">${a}</span>
      <button
        type="button"
        onClick=${async()=>{let o=typeof navigator>"u"?null:navigator.clipboard;if(o?.writeText)try{await o.writeText(a),r(!0),clearTimeout(s.current),s.current=setTimeout(()=>r(!1),1500)}catch{}}}
        aria-label=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        title=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        className=${V("inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-[var(--v2-panel-border)] text-iron-300 hover:text-iron-100 hover:border-white/20","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",n&&"text-emerald-300")}
      >
        <${O} name=${n?"check":"copy"} className="h-4 w-4" />
      </button>
    </li>
  `}function ZN(){let e=R(),t=me();return u`
    <${I} className="p-6 sm:p-8">
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
            ${WD.map(a=>u`<${eM} key=${a} promptKey=${a} />`)}
          </ul>
        </div>

        <div className="mt-6">
          <${D} variant="primary" size="sm" onClick=${()=>t("/chat")}>
            <${O} name="chat" className="mr-1.5 h-4 w-4" />
            ${e("automations.empty.startInChat")}
          <//>
        </div>
      </div>
    <//>
  `}function WN({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,isMutating:s,selectedAutomationId:i,onSelectAutomation:o,onPauseAutomation:l,onResumeAutomation:c,onDeleteAutomation:d}){let m=R(),f=qN(e,t),h=e.length>0,x=f.find(y=>y.automation_id===i)||f[0]||null;return u`
    <div className="space-y-5">
      <${I} className="p-4 sm:p-5">
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
                  className=${V("min-h-9 shrink-0 whitespace-nowrap px-3 py-2 text-xs font-semibold leading-tight",t===y.value?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
                >
                  ${m(y.labelKey)}
                </button>
              `)}
            </div>
            <${D}
              variant="secondary"
              size="icon-sm"
              aria-label=${m("automations.refresh")}
              title=${m(r?"automations.refreshing":"automations.refresh")}
              disabled=${r}
              onClick=${n}
            >
              <${O}
                name="retry"
                className=${V("h-4 w-4",r&&"v2-spin")}
              />
            <//>
          </div>
        </div>
      <//>

      ${f.length?u`
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1.12fr)_minmax(22rem,0.88fr)]">
              <${I} className="overflow-hidden">
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
                            className=${V("border-b border-[var(--v2-panel-border)] last:border-0 hover:bg-white/[0.03]",$&&"bg-[var(--v2-accent-soft)]/30")}
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
                                <${vd} runs=${y.recent_runs} />
                                <${gd} runs=${y.recent_runs} />
                              </div>
                            </td>
                            <td className="px-5 py-4 align-top">
                              <${q}
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

              <${XN}
                automation=${x}
                isMutating=${s}
                onPauseAutomation=${l}
                onResumeAutomation=${c}
                onDeleteAutomation=${d}
              />
            </div>
          `:h?u`
              <${ye}
                title=${m("automations.empty.matchingTitle")}
                description=${m("automations.empty.matchingDescription")}
              />
            `:u`<${ZN} />`}
    </div>
  `}function e_({summary:e,activeFilter:t,onSelectFilter:a}){let n=R(),r=[{key:"scheduled",label:n("automations.summary.scheduled"),value:e?.scheduled??0,tone:"muted",detail:n("automations.summary.scheduledDetail"),filter:"all"},{key:"active",label:n("automations.summary.active"),value:e?.active??0,tone:"signal",detail:n("automations.summary.activeDetail"),filter:"active"},{key:"running",label:n("automations.summary.running"),value:e?.running??0,tone:"info",detail:n("automations.summary.runningDetail"),filter:"running"},{key:"failures",label:n("automations.summary.failures"),value:e?.failures??0,tone:(e?.failures??0)>0?"danger":"success",detail:n("automations.summary.failuresDetail"),filter:(e?.failures??0)>0?"failures":null},{key:"nextRun",label:n("automations.summary.nextRun"),value:e?.nextRun||n("automations.summary.none"),tone:"info",detail:n("automations.summary.nextRunDetail"),valueClassName:"text-lg md:text-xl"}];return u`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        ${r.map(s=>{let i=!!(s.filter&&a),o=i&&t===s.filter,l=u`
            <${We}
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
              className=${V(c,"transition-colors hover:border-white/20 hover:bg-white/[0.05]","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",o&&"border-[var(--v2-accent)]/60 bg-[var(--v2-accent-soft)]/30")}
            >
              ${l}
            </button>
          `:u`<div key=${s.key} className=${c}>${l}</div>`})}
      </div>
    <//>
  `}function tM(e){return e==="active"||e==="scheduled"}function aM(e){return Number.isFinite(e)?e:null}function t_(e,t=Date.now()){let a=Array.isArray(e)?e:[],n=null;for(let r of a){if(!r||(r.has_running_run&&(n=n==null?5e3:Math.min(n,5e3)),!tM(r.state)))continue;let s=aM(r.next_run_timestamp);if(s==null)continue;let i=s-t,o=i<=0?2e3:i<3e4?Math.max(1e3,i+1200):null;o!=null&&(n=n==null?o:Math.min(n,o))}return n}var rM=50,sM=25;function a_(e=!1){let{t,lang:a}=$l(),n=J(),r=K({queryKey:["automations",{includeCompleted:e}],queryFn:()=>Fx({limit:rM,runLimit:sM,includeCompleted:e}),refetchInterval:3e4,refetchIntervalInBackground:!1}),s=p.default.useMemo(()=>zN(r.data,t,a),[r.data,t,a]),i=p.default.useMemo(()=>IN(s),[s]),o=p.default.useMemo(()=>t_(s),[s]);p.default.useEffect(()=>{if(o==null)return;let h=setTimeout(()=>{r.refetch()},o);return()=>clearTimeout(h)},[o,r.refetch]);let l=r.data?.scheduler_enabled!==!1,c=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["automations"]})},[n]),d=Q({mutationFn:h=>Bx({automationId:h}),onSuccess:c}),m=Q({mutationFn:h=>zx({automationId:h}),onSuccess:c}),f=Q({mutationFn:h=>qx({automationId:h}),onSuccess:c});return{automations:s,summary:i,schedulerEnabled:l,isLoading:r.isLoading,isRefreshing:r.isFetching,isMutating:d.isPending||m.isPending||f.isPending,error:r.error||null,actionError:d.error||m.error||f.error||null,pauseAutomation:d.mutate,resumeAutomation:m.mutate,deleteAutomation:f.mutate,refetch:r.refetch}}var n_=["outbound-delivery","preferences"],r_=["outbound-delivery","targets"];function s_(){let e=J(),t=K({queryKey:n_,queryFn:Qx}),a=K({queryKey:r_,queryFn:Vx}),n=Q({mutationFn:({finalReplyTargetId:i})=>Gx({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(n_,i),e.invalidateQueries({queryKey:r_})}}),r=p.default.useMemo(()=>a.data?.targets??[],[a.data]),s=p.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function i_(){let e=R(),[t,a]=p.default.useState("all"),[n,r]=p.default.useState(null),i=a_(t==="completed"),o=s_(),[l,c]=p.default.useState(!1),d=p.default.useRef(null);p.default.useEffect(()=>()=>clearTimeout(d.current),[]);let m=p.default.useCallback(()=>{c(!0),clearTimeout(d.current),d.current=setTimeout(()=>c(!1),1e3),i.refetch()},[i.refetch]),f=i.isRefreshing||l,h=i.error&&!i.isLoading&&i.automations.length===0;return p.default.useEffect(()=>{if(!i.automations.length){r(null);return}i.automations.some(y=>y.automation_id===n)||r(i.automations[0].automation_id)},[i.automations,n]),u`
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
                <${e_}
                  summary=${i.summary}
                  activeFilter=${t}
                  onSelectFilter=${a}
                />
                <${PN} deliveryState=${o} />

                ${i.isLoading?u`
                      <div className="space-y-4">
                        ${[1,2,3].map(x=>u`<div
                              key=${x}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`)}
                      </div>
                    `:u`
                      <${WN}
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
  `}var o_={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function l_({result:e,onDismiss:t}){return p.default.useEffect(()=>{if(!e)return;let a=setTimeout(t,4e3);return()=>clearTimeout(a)},[e,t]),e?u`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",o_[e.type]||o_.info].join(" ")}>
      <${O}
        name=${e.type==="success"?"check":e.type==="error"?"close":"bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">
        <${O} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `:null}var c_="/api/webchat/v2/channels/slack/setup";function d_(){return H(c_)}function m_(e){let t={installation_id:String(e.installation_id||"").trim(),team_id:String(e.team_id||"").trim(),api_app_id:String(e.api_app_id||"").trim(),user_id:u_(e.user_id),shared_subject_user_id:u_(e.shared_subject_user_id)},a=String(e.bot_token||"").trim(),n=String(e.signing_secret||"").trim();return a&&(t.bot_token=a),n&&(t.signing_secret=n),H(c_,{method:"PUT",body:JSON.stringify(t)})}function Bh(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function u_(e){let t=String(e||"").trim();return t||null}var f_="/api/webchat/v2/channels/slack/allowed",iM="/api/webchat/v2/channels/slack/subjects";function p_(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function h_(){return H(f_)}function v_(){return H(iM)}function g_(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return H(f_,{method:"PUT",body:JSON.stringify(n)})}function y_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var b_=["slack-allowed-channels"];function $_({action:e}){let t=R(),a=J(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState([]),c=lM(e,t),d=K({queryKey:b_,queryFn:h_}),m=K({queryKey:["slack-routable-subjects"],queryFn:v_}),f=m.data?.subjects||[],h=x_(f),x=m.isSuccess||m.isError,y=f.length>0;p.default.useEffect(()=>{d.data&&l(zh(d.data.channels||[]))},[d.data]);let $=Q({mutationFn:({channels:C})=>g_(C),onSuccess:C=>{l(zh(C.channels||[])),a.invalidateQueries({queryKey:b_}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let C=n.trim();!C||!m.isSuccess||(l(_=>zh([..._,{channel_id:C,subject_user_id:s}])),r(""))},v=C=>{l(_=>_.filter(A=>A.channel_id!==C))},b=(C,_)=>{l(A=>A.map(L=>L.channel_id===C?{...L,subject_user_id:_}:L))},w=()=>{$.mutate({channels:oM(o)})},N=m.isError&&o.some(C=>!C.subject_user_id);return u`
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
          onChange=${C=>r(C.target.value)}
          onKeyDown=${C=>C.key==="Enter"&&g()}
          placeholder=${c.inputPlaceholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <select
          value=${s}
          onChange=${C=>i(C.target.value)}
          disabled=${!y}
          className="h-9 min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
        >
          ${!y&&u`<option value="">${c.noSubjectsLabel}</option>`}
          ${y&&u`<option value="">${c.autoSubjectLabel}</option>`}
          ${h.map(C=>u`
              <option key=${C.subject_user_id} value=${C.subject_user_id}>
                ${C.display_name}
              </option>
            `)}
        </select>
        <${D}
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
        ${o.map(C=>u`
            <label
              key=${C.channel_id}
              className="flex min-h-10 items-center justify-between gap-3 border-t border-white/[0.05] px-3 first:border-t-0"
            >
              <span className="min-w-0">
                <span className="block truncate font-mono text-xs text-iron-200">
                  ${C.channel_id}
                </span>
              </span>
              <div className="flex shrink-0 items-center gap-2">
                ${y?u`
                    <select
                      value=${C.subject_user_id}
                      onChange=${_=>b(C.channel_id,_.target.value)}
                      className="h-8 rounded-md border border-white/10 bg-white/[0.04] px-2 text-xs text-iron-100 outline-none focus:border-signal/45"
                    >
                      <option value="">${c.autoSubjectLabel}</option>
                      ${x_(f,C).map(_=>u`
                          <option key=${_.subject_user_id} value=${_.subject_user_id}>
                            ${_.display_name}
                          </option>
                        `)}
                    </select>
                  `:u`<span className="max-w-40 truncate text-xs text-iron-500">
                    ${C.subject_user_id?C.subject_display_name||C.subject_user_id:c.autoSubjectLabel}
                  </span>`}
                <input
                  type="checkbox"
                  checked=${!0}
                  aria-label=${c.allowLabel(C.channel_id)}
                  onChange=${()=>v(C.channel_id)}
                  className="h-4 w-4 rounded border-white/20 bg-white/[0.04] text-signal"
                />
              </div>
            </label>
          `)}
      </div>

      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <${D}
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick=${w}
          disabled=${!d.isSuccess||!x||$.isPending||N}
        >
          ${$.isPending?c.savingLabel:c.submitLabel}
        <//>
        ${$.isSuccess&&u`<p className="text-xs text-emerald-300">
          ${c.successMessage}
        </p>`}
        ${(d.isError||m.isError||$.isError)&&u`<p className="text-xs text-red-300">
          ${y_($.error||d.error||m.error,c.errorMessage)}
        </p>`}
      </div>
    </div>
  `}function x_(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function zh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return p_(Array.from(t.keys())).map(a=>t.get(a))}function oM(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function lM(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var qh=["slack-setup"],Kr={installationId:{body:"Local IronClaw name for this Slack install. Choose one and keep it stable.",example:"Example: local-slack"},teamId:{body:"Slack workspace/team ID from the workspace that installed the app.",example:"Example: T0123456789"},appId:{body:"Slack app Basic Information > App Credentials.",example:"Example: A0123456789"},botUser:{body:"Optional Reborn user. Blank uses the current WebUI operator.",example:"Example: user:operator"},sharedSubject:{body:"Optional default team agent for shared channel turns. Usually blank.",example:"Example: user:slack-shared"},botToken:{body:"Slack app OAuth & Permissions > Bot User OAuth Token.",example:"Example: xoxb-..."},signingSecret:{body:"Slack app Basic Information > App Credentials > Signing Secret.",example:""}};function N_({action:e}){let t=K({queryKey:qh,queryFn:d_}),a=t.data?.configured===!0;return u`
    <div className="space-y-3">
      <${uM} action=${e} setupQuery=${t} />
      ${a&&u`<${$_} action=${e} />`}
    </div>
  `}function uM({action:e,setupQuery:t}){let a=J(),[n,r]=p.default.useState(cM()),s=p.default.useRef(!1),i=p.default.useRef(!1),o=t.data,l=dM(e);p.default.useEffect(()=>{!o||s.current||i.current||(r(w_(o)),s.current=!0)},[o]);let c=Q({mutationFn:m_,onSuccess:h=>{i.current=!1,r(w_(h)),s.current=!0,a.setQueryData(qh,h),a.invalidateQueries({queryKey:qh}),a.invalidateQueries({queryKey:["slack-allowed-channels"]}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["extensions"]})}}),d=h=>x=>{i.current=!0,r(y=>({...y,[h]:x.target.value}))},m=()=>c.mutate(n),f=n.installation_id.trim()&&n.team_id.trim()&&n.api_app_id.trim()&&(o?.bot_token_configured||n.bot_token.trim())&&(o?.signing_secret_configured||n.signing_secret.trim());return u`
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
        ${yl("Installation ID",n.installation_id,d("installation_id"),"",Kr.installationId)}
        ${yl("Team ID",n.team_id,d("team_id"),"",Kr.teamId)}
        ${yl("App ID",n.api_app_id,d("api_app_id"),"",Kr.appId)}
        ${yl("Bot user",n.user_id,d("user_id"),"default operator",Kr.botUser)}
        ${yl("Shared subject",n.shared_subject_user_id,d("shared_subject_user_id"),"optional",Kr.sharedSubject)}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        ${S_("Bot token",n.bot_token,d("bot_token"),o?.bot_token_configured,Kr.botToken)}
        ${S_("Signing secret",n.signing_secret,d("signing_secret"),o?.signing_secret_configured,Kr.signingSecret)}
      </div>

      <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <${D}
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
  `}function w_(e){return{installation_id:e.installation_id||"",team_id:e.team_id||"",api_app_id:e.api_app_id||"",user_id:e.user_id||"",shared_subject_user_id:e.shared_subject_user_id||"",bot_token:"",signing_secret:""}}function cM(){return{installation_id:"",team_id:"",api_app_id:"",user_id:"",shared_subject_user_id:"",bot_token:"",signing_secret:""}}function yl(e,t,a,n="",r=null){return u`
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-iron-500">${e}</span>
      <input
        type="text"
        value=${t}
        onChange=${a}
        placeholder=${n}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <${__} help=${r} />
    </label>
  `}function S_(e,t,a,n,r=null){return u`
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
      <${__} help=${r} />
    </label>
  `}function __({help:e}){return e?u`
    <p className="mt-1.5 min-h-8 text-[11px] leading-4 text-iron-400">
      <span className="block">${e.body}</span>
      ${e.example&&u`<span className="mt-0.5 block font-mono text-iron-300">${e.example}</span>`}
    </p>
  `:null}function dM(e){return{title:"Slack setup",instructions:e?.instructions||"Configure the Slack app before assigning channels.",submitLabel:"Save setup",successMessage:"Slack setup saved.",errorMessage:"Slack setup update failed."}}var Ih={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function Hr(e){return e==="wasm_channel"||e==="channel"}var k_={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},R_={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function C_(e){let t=E_(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||Hr(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function E_(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function Kh(e){let t=E_(e);return t==="active"||t==="ready"}function T_({extension:e,secrets:t=[],fields:a=[]}={}){return Kh(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var A_="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",D_="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",M_="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",O_="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",L_="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",mM="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function P_(e){return e.package_ref?.id||""}function fM({actions:e,isBusy:t}){let a=R(),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>{if(!n)return;let i=o=>{s.current&&!s.current.contains(o.target)&&r(!1)};return document.addEventListener("mousedown",i),()=>document.removeEventListener("mousedown",i)},[n]),u`
    <div ref=${s} className="relative shrink-0">
      <button
        type="button"
        aria-label=${a("extensions.moreActions")}
        aria-haspopup="true"
        aria-expanded=${n?"true":"false"}
        disabled=${t}
        onClick=${()=>r(i=>!i)}
        className="grid h-7 w-7 place-items-center rounded-md border border-transparent text-[var(--v2-text-faint)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] disabled:cursor-not-allowed disabled:opacity-50"
      >
        <${O} name="more" className="h-4 w-4" strokeWidth=${2.4} />
      </button>
      ${n&&u`
        <div
          role="menu"
          className="absolute right-0 top-8 z-10 min-w-[156px] rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-1 shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]"
        >
          ${e.map(i=>u`
              <button
                key=${i.id}
                type="button"
                role="menuitem"
                disabled=${t}
                onClick=${()=>{r(!1),i.run()}}
                className=${["flex w-full items-center gap-2.5 rounded-[7px] px-2.5 py-1.5 text-left text-[13px] disabled:cursor-not-allowed disabled:opacity-50",i.danger?"text-[var(--v2-danger-text)] hover:bg-[var(--v2-danger-soft)]":"text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)]"].join(" ")}
              >
                <${O} name=${i.icon||"settings"} className="h-3.5 w-3.5" />
                ${i.label}
              </button>
            `)}
        </div>
      `}
    </div>
  `}function U_({items:e}){return!e||e.length===0?null:u`
    <div className="mt-3 flex flex-wrap gap-1">
      ${e.map(t=>u`<span key=${t} className=${mM}>${t}</span>`)}
    </div>
  `}function wi({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=R(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=k_[i]||"muted",l=s(`extensions.state.${i}`)||R_[i]||i,c=s(`extensions.kind.${e.kind}`)||Ih[e.kind]||e.kind,d=e.display_name||P_(e),m=!!e.package_ref,f=e.tools||[],[h,x]=p.default.useState(!1),$=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],b=[],w=C_(e);w==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):w==="activate"&&v.push({id:"activate",label:"Activate",run:()=>t(g)}),m&&(e.needs_setup||e.has_auth)&&w!=="configure"&&b.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)}),m&&Hr(e.kind)&&(i==="setup_required"||i==="failed")&&b.push({id:"setup",label:"Setup",icon:"settings",run:()=>a(g)}),m&&Hr(e.kind)&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&b.push({id:"reconfigure",label:"Reconfigure",icon:"settings",run:()=>a(g)}),m&&b.push({id:"remove",label:s("common.remove")||"Remove",icon:"trash",danger:!0,run:()=>n(g)});let N=v[0];return u`
    <div className=${A_}>
      <div className="flex items-start gap-2">
        <${q} tone=${o} label=${l} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${b.length>0&&u`<${fM} actions=${b} isBusy=${r} />`}
      </div>

      <div className=${D_}>
        <span>${c}</span>
        ${e.version&&u`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&u`<p className=${M_}>${e.description}</p>`}

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

      <div className=${O_}>
        ${f.length>0?u`
              <button
                type="button"
                aria-expanded=${h?"true":"false"}
                onClick=${()=>x(C=>!C)}
                className=${L_}
              >
                <${O} name="layers" className="h-3.5 w-3.5" />
                <span>${f.length===1?s("extensions.oneCapability"):s("extensions.pluralCapabilities",{count:f.length})}</span>
                <${O}
                  name="chevron"
                  className=${["h-3 w-3",h?"rotate-180":""].join(" ")}
                />
              </button>
            `:u`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">No capabilities</span>`}
        <span className="flex-1"></span>
        ${N&&u`
          <${D} variant="secondary" size="sm" onClick=${N.run} disabled=${r}>
            ${N.label}
          <//>
        `}
      </div>

      ${h&&u`<${U_} items=${f} />`}
    </div>
  `}function Qr({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=R(),s=r(`extensions.kind.${e.kind}`)||Ih[e.kind]||e.kind,i=e.display_name||P_(e),o=!!(e.package_ref&&t),l=e.keywords||[],[c,d]=p.default.useState(!1);return u`
    <div className=${A_}>
      <div className="flex items-start gap-2">
        <${q}
          tone="muted"
          label=${n||r("extensions.state.available")||"available"}
          size="sm"
        />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${i}
        </span>
      </div>

      <div className=${D_}>
        <span>${s}</span>
        ${e.version&&u`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&u`<p className=${M_}>${e.description}</p>`}

      <div className=${O_}>
        ${l.length>0?u`
              <button
                type="button"
                aria-expanded=${c?"true":"false"}
                onClick=${()=>d(m=>!m)}
                className=${L_}
              >
                <${O} name="list" className="h-3.5 w-3.5" />
                <span>${l.length===1?r("extensions.oneKeyword"):r("extensions.pluralKeywords",{count:l.length})}</span>
                <${O}
                  name="chevron"
                  className=${["h-3 w-3",c?"rotate-180":""].join(" ")}
                />
              </button>
            `:u`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]"></span>`}
        <span className="flex-1"></span>
        ${o&&u`
          <${D}
            variant="outline"
            size="sm"
            onClick=${()=>t({packageRef:e.package_ref,displayName:i})}
            disabled=${a}
          >
            <${O} name="plus" className="mr-1.5 h-3.5 w-3.5" />
            Install
          <//>
        `}
      </div>

      ${c&&u`<${U_} items=${l} />`}
    </div>
  `}function j_(){return H("/api/webchat/v2/extensions")}function F_(){return H("/api/webchat/v2/extensions/registry")}function B_(e){return H("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function z_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(bl(e))}/activate`,{method:"POST"})}function q_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(bl(e))}/remove`,{method:"POST"})}function I_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(bl(e))}/setup`)}function K_(e,t,a){return a$(bl(e),{action:"submit",payload:{secrets:t,fields:a}})}function H_(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return H(`/api/webchat/v2/extensions/${encodeURIComponent(bl(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function Q_(){return Promise.resolve({requests:[]})}function V_(){return Promise.resolve({success:!1,message:"Pairing requires a v2 pairing endpoint."})}function bl(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var pM=2e3,hM=10*60*1e3;function Si(e){return e?.package_ref?.id||null}function Hh(e){return e?.display_name||Si(e)||""}function G_(e,t,a){return Si(t)||`${e}:${Hh(t)||"unknown"}:${a}`}function vM(e,t){return e.installed!==t.installed?e.installed?-1:1:Hh(e.entry||e.extension).localeCompare(Hh(t.entry||t.extension))}function Y_(){let e=J(),t=K({queryKey:["gateway-status-extensions"],queryFn:ri,staleTime:1e4}),a=K({queryKey:["extensions"],queryFn:j_}),n=K({queryKey:["extension-registry"],queryFn:F_}),r=K({queryKey:["connectable-channels"],queryFn:Vc}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["extensions"]}),e.invalidateQueries({queryKey:["extension-registry"]}),e.invalidateQueries({queryKey:["gateway-status-extensions"]}),e.invalidateQueries({queryKey:["connectable-channels"]})},[e]),[i,o]=p.default.useState(null),l=p.default.useCallback(()=>o(null),[]),c=Q({mutationFn:({packageRef:k})=>B_(k),onSuccess:(k,{displayName:z})=>{k.success?(o({type:"success",message:k.message||k.instructions||`${z||"Extension"} installed`}),k.auth_url&&window.open(k.auth_url,"_blank","noopener,noreferrer")):o({type:"error",message:k.message||"Install failed"}),s()},onError:k=>{o({type:"error",message:k.message}),s()}}),d=Q({mutationFn:({packageRef:k})=>z_(k),onSuccess:(k,{displayName:z})=>{k.success?(o({type:"success",message:k.message||k.instructions||`${z||"Extension"} activated`}),k.auth_url&&window.open(k.auth_url,"_blank","noopener,noreferrer")):k.auth_url?(window.open(k.auth_url,"_blank","noopener,noreferrer"),o({type:"info",message:"Opening authentication\u2026"})):k.awaiting_token?o({type:"info",message:"Configuration required"}):o({type:"error",message:k.message||"Activation failed"}),s()},onError:k=>{o({type:"error",message:k.message})}}),m=Q({mutationFn:({packageRef:k})=>q_(k),onSuccess:(k,{displayName:z})=>{k.success?o({type:"success",message:`${z||"Extension"} removed`}):o({type:"error",message:k.message||"Remove failed"}),s()},onError:k=>{o({type:"error",message:k.message})}}),f=t.data||{},h=a.data?.extensions||[],x=n.data?.entries||[],y=r.data?.channels||[],$=new Map(h.map(k=>[Si(k),k]).filter(([k])=>!!k)),g=new Set(x.map(k=>Si(k)).filter(Boolean)),v=[...x.map((k,z)=>{let Z=Si(k),ne=Z&&$.get(Z)||null;return{id:G_("registry",k,z),installed:!!(ne||k.installed),entry:k,extension:ne}}),...h.filter(k=>{let z=Si(k);return!z||!g.has(z)}).map((k,z)=>({id:G_("installed",k,z),installed:!0,entry:null,extension:k}))].sort(vM),b=k=>Hr(k.kind),w=h.filter(b),N=h.filter(k=>k.kind==="mcp_server"),C=h.filter(k=>!b(k)&&k.kind!=="mcp_server"),_=x.filter(k=>b(k)&&!k.installed),A=x.filter(k=>k.kind==="mcp_server"&&!k.installed),L=x.filter(k=>k.kind!=="mcp_server"&&!b(k)&&!k.installed),M=a.isLoading||n.isLoading,P=c.isPending||d.isPending||m.isPending;return{status:f,extensions:h,channels:w,mcpServers:N,tools:C,channelRegistry:_,mcpRegistry:A,toolRegistry:L,registry:x,catalogEntries:v,connectableChannels:y,isLoading:M,isBusy:P,actionResult:i,clearResult:l,install:c.mutate,activate:d.mutate,remove:m.mutate,invalidate:s}}function J_(e){let t=K({queryKey:["extension-setup",e?.id||e],queryFn:()=>I_(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function X_(e,t){let a=J(),n=e?.id||e;return Q({mutationFn:({secrets:r,fields:s})=>K_(e,r,s),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function Z_(e){let t=J(),a=e?.id||e,n=p.default.useRef(null),r=p.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=p.default.useCallback(()=>{let l=t.getQueryData(["extension-setup",a]);if(l?.secrets?.length>0&&l.secrets.every(f=>f.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(f=>f.package_ref?.id===a),m=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return m==="active"||m==="ready"},[a,t]),o=p.default.useCallback(l=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||l&&l.closed||Date.now()-c>hM)&&(r(),s())},pM)},[r,s,i]);return p.default.useEffect(()=>r,[r]),Q({mutationFn:({secret:l,popup:c})=>H_(e,l).then(d=>({res:d,popup:c})),onSuccess:({res:l,popup:c})=>{let d=c;l.authorization_url&&c&&!c.closed?c.location.href=l.authorization_url:l.authorization_url?d=window.open(l.authorization_url,"_blank","noopener,noreferrer"):c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(l,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function W_(e,t={}){let a=K({queryKey:["pairing",e],queryFn:()=>Q_(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=J(),r=Q({mutationFn:({code:s})=>V_(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function ek(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var gM={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function tk({channel:e,redeemFn:t,i18nKeys:a=gM,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=R(),o=typeof t=="function",l=W_(e,{enabled:!o}),c=J(),[d,m]=p.default.useState(""),f=yM(i,a,r),h=Q({mutationFn:({code:N})=>t(e,N),onSuccess:()=>{m("");for(let N of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:N})}}),x=p.default.useCallback(N=>l.approve({code:N}),[l.approve]),y=p.default.useCallback(()=>{let N=d.trim();N&&(o?h.mutate({code:N}):(l.approve({code:N}),m("")))},[o,d,l.approve,h]),$=o?[]:l.requests,g=o?!1:l.isLoading,v=o?h.isPending:l.isApproving,b=o?h.isSuccess?h.data:null:l.result,w=o?h.isError?h.error:null:l.error;return g?u`
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
          onChange=${N=>m(N.target.value)}
          onKeyDown=${N=>N.key==="Enter"&&y()}
          placeholder=${f.placeholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${D}
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
        ${ek(w,f.error)}
      </p>`}

      ${s&&$.length>0?u`
            <div className="space-y-2">
              ${$.map(N=>u`
                <div
                  key=${N.code||N.id}
                  className="flex items-center justify-between gap-3 rounded-md border border-white/[0.06] bg-white/[0.02] px-3 py-2"
                >
                  <div className="min-w-0">
                    <span className="font-mono text-sm text-iron-200">${N.code||N.id}</span>
                    ${N.label&&u`
                      <span className="ml-2 text-xs text-iron-300">${N.label}</span>
                    `}
                  </div>
                  <${D}
                    variant="secondary"
                    className="h-7 px-2.5 text-xs"
                    onClick=${()=>x(N.code||N.id)}
                    disabled=${v}
                  >
                    ${f.action}
                  <//>
                </div>
              `)}
            </div>
          `:s&&u`<p className="text-xs text-iron-300">${i(a.empty)}</p>`}
    </div>
  `}function yM(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function bd(e){return e.package_ref?.id||""}function ak(e){return bd(e)==="slack"}function rk(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function sk(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function bM(e){let t=e||[],a=[t.find(rk),t.find(sk)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function nk({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>rk(r)?u`<${N_} action=${r.action} />`:sk(r)?u`<${zc} action=${r.action} />`:null).filter(Boolean);return n.length>0?u`<div className="space-y-3">${n}</div>`:null}function ik({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:l}){let c=R(),d=t||[],m=e.enabled_channels||[],f=bM(a),h=d.some(ak),x=f.length>0&&!h;return u`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          ${c("channels.builtIn")}
        </h3>
        <${Ni}
          name="Web Gateway"
          description=${c("channels.webGatewayDesc")||"Browser-based chat with SSE streaming"}
          enabled=${!0}
          detail=${"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)}
        />
        <${Ni}
          name="HTTP Webhook"
          description=${c("channels.httpWebhookDesc")||"Inbound webhook endpoint for external integrations"}
          enabled=${m.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${Ni}
          name="CLI"
          description=${c("channels.cliDesc")||"Terminal interface with TUI or simple REPL"}
          enabled=${m.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${Ni}
          name="REPL"
          description=${c("channels.replDesc")||"Minimal read-eval-print loop for testing"}
          enabled=${m.includes("repl")}
          detail="ironclaw run --repl"
        />
        ${x&&u`
          <${Ni}
            name=${c("channels.slack")||"Slack"}
            description=${c("channels.slackDesc")||"Tenant app channel for DMs and app mentions"}
            enabled=${!1}
            statusLabel="setup"
            statusTone="muted"
            detail=${c("channels.slackDetail")||"Tenant Slack app install"}
          >
            <${nk}
              slackConnectActions=${f}
            />
          </${Ni}>
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
                <div key=${bd(y)} className="flex flex-col gap-3">
                  <${wi}
                    ext=${y}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${l}
                  />
                  ${ak(y)&&u`<${nk}
                    slackConnectActions=${f}
                  />`}
                  ${(y.onboarding_state==="pairing_required"||y.onboarding_state==="pairing")&&u` <${tk} channel=${bd(y)} /> `}
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
                <${Qr}
                  key=${bd(y)}
                  entry=${y}
                  onInstall=${o}
                  isBusy=${l}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function Ni({name:e,description:t,enabled:a,detail:n,children:r,statusLabel:s=a?"on":"off",statusTone:i=a?"success":"muted"}){return u`
    <div
      className="border-t border-white/[0.06] py-4 first:border-0 first:pt-0"
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-iron-200">${e}</span>
            <${q}
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
  `}function ok({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=R(),s=e?.displayName||e?.packageRef?.id||"Extension",{secrets:i=[],fields:o=[],onboarding:l,isLoading:c,error:d}=J_(e?.packageRef),[m,f]=p.default.useState({}),[h,x]=p.default.useState({}),y=Z_(e?.packageRef),$=X_(e?.packageRef,_=>{_.success!==!1&&(n&&n(_),a())}),g=p.default.useCallback(()=>{let _={};for(let[A,L]of Object.entries(m)){let M=(L||"").trim();M&&(_[A]=M)}$.mutate({secrets:_,fields:h})},[m,h,$]),v=p.default.useCallback(_=>{let A=window.open("about:blank","_blank","width=600,height=600");A&&(A.opener=null),y.mutate({secret:_,popup:A})},[y]),w=i.filter(_=>(_.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,N=Kh(e),C=T_({extension:e,secrets:i,fields:o});return c?u`
      <${xd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(_=>u`<div
                key=${_}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?u`
      <${xd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")||"Failed to load setup:"} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?u`
      <${xd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")||"No configuration required for this extension."}
        </p>
      <//>
    `:u`
    <${xd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
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
          <${O} name="bolt" className="h-3.5 w-3.5" />
        </a>
      `}

      <div className="space-y-4">
        ${i.map(_=>u`
            <div key=${_.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                ${_.prompt||_.name}
                ${_.optional&&u`
                  <span className="font-mono text-[10px] text-iron-700"
                    >${r("common.optional")||"optional"}</span
                  >
                `}
                ${_.provided&&u`
                  <span className="font-mono text-[10px] text-mint"
                    >${r("common.configured")||"configured"}</span
                  >
                `}
              </label>
              ${(_.setup?.kind||"manual_token")==="oauth"?u`
                    <div className="flex items-center justify-between gap-3 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2">
                      <span className="text-xs text-iron-300">
                        ${_.provided?r("extensions.authConfigured")||"Authorization is configured.":r("extensions.authPopup")||"Authorize this provider in a browser popup."}
                      </span>
                      <${D}
                        variant=${_.provided?"secondary":"primary"}
                        onClick=${()=>v(_)}
                        disabled=${y.isPending}
                      >
                        ${y.isPending?r("extensions.opening"):_.provided?r("extensions.reconnect"):r("extensions.authorize")}
                      <//>
                    </div>
                  `:u`
              <input
                type="password"
                placeholder=${_.provided?"\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (leave blank to keep)":""}
                value=${m[_.name]||""}
                onChange=${A=>f(L=>({...L,[_.name]:A.target.value}))}
                onKeyDown=${A=>A.key==="Enter"&&g()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
              ${_.auto_generate&&!_.provided&&u`
                <p className="mt-1 text-xs text-iron-700">
                  ${r("extensions.autoGenerated")||"Auto-generated if left blank"}
                </p>
              `}
                  `}
            </div>
          `)}
        ${o.map(_=>u`
            <div key=${_.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                ${_.prompt||_.name}
                ${_.optional&&u`
                  <span className="font-mono text-[10px] text-iron-700"
                    >${r("common.optional")||"optional"}</span
                  >
                `}
              </label>
              <input
                type="text"
                placeholder=${_.placeholder||""}
                value=${h[_.name]||""}
                onChange=${A=>x(L=>({...L,[_.name]:A.target.value}))}
                onKeyDown=${A=>A.key==="Enter"&&g()}
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
      ${N&&u`
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
        <${D} variant="ghost" onClick=${a}>${r("common.cancel")||"Cancel"}<//>
        ${C&&u`
        <${D}
          variant="primary"
          onClick=${()=>t?.(e)}
        >
          Activate
        <//>
        `}
        ${w&&u`
        <${D}
          variant=${C?"secondary":"primary"}
          onClick=${g}
          disabled=${$.isPending}
        >
          ${$.isPending?"Saving\u2026":r("common.save")||"Save"}
        <//>
        `}
      </div>
    <//>
  `}function xd({onClose:e,title:t,children:a}){return p.default.useEffect(()=>{let n=r=>{r.key==="Escape"&&e()};return window.addEventListener("keydown",n),()=>window.removeEventListener("keydown",n)},[e]),u`
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
            <${O} name="close" className="h-4 w-4" />
          </button>
        </div>
        ${a}
      </div>
    </div>
  `}function lk(e){return e.package_ref?.id||""}function uk({mcpServers:e,mcpRegistry:t,onActivate:a,onConfigure:n,onRemove:r,onInstall:s,isBusy:i}){let o=R();return e.length===0&&t.length===0?u`
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
                <${wi}
                  key=${lk(l)}
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
                <${Qr}
                  key=${lk(l)}
                  entry=${l}
                  onInstall=${s}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function xM(e){return e?.package_ref?.id||""}function $M(e){return e.entry||e.extension||{}}function ck({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=R(),[o,l]=p.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(y=>{let $=$M(y);return($.display_name||xM($)).toLowerCase().includes(c)||($.description||"").toLowerCase().includes(c)||($.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,m=d.filter(y=>y.installed&&y.extension),f=d.filter(y=>y.installed&&!y.extension&&y.entry),h=m.length+f.length,x=d.filter(y=>!y.installed&&y.entry);return e.length===0?u`
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
                      <${wi}
                        key=${y.id}
                        ext=${y.extension||y.entry}
                        onActivate=${a}
                        onConfigure=${n}
                        onRemove=${r}
                        isBusy=${s}
                      />
                    `)}
                  ${f.map(y=>u`
                      <${Qr}
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
                      <${Qr}
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
  `}function Qh(){let{tab:e="registry"}=st(),[t,a]=p.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:l,connectableChannels:c,isLoading:d,isBusy:m,actionResult:f,clearResult:h,install:x,activate:y,remove:$,invalidate:g}=Y_(),v=p.default.useCallback(_=>a(_),[]),b=p.default.useCallback(()=>a(null),[]),w=p.default.useCallback(()=>g(),[g]),N=p.default.useCallback(_=>{_&&(y(_),a(null))},[y]);if(d)return u`
      <div className="flex h-full flex-col overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${[1,2,3].map(_=>u`
                <div
                  key=${_}
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
    `;if(e==="installed")return u`<${it} to="/extensions/registry" replace />`;let C={channels:u`<${ik}
      status=${n}
      channels=${r}
      connectableChannels=${c}
      channelRegistry=${i}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${x}
      isBusy=${m}
    />`,mcp:u`<${uk}
      mcpServers=${s}
      mcpRegistry=${o}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${x}
      isBusy=${m}
    />`,registry:u`<${ck}
      catalogEntries=${l}
      onInstall=${x}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      isBusy=${m}
    />`};return C[e]?u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <${l_} result=${f} onDismiss=${h} />
          ${C[e]}
        </div>
      </div>

      ${t&&u`
        <${ok}
          extension=${t}
          onActivate=${N}
          onClose=${b}
          onSaved=${w}
        />
      `}
    </div>
  `:u`<${it} to="/extensions/registry" replace />`}var dk=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],mk=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],fk=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],Vh=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function pk(e){return String(e||"").trim().toLowerCase()}function hk(e){if(e==null)return"";if(Array.isArray(e))return e.map(hk).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function et(e,t){let a=pk(e);return a?t.map(hk).join(" ").toLowerCase().includes(a):!0}function _i(e,t,a,n){let r=pk(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(l=>et(r,[i,l.key,l.labelKey?n(l.labelKey):l.label,l.descKey?n(l.descKey):l.description,t[l.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function wM({visible:e}){let t=R();return e?u`
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      ${t("tools.saved")}
    </span>
  `:null}function SM({checked:e,onChange:t,label:a}){return u`
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
  `}function NM({field:e,value:t,onSave:a,isSaved:n}){let r=R(),[s,i]=p.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",l=e.descKey?r(e.descKey):e.description||"";p.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=p.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let m=parseInt(d,10);isNaN(m)||a(e.key,m)}else if(e.type==="float"){let m=parseFloat(d);isNaN(m)||a(e.key,m)}else a(e.key,d)},[e.key,e.type,a]);return u`
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">${o}</div>
        ${l&&u`<div className="mt-1 text-xs leading-5 text-iron-300">${l}</div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${e.type==="boolean"?u`
              <${SM}
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
        <${wM} visible=${n} />
      </div>
    </div>
  `}function ki({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=R(),o=t?i(t):e||"";return u`
    <${ee} className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${o}</h3>
      <div>
        ${a.map(l=>u`
              <${NM}
                key=${l.key}
                field=${l}
                value=${n[l.key]}
                onSave=${r}
                isSaved=${s[l.key]}
              />
            `)}
      </div>
    <//>
  `}function St({query:e}){let t=R();return u`
    <${ee} padding="lg">
      <div className="flex items-center gap-3">
        <span
          className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-faint)]"
        >
          <${O} name="search" className="h-4 w-4" />
        </span>
        <div className="min-w-0">
          <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
            ${t("settings.noMatchingSettings",{query:e})}
          </h3>
        </div>
      </div>
    <//>
  `}function vk({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=R();if(n)return u`<${_M} />`;let i=_i(mk,e,r,s);return i.length===0?u`<${St} query=${r} />`:u`
    <div className="space-y-5">
      ${i.map(o=>u`
            <${ki}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function _M(){return u`
    <div className="space-y-5">
      ${[1,2,3].map(e=>u`
            <${ee} key=${e} padding="md">
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
  `}function gk(){let e=K({queryKey:["gateway-status-settings"],queryFn:ri,staleTime:1e4}),t=K({queryKey:["extensions"],queryFn:X$}),a=K({queryKey:["extension-registry"],queryFn:Z$}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(m=>m.kind==="wasm_channel"||m.kind==="channel"),o=s.filter(m=>(m.kind==="wasm_channel"||m.kind==="channel")&&!m.installed),l=r.filter(m=>m.kind==="mcp_server"),c=s.filter(m=>m.kind==="mcp_server"&&!m.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:l,mcpRegistry:c,extensions:r,isLoading:d}}function kM({name:e,description:t,enabled:a,detail:n}){let r=R();return u`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${e}</span>
          <${q}
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
  `}function yk({channel:e,registryEntry:t}){let a=R(),n=t?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},l={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return u`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${n}</span>
          ${s?u`<${q}
                tone=${o[i]||"muted"}
                label=${l[i]||i}
                size="sm"
              />`:u`<${q}
                tone="muted"
                label=${a("channels.available")}
                size="sm"
              />`}
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${r}</div>
      </div>
    </div>
  `}function RM(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function CM({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=RM(e,i).filter(x=>et(s,[i("channels.builtIn"),x.id,x.name,x.description,x.detail])),l=new Set(t.map(x=>x.name)),c=t.filter(x=>et(s,[i("channels.messaging"),x.name,x.display_name,x.description,x.onboarding_state])),d=a.filter(x=>!l.has(x.name)).filter(x=>et(s,[i("channels.messaging"),x.name,x.display_name,x.description])),m=new Set(n.map(x=>x.name)),f=n.filter(x=>et(s,[i("channels.mcpServers"),x.name,x.display_name,x.description,x.active?i("channels.active"):i("channels.inactive")])),h=r.filter(x=>!m.has(x.name)).filter(x=>et(s,[i("channels.mcpServers"),x.name,x.display_name,x.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:h}}function bk({searchQuery:e=""}){let t=R(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=gk();if(o)return u`
      <div className="space-y-5">
        <${ee} padding="md">
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
    `;let{builtInChannels:l,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:f}=CM({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return l.length===0&&c.length===0&&d.length===0&&m.length===0&&f.length===0?u`<${St} query=${e} />`:u`
    <div className="space-y-5">
      ${l.length>0&&u`
      <${ee} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${l.map(h=>u`
            <${kM}
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
        <${ee} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.messaging")}
          </h3>
          ${c.map(h=>u`
              <${yk}
                key=${h.name}
                channel=${h}
                registryEntry=${r.find(x=>x.name===h.name)}
              />
            `)}
          ${d.map(h=>u`
              <${yk} key=${h.name} registryEntry=${h} />
            `)}
        <//>
      `}
      ${(m.length>0||f.length>0)&&u`
        <${ee} padding="md">
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
                      <${q}
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
                      <${q}
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
  `}function xk({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:l,onNearaiWallet:c,onCodexLogin:d,loginBusy:m}){let f=R(),h=e.id===t,x=Br(e,n),y=oi(e,n),$=dw(e,n,t,a),g=Ec(e,n),v=mw(e),b=f(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[w,N]=p.default.useState(h),C=p.default.useCallback(()=>N(Ce=>!Ce),[]);p.default.useEffect(()=>{N(h)},[h]);let _=x?u`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${il(e.adapter)} · ${$||e.default_model||f("llm.none")}
      </span>`:u`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${b}
      </span>`,A=e.id==="nearai"||e.id==="openai_codex",L=e.api_key_set===!0||e.has_api_key===!0,M=e.builtin?e.id==="nearai"&&v&&!L?f("llm.addApiKey"):f("llm.configure"):f("common.edit"),P=v&&e.builtin?u`
          <${D}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${r}
            onClick=${()=>i(e)}
          >
            ${M}
          <//>
        `:null,k=!h&&e.id==="nearai"?u`
          ${P}
          <${D} type="button" variant="secondary" size="sm" disabled=${m} onClick=${c}>
            ${f("onboarding.nearWallet")}
          <//>
          <${D} type="button" variant="secondary" size="sm" disabled=${m} onClick=${()=>l("github")}>
            GitHub
          <//>
          <${D} type="button" variant="secondary" size="sm" disabled=${m} onClick=${()=>l("google")}>
            Google
          <//>
        `:!h&&e.id==="openai_codex"?u`
          <${D} type="button" variant="secondary" size="sm" disabled=${m} onClick=${d}>
            ${f("onboarding.codexSignIn")}
          <//>
        `:null,Z=!h&&x&&(!A||e.id==="nearai"&&e.has_api_key===!0)?u`
        <${D}
          type="button"
          variant="primary"
          size="sm"
          disabled=${r}
          onClick=${()=>s(e)}
        >
          ${f("llm.use")}
        <//>
      `:null,ne=x?null:u`
        <${D}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${r}
          onClick=${()=>i(e)}
        >
          ${f(g==="api_key"?"llm.addApiKey":"llm.configure")}
        <//>
      `,de=h?null:Z||(A?k:ne),fe=!A&&(e.builtin&&e.id!=="bedrock"||!e.builtin)||e.id==="nearai"&&v;return u`
    <${ee}
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
          onClick=${C}
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
            ${h&&u`<${q} tone="positive" label=${f("llm.active")} size="sm" />`}
            ${e.builtin&&!h&&u`<${q} tone="muted" label=${f("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${_}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 py-3 pr-4 sm:pr-5">
          ${de}
          <button
            type="button"
            onClick=${C}
            data-testid="llm-provider-chevron"
            aria-label=${f(w?"llm.collapseDetails":"llm.expandDetails")}
            className=${["grid h-7 w-7 place-items-center rounded-md text-[var(--v2-text-faint)] transition-transform hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]",w?"rotate-180":""].join(" ")}
          >
            <${O} name="chevron" className="h-4 w-4" />
          </button>
        </div>
      </div>

      ${w&&u`
        <div data-testid="llm-provider-details" className="border-t border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-4 sm:px-5">
          <div className="grid gap-3 text-xs text-[var(--v2-text-muted)] sm:grid-cols-3">
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${f("llm.adapter")}</div>
              <div className="mt-1 truncate">${il(e.adapter)}</div>
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
            ${fe&&u`
              <${D}
                type="button"
                variant="secondary"
                size="sm"
                disabled=${r}
                onClick=${()=>i(e)}
              >
                ${M}
              <//>
            `}
            ${!e.builtin&&!h&&u`
              <${D}
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
  `}var EM=[{key:"active",labelKey:"llm.groupActive",dotClass:"bg-[var(--v2-positive-text)]"},{key:"ready",labelKey:"llm.groupReady",dotClass:"bg-[var(--v2-accent)]"},{key:"setup",labelKey:"llm.groupSetup",dotClass:"bg-[var(--v2-warning-text)]"}];function TM({label:e,count:t,dotClass:a}){return u`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full "+a} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${t}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `}function $k({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=R(),r=ed({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=td(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return u`<${St} query=${a} />`;let l=fw(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return u`
    <${ee} className="p-4 sm:p-6">
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${n("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">${n("llm.providersDesc")}</p>
        </div>
        <${D} type="button" variant="secondary" size="sm" className="gap-2" onClick=${()=>r.openDialog(null)}>
          <${O} name="plus" className="h-3.5 w-3.5" />
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

      <${Wc} login=${i} />

      ${s.isLoading?u`<div className="text-sm text-[var(--v2-text-muted)]">${n("common.loading")}</div>`:s.error?u`<div className="text-sm text-red-200">${n("error.loadFailed",{what:n("llm.providers"),message:s.error.message})}</div>`:u`
            <div className="space-y-1">
              ${EM.flatMap(c=>{let d=l[c.key];return d.length?[u`
                    <section
                      key=${c.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${c.key}
                      className="mb-3"
                    >
                      <${TM}
                        label=${n(c.labelKey)}
                        count=${d.length}
                        dotClass=${c.dotClass}
                      />
                      <div className="space-y-2">
                      ${d.map(m=>u`
                          <${xk}
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

      <${Zc}
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
  `}function wk({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=R(),{activeProviderId:o,selectedModel:l,providers:c,hasActiveProvider:d}=li({settings:e,gatewayStatus:t});if(r)return u`<${AM} />`;let m=d?o:"",f=c.find(g=>g.id===o),h=d&&(l||f?.default_model||e.selected_model)||"",x=_i(dk,e,s,i),y=et(s,[i("inference.provider"),i("inference.backend"),m,i("inference.model"),h]),$=et(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!y&&!$&&x.length===0?u`<${St} query=${s} />`:u`
    <div className="space-y-5">
      ${y&&u`
      <${ee} padding="none" className="p-4 sm:p-5">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${i("inference.provider")}</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${i("inference.backend")}</div>
            <div className="mt-1 flex items-center gap-2">
              <span className="font-mono text-lg font-semibold text-[var(--v2-text-strong)]">${m||i("inference.none")}</span>
              ${d?u`<${q} tone="positive" label=${i("inference.active")} size="sm" />`:u`<${q} tone="muted" label=${i("llm.notConfigured")} size="sm" />`}
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
        <${$k}
          settings=${e}
          gatewayStatus=${t}
          searchQuery=${s}
        />
      `}

      ${x.map(g=>u`
            <${ki}
              key=${g.groupKey}
              groupKey=${g.groupKey}
              fields=${g.fields}
              settings=${e}
              onSave=${a}
              savedKeys=${n}
            />
          `)}
    </div>
  `}function ir({className:e=""}){return u`
    <div
      className=${"rounded animate-pulse bg-[var(--v2-surface-muted)] "+e}
    />
  `}function AM(){return u`
    <div className="space-y-5">
      <${ee} padding="md">
        <${ir} className="mb-4 h-3 w-24" />
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${ir} className="h-3 w-16" />
            <${ir} className="mt-2 h-6 w-28" />
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${ir} className="h-3 w-16" />
            <${ir} className="mt-2 h-6 w-40" />
          </div>
        </div>
      <//>
      ${[1,2].map(e=>u`
            <${ee} key=${e} padding="md">
              <${ir} className="mb-4 h-3 w-20" />
              ${[1,2,3].map(t=>u`
                    <div key=${t} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                      <${ir} className="h-4 w-32" />
                      <${ir} className="h-9 w-36" />
                    </div>
                  `)}
            <//>
          `)}
    </div>
  `}function Sk({searchQuery:e=""}){let t=R(),{lang:a,setLang:n}=$l(),r=wl.find(i=>i.code===a)||wl[0],s=wl.filter(i=>et(e,[i.code,i.name,i.native]));return s.length===0?u`<${St} query=${e} />`:u`
    <${ee} padding="md">
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
  `}function Nk({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=R();if(n)return u`
      <div className="space-y-5">
        ${[1,2].map(o=>u`
              <${ee} key=${o} padding="md">
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
    `;let i=_i(fk,e,r,s);return i.length===0?u`<${St} query=${r} />`:u`
    <div className="space-y-5">
      ${i.map(o=>u`
            <${ki}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function _k(){let e=R(),[t,a]=p.default.useState(!1),n=p.default.useCallback(()=>a(!0),[]),r=p.default.useCallback(()=>a(!1),[]),s=p.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function kk({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=R(),r=_k({gatewayStatus:t,gatewayStatusQuery:a});return e?u`
    <div className="space-y-3">
      <div
        role="alert"
        className="flex flex-col gap-3 rounded-xl border border-copper/30 bg-copper/10 px-4 py-3 sm:flex-row sm:items-center"
      >
        <div className="flex min-w-0 flex-1 items-start gap-3">
          <${O} name="bolt" className="mt-0.5 h-4 w-4 shrink-0 text-copper" />
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

        <${D}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${!r.restartEnabled||r.isRestarting}
          onClick=${r.openConfirm}
          title=${r.restartEnabled?void 0:r.unavailableReason}
          className="w-full sm:w-auto"
        >
          <${O} name=${r.isRestarting?"pulse":"bolt"} className="h-4 w-4" />
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

    <${pi}
      open=${r.confirmOpen}
      onClose=${r.closeConfirm}
      title=${n("restart.title")}
      size="sm"
    >
      <${hi} className="space-y-3">
        <p className="text-sm text-[var(--v2-text)]">
          ${n("restart.description")}
        </p>
        <div className="rounded-xl border border-copper/25 bg-copper/10 px-3 py-2 text-xs text-copper">
          ${n("restart.warning")}
        </div>
      <//>
      <${vi}>
        <${D}
          type="button"
          variant="ghost"
          size="sm"
          disabled=${r.isRestarting}
          onClick=${r.closeConfirm}
        >
          ${n("restart.cancel")}
        <//>
        <${D}
          type="button"
          variant="danger"
          size="sm"
          disabled=${r.isRestarting}
          onClick=${r.confirmRestart}
        >
          <${O} name="bolt" className="h-4 w-4" />
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
            <${O} name="pulse" className="h-5 w-5 animate-pulse" />
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
  `:null}function Rk(){let e=J(),t=K({queryKey:["skills"],queryFn:W$}),a=Q({mutationFn:tw,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),n=Q({mutationFn:nw,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),r=Q({mutationFn:({name:c,content:d})=>aw(c,{content:d}),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),s=Q({mutationFn:({name:c,enabled:d})=>rw(c,d),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),i=Q({mutationFn:c=>sw(c),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),o=t.data?.skills||[],l=t.data?.auto_activate_learned!==!1;return{skills:o,query:t,autoActivateLearned:l,fetchSkillContent:ew,installSkill:a.mutateAsync,removeSkill:n.mutateAsync,updateSkill:r.mutateAsync,setSkillAutoActivate:s.mutateAsync,setAutoActivateLearned:i.mutateAsync,isInstalling:a.isPending,isRemoving:n.isPending,isUpdating:r.isPending,isSettingAutoActivate:s.isPending,isSettingAutoActivateLearned:i.isPending}}function Ck({skill:e,onEdit:t,onRemove:a,onUpdate:n,onSetAutoActivate:r,isRemoving:s,isUpdating:i,isSettingAutoActivate:o}){let l=R(),c=e.name||e.id,d=e.trust||e.trust_level||"installed",m=e.source_kind||"installed",f=!!e.can_edit,h=!!e.can_delete,x=e.auto_activate!==!1,[y,$]=p.default.useState(!1),[g,v]=p.default.useState(""),[b,w]=p.default.useState(""),[N,C]=p.default.useState(!1);p.default.useEffect(()=>{y||(v(""),w(""))},[y]);let _=p.default.useCallback(async()=>{C(!0),w("");try{let L=await t(c);v(L?.content||""),$(!0)}catch(L){w(L.message||l("skills.contentLoadFailed"))}finally{C(!1)}},[c,t,l]),A=p.default.useCallback(async()=>{(await n(c,g))?.success&&$(!1)},[g,c,n]);return u`
    <div className="ext-card border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text)]">${c}</span>
            <${q}
              tone=${String(d).toLowerCase()==="trusted"?"positive":"muted"}
              label=${d}
              size="sm"
            />
            <${q}
              tone=${m==="system"?"positive":"muted"}
              label=${l(`skills.source.${m}`)}
              size="sm"
            />
            ${e.version&&u`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v${e.version}</span>`}
          </div>

          ${e.description&&u`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${e.description}</div>`}

          ${y?u`
                <div className="mt-3">
                  <${Fc}
                    rows=${12}
                    value=${g}
                    className="font-mono text-xs leading-5"
                    onInput=${L=>v(L.currentTarget.value)}
                  />
                </div>
              `:u`<${DM} skill=${e} />`}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          ${f&&!y&&u`
            <${D}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${i||N}
              title=${l("skills.edit")}
              onClick=${_}
            >
              <${O} name="file" className="h-4 w-4" />
              ${l(N?"skills.loading":"skills.edit")}
            <//>
          `}
          ${y&&u`
            <${D}
              type="button"
              variant="ghost"
              size="sm"
              disabled=${i}
              onClick=${()=>{v(""),$(!1)}}
            >
              <${O} name="close" className="h-4 w-4" />
              ${l("skills.cancel")}
            <//>
            <${D}
              type="button"
              variant="primary"
              size="sm"
              disabled=${i}
              onClick=${A}
            >
              <${O} name="check" className="h-4 w-4" />
              ${l(i?"skills.saving":"skills.save")}
            <//>
          `}
          ${f&&!y&&u`
            <${D}
              type="button"
              variant=${x?"secondary":"ghost"}
              size="sm"
              disabled=${o}
              title=${l(x?"skills.autoActivateOnTitle":"skills.autoActivateOffTitle")}
              onClick=${()=>r(c,!x)}
            >
              <${O} name=${x?"check":"close"} className="h-4 w-4" />
              ${l(x?"skills.autoActivateOnLabel":"skills.autoActivateOffLabel")}
            <//>
          `}
          ${h&&!y&&u`
            <${D}
              type="button"
              variant="danger"
              size="sm"
              disabled=${s}
              title=${l("skills.delete")}
              onClick=${()=>a(c)}
            >
              <${O} name="trash" className="h-4 w-4" />
              ${l("skills.delete")}
            <//>
          `}
        </div>
      </div>
      ${b&&u`<p className="mt-2 text-xs text-[var(--v2-danger-text)]">${b}</p>`}
    </div>
  `}function DM({skill:e}){let t=R();return u`
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
  `}function Ek({onInstall:e,isInstalling:t}){let a=R(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState({name:"",content:""}),[c,d]=p.default.useState(""),[m,f]=p.default.useState(""),h=p.default.useCallback((y,$)=>{l(g=>!g[y]||!$.trim()?g:{...g,[y]:""})},[]),x=p.default.useCallback(async()=>{let y=MM({name:n,content:s}),$=OM(y,a);if($.name||$.content){l($),d(""),f("");return}l({name:"",content:""}),d(""),f("");try{let g=await e(y);if(!g?.success){d(g?.message||a("skills.installFailed"));return}r(""),i(""),f(g.message||a("skills.installedSuccess",{name:y.name}))}catch(g){d(g.message||a("skills.installFailed"))}},[s,n,e,a]);return u`
    <${ee} padding="md">
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

      <${$n} label=${a("skills.name")} error=${o.name} required>
        <${Et}
          size="sm"
          error=${!!o.name}
          aria-invalid=${o.name?"true":void 0}
          value=${n}
          placeholder=${a("skills.namePlaceholder")}
          onInput=${y=>{let $=y.currentTarget.value;r($),h("name",$)}}
        />
      <//>

      <${$n}
        className="mt-3"
        label=${a("skills.content")}
        error=${o.content}
        hint=${a("skills.contentHint")}
        required
      >
        <${Fc}
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
        <${D} type="button" size="sm" disabled=${t} onClick=${x}>
          <${O} name="upload" className="h-4 w-4" />
          ${a(t?"skills.installing":"skills.install")}
        <//>
      </div>
    <//>
  `}function MM({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function OM(e,t){return{name:e.name?"":t("skills.nameRequired"),content:e.content?"":t("skills.contentRequired")}}function Tk({searchQuery:e=""}){let t=R(),{skills:a,query:n,autoActivateLearned:r,fetchSkillContent:s,installSkill:i,removeSkill:o,updateSkill:l,setSkillAutoActivate:c,setAutoActivateLearned:d,isInstalling:m,isRemoving:f,isUpdating:h,isSettingAutoActivate:x,isSettingAutoActivateLearned:y}=Rk(),[$,g]=p.default.useState(""),[v,b]=p.default.useState(""),w=p.default.useCallback(async L=>{if(window.confirm(t("skills.confirmDelete",{name:L}))){g(""),b("");try{let M=await o(L);if(!M?.success){g(M?.message||t("skills.removeFailed"));return}b(M.message||t("skills.removed",{name:L}))}catch(M){g(M.message||t("skills.removeFailed"))}}},[o,t]),N=p.default.useCallback(async(L,M)=>{if(!M.trim())return g(t("skills.contentRequired")),b(""),{success:!1,message:t("skills.contentRequired")};g(""),b("");try{let P=await l({name:L,content:M});return P?.success?(b(P.message||t("skills.updated",{name:L})),P):(g(P?.message||t("skills.updateFailed")),P)}catch(P){let k=P.message||t("skills.updateFailed");return g(k),{success:!1,message:k}}},[t,l]),C=p.default.useCallback(async(L,M)=>{g(""),b("");try{let P=await c({name:L,enabled:M});if(!P?.success){g(P?.message||t("skills.updateFailed"));return}b(P.message)}catch(P){g(P.message||t("skills.updateFailed"))}},[c,t]),_=p.default.useCallback(async L=>{g(""),b("");try{let M=await d(L);if(!M?.success){g(M?.message||t("skills.updateFailed"));return}b(M.message)}catch(M){g(M.message||t("skills.updateFailed"))}},[d,t]),A;if(n.isLoading)A=u`
      <${ee} padding="md">
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
    `;else if(n.error)A=u`
      <${ee} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">${t("skills.failedLoad",{message:n.error.message})}</p>
        <//>
    `;else{let L=a.filter(P=>et(e,[P.name,P.id,P.description,P.keywords,P.trust_level,P.source_kind,P.version])),M=UM(L);a.length===0?A=u`
        <${ee} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:L.length===0?A=u`<${St} query=${e} />`:A=u`
        <div id="skills-list">
          ${M.map(P=>u`
              <${PM}
                key=${P.id}
                title=${t(P.labelKey)}
                skills=${P.skills}
                onEdit=${s}
                onRemove=${w}
                onUpdate=${N}
                onSetAutoActivate=${C}
                isRemoving=${f}
                isUpdating=${h}
                isSettingAutoActivate=${x}
              />
            `)}
        </div>
      `}return u`
    <div className="space-y-4">
      <${LM}
        enabled=${r}
        isSaving=${y}
        onToggle=${_}
      />
      <${Ek} onInstall=${i} isInstalling=${m} />
      <${jM} error=${$} result=${v} />
      ${A}
    </div>
  `}function LM({enabled:e,isSaving:t,onToggle:a}){let n=R();return u`
    <${ee} padding="md" style=${e?void 0:{background:"var(--v2-danger-soft)"}}>
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
          <${D}
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
  `}function PM({title:e,skills:t,onEdit:a,onRemove:n,onUpdate:r,onSetAutoActivate:s,isRemoving:i,isUpdating:o,isSettingAutoActivate:l}){return t.length===0?null:u`
    <${ee} padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${e}
      </h3>
      ${t.map(c=>u`
          <${Ck}
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
  `}function UM(e){let t=[{id:"user",labelKey:"skills.group.user",skills:[]},{id:"system",labelKey:"skills.group.system",skills:[]},{id:"workspace",labelKey:"skills.group.workspace",skills:[]}],a=t[0];for(let n of e){let r=n.source_kind||"";(r==="system"?t[1]:r==="workspace"?t[2]:a).skills.push(n)}return t.filter(n=>n.skills.length>0)}function jM({error:e,result:t}){return!e&&!t?null:u`
    <div
      className=${e?"rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200":"rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200"}
    >
      ${e||t}
    </div>
  `}function $d(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function Ak(){let e=J(),t=K({queryKey:["settings-tools"],queryFn:Y$}),a=t.data?.tools||[],[n,r]=p.default.useState({}),s=Q({mutationFn:async({name:o,state:l})=>$d(await J$(o,l),"Save failed"),onSuccess:(o,{name:l,state:c})=>{e.setQueryData(["settings-tools"],d=>{if(!d)return d;let m=o?.tool;return{...d,tools:d.tools.map(f=>f.name===l?{...f,state:c,...m||{}}:f)}}),r(d=>({...d,[l]:!0})),setTimeout(()=>r(d=>({...d,[l]:!1})),2e3)}}),i=p.default.useCallback((o,l)=>s.mutate({name:o,state:l}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}var Yh="agent.auto_approve_tools";function FM({visible:e}){let t=R();return e?u`
    <span className="font-mono text-[11px] text-[var(--v2-accent-text)]" role="status">
      ${t("tools.saved")}
    </span>
  `:null}function BM({checked:e,disabled:t=!1,label:a,onChange:n}){return u`
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
  `}function Jh({settings:e,onSave:t,savedKeys:a,isLoading:n}){let r=R(),s=r("settings.field.autoApproveEligibleTools"),i=e?.[Yh],o=i==null?!0:i===!0||i==="true";return u`
    <${ee} padding="md" className="flex items-center justify-between gap-6">
      <div className="min-w-0">
        <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
          ${s}
        </h3>
        <p className="mt-1 text-sm text-[var(--v2-text-muted)]">
          ${r("settings.field.autoApproveEligibleToolsDesc")}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-3">
        <${FM} visible=${a?.[Yh]} />
        <${BM}
          checked=${o}
          disabled=${n}
          label=${s}
          onChange=${l=>t(Yh,l)}
        />
      </div>
    <//>
  `}function zM({tool:e,onPermissionChange:t,isSaved:a}){let n=R(),r=[{value:"default",label:n("tools.followDefault"),tone:"neutral"},{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask_each_time",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],s={default:n("tools.sourceDefault"),global:n("tools.sourceGlobal"),override:n("tools.sourceOverride")},i=e.locked,o=r.find(m=>m.value===e.state)||r[1],l=e.effective_source||"default",c=l==="override"?e.state:"default",d=l==="default"&&e.state===e.default_state;return u`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="flex min-w-0 items-center gap-3">
        ${i&&u`<${O}
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
        ${i?u`<${q} tone=${o.tone} label=${o.label} size="sm" />`:u`
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
  `}function Dk({settings:e={},onSave:t=()=>{},savedKeys:a={},isLoading:n=!1,searchQuery:r=""}){let s=R(),{tools:i,query:o,setPermission:l,savedTools:c}=Ak();if(o.isLoading)return u`
      <div className="space-y-4">
        <${Jh}
          settings=${e}
          onSave=${t}
          savedKeys=${a}
          isLoading=${n}
        />
        <${ee} padding="md">
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
        <${ee} padding="md">
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

      <${ee} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${s("tools.permissions")}
        </h3>
        ${d.length===0?u`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${s("tools.noMatch")}
            </p>`:d.map(m=>u`
                  <${zM}
                    key=${m.name}
                    tool=${m}
                    onPermissionChange=${l}
                    isSaved=${c[m.name]}
                  />
                `)}
      <//>
    </div>
  `}function Mk(e){return(Number(e)||0).toFixed(2)}function qM(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function Ok(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function Vr({label:e,value:t,description:a}){return u`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&u`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function Lk({searchQuery:e=""}){let t=R(),{credits:a,query:n,authorize:r}=Dc();if(!et(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return u`<${St} query=${e} />`;let s;if(n.isLoading)s=u`
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
        <${Vr}
          label=${t("traceCommons.enrollment")}
          value=${a.enrolled?t("traceCommons.enrolled"):t("traceCommons.notEnrolled")}
        />
        <${Vr}
          label=${t("traceCommons.pendingCredit")}
          description=${t("traceCommons.pendingCreditDesc")}
          value=${Mk(a.pending_credit)}
        />
        <${Vr}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${Mk(a.final_credit)}
        />
        <${Vr}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${qM(a.delayed_credit_delta)}
        />
        <${Vr}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${Vr}
          label=${t("traceCommons.lastSubmission")}
          value=${Ok(a.last_submission_at,t)}
        />
        <${Vr}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${Ok(a.last_credit_sync_at,t)}
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
    <${ee} padding="md">
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
  `}function Pk(){let e=J(),t=K({queryKey:["admin-users"],queryFn:lw,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=Q({mutationFn:uw,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=Q({mutationFn:({id:i,payload:o})=>cw(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function IM({onCreate:e,isCreating:t,error:a}){let n=R(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState("member"),[d,m]=p.default.useState(!1),f=h=>{h.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:l},{onSuccess:()=>{s(""),o(""),m(!1)}})};return d?u`
    <${ee} padding="md">
      <h3
        className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${n("users.newUser")}
      </h3>
      <form onSubmit=${f} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <${$n} label=${n("users.displayName")} htmlFor="user-name">
            <${Et}
              id="user-name"
              type="text"
              value=${r}
              onChange=${h=>s(h.target.value)}
              required
            />
          <//>
          <${$n} label=${n("users.email")} htmlFor="user-email">
            <${Et}
              id="user-email"
              type="email"
              value=${i}
              onChange=${h=>o(h.target.value)}
            />
          <//>
        </div>
        <${$n} label=${n("users.role")} htmlFor="user-role">
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
          <${D} type="submit" disabled=${t}>
            ${n(t?"users.creating":"users.createUser")}
          <//>
          <${D}
            variant="ghost"
            type="button"
            onClick=${()=>m(!1)}
            >${n("users.cancel")}<//
          >
        </div>
      </form>
    <//>
  `:u`
      <${D} variant="secondary" onClick=${()=>m(!0)}>
        <${O} name="plus" className="mr-2 h-4 w-4" />
        ${n("users.addUser")}
      <//>
    `}function KM({user:e}){let t=R(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return u`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]"
            >${e.display_name||e.id}</span
          >
          <${q}
            tone=${n}
            label=${e.role==="admin"?t("users.admin"):t("users.member")}
            size="sm"
          />
          <${q} tone=${a} label=${e.status||"active"} size="sm" />
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
  `}function Uk({searchQuery:e=""}){let t=R(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=Pk();if(n.isLoading)return u`
      <${ee} padding="md">
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
      <${ee} padding="lg">
        <div className="flex items-center gap-3">
          <${O} name="lock" className="h-5 w-5 text-[var(--v2-text-faint)]" />
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">
            ${t("users.adminRequired")}
          </h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
          ${t("users.adminRequiredDesc")}
        </p>
      <//>
    `;if(n.error)return u`
      <${ee} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("users.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let l=a.filter(c=>et(e,[c.id,c.display_name,c.email,c.role,c.status,c.last_active]));return u`
    <div className="space-y-5">
      <${IM}
        onCreate=${s}
        isCreating=${o}
        error=${i}
      />

      <${ee} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("users.title",{count:l.length})}
        </h3>
        ${a.length===0?u`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("users.noUsers")}
            </p>`:l.length===0?u`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("settings.noMatchingSettings",{query:e})}
            </p>`:l.map(c=>u`<${KM} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function jk(){let e=J(),t=K({queryKey:["settings-export"],queryFn:B$,staleTime:3e4}),a=t.data?.settings||{},[n,r]=p.default.useState({}),[s,i]=p.default.useState(!1),o=Q({mutationFn:async({key:m,value:f})=>$d(await Jp(m,f),"Save failed"),onSuccess:(m,{key:f,value:h})=>{e.setQueryData(["settings-export"],x=>{if(!x)return x;let y={...x,settings:{...x.settings}};return h==null?delete y.settings[f]:y.settings[f]=h,y}),r(x=>({...x,[f]:!0})),setTimeout(()=>r(x=>({...x,[f]:!1})),2e3),Vh.has(f)&&i(!0),f==="agent.auto_approve_tools"&&e.invalidateQueries({queryKey:["settings-tools"]})}}),l=p.default.useCallback((m,f)=>o.mutate({key:m,value:f}),[o]),c=Q({mutationFn:z$,onSuccess:(m,f)=>{e.invalidateQueries({queryKey:["settings-export"]});let h=Object.keys(f?.settings||{});h.includes("agent.auto_approve_tools")&&e.invalidateQueries({queryKey:["settings-tools"]}),h.some(x=>Vh.has(x))&&i(!0)}}),d=p.default.useCallback(m=>c.mutateAsync(m),[c]);return{settings:a,query:t,save:l,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error||c.error}}function Xh(){let e=R(),{tab:t}=st(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=ba(),s=r?"inference":"language",i=t||s,{settings:o,query:l,save:c,savedKeys:d,needsRestart:m,saveError:f}=jk(),[h,x]=p.default.useState("");p.default.useEffect(()=>{x("")},[i]);let y=l.isLoading,$={inference:u`<${wk}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,agent:u`<${vk}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,channels:u`<${bk} searchQuery=${h} />`,networking:u`<${Nk}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,tools:u`<${Dk}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,skills:u`<${Tk} searchQuery=${h} />`,traces:u`<${Lk} searchQuery=${h} />`,users:u`<${Uk} searchQuery=${h} />`,language:u`<${Sk} searchQuery=${h} />`},g=C=>C==="users"||C==="inference",v=C=>Object.prototype.hasOwnProperty.call($,C),b=Object.keys($).filter(C=>r||!g(C)),N=v(s)&&b.includes(s)?s:b[0]||"language";return!v(i)||!r&&g(i)?u`<${it} to=${`/settings/${N}`} replace />`:u`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${m&&u`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${kk}
                visible=${!0}
                gatewayStatus=${a}
                gatewayStatusQuery=${n}
              />
            </div>`}

            ${f&&u`
              <div
                className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
              >
                ${e("error.saveFailed",{message:f.message})}
              </div>
            `}

            ${$[i]}
          </div>
        </div>
      </div>
    </div>
  `}var Zh=Object.freeze({todo:!0});function Fk(){return Promise.resolve({users:[],total:0,...Zh})}function Bk(e){return Promise.resolve(null)}function zk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function qk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Ik(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Kk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Hk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Qk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Vk(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...Zh})}function Gk(e="day",t){return Promise.resolve({entries:[],...Zh})}function Yk(){return K({queryKey:["admin","usage-summary"],queryFn:Vk,refetchInterval:3e4})}function wd(e="day",t){return K({queryKey:["admin","usage",e,t],queryFn:()=>Gk(e,t),refetchInterval:3e4})}function Ri(){let e=J(),t=K({queryKey:["admin","users"],queryFn:Fk,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=Q({mutationFn:zk,onSuccess:s}),o=Q({mutationFn:({id:f,payload:h})=>qk(f,h),onSuccess:s}),l=Q({mutationFn:f=>Ik(f),onSuccess:s}),c=Q({mutationFn:f=>Kk(f),onSuccess:s}),d=Q({mutationFn:f=>Hk(f),onSuccess:s}),m=Q({mutationFn:({userId:f,name:h})=>Qk(f,h)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(f,h)=>o.mutateAsync({id:f,payload:h}),deleteUser:l.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(f,h)=>m.mutateAsync({userId:f,name:h}),newToken:m.data,clearToken:()=>m.reset()}}function Jk(e){return K({queryKey:["admin","user",e],queryFn:()=>Bk(e),enabled:!!e,refetchInterval:1e4})}function Ya(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function Ca(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function Xk(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function or(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function Ci(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function Ei(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function Ti(e){return e==="admin"?"signal":"muted"}function Zk(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function Wk(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function eR(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function tR(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function aR(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function HM({users:e,onSelectUser:t}){let a=R(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?u`
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
                <td className="py-3 pr-4"><${q} tone=${Ti(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${q} tone=${Ei(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${or(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:u`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function nR({onSelectUser:e,onNavigateTab:t}){let a=R(),n=Yk(),{users:r,query:s}=Ri(),i=n.data||{},o=Zk(r),l=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?u`
      <div className="space-y-5">
        <${I} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            ${[1,2,3,4].map(m=>u`<div key=${m} className="v2-skeleton h-28 rounded-lg" />`)}
          </div>
        <//>
      </div>
    `:u`
    <div className="space-y-5">
      <${I} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.systemOverview")}</h3>
          ${i.uptime_seconds!=null&&u`
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:Xk(i.uptime_seconds)})}</span>
          `}
        </div>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <${We}
            label=${a("admin.dashboard.totalUsers")}
            value=${String(o.total)}
            tone=${o.total>0?"success":"muted"}
          />
          <${We}
            label=${a("admin.dashboard.activeUsers")}
            value=${String(o.active)}
            tone="success"
          />
          <${We}
            label=${a("admin.dashboard.suspended")}
            value=${String(o.suspended)}
            tone=${o.suspended>0?"danger":"muted"}
          />
          <${We}
            label=${a("admin.dashboard.admins")}
            value=${String(o.admins)}
            tone="signal"
          />
        </div>
      <//>

      <${I} className="p-5 sm:p-6">
        <h3 className="mb-5 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.usage30d")}</h3>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <${We}
            label=${a("admin.dashboard.totalJobs")}
            value=${String(c.total||0)}
            tone="muted"
          />
          <${We}
            label=${a("admin.dashboard.llmCalls")}
            value=${String(l.llm_calls||0)}
            tone="muted"
          />
          <${We}
            label=${a("admin.dashboard.totalCost")}
            value=${Ca(l.total_cost)}
            tone="signal"
          />
          <${We}
            label=${a("admin.dashboard.activeJobs")}
            value=${String(c.in_progress||0)}
            tone=${(c.in_progress||0)>0?"success":"muted"}
          />
        </div>
      <//>

      <${I} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.recentUsers")}</h3>
          <button
            onClick=${()=>t("users")}
            className="text-xs text-signal hover:underline"
          >
            ${a("admin.dashboard.viewAll")}
          </button>
        </div>
        <${HM} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var QM=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function VM({value:e,max:t}){let a=t>0?e/t*100:0;return u`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function rR({onSelectUser:e}){let t=R(),[a,n]=p.default.useState("day"),r=wd(a),s=r.data?.usage||[],i=eR(s),o=tR(s),l=aR(i),c=i.length>0?i[0].cost:0;return r.isLoading?u`
      <${I} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
        <div className="grid gap-4 sm:grid-cols-4">
          ${[1,2,3,4].map(d=>u`<div key=${d} className="v2-skeleton h-28 rounded-lg" />`)}
        </div>
      <//>
    `:u`
    <div className="space-y-5">
      <${I} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.overview")}</h3>
          <div className="flex gap-1">
            ${QM.map(d=>u`
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
                <${We} label=${t("admin.usage.totalCalls")} value=${l.calls.toLocaleString()} tone="muted" />
                <${We} label=${t("admin.usage.inputTokens")} value=${Ya(l.input_tokens)} tone="muted" />
                <${We} label=${t("admin.usage.outputTokens")} value=${Ya(l.output_tokens)} tone="muted" />
                <${We} label=${t("admin.usage.totalCost")} value=${Ca(l.cost.toFixed(2))} tone="signal" />
              </div>
            `}
      <//>

      ${i.length>0&&u`
        <${I} className="p-5 sm:p-6">
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
                          ${Ci(d.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ya(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ya(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${Ca(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${VM} value=${d.cost} max=${c} />
                      </td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}

      ${o.length>0&&u`
        <${I} className="p-5 sm:p-6">
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
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ya(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ya(d.output_tokens)}</td>
                      <td className="py-3 font-mono text-xs text-iron-100">${Ca(d.cost.toFixed(2))}</td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}
    </div>
  `}function lr({label:e,children:t}){return u`
    <div className="flex items-start justify-between gap-4 border-t border-white/[0.06] py-3 first:border-0 first:pt-0">
      <span className="text-xs text-iron-300">${e}</span>
      <span className="text-right text-sm text-iron-100">${t}</span>
    </div>
  `}function sR({userId:e,onBack:t}){let a=R(),n=Jk(e),r=wd("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:l,createToken:c,newToken:d,clearToken:m}=Ri(),[f,h]=p.default.useState(null),[x,y]=p.default.useState(!1),$=n.data,g=r.data?.usage||[];if(p.default.useEffect(()=>{$&&f===null&&h($.role)},[$]),n.isLoading)return u`
      <div className="space-y-5">
        <${I} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-2 h-6 w-48 rounded" />
          <div className="v2-skeleton h-4 w-32 rounded" />
        <//>
      </div>
    `;if(n.error)return u`
      <${I} className="p-5 sm:p-6">
        <p className="text-sm text-red-200">${a("error.loadFailed",{what:a("admin.users.user"),message:n.error.message})}</p>
      <//>
    `;if(!$)return null;let v=async()=>{f&&f!==$.role&&await o($.id,{role:f})},b=async()=>{await l($.id),t()},w=async()=>{let N=window.prompt(a("admin.users.tokenNamePrompt",{name:$.display_name||a("admin.users.userFallback")}));N&&await c($.id,N)};return u`
    <div className="space-y-5">
      <button
        onClick=${t}
        className="flex items-center gap-1.5 text-xs text-iron-300 hover:text-white"
      >
        <span>←</span>
        <span>${a("admin.users.backToUsers")}</span>
      </button>

      <${I} className="p-5 sm:p-6">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <h2 className="text-2xl font-semibold tracking-tight text-white">${$.display_name||$.id}</h2>
            <div className="mt-2 flex items-center gap-2">
              <${q} tone=${Ti($.role)} label=${$.role||"member"} />
              <${q} tone=${Ei($.status)} label=${$.status||"active"} />
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            ${$.status==="active"?u`<${D} variant="secondary" onClick=${()=>s($.id)}>${a("admin.users.suspend")}<//>`:u`<${D} variant="secondary" onClick=${()=>i($.id)}>${a("admin.users.activate")}<//>`}
            <${D} variant="secondary" onClick=${w}>${a("admin.users.createToken")}<//>
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
              <${O} name="close" className="h-4 w-4" />
            </button>
          </div>
        </div>
      `}

      <div className="grid gap-5 lg:grid-cols-2">
        <${I} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.profile")}</h3>
          <${lr} label=${a("admin.user.id")}>
            <span className="font-mono text-xs">${$.id}</span>
          <//>
          <${lr} label=${a("admin.user.email")}>${$.email||a("admin.user.notSet")}<//>
          <${lr} label=${a("admin.user.created")}>${or($.created_at)}<//>
          <${lr} label=${a("admin.user.lastLogin")}>${or($.last_login_at)}<//>
          ${$.created_by&&u`
            <${lr} label=${a("admin.user.createdBy")}>
              <span className="font-mono text-xs">${Ci($.created_by)}</span>
            <//>
          `}
        <//>

        <${I} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${lr} label=${a("admin.user.jobs")}>${$.job_count??0}<//>
          <${lr} label=${a("admin.user.totalCost")}>${Ca($.total_cost)}<//>
          <${lr} label=${a("admin.user.lastActive")}>${or($.last_active_at)}<//>
        <//>
      </div>

      <${I} className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.roleManagement")}</h3>
        <div className="flex items-end gap-3">
          <div>
            <label className="mb-1 block text-xs text-iron-300">${a("admin.user.currentRole")}</label>
            <select
              value=${f||$.role}
              onChange=${N=>h(N.target.value)}
              className="v2-select h-9 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            >
              <option value="member">${a("admin.users.member")}</option>
              <option value="admin">${a("admin.users.admin")}</option>
            </select>
          </div>
          <${D} onClick=${v} disabled=${!f||f===$.role}>
            ${a("admin.user.saveRole")}
          <//>
        </div>
      <//>

      <${I} className="p-5 sm:p-6">
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
                    ${g.map((N,C)=>u`
                        <tr key=${C} className="border-b border-white/[0.06] last:border-0">
                          <td className="py-3 pr-4 font-mono text-xs text-iron-100">${N.model}</td>
                          <td className="py-3 pr-4 font-mono text-xs text-iron-300">${(N.call_count||0).toLocaleString()}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ya(N.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ya(N.output_tokens)}</td>
                          <td className="py-3 font-mono text-xs text-iron-100">${Ca(N.total_cost)}</td>
                        </tr>
                      `)}
                  </tbody>
                </table>
              </div>
            `}
      <//>

      ${x&&u`
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick=${()=>y(!1)}>
          <div className="w-full max-w-md rounded-xl border border-white/10 bg-iron-900 p-6" onClick=${N=>N.stopPropagation()}>
            <h3 className="text-lg font-semibold text-white">${a("admin.users.deleteUserTitle")}</h3>
            <p className="mt-2 text-sm text-iron-300">
              ${a("admin.users.deleteUserDesc",{name:$.display_name})}
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <${D} variant="ghost" onClick=${()=>y(!1)}>${a("admin.users.cancel")}<//>
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
  `}function GM(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function YM({token:e,onDismiss:t}){let a=R(),[n,r]=p.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return u`
    <div className="rounded-xl border border-signal/30 bg-signal/10 p-4 sm:p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-iron-100">${a("admin.users.tokenCreated")}</p>
          <p className="mt-1 text-xs text-iron-300">${a("admin.users.tokenCreatedDesc")}</p>
          <div className="mt-3 flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-md border border-iron-700 bg-iron-800/70 px-3 py-2 font-mono text-xs text-iron-100">
              ${e}
            </code>
            <${D} variant="secondary" onClick=${s}>
              ${a(n?"admin.users.copied":"admin.users.copy")}
            <//>
          </div>
        </div>
        <button onClick=${t} className="text-iron-300 hover:text-iron-100">
          <${O} name="close" className="h-4 w-4" />
        </button>
      </div>
    </div>
  `}function JM({onCreate:e,isCreating:t,error:a}){let n=R(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState("member"),[d,m]=p.default.useState(!1),f=async h=>{h.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:l}),s(""),o(""),m(!1))};return d?u`
    <${I} className="p-5 sm:p-6">
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
          <${D} type="submit" disabled=${t}>
            ${n(t?"admin.users.creating":"admin.users.createUser")}
          <//>
          <${D} variant="ghost" type="button" onClick=${()=>m(!1)}>${n("admin.users.cancel")}<//>
        </div>
      </form>
    <//>
  `:u`
      <${D} variant="secondary" onClick=${()=>m(!0)}>
        <${O} name="plus" className="mr-2 h-4 w-4" />
        ${n("admin.users.newUser")}
      <//>
    `}function XM({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=R();return u`
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick=${r}>
      <div className="w-full max-w-md rounded-xl border border-iron-700 bg-iron-900 p-6" onClick=${i=>i.stopPropagation()}>
        <h3 className="text-lg font-semibold text-iron-100">${e}</h3>
        <p className="mt-2 text-sm text-iron-300">${t}</p>
        <div className="mt-5 flex justify-end gap-2">
          <${D} variant="ghost" onClick=${r}>${s("admin.users.cancel")}<//>
          <button
            onClick=${n}
            className="v2-button inline-flex h-10 items-center justify-center rounded-md bg-[var(--v2-danger-soft)] px-4 text-sm font-semibold text-[var(--v2-danger-text)] hover:bg-[color-mix(in_srgb,var(--v2-danger-soft)_65%,var(--v2-danger-text))]"
          >
            ${a}
          </button>
        </div>
      </div>
    </div>
  `}function ZM({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=R();return u`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${q} tone=${Ti(e.role)} label=${e.role||"member"} />
          <${q} tone=${Ei(e.status)} label=${e.status||"active"} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          ${e.email&&u`<span className="font-mono text-xs text-iron-300">${e.email}</span>`}
          <span className="font-mono text-xs text-iron-700">${Ci(e.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-iron-300 sm:inline">
          ${e.job_count!=null?i("admin.users.jobsCount",{count:e.job_count}):""}
          ${e.total_cost!=null?` \xB7 ${Ca(e.total_cost)}`:""}
        </span>
        <span className="hidden text-xs text-iron-700 lg:inline">${or(e.last_active_at)}</span>
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
  `}function iR({selectedUserId:e,onSelectUser:t}){let a=R(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:l,updateUser:c,deleteUser:d,suspendUser:m,activateUser:f,createToken:h,newToken:x,clearToken:y}=Ri(),[$,g]=p.default.useState(""),[v,b]=p.default.useState("all"),[w,N]=p.default.useState(null),C=Wk(n,{search:$,filter:v}),_=GM(a),A=M=>{N({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{m(M),N(null)}})},L=async(M,P)=>{let k=window.prompt(a("admin.users.tokenNamePrompt",{name:P||a("admin.users.userFallback")}));k&&await h(M,k)};return r.isLoading?u`
      <${I} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-3 w-24 rounded" />
        ${[1,2,3].map(M=>u`
          <div key=${M} className="flex items-center justify-between border-t border-iron-700 py-3.5 first:border-0">
            <div className="v2-skeleton h-4 w-32 rounded" />
            <div className="v2-skeleton h-6 w-20 rounded-full" />
          </div>
        `)}
      <//>
    `:s?u`
      <${I} className="p-6 sm:p-8">
        <div className="flex items-center gap-3">
          <${O} name="lock" className="h-5 w-5 text-iron-700" />
          <h3 className="text-lg font-semibold text-iron-100">${a("users.adminRequired")}</h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${a("users.adminRequiredDesc")}
        </p>
      <//>
    `:u`
    <div className="space-y-5">
      ${x&&u`
        <${YM}
          token=${x.token||x.plaintext_token}
          onDismiss=${y}
        />
      `}

      <${JM} onCreate=${i} isCreating=${o} error=${l} />

      <${I} className="p-5 sm:p-6">
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${a("admin.users.title",{count:C.length,total:n.length})}
          </h3>
          <div className="flex items-center gap-2">
            <input
              type="text"
              placeholder=${a("admin.users.searchPlaceholder")}
              value=${$}
              onChange=${M=>g(M.target.value)}
              className="h-8 w-48 rounded-md border border-iron-700 bg-iron-800/70 px-3 text-xs text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
            />
            <div className="flex gap-1">
              ${_.map(M=>u`
                  <button
                    key=${M.value}
                    onClick=${()=>b(M.value)}
                    className=${["rounded-md px-2.5 py-1.5 text-[11px] font-medium",v===M.value?"border border-signal/35 bg-signal/10 text-iron-100":"border border-transparent text-iron-300 hover:text-iron-100"].join(" ")}
                  >
                    ${M.label}
                  </button>
                `)}
            </div>
          </div>
        </div>

        ${C.length===0?u`<p className="py-4 text-sm text-iron-300">${a("admin.users.noMatch")}</p>`:C.map(M=>u`
                <${ZM}
                  key=${M.id}
                  user=${M}
                  onSelect=${t}
                  onSuspend=${A}
                  onActivate=${f}
                  onChangeRole=${(P,k)=>c(P,{role:k})}
                  onCreateToken=${L}
                />
              `)}
      <//>

      ${w&&u`
        <${XM}
          title=${w.title}
          message=${w.message}
          confirmLabel=${w.confirmLabel}
          onConfirm=${w.onConfirm}
          onCancel=${()=>N(null)}
        />
      `}
    </div>
  `}function oR(){let{tab:e="dashboard"}=st(),t=me(),[a,n]=p.default.useState(null),r=p.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=p.default.useCallback(()=>{n(null)},[]),i={dashboard:u`<${nR}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?u`<${sR} userId=${a} onBack=${s} />`:u`<${iR}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:u`<${rR} onSelectUser=${r} />`};return i[e]?u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:u`<${it} to="/admin/dashboard" replace />`}var WM=2e3,eO=500,tO=2e3,aO=new Set([403,404]),nO=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function rO(e=globalThis.location,t=null){let a=new URLSearchParams(e?.search||""),n={active:[]};for(let[r,s,i]of nO){let o=a.get(s)?.trim();o?(n[r]=o,n.active.push({key:r,param:s,labelKey:i,value:o})):n[r]=null}return!n.threadId&&t&&(n.threadId=t),n}function lR({isAdmin:e=!1,defaultThreadId:t=null}={}){let a=Le(),n=a?.search||"",r=p.default.useMemo(()=>rO(a,t),[t,n]),{runId:s,source:i,threadId:o,toolCallId:l,toolName:c,turnId:d}=r,[m,f]=p.default.useState([]),[h,x]=p.default.useState("all"),[y,$]=p.default.useState(""),[g,v]=p.default.useState(!1),[b,w]=p.default.useState(!0),[N,C]=p.default.useState(!0),[_,A]=p.default.useState(null),L=p.default.useRef(new Set),M=p.default.useRef(0),P=!e&&!o;p.default.useEffect(()=>{M.current+=1,f([]),A(null)},[e,s,i,o,l,c,d]);let k=p.default.useCallback(async()=>{if(P){C(!1);return}let ne=++M.current;C(!0);try{let de={limit:eO,level:h==="all"?null:h,target:y.trim()||null,threadId:o,runId:s,turnId:d,toolCallId:l,toolName:c,source:i},fe;try{fe=await(e?Yx(de):Pp(de))}catch(mt){if(!e||!aO.has(mt?.status))throw mt;fe=await Pp(de)}if(ne!==M.current)return;let Ce=L.current,gt=YN(fe).entries.filter(mt=>!Ce.has(mt.id));f(gt),A(null)}catch(de){if(ne!==M.current)return;A(de)}finally{ne===M.current&&C(!1)}},[e,h,P,s,i,y,o,l,c,d]);p.default.useEffect(()=>{k()},[k]),p.default.useEffect(()=>{if(g||P)return;let ne=setInterval(k,WM);return()=>clearInterval(ne)},[k,P,g]);let z=p.default.useCallback(()=>{v(ne=>!ne)},[]),Z=p.default.useCallback(()=>{let ne=[...L.current,...m.map(de=>de.id)].slice(-tO);L.current=new Set(ne),f([])},[m]);return{entries:m,totalCount:m.length,paused:g,togglePause:z,clearEntries:Z,levelFilter:h,setLevelFilter:x,targetFilter:y,setTargetFilter:$,autoScroll:b,setAutoScroll:w,serverLevel:null,changeServerLevel:async()=>{},scope:r,needsThreadScope:P,status:P?"needs_scope":_?"error":N?"loading":"ready",isLoading:N,error:_}}var sO=["all","trace","debug","info","warn","error"],iO=["trace","debug","info","warn","error"],uR={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},oO={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function lO({entry:e}){let t=R(),[a,n]=p.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=uR[e.level]||uR.info,i=oO[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(l=>!!l.value);return u`
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
  `}function cR({value:e,onChange:t,options:a,labelKey:n,t:r}){return u`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>u`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function uO({label:e,value:t,scopeKey:a}){return u`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function dR(){let e=R(),{isAdmin:t=!1,threadsState:a}=ba()||{},{entries:n,totalCount:r,paused:s,togglePause:i,clearEntries:o,levelFilter:l,setLevelFilter:c,targetFilter:d,setTargetFilter:m,autoScroll:f,setAutoScroll:h,serverLevel:x,changeServerLevel:y,scope:$,isLoading:g,error:v,needsThreadScope:b}=lR({isAdmin:t,defaultThreadId:t?null:a?.activeThreadId||null}),w=p.default.useRef(null),N=p.default.useRef(!0);p.default.useEffect(()=>{f&&N.current&&w.current&&(w.current.scrollTop=0)},[n,f]);let C=p.default.useCallback(L=>{N.current=L.currentTarget.scrollTop<=48},[]),_=n.length>0,A=$?.active||[];return u`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${cR}
          value=${l}
          onChange=${c}
          options=${sO}
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

        ${A.length>0&&u`
          <div
            data-testid="logs-scope-toolbar"
            className="flex w-full flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]"
          >
            <span className="font-medium text-[var(--v2-text-strong)]">${e("logs.scoped")}</span>
            ${A.map(L=>u`<${uO} key=${L.param} scopeKey=${L.param} label=${e(L.labelKey)} value=${L.value} />`)}
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
            <${cR}
              value=${x}
              onChange=${y}
              options=${iO}
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
        onScroll=${C}
        className="min-h-0 flex-1 overflow-y-auto bg-[var(--v2-canvas)]"
      >
        ${v&&_?u`
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
            `:v&&!_?u`
              <div
                className="flex h-full items-center justify-center px-6 text-center text-sm text-red-300"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:v.message||v.statusText||"Request failed"})}
              </div>
            `:g&&!_?u`
                <div
                  className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
                >
                  ${e("common.loading")}
                </div>
              `:_?n.map(L=>u`<${lO} key=${L.id} entry=${L} />`):u`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function fR(){return u`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function cO({auth:e}){let t=me(),n=Le().state?.from,r=n?`${n.pathname||Fr}${n.search||""}${n.hash||""}`:Fr,s=`/v2${r==="/"?"":r}`,i=p.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?u`<${fR} />`:e.isAuthenticated?u`<${it} to=${r} replace />`:u`<${k1}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function dO({auth:e,children:t}){let a=Le();return e.isChecking?u`<${fR} />`:e.isAuthenticated?t:u`<${it} to="/login" replace state=${{from:a}} />`}function mO({auth:e}){return u`
    <${dO} auth=${e}>
      <${t1}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        rebornProjectsEnabled=${e.rebornProjectsEnabled}
        onSignOut=${e.signOut}
      />
    <//>
  `}function mR({auth:e}){return e.isAdmin?u`<${oR} />`:u`<${it} to=${Fr} replace />`}function pR(){let e=O$();return u`
    <${Dp} basename="/v2">
      <${Ep}>
        <${ge} path="/login" element=${u`<${cO} auth=${e} />`} />
        <${ge} path="/" element=${u`<${mO} auth=${e} />`}>
          <${ge} index element=${u`<${it} to=${Fr} replace />`} />
          <${ge} path="overview" element=${u`<${it} to=${Fr} replace />`} />
          <${ge} path="welcome" element=${u`<${Z2} />`} />
          <${ge} path="chat" element=${u`<${_h} />`} />
          <${ge} path="chat/:threadId" element=${u`<${_h} />`} />
          <${ge} path="workspace" element=${u`<${Rh} />`} />
          <${ge} path="workspace/*" element=${u`<${Rh} />`} />
          <${ge} path="projects" element=${u`<${hl} />`} />
          <${ge} path="projects/:projectId" element=${u`<${hl} />`} />
          <${ge} path="projects/:projectId/missions/:missionId" element=${u`<${hl} />`} />
          <${ge} path="projects/:projectId/threads/:threadId" element=${u`<${hl} />`} />
          <${ge} path="missions" element=${u`<${Eh} />`} />
          <${ge} path="missions/:missionId" element=${u`<${Eh} />`} />
          <${ge} path="jobs" element=${u`<${Dh} />`} />
          <${ge} path="jobs/:jobId" element=${u`<${Dh} />`} />
          <${ge} path="routines" element=${u`<${Oh} />`} />
          <${ge} path="routines/:routineId" element=${u`<${Oh} />`} />
          <${ge} path="automations" element=${u`<${i_} />`} />
          <${ge} path="extensions" element=${u`<${Qh} />`} />
          <${ge} path="extensions/:tab" element=${u`<${Qh} />`} />
          <${ge} path="logs" element=${u`<${dR} />`} />
          <${ge} path="settings" element=${u`<${Xh} />`} />
          <${ge} path="settings/:tab" element=${u`<${Xh} />`} />
          <${ge} path="admin" element=${u`<${mR} auth=${e} />`} />
          <${ge} path="admin/:tab" element=${u`<${mR} auth=${e} />`} />
        <//>
        <${ge} path="*" element=${u`<${it} to=${Fr} replace />`} />
      <//>
    <//>
  `}ev("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","approval.viewFullCommand":"View full command","approval.showCommandPreview":"Show preview","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.tabDeclined":"Declined","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitDeclined":"declined","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.serviceUnavailable":"Service unavailable","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveEligibleTools":"Always allow eligible tools","settings.field.autoApproveEligibleToolsDesc":"Applies to tools set to \u201CFollow global\u201D. Per-tool settings override this; hard-floor approval gates still ask.","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.followDefault":"Follow global","tools.sourceDefault":"default permission","tools.sourceGlobal":"global setting","tools.sourceOverride":"per-tool override","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","skills.defaultAutoActivationEnabled":"Default skill auto-activation enabled","skills.defaultAutoActivationDisabled":"Default skill auto-activation disabled","skills.defaultAutoActivationOnDesc":"Skills auto-activate by keyword on matching requests. Turn off to require an explicit /name.","skills.defaultAutoActivationOffDesc":"Skills run only when you type /name. Turn on to let them auto-activate by keyword.","skills.defaultAutoActivationOnButton":"Default: On","skills.defaultAutoActivationOffButton":"Default: Off","skills.autoActivateOnTitle":"Auto-activation on \u2014 runs on matching requests. Click to make it explicit-only (/name).","skills.autoActivateOffTitle":"Explicit-only \u2014 runs only when you type /name. Click to enable auto-activation.","skills.autoActivateOnLabel":"Auto-activate: On","skills.autoActivateOffLabel":"Auto-activate: Off","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Memory, files & attachments","workspace.readOnly":"Read-only","workspace.filterPlaceholder":"Filter by name\u2026","workspace.emptyDir":"This folder is empty.","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.noFiles":"No files here.","workspace.noMatches":"Nothing matches that filter.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a file","workspace.pickFileDesc":"Choose a file from the tree to preview or download it. This viewer is read-only.","workspace.parent":"Parent: {path}","workspace.download":"Download","workspace.binaryPreviewUnavailable":"No inline preview for this file type. Download it to view the contents.","workspace.fileMeta":"{mime} \xB7 {size} bytes","workspace.unableOpenDirectory":"Unable to open directory","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.filter.completed":"Completed","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.schedule.onceAt":"Once on {datetime}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.filterAction":"Show {label}","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.empty.onboardingTitle":"No automations yet","automations.empty.onboardingDescription":"Automations are created by chatting with your agent \u2014 there's no form to fill out. Ask it to do something on a schedule and it will set up a recurring automation for you.","automations.empty.examplesTitle":"Try asking your agent","automations.empty.example1":"Check the nearai/ironclaw repo every 10 minutes and summarize new issues, PRs, and commits.","automations.empty.example2":"Every weekday at 9am, send me a summary of my unread email.","automations.empty.example3":"Remind me to review open pull requests every afternoon at 3pm.","automations.empty.startInChat":"Start in chat","automations.empty.copyPrompt":"Copy prompt","automations.empty.copied":"Copied","automations.refreshing":"Refreshing\u2026","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.runs.total":"Recent runs: {count}","automations.runs.ok":"OK: {count}","automations.runs.error":"Failed: {count}","automations.runs.running":"Running: {count}","automations.runs.unknown":"Unknown: {count}","automations.runs.showingOf":"Showing {shown} of {total} recent runs","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up a project for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open project","projects.openGeneralWorkspace":"Open project","projects.noDescription":"No project description yet. The project is still being shaped by recent activity and thread history.","projects.general.label":"General project","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,hR.createRoot)(document.getElementById("v2-root")).render(u`
  <${tv}>
    <${Ld} client=${Rt}>
      <${pR} />
    <//>
  <//>
`);
