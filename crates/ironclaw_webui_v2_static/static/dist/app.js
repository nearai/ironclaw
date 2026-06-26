import{a as Dn,b as Be,c as He,d as p,e as l,f as Yh,g as Jh,h as vl,i as R,j as gl}from"./chunks/chunk-IGTNS7XG.js";var hv=Dn(kl=>{"use strict";var _R=Symbol.for("react.transitional.element"),kR=Symbol.for("react.fragment");function pv(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:_R,type:e,key:n,ref:t!==void 0?t:null,props:a}}kl.Fragment=kR;kl.jsx=pv;kl.jsxs=pv});var Td=Dn((WL,vv)=>{"use strict";vv.exports=hv()});var Tv=Dn(Me=>{"use strict";function Ud(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<Ll(r,t))e[n]=t,e[a]=r,a=n;else break e}}function Ba(e){return e.length===0?null:e[0]}function Ul(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],u=i+1,c=e[u];if(0>Ll(o,a))u<r&&0>Ll(c,o)?(e[n]=c,e[u]=a,n=u):(e[n]=o,e[i]=a,n=i);else if(u<r&&0>Ll(c,a))e[n]=c,e[u]=a,n=u;else break e}}return t}function Ll(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}Me.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?($v=performance,Me.unstable_now=function(){return $v.now()}):(Od=Date,wv=Od.now(),Me.unstable_now=function(){return Od.now()-wv});var $v,Od,wv,rn=[],Ln=[],TR=1,ua=null,yt=3,jd=!1,Ui=!1,ji=!1,Fd=!1,_v=typeof setTimeout=="function"?setTimeout:null,kv=typeof clearTimeout=="function"?clearTimeout:null,Sv=typeof setImmediate<"u"?setImmediate:null;function Pl(e){for(var t=Ba(Ln);t!==null;){if(t.callback===null)Ul(Ln);else if(t.startTime<=e)Ul(Ln),t.sortIndex=t.expirationTime,Ud(rn,t);else break;t=Ba(Ln)}}function Bd(e){if(ji=!1,Pl(e),!Ui)if(Ba(rn)!==null)Ui=!0,ss||(ss=!0,rs());else{var t=Ba(Ln);t!==null&&zd(Bd,t.startTime-e)}}var ss=!1,Fi=-1,Rv=5,Cv=-1;function Ev(){return Fd?!0:!(Me.unstable_now()-Cv<Rv)}function Ld(){if(Fd=!1,ss){var e=Me.unstable_now();Cv=e;var t=!0;try{e:{Ui=!1,ji&&(ji=!1,kv(Fi),Fi=-1),jd=!0;var a=yt;try{t:{for(Pl(e),ua=Ba(rn);ua!==null&&!(ua.expirationTime>e&&Ev());){var n=ua.callback;if(typeof n=="function"){ua.callback=null,yt=ua.priorityLevel;var r=n(ua.expirationTime<=e);if(e=Me.unstable_now(),typeof r=="function"){ua.callback=r,Pl(e),t=!0;break t}ua===Ba(rn)&&Ul(rn),Pl(e)}else Ul(rn);ua=Ba(rn)}if(ua!==null)t=!0;else{var s=Ba(Ln);s!==null&&zd(Bd,s.startTime-e),t=!1}}break e}finally{ua=null,yt=a,jd=!1}t=void 0}}finally{t?rs():ss=!1}}}var rs;typeof Sv=="function"?rs=function(){Sv(Ld)}:typeof MessageChannel<"u"?(Pd=new MessageChannel,Nv=Pd.port2,Pd.port1.onmessage=Ld,rs=function(){Nv.postMessage(null)}):rs=function(){_v(Ld,0)};var Pd,Nv;function zd(e,t){Fi=_v(function(){e(Me.unstable_now())},t)}Me.unstable_IdlePriority=5;Me.unstable_ImmediatePriority=1;Me.unstable_LowPriority=4;Me.unstable_NormalPriority=3;Me.unstable_Profiling=null;Me.unstable_UserBlockingPriority=2;Me.unstable_cancelCallback=function(e){e.callback=null};Me.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):Rv=0<e?Math.floor(1e3/e):5};Me.unstable_getCurrentPriorityLevel=function(){return yt};Me.unstable_next=function(e){switch(yt){case 1:case 2:case 3:var t=3;break;default:t=yt}var a=yt;yt=t;try{return e()}finally{yt=a}};Me.unstable_requestPaint=function(){Fd=!0};Me.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=yt;yt=e;try{return t()}finally{yt=a}};Me.unstable_scheduleCallback=function(e,t,a){var n=Me.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:TR++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,Ud(Ln,e),Ba(rn)===null&&e===Ba(Ln)&&(ji?(kv(Fi),Fi=-1):ji=!0,zd(Bd,a-n))):(e.sortIndex=r,Ud(rn,e),Ui||jd||(Ui=!0,ss||(ss=!0,rs()))),e};Me.unstable_shouldYield=Ev;Me.unstable_wrapCallback=function(e){var t=yt;return function(){var a=yt;yt=t;try{return e.apply(this,arguments)}finally{yt=a}}}});var Dv=Dn((L6,Av)=>{"use strict";Av.exports=Tv()});var Ov=Dn(kt=>{"use strict";var AR=He();function Mv(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Pn(){}var _t={d:{f:Pn,r:function(){throw Error(Mv(522))},D:Pn,C:Pn,L:Pn,m:Pn,X:Pn,S:Pn,M:Pn},p:0,findDOMNode:null},DR=Symbol.for("react.portal");function MR(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:DR,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var Bi=AR.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function jl(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}kt.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=_t;kt.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(Mv(299));return MR(e,t,null,a)};kt.flushSync=function(e){var t=Bi.T,a=_t.p;try{if(Bi.T=null,_t.p=2,e)return e()}finally{Bi.T=t,_t.p=a,_t.d.f()}};kt.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,_t.d.C(e,t))};kt.prefetchDNS=function(e){typeof e=="string"&&_t.d.D(e)};kt.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=jl(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?_t.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&_t.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};kt.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=jl(t.as,t.crossOrigin);_t.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&_t.d.M(e)};kt.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=jl(a,t.crossOrigin);_t.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};kt.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=jl(t.as,t.crossOrigin);_t.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else _t.d.m(e)};kt.requestFormReset=function(e){_t.d.r(e)};kt.unstable_batchedUpdates=function(e,t){return e(t)};kt.useFormState=function(e,t,a){return Bi.H.useFormState(e,t,a)};kt.useFormStatus=function(){return Bi.H.useHostTransitionStatus()};kt.version="19.1.0"});var Uv=Dn((U6,Pv)=>{"use strict";function Lv(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(Lv)}catch(e){console.error(e)}}Lv(),Pv.exports=Ov()});var F0=Dn(ic=>{"use strict";var rt=Dv(),sy=He(),OR=Uv();function j(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function iy(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function Co(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function oy(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function jv(e){if(Co(e)!==e)throw Error(j(188))}function LR(e){var t=e.alternate;if(!t){if(t=Co(e),t===null)throw Error(j(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return jv(r),e;if(s===n)return jv(r),t;s=s.sibling}throw Error(j(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(j(189))}}if(a.alternate!==n)throw Error(j(190))}if(a.tag!==3)throw Error(j(188));return a.stateNode.current===a?e:t}function ly(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=ly(e),t!==null)return t;e=e.sibling}return null}var Ae=Object.assign,PR=Symbol.for("react.element"),Fl=Symbol.for("react.transitional.element"),Yi=Symbol.for("react.portal"),ms=Symbol.for("react.fragment"),uy=Symbol.for("react.strict_mode"),bm=Symbol.for("react.profiler"),UR=Symbol.for("react.provider"),cy=Symbol.for("react.consumer"),cn=Symbol.for("react.context"),hf=Symbol.for("react.forward_ref"),xm=Symbol.for("react.suspense"),$m=Symbol.for("react.suspense_list"),vf=Symbol.for("react.memo"),Fn=Symbol.for("react.lazy");Symbol.for("react.scope");var wm=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var jR=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var Fv=Symbol.iterator;function zi(e){return e===null||typeof e!="object"?null:(e=Fv&&e[Fv]||e["@@iterator"],typeof e=="function"?e:null)}var FR=Symbol.for("react.client.reference");function Sm(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===FR?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case ms:return"Fragment";case bm:return"Profiler";case uy:return"StrictMode";case xm:return"Suspense";case $m:return"SuspenseList";case wm:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case Yi:return"Portal";case cn:return(e.displayName||"Context")+".Provider";case cy:return(e._context.displayName||"Context")+".Consumer";case hf:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case vf:return t=e.displayName||null,t!==null?t:Sm(e.type)||"Memo";case Fn:t=e._payload,e=e._init;try{return Sm(e(t))}catch{}}return null}var Ji=Array.isArray,ne=sy.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,ge=OR.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,Nr={pending:!1,data:null,method:null,action:null},Nm=[],fs=-1;function Va(e){return{current:e}}function dt(e){0>fs||(e.current=Nm[fs],Nm[fs]=null,fs--)}function Le(e,t){fs++,Nm[fs]=e.current,e.current=t}var Ka=Va(null),po=Va(null),Yn=Va(null),hu=Va(null);function vu(e,t){switch(Le(Yn,t),Le(po,e),Le(Ka,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?Hg(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=Hg(t),e=R0(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}dt(Ka),Le(Ka,e)}function Ds(){dt(Ka),dt(po),dt(Yn)}function _m(e){e.memoizedState!==null&&Le(hu,e);var t=Ka.current,a=R0(t,e.type);t!==a&&(Le(po,e),Le(Ka,a))}function gu(e){po.current===e&&(dt(Ka),dt(po)),hu.current===e&&(dt(hu),No._currentValue=Nr)}var km=Object.prototype.hasOwnProperty,gf=rt.unstable_scheduleCallback,qd=rt.unstable_cancelCallback,BR=rt.unstable_shouldYield,zR=rt.unstable_requestPaint,Ha=rt.unstable_now,qR=rt.unstable_getCurrentPriorityLevel,dy=rt.unstable_ImmediatePriority,my=rt.unstable_UserBlockingPriority,yu=rt.unstable_NormalPriority,IR=rt.unstable_LowPriority,fy=rt.unstable_IdlePriority,KR=rt.log,HR=rt.unstable_setDisableYieldValue,Eo=null,Jt=null;function Hn(e){if(typeof KR=="function"&&HR(e),Jt&&typeof Jt.setStrictMode=="function")try{Jt.setStrictMode(Eo,e)}catch{}}var Xt=Math.clz32?Math.clz32:GR,QR=Math.log,VR=Math.LN2;function GR(e){return e>>>=0,e===0?32:31-(QR(e)/VR|0)|0}var Bl=256,zl=4194304;function $r(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Hu(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=$r(n):(i&=o,i!==0?r=$r(i):a||(a=o&~e,a!==0&&(r=$r(a))))):(o=n&~s,o!==0?r=$r(o):i!==0?r=$r(i):a||(a=n&~e,a!==0&&(r=$r(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function To(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function YR(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function py(){var e=Bl;return Bl<<=1,(Bl&4194048)===0&&(Bl=256),e}function hy(){var e=zl;return zl<<=1,(zl&62914560)===0&&(zl=4194304),e}function Id(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function Ao(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function JR(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,u=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Xt(a),m=1<<d;o[d]=0,u[d]=-1;var f=c[d];if(f!==null)for(c[d]=null,d=0;d<f.length;d++){var h=f[d];h!==null&&(h.lane&=-536870913)}a&=~m}n!==0&&vy(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function vy(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Xt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function gy(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Xt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function yf(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function bf(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function yy(){var e=ge.p;return e!==0?e:(e=window.event,e===void 0?32:U0(e.type))}function XR(e,t){var a=ge.p;try{return ge.p=e,t()}finally{ge.p=a}}var ir=Math.random().toString(36).slice(2),bt="__reactFiber$"+ir,Ut="__reactProps$"+ir,Is="__reactContainer$"+ir,Rm="__reactEvents$"+ir,ZR="__reactListeners$"+ir,WR="__reactHandles$"+ir,Bv="__reactResources$"+ir,Do="__reactMarker$"+ir;function xf(e){delete e[bt],delete e[Ut],delete e[Rm],delete e[ZR],delete e[WR]}function ps(e){var t=e[bt];if(t)return t;for(var a=e.parentNode;a;){if(t=a[Is]||a[bt]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=Gg(e);e!==null;){if(a=e[bt])return a;e=Gg(e)}return t}e=a,a=e.parentNode}return null}function Ks(e){if(e=e[bt]||e[Is]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function Xi(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(j(33))}function Ns(e){var t=e[Bv];return t||(t=e[Bv]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function ut(e){e[Do]=!0}var by=new Set,xy={};function Lr(e,t){Ms(e,t),Ms(e+"Capture",t)}function Ms(e,t){for(xy[e]=t,e=0;e<t.length;e++)by.add(t[e])}var eC=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),zv={},qv={};function tC(e){return km.call(qv,e)?!0:km.call(zv,e)?!1:eC.test(e)?qv[e]=!0:(zv[e]=!0,!1)}function au(e,t,a){if(tC(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function ql(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function sn(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var Kd,Iv;function us(e){if(Kd===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);Kd=t&&t[1]||"",Iv=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+Kd+e+Iv}var Hd=!1;function Qd(e,t){if(!e||Hd)return"";Hd=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var m=function(){throw Error()};if(Object.defineProperty(m.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(m,[])}catch(h){var f=h}Reflect.construct(e,[],m)}else{try{m.call()}catch(h){f=h}e.call(m.prototype)}}else{try{throw Error()}catch(h){f=h}(m=e())&&typeof m.catch=="function"&&m.catch(function(){})}}catch(h){if(h&&f&&typeof h.stack=="string")return[h.stack,f.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var u=i.split(`
`),c=o.split(`
`);for(r=n=0;n<u.length&&!u[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===u.length||r===c.length)for(n=u.length-1,r=c.length-1;1<=n&&0<=r&&u[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(u[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||u[n]!==c[r]){var d=`
`+u[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{Hd=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?us(a):""}function aC(e){switch(e.tag){case 26:case 27:case 5:return us(e.type);case 16:return us("Lazy");case 13:return us("Suspense");case 19:return us("SuspenseList");case 0:case 15:return Qd(e.type,!1);case 11:return Qd(e.type.render,!1);case 1:return Qd(e.type,!0);case 31:return us("Activity");default:return""}}function Kv(e){try{var t="";do t+=aC(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function da(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function $y(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function nC(e){var t=$y(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function bu(e){e._valueTracker||(e._valueTracker=nC(e))}function wy(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=$y(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function xu(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var rC=/[\n"\\]/g;function pa(e){return e.replace(rC,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function Cm(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+da(t)):e.value!==""+da(t)&&(e.value=""+da(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?Em(e,i,da(t)):a!=null?Em(e,i,da(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+da(o):e.removeAttribute("name")}function Sy(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+da(a):"",t=t!=null?""+da(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function Em(e,t,a){t==="number"&&xu(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function _s(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+da(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function Ny(e,t,a){if(t!=null&&(t=""+da(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+da(a):""}function _y(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(j(92));if(Ji(n)){if(1<n.length)throw Error(j(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=da(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function Os(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var sC=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function Hv(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||sC.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function ky(e,t,a){if(t!=null&&typeof t!="object")throw Error(j(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&Hv(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&Hv(e,s,t[s])}function $f(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var iC=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),oC=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function nu(e){return oC.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var Tm=null;function wf(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var hs=null,ks=null;function Qv(e){var t=Ks(e);if(t&&(e=t.stateNode)){var a=e[Ut]||null;e:switch(e=t.stateNode,t.type){case"input":if(Cm(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+pa(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[Ut]||null;if(!r)throw Error(j(90));Cm(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&wy(n)}break e;case"textarea":Ny(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&_s(e,!!a.multiple,t,!1)}}}var Vd=!1;function Ry(e,t,a){if(Vd)return e(t,a);Vd=!0;try{var n=e(t);return n}finally{if(Vd=!1,(hs!==null||ks!==null)&&(tc(),hs&&(t=hs,e=ks,ks=hs=null,Qv(t),e)))for(t=0;t<e.length;t++)Qv(e[t])}}function ho(e,t){var a=e.stateNode;if(a===null)return null;var n=a[Ut]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(j(231,t,typeof a));return a}var gn=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),Am=!1;if(gn)try{is={},Object.defineProperty(is,"passive",{get:function(){Am=!0}}),window.addEventListener("test",is,is),window.removeEventListener("test",is,is)}catch{Am=!1}var is,Qn=null,Sf=null,ru=null;function Cy(){if(ru)return ru;var e,t=Sf,a=t.length,n,r="value"in Qn?Qn.value:Qn.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return ru=r.slice(e,1<n?1-n:void 0)}function su(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function Il(){return!0}function Vv(){return!1}function jt(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?Il:Vv,this.isPropagationStopped=Vv,this}return Ae(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=Il)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=Il)},persist:function(){},isPersistent:Il}),t}var Pr={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Qu=jt(Pr),Mo=Ae({},Pr,{view:0,detail:0}),lC=jt(Mo),Gd,Yd,qi,Vu=Ae({},Mo,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:Nf,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==qi&&(qi&&e.type==="mousemove"?(Gd=e.screenX-qi.screenX,Yd=e.screenY-qi.screenY):Yd=Gd=0,qi=e),Gd)},movementY:function(e){return"movementY"in e?e.movementY:Yd}}),Gv=jt(Vu),uC=Ae({},Vu,{dataTransfer:0}),cC=jt(uC),dC=Ae({},Mo,{relatedTarget:0}),Jd=jt(dC),mC=Ae({},Pr,{animationName:0,elapsedTime:0,pseudoElement:0}),fC=jt(mC),pC=Ae({},Pr,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),hC=jt(pC),vC=Ae({},Pr,{data:0}),Yv=jt(vC),gC={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},yC={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},bC={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function xC(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=bC[e])?!!t[e]:!1}function Nf(){return xC}var $C=Ae({},Mo,{key:function(e){if(e.key){var t=gC[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=su(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?yC[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:Nf,charCode:function(e){return e.type==="keypress"?su(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?su(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),wC=jt($C),SC=Ae({},Vu,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),Jv=jt(SC),NC=Ae({},Mo,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:Nf}),_C=jt(NC),kC=Ae({},Pr,{propertyName:0,elapsedTime:0,pseudoElement:0}),RC=jt(kC),CC=Ae({},Vu,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),EC=jt(CC),TC=Ae({},Pr,{newState:0,oldState:0}),AC=jt(TC),DC=[9,13,27,32],_f=gn&&"CompositionEvent"in window,Wi=null;gn&&"documentMode"in document&&(Wi=document.documentMode);var MC=gn&&"TextEvent"in window&&!Wi,Ey=gn&&(!_f||Wi&&8<Wi&&11>=Wi),Xv=" ",Zv=!1;function Ty(e,t){switch(e){case"keyup":return DC.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function Ay(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var vs=!1;function OC(e,t){switch(e){case"compositionend":return Ay(t);case"keypress":return t.which!==32?null:(Zv=!0,Xv);case"textInput":return e=t.data,e===Xv&&Zv?null:e;default:return null}}function LC(e,t){if(vs)return e==="compositionend"||!_f&&Ty(e,t)?(e=Cy(),ru=Sf=Qn=null,vs=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return Ey&&t.locale!=="ko"?null:t.data;default:return null}}var PC={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function Wv(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!PC[e.type]:t==="textarea"}function Dy(e,t,a,n){hs?ks?ks.push(n):ks=[n]:hs=n,t=ju(t,"onChange"),0<t.length&&(a=new Qu("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var eo=null,vo=null;function UC(e){N0(e,0)}function Gu(e){var t=Xi(e);if(wy(t))return e}function eg(e,t){if(e==="change")return t}var My=!1;gn&&(gn?(Hl="oninput"in document,Hl||(Xd=document.createElement("div"),Xd.setAttribute("oninput","return;"),Hl=typeof Xd.oninput=="function"),Kl=Hl):Kl=!1,My=Kl&&(!document.documentMode||9<document.documentMode));var Kl,Hl,Xd;function tg(){eo&&(eo.detachEvent("onpropertychange",Oy),vo=eo=null)}function Oy(e){if(e.propertyName==="value"&&Gu(vo)){var t=[];Dy(t,vo,e,wf(e)),Ry(UC,t)}}function jC(e,t,a){e==="focusin"?(tg(),eo=t,vo=a,eo.attachEvent("onpropertychange",Oy)):e==="focusout"&&tg()}function FC(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return Gu(vo)}function BC(e,t){if(e==="click")return Gu(t)}function zC(e,t){if(e==="input"||e==="change")return Gu(t)}function qC(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var ea=typeof Object.is=="function"?Object.is:qC;function go(e,t){if(ea(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!km.call(t,r)||!ea(e[r],t[r]))return!1}return!0}function ag(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function ng(e,t){var a=ag(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=ag(a)}}function Ly(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?Ly(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function Py(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=xu(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=xu(e.document)}return t}function kf(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var IC=gn&&"documentMode"in document&&11>=document.documentMode,gs=null,Dm=null,to=null,Mm=!1;function rg(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;Mm||gs==null||gs!==xu(n)||(n=gs,"selectionStart"in n&&kf(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),to&&go(to,n)||(to=n,n=ju(Dm,"onSelect"),0<n.length&&(t=new Qu("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=gs)))}function xr(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var ys={animationend:xr("Animation","AnimationEnd"),animationiteration:xr("Animation","AnimationIteration"),animationstart:xr("Animation","AnimationStart"),transitionrun:xr("Transition","TransitionRun"),transitionstart:xr("Transition","TransitionStart"),transitioncancel:xr("Transition","TransitionCancel"),transitionend:xr("Transition","TransitionEnd")},Zd={},Uy={};gn&&(Uy=document.createElement("div").style,"AnimationEvent"in window||(delete ys.animationend.animation,delete ys.animationiteration.animation,delete ys.animationstart.animation),"TransitionEvent"in window||delete ys.transitionend.transition);function Ur(e){if(Zd[e])return Zd[e];if(!ys[e])return e;var t=ys[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in Uy)return Zd[e]=t[a];return e}var jy=Ur("animationend"),Fy=Ur("animationiteration"),By=Ur("animationstart"),KC=Ur("transitionrun"),HC=Ur("transitionstart"),QC=Ur("transitioncancel"),zy=Ur("transitionend"),qy=new Map,Om="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");Om.push("scrollEnd");function Ra(e,t){qy.set(e,t),Lr(t,[e])}var sg=new WeakMap;function ha(e,t){if(typeof e=="object"&&e!==null){var a=sg.get(e);return a!==void 0?a:(t={value:e,source:t,stack:Kv(t)},sg.set(e,t),t)}return{value:e,source:t,stack:Kv(t)}}var ca=[],bs=0,Rf=0;function Yu(){for(var e=bs,t=Rf=bs=0;t<e;){var a=ca[t];ca[t++]=null;var n=ca[t];ca[t++]=null;var r=ca[t];ca[t++]=null;var s=ca[t];if(ca[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&Iy(a,r,s)}}function Ju(e,t,a,n){ca[bs++]=e,ca[bs++]=t,ca[bs++]=a,ca[bs++]=n,Rf|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function Cf(e,t,a,n){return Ju(e,t,a,n),$u(e)}function Hs(e,t){return Ju(e,null,null,t),$u(e)}function Iy(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Xt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function $u(e){if(50<mo)throw mo=0,tf=null,Error(j(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var xs={};function VC(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Yt(e,t,a,n){return new VC(e,t,a,n)}function Ef(e){return e=e.prototype,!(!e||!e.isReactComponent)}function hn(e,t){var a=e.alternate;return a===null?(a=Yt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function Ky(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function iu(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")Ef(e)&&(i=1);else if(typeof e=="string")i=VE(e,a,Ka.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case wm:return e=Yt(31,a,t,r),e.elementType=wm,e.lanes=s,e;case ms:return _r(a.children,r,s,t);case uy:i=8,r|=24;break;case bm:return e=Yt(12,a,t,r|2),e.elementType=bm,e.lanes=s,e;case xm:return e=Yt(13,a,t,r),e.elementType=xm,e.lanes=s,e;case $m:return e=Yt(19,a,t,r),e.elementType=$m,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case UR:case cn:i=10;break e;case cy:i=9;break e;case hf:i=11;break e;case vf:i=14;break e;case Fn:i=16,n=null;break e}i=29,a=Error(j(130,e===null?"null":typeof e,"")),n=null}return t=Yt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function _r(e,t,a,n){return e=Yt(7,e,n,t),e.lanes=a,e}function Wd(e,t,a){return e=Yt(6,e,null,t),e.lanes=a,e}function em(e,t,a){return t=Yt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var $s=[],ws=0,wu=null,Su=0,ma=[],fa=0,kr=null,dn=1,mn="";function wr(e,t){$s[ws++]=Su,$s[ws++]=wu,wu=e,Su=t}function Hy(e,t,a){ma[fa++]=dn,ma[fa++]=mn,ma[fa++]=kr,kr=e;var n=dn;e=mn;var r=32-Xt(n)-1;n&=~(1<<r),a+=1;var s=32-Xt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,dn=1<<32-Xt(t)+r|a<<r|n,mn=s+e}else dn=1<<s|a<<r|n,mn=e}function Tf(e){e.return!==null&&(wr(e,1),Hy(e,1,0))}function Af(e){for(;e===wu;)wu=$s[--ws],$s[ws]=null,Su=$s[--ws],$s[ws]=null;for(;e===kr;)kr=ma[--fa],ma[fa]=null,mn=ma[--fa],ma[fa]=null,dn=ma[--fa],ma[fa]=null}var Rt=null,ze=null,ve=!1,Rr=null,qa=!1,Lm=Error(j(519));function Ar(e){var t=Error(j(418,""));throw yo(ha(t,e)),Lm}function ig(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[bt]=e,t[Ut]=n,a){case"dialog":le("cancel",t),le("close",t);break;case"iframe":case"object":case"embed":le("load",t);break;case"video":case"audio":for(a=0;a<$o.length;a++)le($o[a],t);break;case"source":le("error",t);break;case"img":case"image":case"link":le("error",t),le("load",t);break;case"details":le("toggle",t);break;case"input":le("invalid",t),Sy(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),bu(t);break;case"select":le("invalid",t);break;case"textarea":le("invalid",t),_y(t,n.value,n.defaultValue,n.children),bu(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||k0(t.textContent,a)?(n.popover!=null&&(le("beforetoggle",t),le("toggle",t)),n.onScroll!=null&&le("scroll",t),n.onScrollEnd!=null&&le("scrollend",t),n.onClick!=null&&(t.onclick=rc),t=!0):t=!1,t||Ar(e)}function og(e){for(Rt=e.return;Rt;)switch(Rt.tag){case 5:case 13:qa=!1;return;case 27:case 3:qa=!0;return;default:Rt=Rt.return}}function Ii(e){if(e!==Rt)return!1;if(!ve)return og(e),ve=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||lf(e.type,e.memoizedProps)),a=!a),a&&ze&&Ar(e),og(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(j(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){ze=ka(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}ze=null}}else t===27?(t=ze,or(e.type)?(e=df,df=null,ze=e):ze=t):ze=Rt?ka(e.stateNode.nextSibling):null;return!0}function Oo(){ze=Rt=null,ve=!1}function lg(){var e=Rr;return e!==null&&(Pt===null?Pt=e:Pt.push.apply(Pt,e),Rr=null),e}function yo(e){Rr===null?Rr=[e]:Rr.push(e)}var Pm=Va(null),jr=null,fn=null;function zn(e,t,a){Le(Pm,t._currentValue),t._currentValue=a}function vn(e){e._currentValue=Pm.current,dt(Pm)}function Um(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function jm(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var u=0;u<t.length;u++)if(o.context===t[u]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),Um(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(j(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),Um(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function Lo(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(j(387));if(i=i.memoizedProps,i!==null){var o=r.type;ea(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===hu.current){if(i=r.alternate,i===null)throw Error(j(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(No):e=[No])}r=r.return}e!==null&&jm(t,e,a,n),t.flags|=262144}function Nu(e){for(e=e.firstContext;e!==null;){if(!ea(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function Dr(e){jr=e,fn=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function xt(e){return Qy(jr,e)}function Ql(e,t){return jr===null&&Dr(e),Qy(e,t)}function Qy(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},fn===null){if(e===null)throw Error(j(308));fn=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else fn=fn.next=t;return a}var GC=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},YC=rt.unstable_scheduleCallback,JC=rt.unstable_NormalPriority,at={$$typeof:cn,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function Df(){return{controller:new GC,data:new Map,refCount:0}}function Po(e){e.refCount--,e.refCount===0&&YC(JC,function(){e.controller.abort()})}var ao=null,Fm=0,Ls=0,Rs=null;function XC(e,t){if(ao===null){var a=ao=[];Fm=0,Ls=tp(),Rs={status:"pending",value:void 0,then:function(n){a.push(n)}}}return Fm++,t.then(ug,ug),t}function ug(){if(--Fm===0&&ao!==null){Rs!==null&&(Rs.status="fulfilled");var e=ao;ao=null,Ls=0,Rs=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function ZC(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var cg=ne.S;ne.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&XC(e,t),cg!==null&&cg(e,t)};var Cr=Va(null);function Mf(){var e=Cr.current;return e!==null?e:Ce.pooledCache}function ou(e,t){t===null?Le(Cr,Cr.current):Le(Cr,t.pool)}function Vy(){var e=Mf();return e===null?null:{parent:at._currentValue,pool:e}}var Uo=Error(j(460)),Gy=Error(j(474)),Xu=Error(j(542)),Bm={then:function(){}};function dg(e){return e=e.status,e==="fulfilled"||e==="rejected"}function Vl(){}function Yy(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(Vl,Vl),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,fg(e),e;default:if(typeof t.status=="string")t.then(Vl,Vl);else{if(e=Ce,e!==null&&100<e.shellSuspendCounter)throw Error(j(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,fg(e),e}throw no=t,Uo}}var no=null;function mg(){if(no===null)throw Error(j(459));var e=no;return no=null,e}function fg(e){if(e===Uo||e===Xu)throw Error(j(483))}var Bn=!1;function Of(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function zm(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function Jn(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function Xn(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,(Ne&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=$u(e),Iy(e,null,a),t}return Ju(e,n,t,a),$u(e)}function ro(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,gy(e,a)}}function tm(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var qm=!1;function so(){if(qm){var e=Rs;if(e!==null)throw e}}function io(e,t,a,n){qm=!1;var r=e.updateQueue;Bn=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var u=o,c=u.next;u.next=null,i===null?s=c:i.next=c,i=u;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=u))}if(s!==null){var m=r.baseState;i=0,d=c=u=null,o=s;do{var f=o.lane&-536870913,h=f!==o.lane;if(h?(me&f)===f:(n&f)===f){f!==0&&f===Ls&&(qm=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var x=e,y=o;f=t;var $=a;switch(y.tag){case 1:if(x=y.payload,typeof x=="function"){m=x.call($,m,f);break e}m=x;break e;case 3:x.flags=x.flags&-65537|128;case 0:if(x=y.payload,f=typeof x=="function"?x.call($,m,f):x,f==null)break e;m=Ae({},m,f);break e;case 2:Bn=!0}}f=o.callback,f!==null&&(e.flags|=64,h&&(e.flags|=8192),h=r.callbacks,h===null?r.callbacks=[f]:h.push(f))}else h={lane:f,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=h,u=m):d=d.next=h,i|=f;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;h=o,o=h.next,h.next=null,r.lastBaseUpdate=h,r.shared.pending=null}}while(!0);d===null&&(u=m),r.baseState=u,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),sr|=i,e.lanes=i,e.memoizedState=m}}function Jy(e,t){if(typeof e!="function")throw Error(j(191,e));e.call(t)}function Xy(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)Jy(a[e],t)}var Ps=Va(null),_u=Va(0);function pg(e,t){e=xn,Le(_u,e),Le(Ps,t),xn=e|t.baseLanes}function Im(){Le(_u,xn),Le(Ps,Ps.current)}function Lf(){xn=_u.current,dt(Ps),dt(_u)}var nr=0,oe=null,ke=null,Je=null,ku=!1,Cs=!1,Mr=!1,Ru=0,bo=0,Es=null,WC=0;function Qe(){throw Error(j(321))}function Pf(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!ea(e[a],t[a]))return!1;return!0}function Uf(e,t,a,n,r,s){return nr=s,oe=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,ne.H=e===null||e.memoizedState===null?Cb:Eb,Mr=!1,s=a(n,r),Mr=!1,Cs&&(s=Wy(t,a,n,r)),Zy(e),s}function Zy(e){ne.H=Cu;var t=ke!==null&&ke.next!==null;if(nr=0,Je=ke=oe=null,ku=!1,bo=0,Es=null,t)throw Error(j(300));e===null||ct||(e=e.dependencies,e!==null&&Nu(e)&&(ct=!0))}function Wy(e,t,a,n){oe=e;var r=0;do{if(Cs&&(Es=null),bo=0,Cs=!1,25<=r)throw Error(j(301));if(r+=1,Je=ke=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}ne.H=iE,s=t(a,n)}while(Cs);return s}function eE(){var e=ne.H,t=e.useState()[0];return t=typeof t.then=="function"?jo(t):t,e=e.useState()[0],(ke!==null?ke.memoizedState:null)!==e&&(oe.flags|=1024),t}function jf(){var e=Ru!==0;return Ru=0,e}function Ff(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function Bf(e){if(ku){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}ku=!1}nr=0,Je=ke=oe=null,Cs=!1,bo=Ru=0,Es=null}function Ot(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return Je===null?oe.memoizedState=Je=e:Je=Je.next=e,Je}function Xe(){if(ke===null){var e=oe.alternate;e=e!==null?e.memoizedState:null}else e=ke.next;var t=Je===null?oe.memoizedState:Je.next;if(t!==null)Je=t,ke=e;else{if(e===null)throw oe.alternate===null?Error(j(467)):Error(j(310));ke=e,e={memoizedState:ke.memoizedState,baseState:ke.baseState,baseQueue:ke.baseQueue,queue:ke.queue,next:null},Je===null?oe.memoizedState=Je=e:Je=Je.next=e}return Je}function zf(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function jo(e){var t=bo;return bo+=1,Es===null&&(Es=[]),e=Yy(Es,e,t),t=oe,(Je===null?t.memoizedState:Je.next)===null&&(t=t.alternate,ne.H=t===null||t.memoizedState===null?Cb:Eb),e}function Zu(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return jo(e);if(e.$$typeof===cn)return xt(e)}throw Error(j(438,String(e)))}function qf(e){var t=null,a=oe.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=oe.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=zf(),oe.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=jR;return t.index++,a}function yn(e,t){return typeof t=="function"?t(e):t}function lu(e){var t=Xe();return If(t,ke,e)}function If(e,t,a){var n=e.queue;if(n===null)throw Error(j(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,u=null,c=t,d=!1;do{var m=c.lane&-536870913;if(m!==c.lane?(me&m)===m:(nr&m)===m){var f=c.revertLane;if(f===0)u!==null&&(u=u.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),m===Ls&&(d=!0);else if((nr&f)===f){c=c.next,f===Ls&&(d=!0);continue}else m={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=m,i=s):u=u.next=m,oe.lanes|=f,sr|=f;m=c.action,Mr&&a(s,m),s=c.hasEagerState?c.eagerState:a(s,m)}else f={lane:m,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=f,i=s):u=u.next=f,oe.lanes|=m,sr|=m;c=c.next}while(c!==null&&c!==t);if(u===null?i=s:u.next=o,!ea(s,e.memoizedState)&&(ct=!0,d&&(a=Rs,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=u,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function am(e){var t=Xe(),a=t.queue;if(a===null)throw Error(j(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);ea(s,t.memoizedState)||(ct=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function eb(e,t,a){var n=oe,r=Xe(),s=ve;if(s){if(a===void 0)throw Error(j(407));a=a()}else a=t();var i=!ea((ke||r).memoizedState,a);i&&(r.memoizedState=a,ct=!0),r=r.queue;var o=nb.bind(null,n,r,e);if(Fo(2048,8,o,[e]),r.getSnapshot!==t||i||Je!==null&&Je.memoizedState.tag&1){if(n.flags|=2048,Us(9,Wu(),ab.bind(null,n,r,a,t),null),Ce===null)throw Error(j(349));s||(nr&124)!==0||tb(n,t,a)}return a}function tb(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=oe.updateQueue,t===null?(t=zf(),oe.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function ab(e,t,a,n){t.value=a,t.getSnapshot=n,rb(t)&&sb(e)}function nb(e,t,a){return a(function(){rb(t)&&sb(e)})}function rb(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!ea(e,a)}catch{return!0}}function sb(e){var t=Hs(e,2);t!==null&&Wt(t,e,2)}function Km(e){var t=Ot();if(typeof e=="function"){var a=e;if(e=a(),Mr){Hn(!0);try{a()}finally{Hn(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:yn,lastRenderedState:e},t}function ib(e,t,a,n){return e.baseState=a,If(e,ke,typeof n=="function"?n:yn)}function tE(e,t,a,n,r){if(ec(e))throw Error(j(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};ne.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,ob(t,s)):(s.next=a.next,t.pending=a.next=s)}}function ob(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=ne.T,i={};ne.T=i;try{var o=a(r,n),u=ne.S;u!==null&&u(i,o),hg(e,t,o)}catch(c){Hm(e,t,c)}finally{ne.T=s}}else try{s=a(r,n),hg(e,t,s)}catch(c){Hm(e,t,c)}}function hg(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){vg(e,t,n)},function(n){return Hm(e,t,n)}):vg(e,t,a)}function vg(e,t,a){t.status="fulfilled",t.value=a,lb(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,ob(e,a)))}function Hm(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,lb(t),t=t.next;while(t!==n)}e.action=null}function lb(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function ub(e,t){return t}function gg(e,t){if(ve){var a=Ce.formState;if(a!==null){e:{var n=oe;if(ve){if(ze){t:{for(var r=ze,s=qa;r.nodeType!==8;){if(!s){r=null;break t}if(r=ka(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){ze=ka(r.nextSibling),n=r.data==="F!";break e}}Ar(n)}n=!1}n&&(t=a[0])}}return a=Ot(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:ub,lastRenderedState:t},a.queue=n,a=_b.bind(null,oe,n),n.dispatch=a,n=Km(!1),s=Vf.bind(null,oe,!1,n.queue),n=Ot(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=tE.bind(null,oe,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function yg(e){var t=Xe();return cb(t,ke,e)}function cb(e,t,a){if(t=If(e,t,ub)[0],e=lu(yn)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=jo(t)}catch(i){throw i===Uo?Xu:i}else n=t;t=Xe();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(oe.flags|=2048,Us(9,Wu(),aE.bind(null,r,a),null)),[n,s,e]}function aE(e,t){e.action=t}function bg(e){var t=Xe(),a=ke;if(a!==null)return cb(t,a,e);Xe(),t=t.memoizedState,a=Xe();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function Us(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=oe.updateQueue,t===null&&(t=zf(),oe.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function Wu(){return{destroy:void 0,resource:void 0}}function db(){return Xe().memoizedState}function uu(e,t,a,n){var r=Ot();n=n===void 0?null:n,oe.flags|=e,r.memoizedState=Us(1|t,Wu(),a,n)}function Fo(e,t,a,n){var r=Xe();n=n===void 0?null:n;var s=r.memoizedState.inst;ke!==null&&n!==null&&Pf(n,ke.memoizedState.deps)?r.memoizedState=Us(t,s,a,n):(oe.flags|=e,r.memoizedState=Us(1|t,s,a,n))}function xg(e,t){uu(8390656,8,e,t)}function mb(e,t){Fo(2048,8,e,t)}function fb(e,t){return Fo(4,2,e,t)}function pb(e,t){return Fo(4,4,e,t)}function hb(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function vb(e,t,a){a=a!=null?a.concat([e]):null,Fo(4,4,hb.bind(null,t,e),a)}function Kf(){}function gb(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&Pf(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function yb(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&Pf(t,n[1]))return n[0];if(n=e(),Mr){Hn(!0);try{e()}finally{Hn(!1)}}return a.memoizedState=[n,t],n}function Hf(e,t,a){return a===void 0||(nr&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=l0(),oe.lanes|=e,sr|=e,a)}function bb(e,t,a,n){return ea(a,t)?a:Ps.current!==null?(e=Hf(e,a,n),ea(e,t)||(ct=!0),e):(nr&42)===0?(ct=!0,e.memoizedState=a):(e=l0(),oe.lanes|=e,sr|=e,t)}function xb(e,t,a,n,r){var s=ge.p;ge.p=s!==0&&8>s?s:8;var i=ne.T,o={};ne.T=o,Vf(e,!1,t,a);try{var u=r(),c=ne.S;if(c!==null&&c(o,u),u!==null&&typeof u=="object"&&typeof u.then=="function"){var d=ZC(u,n);oo(e,t,d,Zt(e))}else oo(e,t,n,Zt(e))}catch(m){oo(e,t,{then:function(){},status:"rejected",reason:m},Zt())}finally{ge.p=s,ne.T=i}}function nE(){}function Qm(e,t,a,n){if(e.tag!==5)throw Error(j(476));var r=$b(e).queue;xb(e,r,t,Nr,a===null?nE:function(){return wb(e),a(n)})}function $b(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:Nr,baseState:Nr,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:yn,lastRenderedState:Nr},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:yn,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function wb(e){var t=$b(e).next.queue;oo(e,t,{},Zt())}function Qf(){return xt(No)}function Sb(){return Xe().memoizedState}function Nb(){return Xe().memoizedState}function rE(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=Zt();e=Jn(a);var n=Xn(t,e,a);n!==null&&(Wt(n,t,a),ro(n,t,a)),t={cache:Df()},e.payload=t;return}t=t.return}}function sE(e,t,a){var n=Zt();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},ec(e)?kb(t,a):(a=Cf(e,t,a,n),a!==null&&(Wt(a,e,n),Rb(a,t,n)))}function _b(e,t,a){var n=Zt();oo(e,t,a,n)}function oo(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(ec(e))kb(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,ea(o,i))return Ju(e,t,r,0),Ce===null&&Yu(),!1}catch{}finally{}if(a=Cf(e,t,r,n),a!==null)return Wt(a,e,n),Rb(a,t,n),!0}return!1}function Vf(e,t,a,n){if(n={lane:2,revertLane:tp(),action:n,hasEagerState:!1,eagerState:null,next:null},ec(e)){if(t)throw Error(j(479))}else t=Cf(e,a,n,2),t!==null&&Wt(t,e,2)}function ec(e){var t=e.alternate;return e===oe||t!==null&&t===oe}function kb(e,t){Cs=ku=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function Rb(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,gy(e,a)}}var Cu={readContext:xt,use:Zu,useCallback:Qe,useContext:Qe,useEffect:Qe,useImperativeHandle:Qe,useLayoutEffect:Qe,useInsertionEffect:Qe,useMemo:Qe,useReducer:Qe,useRef:Qe,useState:Qe,useDebugValue:Qe,useDeferredValue:Qe,useTransition:Qe,useSyncExternalStore:Qe,useId:Qe,useHostTransitionStatus:Qe,useFormState:Qe,useActionState:Qe,useOptimistic:Qe,useMemoCache:Qe,useCacheRefresh:Qe},Cb={readContext:xt,use:Zu,useCallback:function(e,t){return Ot().memoizedState=[e,t===void 0?null:t],e},useContext:xt,useEffect:xg,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,uu(4194308,4,hb.bind(null,t,e),a)},useLayoutEffect:function(e,t){return uu(4194308,4,e,t)},useInsertionEffect:function(e,t){uu(4,2,e,t)},useMemo:function(e,t){var a=Ot();t=t===void 0?null:t;var n=e();if(Mr){Hn(!0);try{e()}finally{Hn(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=Ot();if(a!==void 0){var r=a(t);if(Mr){Hn(!0);try{a(t)}finally{Hn(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=sE.bind(null,oe,e),[n.memoizedState,e]},useRef:function(e){var t=Ot();return e={current:e},t.memoizedState=e},useState:function(e){e=Km(e);var t=e.queue,a=_b.bind(null,oe,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:Kf,useDeferredValue:function(e,t){var a=Ot();return Hf(a,e,t)},useTransition:function(){var e=Km(!1);return e=xb.bind(null,oe,e.queue,!0,!1),Ot().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=oe,r=Ot();if(ve){if(a===void 0)throw Error(j(407));a=a()}else{if(a=t(),Ce===null)throw Error(j(349));(me&124)!==0||tb(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,xg(nb.bind(null,n,s,e),[e]),n.flags|=2048,Us(9,Wu(),ab.bind(null,n,s,a,t),null),a},useId:function(){var e=Ot(),t=Ce.identifierPrefix;if(ve){var a=mn,n=dn;a=(n&~(1<<32-Xt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=Ru++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=WC++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:Qf,useFormState:gg,useActionState:gg,useOptimistic:function(e){var t=Ot();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=Vf.bind(null,oe,!0,a),a.dispatch=t,[e,t]},useMemoCache:qf,useCacheRefresh:function(){return Ot().memoizedState=rE.bind(null,oe)}},Eb={readContext:xt,use:Zu,useCallback:gb,useContext:xt,useEffect:mb,useImperativeHandle:vb,useInsertionEffect:fb,useLayoutEffect:pb,useMemo:yb,useReducer:lu,useRef:db,useState:function(){return lu(yn)},useDebugValue:Kf,useDeferredValue:function(e,t){var a=Xe();return bb(a,ke.memoizedState,e,t)},useTransition:function(){var e=lu(yn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:jo(e),t]},useSyncExternalStore:eb,useId:Sb,useHostTransitionStatus:Qf,useFormState:yg,useActionState:yg,useOptimistic:function(e,t){var a=Xe();return ib(a,ke,e,t)},useMemoCache:qf,useCacheRefresh:Nb},iE={readContext:xt,use:Zu,useCallback:gb,useContext:xt,useEffect:mb,useImperativeHandle:vb,useInsertionEffect:fb,useLayoutEffect:pb,useMemo:yb,useReducer:am,useRef:db,useState:function(){return am(yn)},useDebugValue:Kf,useDeferredValue:function(e,t){var a=Xe();return ke===null?Hf(a,e,t):bb(a,ke.memoizedState,e,t)},useTransition:function(){var e=am(yn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:jo(e),t]},useSyncExternalStore:eb,useId:Sb,useHostTransitionStatus:Qf,useFormState:bg,useActionState:bg,useOptimistic:function(e,t){var a=Xe();return ke!==null?ib(a,ke,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:qf,useCacheRefresh:Nb},Ts=null,xo=0;function Gl(e){var t=xo;return xo+=1,Ts===null&&(Ts=[]),Yy(Ts,e,t)}function Ki(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function Yl(e,t){throw t.$$typeof===PR?Error(j(525)):(e=Object.prototype.toString.call(t),Error(j(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function $g(e){var t=e._init;return t(e._payload)}function Tb(e){function t(g,v){if(e){var b=g.deletions;b===null?(g.deletions=[v],g.flags|=16):b.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=hn(g,v),g.index=0,g.sibling=null,g}function s(g,v,b){return g.index=b,e?(b=g.alternate,b!==null?(b=b.index,b<v?(g.flags|=67108866,v):b):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,b,w){return v===null||v.tag!==6?(v=Wd(b,g.mode,w),v.return=g,v):(v=r(v,b),v.return=g,v)}function u(g,v,b,w){var S=b.type;return S===ms?d(g,v,b.props.children,w,b.key):v!==null&&(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Fn&&$g(S)===v.type)?(v=r(v,b.props),Ki(v,b),v.return=g,v):(v=iu(b.type,b.key,b.props,null,g.mode,w),Ki(v,b),v.return=g,v)}function c(g,v,b,w){return v===null||v.tag!==4||v.stateNode.containerInfo!==b.containerInfo||v.stateNode.implementation!==b.implementation?(v=em(b,g.mode,w),v.return=g,v):(v=r(v,b.children||[]),v.return=g,v)}function d(g,v,b,w,S){return v===null||v.tag!==7?(v=_r(b,g.mode,w,S),v.return=g,v):(v=r(v,b),v.return=g,v)}function m(g,v,b){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=Wd(""+v,g.mode,b),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case Fl:return b=iu(v.type,v.key,v.props,null,g.mode,b),Ki(b,v),b.return=g,b;case Yi:return v=em(v,g.mode,b),v.return=g,v;case Fn:var w=v._init;return v=w(v._payload),m(g,v,b)}if(Ji(v)||zi(v))return v=_r(v,g.mode,b,null),v.return=g,v;if(typeof v.then=="function")return m(g,Gl(v),b);if(v.$$typeof===cn)return m(g,Ql(g,v),b);Yl(g,v)}return null}function f(g,v,b,w){var S=v!==null?v.key:null;if(typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint")return S!==null?null:o(g,v,""+b,w);if(typeof b=="object"&&b!==null){switch(b.$$typeof){case Fl:return b.key===S?u(g,v,b,w):null;case Yi:return b.key===S?c(g,v,b,w):null;case Fn:return S=b._init,b=S(b._payload),f(g,v,b,w)}if(Ji(b)||zi(b))return S!==null?null:d(g,v,b,w,null);if(typeof b.then=="function")return f(g,v,Gl(b),w);if(b.$$typeof===cn)return f(g,v,Ql(g,b),w);Yl(g,b)}return null}function h(g,v,b,w,S){if(typeof w=="string"&&w!==""||typeof w=="number"||typeof w=="bigint")return g=g.get(b)||null,o(v,g,""+w,S);if(typeof w=="object"&&w!==null){switch(w.$$typeof){case Fl:return g=g.get(w.key===null?b:w.key)||null,u(v,g,w,S);case Yi:return g=g.get(w.key===null?b:w.key)||null,c(v,g,w,S);case Fn:var E=w._init;return w=E(w._payload),h(g,v,b,w,S)}if(Ji(w)||zi(w))return g=g.get(b)||null,d(v,g,w,S,null);if(typeof w.then=="function")return h(g,v,b,Gl(w),S);if(w.$$typeof===cn)return h(g,v,b,Ql(v,w),S);Yl(v,w)}return null}function x(g,v,b,w){for(var S=null,E=null,k=v,C=v=0,O=null;k!==null&&C<b.length;C++){k.index>C?(O=k,k=null):O=k.sibling;var L=f(g,k,b[C],w);if(L===null){k===null&&(k=O);break}e&&k&&L.alternate===null&&t(g,k),v=s(L,v,C),E===null?S=L:E.sibling=L,E=L,k=O}if(C===b.length)return a(g,k),ve&&wr(g,C),S;if(k===null){for(;C<b.length;C++)k=m(g,b[C],w),k!==null&&(v=s(k,v,C),E===null?S=k:E.sibling=k,E=k);return ve&&wr(g,C),S}for(k=n(k);C<b.length;C++)O=h(k,g,C,b[C],w),O!==null&&(e&&O.alternate!==null&&k.delete(O.key===null?C:O.key),v=s(O,v,C),E===null?S=O:E.sibling=O,E=O);return e&&k.forEach(function(P){return t(g,P)}),ve&&wr(g,C),S}function y(g,v,b,w){if(b==null)throw Error(j(151));for(var S=null,E=null,k=v,C=v=0,O=null,L=b.next();k!==null&&!L.done;C++,L=b.next()){k.index>C?(O=k,k=null):O=k.sibling;var P=f(g,k,L.value,w);if(P===null){k===null&&(k=O);break}e&&k&&P.alternate===null&&t(g,k),v=s(P,v,C),E===null?S=P:E.sibling=P,E=P,k=O}if(L.done)return a(g,k),ve&&wr(g,C),S;if(k===null){for(;!L.done;C++,L=b.next())L=m(g,L.value,w),L!==null&&(v=s(L,v,C),E===null?S=L:E.sibling=L,E=L);return ve&&wr(g,C),S}for(k=n(k);!L.done;C++,L=b.next())L=h(k,g,C,L.value,w),L!==null&&(e&&L.alternate!==null&&k.delete(L.key===null?C:L.key),v=s(L,v,C),E===null?S=L:E.sibling=L,E=L);return e&&k.forEach(function(U){return t(g,U)}),ve&&wr(g,C),S}function $(g,v,b,w){if(typeof b=="object"&&b!==null&&b.type===ms&&b.key===null&&(b=b.props.children),typeof b=="object"&&b!==null){switch(b.$$typeof){case Fl:e:{for(var S=b.key;v!==null;){if(v.key===S){if(S=b.type,S===ms){if(v.tag===7){a(g,v.sibling),w=r(v,b.props.children),w.return=g,g=w;break e}}else if(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Fn&&$g(S)===v.type){a(g,v.sibling),w=r(v,b.props),Ki(w,b),w.return=g,g=w;break e}a(g,v);break}else t(g,v);v=v.sibling}b.type===ms?(w=_r(b.props.children,g.mode,w,b.key),w.return=g,g=w):(w=iu(b.type,b.key,b.props,null,g.mode,w),Ki(w,b),w.return=g,g=w)}return i(g);case Yi:e:{for(S=b.key;v!==null;){if(v.key===S)if(v.tag===4&&v.stateNode.containerInfo===b.containerInfo&&v.stateNode.implementation===b.implementation){a(g,v.sibling),w=r(v,b.children||[]),w.return=g,g=w;break e}else{a(g,v);break}else t(g,v);v=v.sibling}w=em(b,g.mode,w),w.return=g,g=w}return i(g);case Fn:return S=b._init,b=S(b._payload),$(g,v,b,w)}if(Ji(b))return x(g,v,b,w);if(zi(b)){if(S=zi(b),typeof S!="function")throw Error(j(150));return b=S.call(b),y(g,v,b,w)}if(typeof b.then=="function")return $(g,v,Gl(b),w);if(b.$$typeof===cn)return $(g,v,Ql(g,b),w);Yl(g,b)}return typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint"?(b=""+b,v!==null&&v.tag===6?(a(g,v.sibling),w=r(v,b),w.return=g,g=w):(a(g,v),w=Wd(b,g.mode,w),w.return=g,g=w),i(g)):a(g,v)}return function(g,v,b,w){try{xo=0;var S=$(g,v,b,w);return Ts=null,S}catch(k){if(k===Uo||k===Xu)throw k;var E=Yt(29,k,null,g.mode);return E.lanes=w,E.return=g,E}finally{}}}var js=Tb(!0),Ab=Tb(!1),ga=Va(null),Qa=null;function qn(e){var t=e.alternate;Le(nt,nt.current&1),Le(ga,e),Qa===null&&(t===null||Ps.current!==null||t.memoizedState!==null)&&(Qa=e)}function Db(e){if(e.tag===22){if(Le(nt,nt.current),Le(ga,e),Qa===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(Qa=e)}}else In(e)}function In(){Le(nt,nt.current),Le(ga,ga.current)}function pn(e){dt(ga),Qa===e&&(Qa=null),dt(nt)}var nt=Va(0);function Eu(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||cf(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function nm(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:Ae({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var Vm={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=Zt(),r=Jn(n);r.payload=t,a!=null&&(r.callback=a),t=Xn(e,r,n),t!==null&&(Wt(t,e,n),ro(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=Zt(),r=Jn(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=Xn(e,r,n),t!==null&&(Wt(t,e,n),ro(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=Zt(),n=Jn(a);n.tag=2,t!=null&&(n.callback=t),t=Xn(e,n,a),t!==null&&(Wt(t,e,a),ro(t,e,a))}};function wg(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!go(a,n)||!go(r,s):!0}function Sg(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&Vm.enqueueReplaceState(t,t.state,null)}function Or(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=Ae({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var Tu=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function Mb(e){Tu(e)}function Ob(e){console.error(e)}function Lb(e){Tu(e)}function Au(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function Ng(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function Gm(e,t,a){return a=Jn(a),a.tag=3,a.payload={element:null},a.callback=function(){Au(e,t)},a}function Pb(e){return e=Jn(e),e.tag=3,e}function Ub(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){Ng(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){Ng(t,a,n),typeof r!="function"&&(Zn===null?Zn=new Set([this]):Zn.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function oE(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&Lo(t,a,r,!0),a=ga.current,a!==null){switch(a.tag){case 13:return Qa===null?af():a.alternate===null&&qe===0&&(qe=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===Bm?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),pm(e,n,r)),!1;case 22:return a.flags|=65536,n===Bm?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),pm(e,n,r)),!1}throw Error(j(435,a.tag))}return pm(e,n,r),af(),!1}if(ve)return t=ga.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==Lm&&(e=Error(j(422),{cause:n}),yo(ha(e,a)))):(n!==Lm&&(t=Error(j(423),{cause:n}),yo(ha(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=ha(n,a),r=Gm(e.stateNode,n,r),tm(e,r),qe!==4&&(qe=2)),!1;var s=Error(j(520),{cause:n});if(s=ha(s,a),co===null?co=[s]:co.push(s),qe!==4&&(qe=2),t===null)return!0;n=ha(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=Gm(a.stateNode,n,e),tm(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(Zn===null||!Zn.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=Pb(r),Ub(r,e,a,n),tm(a,r),!1}a=a.return}while(a!==null);return!1}var jb=Error(j(461)),ct=!1;function pt(e,t,a,n){t.child=e===null?Ab(t,null,a,n):js(t,e.child,a,n)}function _g(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return Dr(t),n=Uf(e,t,a,i,s,r),o=jf(),e!==null&&!ct?(Ff(e,t,r),bn(e,t,r)):(ve&&o&&Tf(t),t.flags|=1,pt(e,t,n,r),t.child)}function kg(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!Ef(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,Fb(e,t,s,n,r)):(e=iu(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!Gf(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:go,a(i,n)&&e.ref===t.ref)return bn(e,t,r)}return t.flags|=1,e=hn(s,n),e.ref=t.ref,e.return=t,t.child=e}function Fb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if(go(s,n)&&e.ref===t.ref)if(ct=!1,t.pendingProps=n=s,Gf(e,r))(e.flags&131072)!==0&&(ct=!0);else return t.lanes=e.lanes,bn(e,t,r)}return Ym(e,t,a,n,r)}function Bb(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return Rg(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&ou(t,s!==null?s.cachePool:null),s!==null?pg(t,s):Im(),Db(t);else return t.lanes=t.childLanes=536870912,Rg(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(ou(t,s.cachePool),pg(t,s),In(t),t.memoizedState=null):(e!==null&&ou(t,null),Im(),In(t));return pt(e,t,r,a),t.child}function Rg(e,t,a,n){var r=Mf();return r=r===null?null:{parent:at._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&ou(t,null),Im(),Db(t),e!==null&&Lo(e,t,n,!0),null}function cu(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(j(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function Ym(e,t,a,n,r){return Dr(t),a=Uf(e,t,a,n,void 0,r),n=jf(),e!==null&&!ct?(Ff(e,t,r),bn(e,t,r)):(ve&&n&&Tf(t),t.flags|=1,pt(e,t,a,r),t.child)}function Cg(e,t,a,n,r,s){return Dr(t),t.updateQueue=null,a=Wy(t,n,a,r),Zy(e),n=jf(),e!==null&&!ct?(Ff(e,t,s),bn(e,t,s)):(ve&&n&&Tf(t),t.flags|=1,pt(e,t,a,s),t.child)}function Eg(e,t,a,n,r){if(Dr(t),t.stateNode===null){var s=xs,i=a.contextType;typeof i=="object"&&i!==null&&(s=xt(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=Vm,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},Of(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?xt(i):xs,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(nm(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&Vm.enqueueReplaceState(s,s.state,null),io(t,n,s,r),so(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,u=Or(a,o);s.props=u;var c=s.context,d=a.contextType;i=xs,typeof d=="object"&&d!==null&&(i=xt(d));var m=a.getDerivedStateFromProps;d=typeof m=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&Sg(t,s,n,i),Bn=!1;var f=t.memoizedState;s.state=f,io(t,n,s,r),so(),c=t.memoizedState,o||f!==c||Bn?(typeof m=="function"&&(nm(t,a,m,n),c=t.memoizedState),(u=Bn||wg(t,a,u,n,f,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=u):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,zm(e,t),i=t.memoizedProps,d=Or(a,i),s.props=d,m=t.pendingProps,f=s.context,c=a.contextType,u=xs,typeof c=="object"&&c!==null&&(u=xt(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==m||f!==u)&&Sg(t,s,n,u),Bn=!1,f=t.memoizedState,s.state=f,io(t,n,s,r),so();var h=t.memoizedState;i!==m||f!==h||Bn||e!==null&&e.dependencies!==null&&Nu(e.dependencies)?(typeof o=="function"&&(nm(t,a,o,n),h=t.memoizedState),(d=Bn||wg(t,a,d,n,f,h,u)||e!==null&&e.dependencies!==null&&Nu(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,h,u),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,h,u)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=h),s.props=n,s.state=h,s.context=u,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,cu(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=js(t,e.child,null,r),t.child=js(t,null,a,r)):pt(e,t,a,r),t.memoizedState=s.state,e=t.child):e=bn(e,t,r),e}function Tg(e,t,a,n){return Oo(),t.flags|=256,pt(e,t,a,n),t.child}var rm={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function sm(e){return{baseLanes:e,cachePool:Vy()}}function im(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=va),e}function zb(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(nt.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(ve){if(r?qn(t):In(t),ve){var o=ze,u;if(u=o){e:{for(u=o,o=qa;u.nodeType!==8;){if(!o){o=null;break e}if(u=ka(u.nextSibling),u===null){o=null;break e}}o=u}o!==null?(t.memoizedState={dehydrated:o,treeContext:kr!==null?{id:dn,overflow:mn}:null,retryLane:536870912,hydrationErrors:null},u=Yt(18,null,null,0),u.stateNode=o,u.return=t,t.child=u,Rt=t,ze=null,u=!0):u=!1}u||Ar(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return cf(o)?t.lanes=32:t.lanes=536870912,null;pn(t)}return o=n.children,n=n.fallback,r?(In(t),r=t.mode,o=Du({mode:"hidden",children:o},r),n=_r(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=sm(a),r.childLanes=im(e,i,a),t.memoizedState=rm,n):(qn(t),Jm(t,o))}if(u=e.memoizedState,u!==null&&(o=u.dehydrated,o!==null)){if(s)t.flags&256?(qn(t),t.flags&=-257,t=om(e,t,a)):t.memoizedState!==null?(In(t),t.child=e.child,t.flags|=128,t=null):(In(t),r=n.fallback,o=t.mode,n=Du({mode:"visible",children:n.children},o),r=_r(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,js(t,e.child,null,a),n=t.child,n.memoizedState=sm(a),n.childLanes=im(e,i,a),t.memoizedState=rm,t=r);else if(qn(t),cf(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(j(419)),n.stack="",n.digest=i,yo({value:n,source:null,stack:null}),t=om(e,t,a)}else if(ct||Lo(e,t,a,!1),i=(a&e.childLanes)!==0,ct||i){if(i=Ce,i!==null&&(n=a&-a,n=(n&42)!==0?1:yf(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==u.retryLane))throw u.retryLane=n,Hs(e,n),Wt(i,e,n),jb;o.data==="$?"||af(),t=om(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=u.treeContext,ze=ka(o.nextSibling),Rt=t,ve=!0,Rr=null,qa=!1,e!==null&&(ma[fa++]=dn,ma[fa++]=mn,ma[fa++]=kr,dn=e.id,mn=e.overflow,kr=t),t=Jm(t,n.children),t.flags|=4096);return t}return r?(In(t),r=n.fallback,o=t.mode,u=e.child,c=u.sibling,n=hn(u,{mode:"hidden",children:n.children}),n.subtreeFlags=u.subtreeFlags&65011712,c!==null?r=hn(c,r):(r=_r(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=sm(a):(u=o.cachePool,u!==null?(c=at._currentValue,u=u.parent!==c?{parent:c,pool:c}:u):u=Vy(),o={baseLanes:o.baseLanes|a,cachePool:u}),r.memoizedState=o,r.childLanes=im(e,i,a),t.memoizedState=rm,n):(qn(t),a=e.child,e=a.sibling,a=hn(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function Jm(e,t){return t=Du({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function Du(e,t){return e=Yt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function om(e,t,a){return js(t,e.child,null,a),e=Jm(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function Ag(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),Um(e.return,t,a)}function lm(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function qb(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(pt(e,t,n.children,a),n=nt.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&Ag(e,a,t);else if(e.tag===19)Ag(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Le(nt,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&Eu(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),lm(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&Eu(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}lm(t,!0,a,null,s);break;case"together":lm(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function bn(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),sr|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(Lo(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(j(153));if(t.child!==null){for(e=t.child,a=hn(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=hn(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function Gf(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&Nu(e)))}function lE(e,t,a){switch(t.tag){case 3:vu(t,t.stateNode.containerInfo),zn(t,at,e.memoizedState.cache),Oo();break;case 27:case 5:_m(t);break;case 4:vu(t,t.stateNode.containerInfo);break;case 10:zn(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(qn(t),t.flags|=128,null):(a&t.child.childLanes)!==0?zb(e,t,a):(qn(t),e=bn(e,t,a),e!==null?e.sibling:null);qn(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(Lo(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return qb(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Le(nt,nt.current),n)break;return null;case 22:case 23:return t.lanes=0,Bb(e,t,a);case 24:zn(t,at,e.memoizedState.cache)}return bn(e,t,a)}function Ib(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)ct=!0;else{if(!Gf(e,a)&&(t.flags&128)===0)return ct=!1,lE(e,t,a);ct=(e.flags&131072)!==0}else ct=!1,ve&&(t.flags&1048576)!==0&&Hy(t,Su,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")Ef(n)?(e=Or(n,e),t.tag=1,t=Eg(null,t,n,e,a)):(t.tag=0,t=Ym(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===hf){t.tag=11,t=_g(null,t,n,e,a);break e}else if(r===vf){t.tag=14,t=kg(null,t,n,e,a);break e}}throw t=Sm(n)||n,Error(j(306,t,""))}}return t;case 0:return Ym(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Or(n,t.pendingProps),Eg(e,t,n,r,a);case 3:e:{if(vu(t,t.stateNode.containerInfo),e===null)throw Error(j(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,zm(e,t),io(t,n,null,a);var i=t.memoizedState;if(n=i.cache,zn(t,at,n),n!==s.cache&&jm(t,[at],a,!0),so(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=Tg(e,t,n,a);break e}else if(n!==r){r=ha(Error(j(424)),t),yo(r),t=Tg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(ze=ka(e.firstChild),Rt=t,ve=!0,Rr=null,qa=!0,a=Ab(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if(Oo(),n===r){t=bn(e,t,a);break e}pt(e,t,n,a)}t=t.child}return t;case 26:return cu(e,t),e===null?(a=Jg(t.type,null,t.pendingProps,null))?t.memoizedState=a:ve||(a=t.type,e=t.pendingProps,n=Fu(Yn.current).createElement(a),n[bt]=t,n[Ut]=e,vt(n,a,e),ut(n),t.stateNode=n):t.memoizedState=Jg(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return _m(t),e===null&&ve&&(n=t.stateNode=E0(t.type,t.pendingProps,Yn.current),Rt=t,qa=!0,r=ze,or(t.type)?(df=r,ze=ka(n.firstChild)):ze=r),pt(e,t,t.pendingProps.children,a),cu(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&ve&&((r=n=ze)&&(n=OE(n,t.type,t.pendingProps,qa),n!==null?(t.stateNode=n,Rt=t,ze=ka(n.firstChild),qa=!1,r=!0):r=!1),r||Ar(t)),_m(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,lf(r,s)?n=null:i!==null&&lf(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=Uf(e,t,eE,null,null,a),No._currentValue=r),cu(e,t),pt(e,t,n,a),t.child;case 6:return e===null&&ve&&((e=a=ze)&&(a=LE(a,t.pendingProps,qa),a!==null?(t.stateNode=a,Rt=t,ze=null,e=!0):e=!1),e||Ar(t)),null;case 13:return zb(e,t,a);case 4:return vu(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=js(t,null,n,a):pt(e,t,n,a),t.child;case 11:return _g(e,t,t.type,t.pendingProps,a);case 7:return pt(e,t,t.pendingProps,a),t.child;case 8:return pt(e,t,t.pendingProps.children,a),t.child;case 12:return pt(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,zn(t,t.type,n.value),pt(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,Dr(t),r=xt(r),n=n(r),t.flags|=1,pt(e,t,n,a),t.child;case 14:return kg(e,t,t.type,t.pendingProps,a);case 15:return Fb(e,t,t.type,t.pendingProps,a);case 19:return qb(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=Du(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=hn(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return Bb(e,t,a);case 24:return Dr(t),n=xt(at),e===null?(r=Mf(),r===null&&(r=Ce,s=Df(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},Of(t),zn(t,at,r)):((e.lanes&a)!==0&&(zm(e,t),io(t,null,null,a),so()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),zn(t,at,n)):(n=s.cache,zn(t,at,n),n!==r.cache&&jm(t,[at],a,!0))),pt(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(j(156,t.tag))}function on(e){e.flags|=4}function Dg(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!D0(t)){if(t=ga.current,t!==null&&((me&4194048)===me?Qa!==null:(me&62914560)!==me&&(me&536870912)===0||t!==Qa))throw no=Bm,Gy;e.flags|=8192}}function Jl(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?hy():536870912,e.lanes|=t,Fs|=t)}function Hi(e,t){if(!ve)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function Fe(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function uE(e,t,a){var n=t.pendingProps;switch(Af(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return Fe(t),null;case 1:return Fe(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),vn(at),Ds(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Ii(t)?on(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,lg())),Fe(t),null;case 26:return a=t.memoizedState,e===null?(on(t),a!==null?(Fe(t),Dg(t,a)):(Fe(t),t.flags&=-16777217)):a?a!==e.memoizedState?(on(t),Fe(t),Dg(t,a)):(Fe(t),t.flags&=-16777217):(e.memoizedProps!==n&&on(t),Fe(t),t.flags&=-16777217),null;case 27:gu(t),a=Yn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&on(t);else{if(!n){if(t.stateNode===null)throw Error(j(166));return Fe(t),null}e=Ka.current,Ii(t)?ig(t,e):(e=E0(r,n,a),t.stateNode=e,on(t))}return Fe(t),null;case 5:if(gu(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&on(t);else{if(!n){if(t.stateNode===null)throw Error(j(166));return Fe(t),null}if(e=Ka.current,Ii(t))ig(t,e);else{switch(r=Fu(Yn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[bt]=t,e[Ut]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(vt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&on(t)}}return Fe(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&on(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(j(166));if(e=Yn.current,Ii(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=Rt,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[bt]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||k0(e.nodeValue,a)),e||Ar(t)}else e=Fu(e).createTextNode(n),e[bt]=t,t.stateNode=e}return Fe(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Ii(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(j(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(j(317));r[bt]=t}else Oo(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;Fe(t),r=!1}else r=lg(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(pn(t),t):(pn(t),null)}if(pn(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),Jl(t,t.updateQueue),Fe(t),null;case 4:return Ds(),e===null&&ap(t.stateNode.containerInfo),Fe(t),null;case 10:return vn(t.type),Fe(t),null;case 19:if(dt(nt),r=t.memoizedState,r===null)return Fe(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)Hi(r,!1);else{if(qe!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=Eu(e),s!==null){for(t.flags|=128,Hi(r,!1),e=s.updateQueue,t.updateQueue=e,Jl(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)Ky(a,e),a=a.sibling;return Le(nt,nt.current&1|2),t.child}e=e.sibling}r.tail!==null&&Ha()>Ou&&(t.flags|=128,n=!0,Hi(r,!1),t.lanes=4194304)}else{if(!n)if(e=Eu(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,Jl(t,e),Hi(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!ve)return Fe(t),null}else 2*Ha()-r.renderingStartTime>Ou&&a!==536870912&&(t.flags|=128,n=!0,Hi(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=Ha(),t.sibling=null,e=nt.current,Le(nt,n?e&1|2:e&1),t):(Fe(t),null);case 22:case 23:return pn(t),Lf(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(Fe(t),t.subtreeFlags&6&&(t.flags|=8192)):Fe(t),a=t.updateQueue,a!==null&&Jl(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&dt(Cr),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),vn(at),Fe(t),null;case 25:return null;case 30:return null}throw Error(j(156,t.tag))}function cE(e,t){switch(Af(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return vn(at),Ds(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return gu(t),null;case 13:if(pn(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(j(340));Oo()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return dt(nt),null;case 4:return Ds(),null;case 10:return vn(t.type),null;case 22:case 23:return pn(t),Lf(),e!==null&&dt(Cr),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return vn(at),null;case 25:return null;default:return null}}function Kb(e,t){switch(Af(t),t.tag){case 3:vn(at),Ds();break;case 26:case 27:case 5:gu(t);break;case 4:Ds();break;case 13:pn(t);break;case 19:dt(nt);break;case 10:vn(t.type);break;case 22:case 23:pn(t),Lf(),e!==null&&dt(Cr);break;case 24:vn(at)}}function Bo(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){Re(t,t.return,o)}}function rr(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var u=a,c=o;try{c()}catch(d){Re(r,u,d)}}}n=n.next}while(n!==s)}}catch(d){Re(t,t.return,d)}}function Hb(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{Xy(t,a)}catch(n){Re(e,e.return,n)}}}function Qb(e,t,a){a.props=Or(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){Re(e,t,n)}}function lo(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){Re(e,t,r)}}function Ia(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){Re(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){Re(e,t,r)}else a.current=null}function Vb(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){Re(e,e.return,r)}}function um(e,t,a){try{var n=e.stateNode;EE(n,e.type,a,t),n[Ut]=t}catch(r){Re(e,e.return,r)}}function Gb(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&or(e.type)||e.tag===4}function cm(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||Gb(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&or(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function Xm(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=rc));else if(n!==4&&(n===27&&or(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(Xm(e,t,a),e=e.sibling;e!==null;)Xm(e,t,a),e=e.sibling}function Mu(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&or(e.type)&&(a=e.stateNode),e=e.child,e!==null))for(Mu(e,t,a),e=e.sibling;e!==null;)Mu(e,t,a),e=e.sibling}function Yb(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);vt(t,n,a),t[bt]=e,t[Ut]=a}catch(s){Re(e,e.return,s)}}var un=!1,Ve=!1,dm=!1,Mg=typeof WeakSet=="function"?WeakSet:Set,lt=null;function dE(e,t){if(e=e.containerInfo,sf=Iu,e=Py(e),kf(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,u=-1,c=0,d=0,m=e,f=null;t:for(;;){for(var h;m!==a||r!==0&&m.nodeType!==3||(o=i+r),m!==s||n!==0&&m.nodeType!==3||(u=i+n),m.nodeType===3&&(i+=m.nodeValue.length),(h=m.firstChild)!==null;)f=m,m=h;for(;;){if(m===e)break t;if(f===a&&++c===r&&(o=i),f===s&&++d===n&&(u=i),(h=m.nextSibling)!==null)break;m=f,f=m.parentNode}m=h}a=o===-1||u===-1?null:{start:o,end:u}}else a=null}a=a||{start:0,end:0}}else a=null;for(of={focusedElem:e,selectionRange:a},Iu=!1,lt=t;lt!==null;)if(t=lt,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,lt=e;else for(;lt!==null;){switch(t=lt,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var x=Or(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(x,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(y){Re(a,a.return,y)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)uf(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":uf(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(j(163))}if(e=t.sibling,e!==null){e.return=t.return,lt=e;break}lt=t.return}}function Jb(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:Un(e,a),n&4&&Bo(5,a);break;case 1:if(Un(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){Re(a,a.return,i)}else{var r=Or(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){Re(a,a.return,i)}}n&64&&Hb(a),n&512&&lo(a,a.return);break;case 3:if(Un(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{Xy(e,t)}catch(i){Re(a,a.return,i)}}break;case 27:t===null&&n&4&&Yb(a);case 26:case 5:Un(e,a),t===null&&n&4&&Vb(a),n&512&&lo(a,a.return);break;case 12:Un(e,a);break;case 13:Un(e,a),n&4&&Wb(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=xE.bind(null,a),PE(e,a))));break;case 22:if(n=a.memoizedState!==null||un,!n){t=t!==null&&t.memoizedState!==null||Ve,r=un;var s=Ve;un=n,(Ve=t)&&!s?jn(e,a,(a.subtreeFlags&8772)!==0):Un(e,a),un=r,Ve=s}break;case 30:break;default:Un(e,a)}}function Xb(e){var t=e.alternate;t!==null&&(e.alternate=null,Xb(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&xf(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var Oe=null,Lt=!1;function ln(e,t,a){for(a=a.child;a!==null;)Zb(e,t,a),a=a.sibling}function Zb(e,t,a){if(Jt&&typeof Jt.onCommitFiberUnmount=="function")try{Jt.onCommitFiberUnmount(Eo,a)}catch{}switch(a.tag){case 26:Ve||Ia(a,t),ln(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:Ve||Ia(a,t);var n=Oe,r=Lt;or(a.type)&&(Oe=a.stateNode,Lt=!1),ln(e,t,a),fo(a.stateNode),Oe=n,Lt=r;break;case 5:Ve||Ia(a,t);case 6:if(n=Oe,r=Lt,Oe=null,ln(e,t,a),Oe=n,Lt=r,Oe!==null)if(Lt)try{(Oe.nodeType===9?Oe.body:Oe.nodeName==="HTML"?Oe.ownerDocument.body:Oe).removeChild(a.stateNode)}catch(s){Re(a,t,s)}else try{Oe.removeChild(a.stateNode)}catch(s){Re(a,t,s)}break;case 18:Oe!==null&&(Lt?(e=Oe,Vg(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),Ro(e)):Vg(Oe,a.stateNode));break;case 4:n=Oe,r=Lt,Oe=a.stateNode.containerInfo,Lt=!0,ln(e,t,a),Oe=n,Lt=r;break;case 0:case 11:case 14:case 15:Ve||rr(2,a,t),Ve||rr(4,a,t),ln(e,t,a);break;case 1:Ve||(Ia(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&Qb(a,t,n)),ln(e,t,a);break;case 21:ln(e,t,a);break;case 22:Ve=(n=Ve)||a.memoizedState!==null,ln(e,t,a),Ve=n;break;default:ln(e,t,a)}}function Wb(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{Ro(e)}catch(a){Re(t,t.return,a)}}function mE(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new Mg),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new Mg),t;default:throw Error(j(435,e.tag))}}function mm(e,t){var a=mE(e);t.forEach(function(n){var r=$E.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function Qt(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(or(o.type)){Oe=o.stateNode,Lt=!1;break e}break;case 5:Oe=o.stateNode,Lt=!1;break e;case 3:case 4:Oe=o.stateNode.containerInfo,Lt=!0;break e}o=o.return}if(Oe===null)throw Error(j(160));Zb(s,i,r),Oe=null,Lt=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)e0(t,e),t=t.sibling}var _a=null;function e0(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:Qt(t,e),Vt(e),n&4&&(rr(3,e,e.return),Bo(3,e),rr(5,e,e.return));break;case 1:Qt(t,e),Vt(e),n&512&&(Ve||a===null||Ia(a,a.return)),n&64&&un&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=_a;if(Qt(t,e),Vt(e),n&512&&(Ve||a===null||Ia(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[Do]||s[bt]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),vt(s,n,a),s[bt]=e,ut(s),n=s;break e;case"link":var i=Zg("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),vt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=Zg("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),vt(s,n,a),r.head.appendChild(s);break;default:throw Error(j(468,n))}s[bt]=e,ut(s),n=s}e.stateNode=n}else Wg(r,e.type,e.stateNode);else e.stateNode=Xg(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?Wg(r,e.type,e.stateNode):Xg(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&um(e,e.memoizedProps,a.memoizedProps)}break;case 27:Qt(t,e),Vt(e),n&512&&(Ve||a===null||Ia(a,a.return)),a!==null&&n&4&&um(e,e.memoizedProps,a.memoizedProps);break;case 5:if(Qt(t,e),Vt(e),n&512&&(Ve||a===null||Ia(a,a.return)),e.flags&32){r=e.stateNode;try{Os(r,"")}catch(h){Re(e,e.return,h)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,um(e,r,a!==null?a.memoizedProps:r)),n&1024&&(dm=!0);break;case 6:if(Qt(t,e),Vt(e),n&4){if(e.stateNode===null)throw Error(j(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(h){Re(e,e.return,h)}}break;case 3:if(fu=null,r=_a,_a=Bu(t.containerInfo),Qt(t,e),_a=r,Vt(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{Ro(t.containerInfo)}catch(h){Re(e,e.return,h)}dm&&(dm=!1,t0(e));break;case 4:n=_a,_a=Bu(e.stateNode.containerInfo),Qt(t,e),Vt(e),_a=n;break;case 12:Qt(t,e),Vt(e);break;case 13:Qt(t,e),Vt(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(Wf=Ha()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,mm(e,n)));break;case 22:r=e.memoizedState!==null;var u=a!==null&&a.memoizedState!==null,c=un,d=Ve;if(un=c||r,Ve=d||u,Qt(t,e),Ve=d,un=c,Vt(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||u||un||Ve||Sr(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){u=a=t;try{if(s=u.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=u.stateNode;var m=u.memoizedProps.style,f=m!=null&&m.hasOwnProperty("display")?m.display:null;o.style.display=f==null||typeof f=="boolean"?"":(""+f).trim()}}catch(h){Re(u,u.return,h)}}}else if(t.tag===6){if(a===null){u=t;try{u.stateNode.nodeValue=r?"":u.memoizedProps}catch(h){Re(u,u.return,h)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,mm(e,a))));break;case 19:Qt(t,e),Vt(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,mm(e,n)));break;case 30:break;case 21:break;default:Qt(t,e),Vt(e)}}function Vt(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(Gb(n)){a=n;break}n=n.return}if(a==null)throw Error(j(160));switch(a.tag){case 27:var r=a.stateNode,s=cm(e);Mu(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(Os(i,""),a.flags&=-33);var o=cm(e);Mu(e,o,i);break;case 3:case 4:var u=a.stateNode.containerInfo,c=cm(e);Xm(e,c,u);break;default:throw Error(j(161))}}catch(d){Re(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function t0(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;t0(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function Un(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)Jb(e,t.alternate,t),t=t.sibling}function Sr(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:rr(4,t,t.return),Sr(t);break;case 1:Ia(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&Qb(t,t.return,a),Sr(t);break;case 27:fo(t.stateNode);case 26:case 5:Ia(t,t.return),Sr(t);break;case 22:t.memoizedState===null&&Sr(t);break;case 30:Sr(t);break;default:Sr(t)}e=e.sibling}}function jn(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:jn(r,s,a),Bo(4,s);break;case 1:if(jn(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){Re(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var u=r.shared.hiddenCallbacks;if(u!==null)for(r.shared.hiddenCallbacks=null,r=0;r<u.length;r++)Jy(u[r],o)}catch(c){Re(n,n.return,c)}}a&&i&64&&Hb(s),lo(s,s.return);break;case 27:Yb(s);case 26:case 5:jn(r,s,a),a&&n===null&&i&4&&Vb(s),lo(s,s.return);break;case 12:jn(r,s,a);break;case 13:jn(r,s,a),a&&i&4&&Wb(r,s);break;case 22:s.memoizedState===null&&jn(r,s,a),lo(s,s.return);break;case 30:break;default:jn(r,s,a)}t=t.sibling}}function Yf(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&Po(a))}function Jf(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&Po(e))}function za(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)a0(e,t,a,n),t=t.sibling}function a0(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:za(e,t,a,n),r&2048&&Bo(9,t);break;case 1:za(e,t,a,n);break;case 3:za(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&Po(e)));break;case 12:if(r&2048){za(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(u){Re(t,t.return,u)}}else za(e,t,a,n);break;case 13:za(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?za(e,t,a,n):uo(e,t):s._visibility&2?za(e,t,a,n):(s._visibility|=2,cs(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&Yf(i,t);break;case 24:za(e,t,a,n),r&2048&&Jf(t.alternate,t);break;default:za(e,t,a,n)}}function cs(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,u=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:cs(s,i,o,u,r),Bo(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?cs(s,i,o,u,r):uo(s,i):(d._visibility|=2,cs(s,i,o,u,r)),r&&c&2048&&Yf(i.alternate,i);break;case 24:cs(s,i,o,u,r),r&&c&2048&&Jf(i.alternate,i);break;default:cs(s,i,o,u,r)}t=t.sibling}}function uo(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:uo(a,n),r&2048&&Yf(n.alternate,n);break;case 24:uo(a,n),r&2048&&Jf(n.alternate,n);break;default:uo(a,n)}t=t.sibling}}var Zi=8192;function os(e){if(e.subtreeFlags&Zi)for(e=e.child;e!==null;)n0(e),e=e.sibling}function n0(e){switch(e.tag){case 26:os(e),e.flags&Zi&&e.memoizedState!==null&&YE(_a,e.memoizedState,e.memoizedProps);break;case 5:os(e);break;case 3:case 4:var t=_a;_a=Bu(e.stateNode.containerInfo),os(e),_a=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=Zi,Zi=16777216,os(e),Zi=t):os(e));break;default:os(e)}}function r0(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function Qi(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];lt=n,i0(n,e)}r0(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)s0(e),e=e.sibling}function s0(e){switch(e.tag){case 0:case 11:case 15:Qi(e),e.flags&2048&&rr(9,e,e.return);break;case 3:Qi(e);break;case 12:Qi(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,du(e)):Qi(e);break;default:Qi(e)}}function du(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];lt=n,i0(n,e)}r0(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:rr(8,t,t.return),du(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,du(t));break;default:du(t)}e=e.sibling}}function i0(e,t){for(;lt!==null;){var a=lt;switch(a.tag){case 0:case 11:case 15:rr(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:Po(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,lt=n;else e:for(a=e;lt!==null;){n=lt;var r=n.sibling,s=n.return;if(Xb(n),n===a){lt=null;break e}if(r!==null){r.return=s,lt=r;break e}lt=s}}}var fE={getCacheForType:function(e){var t=xt(at),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},pE=typeof WeakMap=="function"?WeakMap:Map,Ne=0,Ce=null,ue=null,me=0,Se=0,Gt=null,Vn=!1,Qs=!1,Xf=!1,xn=0,qe=0,sr=0,Er=0,Zf=0,va=0,Fs=0,co=null,Pt=null,Zm=!1,Wf=0,Ou=1/0,Lu=null,Zn=null,ht=0,Wn=null,Bs=null,As=0,Wm=0,ef=null,o0=null,mo=0,tf=null;function Zt(){if((Ne&2)!==0&&me!==0)return me&-me;if(ne.T!==null){var e=Ls;return e!==0?e:tp()}return yy()}function l0(){va===0&&(va=(me&536870912)===0||ve?py():536870912);var e=ga.current;return e!==null&&(e.flags|=32),va}function Wt(e,t,a){(e===Ce&&(Se===2||Se===9)||e.cancelPendingCommit!==null)&&(zs(e,0),Gn(e,me,va,!1)),Ao(e,a),((Ne&2)===0||e!==Ce)&&(e===Ce&&((Ne&2)===0&&(Er|=a),qe===4&&Gn(e,me,va,!1)),Ga(e))}function u0(e,t,a){if((Ne&6)!==0)throw Error(j(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||To(e,t),r=n?gE(e,t):fm(e,t,!0),s=n;do{if(r===0){Qs&&!n&&Gn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!hE(a)){r=fm(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=co;var u=o.current.memoizedState.isDehydrated;if(u&&(zs(o,i).flags|=256),i=fm(o,i,!1),i!==2){if(Xf&&!u){o.errorRecoveryDisabledLanes|=s,Er|=s,r=4;break e}s=Pt,Pt=r,s!==null&&(Pt===null?Pt=s:Pt.push.apply(Pt,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){zs(e,0),Gn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(j(345));case 4:if((t&4194048)!==t)break;case 6:Gn(n,t,va,!Vn);break e;case 2:Pt=null;break;case 3:case 5:break;default:throw Error(j(329))}if((t&62914560)===t&&(r=Wf+300-Ha(),10<r)){if(Gn(n,t,va,!Vn),Hu(n,0,!0)!==0)break e;n.timeoutHandle=C0(Og.bind(null,n,a,Pt,Lu,Zm,t,va,Er,Fs,Vn,s,2,-0,0),r);break e}Og(n,a,Pt,Lu,Zm,t,va,Er,Fs,Vn,s,0,-0,0)}}break}while(!0);Ga(e)}function Og(e,t,a,n,r,s,i,o,u,c,d,m,f,h){if(e.timeoutHandle=-1,m=t.subtreeFlags,(m&8192||(m&16785408)===16785408)&&(So={stylesheets:null,count:0,unsuspend:GE},n0(t),m=JE(),m!==null)){e.cancelPendingCommit=m(Pg.bind(null,e,t,s,a,n,r,i,o,u,d,1,f,h)),Gn(e,s,i,!c);return}Pg(e,t,s,a,n,r,i,o,u)}function hE(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!ea(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function Gn(e,t,a,n){t&=~Zf,t&=~Er,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Xt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&vy(e,a,t)}function tc(){return(Ne&6)===0?(zo(0,!1),!1):!0}function ep(){if(ue!==null){if(Se===0)var e=ue.return;else e=ue,fn=jr=null,Bf(e),Ts=null,xo=0,e=ue;for(;e!==null;)Kb(e.alternate,e),e=e.return;ue=null}}function zs(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,AE(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),ep(),Ce=e,ue=a=hn(e.current,null),me=t,Se=0,Gt=null,Vn=!1,Qs=To(e,t),Xf=!1,Fs=va=Zf=Er=sr=qe=0,Pt=co=null,Zm=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Xt(n),s=1<<r;t|=e[r],n&=~s}return xn=t,Yu(),a}function c0(e,t){oe=null,ne.H=Cu,t===Uo||t===Xu?(t=mg(),Se=3):t===Gy?(t=mg(),Se=4):Se=t===jb?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Gt=t,ue===null&&(qe=1,Au(e,ha(t,e.current)))}function d0(){var e=ne.H;return ne.H=Cu,e===null?Cu:e}function m0(){var e=ne.A;return ne.A=fE,e}function af(){qe=4,Vn||(me&4194048)!==me&&ga.current!==null||(Qs=!0),(sr&134217727)===0&&(Er&134217727)===0||Ce===null||Gn(Ce,me,va,!1)}function fm(e,t,a){var n=Ne;Ne|=2;var r=d0(),s=m0();(Ce!==e||me!==t)&&(Lu=null,zs(e,t)),t=!1;var i=qe;e:do try{if(Se!==0&&ue!==null){var o=ue,u=Gt;switch(Se){case 8:ep(),i=6;break e;case 3:case 2:case 9:case 6:ga.current===null&&(t=!0);var c=Se;if(Se=0,Gt=null,Ss(e,o,u,c),a&&Qs){i=0;break e}break;default:c=Se,Se=0,Gt=null,Ss(e,o,u,c)}}vE(),i=qe;break}catch(d){c0(e,d)}while(!0);return t&&e.shellSuspendCounter++,fn=jr=null,Ne=n,ne.H=r,ne.A=s,ue===null&&(Ce=null,me=0,Yu()),i}function vE(){for(;ue!==null;)f0(ue)}function gE(e,t){var a=Ne;Ne|=2;var n=d0(),r=m0();Ce!==e||me!==t?(Lu=null,Ou=Ha()+500,zs(e,t)):Qs=To(e,t);e:do try{if(Se!==0&&ue!==null){t=ue;var s=Gt;t:switch(Se){case 1:Se=0,Gt=null,Ss(e,t,s,1);break;case 2:case 9:if(dg(s)){Se=0,Gt=null,Lg(t);break}t=function(){Se!==2&&Se!==9||Ce!==e||(Se=7),Ga(e)},s.then(t,t);break e;case 3:Se=7;break e;case 4:Se=5;break e;case 7:dg(s)?(Se=0,Gt=null,Lg(t)):(Se=0,Gt=null,Ss(e,t,s,7));break;case 5:var i=null;switch(ue.tag){case 26:i=ue.memoizedState;case 5:case 27:var o=ue;if(!i||D0(i)){Se=0,Gt=null;var u=o.sibling;if(u!==null)ue=u;else{var c=o.return;c!==null?(ue=c,ac(c)):ue=null}break t}}Se=0,Gt=null,Ss(e,t,s,5);break;case 6:Se=0,Gt=null,Ss(e,t,s,6);break;case 8:ep(),qe=6;break e;default:throw Error(j(462))}}yE();break}catch(d){c0(e,d)}while(!0);return fn=jr=null,ne.H=n,ne.A=r,Ne=a,ue!==null?0:(Ce=null,me=0,Yu(),qe)}function yE(){for(;ue!==null&&!BR();)f0(ue)}function f0(e){var t=Ib(e.alternate,e,xn);e.memoizedProps=e.pendingProps,t===null?ac(e):ue=t}function Lg(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=Cg(a,t,t.pendingProps,t.type,void 0,me);break;case 11:t=Cg(a,t,t.pendingProps,t.type.render,t.ref,me);break;case 5:Bf(t);default:Kb(a,t),t=ue=Ky(t,xn),t=Ib(a,t,xn)}e.memoizedProps=e.pendingProps,t===null?ac(e):ue=t}function Ss(e,t,a,n){fn=jr=null,Bf(t),Ts=null,xo=0;var r=t.return;try{if(oE(e,r,t,a,me)){qe=1,Au(e,ha(a,e.current)),ue=null;return}}catch(s){if(r!==null)throw ue=r,s;qe=1,Au(e,ha(a,e.current)),ue=null;return}t.flags&32768?(ve||n===1?e=!0:Qs||(me&536870912)!==0?e=!1:(Vn=e=!0,(n===2||n===9||n===3||n===6)&&(n=ga.current,n!==null&&n.tag===13&&(n.flags|=16384))),p0(t,e)):ac(t)}function ac(e){var t=e;do{if((t.flags&32768)!==0){p0(t,Vn);return}e=t.return;var a=uE(t.alternate,t,xn);if(a!==null){ue=a;return}if(t=t.sibling,t!==null){ue=t;return}ue=t=e}while(t!==null);qe===0&&(qe=5)}function p0(e,t){do{var a=cE(e.alternate,e);if(a!==null){a.flags&=32767,ue=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){ue=e;return}ue=e=a}while(e!==null);qe=6,ue=null}function Pg(e,t,a,n,r,s,i,o,u){e.cancelPendingCommit=null;do nc();while(ht!==0);if((Ne&6)!==0)throw Error(j(327));if(t!==null){if(t===e.current)throw Error(j(177));if(s=t.lanes|t.childLanes,s|=Rf,JR(e,a,s,i,o,u),e===Ce&&(ue=Ce=null,me=0),Bs=t,Wn=e,As=a,Wm=s,ef=r,o0=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,wE(yu,function(){return b0(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=ne.T,ne.T=null,r=ge.p,ge.p=2,i=Ne,Ne|=4;try{dE(e,t,a)}finally{Ne=i,ge.p=r,ne.T=n}}ht=1,h0(),v0(),g0()}}function h0(){if(ht===1){ht=0;var e=Wn,t=Bs,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=ne.T,ne.T=null;var n=ge.p;ge.p=2;var r=Ne;Ne|=4;try{e0(t,e);var s=of,i=Py(e.containerInfo),o=s.focusedElem,u=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&Ly(o.ownerDocument.documentElement,o)){if(u!==null&&kf(o)){var c=u.start,d=u.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var m=o.ownerDocument||document,f=m&&m.defaultView||window;if(f.getSelection){var h=f.getSelection(),x=o.textContent.length,y=Math.min(u.start,x),$=u.end===void 0?y:Math.min(u.end,x);!h.extend&&y>$&&(i=$,$=y,y=i);var g=ng(o,y),v=ng(o,$);if(g&&v&&(h.rangeCount!==1||h.anchorNode!==g.node||h.anchorOffset!==g.offset||h.focusNode!==v.node||h.focusOffset!==v.offset)){var b=m.createRange();b.setStart(g.node,g.offset),h.removeAllRanges(),y>$?(h.addRange(b),h.extend(v.node,v.offset)):(b.setEnd(v.node,v.offset),h.addRange(b))}}}}for(m=[],h=o;h=h.parentNode;)h.nodeType===1&&m.push({element:h,left:h.scrollLeft,top:h.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<m.length;o++){var w=m[o];w.element.scrollLeft=w.left,w.element.scrollTop=w.top}}Iu=!!sf,of=sf=null}finally{Ne=r,ge.p=n,ne.T=a}}e.current=t,ht=2}}function v0(){if(ht===2){ht=0;var e=Wn,t=Bs,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=ne.T,ne.T=null;var n=ge.p;ge.p=2;var r=Ne;Ne|=4;try{Jb(e,t.alternate,t)}finally{Ne=r,ge.p=n,ne.T=a}}ht=3}}function g0(){if(ht===4||ht===3){ht=0,zR();var e=Wn,t=Bs,a=As,n=o0;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?ht=5:(ht=0,Bs=Wn=null,y0(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(Zn=null),bf(a),t=t.stateNode,Jt&&typeof Jt.onCommitFiberRoot=="function")try{Jt.onCommitFiberRoot(Eo,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=ne.T,r=ge.p,ge.p=2,ne.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{ne.T=t,ge.p=r}}(As&3)!==0&&nc(),Ga(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===tf?mo++:(mo=0,tf=e):mo=0,zo(0,!1)}}function y0(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,Po(t)))}function nc(e){return h0(),v0(),g0(),b0(e)}function b0(){if(ht!==5)return!1;var e=Wn,t=Wm;Wm=0;var a=bf(As),n=ne.T,r=ge.p;try{ge.p=32>a?32:a,ne.T=null,a=ef,ef=null;var s=Wn,i=As;if(ht=0,Bs=Wn=null,As=0,(Ne&6)!==0)throw Error(j(331));var o=Ne;if(Ne|=4,s0(s.current),a0(s,s.current,i,a),Ne=o,zo(0,!1),Jt&&typeof Jt.onPostCommitFiberRoot=="function")try{Jt.onPostCommitFiberRoot(Eo,s)}catch{}return!0}finally{ge.p=r,ne.T=n,y0(e,t)}}function Ug(e,t,a){t=ha(a,t),t=Gm(e.stateNode,t,2),e=Xn(e,t,2),e!==null&&(Ao(e,2),Ga(e))}function Re(e,t,a){if(e.tag===3)Ug(e,e,a);else for(;t!==null;){if(t.tag===3){Ug(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(Zn===null||!Zn.has(n))){e=ha(a,e),a=Pb(2),n=Xn(t,a,2),n!==null&&(Ub(a,n,t,e),Ao(n,2),Ga(n));break}}t=t.return}}function pm(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new pE;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(Xf=!0,r.add(a),e=bE.bind(null,e,t,a),t.then(e,e))}function bE(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,Ce===e&&(me&a)===a&&(qe===4||qe===3&&(me&62914560)===me&&300>Ha()-Wf?(Ne&2)===0&&zs(e,0):Zf|=a,Fs===me&&(Fs=0)),Ga(e)}function x0(e,t){t===0&&(t=hy()),e=Hs(e,t),e!==null&&(Ao(e,t),Ga(e))}function xE(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),x0(e,a)}function $E(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(j(314))}n!==null&&n.delete(t),x0(e,a)}function wE(e,t){return gf(e,t)}var Pu=null,ds=null,nf=!1,Uu=!1,hm=!1,Tr=0;function Ga(e){e!==ds&&e.next===null&&(ds===null?Pu=ds=e:ds=ds.next=e),Uu=!0,nf||(nf=!0,NE())}function zo(e,t){if(!hm&&Uu){hm=!0;do for(var a=!1,n=Pu;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Xt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,jg(n,s))}else s=me,s=Hu(n,n===Ce?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||To(n,s)||(a=!0,jg(n,s));n=n.next}while(a);hm=!1}}function SE(){$0()}function $0(){Uu=nf=!1;var e=0;Tr!==0&&(TE()&&(e=Tr),Tr=0);for(var t=Ha(),a=null,n=Pu;n!==null;){var r=n.next,s=w0(n,t);s===0?(n.next=null,a===null?Pu=r:a.next=r,r===null&&(ds=a)):(a=n,(e!==0||(s&3)!==0)&&(Uu=!0)),n=r}zo(e,!1)}function w0(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Xt(s),o=1<<i,u=r[i];u===-1?((o&a)===0||(o&n)!==0)&&(r[i]=YR(o,t)):u<=t&&(e.expiredLanes|=o),s&=~o}if(t=Ce,a=me,a=Hu(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&(Se===2||Se===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&qd(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||To(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&qd(n),bf(a)){case 2:case 8:a=my;break;case 32:a=yu;break;case 268435456:a=fy;break;default:a=yu}return n=S0.bind(null,e),a=gf(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&qd(n),e.callbackPriority=2,e.callbackNode=null,2}function S0(e,t){if(ht!==0&&ht!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(nc(!0)&&e.callbackNode!==a)return null;var n=me;return n=Hu(e,e===Ce?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:(u0(e,n,t),w0(e,Ha()),e.callbackNode!=null&&e.callbackNode===a?S0.bind(null,e):null)}function jg(e,t){if(nc())return null;u0(e,t,!0)}function NE(){DE(function(){(Ne&6)!==0?gf(dy,SE):$0()})}function tp(){return Tr===0&&(Tr=py()),Tr}function Fg(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:nu(""+e)}function Bg(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function _E(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=Fg((r[Ut]||null).action),i=n.submitter;i&&(t=(t=i[Ut]||null)?Fg(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Qu("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(Tr!==0){var u=i?Bg(r,i):new FormData(r);Qm(a,{pending:!0,data:u,method:r.method,action:s},null,u)}}else typeof s=="function"&&(o.preventDefault(),u=i?Bg(r,i):new FormData(r),Qm(a,{pending:!0,data:u,method:r.method,action:s},s,u))},currentTarget:r}]})}}for(Xl=0;Xl<Om.length;Xl++)Zl=Om[Xl],zg=Zl.toLowerCase(),qg=Zl[0].toUpperCase()+Zl.slice(1),Ra(zg,"on"+qg);var Zl,zg,qg,Xl;Ra(jy,"onAnimationEnd");Ra(Fy,"onAnimationIteration");Ra(By,"onAnimationStart");Ra("dblclick","onDoubleClick");Ra("focusin","onFocus");Ra("focusout","onBlur");Ra(KC,"onTransitionRun");Ra(HC,"onTransitionStart");Ra(QC,"onTransitionCancel");Ra(zy,"onTransitionEnd");Ms("onMouseEnter",["mouseout","mouseover"]);Ms("onMouseLeave",["mouseout","mouseover"]);Ms("onPointerEnter",["pointerout","pointerover"]);Ms("onPointerLeave",["pointerout","pointerover"]);Lr("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Lr("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Lr("onBeforeInput",["compositionend","keypress","textInput","paste"]);Lr("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Lr("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Lr("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var $o="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),kE=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat($o));function N0(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],u=o.instance,c=o.currentTarget;if(o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Tu(d)}r.currentTarget=null,s=u}else for(i=0;i<n.length;i++){if(o=n[i],u=o.instance,c=o.currentTarget,o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Tu(d)}r.currentTarget=null,s=u}}}}function le(e,t){var a=t[Rm];a===void 0&&(a=t[Rm]=new Set);var n=e+"__bubble";a.has(n)||(_0(t,e,2,!1),a.add(n))}function vm(e,t,a){var n=0;t&&(n|=4),_0(a,e,n,t)}var Wl="_reactListening"+Math.random().toString(36).slice(2);function ap(e){if(!e[Wl]){e[Wl]=!0,by.forEach(function(a){a!=="selectionchange"&&(kE.has(a)||vm(a,!1,e),vm(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[Wl]||(t[Wl]=!0,vm("selectionchange",!1,t))}}function _0(e,t,a,n){switch(U0(t)){case 2:var r=WE;break;case 8:r=e3;break;default:r=ip}a=r.bind(null,t,a,e),r=void 0,!Am||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function gm(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var u=i.tag;if((u===3||u===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=ps(o),i===null)return;if(u=i.tag,u===5||u===6||u===26||u===27){n=s=i;continue e}o=o.parentNode}}n=n.return}Ry(function(){var c=s,d=wf(a),m=[];e:{var f=qy.get(e);if(f!==void 0){var h=Qu,x=e;switch(e){case"keypress":if(su(a)===0)break e;case"keydown":case"keyup":h=wC;break;case"focusin":x="focus",h=Jd;break;case"focusout":x="blur",h=Jd;break;case"beforeblur":case"afterblur":h=Jd;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":h=Gv;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":h=cC;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":h=_C;break;case jy:case Fy:case By:h=fC;break;case zy:h=RC;break;case"scroll":case"scrollend":h=lC;break;case"wheel":h=EC;break;case"copy":case"cut":case"paste":h=hC;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":h=Jv;break;case"toggle":case"beforetoggle":h=AC}var y=(t&4)!==0,$=!y&&(e==="scroll"||e==="scrollend"),g=y?f!==null?f+"Capture":null:f;y=[];for(var v=c,b;v!==null;){var w=v;if(b=w.stateNode,w=w.tag,w!==5&&w!==26&&w!==27||b===null||g===null||(w=ho(v,g),w!=null&&y.push(wo(v,w,b))),$)break;v=v.return}0<y.length&&(f=new h(f,x,null,a,d),m.push({event:f,listeners:y}))}}if((t&7)===0){e:{if(f=e==="mouseover"||e==="pointerover",h=e==="mouseout"||e==="pointerout",f&&a!==Tm&&(x=a.relatedTarget||a.fromElement)&&(ps(x)||x[Is]))break e;if((h||f)&&(f=d.window===d?d:(f=d.ownerDocument)?f.defaultView||f.parentWindow:window,h?(x=a.relatedTarget||a.toElement,h=c,x=x?ps(x):null,x!==null&&($=Co(x),y=x.tag,x!==$||y!==5&&y!==27&&y!==6)&&(x=null)):(h=null,x=c),h!==x)){if(y=Gv,w="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(y=Jv,w="onPointerLeave",g="onPointerEnter",v="pointer"),$=h==null?f:Xi(h),b=x==null?f:Xi(x),f=new y(w,v+"leave",h,a,d),f.target=$,f.relatedTarget=b,w=null,ps(d)===c&&(y=new y(g,v+"enter",x,a,d),y.target=b,y.relatedTarget=$,w=y),$=w,h&&x)t:{for(y=h,g=x,v=0,b=y;b;b=ls(b))v++;for(b=0,w=g;w;w=ls(w))b++;for(;0<v-b;)y=ls(y),v--;for(;0<b-v;)g=ls(g),b--;for(;v--;){if(y===g||g!==null&&y===g.alternate)break t;y=ls(y),g=ls(g)}y=null}else y=null;h!==null&&Ig(m,f,h,y,!1),x!==null&&$!==null&&Ig(m,$,x,y,!0)}}e:{if(f=c?Xi(c):window,h=f.nodeName&&f.nodeName.toLowerCase(),h==="select"||h==="input"&&f.type==="file")var S=eg;else if(Wv(f))if(My)S=zC;else{S=FC;var E=jC}else h=f.nodeName,!h||h.toLowerCase()!=="input"||f.type!=="checkbox"&&f.type!=="radio"?c&&$f(c.elementType)&&(S=eg):S=BC;if(S&&(S=S(e,c))){Dy(m,S,a,d);break e}E&&E(e,f,c),e==="focusout"&&c&&f.type==="number"&&c.memoizedProps.value!=null&&Em(f,"number",f.value)}switch(E=c?Xi(c):window,e){case"focusin":(Wv(E)||E.contentEditable==="true")&&(gs=E,Dm=c,to=null);break;case"focusout":to=Dm=gs=null;break;case"mousedown":Mm=!0;break;case"contextmenu":case"mouseup":case"dragend":Mm=!1,rg(m,a,d);break;case"selectionchange":if(IC)break;case"keydown":case"keyup":rg(m,a,d)}var k;if(_f)e:{switch(e){case"compositionstart":var C="onCompositionStart";break e;case"compositionend":C="onCompositionEnd";break e;case"compositionupdate":C="onCompositionUpdate";break e}C=void 0}else vs?Ty(e,a)&&(C="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(C="onCompositionStart");C&&(Ey&&a.locale!=="ko"&&(vs||C!=="onCompositionStart"?C==="onCompositionEnd"&&vs&&(k=Cy()):(Qn=d,Sf="value"in Qn?Qn.value:Qn.textContent,vs=!0)),E=ju(c,C),0<E.length&&(C=new Yv(C,e,null,a,d),m.push({event:C,listeners:E}),k?C.data=k:(k=Ay(a),k!==null&&(C.data=k)))),(k=MC?OC(e,a):LC(e,a))&&(C=ju(c,"onBeforeInput"),0<C.length&&(E=new Yv("onBeforeInput","beforeinput",null,a,d),m.push({event:E,listeners:C}),E.data=k)),_E(m,e,c,a,d)}N0(m,t)})}function wo(e,t,a){return{instance:e,listener:t,currentTarget:a}}function ju(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=ho(e,a),r!=null&&n.unshift(wo(e,r,s)),r=ho(e,t),r!=null&&n.push(wo(e,r,s))),e.tag===3)return n;e=e.return}return[]}function ls(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function Ig(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,u=o.alternate,c=o.stateNode;if(o=o.tag,u!==null&&u===n)break;o!==5&&o!==26&&o!==27||c===null||(u=c,r?(c=ho(a,s),c!=null&&i.unshift(wo(a,c,u))):r||(c=ho(a,s),c!=null&&i.push(wo(a,c,u)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var RE=/\r\n?/g,CE=/\u0000|\uFFFD/g;function Kg(e){return(typeof e=="string"?e:""+e).replace(RE,`
`).replace(CE,"")}function k0(e,t){return t=Kg(t),Kg(e)===t}function rc(){}function _e(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||Os(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&Os(e,""+n);break;case"className":ql(e,"class",n);break;case"tabIndex":ql(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":ql(e,a,n);break;case"style":ky(e,n,s);break;case"data":if(t!=="object"){ql(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=nu(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&_e(e,t,"name",r.name,r,null),_e(e,t,"formEncType",r.formEncType,r,null),_e(e,t,"formMethod",r.formMethod,r,null),_e(e,t,"formTarget",r.formTarget,r,null)):(_e(e,t,"encType",r.encType,r,null),_e(e,t,"method",r.method,r,null),_e(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=nu(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=rc);break;case"onScroll":n!=null&&le("scroll",e);break;case"onScrollEnd":n!=null&&le("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(j(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(j(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=nu(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":le("beforetoggle",e),le("toggle",e),au(e,"popover",n);break;case"xlinkActuate":sn(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":sn(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":sn(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":sn(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":sn(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":sn(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":sn(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":sn(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":sn(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":au(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=iC.get(a)||a,au(e,a,n))}}function rf(e,t,a,n,r,s){switch(a){case"style":ky(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(j(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(j(60));e.innerHTML=a}}break;case"children":typeof n=="string"?Os(e,n):(typeof n=="number"||typeof n=="bigint")&&Os(e,""+n);break;case"onScroll":n!=null&&le("scroll",e);break;case"onScrollEnd":n!=null&&le("scrollend",e);break;case"onClick":n!=null&&(e.onclick=rc);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!xy.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[Ut]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):au(e,a,n)}}}function vt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":le("error",e),le("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(j(137,t));default:_e(e,t,s,i,a,null)}}r&&_e(e,t,"srcSet",a.srcSet,a,null),n&&_e(e,t,"src",a.src,a,null);return;case"input":le("invalid",e);var o=s=i=r=null,u=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":u=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(j(137,t));break;default:_e(e,t,n,d,a,null)}}Sy(e,s,o,u,c,i,r,!1),bu(e);return;case"select":le("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:_e(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?_s(e,!!n,t,!1):a!=null&&_s(e,!!n,a,!0);return;case"textarea":le("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(j(91));break;default:_e(e,t,i,o,a,null)}_y(e,n,r,s),bu(e);return;case"option":for(u in a)if(a.hasOwnProperty(u)&&(n=a[u],n!=null))switch(u){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:_e(e,t,u,n,a,null)}return;case"dialog":le("beforetoggle",e),le("toggle",e),le("cancel",e),le("close",e);break;case"iframe":case"object":le("load",e);break;case"video":case"audio":for(n=0;n<$o.length;n++)le($o[n],e);break;case"image":le("error",e),le("load",e);break;case"details":le("toggle",e);break;case"embed":case"source":case"link":le("error",e),le("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(j(137,t));default:_e(e,t,c,n,a,null)}return;default:if($f(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&rf(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&_e(e,t,o,n,a,null))}function EE(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,u=null,c=null,d=null;for(h in a){var m=a[h];if(a.hasOwnProperty(h)&&m!=null)switch(h){case"checked":break;case"value":break;case"defaultValue":u=m;default:n.hasOwnProperty(h)||_e(e,t,h,null,n,m)}}for(var f in n){var h=n[f];if(m=a[f],n.hasOwnProperty(f)&&(h!=null||m!=null))switch(f){case"type":s=h;break;case"name":r=h;break;case"checked":c=h;break;case"defaultChecked":d=h;break;case"value":i=h;break;case"defaultValue":o=h;break;case"children":case"dangerouslySetInnerHTML":if(h!=null)throw Error(j(137,t));break;default:h!==m&&_e(e,t,f,h,n,m)}}Cm(e,i,o,u,c,d,s,r);return;case"select":h=i=o=f=null;for(s in a)if(u=a[s],a.hasOwnProperty(s)&&u!=null)switch(s){case"value":break;case"multiple":h=u;default:n.hasOwnProperty(s)||_e(e,t,s,null,n,u)}for(r in n)if(s=n[r],u=a[r],n.hasOwnProperty(r)&&(s!=null||u!=null))switch(r){case"value":f=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==u&&_e(e,t,r,s,n,u)}t=o,a=i,n=h,f!=null?_s(e,!!a,f,!1):!!n!=!!a&&(t!=null?_s(e,!!a,t,!0):_s(e,!!a,a?[]:"",!1));return;case"textarea":h=f=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:_e(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":f=r;break;case"defaultValue":h=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(j(91));break;default:r!==s&&_e(e,t,i,r,n,s)}Ny(e,f,h);return;case"option":for(var x in a)if(f=a[x],a.hasOwnProperty(x)&&f!=null&&!n.hasOwnProperty(x))switch(x){case"selected":e.selected=!1;break;default:_e(e,t,x,null,n,f)}for(u in n)if(f=n[u],h=a[u],n.hasOwnProperty(u)&&f!==h&&(f!=null||h!=null))switch(u){case"selected":e.selected=f&&typeof f!="function"&&typeof f!="symbol";break;default:_e(e,t,u,f,n,h)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var y in a)f=a[y],a.hasOwnProperty(y)&&f!=null&&!n.hasOwnProperty(y)&&_e(e,t,y,null,n,f);for(c in n)if(f=n[c],h=a[c],n.hasOwnProperty(c)&&f!==h&&(f!=null||h!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(f!=null)throw Error(j(137,t));break;default:_e(e,t,c,f,n,h)}return;default:if($f(t)){for(var $ in a)f=a[$],a.hasOwnProperty($)&&f!==void 0&&!n.hasOwnProperty($)&&rf(e,t,$,void 0,n,f);for(d in n)f=n[d],h=a[d],!n.hasOwnProperty(d)||f===h||f===void 0&&h===void 0||rf(e,t,d,f,n,h);return}}for(var g in a)f=a[g],a.hasOwnProperty(g)&&f!=null&&!n.hasOwnProperty(g)&&_e(e,t,g,null,n,f);for(m in n)f=n[m],h=a[m],!n.hasOwnProperty(m)||f===h||f==null&&h==null||_e(e,t,m,f,n,h)}var sf=null,of=null;function Fu(e){return e.nodeType===9?e:e.ownerDocument}function Hg(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function R0(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function lf(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var ym=null;function TE(){var e=window.event;return e&&e.type==="popstate"?e===ym?!1:(ym=e,!0):(ym=null,!1)}var C0=typeof setTimeout=="function"?setTimeout:void 0,AE=typeof clearTimeout=="function"?clearTimeout:void 0,Qg=typeof Promise=="function"?Promise:void 0,DE=typeof queueMicrotask=="function"?queueMicrotask:typeof Qg<"u"?function(e){return Qg.resolve(null).then(e).catch(ME)}:C0;function ME(e){setTimeout(function(){throw e})}function or(e){return e==="head"}function Vg(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&fo(i.documentElement),a&2&&fo(i.body),a&4)for(a=i.head,fo(a),i=a.firstChild;i;){var o=i.nextSibling,u=i.nodeName;i[Do]||u==="SCRIPT"||u==="STYLE"||u==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),Ro(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);Ro(t)}function uf(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":uf(a),xf(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function OE(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[Do])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=ka(e.nextSibling),e===null)break}return null}function LE(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=ka(e.nextSibling),e===null))return null;return e}function cf(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function PE(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function ka(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var df=null;function Gg(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function E0(e,t,a){switch(t=Fu(a),e){case"html":if(e=t.documentElement,!e)throw Error(j(452));return e;case"head":if(e=t.head,!e)throw Error(j(453));return e;case"body":if(e=t.body,!e)throw Error(j(454));return e;default:throw Error(j(451))}}function fo(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);xf(e)}var ya=new Map,Yg=new Set;function Bu(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var $n=ge.d;ge.d={f:UE,r:jE,D:FE,C:BE,L:zE,m:qE,X:KE,S:IE,M:HE};function UE(){var e=$n.f(),t=tc();return e||t}function jE(e){var t=Ks(e);t!==null&&t.tag===5&&t.type==="form"?wb(t):$n.r(e)}var Vs=typeof document>"u"?null:document;function T0(e,t,a){var n=Vs;if(n&&typeof t=="string"&&t){var r=pa(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),Yg.has(r)||(Yg.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),vt(t,"link",e),ut(t),n.head.appendChild(t)))}}function FE(e){$n.D(e),T0("dns-prefetch",e,null)}function BE(e,t){$n.C(e,t),T0("preconnect",e,t)}function zE(e,t,a){$n.L(e,t,a);var n=Vs;if(n&&e&&t){var r='link[rel="preload"][as="'+pa(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+pa(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+pa(a.imageSizes)+'"]')):r+='[href="'+pa(e)+'"]';var s=r;switch(t){case"style":s=qs(e);break;case"script":s=Gs(e)}ya.has(s)||(e=Ae({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),ya.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(qo(s))||t==="script"&&n.querySelector(Io(s))||(t=n.createElement("link"),vt(t,"link",e),ut(t),n.head.appendChild(t)))}}function qE(e,t){$n.m(e,t);var a=Vs;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+pa(n)+'"][href="'+pa(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=Gs(e)}if(!ya.has(s)&&(e=Ae({rel:"modulepreload",href:e},t),ya.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(Io(s)))return}n=a.createElement("link"),vt(n,"link",e),ut(n),a.head.appendChild(n)}}}function IE(e,t,a){$n.S(e,t,a);var n=Vs;if(n&&e){var r=Ns(n).hoistableStyles,s=qs(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(qo(s)))o.loading=5;else{e=Ae({rel:"stylesheet",href:e,"data-precedence":t},a),(a=ya.get(s))&&np(e,a);var u=i=n.createElement("link");ut(u),vt(u,"link",e),u._p=new Promise(function(c,d){u.onload=c,u.onerror=d}),u.addEventListener("load",function(){o.loading|=1}),u.addEventListener("error",function(){o.loading|=2}),o.loading|=4,mu(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function KE(e,t){$n.X(e,t);var a=Vs;if(a&&e){var n=Ns(a).hoistableScripts,r=Gs(e),s=n.get(r);s||(s=a.querySelector(Io(r)),s||(e=Ae({src:e,async:!0},t),(t=ya.get(r))&&rp(e,t),s=a.createElement("script"),ut(s),vt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function HE(e,t){$n.M(e,t);var a=Vs;if(a&&e){var n=Ns(a).hoistableScripts,r=Gs(e),s=n.get(r);s||(s=a.querySelector(Io(r)),s||(e=Ae({src:e,async:!0,type:"module"},t),(t=ya.get(r))&&rp(e,t),s=a.createElement("script"),ut(s),vt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function Jg(e,t,a,n){var r=(r=Yn.current)?Bu(r):null;if(!r)throw Error(j(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=qs(a.href),a=Ns(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=qs(a.href);var s=Ns(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(qo(e)))&&!s._p&&(i.instance=s,i.state.loading=5),ya.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},ya.set(e,a),s||QE(r,e,a,i.state))),t&&n===null)throw Error(j(528,""));return i}if(t&&n!==null)throw Error(j(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=Gs(a),a=Ns(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(j(444,e))}}function qs(e){return'href="'+pa(e)+'"'}function qo(e){return'link[rel="stylesheet"]['+e+"]"}function A0(e){return Ae({},e,{"data-precedence":e.precedence,precedence:null})}function QE(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),vt(t,"link",a),ut(t),e.head.appendChild(t))}function Gs(e){return'[src="'+pa(e)+'"]'}function Io(e){return"script[async]"+e}function Xg(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+pa(a.href)+'"]');if(n)return t.instance=n,ut(n),n;var r=Ae({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),ut(n),vt(n,"style",r),mu(n,a.precedence,e),t.instance=n;case"stylesheet":r=qs(a.href);var s=e.querySelector(qo(r));if(s)return t.state.loading|=4,t.instance=s,ut(s),s;n=A0(a),(r=ya.get(r))&&np(n,r),s=(e.ownerDocument||e).createElement("link"),ut(s);var i=s;return i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),vt(s,"link",n),t.state.loading|=4,mu(s,a.precedence,e),t.instance=s;case"script":return s=Gs(a.src),(r=e.querySelector(Io(s)))?(t.instance=r,ut(r),r):(n=a,(r=ya.get(s))&&(n=Ae({},a),rp(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),ut(r),vt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(j(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,mu(n,a.precedence,e));return t.instance}function mu(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function np(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function rp(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var fu=null;function Zg(e,t,a){if(fu===null){var n=new Map,r=fu=new Map;r.set(a,n)}else r=fu,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[Do]||s[bt]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function Wg(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function VE(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function D0(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var So=null;function GE(){}function YE(e,t,a){if(So===null)throw Error(j(475));var n=So;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=qs(a.href),s=e.querySelector(qo(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=zu.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,ut(s);return}s=e.ownerDocument||e,a=A0(a),(r=ya.get(r))&&np(a,r),s=s.createElement("link"),ut(s);var i=s;i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),vt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=zu.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function JE(){if(So===null)throw Error(j(475));var e=So;return e.stylesheets&&e.count===0&&mf(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&mf(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function zu(){if(this.count--,this.count===0){if(this.stylesheets)mf(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var qu=null;function mf(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,qu=new Map,t.forEach(XE,e),qu=null,zu.call(e))}function XE(e,t){if(!(t.state.loading&4)){var a=qu.get(e);if(a)var n=a.get(null);else{a=new Map,qu.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=zu.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var No={$$typeof:cn,Provider:null,Consumer:null,_currentValue:Nr,_currentValue2:Nr,_threadCount:0};function ZE(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=Id(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=Id(0),this.hiddenUpdates=Id(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function M0(e,t,a,n,r,s,i,o,u,c,d,m){return e=new ZE(e,t,a,i,o,u,c,m),t=1,s===!0&&(t|=24),s=Yt(3,null,null,t),e.current=s,s.stateNode=e,t=Df(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},Of(s),e}function O0(e){return e?(e=xs,e):xs}function L0(e,t,a,n,r,s){r=O0(r),n.context===null?n.context=r:n.pendingContext=r,n=Jn(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=Xn(e,n,t),a!==null&&(Wt(a,e,t),ro(a,e,t))}function ey(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function sp(e,t){ey(e,t),(e=e.alternate)&&ey(e,t)}function P0(e){if(e.tag===13){var t=Hs(e,67108864);t!==null&&Wt(t,e,67108864),sp(e,67108864)}}var Iu=!0;function WE(e,t,a,n){var r=ne.T;ne.T=null;var s=ge.p;try{ge.p=2,ip(e,t,a,n)}finally{ge.p=s,ne.T=r}}function e3(e,t,a,n){var r=ne.T;ne.T=null;var s=ge.p;try{ge.p=8,ip(e,t,a,n)}finally{ge.p=s,ne.T=r}}function ip(e,t,a,n){if(Iu){var r=ff(n);if(r===null)gm(e,t,n,Ku,a),ty(e,n);else if(a3(r,e,t,a,n))n.stopPropagation();else if(ty(e,n),t&4&&-1<t3.indexOf(e)){for(;r!==null;){var s=Ks(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=$r(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var u=1<<31-Xt(i);o.entanglements[1]|=u,i&=~u}Ga(s),(Ne&6)===0&&(Ou=Ha()+500,zo(0,!1))}}break;case 13:o=Hs(s,2),o!==null&&Wt(o,s,2),tc(),sp(s,2)}if(s=ff(n),s===null&&gm(e,t,n,Ku,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else gm(e,t,n,null,a)}}function ff(e){return e=wf(e),op(e)}var Ku=null;function op(e){if(Ku=null,e=ps(e),e!==null){var t=Co(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=oy(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return Ku=e,null}function U0(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(qR()){case dy:return 2;case my:return 8;case yu:case IR:return 32;case fy:return 268435456;default:return 32}default:return 32}}var pf=!1,er=null,tr=null,ar=null,_o=new Map,ko=new Map,Kn=[],t3="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function ty(e,t){switch(e){case"focusin":case"focusout":er=null;break;case"dragenter":case"dragleave":tr=null;break;case"mouseover":case"mouseout":ar=null;break;case"pointerover":case"pointerout":_o.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":ko.delete(t.pointerId)}}function Vi(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Ks(t),t!==null&&P0(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function a3(e,t,a,n,r){switch(t){case"focusin":return er=Vi(er,e,t,a,n,r),!0;case"dragenter":return tr=Vi(tr,e,t,a,n,r),!0;case"mouseover":return ar=Vi(ar,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return _o.set(s,Vi(_o.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,ko.set(s,Vi(ko.get(s)||null,e,t,a,n,r)),!0}return!1}function j0(e){var t=ps(e.target);if(t!==null){var a=Co(t);if(a!==null){if(t=a.tag,t===13){if(t=oy(a),t!==null){e.blockedOn=t,XR(e.priority,function(){if(a.tag===13){var n=Zt();n=yf(n);var r=Hs(a,n);r!==null&&Wt(r,a,n),sp(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function pu(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=ff(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);Tm=n,a.target.dispatchEvent(n),Tm=null}else return t=Ks(a),t!==null&&P0(t),e.blockedOn=a,!1;t.shift()}return!0}function ay(e,t,a){pu(e)&&a.delete(t)}function n3(){pf=!1,er!==null&&pu(er)&&(er=null),tr!==null&&pu(tr)&&(tr=null),ar!==null&&pu(ar)&&(ar=null),_o.forEach(ay),ko.forEach(ay)}function eu(e,t){e.blockedOn===t&&(e.blockedOn=null,pf||(pf=!0,rt.unstable_scheduleCallback(rt.unstable_NormalPriority,n3)))}var tu=null;function ny(e){tu!==e&&(tu=e,rt.unstable_scheduleCallback(rt.unstable_NormalPriority,function(){tu===e&&(tu=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(op(n||a)===null)continue;break}var s=Ks(a);s!==null&&(e.splice(t,3),t-=3,Qm(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function Ro(e){function t(u){return eu(u,e)}er!==null&&eu(er,e),tr!==null&&eu(tr,e),ar!==null&&eu(ar,e),_o.forEach(t),ko.forEach(t);for(var a=0;a<Kn.length;a++){var n=Kn[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<Kn.length&&(a=Kn[0],a.blockedOn===null);)j0(a),a.blockedOn===null&&Kn.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[Ut]||null;if(typeof s=="function")i||ny(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[Ut]||null)o=i.formAction;else if(op(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),ny(a)}}}function lp(e){this._internalRoot=e}sc.prototype.render=lp.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(j(409));var a=t.current,n=Zt();L0(a,n,e,t,null,null)};sc.prototype.unmount=lp.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;L0(e.current,2,null,e,null,null),tc(),t[Is]=null}};function sc(e){this._internalRoot=e}sc.prototype.unstable_scheduleHydration=function(e){if(e){var t=yy();e={blockedOn:null,target:e,priority:t};for(var a=0;a<Kn.length&&t!==0&&t<Kn[a].priority;a++);Kn.splice(a,0,e),a===0&&j0(e)}};var ry=sy.version;if(ry!=="19.1.0")throw Error(j(527,ry,"19.1.0"));ge.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(j(188)):(e=Object.keys(e).join(","),Error(j(268,e)));return e=LR(t),e=e!==null?ly(e):null,e=e===null?null:e.stateNode,e};var r3={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:ne,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(Gi=__REACT_DEVTOOLS_GLOBAL_HOOK__,!Gi.isDisabled&&Gi.supportsFiber))try{Eo=Gi.inject(r3),Jt=Gi}catch{}var Gi;ic.createRoot=function(e,t){if(!iy(e))throw Error(j(299));var a=!1,n="",r=Mb,s=Ob,i=Lb,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=M0(e,1,!1,null,null,a,n,r,s,i,o,null),e[Is]=t.current,ap(e),new lp(t)};ic.hydrateRoot=function(e,t,a){if(!iy(e))throw Error(j(299));var n=!1,r="",s=Mb,i=Ob,o=Lb,u=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(u=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=M0(e,1,!0,t,a??null,n,r,s,i,o,u,c),t.context=O0(null),a=t.current,n=Zt(),n=yf(n),r=Jn(n),r.callback=null,Xn(a,r,n),a=n,t.current.lanes=a,Ao(t,a),Ga(t),e[Is]=t.current,ap(e),new sc(t)};ic.version="19.1.0"});var q0=Dn((F6,z0)=>{"use strict";function B0(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(B0)}catch(e){console.error(e)}}B0(),z0.exports=F0()});var At=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var pR={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},hR=class{#t=pR;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},Ua=new hR;function Xh(e){setTimeout(e,0)}var Dt=typeof window>"u"||"Deno"in globalThis;function De(){}function ev(e,t){return typeof e=="function"?e(t):e}function Ei(e){return typeof e=="number"&&e>=0&&e!==1/0}function yl(e,t){return Math.max(e+(t||0)-Date.now(),0)}function Na(e,t){return typeof e=="function"?e(t):e}function Mt(e,t){return typeof e=="function"?e(t):e}function bl(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==Ti(i,t.options))return!1}else if(!yr(t.queryKey,i))return!1}if(a!=="all"){let u=t.isActive();if(a==="active"&&!u||a==="inactive"&&u)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function xl(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(ja(t.options.mutationKey)!==ja(s))return!1}else if(!yr(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function Ti(e,t){return(t?.queryKeyHashFn||ja)(e)}function ja(e){return JSON.stringify(e,(t,a)=>xd(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function yr(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>yr(e[a],t[a])):!1}var vR=Object.prototype.hasOwnProperty;function Ai(e,t){if(e===t)return e;let a=Zh(e)&&Zh(t);if(!a&&!(xd(e)&&xd(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},u=0;for(let c=0;c<i;c++){let d=a?c:s[c],m=e[d],f=t[d];if(m===f){o[d]=m,(a?c<r:vR.call(e,d))&&u++;continue}if(m===null||f===null||typeof m!="object"||typeof f!="object"){o[d]=f;continue}let h=Ai(m,f);o[d]=h,h===m&&u++}return r===i&&u===r?e:o}function Mn(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function Zh(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function xd(e){if(!Wh(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!Wh(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function Wh(e){return Object.prototype.toString.call(e)==="[object Object]"}function tv(e){return new Promise(t=>{Ua.setTimeout(t,e)})}function Di(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?Ai(e,t):t}function av(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function nv(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var Wr=Symbol();function $l(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===Wr?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function Mi(e,t){return typeof e=="function"?e(...t):!!e}var gR=class extends At{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!Dt&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},es=new gR;function Oi(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var rv=Xh;function yR(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=rv,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(u=>{a(u)})})})};return{batch:o=>{let u;t++;try{u=o()}finally{t--,t||i()}return u},batchCalls:o=>(...u)=>{s(()=>{o(...u)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var de=yR();var bR=class extends At{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!Dt&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},ts=new bR;function xR(e){return Math.min(1e3*2**e,3e4)}function $d(e){return(e??"online")==="online"?ts.isOnline():!0}var wl=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function Sl(e){let t=!1,a=0,n,r=Oi(),s=()=>r.status!=="pending",i=y=>{if(!s()){let $=new wl(y);f($),e.onCancel?.($)}},o=()=>{t=!0},u=()=>{t=!1},c=()=>es.isFocused()&&(e.networkMode==="always"||ts.isOnline())&&e.canRun(),d=()=>$d(e.networkMode)&&e.canRun(),m=y=>{s()||(n?.(),r.resolve(y))},f=y=>{s()||(n?.(),r.reject(y))},h=()=>new Promise(y=>{n=$=>{(s()||c())&&y($)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),x=()=>{if(s())return;let y,$=a===0?e.initialPromise:void 0;try{y=$??e.fn()}catch(g){y=Promise.reject(g)}Promise.resolve(y).then(m).catch(g=>{if(s())return;let v=e.retry??(Dt?0:3),b=e.retryDelay??xR,w=typeof b=="function"?b(a,g):b,S=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!S){f(g);return}a++,e.onFail?.(a,g),tv(w).then(()=>c()?void 0:h()).then(()=>{t?f(g):x()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:u,canStart:d,start:()=>(d()?x():h().then(x),r)}}var Nl=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),Ei(this.gcTime)&&(this.#t=Ua.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(Dt?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(Ua.clearTimeout(this.#t),this.#t=void 0)}};var iv=class extends Nl{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=sv(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=sv(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=Di(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(De).catch(De):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>Mt(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===Wr||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>Na(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!yl(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(u=>u.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=$l(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=Sl({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof wl&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,u)=>{this.#i({type:"failed",failureCount:o,error:u})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof wl){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,...wd(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),de.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function wd(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:$d(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function sv(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var br=class extends At{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=Oi(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),ov(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return Sd(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return Sd(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof Mt(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!Mn(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&lv(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||Mt(this.options.enabled,this.#e)!==Mt(t.enabled,this.#e)||Na(this.options.staleTime,this.#e)!==Na(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||Mt(this.options.enabled,this.#e)!==Mt(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return wR(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(De)),t}#v(){this.#x();let e=Na(this.options.staleTime,this.#e);if(Dt||this.#n.isStale||!Ei(e))return;let a=yl(this.#n.dataUpdatedAt,e)+1;this.#u=Ua.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(Dt||Mt(this.options.enabled,this.#e)===!1||!Ei(this.#l)||this.#l===0)&&(this.#c=Ua.setInterval(()=>{(this.options.refetchIntervalInBackground||es.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(Ua.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(Ua.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,u=e!==a?e.state:this.#a,{state:c}=e,d={...c},m=!1,f;if(t._optimisticResults){let C=this.hasListeners(),O=!C&&ov(e,t),L=C&&lv(e,a,t,n);(O||L)&&(d={...d,...wd(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:h,errorUpdatedAt:x,status:y}=d;f=d.data;let $=!1;if(t.placeholderData!==void 0&&f===void 0&&y==="pending"){let C;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(C=r.data,$=!0):C=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,C!==void 0&&(y="success",f=Di(r?.data,C,t),m=!0)}if(t.select&&f!==void 0&&!$)if(r&&f===s?.data&&t.select===this.#f)f=this.#d;else try{this.#f=t.select,f=t.select(f),f=Di(r?.data,f,t),this.#d=f,this.#i=null}catch(C){this.#i=C}this.#i&&(h=this.#i,f=this.#d,x=Date.now(),y="error");let g=d.fetchStatus==="fetching",v=y==="pending",b=y==="error",w=v&&g,S=f!==void 0,k={status:y,fetchStatus:d.fetchStatus,isPending:v,isSuccess:y==="success",isError:b,isInitialLoading:w,isLoading:w,data:f,dataUpdatedAt:d.dataUpdatedAt,error:h,errorUpdatedAt:x,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>u.dataUpdateCount||d.errorUpdateCount>u.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:b&&!S,isPaused:d.fetchStatus==="paused",isPlaceholderData:m,isRefetchError:b&&S,isStale:Nd(e,t),refetch:this.refetch,promise:this.#o,isEnabled:Mt(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let C=P=>{k.status==="error"?P.reject(k.error):k.data!==void 0&&P.resolve(k.data)},O=()=>{let P=this.#o=k.promise=Oi();C(P)},L=this.#o;switch(L.status){case"pending":e.queryHash===a.queryHash&&C(L);break;case"fulfilled":(k.status==="error"||k.data!==L.value)&&O();break;case"rejected":(k.status!=="error"||k.error!==L.reason)&&O();break}}return k}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),Mn(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){de.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function $R(e,t){return Mt(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function ov(e,t){return $R(e,t)||e.state.data!==void 0&&Sd(e,t,t.refetchOnMount)}function Sd(e,t,a){if(Mt(t.enabled,e)!==!1&&Na(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&Nd(e,t)}return!1}function lv(e,t,a,n){return(e!==t||Mt(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&Nd(e,a)}function Nd(e,t){return Mt(t.enabled,e)!==!1&&e.isStaleByTime(Na(t.staleTime,e))}function wR(e,t){return!Mn(e.getCurrentResult(),t)}function _d(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},u=0,c=async()=>{let d=!1,m=x=>{Object.defineProperty(x,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},f=$l(t.options,t.fetchOptions),h=async(x,y,$)=>{if(d)return Promise.reject();if(y==null&&x.pages.length)return Promise.resolve(x);let v=(()=>{let E={client:t.client,queryKey:t.queryKey,pageParam:y,direction:$?"backward":"forward",meta:t.options.meta};return m(E),E})(),b=await f(v),{maxPages:w}=t.options,S=$?nv:av;return{pages:S(x.pages,b,w),pageParams:S(x.pageParams,y,w)}};if(r&&s.length){let x=r==="backward",y=x?SR:uv,$={pages:s,pageParams:i},g=y(n,$);o=await h($,g,x)}else{let x=e??s.length;do{let y=u===0?i[0]??n.initialPageParam:uv(n,o);if(u>0&&y==null)break;o=await h(o,y),u++}while(u<x)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function uv(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function SR(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var cv=class extends Nl{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||kd(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=Sl({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),de.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function kd(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var dv=class extends At{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new cv({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=_l(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=_l(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=_l(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=_l(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){de.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>xl(t,a))}findAll(e={}){return this.getAll().filter(t=>xl(e,t))}notify(e){de.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return de.batch(()=>Promise.all(e.map(t=>t.continue().catch(De))))}};function _l(e){return e.options.scope?.id}var Rd=class extends At{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),Mn(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&ja(t.mutationKey)!==ja(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??kd();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){de.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function mv(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function NR(e,t,a){let n=e.slice(0);return n[t]=a,n}var Cd=class extends At{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,de.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,m)=>d!==a[m]),u=i||o,c=u?!0:s.some((d,m)=>{let f=this.#e[m];return!f||!Mn(d,f)});!u&&!c||(u&&(this.#r=r),this.#e=s,this.hasListeners()&&(u&&(mv(a,r).forEach(d=>{d.destroy()}),mv(r,a).forEach(d=>{d.subscribe(m=>{this.#c(d,m)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=Ai(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new br(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=NR(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&de.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var fv=class extends At{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??Ti(n,t),s=this.get(r);return s||(s=new iv({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){de.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>bl(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>bl(e,a)):t}notify(e){de.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){de.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){de.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var Ed=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new fv,this.#e=e.mutationCache||new dv,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=es.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=ts.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime(Na(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=ev(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return de.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;de.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return de.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=de.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(De).catch(De)}invalidateQueries(e,t={}){return de.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=de.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(De)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(De)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime(Na(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(De).catch(De)}fetchInfiniteQuery(e){return e.behavior=_d(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(De).catch(De)}ensureInfiniteQueryData(e){return e.behavior=_d(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return ts.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(ja(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{yr(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(ja(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{yr(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=Ti(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===Wr&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var Fa=Be(He(),1);var as=Be(He(),1),gv=Be(Td(),1),Ad=as.createContext(void 0),X=e=>{let t=as.useContext(Ad);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},Dd=({client:e,children:t})=>(as.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,gv.jsx)(Ad.Provider,{value:e,children:t}));var Rl=Be(He(),1),yv=Rl.createContext(!1),Cl=()=>Rl.useContext(yv),t6=yv.Provider;var Li=Be(He(),1),RR=Be(Td(),1);function CR(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var ER=Li.createContext(CR()),El=()=>Li.useContext(ER);var bv=Be(He(),1);var Tl=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},Al=e=>{bv.useEffect(()=>{e.clearReset()},[e])},Dl=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||Mi(a,[e.error,n]));var Ml=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},Ol=(e,t)=>e.isLoading&&e.isFetching&&!t,Pi=(e,t)=>e?.suspense&&t.isPending,ns=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function Md({queries:e,...t},a){let n=X(a),r=Cl(),s=El(),i=Fa.useMemo(()=>e.map(y=>{let $=n.defaultQueryOptions(y);return $._optimisticResults=r?"isRestoring":"optimistic",$}),[e,n,r]);i.forEach(y=>{Ml(y),Tl(y,s)}),Al(s);let[o]=Fa.useState(()=>new Cd(n,i,t)),[u,c,d]=o.getOptimisticResult(i,t.combine),m=!r&&t.subscribed!==!1;Fa.useSyncExternalStore(Fa.useCallback(y=>m?o.subscribe(de.batchCalls(y)):De,[o,m]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),Fa.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let h=u.some((y,$)=>Pi(i[$],y))?u.flatMap((y,$)=>{let g=i[$];if(g){let v=new br(n,g);if(Pi(g,y))return ns(g,v,s);Ol(y,r)&&ns(g,v,s)}return[]}):[];if(h.length>0)throw Promise.all(h);let x=u.find((y,$)=>{let g=i[$];return g&&Dl({result:y,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(x?.error)throw x.error;return c(d())}var On=Be(He(),1);function xv(e,t,a){let n=Cl(),r=El(),s=X(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",Ml(i),Tl(i,r),Al(r);let o=!s.getQueryCache().get(i.queryHash),[u]=On.useState(()=>new t(s,i)),c=u.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(On.useSyncExternalStore(On.useCallback(m=>{let f=d?u.subscribe(de.batchCalls(m)):De;return u.updateResult(),f},[u,d]),()=>u.getCurrentResult(),()=>u.getCurrentResult()),On.useEffect(()=>{u.setOptions(i)},[i,u]),Pi(i,c))throw ns(i,u,r);if(Dl({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!Dt&&Ol(c,n)&&(o?ns(i,u,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(De).finally(()=>{u.updateResult()}),i.notifyOnChangeProps?c:u.trackResult(c)}function H(e,t){return xv(e,br,t)}var nn=Be(He(),1);function G(e,t){let a=X(t),[n]=nn.useState(()=>new Rd(a,e));nn.useEffect(()=>{n.setOptions(e)},[n,e]);let r=nn.useSyncExternalStore(nn.useCallback(i=>n.subscribe(de.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=nn.useCallback((i,o)=>{n.mutate(i,o).catch(De)},[n]);if(r.error&&Mi(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var dR=Be(q0());var aa=Be(He(),1),Z=Be(He(),1),Te=Be(He(),1),Rp=Be(He(),1),dx=Be(He(),1),ye=Be(He(),1),sT=Be(He(),1),iT=Be(He(),1),oT=Be(He(),1),ee=Be(He(),1),kx=Be(He(),1);var I0="popstate";function G0(e={}){function t(n,r){let{pathname:s,search:i,hash:o}=n.location;return dp("",{pathname:s,search:i,hash:o},r.state&&r.state.usr||null,r.state&&r.state.key||"default")}function a(n,r){return typeof r=="string"?r:Ys(r)}return i3(t,a,null,e)}function Ee(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function ta(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function s3(){return Math.random().toString(36).substring(2,10)}function K0(e,t){return{usr:e.state,key:e.key,idx:t}}function dp(e,t,a=null,n){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?Fr(t):t,state:a,key:t&&t.key||n||s3()}}function Ys({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function Fr(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function i3(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",u=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function m(){o="POP";let $=d(),g=$==null?null:$-c;c=$,u&&u({action:o,location:y.location,delta:g})}function f($,g){o="PUSH";let v=dp(y.location,$,g);a&&a(v,$),c=d()+1;let b=K0(v,c),w=y.createHref(v);try{i.pushState(b,"",w)}catch(S){if(S instanceof DOMException&&S.name==="DataCloneError")throw S;r.location.assign(w)}s&&u&&u({action:o,location:y.location,delta:1})}function h($,g){o="REPLACE";let v=dp(y.location,$,g);a&&a(v,$),c=d();let b=K0(v,c),w=y.createHref(v);i.replaceState(b,"",w),s&&u&&u({action:o,location:y.location,delta:0})}function x($){return o3($)}let y={get action(){return o},get location(){return e(r,i)},listen($){if(u)throw new Error("A history only accepts one active listener");return r.addEventListener(I0,m),u=$,()=>{r.removeEventListener(I0,m),u=null}},createHref($){return t(r,$)},createURL:x,encodeLocation($){let g=x($);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:f,replace:h,go($){return i.go($)}};return y}function o3(e,t=!1){let a="http://localhost";typeof window<"u"&&(a=window.location.origin!=="null"?window.location.origin:window.location.href),Ee(a,"No window.location.(origin|href) available to create URL");let n=typeof e=="string"?e:Ys(e);return n=n.replace(/ $/,"%20"),!t&&n.startsWith("//")&&(n=a+n),new URL(n,a)}var l3;l3=new WeakMap;function hp(e,t,a="/"){return u3(e,t,a,!1)}function u3(e,t,a,n){let r=typeof t=="string"?Fr(t):t,s=Ya(r.pathname||"/",a);if(s==null)return null;let i=Y0(e);d3(i);let o=null;for(let u=0;o==null&&u<i.length;++u){let c=w3(s);o=x3(i[u],c,n)}return o}function c3(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function Y0(e,t=[],a=[],n="",r=!1){let s=(i,o,u=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&u)return;Ee(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let m=wn([n,d.relativePath]),f=a.concat(d);i.children&&i.children.length>0&&(Ee(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${m}".`),Y0(i.children,t,f,m,u)),!(i.path==null&&!i.index)&&t.push({path:m,score:y3(m,i.index),routesMeta:f})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let u of J0(i.path))s(i,o,!0,u)}),t}function J0(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=J0(n.join("/")),o=[];return o.push(...i.map(u=>u===""?s:[s,u].join("/"))),r&&o.push(...i),o.map(u=>e.startsWith("/")&&u===""?"/":u)}function d3(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:b3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var m3=/^:[\w-]+$/,f3=3,p3=2,h3=1,v3=10,g3=-2,H0=e=>e==="*";function y3(e,t){let a=e.split("/"),n=a.length;return a.some(H0)&&(n+=g3),t&&(n+=p3),a.filter(r=>!H0(r)).reduce((r,s)=>r+(m3.test(s)?f3:s===""?h3:v3),n)}function b3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function x3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let u=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",m=Ho({path:u.relativePath,caseSensitive:u.caseSensitive,end:c},d),f=u.route;if(!m&&c&&a&&!n[n.length-1].route.index&&(m=Ho({path:u.relativePath,caseSensitive:u.caseSensitive,end:!1},d)),!m)return null;Object.assign(r,m.params),i.push({params:r,pathname:wn([s,m.pathname]),pathnameBase:_3(wn([s,m.pathnameBase])),route:f}),m.pathnameBase!=="/"&&(s=wn([s,m.pathnameBase]))}return i}function Ho(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=$3(e.path,e.caseSensitive,e.end),r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:m},f)=>{if(d==="*"){let x=o[f]||"";i=s.slice(0,s.length-x.length).replace(/(.)\/+$/,"$1")}let h=o[f];return m&&!h?c[d]=void 0:c[d]=(h||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function $3(e,t=!1,a=!0){ta(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,u)=>(n.push({paramName:o,isOptional:u!=null}),u?"/?([^\\/]+)?":"/([^\\/]+)")).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function w3(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return ta(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function Ya(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}function X0(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?Fr(e):e;return{pathname:a?a.startsWith("/")?a:S3(a,t):t,search:k3(n),hash:R3(r)}}function S3(e,t){let a=t.replace(/\/+$/,"").split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function up(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function N3(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function vp(e){let t=N3(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function gp(e,t,a,n=!1){let r;typeof e=="string"?r=Fr(e):(r={...e},Ee(!r.pathname||!r.pathname.includes("?"),up("?","pathname","search",r)),Ee(!r.pathname||!r.pathname.includes("#"),up("#","pathname","hash",r)),Ee(!r.search||!r.search.includes("#"),up("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let m=t.length-1;if(!n&&i.startsWith("..")){let f=i.split("/");for(;f[0]==="..";)f.shift(),m-=1;r.pathname=f.join("/")}o=m>=0?t[m]:"/"}let u=X0(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!u.pathname.endsWith("/")&&(c||d)&&(u.pathname+="/"),u}var wn=e=>e.join("/").replace(/\/\/+/g,"/"),_3=e=>e.replace(/\/+$/,"").replace(/^\/*/,"/"),k3=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,R3=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;function Z0(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}var W0=["POST","PUT","PATCH","DELETE"],B6=new Set(W0),C3=["GET",...W0],z6=new Set(C3);var q6=Symbol("ResetLoaderData");var Br=aa.createContext(null);Br.displayName="DataRouter";var Js=aa.createContext(null);Js.displayName="DataRouterState";var I6=aa.createContext(!1);var yp=aa.createContext({isTransitioning:!1});yp.displayName="ViewTransition";var ex=aa.createContext(new Map);ex.displayName="Fetchers";var E3=aa.createContext(null);E3.displayName="Await";var Ft=aa.createContext(null);Ft.displayName="Navigation";var Xs=aa.createContext(null);Xs.displayName="Location";var na=aa.createContext({outlet:null,matches:[],isDataRoute:!1});na.displayName="Route";var bp=aa.createContext(null);bp.displayName="RouteError";var mp=!0;function tx(e,{relative:t}={}){Ee(zr(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=Z.useContext(Ft),{hash:r,pathname:s,search:i}=Zs(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:wn([a,s])),n.createHref({pathname:o,search:i,hash:r})}function zr(){return Z.useContext(Xs)!=null}function Pe(){return Ee(zr(),"useLocation() may be used only in the context of a <Router> component."),Z.useContext(Xs).location}var ax="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function nx(e){Z.useContext(Ft).static||Z.useLayoutEffect(e)}function pe(){let{isDataRoute:e}=Z.useContext(na);return e?F3():T3()}function T3(){Ee(zr(),"useNavigate() may be used only in the context of a <Router> component.");let e=Z.useContext(Br),{basename:t,navigator:a}=Z.useContext(Ft),{matches:n}=Z.useContext(na),{pathname:r}=Pe(),s=JSON.stringify(vp(n)),i=Z.useRef(!1);return nx(()=>{i.current=!0}),Z.useCallback((u,c={})=>{if(ta(i.current,ax),!i.current)return;if(typeof u=="number"){a.go(u);return}let d=gp(u,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:wn([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var rx=Z.createContext(null);function ba(){return Z.useContext(rx)}function sx(e){let t=Z.useContext(na).outlet;return t&&Z.createElement(rx.Provider,{value:e},t)}function st(){let{matches:e}=Z.useContext(na),t=e[e.length-1];return t?t.params:{}}function Zs(e,{relative:t}={}){let{matches:a}=Z.useContext(na),{pathname:n}=Pe(),r=JSON.stringify(vp(a));return Z.useMemo(()=>gp(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function ix(e,t){return ox(e,t)}function ox(e,t,a,n,r){Ee(zr(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:s}=Z.useContext(Ft),{matches:i}=Z.useContext(na),o=i[i.length-1],u=o?o.params:{},c=o?o.pathname:"/",d=o?o.pathnameBase:"/",m=o&&o.route;if(mp){let v=m&&m.path||"";cx(c,!m||v.endsWith("*")||v.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${c}" (under <Route path="${v}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${v}"> to <Route path="${v==="/"?"*":`${v}/*`}">.`)}let f=Pe(),h;if(t){let v=typeof t=="string"?Fr(t):t;Ee(d==="/"||v.pathname?.startsWith(d),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${d}" but pathname "${v.pathname}" was given in the \`location\` prop.`),h=v}else h=f;let x=h.pathname||"/",y=x;if(d!=="/"){let v=d.replace(/^\//,"").split("/");y="/"+x.replace(/^\//,"").split("/").slice(v.length).join("/")}let $=hp(e,{pathname:y});mp&&(ta(m||$!=null,`No routes matched location "${h.pathname}${h.search}${h.hash}" `),ta($==null||$[$.length-1].route.element!==void 0||$[$.length-1].route.Component!==void 0||$[$.length-1].route.lazy!==void 0,`Matched leaf route at location "${h.pathname}${h.search}${h.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let g=L3($&&$.map(v=>Object.assign({},v,{params:Object.assign({},u,v.params),pathname:wn([d,s.encodeLocation?s.encodeLocation(v.pathname).pathname:v.pathname]),pathnameBase:v.pathnameBase==="/"?d:wn([d,s.encodeLocation?s.encodeLocation(v.pathnameBase).pathname:v.pathnameBase])})),i,a,n,r);return t&&g?Z.createElement(Xs.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",...h},navigationType:"POP"}},g):g}function A3(){let e=ux(),t=Z0(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return mp&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=Z.createElement(Z.Fragment,null,Z.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),Z.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",Z.createElement("code",{style:s},"ErrorBoundary")," or"," ",Z.createElement("code",{style:s},"errorElement")," prop on your route."))),Z.createElement(Z.Fragment,null,Z.createElement("h2",null,"Unexpected Application Error!"),Z.createElement("h3",{style:{fontStyle:"italic"}},t),a?Z.createElement("pre",{style:r},a):null,i)}var D3=Z.createElement(A3,null),M3=class extends Z.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.unstable_onError?this.props.unstable_onError(e,t):console.error("React Router caught the following error during render",e)}render(){return this.state.error!==void 0?Z.createElement(na.Provider,{value:this.props.routeContext},Z.createElement(bp.Provider,{value:this.state.error,children:this.props.component})):this.props.children}};function O3({routeContext:e,match:t,children:a}){let n=Z.useContext(Br);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),Z.createElement(na.Provider,{value:e},a)}function L3(e,t=[],a=null,n=null,r=null){if(e==null){if(!a)return null;if(a.errors)e=a.matches;else if(t.length===0&&!a.initialized&&a.matches.length>0)e=a.matches;else return null}let s=e,i=a?.errors;if(i!=null){let c=s.findIndex(d=>d.route.id&&i?.[d.route.id]!==void 0);Ee(c>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(i).join(",")}`),s=s.slice(0,Math.min(s.length,c+1))}let o=!1,u=-1;if(a)for(let c=0;c<s.length;c++){let d=s[c];if((d.route.HydrateFallback||d.route.hydrateFallbackElement)&&(u=c),d.route.id){let{loaderData:m,errors:f}=a,h=d.route.loader&&!m.hasOwnProperty(d.route.id)&&(!f||f[d.route.id]===void 0);if(d.route.lazy||h){o=!0,u>=0?s=s.slice(0,u+1):s=[s[0]];break}}}return s.reduceRight((c,d,m)=>{let f,h=!1,x=null,y=null;a&&(f=i&&d.route.id?i[d.route.id]:void 0,x=d.route.errorElement||D3,o&&(u<0&&m===0?(cx("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),h=!0,y=null):u===m&&(h=!0,y=d.route.hydrateFallbackElement||null)));let $=t.concat(s.slice(0,m+1)),g=()=>{let v;return f?v=x:h?v=y:d.route.Component?v=Z.createElement(d.route.Component,null):d.route.element?v=d.route.element:v=c,Z.createElement(O3,{match:d,routeContext:{outlet:c,matches:$,isDataRoute:a!=null},children:v})};return a&&(d.route.ErrorBoundary||d.route.errorElement||m===0)?Z.createElement(M3,{location:a.location,revalidation:a.revalidation,component:x,error:f,children:g(),routeContext:{outlet:null,matches:$,isDataRoute:!0},unstable_onError:n}):g()},null)}function xp(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function P3(e){let t=Z.useContext(Br);return Ee(t,xp(e)),t}function $p(e){let t=Z.useContext(Js);return Ee(t,xp(e)),t}function U3(e){let t=Z.useContext(na);return Ee(t,xp(e)),t}function wp(e){let t=U3(e),a=t.matches[t.matches.length-1];return Ee(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function j3(){return wp("useRouteId")}function lx(){return $p("useNavigation").navigation}function Sp(){let{matches:e,loaderData:t}=$p("useMatches");return Z.useMemo(()=>e.map(a=>c3(a,t)),[e,t])}function ux(){let e=Z.useContext(bp),t=$p("useRouteError"),a=wp("useRouteError");return e!==void 0?e:t.errors?.[a]}function F3(){let{router:e}=P3("useNavigate"),t=wp("useNavigate"),a=Z.useRef(!1);return nx(()=>{a.current=!0}),Z.useCallback(async(r,s={})=>{ta(a.current,ax),a.current&&(typeof r=="number"?e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var Q0={};function cx(e,t,a){!t&&!Q0[e]&&(Q0[e]=!0,ta(!1,a))}var K6=Te.memo(B3);function B3({routes:e,future:t,state:a,unstable_onError:n}){return ox(e,void 0,a,n,t)}function it({to:e,replace:t,state:a,relative:n}){Ee(zr(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=Te.useContext(Ft);ta(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=Te.useContext(na),{pathname:i}=Pe(),o=pe(),u=gp(e,vp(s),i,n==="path"),c=JSON.stringify(u);return Te.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function Np(e){return sx(e.context)}function be(e){Ee(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function _p({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1}){Ee(!zr(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let i=e.replace(/^\/*/,"/"),o=Te.useMemo(()=>({basename:i,navigator:r,static:s,future:{}}),[i,r,s]);typeof a=="string"&&(a=Fr(a));let{pathname:u="/",search:c="",hash:d="",state:m=null,key:f="default"}=a,h=Te.useMemo(()=>{let x=Ya(u,i);return x==null?null:{location:{pathname:x,search:c,hash:d,state:m,key:f},navigationType:n}},[i,u,c,d,m,f,n]);return ta(h!=null,`<Router basename="${i}"> is not able to match the URL "${u}${c}${d}" because it does not start with the basename, so the <Router> won't render anything.`),h==null?null:Te.createElement(Ft.Provider,{value:o},Te.createElement(Xs.Provider,{children:t,value:h}))}function kp({children:e,location:t}){return ix(dc(e),t)}function dc(e,t=[]){let a=[];return Te.Children.forEach(e,(n,r)=>{if(!Te.isValidElement(n))return;let s=[...t,r];if(n.type===Te.Fragment){a.push.apply(a,dc(n.props.children,s));return}Ee(n.type===be,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),Ee(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=dc(n.props.children,s)),a.push(i)}),a}var uc="get",cc="application/x-www-form-urlencoded";function mc(e){return e!=null&&typeof e.tagName=="string"}function z3(e){return mc(e)&&e.tagName.toLowerCase()==="button"}function q3(e){return mc(e)&&e.tagName.toLowerCase()==="form"}function I3(e){return mc(e)&&e.tagName.toLowerCase()==="input"}function K3(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function H3(e,t){return e.button===0&&(!t||t==="_self")&&!K3(e)}var oc=null;function Q3(){if(oc===null)try{new FormData(document.createElement("form"),0),oc=!1}catch{oc=!0}return oc}var V3=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function cp(e){return e!=null&&!V3.has(e)?(ta(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${cc}"`),null):e}function G3(e,t){let a,n,r,s,i;if(q3(e)){let o=e.getAttribute("action");n=o?Ya(o,t):null,a=e.getAttribute("method")||uc,r=cp(e.getAttribute("enctype"))||cc,s=new FormData(e)}else if(z3(e)||I3(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let u=e.getAttribute("formaction")||o.getAttribute("action");if(n=u?Ya(u,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||uc,r=cp(e.getAttribute("formenctype"))||cp(o.getAttribute("enctype"))||cc,s=new FormData(o,e),!Q3()){let{name:c,type:d,value:m}=e;if(d==="image"){let f=c?`${c}.`:"";s.append(`${f}x`,"0"),s.append(`${f}y`,"0")}else c&&s.append(c,m)}}else{if(mc(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=uc,n=null,r=cc,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var H6=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");function Cp(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var Y3=Symbol("SingleFetchRedirect");function J3(e,t,a){let n=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return n.pathname==="/"?n.pathname=`_root.${a}`:t&&Ya(n.pathname,t)==="/"?n.pathname=`${t.replace(/\/$/,"")}/_root.${a}`:n.pathname=`${n.pathname.replace(/\/$/,"")}.${a}`,n}async function X3(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function Z3(e){return e!=null&&typeof e.page=="string"}function W3(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function eT(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await X3(s,a);return i.links?i.links():[]}return[]}));return rT(n.flat(1).filter(W3).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function V0(e,t,a,n,r,s){let i=(u,c)=>a[c]?u.route.id!==a[c].route.id:!0,o=(u,c)=>a[c].pathname!==u.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==u.params["*"];return s==="assets"?t.filter((u,c)=>i(u,c)||o(u,c)):s==="data"?t.filter((u,c)=>{let d=n.routes[u.route.id];if(!d||!d.hasLoader)return!1;if(i(u,c)||o(u,c))return!0;if(u.route.shouldRevalidate){let m=u.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:u.params,defaultShouldRevalidate:!0});if(typeof m=="boolean")return m}return!0}):[]}function tT(e,t,{includeHydrateFallback:a}={}){return aT(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function aT(e){return[...new Set(e)]}function nT(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function rT(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!Z3(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(nT(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function mx(){let e=ye.useContext(Br);return Cp(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function lT(){let e=ye.useContext(Js);return Cp(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var Qo=ye.createContext(void 0);Qo.displayName="FrameworkContext";function fx(){let e=ye.useContext(Qo);return Cp(e,"You must render this element inside a <HydratedRouter> element"),e}function uT(e,t){let a=ye.useContext(Qo),[n,r]=ye.useState(!1),[s,i]=ye.useState(!1),{onFocus:o,onBlur:u,onMouseEnter:c,onMouseLeave:d,onTouchStart:m}=t,f=ye.useRef(null);ye.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let y=g=>{g.forEach(v=>{i(v.isIntersecting)})},$=new IntersectionObserver(y,{threshold:.5});return f.current&&$.observe(f.current),()=>{$.disconnect()}}},[e]),ye.useEffect(()=>{if(n){let y=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(y)}}},[n]);let h=()=>{r(!0)},x=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,f,{}]:[s,f,{onFocus:Ko(o,h),onBlur:Ko(u,x),onMouseEnter:Ko(c,h),onMouseLeave:Ko(d,x),onTouchStart:Ko(m,h)}]:[!1,f,{}]}function Ko(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function px({page:e,...t}){let{router:a}=mx(),n=ye.useMemo(()=>hp(a.routes,e,a.basename),[a.routes,e,a.basename]);return n?ye.createElement(dT,{page:e,matches:n,...t}):null}function cT(e){let{manifest:t,routeModules:a}=fx(),[n,r]=ye.useState([]);return ye.useEffect(()=>{let s=!1;return eT(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function dT({page:e,matches:t,...a}){let n=Pe(),{manifest:r,routeModules:s}=fx(),{basename:i}=mx(),{loaderData:o,matches:u}=lT(),c=ye.useMemo(()=>V0(e,t,u,r,n,"data"),[e,t,u,r,n]),d=ye.useMemo(()=>V0(e,t,u,r,n,"assets"),[e,t,u,r,n]),m=ye.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let x=new Set,y=!1;if(t.forEach(g=>{let v=r.routes[g.route.id];!v||!v.hasLoader||(!c.some(b=>b.route.id===g.route.id)&&g.route.id in o&&s[g.route.id]?.shouldRevalidate||v.hasClientLoader?y=!0:x.add(g.route.id))}),x.size===0)return[];let $=J3(e,i,"data");return y&&x.size>0&&$.searchParams.set("_routes",t.filter(g=>x.has(g.route.id)).map(g=>g.route.id).join(",")),[$.pathname+$.search]},[i,o,n,r,c,t,e,s]),f=ye.useMemo(()=>tT(d,r),[d,r]),h=cT(d);return ye.createElement(ye.Fragment,null,m.map(x=>ye.createElement("link",{key:x,rel:"prefetch",as:"fetch",href:x,...a})),f.map(x=>ye.createElement("link",{key:x,rel:"modulepreload",href:x,...a})),h.map(({key:x,link:y})=>ye.createElement("link",{key:x,nonce:a.nonce,...y})))}function mT(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var hx=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{hx&&(window.__reactRouterVersion="7.9.1")}catch{}function Ep({basename:e,children:t,window:a}){let n=ee.useRef();n.current==null&&(n.current=G0({window:a,v5Compat:!0}));let r=n.current,[s,i]=ee.useState({action:r.action,location:r.location}),o=ee.useCallback(u=>{ee.startTransition(()=>i(u))},[i]);return ee.useLayoutEffect(()=>r.listen(o),[r,o]),ee.createElement(_p,{basename:e,children:t,location:s.location,navigationType:s.action,navigator:r})}function vx({basename:e,children:t,history:a}){let[n,r]=ee.useState({action:a.action,location:a.location}),s=ee.useCallback(i=>{ee.startTransition(()=>r(i))},[r]);return ee.useLayoutEffect(()=>a.listen(s),[a,s]),ee.createElement(_p,{basename:e,children:t,location:n.location,navigationType:n.action,navigator:a})}vx.displayName="unstable_HistoryRouter";var gx=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i,Sn=ee.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,state:o,target:u,to:c,preventScrollReset:d,viewTransition:m,...f},h){let{basename:x}=ee.useContext(Ft),y=typeof c=="string"&&gx.test(c),$,g=!1;if(typeof c=="string"&&y&&($=c,hx))try{let O=new URL(window.location.href),L=c.startsWith("//")?new URL(O.protocol+c):new URL(c),P=Ya(L.pathname,x);L.origin===O.origin&&P!=null?c=P+L.search+L.hash:g=!0}catch{ta(!1,`<Link to="${c}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}let v=tx(c,{relative:r}),[b,w,S]=uT(n,f),E=$x(c,{replace:i,state:o,target:u,preventScrollReset:d,relative:r,viewTransition:m});function k(O){t&&t(O),O.defaultPrevented||E(O)}let C=ee.createElement("a",{...f,...S,href:$||v,onClick:g||s?t:k,ref:mT(h,w),target:u,"data-discover":!y&&a==="render"?"true":void 0});return b&&!y?ee.createElement(ee.Fragment,null,C,ee.createElement(px,{page:v})):C});Sn.displayName="Link";var Ja=ee.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:u,...c},d){let m=Zs(i,{relative:c.relative}),f=Pe(),h=ee.useContext(Js),{navigator:x,basename:y}=ee.useContext(Ft),$=h!=null&&_x(m)&&o===!0,g=x.encodeLocation?x.encodeLocation(m).pathname:m.pathname,v=f.pathname,b=h&&h.navigation&&h.navigation.location?h.navigation.location.pathname:null;a||(v=v.toLowerCase(),b=b?b.toLowerCase():null,g=g.toLowerCase()),b&&y&&(b=Ya(b,y)||b);let w=g!=="/"&&g.endsWith("/")?g.length-1:g.length,S=v===g||!r&&v.startsWith(g)&&v.charAt(w)==="/",E=b!=null&&(b===g||!r&&b.startsWith(g)&&b.charAt(g.length)==="/"),k={isActive:S,isPending:E,isTransitioning:$},C=S?t:void 0,O;typeof n=="function"?O=n(k):O=[n,S?"active":null,E?"pending":null,$?"transitioning":null].filter(Boolean).join(" ");let L=typeof s=="function"?s(k):s;return ee.createElement(Sn,{...c,"aria-current":C,className:O,ref:d,style:L,to:i,viewTransition:o},typeof u=="function"?u(k):u)});Ja.displayName="NavLink";var yx=ee.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=uc,action:o,onSubmit:u,relative:c,preventScrollReset:d,viewTransition:m,...f},h)=>{let x=wx(),y=Sx(o,{relative:c}),$=i.toLowerCase()==="get"?"get":"post",g=typeof o=="string"&&gx.test(o);return ee.createElement("form",{ref:h,method:$,action:y,onSubmit:n?u:b=>{if(u&&u(b),b.defaultPrevented)return;b.preventDefault();let w=b.nativeEvent.submitter,S=w?.getAttribute("formmethod")||i;x(w||b.currentTarget,{fetcherKey:t,method:S,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:m})},...f,"data-discover":!g&&e==="render"?"true":void 0})});yx.displayName="Form";function bx({getKey:e,storageKey:t,...a}){let n=ee.useContext(Qo),{basename:r}=ee.useContext(Ft),s=Pe(),i=Sp();Nx({getKey:e,storageKey:t});let o=ee.useMemo(()=>{if(!n||!e)return null;let c=pp(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let u=((c,d)=>{if(!window.history.state||!window.history.state.key){let m=Math.random().toString(32).slice(2);window.history.replaceState({key:m},"")}try{let f=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof f=="number"&&window.scrollTo(0,f)}catch(m){console.error(m),sessionStorage.removeItem(c)}}).toString();return ee.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${u})(${JSON.stringify(t||fp)}, ${JSON.stringify(o)})`}})}bx.displayName="ScrollRestoration";function xx(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function Tp(e){let t=ee.useContext(Br);return Ee(t,xx(e)),t}function fT(e){let t=ee.useContext(Js);return Ee(t,xx(e)),t}function $x(e,{target:t,replace:a,state:n,preventScrollReset:r,relative:s,viewTransition:i}={}){let o=pe(),u=Pe(),c=Zs(e,{relative:s});return ee.useCallback(d=>{if(H3(d,t)){d.preventDefault();let m=a!==void 0?a:Ys(u)===Ys(c);o(e,{replace:m,state:n,preventScrollReset:r,relative:s,viewTransition:i})}},[u,o,c,a,n,t,e,r,s,i])}var pT=0,hT=()=>`__${String(++pT)}__`;function wx(){let{router:e}=Tp("useSubmit"),{basename:t}=ee.useContext(Ft),a=j3();return ee.useCallback(async(n,r={})=>{let{action:s,method:i,encType:o,formData:u,body:c}=G3(n,t);if(r.navigate===!1){let d=r.fetcherKey||hT();await e.fetch(d,a,r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,flushSync:r.flushSync})}else await e.navigate(r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,replace:r.replace,state:r.state,fromRouteId:a,flushSync:r.flushSync,viewTransition:r.viewTransition})},[e,t,a])}function Sx(e,{relative:t}={}){let{basename:a}=ee.useContext(Ft),n=ee.useContext(na);Ee(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...Zs(e||".",{relative:t})},i=Pe();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),u=o.getAll("index");if(u.some(d=>d==="")){o.delete("index"),u.filter(m=>m).forEach(m=>o.append("index",m));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:wn([a,s.pathname])),Ys(s)}var fp="react-router-scroll-positions",lc={};function pp(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:Ya(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function Nx({getKey:e,storageKey:t}={}){let{router:a}=Tp("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=fT("useScrollRestoration"),{basename:s}=ee.useContext(Ft),i=Pe(),o=Sp(),u=lx();ee.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),vT(ee.useCallback(()=>{if(u.state==="idle"){let c=pp(i,o,s,e);lc[c]=window.scrollY}try{sessionStorage.setItem(t||fp,JSON.stringify(lc))}catch(c){ta(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[u.state,e,s,i,o,t])),typeof document<"u"&&(ee.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||fp);c&&(lc=JSON.parse(c))}catch{}},[t]),ee.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(lc,()=>window.scrollY,e?(d,m)=>pp(d,m,s,e):void 0);return()=>c&&c()},[a,s,e]),ee.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{ta(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function vT(e,t){let{capture:a}=t||{};ee.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function _x(e,{relative:t}={}){let a=ee.useContext(yp);Ee(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=Tp("useViewTransitionState"),r=Zs(e,{relative:t});if(!a.isTransitioning)return!1;let s=Ya(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=Ya(a.nextLocation.pathname,n)||a.nextLocation.pathname;return Ho(r.pathname,i)!=null||Ho(r.pathname,s)!=null}var Ct=new Ed({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var Ap="ironclaw_token",Ie="/api/webchat/v2",qr=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function xa(){return sessionStorage.getItem(Ap)||""}function Ws(e){e?sessionStorage.setItem(Ap,e):sessionStorage.removeItem(Ap)}function fc(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function Cx(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function Rx(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function Ex({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=Rx(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=Rx(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function Q(e,t={}){let a=xa(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await Cx(r);throw new qr(Ex({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function pc(){return Q(`${Ie}/session`)}function hc({clientActionId:e,requestedThreadId:t,projectId:a}={}){let n={client_action_id:e||fc()};return t&&(n.requested_thread_id=t),a&&(n.project_id=a),Q(`${Ie}/threads`,{method:"POST",body:JSON.stringify(n)})}function Tx({limit:e,cursor:t}={}){let a=new URL(`${Ie}/threads`,window.location.origin);return e!=null&&a.searchParams.set("limit",String(e)),t&&a.searchParams.set("cursor",t),Q(a.pathname+a.search)}function Ax({threadId:e}={}){return e?Q(`${Ie}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function Dp(e){return`${Ie}/threads/${encodeURIComponent(e)}/files`}function Dx({threadId:e,path:t}={}){if(!e)return Promise.reject(new Error("threadId is required"));let a=new URL(Dp(e),window.location.origin);return t&&a.searchParams.set("path",t),Q(a.pathname+a.search)}function Mx({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${Dp(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),Q(a.pathname+a.search)}function vc({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${Dp(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function Ox({limit:e,runLimit:t,includeCompleted:a}={}){let n=new URLSearchParams;e!=null&&n.set("limit",String(e)),t!=null&&n.set("run_limit",String(t)),a===!0&&n.set("include_completed","true");let r=n.toString();return Q(`${Ie}/automations${r?`?${r}`:""}`)}function Lx({automationId:e}={}){return e?Q(`${Ie}/automations/${encodeURIComponent(e)}/pause`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function Px({automationId:e}={}){return e?Q(`${Ie}/automations/${encodeURIComponent(e)}/resume`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function Ux({automationId:e}={}){return e?Q(`${Ie}/automations/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("automationId is required"))}var jx=`${Ie}/projects`;function gT(e){return`${jx}/${encodeURIComponent(e)}`}function Fx({limit:e}={}){let t=new URL(jx,window.location.origin);return e!=null&&t.searchParams.set("limit",String(e)),Q(t.pathname+t.search)}function Bx({projectId:e}={}){return e?Q(gT(e)):Promise.reject(new Error("projectId is required"))}function zx(){return Q(`${Ie}/outbound/preferences`)}function qx(){return Q(`${Ie}/outbound/targets`)}function Ix({finalReplyTargetId:e}={}){return Q(`${Ie}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function Mp({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:u,source:c,tail:d,follow:m}={}){let f=new URL(`${Ie}/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),u&&f.searchParams.set("tool_name",u),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),Q(f.pathname+f.search)}function Kx({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:u,source:c,tail:d,follow:m}={}){let f=new URL(`${Ie}/operator/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),u&&f.searchParams.set("tool_name",u),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),Q(f.pathname+f.search)}function Hx({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||fc(),content:t};return a.length>0&&(r.attachments=a),Q(`${Ie}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function Qx({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${Ie}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),Q(n.pathname+n.search)}function Vx({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${Ie}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function Ca(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new qr("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=xa(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await Cx(r);throw new qr(Ex({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function Op(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function gc(e){return Op(await Ca(e))}function Gx({threadId:e,afterCursor:t}={}){let a=new URL(`${Ie}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=xa();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function Yx({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||fc()};return a&&(r.reason=a),Q(`${Ie}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function Lp({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let u={client_action_id:i||fc(),resolution:n};return r!=null&&(u.always=r),s&&(u.credential_ref=s),Q(`${Ie}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(u)})}function Jx({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return Q("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function Xx(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),Q(`${Ie}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function ei(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function Zx(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function Wx(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new qr("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new qr("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function e$(){let e=xa();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var yc="anon",t$=yc;function a$(e){t$=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:yc}function $t(){return t$}var n$="ironclaw:v2-thread-pins:",Pp=new Set,Nn=new Set,Up=null;function jp(){return`${n$}${$t()}`}function yT(){try{let e=window.localStorage.getItem(jp());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function bT(){try{Nn.size===0?window.localStorage.removeItem(jp()):window.localStorage.setItem(jp(),JSON.stringify([...Nn]))}catch{}}function r$(){let e=$t();if(e!==Up){Nn.clear();for(let t of yT())Nn.add(t);Up=e}}function s$(){return new Set(Nn)}function i$(){let e=s$();for(let t of Pp)try{t(e)}catch{}}function o$(e){e&&(r$(),Nn.has(e)?Nn.delete(e):Nn.add(e),bT(),i$())}function l$(){return r$(),s$()}function u$(e){return Pp.add(e),()=>{Pp.delete(e)}}function c$(){Nn.clear(),Up=$t();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(n$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}i$()}var xT=0,Ir={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function Fp(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function d$(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":$T(t)?"text":"download"}function $T(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function Vo(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function wT(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function ST(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function NT(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function m$(e,{limits:t,existing:a=[],t:n}){let r=t||Ir,s=[],i=[],o=a.length,u=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!wT(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:Vo(r.maxFileBytes)}));continue}if(u+c.size>r.maxTotalBytes){let y=n("chat.attachmentTotalTooLarge",{max:Vo(r.maxTotalBytes)});i.includes(y)||i.push(y);continue}let d;try{d=await ST(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:m,base64:f}=NT(d,c.type),h=m||"application/octet-stream",x=Fp(h);s.push({id:`staged-${xT++}`,filename:c.name||"attachment",mimeType:h,kind:x,sizeBytes:c.size,sizeLabel:Vo(c.size),dataBase64:f,previewUrl:x==="image"?d:null}),o+=1,u+=c.size}return{staged:s,errors:i}}function f$(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function p$(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}var _T="(files attached)";function kT(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||Fp(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?Vx({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?Vo(n.size_bytes):"",preview_url:null,fetch_url:s}})}function v$(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let m=TT(s);if(!m)continue;let f=`tool-${m.invocationId}`;if(n.has(f))continue;n.add(f),r.push({id:f,role:"tool_activity",...m,timestamp:h$(s)||m.updatedAt||null,sequence:s.sequence,activityOrder:m.activityOrder,activityOrderSource:m.activityOrderSource,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=ET(s),u=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy"),c=kT(s,a),d=o==="user"&&c?.length>0&&s.content===_T?"":s.content||"";r.push({id:i,role:o,content:d,attachments:c,timestamp:h$(s),kind:s.kind,status:u?"error":s.status,...u&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:CT(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=RT(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function RT(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function CT(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function ET(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function h$(e){return e.received_at||e.created_at||null}function TT(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:Bp(t)}var AT="gate_declined";function Bp(e){let t=e.status==="failed"||e.status==="killed",a=e.error_kind||null,n=b$(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Yo(e.title||e.capability_id)||"tool",toolStatus:y$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(g$(a)||e.output_summary||e.output_preview||e.result_ref)||null,toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null,activityOrder:n,activityOrderSource:Number.isFinite(n)?"projection":null}}function zp(e){let t=b$(e.activity_order),a=e.error_kind||null;return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Yo(e.capability_id)||"tool",toolStatus:y$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:null,toolError:g$(a),toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null,activityOrder:t,activityOrderSource:Number.isFinite(t)?"projection":null}}function g$(e){return e||null}function Go(e){return e==="success"||e==="error"||e==="declined"}function Yo(e){let t=typeof e=="string"?e.trim():"";if(!t)return"";let a=t.split(".");return a[a.length-1]||t}function y$(e,t=null){if(t===AT)return"declined";switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}function b$(e){let t=Number(e);return Number.isFinite(t)?t:null}var DT=50,Xa=new Map,MT=30;function Jo(e,t){for(Xa.delete(e),Xa.set(e,t);Xa.size>MT;){let a=Xa.keys().next().value;Xa.delete(a)}}function Xo(e){return`${$t()}:${e}`}function $$(){Xa.clear()}function w$(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?Xa.get(Xo(e)):null,[s,i]=p.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),o=p.default.useRef(new Set),u=p.default.useRef(e);u.current=e;let c=p.default.useCallback(async(m,f={})=>{let{preserveClientOnly:h=!1,finalReplyTimestampByRun:x=null}=f;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(o.current.has(e))return;o.current.add(e);let y=$t(),$=Xo(e);i(g=>({...g,isLoading:!0}));try{let g=await Qx({threadId:e,limit:DT,cursor:m});if($t()!==y)return;let v=m?[]:a?.()||[],b=v$(g.messages||[],v,e),w=g.next_cursor||null;if(m||n?.([]),!m){let S=Xa.get($)?.messages||[],E=x$(b,S,{preserveClientOnly:h,finalReplyTimestampByRun:x});Jo($,{messages:E,nextCursor:w})}i(S=>{if(u.current!==e)return S;let E;return m?E=OT(b,S.messages):E=x$(b,S.messages,{preserveClientOnly:h,finalReplyTimestampByRun:x}),Jo($,{messages:E,nextCursor:w}),{messages:E,nextCursor:w,isLoading:!1,loadError:null}})}catch(g){if(console.error("Failed to load timeline:",g),$t()!==y)return;i(v=>u.current===e?{...v,isLoading:!1,loadError:"Failed to load conversation history."}:v)}finally{o.current.delete(e)}},[e,a,n]);p.default.useEffect(()=>{let m=e?Xa.get(Xo(e)):null;i({messages:m?.messages||[],nextCursor:m?.nextCursor||null,isLoading:!!e&&!m,loadError:null}),e&&c()},[e,c]);let d=p.default.useCallback((m,f)=>{if(!m)return;let h=Xo(m),x=g=>typeof f=="function"?f(g||[]):f;if(u.current===m){i(g=>{let v=x(g.messages||[]);return Jo(h,{messages:v,nextCursor:g.nextCursor||null}),{...g,messages:v}});return}let y=Xa.get(h)||{messages:[],nextCursor:null},$=x(y.messages||[]);Jo(h,{messages:$,nextCursor:y.nextCursor||null})},[]);return{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:c,seedThreadMessages:d,setMessages:m=>i(f=>{let h=typeof m=="function"?m(f.messages):m;return e&&Jo(Xo(e),{messages:h,nextCursor:f.nextCursor}),{...f,messages:h}})}}function OT(e,t){let a=new Set(t.map(n=>n?.id).filter(Boolean));return[...e.filter(n=>!a.has(n?.id)),...t]}function x$(e,t,a={}){let{preserveClientOnly:n=!1,finalReplyTimestampByRun:r=null}=a,s=PT(e,t,{finalReplyTimestampByRun:r}),i=new Set(s.map(u=>u?.id).filter(Boolean)),o=t.filter(u=>!u||typeof u.id!="string"||i.has(u.id)?!1:UT(u)?!0:typeof u.timelineMessageId=="string"&&i.has(`msg-${u.timelineMessageId}`)?!1:LT(u)?!0:n&&u.id.startsWith("err-"));return o.length>0?[...s,...o]:s}function LT(e){return e?.isOptimistic===!0&&typeof e.id=="string"&&e.id.startsWith("pending-")&&(e.role==="user"||e.role==="assistant")}function PT(e,t,a={}){let{finalReplyTimestampByRun:n=null}=a,r=new Map,s=new Map;for(let i of t||[])!i||!i.timestamp||(typeof i.id=="string"&&r.set(i.id,i),typeof i.timelineMessageId=="string"&&r.set(`msg-${i.timelineMessageId}`,i),qp(i)&&typeof i.turnRunId=="string"&&s.set(i.turnRunId,i));return r.size===0&&s.size===0&&!n?e:e.map(i=>{if(!i||i.timestamp||typeof i.id!="string")return i;let o=typeof i.turnRunId=="string"?i.turnRunId:null,u=r.get(i.id)||(qp(i)&&o?s.get(o):null),c=qp(i)&&o?n?.[o]:null,d=u?.timestamp||c;return d?{...i,timestamp:d}:i})}function qp(e){return e?.role==="assistant"&&e?.isFinalReply===!0}function UT(e){return e?.role==="tool_activity"||e?.role==="thinking"}var Wo="__new__",S$="ironclaw:v2-draft:";function ti(e){return`${S$}${$t()}:${e||Wo}`}function Ip(e){try{return window.localStorage.getItem(ti(e))||""}catch{return""}}function Kp(e,t){try{t?window.localStorage.setItem(ti(e),t):window.localStorage.removeItem(ti(e))}catch{}}function N$(e){Kp(e,"")}var Zo=new Map;function Hp(e){return Zo.get(ti(e))||[]}function _$(e,t){let a=ti(e);t&&t.length>0?Zo.set(a,t):Zo.delete(a)}function k$(e){Zo.delete(ti(e))}function R$(){Zo.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(S$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function jT(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function FT(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function BT(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=jT(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?FT(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),xa()?"":(Ws(n),n)}function zT(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var qT={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function IT(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),qT[t]||"Could not complete sign-in. Please try again."):""}function C$(){let[e,t]=p.default.useState(()=>BT()||xa()),[a,n]=p.default.useState(()=>IT()),[r]=p.default.useState(()=>zT()),[s,i]=p.default.useState(null),[o,u]=p.default.useState(()=>!!(r&&!xa())),[c,d]=p.default.useState(()=>!!xa());p.default.useEffect(()=>{if(!r||xa()){u(!1);return}let x=!1;return Wx(r).then(y=>{x||(Ws(y),d(!0),t(y),i(null),n(""),u(!1),Ct.clear())}).catch(()=>{x||(n("Could not complete sign-in. Please try again."),u(!1))}),()=>{x=!0}},[r]),p.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let x=!1;return d(!0),pc().then(y=>{x||(i(y),d(!1))}).catch(y=>{x||(i(null),d(!1),(y?.status===401||y?.status===403)&&(Ws(""),t(""),n("Your session expired. Please sign in again."),Ct.clear()))}),()=>{x=!0}},[e,o]),a$(s);let m=p.default.useRef(null);p.default.useEffect(()=>{let x=$t();m.current&&m.current!==yc&&m.current!==x&&($$(),R$(),c$()),m.current=x},[s]);let f=p.default.useCallback(x=>{Ws(x),d(!!x),t(x),i(null),n(""),Ct.clear()},[]),h=p.default.useCallback(()=>{e$().catch(()=>{}),Ws(""),d(!1),t(""),i(null),n(""),Ct.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,rebornProjectsEnabled:!!s?.features?.reborn_projects,signIn:f,signOut:h}}var Kr="/chat",el=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace"},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"logs",path:"/logs",labelKey:"nav.logs",hidden:!0},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var KT=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"tools",labelKey:"settings.tools",icon:"tool"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],HT=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],QT=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],bc={settings:KT,extensions:HT,admin:QT};var E$="ironclaw:v2-theme";function VT(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(E$);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function xc(){let[e,t]=p.default.useState(VT);p.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(E$,e)}catch{}},[e]);let a=p.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function T$(e){return H({enabled:!!e,queryKey:["gateway-status",e],queryFn:ei,refetchInterval:3e4})}var GT="/api/webchat/v2/operator/config",$c="/api/webchat/v2/settings/tools",ai="agent.auto_approve_tools",A$="tool.",YT=new Set(["always_allow","ask_each_time","disabled"]),JT=new Set(["default","always_allow","ask_each_time","disabled"]);function D$(e){return e==="ask"?"ask_each_time":YT.has(e)?e:"ask_each_time"}function XT(e){return e==="ask"?"ask_each_time":JT.has(e)?e:"default"}function ZT(e){return["default","global","override"].includes(e)?e:"default"}function M$(e){if(!e?.key?.startsWith(A$))return null;let t=e.value||{};return{name:t.name||e.key.slice(A$.length),description:t.description||"",state:D$(t.state),default_state:D$(t.default_state),locked:!!(t.locked||e.mutable===!1),effective_source:ZT(t.effective_source||e.source)}}function WT(e){let t={};for(let a of e.entries||[])a?.key===ai&&(t[ai]=!!a.value);return t}async function O$(){let e=await Q($c);return{settings:WT(e),diagnostics:e.diagnostics||[],precedence:e.precedence||[]}}async function Qp(e,t){if(e===ai){let n=await Q($c,{method:"POST",body:JSON.stringify({enabled:!!t})});return{success:!0,entry:n.entry,value:n.entry?.value}}let a=await Q(`${GT}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({value:t})});return{success:!0,entry:a.entry,value:a.entry?.value}}async function L$(e){let t=e?.settings||{},a=[];return Object.prototype.hasOwnProperty.call(t,ai)&&a.push(await Qp(ai,!!t[ai])),{success:!0,imported:a.length,results:a}}function wc(){return Q("/api/webchat/v2/llm/providers")}function P$(e){return Q("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function U$(e){return Q(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function tl(e){return Q("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function j$(e){return Q("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function F$(e){return Q("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function B$(e){return Q("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function z$(e){return Q("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function q$(){return Q("/api/webchat/v2/llm/codex/login",{method:"POST"})}async function I$(){let e=await Q($c);return{tools:(e.entries||[]).map(M$).filter(Boolean),diagnostics:e.diagnostics||[]}}async function K$(e,t){let a=XT(t),n=await Q(`${$c}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({state:a})});return{success:!0,tool:M$(n.entry),entry:n.entry}}function H$(){return Q("/api/webchat/v2/extensions")}function Q$(){return Q("/api/webchat/v2/extensions/registry")}function V$(){return Q("/api/webchat/v2/skills")}function G$(e){return Q(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function Y$(e){return Q("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function J$(e,t){return Q(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function X$(e){return Q(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function Z$(e,t){return Q(`/api/webchat/v2/skills/${encodeURIComponent(e)}/auto-activate`,{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:t})})}function W$(e){return Q("/api/webchat/v2/skills/auto-activate-learned",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function ew(){return Q("/api/webchat/v2/traces/credit")}function tw(e){return Q(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function aw(){return Promise.resolve({users:[],todo:!0})}function nw(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function rw(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var Vp="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",Gp=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function al(e){return Gp.find(t=>t.value===e)?.label||e}function ni(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function sw(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function Sc(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function iw(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function Hr(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Vp||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?ni(e,t).trim().length>0:!0:!1}function eA(e,t,a){return e.id===a?"active":Hr(e,t)?"ready":"setup"}function ow(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=eA(r,t,a);n[s]&&n[s].push(r)}return n}function Nc(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Vp||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!ni(e,t).trim()?"base_url":"ok"}function Yp(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===Vp&&(i.api_key=void 0),i}function lw(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function uw(e){return/^[a-z0-9_-]+$/.test(e)}function cw(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var tA=Object.freeze({});function ri({settings:e,gatewayStatus:t,enabled:a=!0}){let n=X(),r=H({queryKey:["llm-providers"],queryFn:wc,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=tA,u=(s.providers||[]).map(w=>({...w,name:w.description,has_api_key:w.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,m=d||"nearai",f=s.active?.model||t?.llm_model||"",h=u.filter(w=>w.builtin),x=u.filter(w=>!w.builtin),y=[...u].sort((w,S)=>w.id===d?-1:S.id===d?1:(w.name||w.id).localeCompare(S.name||S.id)),$=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=G({mutationFn:async w=>{if(!Hr(w,o)){let E=Nc(w,o);throw new Error(E==="base_url"?"base_url":"api_key")}let S=Sc(w,o);if(!S)throw new Error("model");return await tl({provider_id:w.id,model:S}),w},onSuccess:$}),v=G({mutationFn:async({provider:w,form:S,apiKey:E,editingProvider:k})=>{let C=!!w?.builtin,L={id:(C?w.id:S.id.trim()).trim(),name:C?w.name||w.id:S.name.trim(),adapter:C?w.adapter:S.adapter,base_url:S.baseUrl.trim()||w?.base_url||"",default_model:S.model.trim()||void 0};return E.trim()&&(L.api_key=E.trim()),(k||w)?.id===m&&L.default_model&&(L.set_active=!0,L.model=L.default_model),await P$(L),L},onSuccess:$}),b=G({mutationFn:async w=>(await U$(w.id),w),onSuccess:$});return{providers:y,builtinProviders:h,customProviders:x,builtinOverrides:o,activeProviderId:d,selectedModel:f,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:w=>g.mutateAsync(w),saveCustomProvider:w=>v.mutateAsync(w),saveBuiltinProvider:w=>v.mutateAsync(w),deleteCustomProvider:w=>b.mutateAsync(w),testConnection:j$,listModels:F$,isBusy:g.isPending||v.isPending||b.isPending}}function dw({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}var mw="ironclaw:v2-sidebar-open";function fw(){return typeof window>"u"?null:window}function pw(){try{return fw()?.localStorage||null}catch{return null}}function hw(e=pw()){try{return e?.getItem(mw)!=="false"}catch{return!0}}function vw(e,t=pw()){try{t?.setItem(mw,e?"true":"false")}catch{}}function gw(e=fw()){try{return e?.matchMedia?.("(min-width: 768px)").matches===!0}catch{return!1}}function yw(e,t){return t?{...e,desktopOpen:!e.desktopOpen}:{...e,mobileOpen:!e.mobileOpen}}function bw(e,t){return t?e.desktopOpen:e.mobileOpen}function xw({onNewChat:e}={}){let t=pe(),[a,n]=p.default.useState(()=>({mobileOpen:!1,desktopOpen:hw()})),[r,s]=p.default.useState(()=>gw());p.default.useEffect(()=>{let d=window.matchMedia("(min-width: 768px)"),m=()=>s(d.matches);return m(),d.addEventListener?.("change",m),()=>d.removeEventListener?.("change",m)},[]),p.default.useEffect(()=>{vw(a.desktopOpen)},[a.desktopOpen]);let i=p.default.useCallback(()=>{n(d=>({...d,mobileOpen:!1}))},[]),o=p.default.useCallback(()=>{n(d=>yw(d,r))},[r]),u=p.default.useCallback(async()=>{let d=await e?.(),m=typeof d=="string"&&d.length>0?d:null;t(m?`/chat/${m}`:"/chat"),i()},[t,i,e]),c=p.default.useCallback(d=>{t(`/chat/${d}`),i()},[t,i]);return{mobileOpen:a.mobileOpen,desktopOpen:a.desktopOpen,currentOpen:bw(a,r),close:i,toggle:o,newChat:u,selectThread:c}}var Jp=new Set,aA=0;function si(e,t={}){let a={id:++aA,message:e,tone:t.tone||"info",duration:t.duration??2600};return Jp.forEach(n=>n(a)),a.id}function $w(e){return Jp.add(e),()=>Jp.delete(e)}function nA(e){return e?.status===409&&e?.payload?.kind==="busy"}function ww(e,t){return nA(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function Sw(){let e=H({queryKey:["threads"],queryFn:()=>Tx({})}),[t,a]=p.default.useState(null),[n,r]=p.default.useState(!1),s=p.default.useRef(new Map),i=p.default.useCallback(async c=>{let d=c||"__global__",m=s.current.get(d);if(m)return m;r(!0);let f=(async()=>{try{let h=await hc(c?{projectId:c}:void 0);Ct.invalidateQueries({queryKey:["threads"]});let x=h?.thread?.thread_id;return x&&a(x),x}finally{s.current.delete(d),r(s.current.size>0)}})();return s.current.set(d,f),f},[]),o=p.default.useCallback(async c=>{await Ax({threadId:c}),t===c&&a(null),Ct.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:p.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var Nw={attach:l`<path
    d="m21.4 11.1-9.2 9.2a6 6 0 0 1-8.5-8.5l9.2-9.2a4 4 0 0 1 5.7 5.7l-9.2 9.2a2 2 0 0 1-2.8-2.8l8.5-8.5"
  />`,bolt:l`<path d="M13 2.8 5.8 13h5.1L10 21.2 18.2 10h-5.4L13 2.8Z" />`,calendar:l`<path d="M6.5 4.5v3M17.5 4.5v3" /><path
      d="M4.5 7h15v12.5h-15V7Z"
    /><path d="M4.5 10.5h15" /><path d="M8 14h.1M12 14h.1M16 14h.1M8 17h.1M12 17h.1" />`,check:l`<path d="m5 12.5 4.3 4.3L19.2 6.7" />`,chat:l`<path d="M5 5.5h14v10H9.4L5 19.2V5.5Z" /><path
      d="M8.4 9h7.2M8.4 12.2h4.8"
    />`,close:l`<path d="m6.5 6.5 11 11M17.5 6.5l-11 11" />`,clock:l`<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17Z" /><path
      d="M12 7.5v5l3.2 2"
    />`,download:l`<path d="M12 3.8v10" /><path d="m8 10 4 4 4-4" /><path
      d="M5 17.5v2.7h14v-2.7"
    />`,file:l`<path d="M6.5 3.5h7.2L18 7.8v12.7H6.5v-17Z" /><path
      d="M13.7 3.5V8H18"
    />`,flag:l`<path d="M6.5 21V4.5" /><path d="M6.5 5h10.7l-1.4 4 1.4 4H6.5" />`,pin:l`<path d="M9 3.5h6l-1 5 3 3.5H7l3-3.5-1-5Z" /><path d="M12 15.5V21" />`,pause:l`<path d="M8.5 5.5v13" /><path d="M15.5 5.5v13" />`,play:l`<path d="M8 5.5 18.5 12 8 18.5V5.5Z" />`,folder:l`<path
    d="M3.5 7h6.2l1.9 2h8.9v9.2a2.3 2.3 0 0 1-2.3 2.3H5.8a2.3 2.3 0 0 1-2.3-2.3V7Z"
  />`,layers:l`<path d="m12 3.7 8.5 4.2-8.5 4.4-8.5-4.4L12 3.7Z" /><path
      d="m5.2 11.2 6.8 3.5 6.8-3.5"
    /><path d="m5.2 14.8 6.8 3.5 6.8-3.5" />`,list:l`<path d="M8.5 6.5h11M8.5 12h11M8.5 17.5h11" /><path
      d="M4.5 6.5h.1M4.5 12h.1M4.5 17.5h.1"
    />`,lock:l`<path d="M7.5 10V7.2a4.5 4.5 0 0 1 9 0V10" /><path
      d="M5.5 10h13v10.5h-13V10Z"
    /><path d="M12 14.4v2.3" />`,logout:l`<path d="M10 17 15 12l-5-5" /><path d="M15 12H3.5" /><path
      d="M14.5 4.5H19a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2h-4.5"
    />`,moon:l`<path
    d="M20.2 14.7A7.7 7.7 0 0 1 9.3 3.8 8.4 8.4 0 1 0 20.2 14.7Z"
  />`,plug:l`<path d="M9 3.5v5M15 3.5v5" /><path
      d="M7.5 8.5h9v3.2a4.5 4.5 0 0 1-9 0V8.5Z"
    /><path d="M12 16.2v4.3" />`,plus:l`<path d="M12 5.5v13M5.5 12h13" />`,pulse:l`<path d="M3.5 12h4l2-5.5 4.2 11 2.2-5.5h4.6" />`,send:l`<path d="M4 11.8 20 4l-4.8 16-3.2-6.8L4 11.8Z" /><path
      d="m12 13.2 4.5-4.6"
    />`,search:l`<path d="M10.8 5.2a5.6 5.6 0 1 1 0 11.2 5.6 5.6 0 0 1 0-11.2Z" /><path
      d="m15.1 15.1 4 4"
    />`,settings:l`
    <path
      d="m19.14 12.94 2.06-1.44-1.73-3-2.47 1a7.07 7.07 0 0 0-1.47-.86L15.12 6h-3.46l-.42 2.64a7.07 7.07 0 0 0-1.47.86l-2.47-1-1.73 3 2.06 1.44a7.1 7.1 0 0 0 0 1.72l-2.06 1.44 1.73 3 2.47-1a7.07 7.07 0 0 0 1.47.86l.42 2.64h3.46l.42-2.64a7.07 7.07 0 0 0 1.47-.86l2.47 1 1.73-3-2.06-1.44a7.1 7.1 0 0 0 0-1.72Z"
    />`,spark:l`<path
    d="M12 3.5 14 10l6.5 2-6.5 2-2 6.5-2-6.5-6.5-2 6.5-2 2-6.5Z"
  />`,sun:l`<path d="M12 7.6a4.4 4.4 0 1 1 0 8.8 4.4 4.4 0 0 1 0-8.8Z" /><path
      d="M12 2.8v2.2M12 19v2.2M4.9 4.9l1.6 1.6M17.5 17.5l1.6 1.6M2.8 12H5M19 12h2.2M4.9 19.1l1.6-1.6M17.5 6.5l1.6-1.6"
    />`,shield:l`<path
      d="M12 3.2 4 7.1v4.5c0 4.7 3.3 8.9 8 10.2 4.7-1.3 8-5.5 8-10.2V7.1l-8-3.9Z"
    /><path d="m9.3 12 2 2 3.8-3.8" />`,tool:l`<path
    d="M15.3 4.4a4.5 4.5 0 0 0-5.7 5.7L4.8 15a2.7 2.7 0 1 0 3.8 3.8l4.9-4.8a4.5 4.5 0 0 0 5.7-5.7l-3.3 3.3-3.2-3.2 2.6-4Z"
  />`,trash:l`<path d="M5.5 7h13" /><path d="M9.5 7V4.5h5V7" /><path
      d="M7.2 7 8 20h8l.8-13"
    /><path d="M10.5 10.5v6M13.5 10.5v6" />`,upload:l`<path d="M12 14.2v-10" /><path d="m8 8.2 4-4 4 4" /><path
      d="M5 17.5v2.7h14v-2.7"
    />`,chevron:l`<path d="m6 9 6 6 6-6" />`,more:l`<path d="M12 5.6h.01M12 12h.01M12 18.4h.01" />`,copy:l`<path d="M9 9h9a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1Z" /><path
      d="M5 15a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h9a1 1 0 0 1 1 1"
    />`,arrowDown:l`<path d="M12 5v14" /><path d="m6 13 6 6 6-6" />`,retry:l`<path d="M3.5 12a8.5 8.5 0 1 1 2.6 6.1" /><path d="M3.2 18.5v-5h5" />`};function M({name:e,className:t="",strokeWidth:a=1.7}){return l`
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
      ${Nw[e]||Nw.spark}
    </svg>
  `}function Y(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=Y(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function _w(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function rA(e){return _w(e).trim().charAt(0).toUpperCase()||"I"}function sA(){let[e,t]=p.default.useState(!1),a=p.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function kw({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=R(),s=sA(),i=_w(a),o=a?.email||a?.role||r("common.gatewaySession");return l`
    <div
      className="relative flex items-center gap-2 border-t border-[var(--v2-panel-border)] px-3 py-3"
    >
      ${s.open&&l`
        <div
          className=${Y("absolute bottom-full left-3 right-3 mb-2 rounded-[10px] border p-3 shadow-lg","border-[var(--v2-panel-border)] bg-[var(--v2-surface)]")}
        >
          <div className="truncate text-sm font-medium text-[var(--v2-text-strong)]">
            ${i}
          </div>
          ${a?.email&&l`<div className="mt-1 truncate text-xs text-[var(--v2-text-muted)]">
            ${a.email}
          </div>`}
          ${a?.role&&l`<div className="mt-2 text-[11px] uppercase text-[var(--v2-text-faint)]">
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
          ${a?.avatar_url?l`<img
              src=${a.avatar_url}
              alt=""
              referrerPolicy="no-referrer"
              className="h-full w-full object-cover"
            />`:l`<span className="place-self-center">${rA(a)}</span>`}
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
  `}var Rw={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",logs:"list",settings:"settings",admin:"shield"},iA=el.filter(e=>e.id!=="chat"&&!e.hidden);function oA({route:e,label:t,onNavigate:a}){return l`
    <${Ja}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>Y("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${M} name=${Rw[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function lA({route:e,label:t,subRoutes:a,onNavigate:n}){let r=R(),s=Pe(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return l`
    <div className="flex flex-col">
      <${Ja}
        to=${o}
        onClick=${n}
        className=${()=>Y("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${M}
          name=${Rw[e.id]||"bolt"}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1 truncate">${t}</span>
        <${M}
          name="chevron"
          className=${Y("h-3.5 w-3.5 shrink-0 transition-transform duration-150",i&&"rotate-180")}
        />
      <//>

      ${i&&l`
        <div className="mt-0.5 flex flex-col gap-0.5 pl-3">
          ${a.map(u=>l`
              <${Ja}
                key=${u.id}
                to=${e.path+"/"+u.id}
                onClick=${n}
                className=${({isActive:c})=>Y("flex items-center gap-2.5 rounded-[8px] py-1.5 pl-7 pr-3 text-[12px] font-medium",c?"text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
              >
                <${M} name=${u.icon} className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate">${r(u.labelKey)}</span>
              <//>
            `)}
        </div>
      `}
    </div>
  `}function Cw({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=R(),s=p.default.useMemo(()=>iA.filter(i=>a||i.id!=="admin"),[a]);return l`
    <div className="flex flex-col px-3 py-2">
      <button
        onClick=${e}
        disabled=${t}
        className=${Y("flex items-center gap-2.5 rounded-[10px] px-3 py-2","border border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]","bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]","hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)] disabled:opacity-50")}
      >
        <${M} name="plus" className="h-4 w-4 shrink-0" />
        <span className="text-[13px] font-medium">
          ${r(t?"chat.creating":"chat.newThread")}
        </span>
      </button>

      <nav className="mt-2 flex flex-col gap-1">
        ${s.map(i=>{let o=(bc[i.id]||[]).filter(u=>a||!(i.id==="settings"&&["users","inference"].includes(u.id)));return o.length>0?l`
              <${lA}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:l`
            <${oA}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var $a=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),nl=new Set([$a.NEEDS_ATTENTION,$a.FAILED]),Xp="ironclaw:v2-thread-attention",Zp=new Set,ii=new Map;function uA(){try{let e=window.localStorage.getItem(Xp);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&nl.has(a[1])):[]}catch{return[]}}function Ew(){let e=[];for(let[t,a]of ii)nl.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(Xp):window.localStorage.setItem(Xp,JSON.stringify(e))}catch{}}for(let[e,t]of uA())ii.set(e,t);function Aw(){return new Map(ii)}function Tw(){let e=Aw();for(let t of Zp)try{t(e)}catch{}}function _c(e,t){if(!e)return;let a=ii.get(e);if(t==null){if(!ii.delete(e))return;nl.has(a)&&Ew(),Tw();return}a!==t&&(ii.set(e,t),(nl.has(t)||nl.has(a))&&Ew(),Tw())}function Dw(e){_c(e,null)}function cA(){return Aw()}function dA(e){return Zp.add(e),()=>{Zp.delete(e)}}function Mw(){let[e,t]=p.default.useState(cA);return p.default.useEffect(()=>dA(t),[]),e}function kc(e){return e.updated_at||e.created_at||null}function Wp(e,t){let a=kc(e)||"",n=kc(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function Ow(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function Lw(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function mA(){let[e,t]=p.default.useState(l$);return p.default.useEffect(()=>u$(t),[]),e}var fA=Object.freeze({[$a.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-transparent"},[$a.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[$a.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function pA(e){return e&&fA[e]||null}function hA(e){let t=String(e?.state||"").toLowerCase();return t==="processing"||t==="running"?$a.RUNNING:t==="awaitingapproval"||t==="awaiting_approval"?$a.NEEDS_ATTENTION:t==="failed"||t==="interrupted"?$a.FAILED:null}function vA({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=R(),o=kc(e),u=Ow(o),c=Lw(o),d=p.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(h=>{window.alert(h?.message||"Unable to delete chat")})},[s,e.id]),m=p.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),o$(e.id)},[e.id]);return l`
    <div
      className=${Y("group flex w-full items-stretch rounded-[8px] border-l-2",n?n.borderClass:t?"border-[var(--v2-accent)]":"border-transparent",t?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
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
          ${n&&l`<span
            aria-label=${n.label}
            className=${Y("h-1.5 w-1.5 shrink-0 rounded-full",n.dotClass)}
          />`}
        </div>
        ${(n||u)&&l`<span
          className=${Y("block truncate text-[11px]",n?n.textClass:"text-[var(--v2-text-faint)]")}
        >
          ${n?n.label:u}
        </span>`}
      </button>
      <button
        type="button"
        onClick=${m}
        title=${i(a?"common.unpin":"common.pin")}
        aria-label=${i(a?"common.unpin":"common.pin")}
        aria-pressed=${a?"true":"false"}
        className=${Y("my-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] transition",a?"text-[var(--v2-accent-text)]":"opacity-0 text-[var(--v2-text-faint)] group-hover:opacity-100 focus:opacity-100","hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-accent-text)]")}
      >
        <${M} name="pin" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>
      ${s&&l`<button
        type="button"
        onClick=${d}
        title=${i("common.deleteChat")}
        aria-label=${i("common.deleteChat")}
        className=${Y("my-1 mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px]","opacity-0 transition group-hover:opacity-100 focus:opacity-100","text-[var(--v2-text-faint)] hover:bg-[var(--v2-danger-soft)] hover:text-[var(--v2-danger-text)]")}
      >
        <${M} name="trash" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>`}
    </div>
  `}function Pw({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:l`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>l`
          <${vA}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${pA(n.get(o.id)||hA(o))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function Uw({threads:e,activeThreadId:t,rebornProjectsEnabled:a=!1,onSelect:n,onDelete:r,onNavigate:s}){let[i,o]=p.default.useState(!1),[u,c]=p.default.useState(""),d=Mw(),m=mA(),f=R(),{pinned:h,recent:x,totalMatches:y}=p.default.useMemo(()=>{let $=u.trim().toLowerCase(),g=$?e.filter(w=>(w.title||w.id||"").toLowerCase().includes($)):e,v=[],b=[];for(let w of g)m.has(w.id)?v.push(w):b.push(w);return v.sort(Wp),b.sort(Wp),{pinned:v,recent:b,totalMatches:v.length+b.length}},[e,u,m]);return l`
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
          className=${Y("h-3.5 w-3.5 text-[var(--v2-text-faint)]",i?"-rotate-90":"")}
          strokeWidth=${2.2}
        />
      </button>

      ${!i&&l`
        ${e.length>0&&l`<div className="relative mb-1 mt-1 px-1">
          <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--v2-text-faint)]">
            <${M} name="search" className="h-3.5 w-3.5" />
          </span>
          <input
            type="text"
            value=${u}
            onInput=${$=>c($.currentTarget.value)}
            placeholder=${f("common.searchChats")}
            className="h-8 w-full rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] pl-8 pr-2 text-[12px] text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]"
          />
        </div>`}
        ${a&&l`<div className="mb-1 px-1">
          <${Ja}
            to="/projects"
            onClick=${s}
            className=${({isActive:$})=>Y("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",$?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
          >
            <${M} name="folder" className="h-4 w-4 shrink-0" />
            <span className="min-w-0 truncate">${f("nav.projects")}</span>
          <//>
        </div>`}
        <div
          className="mt-1 flex flex-col gap-2 overflow-y-auto [scrollbar-width:thin]"
        >
          ${e.length===0&&l`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            ${f("chat.noConversations")}
          </div>`}
          ${e.length>0&&y===0&&l`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            ${f("common.noChatsMatch").replace("{query}",u)}
          </div>`}

          <${Pw}
            label=${f("common.pinned")}
            items=${h}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${m}
            onSelect=${n}
            onDelete=${r}
          />
          <${Pw}
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
  `}function Rc(){let e=X(),t=H({queryKey:["trace-credits"],queryFn:ew,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=G({mutationFn:tw,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function gA(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function jw(){let e=R(),{credits:t}=Rc();if(!t||!t.enrolled)return null;let a=gA(t.final_credit),n=t.submissions_accepted||0,r=t.submissions_submitted||0,s=t.manual_review_hold_count||0;return l`
    <div className="px-3 pb-1">
      <${Sn}
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
          <span className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${a}</span>
        </div>
        <div className="mt-0.5 text-[11px] text-[var(--v2-text-muted)]">
          ${e("traceCommons.cardAccepted",{accepted:n,submitted:r})}
        </div>
        ${s>0&&l`
          <div className="mt-1 text-[11px] font-medium text-[var(--v2-accent-text)]">
            ${e("traceCommons.cardHeld",{count:s})}
          </div>
        `}
      <//>
    </div>
  `}function Fw({id:e,threadsState:t,theme:a,toggleTheme:n,profile:r,isAdmin:s,rebornProjectsEnabled:i=!1,onSignOut:o,onClose:u,onNewChat:c,onSelectThread:d,onDeleteThread:m}){return l`
    <aside
      id=${e}
      className="flex h-full w-[260px] shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
    >
      <div className="flex items-center gap-2.5 px-4 py-5">
        <${Sn}
          to="/chat"
          onClick=${u}
          className="flex items-center gap-2.5 opacity-90 hover:opacity-100"
          aria-label="IronClaw"
        >
          <img src="/v2/assets/logo.jpg" alt="IronClaw" className="h-7 w-auto" />
        <//>
      </div>

      <${Cw}
        onNewChat=${c}
        isCreating=${t.isCreating}
        isAdmin=${s}
        onNavigate=${u}
      />

      <${jw} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${Uw}
          threads=${t.threads}
          activeThreadId=${t.activeThreadId}
          rebornProjectsEnabled=${i}
          onSelect=${d}
          onDelete=${m}
          onNavigate=${u}
        />
      </div>

      <${kw}
        theme=${a}
        toggleTheme=${n}
        profile=${r}
        onSignOut=${o}
      />
    </aside>
  `}var yA="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",bA="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",Bw="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",zw={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},qw={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function A({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=zw[n]??zw.md,u=r?"w-full":"";if(a==="primary")return l`
      <${s}
        style=${{background:yA,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${Y(Bw,o,u,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:bA}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=qw[a]??qw.outline;return l`
    <${s}
      className=${Y(Bw,o,u,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function Iw(){let e=p.default.useMemo(()=>xA(window.location),[]),[t,a]=p.default.useState(null),[n,r]=p.default.useState(null),[s,i]=p.default.useState(!1),[o,u]=p.default.useState(""),[c,d]=p.default.useState(!1);p.default.useEffect(()=>{if(!e)return;let h=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:h.signal}).then(x=>{if(!x.ok)throw new Error(String(x.status));return x.json()}).then(a).catch(()=>{h.signal.aborted||a(null)}),()=>h.abort()},[e]);let m=p.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),u("");try{let h=await fetch(`${e.base}/attestation/report`);if(!h.ok)throw new Error(String(h.status));let x=await h.json();return r(x),x}catch(h){return u(h.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),f=p.default.useCallback(async()=>{let h=n||await m();return!h||!navigator.clipboard?!1:(await navigator.clipboard.writeText(JSON.stringify({...h,instance_attestation:t},null,2)),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[m,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:m,copyReport:f}}function xA(e){let t=e.hostname;if(!t||t==="localhost"||$A(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function $A(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}var wA=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function Kw(){let e=R(),t=Iw(),[a,n]=p.default.useState(!1),r=p.default.useCallback(()=>{n(o=>{let u=!o;return u&&t.loadReport(),u})},[t]),s=p.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=SA({teeInfo:t.teeInfo,report:t.report,t:e});return l`
    <div className="relative">
      <button
        type="button"
        onClick=${r}
        aria-expanded=${a}
        title=${e("tee.title")}
        className=${Y("grid h-8 w-8 place-items-center rounded-[8px]","border border-[color-mix(in_srgb,var(--v2-positive-text)_28%,transparent)]","bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]","hover:border-[color-mix(in_srgb,var(--v2-positive-text)_52%,transparent)]")}
      >
        <${M} name="shield" className="h-4 w-4" />
      </button>

      ${a&&l`
        <div
          className=${Y("absolute right-0 top-full z-40 mt-2 w-[min(22rem,calc(100vw-2rem))]","rounded-[14px] border border-[var(--v2-panel-border)]","bg-[var(--v2-surface)] p-3 shadow-[0_18px_48px_rgba(0,0,0,0.35)]")}
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
            ${i.map(o=>l`
                <div className="rounded-[10px] bg-[var(--v2-surface-soft)] px-3 py-2">
                  <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-[var(--v2-text-faint)]">
                    ${o.label}
                  </div>
                  <div className="mt-1 break-all font-mono text-[11px] text-[var(--v2-text)]">
                    ${o.value}
                  </div>
                </div>
              `)}
            ${t.reportLoading&&l`<div className="text-xs text-[var(--v2-text-muted)]">${e("tee.loading")}</div>`}
            ${t.reportError&&l`<div className="text-xs text-[var(--v2-danger-text)]">${e("tee.loadFailed")}</div>`}
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
  `}function SA({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return wA.map(([r,s])=>({label:a(s),value:NA(n[r])||a("common.unknown")}))}function NA(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var _A="https://docs.ironclaw.com";function Hw({threadsState:e,onToggleSidebar:t,sidebarOpen:a=!0}){let n=R(),r=Pe(),s=p.default.useMemo(()=>{for(let o of el){let u=bc[o.id];if(!u)continue;let c=o.path+"/";if(r.pathname.startsWith(c)){let d=r.pathname.slice(c.length).split("/")[0],m=u.find(f=>f.id===d);if(m)return{parent:n(o.labelKey),current:n(m.labelKey)}}}return null},[r.pathname,n]),i=p.default.useMemo(()=>{if(s)return null;if(r.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(c=>c.id===e.activeThreadId)?.title||n("nav.chat");let o=el.find(u=>r.pathname.startsWith(u.path));return o?n(o.labelKey):""},[r.pathname,e.activeThreadId,e.threads,n,s]);return l`
    <header
      className=${Y("flex h-14 shrink-0 items-center gap-3 px-4","border-b border-[var(--v2-panel-border)]","bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] backdrop-blur-xl")}
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

      ${s?l`
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
          `:l`
            <span
              className="truncate text-[14px] font-semibold text-[var(--v2-text-strong)]"
            >
              ${i}
            </span>
          `}

      <div className="ml-auto flex shrink-0 items-center gap-1">
        <${Kw} />
        <${Ja}
          to="/logs"
          className=${({isActive:o})=>Y("inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",o&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
          title=${n("nav.logs")}
        >
          ${n("nav.logs")}
        <//>
        <a
          href=${_A}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${n("nav.docs")}
        >
          ${n("nav.docs")}
        </a>
      </div>
    </header>
  `}function Qw({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=pe(),i=R(),[o,u]=p.default.useState(""),[c,d]=p.default.useState(0),m=p.default.useRef(null),f=p.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(b=>({id:`thread-${b.id}`,label:b.title||`Thread ${b.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${b.id}`)}));return[...g,...v]},[a,s,n,r]),h=p.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?f.filter(v=>v.label.toLowerCase().includes(g)):f},[f,o]);p.default.useEffect(()=>{if(!e)return;u(""),d(0);let g=window.requestAnimationFrame(()=>m.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),p.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,h.length-1)))},[h.length]);let x=p.default.useCallback(g=>{g&&(t(),g.run())},[t]),y=p.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,h.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),x(h[c])):g.key==="Escape"&&(g.preventDefault(),t())},[h,c,x,t]);if(!e)return null;let $=null;return l`
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh]" role="dialog" aria-modal="true" aria-label="Command palette">
      <button type="button" aria-label="Close" onClick=${t} className="absolute inset-0 bg-black/50"></button>
      <div className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_30px_60px_-20px_rgba(0,0,0,0.8)]">
        <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-3">
          <${M} name="search" className="h-4 w-4 text-[var(--v2-text-faint)]" />
          <input
            ref=${m}
            value=${o}
            onInput=${g=>u(g.currentTarget.value)}
            onKeyDown=${y}
            placeholder=${i("command.placeholder")}
            className="h-12 w-full border-0 bg-transparent text-sm text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)]"
          />
          <kbd className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]">esc</kbd>
        </div>
        <ul className="max-h-[50vh] overflow-y-auto p-1.5">
          ${h.length===0&&l`<li className="px-3 py-6 text-center text-sm text-[var(--v2-text-faint)]">No matches</li>`}
          ${h.map((g,v)=>{let b=g.group!==$;return $=g.group,l`
              ${b&&l`<li key=${`g-${g.group}`} className="px-2 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">${g.group}</li>`}
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
  `}var Vw={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},kA={info:"bolt",success:"check",error:"close"};function Gw(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>$w(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:l`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${e.map(a=>l`
          <div
            key=${a.id}
            role="status"
            className=${["pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",Vw[a.tone]||Vw.info].join(" ")}
          >
            <${M} name=${kA[a.tone]||"bolt"} className="h-4 w-4 shrink-0" />
            <span>${a.message}</span>
          </div>
        `)}
    </div>
  `}function Yw({token:e,profile:t,isChecking:a=!1,isAdmin:n,rebornProjectsEnabled:r=!1,onSignOut:s}){let i=R(),{theme:o,toggleTheme:u}=xc(),c=T$(e),d=Sw(),m=xw({onNewChat:()=>d.setActiveThreadId(null)}),f=c.data,h=Pe(),x=pe(),y=ri({settings:{},gatewayStatus:f,enabled:n}),$=n&&dw({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),g=h.pathname==="/welcome"||h.pathname.startsWith("/settings"),[v,b]=p.default.useState(!1);p.default.useEffect(()=>{let S=E=>{(E.metaKey||E.ctrlKey)&&E.key.toLowerCase()==="k"&&(E.preventDefault(),b(k=>!k))};return window.addEventListener("keydown",S),()=>window.removeEventListener("keydown",S)},[]);let w=p.default.useCallback(async S=>{let E=d.activeThreadId===S;try{await d.deleteThread(S),E&&x("/chat",{replace:!0})}catch(k){console.error("Failed to delete thread:",k),si(ww(k,i),{tone:"error"})}},[x,d,i]);return $&&!g?l`<${it} to="/welcome" replace />`:l`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${m.mobileOpen&&l`<button
        type="button"
        aria-label=${i("nav.close")}
        onClick=${m.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${Y("fixed inset-y-0 left-0 z-50 md:relative md:z-auto",m.mobileOpen?"flex":"hidden",m.desktopOpen?"md:flex":"md:hidden")}
      >
        <${Fw}
          id="gateway-sidebar"
          threadsState=${d}
          theme=${o}
          toggleTheme=${u}
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
        <${Hw}
          threadsState=${d}
          onToggleSidebar=${m.toggle}
          sidebarOpen=${m.currentOpen}
        />
        <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
          ${c.error&&l`
            <div
              className=${Y("m-4 rounded-[14px] border px-4 py-3 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >
              ${c.error.message||i("error.gatewayConnection")}
            </div>
          `}
          <${Np}
            context=${{gatewayStatus:f,gatewayStatusQuery:c,currentUser:t,isChecking:a,isAdmin:n,threadsState:d}}
          />
        </main>
      </div>
      <${Qw}
        open=${v}
        onClose=${()=>b(!1)}
        threadsState=${d}
        onNewChat=${m.newChat}
        onToggleTheme=${u}
      />
      <${Gw} />
    </div>
  `}var Bt=Be(He(),1),ll=e=>e.type==="checkbox",Qr=e=>e instanceof Date,Et=e=>e==null,l1=e=>typeof e=="object",Ge=e=>!Et(e)&&!Array.isArray(e)&&l1(e)&&!Qr(e),RA=e=>Ge(e)&&e.target?ll(e.target)?e.target.checked:e.target.value:e,CA=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,EA=(e,t)=>e.has(CA(t)),TA=e=>{let t=e.constructor&&e.constructor.prototype;return Ge(t)&&t.hasOwnProperty("isPrototypeOf")},ah=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function mt(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(ah&&(e instanceof Blob||n))&&(a||Ge(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!TA(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=mt(e[r]));else return e;return t}var Dc=e=>/^\w*$/.test(e),We=e=>e===void 0,nh=e=>Array.isArray(e)?e.filter(Boolean):[],rh=e=>nh(e.replace(/["|']|\]/g,"").split(/\.|\[/)),J=(e,t,a)=>{if(!t||!Ge(e))return a;let n=(Dc(t)?[t]:rh(t)).reduce((r,s)=>Et(r)?r:r[s],e);return We(n)||n===e?We(e[t])?a:e[t]:n},Za=e=>typeof e=="boolean",Ue=(e,t,a)=>{let n=-1,r=Dc(t)?[t]:rh(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],u=a;if(n!==i){let c=e[o];u=Ge(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=u,e=e[o]}},Jw={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},Ea={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},_n={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},AA=Bt.default.createContext(null);AA.displayName="HookFormContext";var DA=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==Ea.all&&(t._proxyFormState[i]=!n||Ea.all),a&&(a[i]=!0),e[i]}});return r},MA=typeof window<"u"?Bt.default.useLayoutEffect:Bt.default.useEffect;var Wa=e=>typeof e=="string",OA=(e,t,a,n,r)=>Wa(e)?(n&&t.watch.add(e),J(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),J(a,s))):(n&&(t.watchAll=!0),a),th=e=>Et(e)||!l1(e);function lr(e,t,a=new WeakSet){if(th(e)||th(t))return e===t;if(Qr(e)&&Qr(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(Qr(i)&&Qr(o)||Ge(i)&&Ge(o)||Array.isArray(i)&&Array.isArray(o)?!lr(i,o,a):i!==o)return!1}}return!0}var LA=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},il=e=>Array.isArray(e)?e:[e],Xw=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},zt=e=>Ge(e)&&!Object.keys(e).length,sh=e=>e.type==="file",Ta=e=>typeof e=="function",Ec=e=>{if(!ah)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},u1=e=>e.type==="select-multiple",ih=e=>e.type==="radio",PA=e=>ih(e)||ll(e),eh=e=>Ec(e)&&e.isConnected;function UA(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=We(e)?n++:e[t[n++]];return e}function jA(e){for(let t in e)if(e.hasOwnProperty(t)&&!We(e[t]))return!1;return!0}function Ze(e,t){let a=Array.isArray(t)?t:Dc(t)?[t]:rh(t),n=a.length===1?e:UA(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Ge(n)&&zt(n)||Array.isArray(n)&&jA(n))&&Ze(e,a.slice(0,-1)),e}var c1=e=>{for(let t in e)if(Ta(e[t]))return!0;return!1};function Tc(e,t={}){let a=Array.isArray(e);if(Ge(e)||a)for(let n in e)Array.isArray(e[n])||Ge(e[n])&&!c1(e[n])?(t[n]=Array.isArray(e[n])?[]:{},Tc(e[n],t[n])):Et(e[n])||(t[n]=!0);return t}function d1(e,t,a){let n=Array.isArray(e);if(Ge(e)||n)for(let r in e)Array.isArray(e[r])||Ge(e[r])&&!c1(e[r])?We(t)||th(a[r])?a[r]=Array.isArray(e[r])?Tc(e[r],[]):{...Tc(e[r])}:d1(e[r],Et(t)?{}:t[r],a[r]):a[r]=!lr(e[r],t[r]);return a}var rl=(e,t)=>d1(e,t,Tc(t)),Zw={value:!1,isValid:!1},Ww={value:!0,isValid:!0},m1=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!We(e[0].attributes.value)?We(e[0].value)||e[0].value===""?Ww:{value:e[0].value,isValid:!0}:Ww:Zw}return Zw},f1=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>We(e)?e:t?e===""?NaN:e&&+e:a&&Wa(e)?new Date(e):n?n(e):e,e1={isValid:!1,value:null},p1=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,e1):e1;function t1(e){let t=e.ref;return sh(t)?t.files:ih(t)?p1(e.refs).value:u1(t)?[...t.selectedOptions].map(({value:a})=>a):ll(t)?m1(e.refs).value:f1(We(t.value)?e.ref.value:t.value,e)}var FA=(e,t,a,n)=>{let r={};for(let s of e){let i=J(t,s);i&&Ue(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},Ac=e=>e instanceof RegExp,sl=e=>We(e)?e:Ac(e)?e.source:Ge(e)?Ac(e.value)?e.value.source:e.value:e,a1=e=>({isOnSubmit:!e||e===Ea.onSubmit,isOnBlur:e===Ea.onBlur,isOnChange:e===Ea.onChange,isOnAll:e===Ea.all,isOnTouch:e===Ea.onTouched}),n1="AsyncFunction",BA=e=>!!e&&!!e.validate&&!!(Ta(e.validate)&&e.validate.constructor.name===n1||Ge(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===n1)),zA=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),r1=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),ol=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=J(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(ol(o,t))break}else if(Ge(o)&&ol(o,t))break}}};function s1(e,t,a){let n=J(e,a);if(n||Dc(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=J(t,s),o=J(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var qA=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return zt(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||Ea.all))},IA=(e,t,a)=>!e||!t||e===t||il(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),KA=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,HA=(e,t)=>!nh(J(e,t)).length&&Ze(e,t),QA=(e,t,a)=>{let n=il(J(e,a));return Ue(n,"root",t[a]),Ue(e,a,n),e},Cc=e=>Wa(e);function i1(e,t,a="validate"){if(Cc(e)||Array.isArray(e)&&e.every(Cc)||Za(e)&&!e)return{type:a,message:Cc(e)?e:"",ref:t}}var oi=e=>Ge(e)&&!Ac(e)?e:{value:e,message:""},o1=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:u,maxLength:c,minLength:d,min:m,max:f,pattern:h,validate:x,name:y,valueAsNumber:$,mount:g}=e._f,v=J(a,y);if(!g||t.has(y))return{};let b=o?o[0]:i,w=U=>{r&&b.reportValidity&&(b.setCustomValidity(Za(U)?"":U||""),b.reportValidity())},S={},E=ih(i),k=ll(i),C=E||k,O=($||sh(i))&&We(i.value)&&We(v)||Ec(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,L=LA.bind(null,y,n,S),P=(U,T,K,te=_n.maxLength,se=_n.minLength)=>{let ie=U?T:K;S[y]={type:U?te:se,message:ie,ref:i,...L(U?te:se,ie)}};if(s?!Array.isArray(v)||!v.length:u&&(!C&&(O||Et(v))||Za(v)&&!v||k&&!m1(o).isValid||E&&!p1(o).isValid)){let{value:U,message:T}=Cc(u)?{value:!!u,message:u}:oi(u);if(U&&(S[y]={type:_n.required,message:T,ref:b,...L(_n.required,T)},!n))return w(T),S}if(!O&&(!Et(m)||!Et(f))){let U,T,K=oi(f),te=oi(m);if(!Et(v)&&!isNaN(v)){let se=i.valueAsNumber||v&&+v;Et(K.value)||(U=se>K.value),Et(te.value)||(T=se<te.value)}else{let se=i.valueAsDate||new Date(v),ie=gt=>new Date(new Date().toDateString()+" "+gt),Ye=i.type=="time",Ke=i.type=="week";Wa(K.value)&&v&&(U=Ye?ie(v)>ie(K.value):Ke?v>K.value:se>new Date(K.value)),Wa(te.value)&&v&&(T=Ye?ie(v)<ie(te.value):Ke?v<te.value:se<new Date(te.value))}if((U||T)&&(P(!!U,K.message,te.message,_n.max,_n.min),!n))return w(S[y].message),S}if((c||d)&&!O&&(Wa(v)||s&&Array.isArray(v))){let U=oi(c),T=oi(d),K=!Et(U.value)&&v.length>+U.value,te=!Et(T.value)&&v.length<+T.value;if((K||te)&&(P(K,U.message,T.message),!n))return w(S[y].message),S}if(h&&!O&&Wa(v)){let{value:U,message:T}=oi(h);if(Ac(U)&&!v.match(U)&&(S[y]={type:_n.pattern,message:T,ref:i,...L(_n.pattern,T)},!n))return w(T),S}if(x){if(Ta(x)){let U=await x(v,a),T=i1(U,b);if(T&&(S[y]={...T,...L(_n.validate,T.message)},!n))return w(T.message),S}else if(Ge(x)){let U={};for(let T in x){if(!zt(U)&&!n)break;let K=i1(await x[T](v,a),b,T);K&&(U={...K,...L(T,K.message)},w(K.message),n&&(S[y]=U))}if(!zt(U)&&(S[y]={ref:b,...U},!n))return S}}return w(!0),S},VA={mode:Ea.onSubmit,reValidateMode:Ea.onChange,shouldFocusError:!0};function GA(e={}){let t={...VA,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:Ta(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Ge(t.defaultValues)||Ge(t.values)?mt(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:mt(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},u,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},m={...d},f={array:Xw(),state:Xw()},h=t.criteriaMode===Ea.all,x=N=>_=>{clearTimeout(c),c=setTimeout(N,_)},y=async N=>{if(!t.disabled&&(d.isValid||m.isValid||N)){let _=t.resolver?zt((await k()).errors):await O(n,!0);_!==a.isValid&&f.state.next({isValid:_})}},$=(N,_)=>{!t.disabled&&(d.isValidating||d.validatingFields||m.isValidating||m.validatingFields)&&((N||Array.from(o.mount)).forEach(D=>{D&&(_?Ue(a.validatingFields,D,_):Ze(a.validatingFields,D))}),f.state.next({validatingFields:a.validatingFields,isValidating:!zt(a.validatingFields)}))},g=(N,_=[],D,z,F=!0,B=!0)=>{if(z&&D&&!t.disabled){if(i.action=!0,B&&Array.isArray(J(n,N))){let V=D(J(n,N),z.argA,z.argB);F&&Ue(n,N,V)}if(B&&Array.isArray(J(a.errors,N))){let V=D(J(a.errors,N),z.argA,z.argB);F&&Ue(a.errors,N,V),HA(a.errors,N)}if((d.touchedFields||m.touchedFields)&&B&&Array.isArray(J(a.touchedFields,N))){let V=D(J(a.touchedFields,N),z.argA,z.argB);F&&Ue(a.touchedFields,N,V)}(d.dirtyFields||m.dirtyFields)&&(a.dirtyFields=rl(r,s)),f.state.next({name:N,isDirty:P(N,_),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else Ue(s,N,_)},v=(N,_)=>{Ue(a.errors,N,_),f.state.next({errors:a.errors})},b=N=>{a.errors=N,f.state.next({errors:a.errors,isValid:!1})},w=(N,_,D,z)=>{let F=J(n,N);if(F){let B=J(s,N,We(D)?J(r,N):D);We(B)||z&&z.defaultChecked||_?Ue(s,N,_?B:t1(F._f)):K(N,B),i.mount&&y()}},S=(N,_,D,z,F)=>{let B=!1,V=!1,fe={name:N};if(!t.disabled){if(!D||z){(d.isDirty||m.isDirty)&&(V=a.isDirty,a.isDirty=fe.isDirty=P(),B=V!==fe.isDirty);let he=lr(J(r,N),_);V=!!J(a.dirtyFields,N),he?Ze(a.dirtyFields,N):Ue(a.dirtyFields,N,!0),fe.dirtyFields=a.dirtyFields,B=B||(d.dirtyFields||m.dirtyFields)&&V!==!he}if(D){let he=J(a.touchedFields,N);he||(Ue(a.touchedFields,N,D),fe.touchedFields=a.touchedFields,B=B||(d.touchedFields||m.touchedFields)&&he!==D)}B&&F&&f.state.next(fe)}return B?fe:{}},E=(N,_,D,z)=>{let F=J(a.errors,N),B=(d.isValid||m.isValid)&&Za(_)&&a.isValid!==_;if(t.delayError&&D?(u=x(()=>v(N,D)),u(t.delayError)):(clearTimeout(c),u=null,D?Ue(a.errors,N,D):Ze(a.errors,N)),(D?!lr(F,D):F)||!zt(z)||B){let V={...z,...B&&Za(_)?{isValid:_}:{},errors:a.errors,name:N};a={...a,...V},f.state.next(V)}},k=async N=>{$(N,!0);let _=await t.resolver(s,t.context,FA(N||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return $(N),_},C=async N=>{let{errors:_}=await k(N);if(N)for(let D of N){let z=J(_,D);z?Ue(a.errors,D,z):Ze(a.errors,D)}else a.errors=_;return _},O=async(N,_,D={valid:!0})=>{for(let z in N){let F=N[z];if(F){let{_f:B,...V}=F;if(B){let fe=o.array.has(B.name),he=F._f&&BA(F._f);he&&d.validatingFields&&$([z],!0);let Nt=await o1(F,o.disabled,s,h,t.shouldUseNativeValidation&&!_,fe);if(he&&d.validatingFields&&$([z]),Nt[B.name]&&(D.valid=!1,_))break;!_&&(J(Nt,B.name)?fe?QA(a.errors,Nt,B.name):Ue(a.errors,B.name,Nt[B.name]):Ze(a.errors,B.name))}!zt(V)&&await O(V,_,D)}}return D.valid},L=()=>{for(let N of o.unMount){let _=J(n,N);_&&(_._f.refs?_._f.refs.every(D=>!eh(D)):!eh(_._f.ref))&&Ma(N)}o.unMount=new Set},P=(N,_)=>!t.disabled&&(N&&_&&Ue(s,N,_),!lr(gt(),r)),U=(N,_,D)=>OA(N,o,{...i.mount?s:We(_)?r:Wa(N)?{[N]:_}:_},D,_),T=N=>nh(J(i.mount?s:r,N,t.shouldUnregister?J(r,N,[]):[])),K=(N,_,D={})=>{let z=J(n,N),F=_;if(z){let B=z._f;B&&(!B.disabled&&Ue(s,N,f1(_,B)),F=Ec(B.ref)&&Et(_)?"":_,u1(B.ref)?[...B.ref.options].forEach(V=>V.selected=F.includes(V.value)):B.refs?ll(B.ref)?B.refs.forEach(V=>{(!V.defaultChecked||!V.disabled)&&(Array.isArray(F)?V.checked=!!F.find(fe=>fe===V.value):V.checked=F===V.value||!!F)}):B.refs.forEach(V=>V.checked=V.value===F):sh(B.ref)?B.ref.value="":(B.ref.value=F,B.ref.type||f.state.next({name:N,values:mt(s)})))}(D.shouldDirty||D.shouldTouch)&&S(N,F,D.shouldTouch,D.shouldDirty,!0),D.shouldValidate&&Ke(N)},te=(N,_,D)=>{for(let z in _){if(!_.hasOwnProperty(z))return;let F=_[z],B=N+"."+z,V=J(n,B);(o.array.has(N)||Ge(F)||V&&!V._f)&&!Qr(F)?te(B,F,D):K(B,F,D)}},se=(N,_,D={})=>{let z=J(n,N),F=o.array.has(N),B=mt(_);Ue(s,N,B),F?(f.array.next({name:N,values:mt(s)}),(d.isDirty||d.dirtyFields||m.isDirty||m.dirtyFields)&&D.shouldDirty&&f.state.next({name:N,dirtyFields:rl(r,s),isDirty:P(N,B)})):z&&!z._f&&!Et(B)?te(N,B,D):K(N,B,D),r1(N,o)&&f.state.next({...a,name:N}),f.state.next({name:i.mount?N:void 0,values:mt(s)})},ie=async N=>{i.mount=!0;let _=N.target,D=_.name,z=!0,F=J(n,D),B=he=>{z=Number.isNaN(he)||Qr(he)&&isNaN(he.getTime())||lr(he,J(s,D,he))},V=a1(t.mode),fe=a1(t.reValidateMode);if(F){let he,Nt,Pa=_.type?t1(F._f):RA(N),Kt=N.type===Jw.BLUR||N.type===Jw.FOCUS_OUT,Zr=!zA(F._f)&&!t.resolver&&!J(a.errors,D)&&!F._f.deps||KA(Kt,J(a.touchedFields,D),a.isSubmitted,fe,V),En=r1(D,o,Kt);Ue(s,D,Pa),Kt?(!_||!_.readOnly)&&(F._f.onBlur&&F._f.onBlur(N),u&&u(0)):F._f.onChange&&F._f.onChange(N);let gr=S(D,Pa,Kt),ce=!zt(gr)||En;if(!Kt&&f.state.next({name:D,type:N.type,values:mt(s)}),Zr)return(d.isValid||m.isValid)&&(t.mode==="onBlur"?Kt&&y():Kt||y()),ce&&f.state.next({name:D,...En?{}:gr});if(!Kt&&En&&f.state.next({...a}),t.resolver){let{errors:Tn}=await k([D]);if(B(Pa),z){let an=s1(a.errors,n,D),Ht=s1(Tn,n,an.name||D);he=Ht.error,D=Ht.name,Nt=zt(Tn)}}else $([D],!0),he=(await o1(F,o.disabled,s,h,t.shouldUseNativeValidation))[D],$([D]),B(Pa),z&&(he?Nt=!1:(d.isValid||m.isValid)&&(Nt=await O(n,!0)));z&&(F._f.deps&&Ke(F._f.deps),E(D,Nt,he,gr))}},Ye=(N,_)=>{if(J(a.errors,_)&&N.focus)return N.focus(),1},Ke=async(N,_={})=>{let D,z,F=il(N);if(t.resolver){let B=await C(We(N)?N:F);D=zt(B),z=N?!F.some(V=>J(B,V)):D}else N?(z=(await Promise.all(F.map(async B=>{let V=J(n,B);return await O(V&&V._f?{[B]:V}:V)}))).every(Boolean),!(!z&&!a.isValid)&&y()):z=D=await O(n);return f.state.next({...!Wa(N)||(d.isValid||m.isValid)&&D!==a.isValid?{}:{name:N},...t.resolver||!N?{isValid:D}:{},errors:a.errors}),_.shouldFocus&&!z&&ol(n,Ye,N?F:o.mount),z},gt=N=>{let _={...i.mount?s:r};return We(N)?_:Wa(N)?J(_,N):N.map(D=>J(_,D))},ft=(N,_)=>({invalid:!!J((_||a).errors,N),isDirty:!!J((_||a).dirtyFields,N),error:J((_||a).errors,N),isValidating:!!J(a.validatingFields,N),isTouched:!!J((_||a).touchedFields,N)}),je=N=>{N&&il(N).forEach(_=>Ze(a.errors,_)),f.state.next({errors:N?a.errors:{}})},St=(N,_,D)=>{let z=(J(n,N,{_f:{}})._f||{}).ref,F=J(a.errors,N)||{},{ref:B,message:V,type:fe,...he}=F;Ue(a.errors,N,{...he,..._,ref:z}),f.state.next({name:N,errors:a.errors,isValid:!1}),D&&D.shouldFocus&&z&&z.focus&&z.focus()},Cn=(N,_)=>Ta(N)?f.state.subscribe({next:D=>"values"in D&&N(U(void 0,_),D)}):U(N,_,!0),qt=N=>f.state.subscribe({next:_=>{IA(N.name,_.name,N.exact)&&qA(_,N.formState||d,re,N.reRenderRoot)&&N.callback({values:{...s},...a,..._,defaultValues:r})}}).unsubscribe,Da=N=>(i.mount=!0,m={...m,...N.formState},qt({...N,formState:m})),Ma=(N,_={})=>{for(let D of N?il(N):o.mount)o.mount.delete(D),o.array.delete(D),_.keepValue||(Ze(n,D),Ze(s,D)),!_.keepError&&Ze(a.errors,D),!_.keepDirty&&Ze(a.dirtyFields,D),!_.keepTouched&&Ze(a.touchedFields,D),!_.keepIsValidating&&Ze(a.validatingFields,D),!t.shouldUnregister&&!_.keepDefaultValue&&Ze(r,D);f.state.next({values:mt(s)}),f.state.next({...a,..._.keepDirty?{isDirty:P()}:{}}),!_.keepIsValid&&y()},Oa=({disabled:N,name:_})=>{(Za(N)&&i.mount||N||o.disabled.has(_))&&(N?o.disabled.add(_):o.disabled.delete(_))},It=(N,_={})=>{let D=J(n,N),z=Za(_.disabled)||Za(t.disabled);return Ue(n,N,{...D||{},_f:{...D&&D._f?D._f:{ref:{name:N}},name:N,mount:!0,..._}}),o.mount.add(N),D?Oa({disabled:Za(_.disabled)?_.disabled:t.disabled,name:N}):w(N,!0,_.value),{...z?{disabled:_.disabled||t.disabled}:{},...t.progressive?{required:!!_.required,min:sl(_.min),max:sl(_.max),minLength:sl(_.minLength),maxLength:sl(_.maxLength),pattern:sl(_.pattern)}:{},name:N,onChange:ie,onBlur:ie,ref:F=>{if(F){It(N,_),D=J(n,N);let B=We(F.value)&&F.querySelectorAll&&F.querySelectorAll("input,select,textarea")[0]||F,V=PA(B),fe=D._f.refs||[];if(V?fe.find(he=>he===B):B===D._f.ref)return;Ue(n,N,{_f:{...D._f,...V?{refs:[...fe.filter(eh),B,...Array.isArray(J(r,N))?[{}]:[]],ref:{type:B.type,name:N}}:{ref:B}}}),w(N,!1,void 0,B)}else D=J(n,N,{}),D._f&&(D._f.mount=!1),(t.shouldUnregister||_.shouldUnregister)&&!(EA(o.array,N)&&i.action)&&o.unMount.add(N)}}},ot=()=>t.shouldFocusError&&ol(n,Ye,o.mount),oa=N=>{Za(N)&&(f.state.next({disabled:N}),ol(n,(_,D)=>{let z=J(n,D);z&&(_.disabled=z._f.disabled||N,Array.isArray(z._f.refs)&&z._f.refs.forEach(F=>{F.disabled=z._f.disabled||N}))},0,!1))},wa=(N,_)=>async D=>{let z;D&&(D.preventDefault&&D.preventDefault(),D.persist&&D.persist());let F=mt(s);if(f.state.next({isSubmitting:!0}),t.resolver){let{errors:B,values:V}=await k();a.errors=B,F=mt(V)}else await O(n);if(o.disabled.size)for(let B of o.disabled)Ze(F,B);if(Ze(a.errors,"root"),zt(a.errors)){f.state.next({errors:{}});try{await N(F,D)}catch(B){z=B}}else _&&await _({...a.errors},D),ot(),setTimeout(ot);if(f.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:zt(a.errors)&&!z,submitCount:a.submitCount+1,errors:a.errors}),z)throw z},La=(N,_={})=>{J(n,N)&&(We(_.defaultValue)?se(N,mt(J(r,N))):(se(N,_.defaultValue),Ue(r,N,mt(_.defaultValue))),_.keepTouched||Ze(a.touchedFields,N),_.keepDirty||(Ze(a.dirtyFields,N),a.isDirty=_.defaultValue?P(N,mt(J(r,N))):P()),_.keepError||(Ze(a.errors,N),d.isValid&&y()),f.state.next({...a}))},la=(N,_={})=>{let D=N?mt(N):r,z=mt(D),F=zt(N),B=F?r:z;if(_.keepDefaultValues||(r=D),!_.keepValues){if(_.keepDirtyValues){let V=new Set([...o.mount,...Object.keys(rl(r,s))]);for(let fe of Array.from(V))J(a.dirtyFields,fe)?Ue(B,fe,J(s,fe)):se(fe,J(B,fe))}else{if(ah&&We(N))for(let V of o.mount){let fe=J(n,V);if(fe&&fe._f){let he=Array.isArray(fe._f.refs)?fe._f.refs[0]:fe._f.ref;if(Ec(he)){let Nt=he.closest("form");if(Nt){Nt.reset();break}}}}if(_.keepFieldsRef)for(let V of o.mount)se(V,J(B,V));else n={}}s=t.shouldUnregister?_.keepDefaultValues?mt(r):{}:mt(B),f.array.next({values:{...B}}),f.state.next({values:{...B}})}o={mount:_.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!_.keepIsValid||!!_.keepDirtyValues,i.watch=!!t.shouldUnregister,f.state.next({submitCount:_.keepSubmitCount?a.submitCount:0,isDirty:F?!1:_.keepDirty?a.isDirty:!!(_.keepDefaultValues&&!lr(N,r)),isSubmitted:_.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:F?{}:_.keepDirtyValues?_.keepDefaultValues&&s?rl(r,s):a.dirtyFields:_.keepDefaultValues&&N?rl(r,N):_.keepDirty?a.dirtyFields:{},touchedFields:_.keepTouched?a.touchedFields:{},errors:_.keepErrors?a.errors:{},isSubmitSuccessful:_.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},Sa=(N,_)=>la(Ta(N)?N(s):N,_),W=(N,_={})=>{let D=J(n,N),z=D&&D._f;if(z){let F=z.refs?z.refs[0]:z.ref;F.focus&&(F.focus(),_.shouldSelect&&Ta(F.select)&&F.select())}},re=N=>{a={...a,...N}},we={control:{register:It,unregister:Ma,getFieldState:ft,handleSubmit:wa,setError:St,_subscribe:qt,_runSchema:k,_focusError:ot,_getWatch:U,_getDirty:P,_setValid:y,_setFieldArray:g,_setDisabledField:Oa,_setErrors:b,_getFieldArray:T,_reset:la,_resetDefaultValues:()=>Ta(t.defaultValues)&&t.defaultValues().then(N=>{Sa(N,t.resetOptions),f.state.next({isLoading:!1})}),_removeUnmounted:L,_disableForm:oa,_subjects:f,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(N){i=N},get _defaultValues(){return r},get _names(){return o},set _names(N){o=N},get _formState(){return a},get _options(){return t},set _options(N){t={...t,...N}}},subscribe:Da,trigger:Ke,register:It,handleSubmit:wa,watch:Cn,setValue:se,getValues:gt,reset:Sa,resetField:La,clearErrors:je,unregister:Ma,setError:St,setFocus:W,getFieldState:ft};return{...we,formControl:we}}function h1(e={}){let t=Bt.default.useRef(void 0),a=Bt.default.useRef(void 0),[n,r]=Bt.default.useState({isDirty:!1,isValidating:!1,isLoading:Ta(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:Ta(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!Ta(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=GA(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,MA(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),Bt.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),Bt.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),Bt.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),Bt.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),Bt.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),Bt.default.useEffect(()=>{e.values&&!lr(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),Bt.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=DA(n,s),t.current}var v1={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},g1={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},YA={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function ae({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return l`
    <${s}
      className=${Y(v1[a]??v1.default,g1[n]??g1.md,YA[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var oh="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",Mc={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function Tt({className:e="",size:t="md",error:a=!1,...n}){return l`
    <input
      className=${Y(oh,Mc[t]??Mc.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Oc({className:e="",error:t=!1,rows:a=4,...n}){return l`
    <textarea
      rows=${a}
      className=${Y(oh,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function lh({children:e,className:t="",size:a="md",error:n=!1,...r}){return l`
    <div className="relative w-full">
      <select
        className=${Y(oh,Mc[a]??Mc.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
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
  `}function JA({children:e,className:t="",required:a=!1,...n}){return l`
    <label
      className=${Y("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&l`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function kn({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return l`
    <div className=${Y("flex flex-col gap-2",s)}>
      ${e&&l`<${JA} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&l`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&l`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var XA={google:"Google",github:"GitHub",apple:"Apple"};function ZA(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function y1({providers:e,redirectAfter:t}){let a=R();return e.length?l`
    <div className="mt-6 space-y-3">
      <div className="flex items-center gap-3 text-[11px] uppercase text-[var(--v2-text-faint)]">
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
        <span>${a("login.oauthDivider")}</span>
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
      </div>
      <div className="grid gap-2">
        ${e.map(n=>l`
            <${A}
              key=${n}
              as="a"
              href=${ZA(n,t)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <${M} name="shield" className="h-4 w-4" />
              ${a("login.oauthProvider",{provider:XA[n]||n})}
            <//>
          `)}
      </div>
    </div>
  `:null}var WA=["google","github","apple"];function b1(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>{let a=!1;return Zx().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(WA.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function x1({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=R(),{theme:s,toggleTheme:i}=xc(),o=b1(),{formState:{errors:u,isSubmitting:c},handleSubmit:d,register:m}=h1({defaultValues:{token:e||""}});return l`
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
      <${ae}
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
          <${kn}
            label=${r("login.tokenLabel")}
            htmlFor="v2-token"
            error=${u.token?.message??""}
            hint=${r("login.tokenHint")}
          >
            <${Tt}
              id="v2-token"
              type="password"
              error=${!!u.token}
              ...${m("token",{required:r("login.tokenRequired"),setValueAs:f=>f.trim()})}
              placeholder=${r("login.tokenPlaceholder")}
              autocomplete="current-password"
            />
          <//>

          ${t&&l`<p
              className=${Y("rounded-[10px] border px-3 py-2 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
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

        <${y1}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var $1={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},w1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function q({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return l`
    <span
      className=${Y("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",w1[n]??w1.md,$1[e]??$1.muted,r)}
    >
      ${a&&l`<span
          className=${Y("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var e4=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,S1=/(bash|shell|exec|run|command|terminal|spawn|process)/,N1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function _1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return e4.test(n)?{tone:"danger",key:"tool.riskWrite"}:S1.test(n)?{tone:"warning",key:"tool.riskExec"}:N1.test(n)?{tone:"info",key:"tool.riskNetwork"}:S1.test(r)?{tone:"warning",key:"tool.riskExec"}:N1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}var Lc=480;function t4(e,t){return t&&t.length>0?t.some(a=>typeof a?.value=="string"&&a.value.length>Lc):typeof e=="string"&&e.length>Lc}function k1(e,t){return typeof e!="string"||t||e.length<=Lc?e:`${e.slice(0,Lc).trimEnd()}
...`}function R1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=R(),{toolName:s,description:i,parameters:o,allowAlways:u,approvalDetails:c=[]}=e,[d,m]=p.default.useState(!1),[f,h]=p.default.useState(!1),[x,y]=p.default.useState(!1),$=p.default.useRef(!1);p.default.useEffect(()=>{h(!1),$.current=!1,y(!1)},[e]);let g=p.default.useMemo(()=>_1(s,i,o),[s,i,o]),v=s||r("approval.thisTool"),b=t4(o,c),w=f?"max-h-72":"max-h-36",S=p.default.useCallback(async k=>{if(!$.current){$.current=!0,y(!0);try{await k?.()}finally{$.current=!1,y(!1)}}},[]),E=p.default.useCallback(()=>{S(d&&u?n:t)},[d,u,n,t,S]);return l`
    <div
      data-testid="approval-card"
      className="mx-auto max-w-lg rounded-xl border border-copper/30 bg-copper/10 p-4"
    >
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-copper/25 bg-copper/10 text-copper">
          <${M} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">${r("approval.title")}</span>
        <${q}
          tone=${g.tone}
          label=${r(g.key)}
          dot=${!1}
          size="sm"
          className="ml-auto"
        />
      </div>
      ${s&&l`<div className="mb-1 break-all font-mono text-sm font-medium text-iron-100">${s}</div>`}
      ${i&&l`<div className="mb-3 break-words text-sm text-iron-200">${i}</div>`}
      ${c.length>0?l`
            <dl className=${`mb-2 ${w} overflow-y-auto rounded-md border border-iron-800 bg-iron-950/80 text-xs`}>
              ${c.map(k=>l`
                  <div className="grid gap-1 border-b border-iron-800/70 px-3 py-2 last:border-b-0 sm:grid-cols-[7rem_1fr]">
                    <dt className="font-medium text-iron-400">${k.label}</dt>
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${k1(k.value,f)}</dd>
                  </div>
                `)}
            </dl>
          `:o&&l`<pre className=${`mb-2 ${w} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${k1(o,f)}</pre>`}

      ${b&&l`
        <${A}
          variant="ghost"
          size="sm"
          className="mb-3 px-0 text-[var(--v2-accent)] hover:bg-transparent"
          onClick=${()=>h(k=>!k)}
          type="button"
        >
          ${r(f?"approval.showCommandPreview":"approval.viewFullCommand")}
        <//>
      `}

      ${u&&l`
        <label className="mb-3 flex items-center gap-2 text-xs text-iron-200">
          <input
            type="checkbox"
            checked=${d}
            onChange=${k=>m(k.currentTarget.checked)}
            disabled=${x}
            className="h-3.5 w-3.5 accent-[var(--v2-accent)]"
          />
          ${r("approval.alwaysAllowToolLabel",{tool:v})}
        </label>
      `}

      <div className="flex flex-wrap gap-2">
        <${A} variant="primary" onClick=${E} disabled=${x}>
          ${r(d&&u?"approval.approveAndAlways":"approval.approve")}
        <//>
        <${A}
          variant="secondary"
          onClick=${()=>S(a)}
          disabled=${x}
        >
          ${r("approval.deny")}
        <//>
      </div>
    </div>
  `}function li({icon:e="lock",headline:t,provider:a,accountLabel:n,body:r,expiresAt:s,pillHint:i,defaultExpanded:o=!0,testId:u="auth-gate",challengeKind:c="",children:d}){let m=R(),[f,h]=p.default.useState(o),x=p.default.useId(),y=n||a||"";return l`
    <div
      data-testid=${u}
      data-auth-challenge=${c||void 0}
      className="mx-auto w-full max-w-lg rounded-xl border border-[rgba(76,167,230,0.34)] bg-[rgba(76,167,230,0.08)]"
    >
      <button
        type="button"
        onClick=${()=>h($=>!$)}
        aria-expanded=${f?"true":"false"}
        aria-controls=${x}
        className="flex w-full items-center gap-3 rounded-xl border-0 bg-transparent px-4 py-3 text-left"
      >
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-md border border-[rgba(76,167,230,0.28)] bg-[rgba(76,167,230,0.1)] text-[#8fc8f2]">
          <${M} name=${e} className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold text-white">
            ${t||m("authGate.title")}
          </span>
          ${y&&l`<span className="block truncate text-xs text-iron-300">${y}</span>`}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 text-xs font-medium text-[#8fc8f2]">
          ${i&&l`<span className="hidden sm:inline">${i}</span>`}
          <${M}
            name="chevron"
            className=${["h-4 w-4",f?"rotate-180":""].join(" ")}
          />
        </span>
      </button>

      ${f&&l`
        <div
          id=${x}
          className="border-t border-[rgba(76,167,230,0.2)] px-4 pb-4 pt-3"
        >
          ${r&&l`<div className="mb-3 text-sm text-iron-200">${r}</div>`}
          ${d}
          ${s&&l`
            <p className="mt-2 text-xs text-iron-300">
              ${m("authGate.expiresAt")}: ${new Date(s).toLocaleString()}
            </p>
          `}
        </div>
      `}
    </div>
  `}function C1({gate:e,onCancel:t}){let a=R();return l`
    <${li}
      icon="lock"
      headline=${e?.headline||a("authGate.title")}
      body=${e?.body||""}
      challengeKind="other"
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
  `}function E1({gate:e,onCancel:t}){let a=R(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),o=p.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]);p.default.useEffect(()=>{i("")},[e.authorizationUrl,e.gateRef,e.runId]);let u=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),c=p.default.useCallback(()=>{if(!o){i(a("authGate.serviceUnavailable"));return}i(""),window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0)},[e.authorizationUrl,o]),d=n?a("authGate.reopenAuthorization",{provider:u}):a("authGate.openAuthorization",{provider:u});return l`
    <${li}
      icon="link"
      headline=${e?.headline||a("authGate.oauthTitle")}
      provider=${e?.provider?u:""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      expiresAt=${e?.expiresAt||""}
      pillHint=${a("authGate.pillAuthorize")}
      challengeKind="oauth_url"
    >
      <div className="flex flex-wrap gap-2">
        <${A}
          as="a"
          href=${o?e.authorizationUrl:void 0}
          target="_blank"
          rel="noopener noreferrer"
          className="auth-oauth"
          data-testid="auth-oauth-open"
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

      ${s&&l`
        <div
          className="mt-3 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
          role="alert"
        >
          ${s}
        </div>
      `}
      ${n&&l`
        <p className="mt-2 text-xs text-iron-300">${a("authGate.oauthWaiting")}</p>
      `}
    <//>
  `}function T1({gate:e,onSubmit:t,onCancel:a}){let n=R(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[u,c]=p.default.useState(!1),d=p.default.useCallback(async m=>{m.preventDefault();let f=r.trim();if(!f){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(f),s("")}catch(h){o(h?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return l`
    <${li}
      icon="lock"
      headline=${e?.headline||n("authGate.title")}
      provider=${e?.provider||""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      pillHint=${n("authGate.pillEnterToken")}
      challengeKind="manual_token"
    >
      <form onSubmit=${d}>
        <div className="mb-3">
          <${Tt}
            type="password"
            autoComplete="off"
            spellCheck=${!1}
            value=${r}
            disabled=${u}
            placeholder=${n("authGate.tokenPlaceholder")}
            aria-label=${n("authGate.tokenLabel")}
            data-testid="auth-token-input"
            error=${!!i}
            onInput=${m=>s(m.currentTarget.value)}
          />
          ${i&&l`
            <p className="mt-2 text-xs text-[var(--v2-danger-text)]" role="alert">
              ${i}
            </p>
          `}
        </div>
        <div className="flex flex-wrap gap-2">
          <${A} type="submit" variant="primary" disabled=${u}>
            ${n(u?"authGate.submitting":"authGate.submit")}
          <//>
          <${A}
            type="button"
            variant="secondary"
            disabled=${u}
            onClick=${()=>a?.()}
          >
            ${n("authGate.cancel")}
          <//>
        </div>
      </form>
    <//>
  `}var a4="/api/webchat/v2/extensions/pairing/redeem";function A1(e){return Q(a4,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function Pc({action:e}){let t=R(),a=X(),n=G({mutationFn:({code:u})=>A1(u),onSuccess:()=>{s(""),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=p.default.useState(""),i=n4(e,t),o=()=>{let u=r.trim().toUpperCase();u&&n.mutate({code:u})};return l`
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
          type="text"
          value=${r}
          onChange=${u=>s(u.target.value)}
          onKeyDown=${u=>u.key==="Enter"&&o()}
          placeholder=${i.codePlaceholder}
          data-testid="slack-pairing-code-input"
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${A}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          data-testid="slack-pairing-submit"
          onClick=${o}
          disabled=${n.isPending||!r.trim()}
        >
          ${i.submitLabel}
        <//>
      </div>

      ${n.isSuccess&&l`<p data-testid="slack-pairing-success" className="text-xs text-emerald-300">
        ${n.data?.message||i.successMessage}
      </p>`}
      ${n.isError&&l`<p data-testid="slack-pairing-error" className="text-xs text-red-300">
        ${r4(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function n4(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function r4(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function s4(e,t){return e?.channel==="slack"&&e.strategy===t}function D1({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return l`
    <div
      data-testid="channel-connect-card"
      data-channel=${a||""}
      data-strategy=${e.strategy||""}
      className="rounded-[16px] border border-white/[0.06] bg-white/[0.02] p-3"
    >
      <div className="mb-2 flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            Connect ${e.display_name||a}
          </div>
        </div>
        ${t&&l`
          <button
            type="button"
            aria-label="Dismiss connect action"
            data-testid="channel-connect-dismiss"
            onClick=${t}
            className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-iron-400 hover:bg-white/[0.04] hover:text-iron-100"
          >
            <${M} name="close" className="h-4 w-4" />
          </button>
        `}
      </div>

      ${s4(e,"inbound_proof_code")?l`<${Pc} action=${e.action} />`:l`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function i4(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):Ir.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:Ir.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:Ir.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:Ir.maxTotalBytes}:Ir}function M1(){let e=xa(),t=H({enabled:!!e,queryKey:["session"],queryFn:pc,staleTime:5*6e4});return i4(t.data)}var o4="(files attached)";function Uc({onSend:e,onCancel:t,disabled:a,sendDisabled:n=a,canCancel:r=!1,initialText:s="",resetKey:i="",draftKey:o=Wo,variant:u="dock",context:c={},statusText:d=""}){let m=R(),f=u==="hero",h=M1(),[x,y]=p.default.useState(()=>Ip(o)),[$,g]=p.default.useState(()=>Hp(o)),[v,b]=p.default.useState(""),[w,S]=p.default.useState(!1),[E,k]=p.default.useState(!1),[C,O]=p.default.useState(!1),L=p.default.useRef(null),P=p.default.useRef(null),U=p.default.useRef(!1),T=a||n||w;U.current=T;let K=p.default.useRef([]),te=p.default.useRef(Promise.resolve());p.default.useEffect(()=>{K.current=$},[$]);let se=p.default.useRef(null),ie=p.default.useRef(null),Ye=p.default.useCallback(()=>{ie.current&&(window.clearTimeout(ie.current),ie.current=null);let _=se.current;se.current=null,_&&_.scope===$t()&&Kp(_.key,_.text)},[]),Ke=p.default.useCallback(()=>{ie.current&&(window.clearTimeout(ie.current),ie.current=null),se.current=null},[]),gt=p.default.useCallback(()=>{let _=L.current;_&&(_.style.height="auto",_.style.height=`${Math.min(_.scrollHeight,200)}px`)},[]);p.default.useEffect(()=>{gt()},[x,gt]),p.default.useEffect(()=>(y(Ip(o)),()=>Ye()),[o,Ye]);let ft=p.default.useRef(o);p.default.useEffect(()=>{if(ft.current!==o){ft.current=o,g(Hp(o)),b("");return}_$(o,$)},[o,$]),p.default.useEffect(()=>{s&&(y(s),window.requestAnimationFrame(()=>{L.current&&(L.current.focus(),L.current.setSelectionRange(s.length,s.length))}))},[s,i]);let je=p.default.useCallback(_=>{a||!_||_.length===0||(te.current=te.current.then(async()=>{let{staged:D,errors:z}=await m$(_,{limits:h,existing:K.current,t:m});D.length>0&&g(F=>{let B=[...F,...D];return K.current=B,B}),b(z.length>0?z.join(" "):"")}).catch(()=>{b(m("chat.attachmentStagingFailed"))}))},[a,h,m]),St=p.default.useCallback(_=>{g(D=>{let z=D.filter(F=>F.id!==_);return K.current=z,z}),b("")},[]),Cn=p.default.useCallback(()=>{a||P.current?.click()},[a]),qt=p.default.useCallback(_=>{let D=Array.from(_.target.files||[]);je(D),_.target.value=""},[je]),Da=p.default.useCallback(async()=>{let _=x.trim(),D=$.length>0,z=_||(D?o4:"");if(!(!z||U.current)){U.current=!0,S(!0);try{if(await e(z,{attachments:$,displayContent:_})===null)return;y(""),g([]),K.current=[],b(""),Ke(),N$(o),k$(o),L.current&&(L.current.style.height="auto")}catch{}finally{U.current=a||n,S(!1)}}},[x,$,e,o,Ke,a,n]),Ma=p.default.useCallback(_=>{let D=_.target.value;y(D),se.current={key:o,text:D,scope:$t()},ie.current&&window.clearTimeout(ie.current),ie.current=window.setTimeout(Ye,300)},[o,Ye]),Oa=p.default.useCallback(async()=>{if(!(!r||E||!t)){k(!0);try{await t()}finally{k(!1)}}},[r,E,t]),It=p.default.useCallback(_=>{if(_.key==="Enter"&&!_.shiftKey){if(_.preventDefault(),L.current?.dataset?.sendDisabled==="true"||U.current)return;Da()}},[Da]),ot=p.default.useCallback(_=>{let D=Array.from(_.clipboardData?.files||[]);D.length>0&&(_.preventDefault(),je(D))},[je]),oa=p.default.useCallback(_=>{_.preventDefault(),O(!1);let D=Array.from(_.dataTransfer?.files||[]);D.length>0&&je(D)},[je]),wa=p.default.useCallback(_=>{_.preventDefault(),!a&&O(!0)},[a]),La=p.default.useCallback(_=>{_.currentTarget.contains(_.relatedTarget)||O(!1)},[]),la=x.trim()||$.length>0,Sa=a||n,W=m(f?"chat.heroPlaceholder":"chat.followUpPlaceholder"),re=h.accept.length>0?h.accept.join(","):void 0,$e=f?"w-full":"px-4 py-3 sm:px-5 lg:px-8",we=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",f?"min-h-[120px]":"",a?"opacity-70":""].join(" "),N=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",f?"min-h-[72px]":"min-h-[40px]"].join(" ");return l`
    <div className=${$e}>
      <div
        className=${we}
        onDrop=${oa}
        onDragOver=${wa}
        onDragLeave=${La}
      >
        ${C&&l`
          <div className="pointer-events-none absolute inset-1 z-10 flex items-center justify-center rounded-[16px] border border-dashed border-[color-mix(in_srgb,var(--v2-accent)_55%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-canvas)_82%,transparent)] text-sm font-medium text-[var(--v2-accent-text)]">
            ${m("chat.attachmentDropHint")}
          </div>
        `}
        ${v&&l`
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

        ${$.length>0&&l`
          <div className="mb-2 flex flex-wrap gap-2 px-1">
            ${$.map(_=>l`
                <div
                  key=${_.id}
                  className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
                >
                  ${_.previewUrl?l`<img
                        src=${_.previewUrl}
                        alt=${_.filename}
                        className="h-9 w-9 shrink-0 rounded object-cover"
                      />`:l`<span
                        className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                      >
                        <${M} name="file" className="h-4 w-4" />
                      </span>`}
                  <span className="flex min-w-0 flex-col">
                    <span className="max-w-[12rem] truncate font-medium">
                      ${_.filename}
                    </span>
                    <span className="text-[10px] text-iron-400">${_.sizeLabel}</span>
                  </span>
                  <button
                    type="button"
                    onClick=${()=>St(_.id)}
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
          ref=${L}
          data-testid="chat-composer"
          value=${x}
          onChange=${Ma}
          onKeyDown=${It}
          onPaste=${ot}
          data-send-disabled=${Sa?"true":"false"}
          placeholder=${W}
          rows=${1}
          disabled=${a}
          className=${N}
        />

        <input
          ref=${P}
          type="file"
          multiple
          accept=${re}
          className="hidden"
          onChange=${qt}
        />

        <div className="mt-2 flex items-center gap-2">
          ${Sa&&l`
            <span className="inline-flex items-center gap-2 text-xs text-[var(--v2-text-muted)]">
              <span className="h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
              ${d||m("chat.statusWorking")}
            </span>
          `}
          <div className="ml-auto flex items-center gap-1.5">
            <button
              type="button"
              onClick=${Cn}
              disabled=${a}
              aria-label=${m("chat.attachFiles")}
              title=${m("chat.attachFiles")}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <${M} name="plus" className="h-5 w-5" />
            </button>
            ${r?l`
                <${A}
                  type="button"
                  variant="danger"
                  size="icon-sm"
                  onClick=${Oa}
                  disabled=${E}
                  aria-label=${m("common.cancel")}
                  title=${m("common.cancel")}
                  className="rounded-full"
                >
                  <${M} name="close" className="h-5 w-5" />
                <//>
              `:l`
                <${A}
                  type="button"
                  variant="primary"
                  size="icon-sm"
                  onClick=${Da}
                  disabled=${Sa||w||!la}
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
  `}var O1={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function L1({status:e}){let t=R();if(e==="idle"||e==="connecting"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return l`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",O1[e]||O1.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function P1({onSuggestion:e,onSend:t,disabled:a,sendDisabled:n,initialText:r,resetKey:s,draftKey:i,context:o,statusText:u,canCancel:c,onCancel:d}){let m=R(),f=[{icon:"tool",title:m("chat.suggestion1"),detail:m("chat.suggestion1Desc")},{icon:"shield",title:m("chat.suggestion2"),detail:m("chat.suggestion2Desc")},{icon:"plug",title:m("chat.suggestion3"),detail:m("chat.suggestion3Desc")}];return l`
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
        <${Uc}
          onSend=${t}
          disabled=${a}
          sendDisabled=${n}
          initialText=${r}
          resetKey=${s}
          draftKey=${i}
          variant="hero"
          context=${o}
          statusText=${u}
          canCancel=${c}
          onCancel=${d}
        />
      </div>

      <div className="mt-8 grid w-full max-w-5xl gap-2">
        ${f.map(h=>l`
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
  `}var l4=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function U1({open:e,onClose:t}){let a=R();return e?l`
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
          ${l4.map((n,r)=>l`
              <li
                key=${r}
                className="flex items-center justify-between gap-3 text-sm text-[var(--v2-text)]"
              >
                <span>${a(n.descKey)}</span>
                <span className="flex items-center gap-1">
                  ${n.keys.map((s,i)=>l`<kbd
                      key=${i}
                      className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2 py-0.5 font-mono text-[11px] text-[var(--v2-text-muted)]"
                    >${s}</kbd>`)}
                </span>
              </li>
            `)}
        </ul>
      </div>
    </div>
  `:null}function F1(e){let t=0,a=0,n=0,r=0,s=0;for(let o of e){if(o.role==="thinking"&&(t+=1),o.role==="tool_activity"){let u=j1([o]);a+=u.tools,n+=u.failed,r+=u.declined,s+=u.running}if(u4(o)){let u=j1(o.toolCalls);a+=u.tools,n+=u.failed,r+=u.declined,s+=u.running}}let i=[];return t&&i.push(`${t} reasoning`),a&&i.push(`${a} ${a===1?"tool":"tools"}`),n&&i.push(`${n} failed`),r&&i.push(`${r} declined`),!n&&!r&&s&&i.push("running"),{hasError:n>0,hasDeclined:r>0,label:`Activity${i.length?` - ${i.join(", ")}`:""}`}}function j1(e){let t=0,a=0,n=0;for(let r of e)r.toolStatus==="error"&&(t+=1),r.toolStatus==="declined"&&(a+=1),r.toolStatus==="running"&&(n+=1);return{tools:e.length,failed:t,declined:a,running:n}}function u4(e){return e.toolCalls&&e.toolCalls.length>0}var B1=!1;function c4(){B1||!window.DOMPurify||(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),B1=!0)}function z1(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}c4();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var uh=360;function d4(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let u=s("Copy");if(u.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),u.textContent="Copied",si("Code copied",{tone:"success"}),setTimeout(()=>u.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(u),n.appendChild(r),t.scrollHeight>uh){t.style.maxHeight=`${uh}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${uh}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function m4({content:e,className:t=""}){let a=p.default.useRef(null),n=p.default.useMemo(()=>z1(e),[e]);return p.default.useEffect(()=>{d4(a.current)},[n]),l`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var ra=p.default.memo(m4);var q1={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",declined:"bg-iron-400",error:"bg-[var(--v2-danger-text)]"},f4={success:"ok",declined:"declined",error:"err",running:"run"},p4=2;function ui({activity:e}){return e.toolCalls&&e.toolCalls.length>0?l`<${v4} tools=${e.toolCalls} />`:l`<${g4} activity=${e} />`}function h4(e,t){let a=0,n=0,r=0,s=0;for(let u of t){let c=String(u.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function v4({tools:e}){let t=R(),a=e.some(o=>o.toolStatus==="error"),n=e.some(o=>o.toolStatus==="error"||o.toolStatus==="declined"),[r,s]=p.default.useState(n);if(p.default.useEffect(()=>{n&&s(!0)},[n]),e.length<=p4)return l`
      <div className="flex flex-col gap-3">
        ${e.map((o,u)=>l`<${ui}
            key=${o.id||o.callId||`${o.toolName}-${u}`}
            activity=${o}
          />`)}
      </div>
    `;let i=h4(t,e);return l`
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

      ${r&&l`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((o,u)=>l`<${ui}
              key=${o.id||o.callId||`${o.toolName}-${u}`}
              activity=${o}
            />`)}
        </div>
      `}
    </div>
  `}function g4({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:u}=e,[c,d]=p.default.useState(n==="error"||n==="declined");p.default.useEffect(()=>{(n==="error"||n==="declined")&&d(!0)},[n]);let m=q1[n]||q1.running,f=i!=null,h=p.default.useId(),x=l`
    <button
      type="button"
      onClick=${()=>d(y=>!y)}
      aria-expanded=${c?"true":"false"}
      aria-controls=${h}
      data-testid="tool-activity-toggle"
      className="v2-button flex w-full items-center gap-2.5 border-0 border-b border-iron-700/40 bg-transparent px-1 py-2 text-left text-sm"
    >
      <span className=${["h-2 w-2 shrink-0 rounded-full",m].join(" ")} />
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-wide text-iron-300"
        >${f4[n]||"run"}</span
      >
      <span className="shrink-0 truncate font-mono text-[13px] font-medium text-iron-100"
        >${a}</span
      >
      ${r&&l`<span className="min-w-0 truncate font-mono text-xs text-iron-400"
        >${r}</span
      >`}
      <span className="ml-auto flex shrink-0 items-center gap-2">
        ${f&&l`<span className="font-mono text-[11px] text-iron-300">${i}ms</span>`}
        <${M}
          name="chevron"
          className=${["h-3.5 w-3.5 text-iron-400",c?"rotate-180":""].join(" ")}
        />
      </span>
    </button>
  `;return l`
    <div
      className=${t?"":"flex gap-3"}
      data-testid="tool-activity-card"
      data-tool-name=${a||""}
      data-tool-status=${n||""}
    >
      ${!t&&l`
        <div
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
        >
          <${M} name="tool" className="h-4 w-4" />
        </div>
      `}
      <div className=${t?"min-w-0 flex-1":"min-w-0 max-w-[85%] flex-1"}>
        ${x}
        ${c&&l`<${y4}
          controlsId=${h}
          toolDetail=${r}
          toolParameters=${o}
          toolResultPreview=${u}
          toolError=${s}
          toolStatus=${n}
          toolDurationMs=${f?i:null}
        />`}
      </div>
    </div>
  `}function y4({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=R(),u=p.default.useMemo(()=>{let f=[];return r&&f.push({id:s==="declined"?"declined":"error",label:o(s==="declined"?"tool.tabDeclined":"tool.tabError")}),t&&f.push({id:"details",label:o("tool.tabDetails")}),a&&f.push({id:"params",label:o("tool.tabParameters")}),n&&f.push({id:"result",label:o("tool.tabResult")}),f},[o,r,t,a,n,s]),[c,d]=p.default.useState(null),m=c&&u.some(f=>f.id===c)?c:u[0]?.id;return p.default.useEffect(()=>{r&&d(s==="declined"?"declined":"error")},[r,s]),u.length===0?l`
      <div
        id=${e}
        className="rounded-b-lg border-x border-b border-iron-700/40 bg-iron-950 px-3 py-2 font-mono text-xs text-iron-400"
      >
        ${o("tool.noDetail")}
      </div>
    `:l`
    <div
      id=${e}
      data-testid="tool-activity-detail"
      className="rounded-b-lg border-x border-b border-iron-700/40 bg-iron-950"
    >
      <div className="flex items-center gap-1 border-b border-iron-700/40 px-2 pt-1.5">
        ${u.map(f=>l`
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
        ${m==="details"&&l`<div className="whitespace-pre-wrap text-iron-200">${t}</div>`}
        ${m==="params"&&l`<pre className="overflow-x-auto rounded bg-iron-900 p-2 font-mono text-iron-100">${a}</pre>`}
        ${m==="result"&&l`<${b4} text=${n} />`}
        ${(m==="error"||m==="declined")&&l`<pre
          className=${["overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono",m==="declined"?"text-iron-300":"text-[var(--v2-danger-text)]"].join(" ")}
        >${r}</pre>`}
      </div>
    </div>
  `}function b4({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return l`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(x4)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return l`
      <div className="overflow-x-auto rounded border border-iron-700/60">
        <table className="w-full border-collapse text-left font-mono text-[11px]">
          <thead>
            <tr>
              ${n.map(r=>l`<th
                  key=${r}
                  className="border-b border-iron-700/60 bg-iron-900 px-2 py-1 font-semibold text-iron-100"
                >${r}</th>`)}
            </tr>
          </thead>
          <tbody>
            ${a.map((r,s)=>l`<tr key=${s}>
                ${n.map(i=>l`<td
                    key=${i}
                    className="border-b border-iron-700/40 px-2 py-1 text-iron-200"
                  >${$4(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?l`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:l`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function x4(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function $4(e){return e==null?"":String(e)}function I1({activity:e}){let t=F1(e),a=N4(e),[n,r]=p.default.useState(a);return p.default.useEffect(()=>{a&&r(!0)},[a]),l`
    <div className="mr-auto flex w-full max-w-[85%] flex-col" data-testid="activity-run">
      <button
        type="button"
        onClick=${()=>r(s=>!s)}
        aria-expanded=${n?"true":"false"}
        data-testid="activity-run-toggle"
        className=${["v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",t.hasError?"text-[var(--v2-danger-text)]":"text-iron-400 hover:text-iron-200"].join(" ")}
      >
        <${M} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${t.label}</span>
        <${M}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0",n?"rotate-180":""].join(" ")}
        />
      </button>

      ${n&&l`
        <div className="mt-2 flex flex-col gap-3" data-testid="activity-run-items">
          ${e.map((s,i)=>l`
            <${w4}
              key=${s.id||`${s.role||"activity"}-${i}`}
              item=${s}
            />
          `)}
        </div>
      `}
    </div>
  `}function w4({item:e}){if(e.role==="thinking")return l`<${S4} content=${e.content} />`;if(e.role==="tool_activity"||ch(e)){let t=ch(e)?{id:e.id,toolCalls:e.toolCalls}:e;return l`<${ui} activity=${t} />`}return null}function S4({content:e}){return e?l`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <${M} name="spark" className="h-4 w-4" />
      </div>
      <div className="min-w-0 max-w-[85%] flex-1 border-l-2 border-white/10 pl-3 text-iron-300">
        <${ra} content=${e} className="text-[13px]" />
      </div>
    </div>
  `:null}function ch(e){return e?.toolCalls&&e.toolCalls.length>0}function N4(e){return(e||[]).some(t=>t?.role==="thinking"||t?.toolStatus==="running"||t?.toolStatus==="error"||t?.toolStatus==="declined"?!0:ch(t)?t.toolCalls.some(a=>a?.toolStatus==="running"||a?.toolStatus==="error"||a?.toolStatus==="declined"):!1)}function ci(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function _4({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=p.default.useState(()=>t&&e.preview_url||null);return p.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return gc(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?l`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:l`<${M} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var K1="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",H1="px-3 py-2";function jc({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await Ca(e.fetch_url);ci(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),u=l`
    <${_4} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?l`<div
      className=${`${K1} ${H1} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${u}
    </div>`:l`<div className=${`${K1} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${H1} text-left transition-colors hover:bg-iron-900/80`}
    >
      ${u}
    </button>
    ${e.fetch_url&&l`<button
      type="button"
      onClick=${o}
      disabled=${s}
      aria-label=${`Download ${e.filename||"attachment"}`}
      data-testid=${r}
      className="flex shrink-0 items-center border-l border-iron-700 px-2.5 text-iron-200 transition-colors hover:bg-iron-900/80 hover:text-white disabled:opacity-50"
    >
      <${M} name="download" className="h-3.5 w-3.5" />
    </button>`}
  </div>`}var Q1={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function di({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return p.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),p.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?l`
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
        className=${Y("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",Q1[n]??Q1.md,r)}
      >
        ${a?l`<${dh} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function dh({children:e,onClose:t,className:a=""}){return l`
    <div
      className=${Y("flex shrink-0 items-center justify-between gap-4","px-5 py-4 md:px-7 md:py-5","border-b border-[var(--v2-panel-border)]",a)}
    >
      <h2
        className="text-[1.1rem] font-semibold tracking-[-0.02em] text-[var(--v2-text-strong)] md:text-[1.2rem]"
      >
        ${e}
      </h2>
      ${t&&l`
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
  `}function mi({children:e,className:t=""}){return l`
    <div className=${Y("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function fi({children:e,className:t=""}){return l`
    <div
      className=${Y("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var V1=1e5;function Fc({attachment:e,onClose:t}){let a=!!e,[n,r]=p.default.useState("loading"),[s,i]=p.default.useState({}),o=e?d$(e.mime_type):"download";if(p.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return Ca(e.fetch_url).then(async m=>{d=URL.createObjectURL(m);let f={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")f.dataUrl=await Op(m);else if(o==="pdf")f.frameUrl=d;else if(o==="text"){let h=await m.text();f.truncated=h.length>V1,f.text=f.truncated?h.slice(0,V1):h}if(c){URL.revokeObjectURL(d);return}i(f),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let u=e.filename||"attachment";return l`
    <${di} open=${a} onClose=${t} size="xl">
      <${dh} onClose=${t}>
        <span className="block truncate">${u}</span>
      <//>
      <${mi} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&l`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&l`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&l`<${k4} mode=${o} view=${s} filename=${u} />`}
      <//>
      <${fi}>
        ${s.downloadUrl&&l`<a
          href=${s.downloadUrl}
          download=${u}
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
  `}function k4({mode:e,view:t,filename:a}){switch(e){case"image":return l`<img
        src=${t.dataUrl}
        alt=${a}
        className="mx-auto max-h-[70vh] w-auto rounded object-contain"
      />`;case"audio":return l`<audio controls src=${t.dataUrl} className="w-full" />`;case"video":return l`<video controls src=${t.dataUrl} className="max-h-[70vh] w-full rounded" />`;case"pdf":return l`<iframe
        src=${t.frameUrl}
        title=${a}
        className="h-[70vh] w-full rounded border border-iron-700 bg-white"
      />`;case"text":return l`<div className="w-full">
        <pre
          className="max-h-[70vh] w-full overflow-auto whitespace-pre-wrap break-words rounded bg-iron-900/60 p-3 text-xs text-iron-200"
        >${t.text}</pre>
        ${t.truncated&&l`<div className="mt-2 text-xs text-iron-400">
          Preview truncated — download the file to see the rest.
        </div>`}
      </div>`;default:return l`<div className="flex flex-col items-center gap-2 text-iron-400">
        <${M} name="file" className="h-10 w-10 text-signal" />
        <div className="text-sm">This file type can't be previewed.</div>
      </div>`}}var R4=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function C4(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function G1(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of C4(e).matchAll(R4)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function Y1(e){return e.split("/").filter(Boolean).pop()||e}function J1(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function E4({threadId:e,path:t,onPreview:a}){let[n,r]=p.default.useState({mime_type:"",size_label:""});p.default.useEffect(()=>{let i=!0;return Mx({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:J1(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:Y1(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:vc({threadId:e,path:t})};return l`<${jc}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function X1({threadId:e,content:t}){let a=p.default.useMemo(()=>G1(t),[t]),[n,r]=p.default.useState(null);return!e||a.length===0?null:l`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>l`<${E4}
          key=${s}
          threadId=${e}
          path=${s}
          onPreview=${r}
        />`)}
      <${Fc}
        attachment=${n}
        onClose=${()=>r(null)}
      />
    </div>
  `}var Z1={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function T4(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function A4({content:e}){let[t,a]=p.default.useState(!1);return e?l`
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
      ${t&&l`
        <div className="mt-1 border-l-2 border-white/10 pl-3 text-iron-300">
          <${ra} content=${e} className="text-[13px]" />
        </div>
      `}
    </div>
  `:null}function D4({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:u,status:c,error:d,toolCalls:m,timestamp:f}=e,h=n==="user",[x,y]=p.default.useState(!1),[$,g]=p.default.useState(null),v=p.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),y(!0),si("Copied to clipboard",{tone:"success"}),setTimeout(()=>y(!1),1400)}catch{}},[r]);if(n==="tool_activity"||m&&m.length>0){let L=m&&m.length>0?{id:e.id,toolCalls:m}:e;return l`<${ui} activity=${L} />`}if(n==="thinking")return l`<${A4} content=${r} />`;if(n==="image")return l`
      <div className="flex">
        <div className="flex flex-wrap gap-2">
          ${(o||[]).map((P,U)=>P.data_url?l`<img key=${U} src=${P.data_url} className="max-h-64 rounded-lg border border-iron-700 object-cover" alt="Generated result" />`:l`
                  <div key=${U} className="rounded-lg border border-iron-700 bg-iron-900/70 px-4 py-3 text-sm text-iron-200">
                    <div>Generated image unavailable in history payload</div>
                    ${P.path&&l`<div className="mt-1 font-mono text-xs text-iron-300">${P.path}</div>`}
                  </div>
                `)}
        </div>
      </div>
    `;let b=T4(f),w=n==="user"||n==="assistant"&&!u,S=n==="system"||n==="error",E=h?"max-w-[85%]":S?"mx-auto max-w-[85%]":"w-full max-w-[85%]",k=h?"":"w-full min-w-0 max-w-full",C=c==="error"&&t,O=w||C||b;return l`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",h?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col",E].join(" ")}>
        <div
          className=${["text-base leading-7",k,Z1[n]||Z1.assistant,u?"opacity-70":""].join(" ")}
        >
          ${n==="assistant"||n==="system"||n==="error"?l`<${ra} content=${r} />`:l`<div className="whitespace-pre-wrap">${r}</div>`}

          ${c==="error"&&l`
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-300">
              <span>${d}</span>
            </div>
          `}

          ${s&&s.length>0&&l`
            <div className="mt-2 flex flex-wrap gap-2">
              ${s.map((L,P)=>l`<img key=${P} src=${L} className="max-h-48 rounded-lg border border-iron-700 object-cover" alt="Message attachment" />`)}
            </div>
          `}

          ${i&&i.length>0&&l`
            <div className="mt-2 flex flex-col gap-1.5">
              ${i.map((L,P)=>l`<${jc}
                key=${L.id||P}
                att=${L}
                onPreview=${g}
              />`)}
            </div>
            <${Fc}
              attachment=${$}
              onClose=${()=>g(null)}
            />
          `}

          ${n==="assistant"&&l`<${X1}
            threadId=${a}
            content=${typeof r=="string"?r:""}
          />`}
        </div>
      </div>

      ${O&&l`
        <div
          className=${["mt-1 flex min-h-7 w-max max-w-[85%] flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100",h?"self-end justify-end":S?"self-center justify-center":"self-start justify-start"].join(" ")}
        >
          ${b&&l`<time dateTime=${f} className="shrink-0 font-mono text-[11px] text-iron-500">${b}</time>`}
          ${(w||C)&&l`
            <div className="flex shrink-0 items-center gap-1">
            ${w&&l`
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
            ${C&&l`
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
  `}var W1=p.default.memo(D4);function s2(e){let t=M4(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(i2(r)){let s=e2(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){t2(a,s),a2(a,r),n+=s.length;continue}}if(mh(r)){let s=e2(t,n);t2(a,s),n+=s.length-1;continue}a2(a,r)}return a}function M4(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=Bc(i);o&&i2(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!mh(i))continue;let o=Bc(i),u=o?t.get(o):void 0;if(u===void 0||u>=s)continue;let c=a.get(u)||[];c.push(i),a.set(u,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function e2(e,t){let a=t,n=Bc(e[t]);for(;a<e.length&&mh(e[a])&&O4(n,e[a]);)a+=1;return e.slice(t,a)}function O4(e,t){let a=Bc(t);return!e||!a||a===e}function t2(e,t){if(t.length===0)return;let a=L4(t);e.push({type:"activity-run",id:`activity-run-${a[0].id}`,activity:a})}function a2(e,t){e.push({type:"message",id:t.id,message:t})}function i2(e){return e.role==="assistant"&&!o2(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function mh(e){return e.role==="thinking"||e.role==="tool_activity"||o2(e)}function o2(e){return e?.toolCalls&&e.toolCalls.length>0}function Bc(e){return e?.turnRunId||null}function L4(e){return[...e].sort((t,a)=>t?.role!=="tool_activity"||a?.role!=="tool_activity"?0:P4(t,a))}function P4(e,t){if(Number.isFinite(e.activityOrder)&&Number.isFinite(t.activityOrder)){let n=e.activityOrder-t.activityOrder;if(n!==0)return n}let a=n2(r2(e.updatedAt||e.timestamp),r2(t.updatedAt||t.timestamp));return a!==0?a:n2(e.sequence,t.sequence)}function n2(e,t){let a=Number.isFinite(e)?e:null,n=Number.isFinite(t)?t:null;return a===null&&n===null?0:a===null?1:n===null?-1:a-n}function r2(e){if(!e)return null;let t=Date.parse(e);return Number.isFinite(t)?t:null}var U4=100,j4=100;function F4(e){return e?e.scrollHeight-e.scrollTop-e.clientHeight:Number.POSITIVE_INFINITY}function l2(e,t=U4){return F4(e)<=t}function u2(e){e&&(e.scrollTop=Math.max(0,e.scrollHeight-e.clientHeight))}function c2(e){return e?.id?`${e.role||""}:${e.id}`:null}function B4(e,t){let a=c2(t);return!!(a&&t?.role==="user"&&a!==e)}function z4(){return typeof window>"u"||!window.getSelection?"":String(window.getSelection()?.toString?.()||"")}function d2({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let u=R(),c=p.default.useRef(null),d=p.default.useRef(null),m=p.default.useRef(!0),f=p.default.useRef(null),h=p.default.useRef(null),x=p.default.useRef(null),y=p.default.useRef(0),$=p.default.useRef(!1),[g,v]=p.default.useState(!0),b=p.default.useCallback(()=>{h.current!==null&&(window.cancelAnimationFrame(h.current),h.current=null)},[]),w=p.default.useCallback((T=!1)=>{c.current&&(T&&(m.current=!0,$.current=!1),m.current&&(b(),h.current=window.requestAnimationFrame(()=>{h.current=null;let te=c.current;!te||!T&&!m.current||(u2(te),y.current=te.scrollTop,$.current=!1,v(!0))})))},[b]),S=p.default.useCallback(()=>{x.current!==null&&(window.cancelAnimationFrame(x.current),x.current=null)},[]);p.default.useLayoutEffect(()=>{let T=e.length>0?e[e.length-1]:null,K=c2(T),te=B4(f.current,T);return f.current=K,w(te),b},[e,i,w,b]),p.default.useLayoutEffect(()=>{let T=d.current;if(!T||typeof ResizeObserver!="function")return;let K=new ResizeObserver(()=>{w()});return K.observe(T),()=>{K.disconnect(),b()}},[w,b]);let E=p.default.useCallback(()=>{x.current=null;let T=c.current;if(!T)return;let K=l2(T);y.current=T.scrollTop,K?(m.current=!0,$.current=!1,v(!0)):$.current?(m.current=!1,v(!1)):(m.current=!0,v(!0),w()),a&&T.scrollTop<j4&&n&&!t&&n()},[a,n,t,w]),k=p.default.useCallback(()=>{$.current=!0},[]),C=p.default.useCallback(T=>{let K=c.current;if(!K||typeof T?.clientX!="number")return;let te=K.offsetWidth-K.clientWidth;if(te<=0)return;let se=K.getBoundingClientRect().right;T.clientX>=se-te-2&&($.current=!0)},[]),O=p.default.useCallback(()=>{let T=c.current;if(!T)return;let K=l2(T),te=T.scrollTop<y.current;y.current=T.scrollTop,!K&&te&&($.current=!0),K?(m.current=!0,$.current=!1):$.current?(m.current=!1,b()):m.current=!0,x.current===null&&(x.current=window.requestAnimationFrame(E))},[b,E]),L=p.default.useCallback(()=>{let T=c.current;T&&(u2(T),y.current=T.scrollTop,m.current=!0,$.current=!1,v(!0))},[]),P=p.default.useCallback(T=>{let K=z4();!K||!T.clipboardData||(T.preventDefault(),T.clipboardData.clearData(),T.clipboardData.setData("text/plain",K))},[]);p.default.useEffect(()=>S,[S]);let U=p.default.useMemo(()=>s2(e),[e]);return l`
    <div className="relative flex min-h-0 min-w-0 flex-1">
    <div
      ref=${c}
      onScroll=${O}
      onWheel=${k}
      onTouchMove=${k}
      onPointerDown=${C}
      onCopy=${P}
      data-testid="message-list-scroll"
      className="flex min-w-0 flex-1 overflow-y-auto px-4 pt-6 pb-14 sm:px-5 lg:px-8"
    >
      <div
        ref=${d}
        data-testid="message-list-content"
        className="mx-auto flex w-full min-w-0 max-w-5xl flex-col gap-5"
      >
        ${a&&l`
          <div className="text-center">
            <button
              onClick=${n}
              disabled=${t}
              data-testid="message-list-load-older"
              className="v2-button rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-300 hover:border-signal/35 hover:text-white disabled:opacity-50"
            >
              ${u(t?"chat.history.loading":"chat.history.loadOlder")}
            </button>
          </div>
        `}
        ${U.map(T=>T.type==="activity-run"?l`<${I1} key=${T.id} activity=${T.activity} />`:l`<${W1}
                key=${T.id}
                message=${T.message}
                onRetry=${r}
                threadId=${s}
              />`)}
        ${o}
      </div>
    </div>
    ${!g&&l`
      <button
        type="button"
        onClick=${L}
        aria-label=${u("chat.jumpToLatest")}
        className="absolute bottom-4 left-1/2 inline-flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-3 py-1.5 text-xs font-medium text-[var(--v2-text-strong)] shadow-[0_10px_30px_-12px_rgba(0,0,0,0.7)] hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]"
      >
        <${M} name="arrowDown" className="h-3.5 w-3.5" />
        ${u("chat.jumpToLatest")}
      </button>
    `}
    </div>
  `}function m2({notice:e,onRecover:t}){return l`
    <div className="mx-auto flex max-w-xl flex-wrap items-center justify-center gap-3 rounded-lg border border-copper/30 bg-copper/10 px-4 py-3 text-sm text-copper">
      <span>${e.message}</span>
      ${e.status!=="loading"&&l`
        <button
          type="button"
          onClick=${t}
          className="rounded-md border border-copper/40 px-2.5 py-1 text-xs font-medium hover:bg-copper/10"
        >
          Reload history
        </button>
      `}
    </div>
  `}function f2({suggestions:e,onSelect:t,disabled:a=!1}){return!e||e.length===0?null:l`
    <div className="px-4 pb-3 sm:px-5 lg:px-8">
      <div className="mx-auto flex max-w-5xl flex-wrap gap-2">
        ${e.map(n=>l`
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
  `}function p2(){return l`
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
  `}function zc(){return Q("/api/webchat/v2/channels/connectable")}function h2(e,t){if(!fh(e))return null;let a=qc(e),n=H4(a),r=null;for(let s of t||[]){if(!K4(s))continue;let i=Q4(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function fh(e){let t=qc(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function q4(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function I4(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>v2(qc(n))):a}function K4(e){return e?.strategy!=="admin_managed_channels"}function H4(e){return g2(e,"slack")&&v2(e)}function v2(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function qc(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function Q4(e,t,a={}){return(a.commandAliasesOnly?I4(t,{channelManagementOnly:!0}):q4(t)).reduce((r,s)=>{let i=qc(s);return g2(e,i)?Math.max(r,i.length):r},0)}function g2(e,t){return t?` ${e} `.includes(` ${t} `):!1}function y2(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n={kind:"gate",gateKind:"approval",runId:t.turn_run_id,gateRef:t.gate_ref,invocationId:t.invocation_id||null,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return V4(n,a,t.body)}return e==="auth_required"?{kind:"auth_required",gateKind:"auth",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,invocationId:t.invocation_id||null,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function b2(e){if(!e?.run_id||!e.gate_ref)return null;let t=e.gate_kind||"generic",a={gateKind:t,runId:e.run_id,gateRef:e.gate_ref,invocationId:e.invocation_id||null,headline:e.headline,body:e.body||"",allowAlways:e.allow_always===!0};if(t==="auth"){let n=e.auth_context||{};return{...a,kind:"auth_required",challengeKind:n.challenge_kind||"other",provider:n.provider||null,accountLabel:n.account_label||"",authorizationUrl:n.authorization_url||null,expiresAt:n.expires_at||null}}return{...a,kind:"gate"}}function V4(e,t,a){if(!t)return e;let n=G4(t);return{...e,toolName:t.tool_name||null,description:t.reason||a,actionLabel:t.action?.label||null,destination:t.destination||null,approvalScope:t.scope||null,approvalDetails:n,parameters:n.length?n.map(r=>`${r.label}: ${r.value}`).join(`
`):null}}function G4(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function x2({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function $2(){return{terminalByInvocation:new Map}}function w2(e){e?.current?.terminalByInvocation?.clear()}function hh(e,t,a){let n=N2(t,{toolStatus:"running"});n&&pi(e,n,a)}function S2(e,t,a,n="gate_declined"){let r=N2(t,{toolStatus:"declined",toolError:n,toolErrorKind:"gate_declined"});r&&pi(e,r,a)}function pi(e,t,a){if(!t)return;let n=e5(t);n=W4(n,a),e(r=>{let s=_2(n),i=J4(r,n,s);if(i>=0){let u=[...r];return u[i]=X4(u[i],n),ph(u[i],a),u}let o={id:s,role:"tool_activity",...n};return ph(o,a),[...r,o]})}function N2(e,t={}){let a=e?.kind==="gate",n=e?.kind==="auth_required",r=n&&t.toolStatus==="declined";if(!e?.runId||!e?.gateRef||!e.invocationId&&!r||!a&&!n)return null;let s=e.invocationId||Y4(e),i=e.toolName||e.headline||e.gateKind||"gate";return{invocationId:s,callId:s,capabilityId:e.toolName||e.gateKind||null,toolName:Yo(i)||i,toolStatus:t.toolStatus||"running",toolDetail:null,toolParameters:null,toolResultPreview:null,toolError:t.toolError||null,toolErrorKind:t.toolErrorKind||null,toolDurationMs:null,updatedAt:t.updatedAt||new Date().toISOString(),resultRef:null,truncated:!1,outputBytes:null,outputKind:null,turnRunId:e.runId,gateRef:e.gateRef,gateActivity:!0}}function Y4(e){return`gate:${e.runId}:${e.kind}:${e.gateRef}`}function _2(e){return`tool-${e.invocationId}`}function J4(e,t,a){let n=e.findIndex(s=>s?.id===a);if(n>=0)return n;let r=t.gateRef||null;if(r){let s=e.findIndex(i=>i?.role==="tool_activity"&&i.turnRunId===t.turnRunId&&i.gateRef===r);if(s>=0)return s}return-1}function X4(e,t){let a=Go(e.toolStatus),n=Go(t.toolStatus),r=a&&!n,s=t.gateActivity&&!e.gateActivity,i={...e,...t,id:e.id,role:"tool_activity",invocationId:e.gateActivity&&!t.gateActivity?t.invocationId:e.invocationId||t.invocationId,callId:e.gateActivity&&!t.gateActivity?t.callId:e.callId||t.callId,toolName:s?e.toolName:t.toolName||e.toolName,toolStatus:r?e.toolStatus:t.toolStatus,toolError:t.toolError||e.toolError,toolErrorKind:t.toolErrorKind||e.toolErrorKind||null,updatedAt:r?e.updatedAt||t.updatedAt:t.updatedAt||e.updatedAt,turnRunId:t.turnRunId||e.turnRunId||null,gateRef:t.gateRef||e.gateRef||null,gateActivity:e.gateActivity&&t.gateActivity,capabilityId:s?e.capabilityId||t.capabilityId||null:t.capabilityId||e.capabilityId||null,activityOrder:Z4(e,t),activityOrderSource:t.activityOrderSource||e.activityOrderSource||null};return e.gateActivity&&!t.gateActivity&&(i.id=_2(t),i.gateActivity=!1),i}function Z4(e,t){return Number.isFinite(t.activityOrder)?t.activityOrder:e.activityOrder}function W4(e,t){if(!e?.invocationId)return e;if(Go(e.toolStatus))return ph(e,t),e;let a=t?.current?.terminalByInvocation?.get(e.invocationId);return a?Number.isFinite(e.activityOrder)?{...a,activityOrder:e.activityOrder,activityOrderSource:e.activityOrderSource||a.activityOrderSource||null}:a:e}function ph(e,t){!e?.invocationId||!Go(e.toolStatus)||t?.current?.terminalByInvocation?.set(e.invocationId,e)}function e5(e){let t=Yo(e.toolName||e.capabilityId);return{...e,toolName:t||e.toolName||"tool"}}function T2({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o,onRunSettled:u}){let c=p.default.useRef(new Set),d=p.default.useRef(null),m=p.default.useRef(null);return p.default.useCallback(f=>{let{type:h,frame:x}=f||{};if(!(!h||!x))switch(h){case"accepted":{let y=x.ack||{};y.run_id&&(d.current=y.run_id),r?.({runId:y.run_id||null,threadId:y.thread_id||e,status:y.status||null}),a(!0);return}case"running":case"capability_progress":{let y=x.progress||{};y.turn_run_id&&(d.current=y.turn_run_id,r?.($=>$&&$.runId===y.turn_run_id?{...$,status:"running"}:{runId:y.turn_run_id,threadId:e,status:"running"}),t5(n,y.turn_run_id,m)),a(!0);return}case"capability_activity":{let y=x.activity;if(!y||!y.invocation_id)return;pi(t,zp(y),o);return}case"capability_display_preview":{let y=x.preview;if(!y||!y.invocation_id)return;let $=Bp(y);pi(t,$,o);return}case"gate":case"auth_required":{let y=y2(h,x.prompt);y&&(hh(t,y,o),n(y),r?.({runId:y.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let y=x.reply||{};t($=>[...$,{id:`reply-${y.turn_run_id||Date.now()}`,role:"assistant",content:y.text||"",timestamp:y.generated_at||new Date().toISOString(),turnRunId:y.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let y=x.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Hc(c,u,y,!1);return}case"failed":{let y=x.run_state||{},$=y.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),gh(t,{runId:$,status:y.status||"failed",failureCategory:s5(y),failureSummary:null}),Hc(c,u,$,!1);return}case"projection_snapshot":case"projection_update":{let y=x.state?.items||[];n5({items:y,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:u,settledRunsRef:c,latestRunIdRef:d,promptRunIdRef:m,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i,o,u])}function Hc(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var k2=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),R2=new Set(["completed","succeeded"]),Ic=new Set(["blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]),Kc=new Set(["awaiting_gate","blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]);function C2(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function t5(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function a5(e,t,a,n,r,s){let i=t?.runId||null;if(!i||r?.has(i))return!0;let o=a?.get(i);if(o)return!Kc.has(o);let u=e?.current,c=u?.runId||n?.current||null;if(c&&i!==c)return!0;let d=s?.current===c;return c&&i===c&&!d&&u?.status&&!Kc.has(u.status)?!0:!u?.runId||!u.status?!1:!Kc.has(u.status)}function n5({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,activeRunRef:d,locallyResolvedGatesRef:m,toolActivityStateRef:f}){let h=new Map,x=new Set,y=d?.current||null,$=y?.runId||u?.current||null;for(let v of e){let b=v.run_status;b?.run_id&&b.status&&(h.set(b.run_id,b.status),$&&$!==b.run_id&&y?.status&&!k2.has(y.status)&&Ic.has(b.status)&&x.add(b.run_id))}let g=u?.current??null;for(let v of e){if(v.run_status){let{run_id:b,status:w,failure_category:S,failure_summary:E}=v.run_status,k=k2.has(w),C=d?.current?.source==="local"?d.current.runId:null,O=!!(b&&C&&C!==b),L=g??u?.current??null,P=!!(k&&b&&L&&L!==b),U=b&&Ic.has(w)?E2(m,b):null;if(b&&x.has(b)||O)continue;if(P){E2(m,d?.current?.runId)?.outcome==="resumed"&&(r5({runId:b,activePromptRunId:d?.current?.runId,success:R2.has(w),status:w,failureCategory:S,failureSummary:E,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,locallyResolvedGatesRef:m}),g=null);continue}if(U){C2(r,b,c),U.outcome==="resumed"?(n(!0),s?.(T=>T&&T.runId===b?{...T,status:T.status==="awaiting_gate"?"queued":T.status||"queued"}:{runId:b,threadId:t,status:"queued"}),g=b,u&&(u.current=b)):(n(!1),d?.current?.runId===b&&s?.(null),g=null,u?.current===b&&(u.current=null));continue}b&&(g=b,!k&&u&&(u.current=b),s?.(T=>T&&T.runId===b?{...T,status:w}:{runId:b,threadId:t,status:w})),b&&Ic.has(w)?c&&(c.current=b):b&&c?.current===b&&(c.current=null),k?(n(!1),r(null),s?.(null),vh(m,b),g=null,u&&(u.current=null),b&&c?.current===b&&(c.current=null),Hc(o,i,b,R2.has(w)),(w==="failed"||w==="recovery_required")&&gh(a,{runId:b,status:w,failureCategory:S,failureSummary:E})):Ic.has(w)||(C2(r,b,c),vh(m,b),n(!0))}if(v.text){let b=`text-${v.text.id}`;a(w=>{let S=w.findIndex(k=>k.id===b),E={id:b,role:"assistant",content:v.text.body||"",timestamp:new Date().toISOString(),isFinalReply:!0};if(S>=0){let k=[...w];return k[S]=E,k}return[...w,E]}),n(!1)}if(v.thinking){let b=`thinking-${v.thinking.id}`;a(w=>{let S=w.findIndex(k=>k.id===b),E={id:b,role:"thinking",content:v.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:v.thinking.run_id||null};if(S>=0){let k=[...w];return k[S]=E,k}return[...w,E]})}if(v.capability_activity){let b=v.capability_activity;b.invocation_id&&pi(a,zp(b),f)}if(v.gate){let b=b2(v.gate),w=b?.runId||null;w&&!a5(d,b,h,u,x,c)&&!o5(m,w,b.gateRef)&&(hh(a,b,f),r(S=>S||b),s?.(S=>S&&S.runId===w?{...S,status:Kc.has(S.status)?S.status:"awaiting_gate"}:{runId:w,threadId:t,status:"awaiting_gate"}),c&&(c.current=w),n(!1))}if(v.skill_activation){let{id:b,skill_names:w=[],feedback:S=[]}=v.skill_activation;if(w.length||S.length){let E=`skill-${b||w.join("-")||"activation"}`,k=[w.length?`Skill activated: ${w.join(", ")}`:"",...S].filter(Boolean).join(`
`);a(C=>C.some(O=>O.id===E)?C:[...C,{id:E,role:"system",content:k,timestamp:new Date().toISOString()}])}}}u&&g&&(u.current=g)}function r5({runId:e,activePromptRunId:t,success:a,status:n,failureCategory:r,failureSummary:s,setMessages:i,setIsProcessing:o,setPendingGate:u,setActiveRun:c,onRunSettled:d,settledRunsRef:m,latestRunIdRef:f,promptRunIdRef:h,locallyResolvedGatesRef:x}){o(!1),u(null),c?.(null),vh(x,t),f&&(f.current=null),h?.current===t&&(h.current=null),Hc(m,d,e,a),(n==="failed"||n==="recovery_required")&&gh(i,{runId:e,status:n,failureCategory:r,failureSummary:s})}function s5(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function gh(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`;e(i=>{let o=i.findIndex(c=>c.id===s),u=x2({status:a,failureCategory:n,failureSummary:r});if(o>=0){if(!r||i[o].content===u)return i;let c=[...i];return c[o]={...c[o],content:u},c}return[...i,{id:s,role:"error",content:u,timestamp:new Date().toISOString()}]})}function E2(e,t){if(!t)return null;let a=e?.current;if(!a)return null;for(let[n,r]of a.entries())if(n.startsWith(`${t}
`))return i5(r);return null}function i5(e){return e&&typeof e=="object"?{resolution:e.resolution||null,outcome:e.outcome||null}:{resolution:e||null,outcome:null}}function vh(e,t){if(!t)return;let a=e?.current;if(a)for(let n of Array.from(a.keys()))n.startsWith(`${t}
`)&&a.delete(n)}function o5(e,t,a){return!t||!a?!1:!!e?.current?.has(`${t}
${a}`)}function A2(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function D2(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function M2(e,t,a,n){let r=yh(n);return r?(l5(e,t,a,{timelineMessageId:r}),r):null}function l5(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function yh(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var u5=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function O2({threadId:e,onEvent:t,enabled:a}){let[n,r]=p.default.useState("idle"),s=p.default.useRef(t);s.current=t;let i=p.default.useRef(null);return p.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,u=null,c=0,d=3e4;function m(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=Gx({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let y=Math.min(1e3*2**c,d);u=setTimeout(m,y)};let x=(y,$)=>{let g=null;try{g=JSON.parse(y.data)}catch{return}!g||typeof g!="object"||(y.lastEventId&&(i.current=y.lastEventId),s.current?.({type:g.type||$,frame:g,lastEventId:y.lastEventId||null}))};o.onmessage=y=>x(y,"message");for(let y of u5)o.addEventListener(y,$=>x($,y))}function f(){u&&(clearTimeout(u),u=null),o&&(o.close(),o=null),r("paused")}function h(){document.visibilityState==="hidden"?f():o||m()}return m(),document.addEventListener("visibilitychange",h),()=>{document.removeEventListener("visibilitychange",h),u&&clearTimeout(u),o&&o.close()}},[a,e]),{status:n}}var c5=3e4,d5="credential_stored_gate_resolution_failed",m5="approval_gate_pending_send_blocked",f5="ironclaw-product-auth",bh="ironclaw:product-auth:oauth-complete",p5="ironclaw:product-auth:oauth-complete";async function L2(e){let t=new AbortController,a=setTimeout(()=>t.abort(),c5);try{return await e(t.signal)}finally{clearTimeout(a)}}function h5(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=d5,t.cause=e,t}function v5(){let e=new Error("Resolve the approval request before sending another message.");return e.safeErrorCode=m5,e}function g5(e){let a=Ct.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function P2(e,t){return!e||!t?.runId||!t?.gateRef?null:`${e}
${t.runId}
${t.gateRef}`}function y5(e){return e?.continuation?.type==="turn_gate_resume"}function b5(e){if(e?.outcome)return e.outcome;let t=String(e?.status||"").toLowerCase();return t==="queued"||t==="running"?"resumed":t==="cancelled"||e?.already_terminal===!0?"cancelled":e?.already_terminal===!1?"resumed":null}function U2(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function x5(e){return e?.type===p5&&e?.status==="completed"}function $5(e,t,a){if(!x5(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function xh(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function w5(e){if(!fh(e))return null;try{let a=(await Ct.fetchQuery({queryKey:["connectable-channels"],queryFn:zc}))?.channels||[];return h2(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function j2(e){let t=p.default.useRef(e),a=p.default.useRef(new Map),n=p.default.useRef(1),[r,s]=p.default.useState(0),[i,o]=p.default.useState(Date.now()),[u,c]=p.default.useState(null),d=p.default.useRef(u),m=p.default.useCallback(W=>{let re=typeof W=="function"?W(d.current):W;d.current=re,c(re)},[]);p.default.useEffect(()=>{d.current=u},[u]);let[f,h]=p.default.useState(null),x=p.default.useCallback(()=>a.current.get(e||"__new__")||[],[e]),y=p.default.useCallback(W=>{let re=e||"__new__";W.length>0?a.current.set(re,W):a.current.delete(re)},[e]),{messages:$,hasMore:g,nextCursor:v,isLoading:b,loadError:w,loadHistory:S,seedThreadMessages:E,setMessages:k}=w$(e,{getPendingMessages:x,setPendingMessages:y}),[C,O]=p.default.useState(!1),L=p.default.useRef(C),P=p.default.useCallback(W=>{let re=typeof W=="function"?W(L.current):W;L.current=re,O(re)},[]),[U,T]=p.default.useState(null),K=p.default.useRef(U),[te,se]=p.default.useState(null),ie=p.default.useCallback(W=>{let re=K.current,$e=typeof W=="function"?W(re):W;Object.is($e,re)||(K.current=$e,T($e))},[]),[Ye,Ke]=p.default.useState(e),gt=p.default.useRef($2()),ft=p.default.useRef(new Map),je=p.default.useRef({gateKey:null,credentialRef:null,inFlight:!1}),St=p.default.useRef(!1);Ye!==e&&(Ke(e),O(!1),T(null),se(null),c(null),h(null)),p.default.useEffect(()=>{t.current=e},[e]),p.default.useEffect(()=>{K.current=U},[U]),p.default.useEffect(()=>{L.current=C},[C]),p.default.useEffect(()=>{let W=P2(e,U);se(re=>re&&re.gateKey!==W?null:re)},[U,e]),p.default.useEffect(()=>{w2(gt),ft.current.clear()},[e]);let Cn=Math.max(0,Math.ceil((r-i)/1e3)),qt=U?.runId&&U?.gateRef?`${U.runId}
${U.gateRef}`:null;p.default.useEffect(()=>{if(!r)return;let W=setInterval(()=>o(Date.now()),250);return()=>clearInterval(W)},[r]),p.default.useEffect(()=>{je.current.gateKey!==qt&&(je.current={gateKey:qt,credentialRef:null,inFlight:!1})},[qt]),p.default.useEffect(()=>{if(!U2(U))return;let W=Date.now(),re=_=>{$5(_,U,W)&&(ie(D=>U2(D)?null:D),P(!0))},$e=null;typeof window.BroadcastChannel=="function"&&($e=new window.BroadcastChannel(f5),$e.onmessage=_=>re(_.data));let we=_=>{_.key===bh&&re(xh(_.newValue))};window.addEventListener("storage",we),re(xh(window.localStorage?.getItem?.(bh)));let N=window.setInterval(()=>{re(xh(window.localStorage?.getItem?.(bh)))},500);return()=>{window.clearInterval(N),$e&&$e.close(),window.removeEventListener("storage",we)}},[U]);let Da=T2({threadId:e,setMessages:k,setIsProcessing:P,setPendingGate:ie,setActiveRun:m,activeRunRef:d,locallyResolvedGatesRef:ft,toolActivityStateRef:gt,onRunSettled:(W,{success:re})=>{St.current=!1,re&&y([]),S(void 0,{preserveClientOnly:!0,finalReplyTimestampByRun:W&&re?{[W]:new Date().toISOString()}:null})}}),{status:Ma}=O2({threadId:e,onEvent:Da,enabled:!!e}),Oa=p.default.useCallback(async(W,re={})=>{let{threadId:$e,attachments:we=[],displayContent:N}=re,_=we.map(f$),D=we.map(p$),z=typeof N=="string"?N:W;if(U||K.current)throw v5();let F=d.current,B=F&&(!$e||F.threadId===$e||F.threadId===e);if(St.current||L.current||B)return null;if(we.length===0){let ce=await w5(W);if(ce)return h(ce),{channel_connect_action:ce}}h(null);let V=$e||e;if(!V){let ce=await hc();if(Ct.invalidateQueries({queryKey:["threads"]}),V=ce?.thread?.thread_id,!V)throw new Error("createThread returned no thread_id")}let fe=V,he={id:`pending-${n.current++}`,role:"user",content:z,attachments:D,retryContent:W,retryDisplayContent:z,retryAttachments:we,timestamp:new Date().toISOString(),isOptimistic:!0},Nt={id:he.id,role:"user",content:z,attachments:D,retryContent:W,retryDisplayContent:z,retryAttachments:we,timestamp:he.timestamp,isOptimistic:!0};A2(a.current,fe,he);let Pa=he.id,Kt=!e||V===e,Zr=ce=>{Kt&&k(ce)},En=ce=>{V!==e&&E(V,ce)},gr=ce=>{Kt&&ce()};St.current=!0,Zr(ce=>[...ce,Nt]),En(ce=>[...ce,Nt]),gr(()=>{P(!0),K.current||ie(null)});try{let ce=await Hx({threadId:V,content:W,attachments:_});g5(V)&&Ct.invalidateQueries({queryKey:["threads"]}),ce?.run_id&&Kt&&m({runId:ce.run_id,threadId:ce.thread_id||V,status:ce.status||null,source:"local"});let Tn=M2(a.current,fe,Pa,ce?.accepted_message_ref)||yh(ce?.accepted_message_ref);if(Tn){let an=Ht=>Ht.map(An=>An.id===Pa?{...An,timelineMessageId:Tn}:An);Zr(an),En(an)}if(ce?.outcome==="rejected_busy"){let an=Ht=>Ht.map(An=>An.id===Pa?{...An,isOptimistic:!1,status:"error"}:An);if(Zr(an),En(an),ce?.notice){let Ht=(Ci=Kt)=>{let mR={id:`system-rejected-${n.current++}`,role:"system",content:ce.notice,timestamp:new Date().toISOString(),isOptimistic:!1},Gh=fR=>[...fR,mR];Ci&&k(Gh),(!Ci||V!==e)&&E(V,Gh)};if(!t.current||t.current===V){let Ci=P2(V,K.current);Ci?se({gateKey:Ci,content:ce.notice}):Ht()}else Ht(!1)}gr(()=>P(!1)),St.current=!1}else ce?.run_id||(St.current=!1);return ce}catch(ce){ce.status===429&&s(Date.now()+N5(ce));let Tn=an=>an.map(Ht=>Ht.id===Pa?{...Ht,isOptimistic:!1,status:"error",error:ce.message}:Ht);throw Zr(Tn),En(Tn),gr(()=>P(!1)),St.current=!1,ce}finally{D2(a.current,fe,Pa)}},[e,U,k,E,P,ie,m]),It=p.default.useCallback(async(W,re={})=>{if(!U)return;let{runId:$e,gateRef:we}=U;if(!$e||!we)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");let N=await Lp({threadId:e,runId:$e,gateRef:we,resolution:W,always:re.always,credentialRef:re.credentialRef}),_=b5(N);if(ft.current.set(`${$e}
${we}`,{resolution:W,outcome:_}),S5(W)&&_==="resumed"&&S2(k,U,gt),ie(null),_==="resumed"){P(!0),m({runId:N?.run_id||$e,threadId:N?.thread_id||e,status:N?.status||"queued"});return}P(!1),m(null)},[U,e,k,m]),ot=p.default.useCallback(async W=>{if(!U)throw new Error("auth gate is no longer pending");let{runId:re,gateRef:$e,provider:we}=U;if(!re||!$e||!we)throw new Error("auth gate is missing required credential metadata");let N=U.accountLabel||`${we} credential`,_=`${re}
${$e}`;if(je.current.gateKey!==_&&(je.current={gateKey:_,credentialRef:null,inFlight:!1}),je.current.inFlight)throw new Error("auth token submission already in progress");je.current.inFlight=!0;try{let D=je.current.credentialRef,z=null;if(!D){if(z=await L2(F=>Jx({provider:we,accountLabel:N,token:W,threadId:e,runId:re,gateRef:$e,signal:F})),D=z?.credential_ref,!D)throw new Error("manual token submit returned no credential_ref");je.current.credentialRef=D}if(!y5(z))try{await L2(F=>Lp({threadId:e,runId:re,gateRef:$e,resolution:"credential_provided",credentialRef:D,signal:F}))}catch(F){throw h5(F)}je.current={gateKey:null,credentialRef:null,inFlight:!1},ie(null),P(!0)}catch(D){throw je.current.gateKey===_&&(je.current.inFlight=!1),D}},[U,e]),oa=p.default.useCallback(async W=>{let re=u?.runId;!re||!e||(ie(null),P(!1),m(null),St.current=!1,await Yx({threadId:e,runId:re,reason:W}))},[u,e]),wa=p.default.useCallback(()=>{g&&v&&S(v)},[g,v,S]),La=p.default.useCallback(async(W,re,$e)=>{let we="approved",N=!1;re==="deny"?we="denied":re==="cancel"?we="cancelled":re==="always"&&(we="approved",N=!0),await It(we,{always:N})},[It]),la=p.default.useCallback(()=>{},[]),Sa=p.default.useCallback(async W=>{if(!W||W.status!=="error")return;let re=typeof W.retryContent=="string"?W.retryContent:typeof W.content=="string"?W.content:"",$e=Array.isArray(W.retryAttachments)?W.retryAttachments:[];if(!re&&$e.length===0)return;let we=N=>N.filter(_=>_.id!==W.id);k(we),e&&E(e,we);try{await Oa(re,{threadId:e,attachments:$e,displayContent:typeof W.retryDisplayContent=="string"?W.retryDisplayContent:W.content})}catch{}},[Oa,E,k,e]);return{messages:$,isProcessing:C,pendingGate:U,busyGateNotice:te,channelConnectAction:f,activeRun:u,sseStatus:Ma,historyLoading:b,historyLoadError:w,hasMore:g,cooldownSeconds:Cn,send:Oa,resolveGate:It,submitAuthToken:ot,cancelRun:oa,loadMore:wa,dismissChannelConnectAction:()=>h(null),suggestions:[],setSuggestions:la,retryMessage:Sa,approve:La,recoverHistory:la,recoveryNotice:null}}function S5(e){return e==="denied"||e==="cancelled"}function N5(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function F2({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}function _5(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function Qc({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let u=o.toString(),c=`/logs${u?`?${u}`:""}`;return i?`/v2${c}`:c}function B2(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(_5),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}var k5=1500;function z2({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let o=R(),{messages:u,isProcessing:c,pendingGate:d,busyGateNotice:m,channelConnectAction:f,suggestions:h,sseStatus:x,historyLoading:y,historyLoadError:$,hasMore:g,cooldownSeconds:v,recoveryNotice:b,activeRun:w,send:S,cancelRun:E,retryMessage:k,approve:C,recoverHistory:O,loadMore:L,setSuggestions:P,submitAuthToken:U,dismissChannelConnectAction:T}=j2(t),K=p.default.useMemo(()=>e.find(ot=>ot.id===t)||null,[e,t]),te=p.default.useMemo(()=>F2({gatewayStatus:i,activeThread:K}),[i,K]),se=u.length>0||c||!!d||!!f,ie=!y&&!se&&!$,Ye=d?"Resolve the approval request before sending another message.":"",Ke=!!d||c&&!d||v>0,gt=p.default.useRef(Ke);gt.current=Ke;let ft=Ye||(v>0?`Retry in ${v}s`:void 0),je=t||Wo,St=!!(t&&w?.runId&&w.threadId===t&&c&&!d),Cn=t&&w?.runId&&w.threadId===t?Qc({threadId:t,runId:w.runId},{absolute:!0}):null,qt=p.default.useCallback(async(ot,{images:oa=[],attachments:wa=[],displayContent:La}={})=>{if(d)throw new Error(Ye);if(gt.current)return null;let la=await S(ot,{images:oa,attachments:wa,displayContent:La,threadId:t}),Sa=la?.thread_id||t;return!t&&Sa&&a&&a(Sa,{replace:!0}),la},[t,Ye,Ke,a,d,S]),Da=p.default.useCallback(async ot=>{Ke||(P([]),await qt(ot))},[Ke,qt,P]),Ma=p.default.useCallback(()=>E("user_requested"),[E]);p.default.useEffect(()=>{if(!t)return;if(d){_c(t,$a.NEEDS_ATTENTION);return}if(c){_c(t,$a.RUNNING);return}let ot=setTimeout(()=>Dw(t),k5);return()=>clearTimeout(ot)},[t,d,c]);let[Oa,It]=p.default.useState(!1);return p.default.useEffect(()=>{let ot=oa=>{if(oa.key==="Escape"){It(!1);return}if(oa.key!=="?")return;let wa=oa.target,La=wa?.tagName;La==="INPUT"||La==="TEXTAREA"||wa?.isContentEditable||(oa.preventDefault(),It(la=>!la))};return window.addEventListener("keydown",ot),()=>window.removeEventListener("keydown",ot)},[]),l`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${L1} status=${x} />

        ${c&&!d&&Cn&&l`
          <div className="flex justify-end border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-1.5">
            <${Sn}
              to=${Cn}
              className="inline-flex h-8 items-center gap-1.5 rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
              title=${o("nav.logs")}
            >
              <${M} name="list" className="h-3.5 w-3.5" />
              ${o("nav.logs")}
            <//>
          </div>
        `}

        ${$&&l`
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            ${$}
          </div>
        `}

        ${ie&&l`
          <${P1}
            onSuggestion=${Da}
            onSend=${qt}
            disabled=${!1}
            sendDisabled=${Ke}
            initialText=${r}
            resetKey=${s}
            draftKey=${je}
            context=${te}
            statusText=${ft}
            canCancel=${St}
            onCancel=${Ma}
          />
        `}
        ${!ie&&l`
          <${d2}
            messages=${u}
            isLoading=${y}
            hasMore=${g}
            onLoadMore=${L}
            onRetryMessage=${k}
            threadId=${t}
            pending=${c}
          >
            ${b&&l`
              <${m2}
                notice=${b}
                onRecover=${O}
              />
            `}
            ${c&&!d&&l`<${p2} />`}
            ${f&&l`
              <${D1}
                connectAction=${f}
                onDismiss=${T}
              />
            `}
            ${d&&(d.kind==="auth_required"?d.challengeKind==="oauth_url"?l`
                  <${E1}
                    gate=${d}
                    onCancel=${()=>C(d.requestId,"cancel",d.kind)}
                  />
                `:d.challengeKind==="manual_token"?l`
                  <${T1}
                    gate=${d}
                    onSubmit=${U}
                    onCancel=${()=>C(d.requestId,"cancel",d.kind)}
                  />
                `:l`
                  <${C1}
                    gate=${d}
                    onCancel=${()=>C(d.requestId,"cancel",d.kind)}
                  />
                `:l`
              <${R1}
                gate=${d}
                onApprove=${()=>C(d.requestId,"approve",d.kind)}
                onDeny=${()=>C(d.requestId,"deny",d.kind)}
                onAlways=${()=>C(d.requestId,"always",d.kind)}
              />
            `)}
            ${m&&l`
              <div
                data-testid="busy-gate-notice"
                role="status"
                className="mx-auto mt-3 max-w-lg rounded-lg border border-copper/25 bg-copper/10 px-4 py-3 text-center text-sm leading-6 text-copper"
              >
                ${m.content}
              </div>
            `}
          <//>

          <${f2}
            suggestions=${h}
            onSelect=${Da}
            disabled=${Ke}
          />

          <${Uc}
            onSend=${qt}
            disabled=${!1}
            sendDisabled=${Ke}
            initialText=${r}
            resetKey=${s}
            draftKey=${je}
            context=${te}
            statusText=${ft}
            canCancel=${St}
            onCancel=${Ma}
          />
        `}
      </div>
      <${U1}
        open=${Oa}
        onClose=${()=>It(!1)}
      />
    </div>
  `}function $h(){let{threadsState:e,gatewayStatus:t}=ba(),{threadId:a}=st(),n=pe(),r=Pe(),s=r.state?.composerDraft||"";p.default.useEffect(()=>{a&&a!==e.activeThreadId?e.setActiveThreadId(a):a||e.setActiveThreadId(null)},[a]);let i=p.default.useCallback((o,u={})=>{if(!o){e.setActiveThreadId(null),n("/chat",u);return}e.setActiveThreadId(o),n(`/chat/${o}`,u)},[e,n]);return l`
    <${z2}
      threads=${e.threads}
      activeThreadId=${e.activeThreadId}
      onSelectThread=${i}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function q2(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?ni(e,t):"",model:e?Sc(e,t):""}}function I2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u}){let[c,d]=p.default.useState(()=>q2(e,a)),[m,f]=p.default.useState(""),[h,x]=p.default.useState([]),[y,$]=p.default.useState(null),[g,v]=p.default.useState(""),b=p.default.useRef(!!e);p.default.useEffect(()=>{n&&(d(q2(e,a)),f(""),x([]),$(null),v(""),b.current=!!e)},[n,e,a]);let w=e?.builtin===!0,S=e&&!e.builtin,E=p.default.useCallback((P,U)=>{d(T=>{let K={...T,[P]:U};return P==="name"&&!b.current&&(K.id=lw(U)),K})},[]),k=p.default.useCallback(()=>!w&&(!c.name.trim()||!c.id.trim())?u("llm.fieldsRequired"):!w&&!uw(c.id.trim())?u("llm.invalidId"):!S&&!w&&t.includes(c.id.trim())?u("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,w,S,u]),C=p.default.useCallback(async()=>{let P=k();if(P){$({tone:"error",text:P});return}v("save");try{await s({form:c,apiKey:m,provider:e}),r()}catch(U){$({tone:"error",text:U.message})}finally{v("")}},[m,c,r,s,e,k]),O=p.default.useCallback(async()=>{if(!c.model.trim()){$({tone:"error",text:u("llm.modelRequired")});return}v("test");try{let P=await i(Yp(e,c,m,a));$({tone:P.ok?"success":"error",text:P.message})}catch(P){$({tone:"error",text:P.message})}finally{v("")}},[m,a,c,i,e,u]),L=p.default.useCallback(async()=>{if((w?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){$({tone:"error",text:u("llm.baseUrlRequired")});return}v("models");try{let U=await o(Yp(e,c,m,a));if(!U.ok||!Array.isArray(U.models)||!U.models.length)$({tone:"error",text:U.message||u("llm.modelsFetchFailed")});else{x(U.models);let T=cw(c.model,U.models);T!==null&&E("model",T),$({tone:"success",text:u("llm.modelsFetched",{count:U.models.length})})}}catch(U){$({tone:"error",text:U.message})}finally{v("")}},[m,a,c,w,o,e,u,E]);return{form:c,apiKey:m,models:h,message:y,busy:g,isBuiltin:w,isEditing:S,setApiKey:f,update:E,submit:C,runTest:O,fetchModels:L,markIdEdited:()=>{b.current=!0}}}function Vc({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let u=R(),c=I2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u});if(!n)return null;let{form:d,apiKey:m,models:f,message:h,busy:x,isBuiltin:y,isEditing:$}=c,g=y?u("llm.configureProvider",{name:e.name||e.id}):u($?"llm.editProvider":"llm.newProvider");return l`
    <${di} open=${n} onClose=${r} title=${g} size="lg">
      <${mi} className="space-y-4">
        ${!y&&l`
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${u("llm.providerName")}
              <${Tt} value=${d.name} onChange=${v=>c.update("name",v.target.value)} />
            </label>
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${u("llm.providerId")}
              <${Tt}
                value=${d.id}
                disabled=${$}
                onChange=${v=>{c.markIdEdited(),c.update("id",v.target.value)}}
              />
            </label>
          </div>
          <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
            ${u("llm.adapter")}
            <${lh} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${Gp.map(v=>l`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
            <//>
          </label>
        `}

        ${y&&l`
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            ${al(e.adapter)}
          </div>
        `}

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.baseUrl")}
          <${Tt} value=${d.baseUrl} placeholder=${e?.base_url||""} onChange=${v=>c.update("baseUrl",v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.apiKey")}
          <${Tt} type="password" value=${m} placeholder=${u("llm.apiKeyPlaceholder")} onChange=${v=>c.setApiKey(v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.defaultModel")}
          <div className="flex items-stretch gap-2">
            <${Tt} value=${d.model} onChange=${v=>c.update("model",v.target.value)} />
            <${A} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${x!==""} onClick=${c.fetchModels}>
              ${u(x==="models"?"llm.fetchingModels":"llm.fetchModels")}
            <//>
          </div>
        </label>

        ${f.length>0&&l`
          <${lh} value=${d.model} onChange=${v=>c.update("model",v.target.value)}>
            ${f.map(v=>l`<option key=${v} value=${v}>${v}</option>`)}
          <//>
        `}

        ${h&&l`
          <div className=${h.tone==="error"?"text-sm text-red-200":"text-sm text-mint"} role="status">
            ${h.text}
          </div>
        `}
      <//>
      <${fi}>
        <${A} type="button" variant="secondary" disabled=${x!==""} onClick=${c.runTest}>
          ${u(x==="test"?"llm.testing":"llm.testConnection")}
        <//>
        <${A} type="button" variant="ghost" disabled=${x!==""} onClick=${r}>${u("common.cancel")}<//>
        <${A} type="button" disabled=${x!==""} onClick=${c.submit}>
          ${u(x==="save"?"common.saving":"common.save")}
        <//>
      <//>
    <//>
  `}function Gc({login:e}){let t=R(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return l`
    ${a&&l`<div className="text-center text-xs text-[var(--v2-text-muted)]">
      ${t("onboarding.nearaiWaiting")}
    </div>`}
    ${n&&l`<div className="text-center text-xs text-red-300">${n}</div>`}

    ${i&&l`<div
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
    ${r&&l`<div className="text-center text-xs text-[var(--v2-text-muted)]">
      ${t("onboarding.codexWaiting")}
    </div>`}
    ${s&&l`<div className="text-center text-xs text-red-300">${s}</div>`}
  `}function R5(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function Yc({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=ri({settings:e,gatewayStatus:t}),[s,i]=p.default.useState(null),[o,u]=p.default.useState(!1),[c,d]=p.default.useState(null),m=p.default.useRef(null),f=p.default.useCallback((g,v)=>{m.current&&window.clearTimeout(m.current),d({tone:g,text:v}),m.current=window.setTimeout(()=>d(null),3500)},[]);p.default.useEffect(()=>()=>{m.current&&window.clearTimeout(m.current)},[]);let h=p.default.useCallback((g=null)=>{i(g),u(!0)},[]),x=p.default.useCallback(async g=>{try{await r.setActiveProvider(g),f("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(h(g),f("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):f("error",v.message)}},[h,r,f,n]),y=p.default.useCallback(async({form:g,apiKey:v,provider:b})=>{if(b?.builtin){await r.saveBuiltinProvider({provider:b,form:g,apiKey:v}),f("success",n("llm.providerConfigured",{name:b.name||b.id}));return}let w=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:b});f("success",n(b?"llm.providerUpdated":"llm.providerAdded",{name:w.name||w.id}))},[r,f,n]),$=p.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),f("success",n("llm.providerDeleted"))}catch(v){f("error",v.message)}},[r,f,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>R5(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:h,closeDialog:()=>u(!1),handleUse:x,handleSave:y,handleDelete:$}}var C5=3e5;function E5(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function T5(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function A5(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=u=>{let c=u.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},C5);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var D5=3e5,M5=9e5,O5=2e3;async function K2(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,O5)),(await wc().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function Jc({onSuccess:e}={}){let t=R(),a=X(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),[o,u]=p.default.useState(!1),[c,d]=p.default.useState(""),[m,f]=p.default.useState(null),h=p.default.useCallback(()=>{i(""),d(""),f(null)},[]),x=p.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),y=p.default.useCallback(async v=>{if(h(),E5()){i(t("onboarding.nearaiLocalSso"));return}let b=window.open("about:blank","_blank");if(!b){i(t("onboarding.nearaiFailed"));return}try{b.opener=null}catch{}r(!0);try{let{auth_url:w}=await B$({provider:v,origin:window.location.origin});b.location.href=w;let S=await K2("nearai",D5,b);if(S==="active"){await x();return}b.close(),i(t(S==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{b.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,h,t]),$=p.default.useCallback(async()=>{h(),r(!0);try{let v=T5(),b=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!b){i(t("onboarding.nearaiFailed"));return}b.opener=null;let w=await A5(b,v);if(!w){i(t("onboarding.nearaiFailed"));return}await z$({account_id:w.accountId,public_key:w.publicKey,signature:w.signature,message:w.message,recipient:w.recipient,nonce:w.nonce}),await x()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,h,t]),g=p.default.useCallback(async()=>{h();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}u(!0);try{let{user_code:b,verification_uri:w}=await q$();f({userCode:b,verificationUri:w}),v&&(v.location.href=w);let S=await K2("openai_codex",M5,v);if(S==="active"){await x();return}v&&v.close(),d(t(S==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{u(!1)}},[x,h,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:m,startNearai:y,startNearaiWallet:$,startCodex:g}}var H2="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",L5="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",P5="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",U5="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",j5={nearai:{color:"#00ec97",path:L5},openai_codex:{color:"#10a37f",path:H2},openai:{color:"#10a37f",path:H2},anthropic:{color:"#d97757",path:P5},ollama:{color:null,path:U5}};function Q2({id:e,name:t}){let a=j5[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return l`
      <span
        className=${`${n} bg-[var(--v2-surface-muted)] text-sm font-semibold text-[var(--v2-text-strong)]`}
      >
        ${s}
      </span>
    `}let r=a.color?{background:`color-mix(in srgb, ${a.color} 16%, transparent)`,color:a.color}:{background:"var(--v2-surface-muted)",color:"var(--v2-text-strong)"};return l`
    <span className=${n} style=${r}>
      <svg viewBox="0 0 24 24" className="h-5 w-5" fill="currentColor" aria-hidden="true">
        <path d=${a.path} />
      </svg>
    </span>
  `}var F5=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function B5({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=p.default.useState(!1),o=p.default.useRef(null),u=t||a.nearaiBusy;p.default.useEffect(()=>{if(!s)return;let d=f=>{o.current&&!o.current.contains(f.target)&&i(!1)},m=f=>{f.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",m),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",m)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return l`
    <div ref=${o} className="relative shrink-0">
      <${A}
        type="button"
        variant="primary"
        size="sm"
        className="gap-1.5"
        aria-haspopup="true"
        aria-expanded=${s?"true":"false"}
        disabled=${u}
        onClick=${()=>i(d=>!d)}
      >
        ${n("onboarding.setUp")}
        <${M} name="chevron" className="h-3.5 w-3.5" />
      <//>
      ${s&&l`
        <div
          role="menu"
          className="absolute right-0 top-10 z-20 min-w-[176px] rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-1 shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]"
        >
          ${c.map(d=>l`
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
  `}function z5({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let u=s(e.nameKey),c;return e.auth==="nearai"?c=l`<${B5} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=l`
      <${A} type="button" variant="secondary" size="sm" disabled=${r.codexBusy} onClick=${r.startCodex}>
        ${s("onboarding.signIn")}
      <//>
    `:a?c=l`<${A} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>i(t)}>
      ${s("llm.use")}
    <//>`:c=l`<${A} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>o(t)}>
      ${s("onboarding.setUp")}
    <//>`,l`
    <${ae} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-4">
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <${Q2} id=${e.id} name=${u} />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">${u}</span>
            ${a&&l`<${q} tone="positive" label=${s("onboarding.ready")} size="sm" />`}
          </div>
          <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">${s(e.descKey)}</div>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap gap-2 sm:justify-end">${c}</div>
    <//>
  `}function V2(){let{isAdmin:e=!1,isChecking:t=!1}=ba();return t?null:e?l`<${q5} />`:l`<${it} to="/chat" replace />`}function q5(){let e=R(),t=pe(),a=X(),{gatewayStatus:n}=ba(),r=Yc({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=F5.map(m=>({entry:m,provider:s.providers.find(f=>f.id===m.id)})).filter(m=>m.provider),o=p.default.useCallback(()=>t("/chat"),[t]),u=Jc({onSuccess:o}),c=p.default.useCallback(async m=>{let f=m.active_model||m.default_model||"";await tl({provider_id:m.id,model:f}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=p.default.useCallback(async({form:m,apiKey:f,provider:h})=>{await r.handleSave({form:m,apiKey:f,provider:h});let x=h?.id||m.id.trim(),y=m.model?.trim()||h?.default_model||"";await tl({provider_id:x,model:y}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?l`
      <div className="grid h-full place-items-center text-sm text-[var(--v2-text-muted)]">
        ${e("common.loading")}
      </div>
    `:l`
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex min-h-full max-w-2xl flex-col justify-center gap-6 p-6">
        <div className="text-center">
          <h1 className="text-2xl font-semibold text-[var(--v2-text-strong)]">
            ${e("onboarding.title")}
          </h1>
          <p className="mt-2 text-sm text-[var(--v2-text-muted)]">${e("onboarding.subtitle")}</p>
        </div>

        <div className="flex flex-col gap-3">
          ${i.map(({entry:m,provider:f})=>l`
              <${z5}
                key=${m.id}
                entry=${m}
                provider=${f}
                configured=${Hr(f,s.builtinOverrides)}
                isBusy=${s.isBusy}
                login=${u}
                t=${e}
                onUse=${c}
                onSetUp=${r.openDialog}
              />
            `)}
        </div>

        <${Gc} login=${u} />

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

      <${Vc}
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
  `}function I({children:e,className:t="",...a}){return l`<${ae} className=${t} ...${a}>${e}<//>`}function et({label:e,value:t,tone:a="muted",badgeLabel:n,detail:r,showDivider:s=!0,className:i="",valueClassName:o="text-[1.75rem] md:text-[2rem]"}){return l`
    <div
      className=${Y("px-1 py-4",s&&"border-t border-[var(--v2-panel-border)]",i)}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div
            className="font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]"
          >
            ${e}
          </div>
          <div
            className=${Y("mt-3 truncate font-medium tracking-[-0.05em] text-[var(--v2-text-strong)]",o)}
          >
            ${t}
          </div>
          ${r&&l`<div className="mt-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            ${r}
          </div>`}
        </div>
        <${q} tone=${a} label=${n??a} />
      </div>
    </div>
  `}function G2({items:e}){return l`
    <div className="grid gap-3">
      ${e.map((t,a)=>l`
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
  `}function xe({title:e,description:t,children:a,boxed:n=!0}){let r=l`
    <div className="max-w-xl">
      <h2
        className="text-[1.35rem] font-medium tracking-[-0.03em] text-[var(--v2-text-strong)] md:text-[1.6rem]"
      >
        ${e}
      </h2>
      <p className="mt-3 text-[15px] leading-relaxed text-[var(--v2-text-muted)]">
        ${t}
      </p>
      ${a&&l`<div className="mt-5">${a}</div>`}
    </div>
  `;return n?l`<${ae} padding="lg">${r}<//>`:l`<div className="py-8">${r}</div>`}var Y2={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function en({result:e,onDismiss:t}){return e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",Y2[e.type]||Y2.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}var J2="",I5={workspace:"home"};function Xc(e){return I5[e]||e}function ul(e){return[...e||[]].sort((t,a)=>t.is_dir!==a.is_dir?t.is_dir?-1:1:t.name.localeCompare(a.name,void 0,{sensitivity:"base"}))}function hi(e){return e?e.split("/").filter(Boolean):[]}function Zc(e){return e?`/workspace/${hi(e).map(encodeURIComponent).join("/")}`:"/workspace"}function wh(e){let t=hi(e);return t.pop(),t.join("/")}function X2(e){return/\.mdx?$/i.test(e||"")}function Wc({path:e,onNavigate:t}){let a=R(),n=hi(e),r="";return l`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button
        type="button"
        onClick=${()=>t("/workspace")}
        className="text-signal hover:underline"
      >
        ${a("workspace.breadcrumbRoot")}
      </button>
      ${n.map((s,i)=>{r=r?`${r}/${s}`:s;let o=r,u=i===0?Xc(s):s;return l`
          <span key=${o} className="text-iron-400">/</span>
          <button
            key=${`${o}-button`}
            type="button"
            onClick=${()=>t(Zc(o))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${u}
          </button>
        `})}
    </div>
  `}function K5(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function Z2({path:e,entries:t,isLoading:a,filter:n,onOpen:r,onNavigate:s}){let i=R();if(a)return l`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;let o=(t||[]).filter(f=>!K5(f.path)),u=String(n||"").trim().toLowerCase(),c=u?o.filter(f=>f.name.toLowerCase().includes(u)):o,d=ul(c),m;return o.length?d.length?m=l`
      <div className="divide-y divide-white/[0.06]">
        ${d.map(f=>l`
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
    `:m=l`<div className="px-4 py-10 text-center text-sm text-iron-300">${i("workspace.noMatches")}</div>`:m=l`<div className="px-4 py-10 text-center text-sm text-iron-300">${i("workspace.emptyDir")}</div>`,l`
    <${I} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${Wc} path=${e} onNavigate=${s} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">${m}</div>
    <//>
  `}var ed="/api/webchat/v2/fs",H5=1024*1024,Q5=8*1024*1024;function W2(e){let t=String(e||"").split("/").filter(Boolean);return{mount:t.shift()||"",path:t.join("/")}}function V5(e,t){return t?`${e}/${t}`:e}function G5(e){let t=String(e||"").toLowerCase();return t.startsWith("text/")||t==="application/json"||t==="application/javascript"||t==="application/xml"||t.endsWith("+json")||t.endsWith("+xml")}function Y5(e){return String(e||"").toLowerCase().startsWith("image/")}function J5(e){let t=String(e||"").toLowerCase();return t.startsWith("audio/")||t.startsWith("video/")||t.startsWith("font/")||t==="application/pdf"||t==="application/zip"||t==="application/gzip"}function X5(e){if(e.subarray(0,Math.min(e.length,8192)).indexOf(0)!==-1)return!0;try{return new TextDecoder("utf-8",{fatal:!0}).decode(e),!1}catch{return!0}}function Z5(e,t){let a=new URL(`${ed}/content`,window.location.origin);return a.searchParams.set("mount",e),a.searchParams.set("path",t),a.pathname+a.search}async function W5(){return(await Q(`${ed}/mounts`))?.mounts||[]}async function vi(e=""){if(!e)return{entries:(await W5()).map(o=>({name:Xc(o.mount),path:o.mount,is_dir:!0}))};let{mount:t,path:a}=W2(e),n=new URL(`${ed}/list`,window.location.origin);return n.searchParams.set("mount",t),a&&n.searchParams.set("path",a),{entries:((await Q(n.pathname+n.search))?.entries||[]).map(i=>({name:i.name,path:V5(t,i.path),is_dir:i.kind==="directory"}))}}async function eS(e){let{mount:t,path:a}=W2(e);if(!t||!a)return{kind:"directory",path:e};let n=new URL(`${ed}/stat`,window.location.origin);n.searchParams.set("mount",t),n.searchParams.set("path",a);let s=(await Q(n.pathname+n.search))?.stat||{},i=s.mime_type||"application/octet-stream",o=Number(s.size_bytes||0),u=Z5(t,a),c={path:e,mime:i,size_bytes:o,download_path:u};if(s.kind&&s.kind!=="file")return{...c,kind:"directory"};if(Y5(i)){if(o>Q5)return{...c,kind:"binary"};let h=await gc(u);return{...c,kind:"image",image_data_url:h}}if(J5(i)||o>H5)return{...c,kind:"binary"};let d=await Ca(u),m=new Uint8Array(await d.arrayBuffer());if(!G5(i)&&X5(m))return{...c,kind:"binary"};let f=new TextDecoder("utf-8").decode(m);return{...c,kind:"text",content:f}}function tS(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function eD(e,t,a){let n=String(t||"").trim().toLowerCase(),r=(e||[]).filter(s=>!tS(s.path)).filter(s=>!n||s.name.toLowerCase().includes(n)?!0:s.is_dir&&a.has(s.path));return ul(r)}function aS({entry:e,depth:t,selectedPath:a,expandedPaths:n,filter:r,onToggleDirectory:s,onSelectFile:i}){let o=R(),u=n.has(e.path),c=H({queryKey:["workspace-list",e.path],queryFn:()=>vi(e.path),enabled:e.is_dir&&u});if(e.is_dir){let d=eD(c.data?.entries,r,n);return l`
      <div>
        <button
          type="button"
          onClick=${()=>{i(e.path),s(e.path)}}
          className=${["flex min-h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm hover:bg-white/[0.05] hover:text-white",a===e.path?"bg-signal/10 text-signal":"text-iron-200"].join(" ")}
          style=${{paddingLeft:`${8+t*16}px`}}
          aria-expanded=${u}
        >
          <span className=${["w-3 text-[10px]",u?"rotate-90":""].join(" ")}>></span>
          <span className="min-w-0 truncate font-semibold">${e.name}</span>
        </button>
        ${u&&l`
          <div className="space-y-1">
            ${c.isLoading?l`<div className="px-4 py-2 text-xs text-iron-400">${o("workspace.loading")}</div>`:c.isError?l`<div className="px-4 py-2 text-xs text-red-300">${o("workspace.unableOpenDirectory")}</div>`:d.map(m=>l`
                  <${aS}
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
    `}return l`
    <button
      type="button"
      onClick=${()=>i(e.path)}
      className=${["flex min-h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm",a===e.path?"bg-signal/10 text-signal":"text-iron-300 hover:bg-white/[0.05] hover:text-white"].join(" ")}
      style=${{paddingLeft:`${24+t*16}px`}}
    >
      <span className="min-w-0 truncate">${e.name}</span>
    </button>
  `}function nS({entries:e,selectedPath:t,expandedPaths:a,filter:n,onToggleDirectory:r,onSelectFile:s,isLoading:i}){let o=R();if(i)return l`<div className="space-y-2 p-3">${[1,2,3,4].map(c=>l`<div key=${c} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let u=ul(e.filter(c=>!tS(c.path)));return u.length?l`
    <div className="space-y-1 p-2">
      ${u.map(c=>l`
        <${aS}
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
  `:l`<div className="px-4 py-8 text-sm text-iron-300">${o("workspace.noFiles")}</div>`}function rS({rootEntries:e,selectedPath:t,expandedPaths:a,filter:n,onFilterChange:r,isLoadingTree:s,onToggleDirectory:i,onSelectFile:o}){let u=R();return l`
    <${I} className="flex min-h-[420px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="border-b border-white/10 p-3">
        <input
          value=${n}
          onInput=${c=>r(c.target.value)}
          placeholder=${u("workspace.filterPlaceholder")}
          className="h-9 w-full rounded-md border border-white/10 bg-iron-950/80 px-3 text-sm text-white outline-none placeholder:text-iron-400 focus:border-signal/45"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <${nS}
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
  `}function sS(e){return hi(e).pop()||"download"}function tD({path:e,file:t}){let a=R();return t.kind==="image"?l`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${t.image_data_url}
          alt=${sS(e)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `:t.kind==="text"?l`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${X2(e)?l`<${ra} content=${t.content} className="max-w-4xl text-base leading-7" />`:l`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
      </div>
    `:l`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${a("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `}function iS({path:e,file:t,isLoading:a,onNavigate:n}){let r=R(),[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(t?.download_path){i(!0);try{let c=await Ca(t.download_path);ci(c,sS(e))}catch{}finally{i(!1)}}},[t,e]);if(a)return l`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;if(!t||t.kind==="directory")return l`
      <${xe}
        title=${r("workspace.pickFileTitle")}
        description=${r("workspace.pickFileDesc")}
      />
    `;let u=r("workspace.fileMeta",{mime:t.mime||"application/octet-stream",size:Number(t.size_bytes||0)});return l`
    <${I} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${Wc} path=${e} onNavigate=${n} />
        <div className="flex items-center gap-2">
          <${q} tone="muted" label=${u} />
          <${A}
            variant="secondary"
            size="sm"
            onClick=${o}
            disabled=${s}
          >${r("workspace.download")}<//>
        </div>
      </div>

      <${tD} path=${e} file=${t} />

      ${wh(e)&&l`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${r("workspace.parent",{path:wh(e)})}
        </div>
      `}
    <//>
  `}function oS(e){let t=R(),a=X(),[n,r]=p.default.useState(new Set),[s,i]=p.default.useState(""),[o,u]=p.default.useState(null),c=H({queryKey:["workspace-list",""],queryFn:()=>vi("")}),d=H({queryKey:["workspace-file",e],queryFn:()=>eS(e),enabled:!!e}),m=e===""||d.data?.kind==="directory",f=H({queryKey:["workspace-list",e],queryFn:()=>vi(e),enabled:m});p.default.useEffect(()=>{u(null)},[e]);let h=p.default.useCallback(y=>a.fetchQuery({queryKey:["workspace-list",y],queryFn:()=>vi(y)}),[a]),x=p.default.useCallback(async y=>{let $=new Set(n);if($.has(y)){$.delete(y),r($);return}$.add(y),r($);try{await h(y)}catch(g){u({type:"error",message:g.message||t("workspace.unableOpenDirectory")})}},[n,h,t]);return{rootEntries:c.data?.entries||[],file:d.data||null,selectionIsDirectory:m,currentEntries:f.data?.entries||[],expandedPaths:n,filter:s,setFilter:i,result:o,clearResult:()=>u(null),isLoadingTree:c.isLoading,isLoadingFile:d.isLoading,isLoadingListing:f.isLoading,isFetching:c.isFetching||d.isFetching||f.isFetching,error:c.error||d.error||f.error||null,loadDirectory:h,toggleDirectory:x,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function Sh(){let e=R(),t=pe(),n=st()["*"]||J2,r=oS(n),s=p.default.useCallback(i=>{t(Zc(i))},[t]);return l`
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
            <${A}
              variant="secondary"
              size="sm"
              onClick=${r.refresh}
              disabled=${r.isFetching}
            >
              ${r.isFetching?e("workspace.refreshing"):e("workspace.refresh")}
            <//>
          </div>

          ${r.error&&l`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${r.error.message}
            </div>
          `}
          <${en}
            result=${r.result}
            onDismiss=${r.clearResult}
          />

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <${rS}
              rootEntries=${r.rootEntries}
              selectedPath=${n}
              expandedPaths=${r.expandedPaths}
              filter=${r.filter}
              onFilterChange=${r.setFilter}
              isLoadingTree=${r.isLoadingTree}
              onToggleDirectory=${r.toggleDirectory}
              onSelectFile=${s}
            />
            ${r.selectionIsDirectory?l`
                  <${Z2}
                    path=${n}
                    entries=${r.currentEntries}
                    isLoading=${r.isLoadingListing}
                    filter=${r.filter}
                    onOpen=${s}
                    onNavigate=${t}
                  />
                `:l`
                  <${iS}
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
  `}function lS(e){if(!e)return null;let t=e.metadata&&typeof e.metadata=="object"&&!Array.isArray(e.metadata)?e.metadata:{};return{id:e.project_id,name:e.name,description:e.description,goals:Array.isArray(t.goals)?t.goals:[],icon:e.icon||null,color:e.color||null,state:e.state,role:e.role,metadata:t,created_at:e.created_at,updated_at:e.updated_at,health:e.state==="archived"?"muted":"green"}}async function uS(){let t=((await Fx({limit:200}))?.projects||[]).map(lS);return{attention:[],projects:t}}async function cS(e){if(!e)return null;let t=await Bx({projectId:e});return lS(t?.project)}function dS(e){return Promise.resolve({missions:[],todo:!0})}function mS(e){return Promise.resolve({threads:[],todo:!0})}function fS(e){return Promise.resolve({widgets:[],todo:!0})}function pS(e){return Promise.resolve(null)}function hS(e){return Promise.resolve(null)}function vS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function gS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function yS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function bS(){let e=X(),t=H({queryKey:["projects-overview"],queryFn:uS,refetchInterval:5e3}),a=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function xS(e){let t=X(),a=!!e,n=H({queryKey:["project-detail",e],queryFn:()=>cS(e),enabled:a,refetchInterval:a?7e3:!1}),r=H({queryKey:["project-missions",e],queryFn:()=>dS(e),enabled:a,refetchInterval:a?5e3:!1}),s=H({queryKey:["project-threads",e],queryFn:()=>mS(e),enabled:a,refetchInterval:a?4e3:!1}),i=H({queryKey:["project-widgets",e],queryFn:()=>fS(e),enabled:a,refetchInterval:a?15e3:!1}),o=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function $S({projectId:e,missionId:t,threadId:a}){let n=X(),[r,s]=p.default.useState(null),i=H({queryKey:["project-mission-detail",t],queryFn:()=>pS(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=H({queryKey:["project-thread-detail",a],queryFn:()=>hS(a),enabled:!!a,refetchInterval:a?4e3:!1}),u=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=G({mutationFn:({targetMissionId:f})=>vS(f),onSuccess:f=>{s({type:"success",message:f?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),u()},onError:f=>{s({type:"error",message:f.message||"Unable to fire mission"})}}),d=G({mutationFn:({targetMissionId:f})=>gS(f),onSuccess:()=>{s({type:"success",message:"Mission paused."}),u()},onError:f=>{s({type:"error",message:f.message||"Unable to pause mission"})}}),m=G({mutationFn:({targetMissionId:f})=>yS(f),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),u()},onError:f=>{s({type:"error",message:f.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending}}function td(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function ad(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function wS(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function SS(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function aD(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function NS(e){let t=aD(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function _S(e){let t=e?.projects||[],a=t.reduce((o,u)=>o+Number(u.cost_today_usd||0),0),n=t.reduce((o,u)=>o+Number(u.active_missions||0),0),r=t.reduce((o,u)=>o+Number(u.threads_today||0),0),s=t.reduce((o,u)=>o+Number(u.pending_gates||0),0),i=t.reduce((o,u)=>o+Number(u.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function cl(e,t){return`${e} ${t}${e===1?"":"s"}`}var nD={projects:"muted",attention:"warning",spend:"success"};function kS({overview:e}){let t=_S(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:ad(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return l`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${q} tone=${nD[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function rD(e){return e?.type==="failure"?"danger":"warning"}function sD(e){return e?.type==="failure"?"failure":"gate"}function RS({items:e,onOpenItem:t}){return e?.length?l`
    <${I} className="overflow-hidden border-amber-300/10 p-0">
      <div className="border-b border-amber-300/10 px-5 py-4 sm:px-6">
        <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-copper">Needs attention</div>
        <p className="mt-2 max-w-[70ch] text-sm leading-6 text-iron-200">
          Operator-visible gates and recent failures across your project workspace.
        </p>
      </div>
      <div className="grid gap-3 p-4 sm:p-5 xl:grid-cols-2">
        ${e.map(a=>l`
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
              <${q} tone=${rD(a)} label=${sD(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function iD({project:e,onOpen:t,t:a}){return l`
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
        <${q} tone=${wS(e.health)} label=${e.health||"unknown"} />
      </div>

      ${e.goals?.length?l`
            <div className="mt-4 flex flex-wrap gap-2">
              ${e.goals.slice(0,3).map((n,r)=>l`
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
            ${a("projects.card.threadsToday",{count:cl(e.threads_today||0,"thread")})}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${a("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">${cl(e.pending_gates||0,"gate")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${a("projects.card.failures24h",{count:cl(e.failures_24h||0,"failure")})}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>${a("projects.card.spendToday",{value:ad(e.cost_today_usd||0)})}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${td(e.last_activity)}</div>
        </div>
        <${A}
          data-testid="project-open-workspace"
          variant="secondary"
          onClick=${n=>{n.stopPropagation(),t(e.id)}}
        >${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function oD({project:e,onOpen:t,t:a}){return l`
    <${I}
      data-testid="project-card"
      data-project-id=${e.id}
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
            ${cl(e.threads_today||0,"thread")} today
          </div>
          <${A}
            data-testid="project-open-workspace"
            variant="secondary"
            onClick=${n=>{n.stopPropagation(),t(e.id)}}
          >${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function CS({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=R(),u=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return t?l`
    <div data-testid="projects-grid" className="space-y-5">
      ${u&&l`<${oD} project=${u} onOpen=${r} t=${o} />`}

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

      ${c.length?l`<div className="grid gap-4 xl:grid-cols-2 2xl:grid-cols-3">
            ${c.map(d=>l`<${iD} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
          </div>`:e.length?l`
            <${xe}
              title=${o("projects.scoped.onlyGeneralTitle")}
              description=${o("projects.scoped.onlyGeneralDesc")}
            >
              <${A} onClick=${s}>${o(i?"projects.preparingChat":"projects.startProject")}<//>
            <//>
          `:l`
              <${xe}
                title=${o("projects.empty.noMatchTitle")}
                description=${o("projects.empty.noMatchDesc")}
              />
            `}
    </div>
  `:l`
      <${xe}
        title=${o("projects.empty.noneTitle")}
        description=${o("projects.empty.noneDesc")}
      >
        <${A} onClick=${s}>${o("projects.createFromChat")}<//>
      <//>
    `}function ES({threads:e,selectedThreadId:t,onSelectThread:a,onNewConversation:n,isStartingConversation:r}){let s=[...e].sort((i,o)=>new Date(o.updated_at||o.created_at)-new Date(i.updated_at||i.created_at));return l`
    <${I} className="p-4 sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Conversations</div>
          <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">Project conversations</h2>
        </div>
        ${n&&l`
          <${A} onClick=${n} disabled=${r}>
            ${r?"Starting\u2026":"New conversation"}
          <//>
        `}
      </div>

      <div className="mt-5 space-y-3">
        ${s.length?s.slice(0,18).map(i=>{let o=NS(i);return l`
                <button
                  key=${i.id}
                  onClick=${()=>a(i.id)}
                  className=${["w-full rounded-[20px] border p-4 text-left",t===i.id?"border-signal/35 bg-signal/10":"border-white/10 bg-white/[0.025] hover:border-signal/25 hover:bg-white/[0.045]"].join(" ")}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="truncate text-base font-semibold text-white">${o.title}</div>
                      <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-400">${o.subtitle}</div>
                      ${o.brief?l`<p className="mt-3 line-clamp-2 text-sm leading-6 text-iron-300">${o.brief}</p>`:null}
                    </div>
                    <${q} tone=${SS(i.state)} label=${i.state} />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                    <span>${i.step_count||0} steps</span>
                    <span>${i.total_tokens||0} tokens</span>
                    <span>${td(i.updated_at||i.created_at)}</span>
                  </div>
                </button>
              `}):l`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                No project threads yet. When an automation runs or scoped chat work happens inside this project, activity will appear here.
              </div>
            `}
      </div>
    <//>
  `}var lD="/workspace";function uD(e){let t=a=>a.kind==="directory"?0:1;return[...e].sort((a,n)=>t(a)-t(n)||a.name.localeCompare(n.name,void 0,{sensitivity:"base"}))}function cD(e){return e?String(e).replace(/^\/workspace\/?/,"").split("/").filter(Boolean):[]}function TS({threadId:e}){let t=R(),[a,n]=p.default.useState(void 0),[r,s]=p.default.useState(null),i=H({queryKey:["project-files",e||"",a||""],queryFn:()=>Dx({threadId:e,path:a}),enabled:!!e}),o=p.default.useMemo(()=>uD(i.data?.entries||[]),[i.data]),u=p.default.useCallback(async m=>{if(m.kind==="directory"){s(null),n(m.path);return}try{s(null);let f=await Ca(vc({threadId:e,path:m.path})),h=URL.createObjectURL(f),x=document.createElement("a");x.href=h,x.download=m.name,document.body.appendChild(x),x.click(),x.remove(),URL.revokeObjectURL(h)}catch(f){s(f?.message||"Unable to download file")}},[e]),c=cD(a),d=l`
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
          ${"Files"}
        </div>
        <${q} tone="muted" label=${t("workspace.readOnly")} />
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
  `;return e?l`
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
        ${c.map((m,f)=>{let h=`${lD}/${c.slice(0,f+1).join("/")}`;return l`
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

      ${r&&l`
        <div className="mt-3 rounded-xl border border-red-400/30 bg-red-500/10 px-3 py-2 text-xs text-red-200">
          ${r}
        </div>
      `}
      ${i.error&&l`
        <div className="mt-3 rounded-xl border border-red-400/30 bg-red-500/10 px-3 py-2 text-xs text-red-200">
          ${i.error.message}
        </div>
      `}

      <div className="mt-3 space-y-1">
        ${i.isLoading?[1,2,3,4].map(m=>l`<div key=${m} className="v2-skeleton h-9 rounded-[12px]" />`):o.length?o.map(m=>l`
                <button
                  key=${m.path}
                  type="button"
                  onClick=${()=>u(m)}
                  className="flex w-full items-center gap-3 rounded-[12px] border border-transparent px-3 py-2 text-left hover:border-white/10 hover:bg-white/[0.04]"
                >
                  <${M}
                    name=${m.kind==="directory"?"folder":"file"}
                    className="h-4 w-4 shrink-0 text-iron-300"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm text-white">${m.name}</span>
                  ${m.kind==="directory"?l`<${M} name="chevron" className="h-3.5 w-3.5 shrink-0 -rotate-90 text-iron-500" />`:l`<${M} name="download" className="h-3.5 w-3.5 shrink-0 text-iron-500" />`}
                </button>
              `):l`
              <div className="rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                ${"This folder is empty."}
              </div>
            `}
      </div>
    <//>
  `:l`
      <${I} className="p-4 sm:p-5">
        ${d}
        <div className="mt-4 rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
          ${"No files yet \u2014 they appear once a thread has run in this project."}
        </div>
      <//>
    `}function dD(e){return[...e||[]].sort((a,n)=>new Date(n.updated_at||n.created_at)-new Date(a.updated_at||a.created_at))[0]?.id||null}function AS({project:e,threads:t,selectedThreadId:a,onSelectThread:n,onNewConversation:r,isStartingConversation:s}){let i=dD(t);return l`
    <div
      data-testid="project-workspace"
      data-project-id=${e.id}
      className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]"
    >
      <div className="space-y-5">
        <div className="min-w-0">
          <h2 data-testid="project-workspace-title" className="text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
          ${e.description?l`<p className="mt-1 text-sm leading-6 text-iron-300">${e.description}</p>`:null}
        </div>

        <${ES}
          threads=${t}
          selectedThreadId=${a}
          onSelectThread=${n}
          onNewConversation=${r}
          isStartingConversation=${s}
        />
      </div>

      <${TS} threadId=${i} />
    </div>
  `}function dl(){let e=R(),t=pe(),{threadsState:a}=ba(),{projectId:n=null,threadId:r=null}=st(),[s,i]=p.default.useState(""),[o,u]=p.default.useState(null),c=bS(),d=xS(n),m=$S({projectId:n,threadId:r}),f=p.default.useMemo(()=>{let k=s.trim().toLowerCase();return k?c.overview.projects.filter(C=>[C.name,C.description,...C.goals||[]].some(O=>String(O||"").toLowerCase().includes(k))):c.overview.projects},[c.overview.projects,s]),h=p.default.useMemo(()=>c.overview.projects.find(k=>k.id===n)||null,[c.overview.projects,n]),x=p.default.useCallback(()=>{c.invalidate(),d.invalidate()},[c,d]),y=p.default.useCallback(k=>{t(`/projects/${k}`)},[t]),$=p.default.useCallback(k=>{if(k.thread_id){t(`/projects/${k.project_id}/threads/${k.thread_id}`);return}t(`/projects/${k.project_id}`)},[t]),g=p.default.useCallback(async()=>{let k=null;u(null);try{k=await a.createThread()}catch(C){u({type:"error",message:C.message||e("projects.chatAutoFail")})}t(k?`/chat/${k}`:"/chat",{state:{composerDraft:e("projects.creationDraft")}})},[t,a,e]),v=p.default.useCallback(k=>{t(`/projects/${n}/threads/${k}`)},[t,n]),b=p.default.useCallback(async()=>{u(null);try{let k=await a.createThread(n);t(k?`/chat/${k}`:"/chat"),d.invalidate()}catch(k){u({type:"error",message:k.message||e("projects.chatAutoFail")})}},[t,a,n,d,e]),w=p.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),S=l`
    ${n&&l`<${A} variant="ghost" onClick=${()=>t("/projects")}>${e("projects.allProjects")}<//>`}
  `,E=null;return n?d.isLoading?E=l`
        <div className="space-y-4">
          ${[1,2,3].map(k=>l`<div key=${k} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `:d.error||!d.project&&!h?E=l`
        <${xe}
          title=${e("projects.unavailable")}
          description=${d.error?.message||e("projects.unavailableDesc")}
        >
          <${A} variant="secondary" onClick=${()=>t("/projects")}>${e("projects.returnToProjects")}<//>
        <//>
      `:E=l`
        <${AS}
          project=${d.project||h}
          threads=${d.threads}
          selectedThreadId=${r}
          onSelectThread=${v}
          onNewConversation=${b}
          isStartingConversation=${a.isCreating}
        />
      `:E=c.isLoading?l`
          <div className="space-y-4">
            ${[1,2,3].map(k=>l`<div key=${k} className="v2-skeleton h-40 rounded-[20px]" />`)}
          </div>
        `:l`
          <${CS}
            projects=${f}
            totalProjects=${c.overview.projects.length}
            search=${s}
            onSearchChange=${i}
            onOpenProject=${y}
            onCreateProject=${g}
            isPreparingChat=${a.isCreating}
          />
        `,l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <div className="flex flex-wrap justify-end gap-2">
            ${S}
          </div>
          ${c.error&&l`
            <div className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
              ${c.error.message}
            </div>
          `}
          <${en} result=${o} onDismiss=${()=>u(null)} />
          <${en} result=${m.actionResult} onDismiss=${m.clearActionResult} />
          ${!n&&l`
            <${kS} overview=${c.overview} />
            <${RS} items=${c.overview.attention} onOpenItem=${$} />
          `}
          ${E}
        </div>
      </div>
    </div>
  `}function ml(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not scheduled"}function fl(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function DS(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function MS(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);return r!==0?r:new Date(n.updated_at||0).getTime()-new Date(a.updated_at||0).getTime()})}function nd({label:e,value:t}){return l`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function mD({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=R();return e.status==="Active"?l`
      <${A} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.fireNow")}<//>
      <${A} variant="secondary" onClick=${()=>n(e.id)} disabled=${t}>${s("missions.action.pause")}<//>
    `:e.status==="Paused"?l`
      <${A} onClick=${()=>r(e.id)} disabled=${t}>${s("missions.action.resume")}<//>
      <${A} variant="secondary" onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runOnce")}<//>
    `:l`<${A} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runAgain")}<//>`}function OS({mission:e,isLoading:t,error:a,isBusy:n,onFire:r,onPause:s,onResume:i,onOpenProject:o,onOpenThread:u}){let c=R();return t?l`
      <div className="space-y-4">
        ${[1,2,3].map(d=>l`<div key=${d} className="v2-skeleton h-36 rounded-xl" />`)}
      </div>
    `:a||!e?l`
      <${xe}
        title=${c("missions.unavailable")}
        description=${a?.message||c("missions.unavailableDesc")}
      />
    `:l`
    <div className="space-y-4">
      <${I} className="p-4 sm:p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.dossier")}</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
            ${e.project&&l`
              <button
                type="button"
                onClick=${()=>o(e.project.id)}
                className="mt-2 text-sm text-signal underline-offset-4 hover:underline"
              >
                ${e.project.name}
              </button>
            `}
          </div>
          <${q} tone=${fl(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${nd} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${nd} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${nd} label=${c("missions.meta.nextFire")} value=${ml(e.next_fire_at)} />
          <${nd} label=${c("missions.meta.updated")} value=${ml(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${mD}
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
          <${ra} content=${e.goal||c("missions.noGoal")} />
        </div>
      <//>

      ${e.current_focus&&l`
        <${I} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${ra} content=${e.current_focus} />
          </div>
        <//>
      `}

      ${e.success_criteria&&l`
        <${I} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${ra} content=${e.success_criteria} />
          </div>
        <//>
      `}

      ${e.threads?.length?l`
        <${I} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.spawnedThreads")}</div>
          <div className="mt-4 space-y-3">
            ${e.threads.map(d=>l`
              <button
                key=${d.id}
                type="button"
                onClick=${()=>u(d)}
                className="w-full rounded-xl border border-white/8 bg-iron-950/60 p-4 text-left hover:border-signal/30 hover:bg-white/[0.05]"
              >
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0 truncate text-sm font-semibold text-white">${d.title||d.goal}</div>
                  <${q} tone=${fl(d.state==="Running"?"Active":d.state==="Failed"?"Failed":"Completed")} label=${d.state} />
                </div>
              </button>
            `)}
          </div>
        <//>
      `:null}
    </div>
  `}function fD(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function LS({value:e,onChange:t,children:a,label:n}){return l`
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
  `}function pD({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=R(),s=t===e.id;return l`
    <div
      className=${["w-full rounded-xl border p-4 text-left",s?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80"].join(" ")}
    >
      <button type="button" onClick=${()=>a(e.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${e.name}</div>
              <${q} tone=${fl(e.status)} label=${e.status} />
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
          ${r("missions.updated",{value:ml(e.updated_at)})}
        </span>
        <${A}
          variant="ghost"
          onClick=${i=>{i.stopPropagation(),n(e.project.id)}}
        >
          ${e.project.name}
        <//>
      </div>
    </div>
  `}function Nh({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:u,projectOptions:c,onSelectMission:d,onOpenProject:m}){let f=R(),h=fD(f);return l`
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
        <${LS} value=${s} onChange=${i} label=${f("missions.filter.status")}>
          ${h.map(x=>l`<option key=${x.value} value=${x.value}>${x.label}<//>`)}
        <//>
        <${LS} value=${o} onChange=${u} label=${f("missions.filter.project")}>
          <option value="all">${f("missions.filter.allProjects")}</option>
          ${c.map(x=>l`<option key=${x.id} value=${x.id}>${x.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(x=>l`
              <${pD}
                key=${x.id}
                mission=${x}
                selectedMissionId=${a}
                onSelectMission=${d}
                onOpenProject=${m}
              />
            `):l`
              <${xe}
                title=${f("missions.emptyTitle")}
                description=${f("missions.emptyDesc")}
                boxed=${!1}
              />
            `}
      </div>
    <//>
  `}function hD(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function PS({summary:e}){let t=R(),a=hD(t);return l`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
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
  `}function US(){return Promise.resolve({projects:[],todo:!0})}function jS({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function FS(e){return Promise.resolve(null)}function BS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function zS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function qS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function IS(e){let t=H({queryKey:["mission-detail",e],queryFn:()=>FS(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function vD(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function KS(){let e=X(),[t,a]=p.default.useState(null),n=H({queryKey:["projects-overview"],queryFn:US,refetchInterval:7e3}),r=n.data?.projects||[],s=Md({queries:r.map(f=>({queryKey:["missions","project",f.id],queryFn:()=>jS({projectId:f.id}),refetchInterval:5e3,select:h=>h?.missions||[]}))}),i=s.flatMap((f,h)=>{let x=r[h];return(f.data||[]).map(y=>vD(y,x))}),o=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),u=(f,h)=>({mutationFn:({missionId:x})=>f(x),onSuccess:()=>{a({type:"success",message:h}),o()},onError:x=>{a({type:"error",message:x.message||"Unable to update mission"})}}),c=G(u(BS,"Mission fired and a run was queued.")),d=G(u(zS,"Mission paused.")),m=G(u(qS,"Mission resumed."));return{projects:r,missions:i,summary:DS(i),isLoading:n.isLoading||s.some(f=>f.isLoading),isRefreshing:n.isFetching||s.some(f=>f.isFetching),error:n.error||s.find(f=>f.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending,invalidate:o}}function _h(){let e=R(),t=pe(),{missionId:a=null}=st(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,u]=p.default.useState("all"),c=KS(),d=IS(a),m=p.default.useMemo(()=>{let g=n.trim().toLowerCase();return MS(c.missions).filter(v=>{let b=!g||[v.name,v.goal,v.project?.name].some(E=>String(E||"").toLowerCase().includes(g)),w=s==="all"||v.status===s,S=o==="all"||v.project?.id===o;return b&&w&&S})},[c.missions,o,n,s]),f=p.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),h=d.mission?{...f,...d.mission,project:f?.project||null}:f,x=p.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),y=p.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),$=a?l`
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <${Nh}
            missions=${m}
            totalMissions=${c.missions.length}
            selectedMissionId=${a}
            search=${n}
            onSearchChange=${r}
            statusFilter=${s}
            onStatusFilterChange=${i}
            projectFilter=${o}
            onProjectFilterChange=${u}
            projectOptions=${c.projects}
            onSelectMission=${g=>t(`/missions/${g}`)}
            onOpenProject=${g=>t(`/projects/${g}`)}
          />
          <${OS}
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
      `:l`
        <${Nh}
          missions=${m}
          totalMissions=${c.missions.length}
          selectedMissionId=${a}
          search=${n}
          onSearchChange=${r}
          statusFilter=${s}
          onStatusFilterChange=${i}
          projectFilter=${o}
          onProjectFilterChange=${u}
          projectOptions=${c.projects}
          onSelectMission=${g=>t(`/missions/${g}`)}
          onOpenProject=${g=>t(`/projects/${g}`)}
        />
      `;return l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          ${a&&l`<div className="flex flex-wrap justify-end gap-2">
            <${A}
              variant="ghost"
              onClick=${()=>t("/missions")}
              >${e("missions.allMissions")}<//
            >
          </div>`}

          ${c.error&&l`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${c.error.message}
            </div>
          `}

          <${en}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${PS} summary=${c.summary} />

          ${c.isLoading?l`
                <div className="space-y-4">
                  ${[1,2,3].map(g=>l`<div
                        key=${g}
                        className="v2-skeleton h-32 rounded-xl"
                      />`)}
                </div>
              `:$}
        </div>
      </div>
    </div>
  `}var HS=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],gD=new Set(["pending","in_progress"]),QS=new Set(["failed","interrupted","stuck","cancelled"]);function ur(e){return e?String(e).replace(/_/g," "):"unknown"}function gi(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":QS.has(e)?"danger":"muted":"muted"}function yD(e){return gD.has(e)}function rd(e){return yD(e?.state)}function VS(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":QS.has(e.state):!1}function Vr(e,t=8){return e?String(e).slice(0,t):"unknown"}function sa(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function GS(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function kh(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${sa(e.started_at)}`:null].filter(Boolean).join(" / ")}var bD=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function YS(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function xD({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?l`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${t==="tool_use"?a.tool_name||"Tool call":a.tool_name||"Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${YS(t==="tool_use"?a.input:a.output||a.error||a)}</pre>
      </details>
    `:t==="message"?l`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a.role||"assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${a.content||""}</div>
      </div>
    `:l`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t.replace(/_/g," ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${a.message||a.status||YS(a)}</div>
    </div>
  `}function JS({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=R(),[s,i]=p.default.useState("all"),[o,u]=p.default.useState(""),[c,d]=p.default.useState(!0),m=p.default.useRef(null),f=p.default.useMemo(()=>s==="all"?t:t.filter(x=>x.event_type===s),[t,s]);p.default.useEffect(()=>{c&&m.current&&(m.current.scrollTop=m.current.scrollHeight)},[c,f.length]);let h=p.default.useCallback(async(x=!1)=>{let y=o.trim();if(!(!y&&!x))try{await a({content:y||"(done)",done:x}),u("")}catch{}},[o,a]);return l`
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
            ${bD.map(x=>l`<option key=${x.value} value=${x.value}>${x.label}</option>`)}
          </select>
          <label className="flex items-center gap-2 text-sm text-iron-300">
            <input type="checkbox" checked=${c} onChange=${x=>d(x.target.checked)} />
            Auto-scroll
          </label>
        </div>
      </div>

      <div ref=${m} className="mt-5 max-h-[56vh] space-y-3 overflow-y-auto rounded-[18px] border border-white/10 bg-iron-950/78 p-4">
        ${f.length?f.map(x=>l`
              <div key=${x.id||`${x.event_type}-${x.created_at}`}>
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${sa(x.created_at)}</div>
                <${xD} event=${x} />
              </div>
            `):l`
              <${xe}
                title=${r("job.noActivityTitle")}
                description=${r("job.noActivityDesc")}
              />
            `}
      </div>

      ${e.can_prompt&&l`
        <div className="mt-5 grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto_auto]">
          <input
            value=${o}
            onInput=${x=>u(x.target.value)}
            onKeyDown=${x=>{x.key==="Enter"&&!x.shiftKey&&(x.preventDefault(),h(!1))}}
            placeholder=${r("job.followupPlaceholder")}
            className="h-11 rounded-md border border-white/10 bg-iron-950/90 px-3 text-sm text-white outline-none focus:border-signal/45"
          />
          <${A} variant="secondary" disabled=${n} onClick=${()=>h(!0)}>${r("common.done")}<//>
          <${A} variant="primary" disabled=${n} onClick=${()=>h(!1)}>${r("common.send")}<//>
        </div>
      `}
    <//>
  `}function XS({job:e,activeTab:t,onTabChange:a,onBack:n,onCancel:r,onRestart:s,isBusy:i,children:o}){return l`
    <div className="space-y-5">
      <${I} className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick=${n} className="text-sm text-signal hover:text-white">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-semibold tracking-tight text-white">${e.title||"Untitled job"}</h2>
              <${q} tone=${gi(e.state)} label=${ur(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${Vr(e.id)}</span>
              <span>created ${sa(e.created_at)}</span>
              ${kh(e)&&l`<span>${kh(e)}</span>`}
            </div>
          </div>

          <div className="flex flex-wrap gap-2">
            ${e.browse_url&&l`
              <a
                href=${e.browse_url}
                target="_blank"
                rel="noreferrer noopener"
                className="v2-button inline-flex h-10 items-center rounded-md border border-white/12 bg-white/[0.04] px-4 text-sm font-semibold text-iron-100 hover:border-signal/45 hover:bg-signal/10"
              >
                Browse files
              </a>
            `}
            ${rd(e)&&l`
              <${A} variant="secondary" disabled=${i} onClick=${()=>r(e.id)}>Cancel<//>
            `}
            ${VS(e)&&l`
              <${A} variant="primary" disabled=${i} onClick=${()=>s(e.id)}>Restart<//>
            `}
          </div>
        </div>
      <//>

      <div className="flex flex-wrap gap-2">
        ${HS.map(u=>l`
          <button
            key=${u.id}
            onClick=${()=>a(u.id)}
            className=${["v2-button rounded-full border px-4 py-2 text-sm",t===u.id?"border-signal/35 bg-signal/12 text-white":"border-white/10 bg-white/[0.03] text-iron-300 hover:border-signal/25 hover:text-white"].join(" ")}
          >
            ${u.label}
          </button>
        `)}
      </div>

      ${o}
    </div>
  `}function ZS({nodes:e,depth:t=0,selectedPath:a,expandingPath:n,onToggleDirectory:r,onSelectPath:s}){return l`
    ${e.map(i=>l`
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
        ${i.isDir&&i.expanded&&i.children?.length?l`<${ZS}
              nodes=${i.children}
              depth=${t+1}
              selectedPath=${a}
              expandingPath=${n}
              onToggleDirectory=${r}
              onSelectPath=${s}
            />`:null}
      </div>
    `)}
  `}function WS({canBrowse:e,tree:t,selectedPath:a,selectedFile:n,fileError:r,isLoadingTree:s,isLoadingFile:i,expandingPath:o,treeError:u,onToggleDirectory:c,onSelectPath:d}){return e?l`
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <${I} className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          ${u&&l`<div className="mx-2 mb-3 rounded-md border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${u}</div>`}
          ${s?l`<div className="space-y-2 px-2">${[1,2,3,4].map(m=>l`<div key=${m} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?l`
                  <${ZS}
                    nodes=${t}
                    selectedPath=${a}
                    expandingPath=${o}
                    onToggleDirectory=${c}
                    onSelectPath=${d}
                  />
                `:l`<div className="px-2 py-6 text-sm text-iron-300">No files were recorded for this workspace.</div>`}
        </div>
      <//>

      <${I} className="min-h-[440px] p-5 sm:p-6">
        <div className="border-b border-white/10 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">File preview</div>
          <p className="mt-2 break-all text-sm leading-6 text-iron-300">${n?.path||a||"Select a file from the tree to inspect its contents."}</p>
        </div>

        ${r&&!i?l`<div className="mt-5 rounded-md border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">${r}</div>`:i?l`<div className="mt-5 space-y-3">${[1,2,3,4,5].map(m=>l`<div key=${m} className="v2-skeleton h-4 rounded" />`)}</div>`:n?l`<pre className="mt-5 max-h-[60vh] overflow-auto whitespace-pre-wrap rounded-[18px] border border-white/10 bg-iron-950/90 p-4 font-mono text-xs leading-6 text-iron-100">${n.content}</pre>`:l`
                <${xe}
                  title="No file selected"
                  description="Pick a concrete file from the workspace tree to render it here."
                />
              `}
      <//>
    </div>
  `:l`
      <${xe}
        title="No project workspace"
        description="File browsing is only available for sandbox jobs that produced a mounted project directory."
      />
    `}function yi({label:e,value:t}){return l`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function eN({job:e}){let t=(e.transitions||[]).map(a=>({title:`${ur(a.from)} -> ${ur(a.to)}`,description:[sa(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${I} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${q} tone=${gi(e.state)} label=${ur(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${yi} label="Created" value=${sa(e.created_at)} />
          <${yi} label="Started" value=${sa(e.started_at)} />
          <${yi} label="Completed" value=${sa(e.completed_at)} />
          <${yi} label="Duration" value=${GS(e.elapsed_secs)} />
          <${yi} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${yi} label="Mode" value=${e.job_mode||"Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${I} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${e.description?l`<${ra} content=${e.description} className="mt-4 text-sm leading-7 text-iron-200" />`:l`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
        <//>

        ${t.length?l`
              <${I} className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Transitions</div>
                <h3 className="mt-2 text-xl font-semibold text-white">State timeline</h3>
                <div className="mt-3">
                  <${G2} items=${t} />
                </div>
              <//>
            `:l`
              <${xe}
                title="No state history yet"
                description="Transitions appear here once the job advances or records a recovery event."
              />
            `}
      </div>
    </div>
  `}function tN({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:u,isBusy:c,isRefreshing:d}){let m=R(),f=[{value:"all",label:m("jobs.list.filter.all")},{value:"pending",label:m("jobs.list.filter.pending")},{value:"in_progress",label:m("jobs.list.filter.inProgress")},{value:"completed",label:m("jobs.list.filter.completed")},{value:"failed",label:m("jobs.list.filter.failed")},{value:"stuck",label:m("jobs.list.filter.stuck")}];if(!e.length){let h=!!n.trim()||s!=="all";return l`
      <${xe}
        title=${m(t&&h?"jobs.list.empty.noMatchTitle":"jobs.list.empty.noJobsTitle")}
        description=${m(t&&h?"jobs.list.empty.noMatchDesc":"jobs.list.empty.noJobsDesc")}
      />
    `}return l`
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
            ${f.map(h=>l`<option key=${h.value} value=${h.value}>${h.label}</option>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(h=>l`
          <article
            key=${h.id}
            className=${["group flex flex-col gap-4 rounded-[18px] border p-5",a===h.id?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
          >
            <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
              <button onClick=${()=>o(h.id)} className="min-w-0 text-left">
                <div className="flex flex-wrap items-center gap-2">
                  <h3 className="truncate text-lg font-semibold text-iron-100">${h.title||m("jobs.list.untitled")}</h3>
                  <${q} tone=${gi(h.state)} label=${ur(h.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${Vr(h.id)}</span>
                  <span>${m("jobs.list.created",{value:sa(h.created_at)})}</span>
                  ${h.started_at&&l`<span>${m("jobs.list.started",{value:sa(h.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${rd(h)&&l`
                  <${A}
                    variant="secondary"
                    className="h-9 px-3 text-xs"
                    disabled=${c}
                    onClick=${()=>u(h.id)}
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
  `}var $D=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function aN({summary:e}){return l`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${$D.map(t=>l`
          <div
            key=${t.key}
            className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
          >
            <${et}
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
  `}function nN(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function rN(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function sN(e){return Promise.resolve(null)}function iN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function oN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function lN(e){return Promise.resolve({events:[],todo:!0})}function uN(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function Rh(e,t=""){return Promise.resolve({entries:[],todo:!0})}function cN(e,t){return Promise.resolve({content:"",todo:!0})}function dN(e){let t=X(),[a,n]=p.default.useState(null),r=H({queryKey:["job-detail",e],queryFn:()=>sN(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=H({queryKey:["job-events",e],queryFn:()=>lN(e),enabled:!!e,refetchInterval:e?2500:!1}),i=G({mutationFn:({content:o,done:u})=>uN(e,{content:o,done:u}),onSuccess:(o,{done:u})=>{n({type:"success",message:u?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return p.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function mN(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function fN(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=fN(a.children,t);if(n)return n}}return null}function sd(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:sd(n.children,t,a)}:n)}function pN(e){let[t,a]=p.default.useState([]),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,u]=p.default.useState(""),c=!!(e?.project_dir&&e?.id),d=H({queryKey:["job-files-root",e?.id],queryFn:()=>Rh(e.id,""),enabled:c}),m=H({queryKey:["job-file",e?.id,n],queryFn:()=>cN(e.id,n),enabled:!!(c&&n)});p.default.useEffect(()=>{a([]),r(""),i(""),u("")},[e?.id]),p.default.useEffect(()=>{d.data?.entries?(a(mN(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let f=p.default.useCallback(async h=>{let x=fN(t,h);if(!(!x||!e?.id)){if(x.expanded){a(y=>sd(y,h,$=>({...$,expanded:!1})));return}if(x.loaded){a(y=>sd(y,h,$=>({...$,expanded:!0})));return}u(h);try{let y=await Rh(e.id,h);a($=>sd($,h,g=>({...g,expanded:!0,loaded:!0,children:mN(y.entries)}))),i("")}catch(y){i(y.message||"Unable to open folder")}finally{u("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:m.data||null,fileError:m.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:m.isLoading||m.isFetching,expandingPath:o,treeError:s,toggleDirectory:f}}function hN(){let e=X(),[t,a]=p.default.useState(null),n=H({queryKey:["jobs-summary"],queryFn:rN,refetchInterval:5e3}),r=H({queryKey:["jobs"],queryFn:nN,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=G({mutationFn:({jobId:u})=>iN(u),onSuccess:(u,{jobId:c})=>{a({type:"success",message:`Job ${Vr(c)} cancelled`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to cancel job"})}}),o=G({mutationFn:({jobId:u})=>oN(u),onSuccess:u=>{a({type:"success",message:`Restart queued as ${Vr(u?.new_job_id)}`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function vN({result:e,onDismiss:t}){let a=R();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return l`
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
  `}function Ch(){let e=R(),t=pe(),{jobId:a=null}=st(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,u]=p.default.useState(a?"activity":"overview"),c=hN(),d=dN(a),m=pN(d.job);p.default.useEffect(()=>{u(a?"activity":"overview")},[a]);let f=p.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(b=>{let w=!v||b.title.toLowerCase().includes(v)||b.id.toLowerCase().includes(v),S=s==="all"||b.state===s;return w&&S})},[c.jobs,n,s]),h=p.default.useCallback(v=>t(`/jobs/${v}`),[t]),x=p.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),y=p.default.useCallback(async v=>{try{let b=await c.restartJob({jobId:v});b?.new_job_id&&t(`/jobs/${b.new_job_id}`)}catch{}},[c,t]),$=l`
    ${a&&l`<${A} variant="ghost" onClick=${()=>t("/jobs")}
      >${e("jobs.allJobs")}<//
    >`}
  `,g=null;if(a)if(d.isLoading)g=l`
        <div className="space-y-4">
          ${[1,2,3].map(v=>l`<div key=${v} className="v2-skeleton h-32 rounded-[18px]" />`)}
        </div>
      `;else if(d.error||!d.job)g=l`
        <${xe}
          title=${e("jobs.unavailable")}
          description=${d.error?.message||e("jobs.unavailableDesc")}
        >
          <${A} variant="secondary" onClick=${()=>t("/jobs")}
            >${e("jobs.returnToJobs")}<//
          >
        <//>
      `;else{let v={overview:l`<${eN} job=${d.job} />`,activity:l`
          <${JS}
            job=${d.job}
            events=${d.events}
            onSendPrompt=${d.sendPrompt}
            isSendingPrompt=${d.isSendingPrompt}
          />
        `,files:l`
          <${WS}
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
        `};g=l`
        <${XS}
          job=${d.job}
          activeTab=${o}
          onTabChange=${u}
          onBack=${()=>t("/jobs")}
          onCancel=${x}
          onRestart=${y}
          isBusy=${c.isBusy}
        >
          ${v[o]||v.overview}
        <//>
      `}else g=c.isLoading?l`
          <div className="space-y-4">
            ${[1,2,3].map(v=>l`<div
                  key=${v}
                  className="v2-skeleton h-28 rounded-[18px]"
                />`)}
          </div>
        `:l`
          <${tN}
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
        `;return l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          ${a&&l`<div className="flex flex-wrap justify-end gap-2">
            ${$}
          </div>`}
          ${c.error&&l`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${c.error.message}
            </div>
          `}
          <${vN}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${vN}
            result=${d.promptResult}
            onDismiss=${d.clearPromptResult}
          />
          <${aN} summary=${c.summary} />
          ${g}
        </div>
      </div>
    </div>
  `}function cr(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not scheduled"}function id(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function od(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function gN(e=[]){return[...e].sort((t,a)=>t.enabled!==a.enabled?t.enabled?-1:1:new Date(a.next_fire_at||a.last_run_at||0).getTime()-new Date(t.next_fire_at||t.last_run_at||0).getTime())}function yN(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function wD(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function bN({runs:e}){return e?.length?l`
    <div className="space-y-3">
      ${e.map(t=>l`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${q} tone=${wD(t.status)} label=${t.status} />
              <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                ${cr(t.started_at)}
              </span>
            </div>
            ${t.result_summary&&l`<p className="mt-3 text-sm leading-6 text-iron-300">${t.result_summary}</p>`}
          </div>
        `)}
    </div>
  `:l`
      <div className="rounded-xl border border-iron-700 bg-iron-950/40 p-4 text-sm text-iron-300">
        No runs recorded yet.
      </div>
    `}function dr({label:e,value:t}){return l`
    <div className="rounded-xl border border-iron-700 bg-iron-950/50 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div className="mt-2 min-w-0 break-words text-sm text-iron-100">
        ${t||"\u2014"}
      </div>
    </div>
  `}function xN({title:e,value:t}){return l`
    <div>
      <h3 className="text-sm font-semibold text-iron-100">${e}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >${JSON.stringify(t||{},null,2)}</pre>
    </div>
  `}function $N({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=pe(),u=R();return t?l`
      <div className="space-y-4">
        ${[1,2,3].map(c=>l`<div key=${c} className="v2-skeleton h-32 rounded-xl" />`)}
      </div>
    `:a||!e?l`
      <${xe}
        title=${u("routine.unavailable")}
        description=${a?.message||u("routine.unavailableDesc")}
      />
    `:l`
    <${I} className="p-4 sm:p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="truncate text-2xl font-semibold tracking-tight text-iron-100">
              ${e.name}
            </h2>
            <${q}
              tone=${id(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${q}
              tone=${od(e.verification_status)}
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
        <${dr} label="Trigger" value=${e.trigger_summary||e.trigger_type} />
        <${dr} label="Action" value=${yN(e.action)} />
        <${dr} label="Next fire" value=${cr(e.next_fire_at)} />
        <${dr} label="Last run" value=${cr(e.last_run_at)} />
        <${dr} label="Run count" value=${e.run_count} />
        <${dr} label="Failures" value=${e.consecutive_failures} />
        <${dr} label="Created" value=${cr(e.created_at)} />
        <${dr} label="Routine ID" value=${e.id} />
      </div>

      ${e.conversation_id&&l`
        <div className="mt-5">
          <${A} variant="secondary" onClick=${()=>o(`/chat/${e.conversation_id}`)}>
            Open routine thread
          <//>
        </div>
      `}

      <div className="mt-6 grid gap-6 xl:grid-cols-2">
        <${xN} title=${u("routine.triggerPayload")} value=${e.trigger} />
        <${xN} title=${u("routine.actionPayload")} value=${e.action} />
      </div>

      <div className="mt-6">
        <h3 className="mb-3 text-sm font-semibold text-iron-100">Recent runs</h3>
        <${bN} runs=${e.recent_runs} />
      </div>
    <//>
  `}function wN({routine:e,selectedRoutineId:t,onSelectRoutine:a,onTriggerRoutine:n,onToggleRoutine:r,isBusy:s}){let i=t===e.id;return l`
    <article
      className=${["group flex flex-col gap-4 rounded-[18px] border p-5",i?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick=${()=>a(e.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">${e.name}</h3>
            <${q}
              tone=${id(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${q}
              tone=${od(e.verification_status)}
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
            <span>next ${cr(e.next_fire_at)}</span>
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
  `}var SD=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function Eh({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:u,onToggleRoutine:c,isBusy:d,isRefreshing:m}){let f=R();if(!e.length){let h=!!n.trim()||s!=="all";return l`
      <${xe}
        title=${t&&h?"No routines match":"No routines yet"}
        description=${t&&h?"Adjust the search or status filter to find a saved routine.":"Routines created from chat will appear here after they are saved."}
      />
    `}return l`
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
            ${SD.map(h=>l`<option key=${h.value} value=${h.value}>${h.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(h=>l`
            <${wN}
              key=${h.id}
              routine=${h}
              selectedRoutineId=${a}
              onSelectRoutine=${o}
              onTriggerRoutine=${u}
              onToggleRoutine=${c}
              isBusy=${d}
            />
          `)}
      </div>
    </div>
  `}var ND=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function SN({summary:e}){return l`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${ND.map(t=>l`
            <div
              key=${t.key}
              className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
            >
              <${et}
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
  `}function NN(e){let[t,a]=p.default.useState(""),[n,r]=p.default.useState("all");return{filteredRoutines:p.default.useMemo(()=>{let i=t.trim().toLowerCase();return gN(e).filter(o=>{let u=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||u.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function _N(){return Promise.resolve({routines:[],todo:!0})}function kN(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function RN(e){return Promise.resolve(null)}function ld(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function ud(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function CN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function EN(e){let t=X(),[a,n]=p.default.useState(null),r=H({queryKey:["routine-detail",e],queryFn:()=>RN(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:m=>{n({type:"error",message:m.message||"Unable to update routine"})}}),o=G(i(ld,"Routine run queued.")),u=G(i(ud,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,isBusy:o.isPending||u.isPending}}function TN(){let e=X(),[t,a]=p.default.useState(null),n=H({queryKey:["routines-summary"],queryFn:kN,refetchInterval:5e3}),r=H({queryKey:["routines"],queryFn:_N,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,m)=>({mutationFn:({routineId:f})=>d(f),onSuccess:()=>{a({type:"success",message:m}),s()},onError:f=>{a({type:"error",message:f.message||"Unable to update routine"})}}),o=G(i(ld,"Routine run queued.")),u=G(i(ud,"Routine status updated.")),c=G(i(CN,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||u.isPending||c.isPending,invalidate:s}}function Th(){let e=pe(),{routineId:t=null}=st(),a=TN(),n=EN(t),r=NN(a.routines),s=p.default.useCallback(async(u,c)=>{try{await u({routineId:c})}catch{}},[]),i=p.default.useCallback(async(u,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:u}),e("/routines")}catch{}},[e,a]),o=t?l`
        <div className="grid gap-5 xl:grid-cols-[minmax(0,0.9fr)_minmax(440px,1.1fr)]">
          <${Eh}
            routines=${r.filteredRoutines}
            totalRoutines=${a.routines.length}
            selectedRoutineId=${t}
            search=${r.search}
            onSearchChange=${r.setSearch}
            statusFilter=${r.statusFilter}
            onStatusFilterChange=${r.setStatusFilter}
            onSelectRoutine=${u=>e(`/routines/${u}`)}
            onTriggerRoutine=${u=>s(a.triggerRoutine,u)}
            onToggleRoutine=${u=>s(a.toggleRoutine,u)}
            isBusy=${a.isBusy}
            isRefreshing=${a.isRefreshing}
          />
          <${$N}
            routine=${n.routine}
            isLoading=${n.isLoading}
            error=${n.error}
            isBusy=${n.isBusy}
            onTriggerRoutine=${n.triggerRoutine}
            onToggleRoutine=${n.toggleRoutine}
            onDeleteRoutine=${()=>i(t,n.routine?.name||t)}
          />
        </div>
      `:l`
        <${Eh}
          routines=${r.filteredRoutines}
          totalRoutines=${a.routines.length}
          selectedRoutineId=${t}
          search=${r.search}
          onSearchChange=${r.setSearch}
          statusFilter=${r.statusFilter}
          onStatusFilterChange=${r.setStatusFilter}
          onSelectRoutine=${u=>e(`/routines/${u}`)}
          onTriggerRoutine=${u=>s(a.triggerRoutine,u)}
          onToggleRoutine=${u=>s(a.toggleRoutine,u)}
          isBusy=${a.isBusy}
          isRefreshing=${a.isRefreshing}
        />
      `;return l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          ${t&&l`<div className="flex flex-wrap justify-end gap-2">
            <${A} variant="ghost" onClick=${()=>e("/routines")}>
              All routines
            <//>
          </div>`}

          ${a.error&&l`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${a.error.message}
            </div>
          `}

          <${en}
            result=${a.actionResult}
            onDismiss=${a.clearActionResult}
          />
          <${en}
            result=${n.actionResult}
            onDismiss=${n.clearActionResult}
          />
          <${SN} summary=${a.summary} />

          ${a.isLoading?l`
                <div className="space-y-4">
                  ${[1,2,3].map(u=>l`<div key=${u} className="v2-skeleton h-32 rounded-xl" />`)}
                </div>
              `:o}
        </div>
      </div>
    </div>
  `}function _D(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function kD(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function AN({deliveryState:e}){let t=R(),a=e.currentTarget?.target_id||"",[n,r]=p.default.useState(a),[s,i]=p.default.useState(!1),o=p.default.useRef(null);p.default.useEffect(()=>{r(a)},[a]),p.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let u=n!==a,c=e.isLoading||e.isSaving,d=u&&!c,m=!!a&&!c,f=e.finalReplyTargets.length>0,h=e.targets.some(O=>O?.capabilities?.final_replies&&O?.target?.status==="unavailable"),x=f||h,y=O=>(o.current&&clearTimeout(o.current),i(!1),O.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),$=()=>{d&&y(e.saveFinalReplyTarget(n||null))},g=()=>{m&&(r(""),y(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),b=e.currentStatus,w=b==="available"?"success":b==="unavailable"?"warning":"muted",S=t(b==="available"?"automations.delivery.pill.ready":b==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),E=!!e.currentTarget,k=t(E?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),C=kD(t("automations.delivery.footnote"),{command:l`<code
        key="cmd"
        className="rounded px-1.5 py-0.5 font-mono text-[0.6875rem] bg-[var(--v2-surface-muted)] text-[var(--v2-accent-text)]"
      >
        approve &lt;code&gt;
      </code>`});return l`
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
        ${E&&l`
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
              <${q} tone=${w} label=${S} />
            </div>
          </div>
        `}

        <!-- ── Radio option rows ────────────────────────────────────── -->
        <div>
          <span className="mb-1.5 block font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
            ${k}
          </span>
          <div
            className="flex flex-col gap-3"
            role="radiogroup"
            aria-label=${t("automations.delivery.title")}
          >

            <!-- Available external targets -->
            ${e.finalReplyTargets.map(O=>{let L=O?.target?.target_id??"",P=O?.target?.display_name||O?.target?.target_id||"",U=O?.target?.description||"",T=O?.target?.status??"available",K=n===L;return l`
                <label
                  key=${L}
                  className=${Y("flex items-start gap-3.5 rounded-xl border px-4 py-3.5 cursor-pointer","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]","hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",K&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
                >
                  <input
                    type="radio"
                    name="delivery-target"
                    value=${L}
                    checked=${K}
                    disabled=${c}
                    onChange=${()=>r(L)}
                    className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-semibold text-[var(--v2-text-strong)] leading-snug">
                      ${P}
                    </div>
                    ${U&&l`<div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                      ${U}
                    </div>`}
                  </div>
                  <${q}
                    tone=${_D(T)}
                    label=${t(T==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.ready")}
                    className="self-center shrink-0"
                  />
                </label>
              `})}

            <!-- Unpaired notice rows (targets present but status=unavailable
                 and NOT already shown above because they lack final_replies) -->
            ${h&&l`
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
              className=${Y("flex items-start gap-3.5 rounded-xl border px-4 py-3.5","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]",f?"cursor-pointer hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]":"cursor-default",n===""&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
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
          <${A}
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
          ${s&&l`
            <span
              role="status"
              className="flex items-center gap-1.5 text-xs font-semibold text-[var(--v2-positive-text)]"
            >
              <${M} name="check" className="h-3 w-3" />
              ${t("automations.delivery.saved")}
            </span>
          `}
          ${e.saveError&&!s&&l`
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
        ${x&&l`
          <div
            className="rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-faint)]"
          >
            ${C}
          </div>
        `}

      </div>
    <//>
  `}var RD=["schedule","once"],MN={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},ON={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},LN={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function ia(e){return typeof e=="function"?e:t=>t}var Dh=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:Rn},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:qD},{value:"completed",labelKey:"automations.filter.completed",predicate:ID}];function PN(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>RD.includes(r?.source?.type)).map(r=>UD(r,t,a)).sort(zD)}function UN(e,t){let a=Dh.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function jN(e){let t=e.filter(i=>i.state!=="completed"),a=t.filter(i=>Rn(i)).length,n=t.filter(i=>i.has_running_run).length,r=t.filter(i=>i.has_failed_runs).length,s=t.filter(i=>Rn(i)&&Ah(i)!=null).sort((i,o)=>(i.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(o.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:t.length,active:a,running:n,failures:r,nextRun:s?.next_run_label||null}}function CD(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=VD(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:u,month:c,dayOfWeek:d,year:m}=s,f=t&&typeof t=="string"?t:null,h=f?` (${f})`:"",x=m==="*"&&u==="*"&&c==="*"&&d==="*";if(x&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=GD(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(mr(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let y=KD(o,i,n);if(!y)return r("automations.schedule.custom");if(x)return r("automations.schedule.everyDayAt",{time:y})+h;let $=YD(d);if(m==="*"&&u==="*"&&c==="*"&&$==="1-5")return r("automations.schedule.weekdaysAt",{time:y})+h;if(m==="*"&&u==="*"&&c==="*"&&mr($,0,7)){let g=HD(Number($)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:y})+h}if(m==="*"&&mr(u,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(u),time:y})+h;if(mr(u,1,31)&&mr(c,1,12)&&d==="*"&&(m==="*"||mr(m,1970,9999))){let g=QD(Number(c),Number(u),m==="*"?null:Number(m),n);return r("automations.schedule.dateAt",{date:g,time:y})+h}return r("automations.schedule.custom")}function Gr(e,t="Unknown",a,n){if(!e)return t;let r=new Date(e);if(Number.isNaN(r.getTime()))return t;let s=n&&typeof n=="string"?{timeZone:n}:{};try{return r.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...s})}catch{return r.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function FN(e,t){let a=MN[e]?.labelKey||"automations.state.unknown";return ia(t)(a)}function BN(e){return MN[e]?.tone||"muted"}function ED(e,t){return Rn(e)&&e?.has_running_run?ia(t)("automations.status.running"):Rn(e)&&e?.has_failed_runs?ia(t)("automations.status.needsReview"):FN(e?.state,t)}function TD(e){return Rn(e)&&e?.has_running_run?"info":Rn(e)&&e?.has_failed_runs?"danger":BN(e?.state)}function AD(e,t){let a=ON[e]?.labelKey||"automations.lastStatus.none";return ia(t)(a)}function DD(e){return ON[e]?.tone||"muted"}function MD(e,t){let a=LN[cd(e)]?.labelKey||"automations.runStatus.unknown";return ia(t)(a)}function OD(e){return LN[cd(e)]?.tone||"muted"}function LD(e,t,a,n){if(!e)return ia(a)("automations.schedule.custom");let r=Gr(e,null,n,t);if(!r)return ia(a)("automations.schedule.custom");let s=t&&typeof t=="string"?` (${t})`:"";return ia(a)("automations.schedule.onceAt",{datetime:r})+s}function PD(e,t,a){return e?.type==="once"?LD(e.at,e.timezone,t,a):e?.type==="schedule"?CD(e.cron,e.timezone||"UTC",t,a):ia(t)("automations.schedule.custom")}function UD(e,t,a){let n=ia(t),r=jD(e.recent_runs,t,a),s=r[0]||null,i=r.find(m=>m.status==="running")||null,o=r.find(m=>m.status==="ok"||m.status==="error")||null,u=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null,d={...e,recent_runs:r,has_running_run:r.some(m=>m.status==="running"),has_failed_runs:r.some(m=>m.status==="error")};return{...d,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:PD(e.source,t,a),state_label:FN(e.state,t),state_tone:BN(e.state),primary_status_label:ED(d,t),primary_status_tone:TD(d),next_run_timestamp:Mh(e.next_run_at),next_run_label:Gr(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:Gr(c,n("automations.date.noRuns"),a),last_status_label:AD(u,t),last_status_tone:DD(u),created_label:Gr(e.created_at,n("automations.date.unknown"),a),latest_run:s,current_run:i,success_rate_label:BD(r,t)}}function jD(e,t,a){let n=ia(t);return Array.isArray(e)?e.map(r=>{let s=cd(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=Mh(i);return{...r,status:s,status_label:MD(s,t),status_tone:OD(s),timestamp:o,timestamp_source:i,fired_label:Gr(i,n("automations.date.unscheduled"),a),submitted_label:Gr(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:Gr(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function cd(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function zN(e){let t=Array.isArray(e)?e:[],a={total:t.length,ok:0,error:0,running:0,unknown:0};for(let n of t){let r=cd(n?.status);Object.prototype.hasOwnProperty.call(a,r)?a[r]+=1:a.unknown+=1}return a}function FD(e){let t=zN(e);return[{key:"ok",tone:"text-emerald-300",count:t.ok},{key:"error",tone:"text-red-300",count:t.error},{key:"running",tone:"text-sky-300",count:t.running},{key:"unknown",tone:"text-iron-400",count:t.unknown}].filter(a=>a.count>0)}function qN(e,t){let a=ia(t),n=zN(e),r=FD(e).map(s=>({...s,text:a(`automations.runs.${s.key}`,{count:s.count})}));return{total:n.total,totalText:a("automations.runs.total",{count:n.total}),chips:r}}function BD(e,t){let a=ia(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function zD(e,t){let a=Rn(e),n=Rn(t);return a!==n?a?-1:1:(Ah(e)??Number.MAX_SAFE_INTEGER)-(Ah(t)??Number.MAX_SAFE_INTEGER)}function Mh(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function Rn(e){return e?.state==="active"||e?.state==="scheduled"}function qD(e){return["paused","disabled","inactive"].includes(e?.state)}function ID(e){return e?.state==="completed"}function Ah(e){return e?.next_run_timestamp??Mh(e?.next_run_at)}function Oh(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function KD(e,t,a){return!mr(e,0,23)||!mr(t,0,59)?null:Oh(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function HD(e,t){return Oh(t,{weekday:"long"},new Date(2001,0,7+e))}function QD(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return Oh(n,r,new Date(a??2e3,e-1,t))}function VD(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&DN(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&DN(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function DN(e){return/^0+$/.test(e)}function mr(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function GD(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function YD(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}var JD=8;function Lh(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function dd({runs:e=[]}){let t=R(),a=Array.isArray(e)?e:[],n=a.slice(0,JD);if(!n.length)return l`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;let r=a.length-n.length,s=`+${Math.min(r,999)}`;return l`
    <div
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
    >
      ${n.map(i=>l`
        <span
          key=${Lh(i)}
          title=${`${i.status_label} \xB7 ${i.fired_label}`}
          className=${Y("h-3 w-3 rounded-full border",i.status==="ok"&&"border-emerald-300/50 bg-emerald-400",i.status==="error"&&"border-red-300/50 bg-red-400",i.status==="running"&&"border-sky-300/60 bg-sky-400",i.status==="unknown"&&"border-iron-500 bg-iron-600")}
        />
      `)}
      ${r>0&&l`<span
        className="ml-0.5 font-mono text-[11px] text-iron-400"
        title=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
      >
        ${s}
      </span>`}
    </div>
  `}function md({runs:e=[],className:t=""}){let a=R(),n=qN(e,a);return n.total?l`
    <div className=${Y("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]",t)}>
      <span className="text-iron-300">${n.totalText}</span>
      ${n.chips.map(r=>l`<span key=${r.key} className=${r.tone}>${r.text}</span>`)}
    </div>
  `:l`<span className=${Y("text-[11px] text-iron-400",t)}>
      ${a("automations.table.noRuns")}
    </span>`}function IN({run:e,onOpenRun:t,onOpenLogs:a}){let n=R(),r=!!e.chat_path,s=Qc({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return l`
    <div className="grid gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto] sm:items-center">
      <div>
        <${q} tone=${e.status_tone} label=${e.status_label} />
      </div>
      <div className="min-w-0">
        <div className="text-sm font-semibold text-iron-100">${e.fired_label}</div>
        <div className="mt-1 truncate font-mono text-[11px] text-iron-400">
          ${e.thread_id?`${n("automations.detail.thread")} ${e.thread_id}`:n("automations.detail.noThread")}
        </div>
        ${e.run_id&&l`
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
  `}function fd({label:e,value:t,tone:a}){return l`
    <div className="min-w-0 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div
        className=${Y("mt-2 min-w-0 break-words text-sm text-iron-100",a==="success"&&"text-emerald-200",a==="danger"&&"text-red-200",a==="info"&&"text-sky-200")}
      >
        ${t||"\u2014"}
      </div>
    </div>
  `}function KN({automation:e,isMutating:t=!1,onPauseAutomation:a,onResumeAutomation:n,onDeleteAutomation:r}){let s=R(),i=pe();if(!e)return l`
      <${I} className="p-4 sm:p-5">
        <${xe}
          boxed=${!1}
          title=${s("automations.detail.emptyTitle")}
          description=${s("automations.detail.emptyDescription")}
        />
      <//>
    `;let o=e.current_run,u=e.state==="paused",c=e.state==="active"||e.state==="scheduled",m=`${s(u?"missions.action.resume":"missions.action.pause")}: ${e.display_name}`,f=()=>{if(u){n?.(e.automation_id);return}c&&a?.(e.automation_id)},h=`${s("common.delete")}: ${e.display_name}`,x=()=>{window.confirm(h)&&r?.(e.automation_id)};return l`
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
            ${(c||u)&&l`
              <${A}
                type="button"
                variant=${u?"primary":"secondary"}
                size="icon-sm"
                aria-label=${m}
                title=${m}
                disabled=${t}
                onClick=${f}
              >
                <${M} name=${u?"play":"pause"} className="h-4 w-4" />
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
          <${fd} label=${s("automations.detail.schedule")} value=${e.schedule_label} />
          <${fd}
            label=${s("automations.detail.successRate")}
            value=${e.success_rate_label}
            tone=${e.has_failed_runs?"danger":"success"}
          />
          <${fd} label=${s("automations.detail.lastCompleted")} value=${e.last_run_label} />
          <${fd}
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
              <${dd} runs=${e.recent_runs} />
              <${md} runs=${e.recent_runs} />
            </div>
          </div>

          ${e.recent_runs.length?l`
                <div>
                  ${e.recent_runs.map(y=>l`
                    <${IN}
                      key=${Lh(y)}
                      run=${y}
                      onOpenRun=${i}
                      onOpenLogs=${i}
                    />
                  `)}
                </div>
              `:l`
                <div className="rounded-xl border border-dashed border-[var(--v2-panel-border)] p-4 text-sm text-iron-300">
                  ${s("automations.detail.noRuns")}
                </div>
              `}
        </div>
      </div>
    <//>
  `}var XD=["automations.empty.example1","automations.empty.example2","automations.empty.example3"];function ZD({promptKey:e}){let t=R(),a=t(e),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>()=>clearTimeout(s.current),[]),l`
    <li
      className="flex items-center gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3"
    >
      <span className="min-w-0 flex-1 text-sm leading-6 text-iron-200">${a}</span>
      <button
        type="button"
        onClick=${async()=>{let o=typeof navigator>"u"?null:navigator.clipboard;if(o?.writeText)try{await o.writeText(a),r(!0),clearTimeout(s.current),s.current=setTimeout(()=>r(!1),1500)}catch{}}}
        aria-label=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        title=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        className=${Y("inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-[var(--v2-panel-border)] text-iron-300 hover:text-iron-100 hover:border-white/20","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",n&&"text-emerald-300")}
      >
        <${M} name=${n?"check":"copy"} className="h-4 w-4" />
      </button>
    </li>
  `}function HN(){let e=R(),t=pe();return l`
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
            ${XD.map(a=>l`<${ZD} key=${a} promptKey=${a} />`)}
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
  `}function QN({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,isMutating:s,selectedAutomationId:i,onSelectAutomation:o,onPauseAutomation:u,onResumeAutomation:c,onDeleteAutomation:d}){let m=R(),f=UN(e,t),h=e.length>0,x=f.find(y=>y.automation_id===i)||f[0]||null;return l`
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
              ${Dh.map(y=>l`
                <button
                  key=${y.value}
                  type="button"
                  aria-pressed=${t===y.value}
                  onClick=${()=>a(y.value)}
                  className=${Y("min-h-9 shrink-0 whitespace-nowrap px-3 py-2 text-xs font-semibold leading-tight",t===y.value?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
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
                className=${Y("h-4 w-4",r&&"v2-spin")}
              />
            <//>
          </div>
        </div>
      <//>

      ${f.length?l`
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
                      ${f.map(y=>{let $=y.automation_id===x?.automation_id;return l`
                          <tr
                            key=${y.automation_id}
                            className=${Y("border-b border-[var(--v2-panel-border)] last:border-0 hover:bg-white/[0.03]",$&&"bg-[var(--v2-accent-soft)]/30")}
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
                                <${dd} runs=${y.recent_runs} />
                                <${md} runs=${y.recent_runs} />
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

              <${KN}
                automation=${x}
                isMutating=${s}
                onPauseAutomation=${u}
                onResumeAutomation=${c}
                onDeleteAutomation=${d}
              />
            </div>
          `:h?l`
              <${xe}
                title=${m("automations.empty.matchingTitle")}
                description=${m("automations.empty.matchingDescription")}
              />
            `:l`<${HN} />`}
    </div>
  `}function VN({summary:e,activeFilter:t,onSelectFilter:a}){let n=R(),r=[{key:"scheduled",label:n("automations.summary.scheduled"),value:e?.scheduled??0,tone:"muted",detail:n("automations.summary.scheduledDetail"),filter:"all"},{key:"active",label:n("automations.summary.active"),value:e?.active??0,tone:"signal",detail:n("automations.summary.activeDetail"),filter:"active"},{key:"running",label:n("automations.summary.running"),value:e?.running??0,tone:"info",detail:n("automations.summary.runningDetail"),filter:"running"},{key:"failures",label:n("automations.summary.failures"),value:e?.failures??0,tone:(e?.failures??0)>0?"danger":"success",detail:n("automations.summary.failuresDetail"),filter:(e?.failures??0)>0?"failures":null},{key:"nextRun",label:n("automations.summary.nextRun"),value:e?.nextRun||n("automations.summary.none"),tone:"info",detail:n("automations.summary.nextRunDetail"),valueClassName:"text-lg md:text-xl"}];return l`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        ${r.map(s=>{let i=!!(s.filter&&a),o=i&&t===s.filter,u=l`
            <${et}
              label=${s.label}
              value=${s.value}
              tone=${s.tone}
              badgeLabel=${n(`automations.badge.${s.tone}`)}
              detail=${s.detail}
              valueClassName=${s.valueClassName}
              showDivider=${!1}
              className="px-0 py-0"
            />
          `,c="rounded-[14px] border border-white/8 bg-white/[0.03] p-4 text-left";return i?l`
            <button
              key=${s.key}
              type="button"
              aria-pressed=${o}
              title=${n("automations.summary.filterAction",{label:s.label})}
              onClick=${()=>a(s.filter)}
              className=${Y(c,"transition-colors hover:border-white/20 hover:bg-white/[0.05]","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",o&&"border-[var(--v2-accent)]/60 bg-[var(--v2-accent-soft)]/30")}
            >
              ${u}
            </button>
          `:l`<div key=${s.key} className=${c}>${u}</div>`})}
      </div>
    <//>
  `}function WD(e){return e==="active"||e==="scheduled"}function eM(e){return Number.isFinite(e)?e:null}function GN(e,t=Date.now()){let a=Array.isArray(e)?e:[],n=null;for(let r of a){if(!r||(r.has_running_run&&(n=n==null?5e3:Math.min(n,5e3)),!WD(r.state)))continue;let s=eM(r.next_run_timestamp);if(s==null)continue;let i=s-t,o=i<=0?2e3:i<3e4?Math.max(1e3,i+1200):null;o!=null&&(n=n==null?o:Math.min(n,o))}return n}var aM=50,nM=25;function YN(e=!1){let{t,lang:a}=vl(),n=X(),r=H({queryKey:["automations",{includeCompleted:e}],queryFn:()=>Ox({limit:aM,runLimit:nM,includeCompleted:e}),refetchInterval:3e4,refetchIntervalInBackground:!1}),s=p.default.useMemo(()=>PN(r.data,t,a),[r.data,t,a]),i=p.default.useMemo(()=>jN(s),[s]),o=p.default.useMemo(()=>GN(s),[s]);p.default.useEffect(()=>{if(o==null)return;let h=setTimeout(()=>{r.refetch()},o);return()=>clearTimeout(h)},[o,r.refetch]);let u=r.data?.scheduler_enabled!==!1,c=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["automations"]})},[n]),d=G({mutationFn:h=>Lx({automationId:h}),onSuccess:c}),m=G({mutationFn:h=>Px({automationId:h}),onSuccess:c}),f=G({mutationFn:h=>Ux({automationId:h}),onSuccess:c});return{automations:s,summary:i,schedulerEnabled:u,isLoading:r.isLoading,isRefreshing:r.isFetching,isMutating:d.isPending||m.isPending||f.isPending,error:r.error||null,actionError:d.error||m.error||f.error||null,pauseAutomation:d.mutate,resumeAutomation:m.mutate,deleteAutomation:f.mutate,refetch:r.refetch}}var JN=["outbound-delivery","preferences"],XN=["outbound-delivery","targets"];function ZN(){let e=X(),t=H({queryKey:JN,queryFn:zx}),a=H({queryKey:XN,queryFn:qx}),n=G({mutationFn:({finalReplyTargetId:i})=>Ix({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(JN,i),e.invalidateQueries({queryKey:XN})}}),r=p.default.useMemo(()=>a.data?.targets??[],[a.data]),s=p.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function WN(){let e=R(),[t,a]=p.default.useState("all"),[n,r]=p.default.useState(null),i=YN(t==="completed"),o=ZN(),[u,c]=p.default.useState(!1),d=p.default.useRef(null);p.default.useEffect(()=>()=>clearTimeout(d.current),[]);let m=p.default.useCallback(()=>{c(!0),clearTimeout(d.current),d.current=setTimeout(()=>c(!1),1e3),i.refetch()},[i.refetch]),f=i.isRefreshing||u,h=i.error&&!i.isLoading&&i.automations.length===0;return p.default.useEffect(()=>{if(!i.automations.length){r(null);return}i.automations.some(y=>y.automation_id===n)||r(i.automations[0].automation_id)},[i.automations,n]),l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          ${i.error&&l`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${e("automations.error.loadFailed")}
            </div>
          `}
          ${i.actionError&&l`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${i.actionError.message}
            </div>
          `}

          ${h?null:l`
                ${!i.isLoading&&!i.schedulerEnabled&&l`
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
                <${VN}
                  summary=${i.summary}
                  activeFilter=${t}
                  onSelectFilter=${a}
                />
                <${AN} deliveryState=${o} />

                ${i.isLoading?l`
                      <div className="space-y-4">
                        ${[1,2,3].map(x=>l`<div
                              key=${x}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`)}
                      </div>
                    `:l`
                      <${QN}
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
  `}var e_={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function t_({result:e,onDismiss:t}){return p.default.useEffect(()=>{if(!e)return;let a=setTimeout(t,4e3);return()=>clearTimeout(a)},[e,t]),e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",e_[e.type]||e_.info].join(" ")}>
      <${M}
        name=${e.type==="success"?"check":e.type==="error"?"close":"bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">
        <${M} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `:null}var n_="/api/webchat/v2/channels/slack/setup";function r_(){return Q(n_)}function s_(e){let t={installation_id:String(e.installation_id||"").trim(),team_id:String(e.team_id||"").trim(),api_app_id:String(e.api_app_id||"").trim(),user_id:a_(e.user_id),shared_subject_user_id:a_(e.shared_subject_user_id)},a=String(e.bot_token||"").trim(),n=String(e.signing_secret||"").trim();return a&&(t.bot_token=a),n&&(t.signing_secret=n),Q(n_,{method:"PUT",body:JSON.stringify(t)})}function Ph(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function a_(e){let t=String(e||"").trim();return t||null}var i_="/api/webchat/v2/channels/slack/allowed",rM="/api/webchat/v2/channels/slack/subjects";function o_(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function l_(){return Q(i_)}function u_(){return Q(rM)}function c_(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return Q(i_,{method:"PUT",body:JSON.stringify(n)})}function d_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var m_=["slack-allowed-channels"];function p_({action:e}){let t=R(),a=X(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,u]=p.default.useState([]),c=iM(e,t),d=H({queryKey:m_,queryFn:l_}),m=H({queryKey:["slack-routable-subjects"],queryFn:u_}),f=m.data?.subjects||[],h=f_(f),x=m.isSuccess||m.isError,y=f.length>0;p.default.useEffect(()=>{d.data&&u(Uh(d.data.channels||[]))},[d.data]);let $=G({mutationFn:({channels:E})=>c_(E),onSuccess:E=>{u(Uh(E.channels||[])),a.invalidateQueries({queryKey:m_}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let E=n.trim();!E||!m.isSuccess||(u(k=>Uh([...k,{channel_id:E,subject_user_id:s}])),r(""))},v=E=>{u(k=>k.filter(C=>C.channel_id!==E))},b=(E,k)=>{u(C=>C.map(O=>O.channel_id===E?{...O,subject_user_id:k}:O))},w=()=>{$.mutate({channels:sM(o)})},S=m.isError&&o.some(E=>!E.subject_user_id);return l`
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
        ${d.data?.team_id&&l`<span className="shrink-0 rounded-md border border-white/[0.08] px-2 py-1 font-mono text-[10px] text-iron-500">
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
          ${!y&&l`<option value="">${c.noSubjectsLabel}</option>`}
          ${y&&l`<option value="">${c.autoSubjectLabel}</option>`}
          ${h.map(E=>l`
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
        ${d.isLoading&&l`<div className="px-3 py-2 text-xs text-iron-400">${c.loadingMessage}</div>`}
        ${!d.isLoading&&o.length===0&&l`<div className="px-3 py-2 text-xs text-iron-500">
          ${c.emptyMessage}
        </div>`}
        ${o.map(E=>l`
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
                ${y?l`
                    <select
                      value=${E.subject_user_id}
                      onChange=${k=>b(E.channel_id,k.target.value)}
                      className="h-8 rounded-md border border-white/10 bg-white/[0.04] px-2 text-xs text-iron-100 outline-none focus:border-signal/45"
                    >
                      <option value="">${c.autoSubjectLabel}</option>
                      ${f_(f,E).map(k=>l`
                          <option key=${k.subject_user_id} value=${k.subject_user_id}>
                            ${k.display_name}
                          </option>
                        `)}
                    </select>
                  `:l`<span className="max-w-40 truncate text-xs text-iron-500">
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
        ${$.isSuccess&&l`<p className="text-xs text-emerald-300">
          ${c.successMessage}
        </p>`}
        ${(d.isError||m.isError||$.isError)&&l`<p className="text-xs text-red-300">
          ${d_($.error||d.error||m.error,c.errorMessage)}
        </p>`}
      </div>
    </div>
  `}function f_(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function Uh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return o_(Array.from(t.keys())).map(a=>t.get(a))}function sM(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function iM(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var jh=["slack-setup"],Yr={installationId:{body:"Local IronClaw name for this Slack install. Choose one and keep it stable.",example:"Example: local-slack"},teamId:{body:"Slack workspace/team ID from the workspace that installed the app.",example:"Example: T0123456789"},appId:{body:"Slack app Basic Information > App Credentials.",example:"Example: A0123456789"},botUser:{body:"Optional Reborn user. Blank uses the current WebUI operator.",example:"Example: user:operator"},sharedSubject:{body:"Optional default team agent for shared channel turns. Usually blank.",example:"Example: user:slack-shared"},botToken:{body:"Slack app OAuth & Permissions > Bot User OAuth Token.",example:"Example: xoxb-..."},signingSecret:{body:"Slack app Basic Information > App Credentials > Signing Secret.",example:""}};function g_({action:e}){let t=H({queryKey:jh,queryFn:r_}),a=t.data?.configured===!0;return l`
    <div className="space-y-3">
      <${oM} action=${e} setupQuery=${t} />
      ${a&&l`<${p_} action=${e} />`}
    </div>
  `}function oM({action:e,setupQuery:t}){let a=X(),[n,r]=p.default.useState(lM()),s=p.default.useRef(!1),i=p.default.useRef(!1),o=t.data,u=uM(e);p.default.useEffect(()=>{!o||s.current||i.current||(r(h_(o)),s.current=!0)},[o]);let c=G({mutationFn:s_,onSuccess:h=>{i.current=!1,r(h_(h)),s.current=!0,a.setQueryData(jh,h),a.invalidateQueries({queryKey:jh}),a.invalidateQueries({queryKey:["slack-allowed-channels"]}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["extensions"]})}}),d=h=>x=>{i.current=!0,r(y=>({...y,[h]:x.target.value}))},m=()=>c.mutate(n),f=n.installation_id.trim()&&n.team_id.trim()&&n.api_app_id.trim()&&(o?.bot_token_configured||n.bot_token.trim())&&(o?.signing_secret_configured||n.signing_secret.trim());return l`
    <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h4 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${u.title}
          </h4>
          <p className="mt-2 text-xs leading-5 text-iron-300">
            ${u.instructions}
          </p>
        </div>
        ${o?.configured&&l`<span className="shrink-0 rounded-md border border-emerald-400/20 px-2 py-1 text-[10px] text-emerald-300">
          Configured
        </span>`}
      </div>

      <div className="grid gap-3 sm:grid-cols-3">
        ${pl("Installation ID",n.installation_id,d("installation_id"),"",Yr.installationId)}
        ${pl("Team ID",n.team_id,d("team_id"),"",Yr.teamId)}
        ${pl("App ID",n.api_app_id,d("api_app_id"),"",Yr.appId)}
        ${pl("Bot user",n.user_id,d("user_id"),"default operator",Yr.botUser)}
        ${pl("Shared subject",n.shared_subject_user_id,d("shared_subject_user_id"),"optional",Yr.sharedSubject)}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        ${v_("Bot token",n.bot_token,d("bot_token"),o?.bot_token_configured,Yr.botToken)}
        ${v_("Signing secret",n.signing_secret,d("signing_secret"),o?.signing_secret_configured,Yr.signingSecret)}
      </div>

      <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <${A}
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick=${m}
          disabled=${!f||c.isPending}
        >
          ${c.isPending?"Saving...":u.submitLabel}
        <//>
        ${t.isError&&l`<p className="text-xs text-red-300">
          ${Ph(t.error,u.errorMessage)}
        </p>`}
        ${c.isError&&l`<p className="text-xs text-red-300">
          ${Ph(c.error,u.errorMessage)}
        </p>`}
        ${c.isSuccess&&l`<p className="text-xs text-emerald-300">${u.successMessage}</p>`}
      </div>
    </div>
  `}function h_(e){return{installation_id:e.installation_id||"",team_id:e.team_id||"",api_app_id:e.api_app_id||"",user_id:e.user_id||"",shared_subject_user_id:e.shared_subject_user_id||"",bot_token:"",signing_secret:""}}function lM(){return{installation_id:"",team_id:"",api_app_id:"",user_id:"",shared_subject_user_id:"",bot_token:"",signing_secret:""}}function pl(e,t,a,n="",r=null){return l`
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-iron-500">${e}</span>
      <input
        type="text"
        value=${t}
        onChange=${a}
        placeholder=${n}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <${y_} help=${r} />
    </label>
  `}function v_(e,t,a,n,r=null){return l`
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
      <${y_} help=${r} />
    </label>
  `}function y_({help:e}){return e?l`
    <p className="mt-1.5 min-h-8 text-[11px] leading-4 text-iron-400">
      <span className="block">${e.body}</span>
      ${e.example&&l`<span className="mt-0.5 block font-mono text-iron-300">${e.example}</span>`}
    </p>
  `:null}function uM(e){return{title:"Slack setup",instructions:e?.instructions||"Configure the Slack app before assigning channels.",submitLabel:"Save setup",successMessage:"Slack setup saved.",errorMessage:"Slack setup update failed."}}var Fh={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function fr(e){return e==="wasm_channel"||e==="channel"}var b_={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},x_={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function $_(e){let t=w_(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||fr(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function w_(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function Bh(e){let t=w_(e);return t==="active"||t==="ready"}function S_({extension:e,secrets:t=[],fields:a=[]}={}){return Bh(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var N_="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",__="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",k_="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",R_="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",C_="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",cM="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function E_(e){return e.package_ref?.id||""}function dM({actions:e,isBusy:t}){let a=R(),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>{if(!n)return;let i=o=>{s.current&&!s.current.contains(o.target)&&r(!1)};return document.addEventListener("mousedown",i),()=>document.removeEventListener("mousedown",i)},[n]),l`
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
        <${M} name="more" className="h-4 w-4" strokeWidth=${2.4} />
      </button>
      ${n&&l`
        <div
          role="menu"
          className="absolute right-0 top-8 z-10 min-w-[156px] rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-1 shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]"
        >
          ${e.map(i=>l`
              <button
                key=${i.id}
                type="button"
                role="menuitem"
                disabled=${t}
                onClick=${()=>{r(!1),i.run()}}
                className=${["flex w-full items-center gap-2.5 rounded-[7px] px-2.5 py-1.5 text-left text-[13px] disabled:cursor-not-allowed disabled:opacity-50",i.danger?"text-[var(--v2-danger-text)] hover:bg-[var(--v2-danger-soft)]":"text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)]"].join(" ")}
              >
                <${M} name=${i.icon||"settings"} className="h-3.5 w-3.5" />
                ${i.label}
              </button>
            `)}
        </div>
      `}
    </div>
  `}function T_({items:e}){return!e||e.length===0?null:l`
    <div className="mt-3 flex flex-wrap gap-1">
      ${e.map(t=>l`<span key=${t} className=${cM}>${t}</span>`)}
    </div>
  `}function bi({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=R(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=b_[i]||"muted",u=s(`extensions.state.${i}`)||x_[i]||i,c=s(`extensions.kind.${e.kind}`)||Fh[e.kind]||e.kind,d=e.display_name||E_(e),m=!!e.package_ref,f=e.tools||[],[h,x]=p.default.useState(!1),$=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],b=[],w=$_(e);w==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):w==="activate"&&v.push({id:"activate",label:"Activate",run:()=>t(g)}),m&&(e.needs_setup||e.has_auth)&&w!=="configure"&&b.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)});let S=b.some(k=>k.id==="configure");m&&w!=="configure"&&fr(e.kind)&&(i==="setup_required"||i==="failed")&&b.push({id:"setup",label:"Setup",icon:"settings",run:()=>a(g)}),m&&fr(e.kind)&&!S&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&b.push({id:"reconfigure",label:"Reconfigure",icon:"settings",run:()=>a(g)}),m&&b.push({id:"remove",label:s("common.remove")||"Remove",icon:"trash",danger:!0,run:()=>n(g)});let E=v[0];return l`
    <div className=${N_}>
      <div className="flex items-start gap-2">
        <${q} tone=${o} label=${u} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${b.length>0&&l`<${dM} actions=${b} isBusy=${r} />`}
      </div>

      <div className=${__}>
        <span>${c}</span>
        ${e.version&&l`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&l`<p className=${k_}>${e.description}</p>`}

      ${e.activation_error&&l`
        <div
          className="mt-2 rounded-[10px] border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-1.5 text-xs text-[var(--v2-danger-text)]"
        >
          ${e.activation_error}
        </div>
      `}

      ${$&&l`
        <div className="mt-2 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2 text-xs leading-5 text-[var(--v2-text-muted)]">
          ${$}
        </div>
      `}

      <div className=${R_}>
        ${f.length>0?l`
              <button
                type="button"
                aria-expanded=${h?"true":"false"}
                onClick=${()=>x(k=>!k)}
                className=${C_}
              >
                <${M} name="layers" className="h-3.5 w-3.5" />
                <span>${f.length===1?s("extensions.oneCapability"):s("extensions.pluralCapabilities",{count:f.length})}</span>
                <${M}
                  name="chevron"
                  className=${["h-3 w-3",h?"rotate-180":""].join(" ")}
                />
              </button>
            `:l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">No capabilities</span>`}
        <span className="flex-1"></span>
        ${E&&l`
          <${A} variant="secondary" size="sm" onClick=${E.run} disabled=${r}>
            ${E.label}
          <//>
        `}
      </div>

      ${h&&l`<${T_} items=${f} />`}
    </div>
  `}function Jr({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=R(),s=r(`extensions.kind.${e.kind}`)||Fh[e.kind]||e.kind,i=e.display_name||E_(e),o=!!(e.package_ref&&t),u=!!(e.needs_setup||e.has_auth||fr(e.kind)),c=e.keywords||[],[d,m]=p.default.useState(!1);return l`
    <div className=${N_}>
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

      <div className=${__}>
        <span>${s}</span>
        ${e.version&&l`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&l`<p className=${k_}>${e.description}</p>`}

      <div className=${R_}>
        ${c.length>0?l`
              <button
                type="button"
                aria-expanded=${d?"true":"false"}
                onClick=${()=>m(f=>!f)}
                className=${C_}
              >
                <${M} name="list" className="h-3.5 w-3.5" />
                <span>${c.length===1?r("extensions.oneKeyword"):r("extensions.pluralKeywords",{count:c.length})}</span>
                <${M}
                  name="chevron"
                  className=${["h-3 w-3",d?"rotate-180":""].join(" ")}
                />
              </button>
            `:l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]"></span>`}
        <span className="flex-1"></span>
        ${o&&l`
          <${A}
            variant="outline"
            size="sm"
            onClick=${()=>t({packageRef:e.package_ref,displayName:i,configureAfterInstall:u})}
            disabled=${a}
          >
            <${M} name="plus" className="mr-1.5 h-3.5 w-3.5" />
            Install
          <//>
        `}
      </div>

      ${d&&l`<${T_} items=${c} />`}
    </div>
  `}var mM="/api/webchat/v2/extensions/pairing/redeem";function A_(e,t){return Q(mM,{method:"POST",body:JSON.stringify({channel:e,code:t})}).then(a=>({success:!0,provider:a.provider,provider_user_id:a.provider_user_id}))}function D_(){return Q("/api/webchat/v2/extensions")}function M_(){return Q("/api/webchat/v2/extensions/registry")}function O_(e){return Q("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function L_(e){return Q(`/api/webchat/v2/extensions/${encodeURIComponent(hl(e))}/activate`,{method:"POST"})}function P_(e){return Q(`/api/webchat/v2/extensions/${encodeURIComponent(hl(e))}/remove`,{method:"POST"})}function U_(e){return Q(`/api/webchat/v2/extensions/${encodeURIComponent(hl(e))}/setup`)}function j_(e,t,a){return Xx(hl(e),{action:"submit",payload:{secrets:t,fields:a}})}function F_(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return Q(`/api/webchat/v2/extensions/${encodeURIComponent(hl(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function B_(){return Promise.resolve({requests:[]})}function z_(e,t){return A_(e,t)}function hl(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var fM=2e3,pM=10*60*1e3;function I_(e){try{return new URL(e).protocol==="https:"}catch{return!1}}function pd(e,t=null){return I_(e)?t&&!t.closed?(t.location.href=e,{ok:!0,popup:t}):{ok:!0,popup:window.open(e,"_blank","noopener,noreferrer")}:{ok:!1,popup:null}}function xi(e){return e?.package_ref?.id||null}function zh(e){return e?.display_name||xi(e)||""}function q_(e,t,a){return xi(t)||`${e}:${zh(t)||"unknown"}:${a}`}function hM(e,t){return e.installed!==t.installed?e.installed?-1:1:zh(e.entry||e.extension).localeCompare(zh(t.entry||t.extension))}function K_(){let e=X(),t=H({queryKey:["gateway-status-extensions"],queryFn:ei,staleTime:1e4}),a=H({queryKey:["extensions"],queryFn:D_,refetchOnMount:"always"}),n=H({queryKey:["extension-registry"],queryFn:M_,refetchOnMount:"always"}),r=H({queryKey:["connectable-channels"],queryFn:zc,refetchOnMount:"always"}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["extensions"]}),e.invalidateQueries({queryKey:["extension-registry"]}),e.invalidateQueries({queryKey:["gateway-status-extensions"]}),e.invalidateQueries({queryKey:["connectable-channels"]})},[e]),[i,o]=p.default.useState(null),u=p.default.useCallback(()=>o(null),[]),c=G({mutationFn:({packageRef:T})=>O_(T),onSuccess:(T,{displayName:K,configureAfterInstall:te,onNeedsSetup:se,packageRef:ie})=>{T.success?(o({type:"success",message:T.message||T.instructions||`${K||"Extension"} installed`}),T.auth_url&&!pd(T.auth_url).ok?o({type:"error",message:"Authentication URL must use HTTPS."}):!T.auth_url&&te&&typeof se=="function"&&se({packageRef:ie,displayName:K,active:!1,activationStatus:"setup_required",onboardingState:"setup_required"})):o({type:"error",message:T.message||"Install failed"}),s()},onError:T=>{o({type:"error",message:T.message}),s()}}),d=G({mutationFn:({packageRef:T})=>L_(T),onSuccess:(T,{displayName:K})=>{T.success?(o({type:"success",message:T.message||T.instructions||`${K||"Extension"} activated`}),T.auth_url&&!pd(T.auth_url).ok&&o({type:"error",message:"Authentication URL must use HTTPS."})):T.auth_url?pd(T.auth_url).ok?o({type:"info",message:"Opening authentication\u2026"}):o({type:"error",message:"Authentication URL must use HTTPS."}):T.awaiting_token?o({type:"info",message:"Configuration required"}):o({type:"error",message:T.message||"Activation failed"}),s()},onError:T=>{o({type:"error",message:T.message})}}),m=G({mutationFn:({packageRef:T})=>P_(T),onSuccess:(T,{displayName:K})=>{T.success?o({type:"success",message:`${K||"Extension"} removed`}):o({type:"error",message:T.message||"Remove failed"}),s()},onError:T=>{o({type:"error",message:T.message})}}),f=t.data||{},h=a.data?.extensions||[],x=n.data?.entries||[],y=r.data?.channels||[],$=new Map(h.map(T=>[xi(T),T]).filter(([T])=>!!T)),g=new Set(x.map(T=>xi(T)).filter(Boolean)),v=[...x.map((T,K)=>{let te=xi(T),se=te&&$.get(te)||null;return{id:q_("registry",T,K),installed:!!(se||T.installed),entry:T,extension:se}}),...h.filter(T=>{let K=xi(T);return!K||!g.has(K)}).map((T,K)=>({id:q_("installed",T,K),installed:!0,entry:null,extension:T}))].sort(hM),b=T=>fr(T.kind),w=h.filter(b),S=h.filter(T=>T.kind==="mcp_server"),E=h.filter(T=>!b(T)&&T.kind!=="mcp_server"),k=x.filter(T=>b(T)&&!T.installed),C=x.filter(T=>T.kind==="mcp_server"&&!T.installed),O=x.filter(T=>T.kind!=="mcp_server"&&!b(T)&&!T.installed),L=a.isLoading||n.isLoading,P=c.isPending||d.isPending||m.isPending,U=p.default.useCallback(T=>{let K=T?.displayName||T?.packageRef?.id||"this extension";window.confirm(`Remove ${K}?`)&&m.mutate(T)},[m]);return{status:f,extensions:h,channels:w,mcpServers:S,tools:E,channelRegistry:k,mcpRegistry:C,toolRegistry:O,registry:x,catalogEntries:v,connectableChannels:y,isLoading:L,isBusy:P,actionResult:i,clearResult:u,install:c.mutate,activate:d.mutate,remove:U,invalidate:s}}function H_(e){let t=H({queryKey:["extension-setup",e?.id||e],queryFn:()=>U_(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function Q_(e,t){let a=X(),n=e?.id||e;return G({mutationFn:({secrets:r,fields:s})=>j_(e,r,s).then(i=>{if(i.success===!1)throw new Error(i.message||"Setup failed");return i}),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function V_(e){let t=X(),a=e?.id||e,n=p.default.useRef(null),r=p.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=p.default.useCallback(()=>{let u=t.getQueryData(["extension-setup",a]);if(u?.secrets?.length>0&&u.secrets.every(f=>f.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(f=>f.package_ref?.id===a),m=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return m==="active"||m==="ready"},[a,t]),o=p.default.useCallback(u=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||u&&u.closed||Date.now()-c>pM)&&(r(),s())},fM)},[r,s,i]);return p.default.useEffect(()=>r,[r]),G({mutationFn:({secret:u,popup:c})=>F_(e,u).then(d=>{if(d.success===!1)throw new Error(d.message||"OAuth setup failed");if(d.authorization_url&&!I_(d.authorization_url))throw new Error("Authorization URL must use HTTPS.");return{res:d,popup:c}}),onSuccess:({res:u,popup:c})=>{let d=c;u.authorization_url?d=pd(u.authorization_url,c).popup:c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(u,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function G_(e,t={}){let a=H({queryKey:["pairing",e],queryFn:()=>B_(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=X(),r=G({mutationFn:({code:s})=>z_(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function Y_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var vM={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function J_({channel:e,redeemFn:t,i18nKeys:a=vM,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=R(),o=typeof t=="function",u=G_(e,{enabled:!o}),c=X(),[d,m]=p.default.useState(""),f=gM(i,a,r),h=G({mutationFn:({code:S})=>t(e,S),onSuccess:()=>{m("");for(let S of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:S})}}),x=p.default.useCallback(S=>u.approve({code:S}),[u.approve]),y=p.default.useCallback(()=>{let S=d.trim().toUpperCase();S&&(o?h.mutate({code:S}):u.approve({code:S}))},[o,d,u.approve,h]),$=o?[]:u.requests,g=o?!1:u.isLoading,v=o?h.isPending:u.isApproving,b=o?h.isSuccess?h.data:null:u.result,w=o?h.isError?h.error:null:u.error;return p.default.useEffect(()=>{b?.success&&m("")},[b?.success]),g?l`
      <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
        <div className="v2-skeleton h-3 w-24 rounded" />
      </div>
    `:l`
    <div
      data-testid="pairing-section"
      className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4"
    >
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
          data-testid="pairing-code-input"
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${A}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${y}
          disabled=${v||!d.trim()}
          data-testid="pairing-submit"
        >
          ${f.action}
        <//>
      </div>

      ${b?.success&&l`<p data-testid="pairing-success" className="mb-3 text-xs text-emerald-300">
        ${b.message||f.success}
      </p>`}
      ${b&&!b.success&&l`<p data-testid="pairing-error" className="mb-3 text-xs text-red-300">
        ${b.message||f.error}
      </p>`}
      ${w&&l`<p data-testid="pairing-error" className="mb-3 text-xs text-red-300">
        ${Y_(w,f.error)}
      </p>`}

      ${s&&$.length>0?l`
            <div className="space-y-2">
              ${$.map(S=>l`
                <div
                  key=${S.code||S.id}
                  className="flex items-center justify-between gap-3 rounded-md border border-white/[0.06] bg-white/[0.02] px-3 py-2"
                >
                  <div className="min-w-0">
                    <span className="font-mono text-sm text-iron-200">${S.code||S.id}</span>
                    ${S.label&&l`
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
          `:s&&l`<p className="text-xs text-iron-300">${i(a.empty)}</p>`}
    </div>
  `}function gM(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function hd(e){return e.package_ref?.id||""}function X_(e){return hd(e)==="slack"}function W_(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function ek(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function yM(e){let t=e||[],a=[t.find(W_),t.find(ek)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function Z_({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>W_(r)?l`<${g_} action=${r.action} />`:ek(r)?l`<${Pc} action=${r.action} />`:null).filter(Boolean);return n.length>0?l`<div className="space-y-3">${n}</div>`:null}function tk({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:u}){let c=R(),d=t||[],m=e.enabled_channels||[],f=yM(a),h=d.some(X_),x=f.length>0&&!h;return l`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          ${c("channels.builtIn")}
        </h3>
        <${$i}
          name="Web Gateway"
          description=${c("channels.webGatewayDesc")||"Browser-based chat with SSE streaming"}
          enabled=${!0}
          detail=${"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)}
        />
        <${$i}
          name="HTTP Webhook"
          description=${c("channels.httpWebhookDesc")||"Inbound webhook endpoint for external integrations"}
          enabled=${m.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${$i}
          name="CLI"
          description=${c("channels.cliDesc")||"Terminal interface with TUI or simple REPL"}
          enabled=${m.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${$i}
          name="REPL"
          description=${c("channels.replDesc")||"Minimal read-eval-print loop for testing"}
          enabled=${m.includes("repl")}
          detail="ironclaw run --repl"
        />
        ${x&&l`
          <${$i}
            name=${c("channels.slack")||"Slack"}
            description=${c("channels.slackDesc")||"Tenant app channel for DMs and app mentions"}
            enabled=${!1}
            statusLabel="setup"
            statusTone="muted"
            detail=${c("channels.slackDetail")||"Tenant Slack app install"}
          >
            <${Z_}
              slackConnectActions=${f}
            />
          </${$i}>
        `}
      </div>

      ${d.length>0&&l`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            ${c("channels.messaging")}
          </h3>
          <div className="grid grid-cols-1 gap-4">
            ${d.map(y=>l`
                <div key=${hd(y)} className="flex flex-col gap-3">
                  <${bi}
                    ext=${y}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${u}
                  />
                  ${X_(y)&&l`<${Z_}
                    slackConnectActions=${f}
                  />`}
                  ${(y.onboarding_state==="pairing_required"||y.onboarding_state==="pairing")&&l` <${J_} channel=${hd(y)} /> `}
                </div>
              `)}
          </div>
        </div>
      `}
      ${n.length>0&&l`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            ${c("channels.availableChannels")}
          </h3>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            ${n.map(y=>l`
                <${Jr}
                  key=${hd(y)}
                  entry=${y}
                  onInstall=${o}
                  isBusy=${u}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function $i({name:e,description:t,enabled:a,detail:n,children:r,statusLabel:s=a?"on":"off",statusTone:i=a?"success":"muted"}){return l`
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
          ${n&&l`<div className="mt-1 font-mono text-[11px] text-iron-700">
            ${n}
          </div>`}
        </div>
      </div>
      ${r}
    </div>
  `}function ak({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=R(),s=e?.displayName||e?.packageRef?.id||"Extension",{secrets:i=[],fields:o=[],onboarding:u,isLoading:c,error:d}=H_(e?.packageRef),[m,f]=p.default.useState({}),[h,x]=p.default.useState({}),y=V_(e?.packageRef),$=Q_(e?.packageRef,C=>{C.success!==!1&&(n&&n(C),a())}),g=p.default.useCallback(()=>{let C={};for(let[O,L]of Object.entries(m)){let P=(L||"").trim();P&&(C[O]=P)}$.mutate({secrets:C,fields:h})},[m,h,$]),v=p.default.useCallback(C=>{let O=window.open("about:blank","_blank","width=600,height=600");O&&(O.opener=null),y.mutate({secret:C,popup:O})},[y]),w=i.filter(C=>(C.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,S=Bh(e),E=S_({extension:e,secrets:i,fields:o}),k=bM(u?.setup_url);return c?l`
      <${vd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(C=>l`<div
                key=${C}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?l`
      <${vd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")||"Failed to load setup:"} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?l`
      <${vd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")||"No configuration required for this extension."}
        </p>
      <//>
    `:l`
    <${vd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
      ${u?.credential_instructions&&l`
        <p className="mb-4 text-sm leading-6 text-iron-300">
          ${u.credential_instructions}
        </p>
      `}
      ${k&&l`
        <a
          href=${k}
          target="_blank"
          rel="noopener noreferrer"
          className="mb-4 inline-flex items-center gap-1.5 text-sm text-signal hover:underline"
        >
          Get credentials
          <${M} name="bolt" className="h-3.5 w-3.5" />
        </a>
      `}

      <div className="space-y-4">
        ${i.map(C=>l`
            <div key=${C.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                ${C.prompt||C.name}
                ${C.optional&&l`
                  <span className="font-mono text-[10px] text-iron-700"
                    >${r("common.optional")||"optional"}</span
                  >
                `}
                ${C.provided&&l`
                  <span className="font-mono text-[10px] text-mint"
                    >${r("common.configured")||"configured"}</span
                  >
                `}
              </label>
              ${(C.setup?.kind||"manual_token")==="oauth"?l`
                    <div className="flex items-center justify-between gap-3 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2">
                      <span className="text-xs text-iron-300">
                        ${C.provided?r("extensions.authConfigured")||"Authorization is configured.":r("extensions.authPopup")||"Authorize this provider in a browser popup."}
                      </span>
                      <${A}
                        variant=${C.provided?"secondary":"primary"}
                        onClick=${()=>v(C)}
                        disabled=${y.isPending}
                      >
                        ${y.isPending?r("extensions.opening"):C.provided?r("extensions.reconnect"):r("extensions.authorize")}
                      <//>
                    </div>
                  `:l`
              <input
                type="password"
                placeholder=${C.provided?"\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (leave blank to keep)":""}
                value=${m[C.name]||""}
                onChange=${O=>f(L=>({...L,[C.name]:O.target.value}))}
                onKeyDown=${O=>O.key==="Enter"&&g()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
              ${C.auto_generate&&!C.provided&&l`
                <p className="mt-1 text-xs text-iron-700">
                  ${r("extensions.autoGenerated")||"Auto-generated if left blank"}
                </p>
              `}
                  `}
            </div>
          `)}
        ${o.map(C=>l`
            <div key=${C.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                ${C.prompt||C.name}
                ${C.optional&&l`
                  <span className="font-mono text-[10px] text-iron-700"
                    >${r("common.optional")||"optional"}</span
                  >
                `}
              </label>
              <input
                type="text"
                placeholder=${C.placeholder||""}
                value=${h[C.name]||""}
                onChange=${O=>x(L=>({...L,[C.name]:O.target.value}))}
                onKeyDown=${O=>O.key==="Enter"&&g()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
            </div>
          `)}
      </div>

      ${u?.credential_next_step&&l`
        <p className="mt-4 text-xs leading-5 text-iron-300">
          ${u.credential_next_step}
        </p>
      `}
      ${S&&l`
        <div
          className="mt-4 rounded-md border border-mint/20 bg-mint/10 px-3 py-2 text-xs text-mint"
        >
          ${r("extensions.activeConfigured")}
        </div>
      `}
      ${$.error&&l`
        <div
          className="mt-4 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
        >
          ${$.error.message}
        </div>
      `}
      ${y.error&&l`
        <div
          className="mt-4 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
        >
          ${y.error.message}
        </div>
      `}

      <div className="mt-6 flex items-center justify-end gap-3">
        <${A} variant="ghost" onClick=${a}>${r("common.cancel")||"Cancel"}<//>
        ${E&&l`
        <${A}
          variant="primary"
          onClick=${()=>t?.(e)}
        >
          Activate
        <//>
        `}
        ${w&&l`
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
  `}function bM(e){if(!e)return null;try{let t=new URL(String(e));return t.protocol==="https:"?t.href:null}catch{return null}}function vd({onClose:e,title:t,children:a}){let n=p.default.useId();return p.default.useEffect(()=>{let r=s=>{s.key==="Escape"&&e()};return window.addEventListener("keydown",r),()=>window.removeEventListener("keydown",r)},[e]),l`
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick=${r=>{r.target===r.currentTarget&&e()}}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby=${n}
        className="v2-panel mx-4 w-full max-w-lg rounded-2xl p-6"
        onClick=${r=>r.stopPropagation()}
      >
        <div className="mb-5 flex items-center justify-between">
          <h3 id=${n} className="text-lg font-semibold text-white">${t}</h3>
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
  `}function nk(e){return e.package_ref?.id||""}function rk({mcpServers:e,mcpRegistry:t,onActivate:a,onConfigure:n,onRemove:r,onInstall:s,isBusy:i}){let o=R();return e.length===0&&t.length===0?l`
      <div className="v2-panel rounded-[18px] p-6 sm:p-8">
        <h3 className="text-lg font-semibold text-white">${o("extensions.emptyMcpTitle")}</h3>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${o("extensions.emptyMcpDesc")}
        </p>
      </div>
    `:l`
    <div className="space-y-5">
      ${e.length>0&&l`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            ${o("mcp.installed")}
          </h3>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            ${e.map(u=>l`
                <${bi}
                  key=${nk(u)}
                  ext=${u}
                  onActivate=${a}
                  onConfigure=${n}
                  onRemove=${r}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
      ${t.length>0&&l`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            Available MCP servers
          </h3>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            ${t.map(u=>l`
                <${Jr}
                  key=${nk(u)}
                  entry=${u}
                  onInstall=${s}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function xM(e){return e?.package_ref?.id||""}function $M(e){return e.entry||e.extension||{}}function sk({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=R(),[o,u]=p.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(y=>{let $=$M(y);return($.display_name||xM($)).toLowerCase().includes(c)||($.description||"").toLowerCase().includes(c)||($.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,m=d.filter(y=>y.installed&&y.extension),f=d.filter(y=>y.installed&&!y.extension&&y.entry),h=m.length+f.length,x=d.filter(y=>!y.installed&&y.entry);return e.length===0?l`
      <div className="v2-panel rounded-[18px] p-6 sm:p-8">
        <h3 className="text-lg font-semibold text-white">
          ${i("ext.registry.emptyTitle")}
        </h3>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${i("ext.registry.emptyDesc")}
        </p>
      </div>
    `:l`
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <input
          type="text"
          value=${o}
          onChange=${y=>u(y.target.value)}
          placeholder=${i("ext.registry.searchPlaceholder")}
          className="h-9 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <span className="font-mono text-[11px] text-iron-700">
          ${d.length} / ${e.length}
        </span>
      </div>

      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        ${d.length===0?l`<p className="py-4 text-sm text-iron-300">
              ${i("ext.registry.noMatch")}
            </p>`:l`
              ${h>0&&l`
                <h3
                  className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
                >
                  ${i("extensions.installed")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${m.map(y=>l`
                      <${bi}
                        key=${y.id}
                        ext=${y.extension||y.entry}
                        onActivate=${a}
                        onConfigure=${n}
                        onRemove=${r}
                        isBusy=${s}
                      />
                    `)}
                  ${f.map(y=>l`
                      <${Jr}
                        key=${y.id}
                        entry=${y.entry}
                        statusLabel=${i("extensions.installed")}
                        isBusy=${s}
                      />
                    `)}
                </div>
              `}

              ${x.length>0&&l`
                <h3
                  className=${["mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal",h>0?"mt-6":""].join(" ")}
                >
                  ${i("ext.registry.availableTitle")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${x.map(y=>l`
                      <${Jr}
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
  `}function qh(){let{tab:e="registry"}=st(),[t,a]=p.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:u,connectableChannels:c,isLoading:d,isBusy:m,actionResult:f,clearResult:h,install:x,activate:y,remove:$,invalidate:g}=K_(),v=p.default.useCallback(C=>a(C),[]),b=p.default.useCallback(C=>x({...C,onNeedsSetup:v}),[v,x]),w=p.default.useCallback(()=>a(null),[]),S=p.default.useCallback(()=>g(),[g]),E=p.default.useCallback(C=>{C&&(y(C),a(null))},[y]);if(d)return l`
      <div className="flex h-full flex-col overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${[1,2,3].map(C=>l`
                <div
                  key=${C}
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
    `;if(e==="installed")return l`<${it} to="/extensions/registry" replace />`;let k={channels:l`<${tk}
      status=${n}
      channels=${r}
      connectableChannels=${c}
      channelRegistry=${i}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${b}
      isBusy=${m}
    />`,mcp:l`<${rk}
      mcpServers=${s}
      mcpRegistry=${o}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${b}
      isBusy=${m}
    />`,registry:l`<${sk}
      catalogEntries=${u}
      onInstall=${b}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      isBusy=${m}
    />`};return k[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <${t_} result=${f} onDismiss=${h} />
          ${k[e]}
        </div>
      </div>

      ${t&&l`
        <${ak}
          extension=${t}
          onActivate=${E}
          onClose=${w}
          onSaved=${S}
        />
      `}
    </div>
  `:l`<${it} to="/extensions/registry" replace />`}var ik=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],ok=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],lk=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],Ih=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function uk(e){return String(e||"").trim().toLowerCase()}function ck(e){if(e==null)return"";if(Array.isArray(e))return e.map(ck).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function tt(e,t){let a=uk(e);return a?t.map(ck).join(" ").toLowerCase().includes(a):!0}function wi(e,t,a,n){let r=uk(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(u=>tt(r,[i,u.key,u.labelKey?n(u.labelKey):u.label,u.descKey?n(u.descKey):u.description,t[u.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function wM({visible:e}){let t=R();return e?l`
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      ${t("tools.saved")}
    </span>
  `:null}function SM({checked:e,onChange:t,label:a}){return l`
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
  `}function NM({field:e,value:t,onSave:a,isSaved:n}){let r=R(),[s,i]=p.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",u=e.descKey?r(e.descKey):e.description||"";p.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=p.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let m=parseInt(d,10);isNaN(m)||a(e.key,m)}else if(e.type==="float"){let m=parseFloat(d);isNaN(m)||a(e.key,m)}else a(e.key,d)},[e.key,e.type,a]);return l`
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">${o}</div>
        ${u&&l`<div className="mt-1 text-xs leading-5 text-iron-300">${u}</div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${e.type==="boolean"?l`
              <${SM}
                checked=${t===!0||t==="true"}
                onChange=${d=>a(e.key,d?"true":"false")}
                label=${o}
              />
            `:e.type==="select"?l`
              <select
                value=${s}
                onChange=${d=>{i(d.target.value),c(d.target.value)}}
                aria-label=${o}
                className="v2-select h-9 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
              >
                <option value="">${r("tools.default")}</option>
                ${e.options.map(d=>l`<option key=${d} value=${d}>${d}</option>`)}
              </select>
            `:l`
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
  `}function Si({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=R(),o=t?i(t):e||"";return l`
    <${ae} className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${o}</h3>
      <div>
        ${a.map(u=>l`
              <${NM}
                key=${u.key}
                field=${u}
                value=${n[u.key]}
                onSave=${r}
                isSaved=${s[u.key]}
              />
            `)}
      </div>
    <//>
  `}function wt({query:e}){let t=R();return l`
    <${ae} padding="lg">
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
  `}function dk({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=R();if(n)return l`<${_M} />`;let i=wi(ok,e,r,s);return i.length===0?l`<${wt} query=${r} />`:l`
    <div className="space-y-5">
      ${i.map(o=>l`
            <${Si}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function _M(){return l`
    <div className="space-y-5">
      ${[1,2,3].map(e=>l`
            <${ae} key=${e} padding="md">
              <div className="mb-4 h-3 w-20 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              ${[1,2,3,4].map(t=>l`
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
  `}function mk(){let e=H({queryKey:["gateway-status-settings"],queryFn:ei,staleTime:1e4}),t=H({queryKey:["extensions"],queryFn:H$}),a=H({queryKey:["extension-registry"],queryFn:Q$}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(m=>m.kind==="wasm_channel"||m.kind==="channel"),o=s.filter(m=>(m.kind==="wasm_channel"||m.kind==="channel")&&!m.installed),u=r.filter(m=>m.kind==="mcp_server"),c=s.filter(m=>m.kind==="mcp_server"&&!m.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:u,mcpRegistry:c,extensions:r,isLoading:d}}function kM({name:e,description:t,enabled:a,detail:n}){let r=R();return l`
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
        ${n&&l`<div className="mt-1 font-mono text-[11px] text-[var(--v2-text-faint)]">
          ${n}
        </div>`}
      </div>
    </div>
  `}function fk({channel:e,registryEntry:t}){let a=R(),n=t?.display_name||e?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},u={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return l`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${n}</span>
          ${s?l`<${q}
                tone=${o[i]||"muted"}
                label=${u[i]||i}
                size="sm"
              />`:l`<${q}
                tone="muted"
                label=${a("channels.available")}
                size="sm"
              />`}
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${r}</div>
      </div>
    </div>
  `}function RM(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function CM({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=RM(e,i).filter(x=>tt(s,[i("channels.builtIn"),x.id,x.name,x.description,x.detail])),u=new Set(t.map(x=>x.name)),c=t.filter(x=>tt(s,[i("channels.messaging"),x.name,x.display_name,x.description,x.onboarding_state])),d=a.filter(x=>!u.has(x.name)).filter(x=>tt(s,[i("channels.messaging"),x.name,x.display_name,x.description])),m=new Set(n.map(x=>x.name)),f=n.filter(x=>tt(s,[i("channels.mcpServers"),x.name,x.display_name,x.description,x.active?i("channels.active"):i("channels.inactive")])),h=r.filter(x=>!m.has(x.name)).filter(x=>tt(s,[i("channels.mcpServers"),x.name,x.display_name,x.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:h}}function pk({searchQuery:e=""}){let t=R(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=mk();if(o)return l`
      <div className="space-y-5">
        <${ae} padding="md">
          <div className="mb-4 h-3 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(h=>l`
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
    `;let{builtInChannels:u,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:f}=CM({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return u.length===0&&c.length===0&&d.length===0&&m.length===0&&f.length===0?l`<${wt} query=${e} />`:l`
    <div className="space-y-5">
      ${u.length>0&&l`
      <${ae} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${u.map(h=>l`
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

      ${(c.length>0||d.length>0)&&l`
        <${ae} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.messaging")}
          </h3>
          ${c.map(h=>l`
              <${fk}
                key=${h.name}
                channel=${h}
                registryEntry=${r.find(x=>x.name===h.name)}
              />
            `)}
          ${d.map(h=>l`
              <${fk} key=${h.name} registryEntry=${h} />
            `)}
        <//>
      `}
      ${(m.length>0||f.length>0)&&l`
        <${ae} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.mcpServers")}
          </h3>
          ${m.map(h=>l`
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
          ${f.map(h=>l`
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
  `}function hk({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:u,onNearaiWallet:c,onCodexLogin:d,loginBusy:m}){let f=R(),h=e.id===t,x=Hr(e,n),y=ni(e,n),$=sw(e,n,t,a),g=Nc(e,n),v=iw(e),b=f(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[w,S]=p.default.useState(h),E=p.default.useCallback(()=>S(Ye=>!Ye),[]);p.default.useEffect(()=>{S(h)},[h]);let k=x?l`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${al(e.adapter)} · ${$||e.default_model||f("llm.none")}
      </span>`:l`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${b}
      </span>`,C=e.id==="nearai"||e.id==="openai_codex",O=e.api_key_set===!0||e.has_api_key===!0,L=e.builtin?e.id==="nearai"&&v&&!O?f("llm.addApiKey"):f("llm.configure"):f("common.edit"),P=v&&e.builtin?l`
          <${A}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${r}
            onClick=${()=>i(e)}
          >
            ${L}
          <//>
        `:null,U=!h&&e.id==="nearai"?l`
          ${P}
          <${A} type="button" variant="secondary" size="sm" disabled=${m} onClick=${c}>
            ${f("onboarding.nearWallet")}
          <//>
          <${A} type="button" variant="secondary" size="sm" disabled=${m} onClick=${()=>u("github")}>
            GitHub
          <//>
          <${A} type="button" variant="secondary" size="sm" disabled=${m} onClick=${()=>u("google")}>
            Google
          <//>
        `:!h&&e.id==="openai_codex"?l`
          <${A} type="button" variant="secondary" size="sm" disabled=${m} onClick=${d}>
            ${f("onboarding.codexSignIn")}
          <//>
        `:null,K=!h&&x&&(!C||e.id==="nearai"&&e.has_api_key===!0)?l`
        <${A}
          type="button"
          variant="primary"
          size="sm"
          disabled=${r}
          onClick=${()=>s(e)}
        >
          ${f("llm.use")}
        <//>
      `:null,te=x?null:l`
        <${A}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${r}
          onClick=${()=>i(e)}
        >
          ${f(g==="api_key"?"llm.addApiKey":"llm.configure")}
        <//>
      `,se=h?null:K||(C?U:te),ie=!C&&(e.builtin&&e.id!=="bedrock"||!e.builtin)||e.id==="nearai"&&v;return l`
    <${ae}
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
            ${h&&l`<${q} tone="positive" label=${f("llm.active")} size="sm" />`}
            ${e.builtin&&!h&&l`<${q} tone="muted" label=${f("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${k}</span>
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

      ${w&&l`
        <div data-testid="llm-provider-details" className="border-t border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-4 sm:px-5">
          <div className="grid gap-3 text-xs text-[var(--v2-text-muted)] sm:grid-cols-3">
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${f("llm.adapter")}</div>
              <div className="mt-1 truncate">${al(e.adapter)}</div>
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
            ${ie&&l`
              <${A}
                type="button"
                variant="secondary"
                size="sm"
                disabled=${r}
                onClick=${()=>i(e)}
              >
                ${L}
              <//>
            `}
            ${!e.builtin&&!h&&l`
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
  `}var EM=[{key:"active",labelKey:"llm.groupActive",dotClass:"bg-[var(--v2-positive-text)]"},{key:"ready",labelKey:"llm.groupReady",dotClass:"bg-[var(--v2-accent)]"},{key:"setup",labelKey:"llm.groupSetup",dotClass:"bg-[var(--v2-warning-text)]"}];function TM({label:e,count:t,dotClass:a}){return l`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full "+a} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${t}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `}function vk({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=R(),r=Yc({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=Jc(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return l`<${wt} query=${a} />`;let u=ow(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return l`
    <${ae} className="p-4 sm:p-6">
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

      ${r.message&&l`
        <div
          className=${["mb-4 rounded-md border px-3 py-2 text-sm",r.message.tone==="error"?"border-red-400/30 bg-red-500/10 text-red-200":"border-mint/30 bg-mint/10 text-mint"].join(" ")}
          role="status"
        >
          ${r.message.text}
        </div>
      `}

      <${Gc} login=${i} />

      ${s.isLoading?l`<div className="text-sm text-[var(--v2-text-muted)]">${n("common.loading")}</div>`:s.error?l`<div className="text-sm text-red-200">${n("error.loadFailed",{what:n("llm.providers"),message:s.error.message})}</div>`:l`
            <div className="space-y-1">
              ${EM.flatMap(c=>{let d=u[c.key];return d.length?[l`
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
                      ${d.map(m=>l`
                          <${hk}
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

      <${Vc}
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
  `}function gk({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=R(),{activeProviderId:o,selectedModel:u,providers:c,hasActiveProvider:d}=ri({settings:e,gatewayStatus:t});if(r)return l`<${AM} />`;let m=d?o:"",f=c.find(g=>g.id===o),h=d&&(u||f?.default_model||e.selected_model)||"",x=wi(ik,e,s,i),y=tt(s,[i("inference.provider"),i("inference.backend"),m,i("inference.model"),h]),$=tt(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!y&&!$&&x.length===0?l`<${wt} query=${s} />`:l`
    <div className="space-y-5">
      ${y&&l`
      <${ae} padding="none" className="p-4 sm:p-5">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${i("inference.provider")}</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${i("inference.backend")}</div>
            <div className="mt-1 flex items-center gap-2">
              <span className="font-mono text-lg font-semibold text-[var(--v2-text-strong)]">${m||i("inference.none")}</span>
              ${d?l`<${q} tone="positive" label=${i("inference.active")} size="sm" />`:l`<${q} tone="muted" label=${i("llm.notConfigured")} size="sm" />`}
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

      ${$&&l`
        <${vk}
          settings=${e}
          gatewayStatus=${t}
          searchQuery=${s}
        />
      `}

      ${x.map(g=>l`
            <${Si}
              key=${g.groupKey}
              groupKey=${g.groupKey}
              fields=${g.fields}
              settings=${e}
              onSave=${a}
              savedKeys=${n}
            />
          `)}
    </div>
  `}function pr({className:e=""}){return l`
    <div
      className=${"rounded animate-pulse bg-[var(--v2-surface-muted)] "+e}
    />
  `}function AM(){return l`
    <div className="space-y-5">
      <${ae} padding="md">
        <${pr} className="mb-4 h-3 w-24" />
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${pr} className="h-3 w-16" />
            <${pr} className="mt-2 h-6 w-28" />
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${pr} className="h-3 w-16" />
            <${pr} className="mt-2 h-6 w-40" />
          </div>
        </div>
      <//>
      ${[1,2].map(e=>l`
            <${ae} key=${e} padding="md">
              <${pr} className="mb-4 h-3 w-20" />
              ${[1,2,3].map(t=>l`
                    <div key=${t} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                      <${pr} className="h-4 w-32" />
                      <${pr} className="h-9 w-36" />
                    </div>
                  `)}
            <//>
          `)}
    </div>
  `}function yk({searchQuery:e=""}){let t=R(),{lang:a,setLang:n}=vl(),r=gl.find(i=>i.code===a)||gl[0],s=gl.filter(i=>tt(e,[i.code,i.name,i.native]));return s.length===0?l`<${wt} query=${e} />`:l`
    <${ae} padding="md">
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
        ${s.map(i=>l`
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
  `}function bk({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=R();if(n)return l`
      <div className="space-y-5">
        ${[1,2].map(o=>l`
              <${ae} key=${o} padding="md">
                <div className="mb-4 h-3 w-20 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                ${[1,2].map(u=>l`
                      <div key=${u} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                        <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                        <div className="h-9 w-36 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                      </div>
                    `)}
              <//>
            `)}
      </div>
    `;let i=wi(lk,e,r,s);return i.length===0?l`<${wt} query=${r} />`:l`
    <div className="space-y-5">
      ${i.map(o=>l`
            <${Si}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function xk(){let e=R(),[t,a]=p.default.useState(!1),n=p.default.useCallback(()=>a(!0),[]),r=p.default.useCallback(()=>a(!1),[]),s=p.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function $k({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=R(),r=xk({gatewayStatus:t,gatewayStatusQuery:a});return e?l`
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
            ${!r.restartEnabled&&l`
              <p className="mt-1 text-xs text-[var(--v2-text-muted)]">
                ${r.unavailableReason}
              </p>
            `}
            ${r.isRestarting&&l`
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

      ${r.error&&l`
        <div className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
          ${r.error}
        </div>
      `}

      ${r.message&&l`
        <div className="rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200">
          ${r.message}
        </div>
      `}
    </div>

    <${di}
      open=${r.confirmOpen}
      onClose=${r.closeConfirm}
      title=${n("restart.title")}
      size="sm"
    >
      <${mi} className="space-y-3">
        <p className="text-sm text-[var(--v2-text)]">
          ${n("restart.description")}
        </p>
        <div className="rounded-xl border border-copper/25 bg-copper/10 px-3 py-2 text-xs text-copper">
          ${n("restart.warning")}
        </div>
      <//>
      <${fi}>
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

    ${r.isRestarting&&l`
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
  `:null}function wk(){let e=X(),t=H({queryKey:["skills"],queryFn:V$}),a=G({mutationFn:Y$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),n=G({mutationFn:X$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),r=G({mutationFn:({name:c,content:d})=>J$(c,{content:d}),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),s=G({mutationFn:({name:c,enabled:d})=>Z$(c,d),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),i=G({mutationFn:c=>W$(c),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),o=t.data?.skills||[],u=t.data?.auto_activate_learned!==!1;return{skills:o,query:t,autoActivateLearned:u,fetchSkillContent:G$,installSkill:a.mutateAsync,removeSkill:n.mutateAsync,updateSkill:r.mutateAsync,setSkillAutoActivate:s.mutateAsync,setAutoActivateLearned:i.mutateAsync,isInstalling:a.isPending,isRemoving:n.isPending,isUpdating:r.isPending,isSettingAutoActivate:s.isPending,isSettingAutoActivateLearned:i.isPending}}function Sk({skill:e,onEdit:t,onRemove:a,onUpdate:n,onSetAutoActivate:r,isRemoving:s,isUpdating:i,isSettingAutoActivate:o}){let u=R(),c=e.name||e.id,d=e.trust||e.trust_level||"installed",m=e.source_kind||"installed",f=!!e.can_edit,h=!!e.can_delete,x=e.auto_activate!==!1,[y,$]=p.default.useState(!1),[g,v]=p.default.useState(""),[b,w]=p.default.useState(""),[S,E]=p.default.useState(!1);p.default.useEffect(()=>{y||(v(""),w(""))},[y]);let k=p.default.useCallback(async()=>{E(!0),w("");try{let O=await t(c);v(O?.content||""),$(!0)}catch(O){w(O.message||u("skills.contentLoadFailed"))}finally{E(!1)}},[c,t,u]),C=p.default.useCallback(async()=>{(await n(c,g))?.success&&$(!1)},[g,c,n]);return l`
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
              label=${u(`skills.source.${m}`)}
              size="sm"
            />
            ${e.version&&l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v${e.version}</span>`}
          </div>

          ${e.description&&l`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${e.description}</div>`}

          ${y?l`
                <div className="mt-3">
                  <${Oc}
                    rows=${12}
                    value=${g}
                    className="font-mono text-xs leading-5"
                    onInput=${O=>v(O.currentTarget.value)}
                  />
                </div>
              `:l`<${DM} skill=${e} />`}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          ${f&&!y&&l`
            <${A}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${i||S}
              title=${u("skills.edit")}
              onClick=${k}
            >
              <${M} name="file" className="h-4 w-4" />
              ${u(S?"skills.loading":"skills.edit")}
            <//>
          `}
          ${y&&l`
            <${A}
              type="button"
              variant="ghost"
              size="sm"
              disabled=${i}
              onClick=${()=>{v(""),$(!1)}}
            >
              <${M} name="close" className="h-4 w-4" />
              ${u("skills.cancel")}
            <//>
            <${A}
              type="button"
              variant="primary"
              size="sm"
              disabled=${i}
              onClick=${C}
            >
              <${M} name="check" className="h-4 w-4" />
              ${u(i?"skills.saving":"skills.save")}
            <//>
          `}
          ${f&&!y&&l`
            <${A}
              type="button"
              variant=${x?"secondary":"ghost"}
              size="sm"
              disabled=${o}
              title=${u(x?"skills.autoActivateOnTitle":"skills.autoActivateOffTitle")}
              onClick=${()=>r(c,!x)}
            >
              <${M} name=${x?"check":"close"} className="h-4 w-4" />
              ${u(x?"skills.autoActivateOnLabel":"skills.autoActivateOffLabel")}
            <//>
          `}
          ${h&&!y&&l`
            <${A}
              type="button"
              variant="danger"
              size="sm"
              disabled=${s}
              title=${u("skills.delete")}
              onClick=${()=>a(c)}
            >
              <${M} name="trash" className="h-4 w-4" />
              ${u("skills.delete")}
            <//>
          `}
        </div>
      </div>
      ${b&&l`<p className="mt-2 text-xs text-[var(--v2-danger-text)]">${b}</p>`}
    </div>
  `}function DM({skill:e}){let t=R();return l`
    ${e.keywords?.length>0&&l`
      <div className="mt-2 text-xs text-[var(--v2-text-muted)]">
        <span className="text-[var(--v2-text-faint)]">${t("skills.activatesOn")}:</span>
        ${e.keywords.join(", ")}
      </div>
    `}
    ${e.usage_hint&&l`<div className="mt-2 text-xs text-[var(--v2-text-muted)]">${e.usage_hint}</div>`}
    ${e.setup_hint&&l`<div className="mt-2 text-xs text-[var(--v2-warning-text)]">${e.setup_hint}</div>`}
    ${(e.has_requirements||e.has_scripts||e.install_source_url)&&l`
      <div className="mt-2 flex flex-wrap gap-1.5">
        ${e.has_requirements&&l`<${Kh}>requirements.txt<//>`}
        ${e.has_scripts&&l`<${Kh}>scripts/<//>`}
        ${e.install_source_url&&l`<${Kh}>${t("skills.imported")}<//>`}
      </div>
    `}
  `}function Kh({children:e}){return l`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${e}
    </span>
  `}function Nk({onInstall:e,isInstalling:t}){let a=R(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,u]=p.default.useState({name:"",content:""}),[c,d]=p.default.useState(""),[m,f]=p.default.useState(""),h=p.default.useCallback((y,$)=>{u(g=>!g[y]||!$.trim()?g:{...g,[y]:""})},[]),x=p.default.useCallback(async()=>{let y=MM({name:n,content:s}),$=OM(y,a);if($.name||$.content){u($),d(""),f("");return}u({name:"",content:""}),d(""),f("");try{let g=await e(y);if(!g?.success){d(g?.message||a("skills.installFailed"));return}r(""),i(""),f(g.message||a("skills.installedSuccess",{name:y.name}))}catch(g){d(g.message||a("skills.installFailed"))}},[s,n,e,a]);return l`
    <${ae} padding="md">
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

      <${kn} label=${a("skills.name")} error=${o.name} required>
        <${Tt}
          size="sm"
          error=${!!o.name}
          aria-invalid=${o.name?"true":void 0}
          value=${n}
          placeholder=${a("skills.namePlaceholder")}
          onInput=${y=>{let $=y.currentTarget.value;r($),h("name",$)}}
        />
      <//>

      <${kn}
        className="mt-3"
        label=${a("skills.content")}
        error=${o.content}
        hint=${a("skills.contentHint")}
        required
      >
        <${Oc}
          rows=${5}
          error=${!!o.content}
          aria-invalid=${o.content?"true":void 0}
          value=${s}
          placeholder=${a("skills.contentPlaceholder")}
          onInput=${y=>{let $=y.currentTarget.value;i($),h("content",$)}}
        />
      <//>

      ${c&&l`<p className="mt-3 text-sm text-[var(--v2-danger-text)]">${c}</p>`}
      ${m&&l`<p className="mt-3 text-sm text-[var(--v2-positive-text)]">${m}</p>`}

      <div className="mt-4 flex justify-end">
        <${A} type="button" size="sm" disabled=${t} onClick=${x}>
          <${M} name="upload" className="h-4 w-4" />
          ${a(t?"skills.installing":"skills.install")}
        <//>
      </div>
    <//>
  `}function MM({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function OM(e,t){return{name:e.name?"":t("skills.nameRequired"),content:e.content?"":t("skills.contentRequired")}}function _k({searchQuery:e=""}){let t=R(),{skills:a,query:n,autoActivateLearned:r,fetchSkillContent:s,installSkill:i,removeSkill:o,updateSkill:u,setSkillAutoActivate:c,setAutoActivateLearned:d,isInstalling:m,isRemoving:f,isUpdating:h,isSettingAutoActivate:x,isSettingAutoActivateLearned:y}=wk(),[$,g]=p.default.useState(""),[v,b]=p.default.useState(""),w=p.default.useCallback(async O=>{if(window.confirm(t("skills.confirmDelete",{name:O}))){g(""),b("");try{let L=await o(O);if(!L?.success){g(L?.message||t("skills.removeFailed"));return}b(L.message||t("skills.removed",{name:O}))}catch(L){g(L.message||t("skills.removeFailed"))}}},[o,t]),S=p.default.useCallback(async(O,L)=>{if(!L.trim())return g(t("skills.contentRequired")),b(""),{success:!1,message:t("skills.contentRequired")};g(""),b("");try{let P=await u({name:O,content:L});return P?.success?(b(P.message||t("skills.updated",{name:O})),P):(g(P?.message||t("skills.updateFailed")),P)}catch(P){let U=P.message||t("skills.updateFailed");return g(U),{success:!1,message:U}}},[t,u]),E=p.default.useCallback(async(O,L)=>{g(""),b("");try{let P=await c({name:O,enabled:L});if(!P?.success){g(P?.message||t("skills.updateFailed"));return}b(P.message)}catch(P){g(P.message||t("skills.updateFailed"))}},[c,t]),k=p.default.useCallback(async O=>{g(""),b("");try{let L=await d(O);if(!L?.success){g(L?.message||t("skills.updateFailed"));return}b(L.message)}catch(L){g(L.message||t("skills.updateFailed"))}},[d,t]),C;if(n.isLoading)C=l`
      <${ae} padding="md">
          <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(O=>l`
            <div key=${O} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
              <div>
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              </div>
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
        <//>
    `;else if(n.error)C=l`
      <${ae} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">${t("skills.failedLoad",{message:n.error.message})}</p>
        <//>
    `;else{let O=a.filter(P=>tt(e,[P.name,P.id,P.description,P.keywords,P.trust_level,P.source_kind,P.version])),L=UM(O);a.length===0?C=l`
        <${ae} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:O.length===0?C=l`<${wt} query=${e} />`:C=l`
        <div id="skills-list">
          ${L.map(P=>l`
              <${PM}
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
      `}return l`
    <div className="space-y-4">
      <${LM}
        enabled=${r}
        isSaving=${y}
        onToggle=${k}
      />
      <${Nk} onInstall=${i} isInstalling=${m} />
      <${jM} error=${$} result=${v} />
      ${C}
    </div>
  `}function LM({enabled:e,isSaving:t,onToggle:a}){let n=R();return l`
    <${ae} padding="md" style=${e?void 0:{background:"var(--v2-danger-soft)"}}>
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
  `}function PM({title:e,skills:t,onEdit:a,onRemove:n,onUpdate:r,onSetAutoActivate:s,isRemoving:i,isUpdating:o,isSettingAutoActivate:u}){return t.length===0?null:l`
    <${ae} padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${e}
      </h3>
      ${t.map(c=>l`
          <${Sk}
            key=${`${c.source_kind||"skill"}:${c.name||c.id}`}
            skill=${c}
            onEdit=${a}
            onRemove=${n}
            onUpdate=${r}
            onSetAutoActivate=${s}
            isRemoving=${i}
            isUpdating=${o}
            isSettingAutoActivate=${u}
          />
        `)}
    <//>
  `}function UM(e){let t=[{id:"user",labelKey:"skills.group.user",skills:[]},{id:"system",labelKey:"skills.group.system",skills:[]},{id:"workspace",labelKey:"skills.group.workspace",skills:[]}],a=t[0];for(let n of e){let r=n.source_kind||"";(r==="system"?t[1]:r==="workspace"?t[2]:a).skills.push(n)}return t.filter(n=>n.skills.length>0)}function jM({error:e,result:t}){return!e&&!t?null:l`
    <div
      className=${e?"rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200":"rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200"}
    >
      ${e||t}
    </div>
  `}function FM(e,t){ci(new Blob([JSON.stringify(t,null,2)],{type:"application/json"}),e)}function BM(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{try{t(JSON.parse(n.result))}catch(r){a(r)}},n.onerror=()=>a(n.error||new Error("Unable to read file")),n.readAsText(e)})}function kk({settingsExport:e,onImport:t,isImporting:a,searchQuery:n,onSearchChange:r,onSearchClear:s,onBack:i,canGoBack:o}){let u=R(),c=p.default.useRef(null),d=p.default.useRef(null),[m,f]=p.default.useState(null),h=p.default.useCallback(($,g)=>{d.current&&window.clearTimeout(d.current),f({tone:$,text:g}),d.current=window.setTimeout(()=>f(null),3500)},[]);p.default.useEffect(()=>()=>{d.current&&window.clearTimeout(d.current)},[]);let x=p.default.useCallback(()=>{e&&(FM("ironclaw-settings.json",e),h("success",u("settings.exportSuccess")))},[e,h,u]),y=p.default.useCallback(async $=>{let g=$.target.files?.[0];if($.target.value="",!!g)try{let v=await BM(g);if(!v||typeof v!="object"||!v.settings||typeof v.settings!="object"||Array.isArray(v.settings))throw new Error(u("settings.importInvalid"));await t(v),h("success",u("settings.importSuccess"))}catch(v){h("error",u("settings.importFailed",{message:v.message}))}},[t,h,u]);return l`
    <div className="rounded-md border border-white/10 bg-white/[0.03] px-3 py-3">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center">
        <div className="flex min-w-0 flex-1 flex-col gap-3 sm:flex-row sm:items-center">
          ${o&&l`
            <${A}
              type="button"
              variant="ghost"
              size="sm"
              onClick=${i}
              className="w-fit gap-2"
            >
              <${M} name="chevron" className="h-3.5 w-3.5 rotate-90" />
              ${u("settings.back")}
            <//>
          `}

          <label className="relative min-w-0 flex-1">
            <span className="sr-only">${u("settings.searchPlaceholder")}</span>
            <${M}
              name="search"
              className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--v2-text-faint)]"
            />
            <input
              type="search"
              value=${n}
              onChange=${$=>r($.target.value)}
              placeholder=${u("settings.searchPlaceholder")}
              className="h-9 w-full rounded-md border border-white/12 bg-white/[0.04] pl-9 pr-9 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
            />
            ${n&&l`
              <button
                type="button"
                onClick=${s}
                aria-label=${u("settings.clearSearch")}
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
            ${u("settings.export")}
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
            ${u(a?"settings.importing":"settings.import")}
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
        <div className="text-xs font-medium text-iron-400">${u("settings.manageJson")}</div>
        ${m&&l`
          <div
            role="status"
            className=${["mt-1 text-xs",m.tone==="error"?"text-red-200":"text-mint"].join(" ")}
          >
            ${m.text}
          </div>
        `}
      </div>
    </div>
  `}function gd(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function Rk(){let e=X(),t=H({queryKey:["settings-tools"],queryFn:I$}),a=t.data?.tools||[],[n,r]=p.default.useState({}),s=G({mutationFn:async({name:o,state:u})=>gd(await K$(o,u),"Save failed"),onSuccess:(o,{name:u,state:c})=>{e.setQueryData(["settings-tools"],d=>{if(!d)return d;let m=o?.tool;return{...d,tools:d.tools.map(f=>f.name===u?{...f,state:c,...m||{}}:f)}}),r(d=>({...d,[u]:!0})),setTimeout(()=>r(d=>({...d,[u]:!1})),2e3)}}),i=p.default.useCallback((o,u)=>s.mutate({name:o,state:u}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}var yd="agent.auto_approve_tools";function zM({visible:e}){let t=R();return e?l`
    <span className="font-mono text-[11px] text-[var(--v2-accent-text)]" role="status">
      ${t("tools.saved")}
    </span>
  `:null}function qM({checked:e,disabled:t=!1,label:a,onChange:n}){return l`
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
  `}function Hh({settings:e,onSave:t,savedKeys:a,isLoading:n}){let r=R(),s=r("settings.field.autoApproveEligibleTools"),i=e?.[yd]===!0||e?.[yd]==="true";return l`
    <${ae} padding="md" className="flex items-center justify-between gap-6">
      <div className="min-w-0">
        <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
          ${s}
        </h3>
        <p className="mt-1 text-sm text-[var(--v2-text-muted)]">
          ${r("settings.field.autoApproveEligibleToolsDesc")}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-3">
        <${zM} visible=${a?.[yd]} />
        <${qM}
          checked=${i}
          disabled=${n}
          label=${s}
          onChange=${o=>t(yd,o)}
        />
      </div>
    <//>
  `}function IM({tool:e,onPermissionChange:t,isSaved:a}){let n=R(),r=[{value:"default",label:n("tools.followDefault"),tone:"neutral"},{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask_each_time",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],s={default:n("tools.sourceDefault"),global:n("tools.sourceGlobal"),override:n("tools.sourceOverride")},i=e.locked,o=r.find(m=>m.value===e.state)||r[1],u=e.effective_source||"default",c=u==="override"?e.state:"default",d=u==="default"&&e.state===e.default_state;return l`
    <div
      data-testid="settings-tool-row"
      data-tool-name=${e.name}
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="flex min-w-0 items-center gap-3">
        ${i&&l`<span data-testid="settings-tool-lock" className="shrink-0">
          <${M}
            name="lock"
            className="h-3.5 w-3.5 text-[var(--v2-text-faint)]"
          />
        </span>`}
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate font-mono text-sm text-[var(--v2-text)]"
              >${e.name}</span
            >
            ${d&&l`
              <span
                className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]"
              >
                ${n("tools.default")}
              </span>
            `}
            <span
              className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]"
            >
              ${s[u]||s.default}
            </span>
          </div>
          ${e.description&&l`
            <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">
              ${e.description}
            </div>
          `}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${i?l`<${q} tone=${o.tone} label=${o.label} size="sm" />`:l`
              <select
                value=${c}
                onChange=${m=>t(e.name,m.target.value)}
                aria-label=${n("tools.permissionFor",{name:e.name})}
                className="v2-select h-8 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2.5 font-mono text-xs text-[var(--v2-text-strong)] outline-none focus:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]"
              >
                ${r.map(m=>l`<option key=${m.value} value=${m.value}>
                      ${m.label}
                    </option>`)}
              </select>
            `}
        ${a&&l`
          <span className="font-mono text-[11px] text-[var(--v2-accent-text)]"
            >${n("tools.saved")}</span
          >
        `}
      </div>
    </div>
  `}function Ck({settings:e={},onSave:t=()=>{},savedKeys:a={},isLoading:n=!1,searchQuery:r=""}){let s=R(),{tools:i,query:o,setPermission:u,savedTools:c}=Rk();if(o.isLoading)return l`
      <div className="space-y-4">
        <${Hh}
          settings=${e}
          onSave=${t}
          savedKeys=${a}
          isLoading=${n}
        />
        <${ae} padding="md">
          <div className="mb-4 h-3 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3,4,5].map(m=>l`
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
    `;if(o.error)return l`
      <div className="space-y-4">
        <${Hh}
          settings=${e}
          onSave=${t}
          savedKeys=${a}
          isLoading=${n}
        />
        <${ae} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">
            ${s("tools.failedLoad",{message:o.error.message})}
          </p>
        <//>
      </div>
    `;let d=i.filter(m=>tt(r,[m.name,m.description,m.state,m.default_state,m.effective_source,m.locked?s("tools.disabled"):""]));return l`
    <div className="space-y-4">
      <${Hh}
        settings=${e}
        onSave=${t}
        savedKeys=${a}
        isLoading=${n}
      />

      ${r&&l`
        <div className="flex justify-end">
          <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">
            ${d.length} / ${i.length}
          </span>
        </div>
      `}

      <${ae} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${s("tools.permissions")}
        </h3>
        ${d.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${s("tools.noMatch")}
            </p>`:d.map(m=>l`
                  <${IM}
                    key=${m.name}
                    tool=${m}
                    onPermissionChange=${u}
                    isSaved=${c[m.name]}
                  />
                `)}
      <//>
    </div>
  `}function Ek(e){return(Number(e)||0).toFixed(2)}function KM(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function Tk(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function Xr({label:e,value:t,description:a}){return l`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&l`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function Ak({searchQuery:e=""}){let t=R(),{credits:a,query:n,authorize:r}=Rc();if(!tt(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return l`<${wt} query=${e} />`;let s;if(n.isLoading)s=l`
      <div className="mt-4">
        ${[1,2,3].map(i=>l`
            <div
              key=${i}
              className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-3 first:border-0"
            >
              <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              <div className="h-4 w-16 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
      </div>
    `;else if(n.isError)s=l`
      <div
        className="mt-4 rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
      >
        ${t("traceCommons.loadFailed")}
      </div>
    `;else if(!a||!a.enrolled&&!(a.submissions_total>0))s=l`
      <div
        className="mt-4 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-6 text-center text-sm text-[var(--v2-text-muted)]"
      >
        ${t("traceCommons.emptyState")}
      </div>
    `;else{let i=a.recent_explanations||[],o=a.holds||[];s=l`
      <div className="mt-4">
        <${Xr}
          label=${t("traceCommons.enrollment")}
          value=${a.enrolled?t("traceCommons.enrolled"):t("traceCommons.notEnrolled")}
        />
        <${Xr}
          label=${t("traceCommons.pendingCredit")}
          description=${t("traceCommons.pendingCreditDesc")}
          value=${Ek(a.pending_credit)}
        />
        <${Xr}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${Ek(a.final_credit)}
        />
        <${Xr}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${KM(a.delayed_credit_delta)}
        />
        <${Xr}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${Xr}
          label=${t("traceCommons.lastSubmission")}
          value=${Tk(a.last_submission_at,t)}
        />
        <${Xr}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${Tk(a.last_credit_sync_at,t)}
        />
      </div>
      ${i.length>0&&l`
        <div className="mt-5">
          <h4
            className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("traceCommons.recentExplanations")}
          </h4>
          <ul className="ml-4 list-disc space-y-1 text-xs text-[var(--v2-text-muted)]">
            ${i.map((u,c)=>l`<li key=${c}>${u}</li>`)}
          </ul>
        </div>
      `}
      ${o.length>0&&l`
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
            ${o.map(u=>l`
                <li
                  key=${u.submission_id}
                  className="flex items-start justify-between gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2"
                >
                  <div className="min-w-0">
                    <div className="text-xs text-[var(--v2-text-strong)]">${u.reason}</div>
                    <div className="mt-0.5 truncate font-mono text-[10px] text-[var(--v2-text-faint)]">
                      ${u.submission_id}
                    </div>
                  </div>
                  <button
                    type="button"
                    onClick=${()=>r.mutate(u.submission_id)}
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
    `}return l`
    <${ae} padding="md">
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
  `}function Dk(){let e=X(),t=H({queryKey:["admin-users"],queryFn:aw,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=G({mutationFn:nw,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=G({mutationFn:({id:i,payload:o})=>rw(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function HM({onCreate:e,isCreating:t,error:a}){let n=R(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[u,c]=p.default.useState("member"),[d,m]=p.default.useState(!1),f=h=>{h.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:u},{onSuccess:()=>{s(""),o(""),m(!1)}})};return d?l`
    <${ae} padding="md">
      <h3
        className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${n("users.newUser")}
      </h3>
      <form onSubmit=${f} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <${kn} label=${n("users.displayName")} htmlFor="user-name">
            <${Tt}
              id="user-name"
              type="text"
              value=${r}
              onChange=${h=>s(h.target.value)}
              required
            />
          <//>
          <${kn} label=${n("users.email")} htmlFor="user-email">
            <${Tt}
              id="user-email"
              type="email"
              value=${i}
              onChange=${h=>o(h.target.value)}
            />
          <//>
        </div>
        <${kn} label=${n("users.role")} htmlFor="user-role">
          <select
            id="user-role"
            value=${u}
            onChange=${h=>c(h.target.value)}
            className="v2-select h-9 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 text-sm text-[var(--v2-text-strong)] outline-none focus:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]"
          >
            <option value="member">${n("users.member")}</option>
            <option value="admin">${n("users.admin")}</option>
          </select>
        <//>
        ${a&&l` <p className="text-sm text-[var(--v2-danger-text)]">${a.message}</p> `}
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
  `:l`
      <${A} variant="secondary" onClick=${()=>m(!0)}>
        <${M} name="plus" className="mr-2 h-4 w-4" />
        ${n("users.addUser")}
      <//>
    `}function QM({user:e}){let t=R(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return l`
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
        ${e.email&&l`
          <div className="mt-0.5 font-mono text-xs text-[var(--v2-text-muted)]">
            ${e.email}
          </div>
        `}
      </div>
      <div
        className="flex shrink-0 items-center gap-4 font-mono text-[11px] text-[var(--v2-text-faint)]"
      >
        ${e.last_active&&l`<span>${new Date(e.last_active).toLocaleDateString()}</span>`}
      </div>
    </div>
  `}function Mk({searchQuery:e=""}){let t=R(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=Dk();if(n.isLoading)return l`
      <${ae} padding="md">
        <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
        ${[1,2,3].map(c=>l`
            <div
              key=${c}
              className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-3.5 first:border-0"
            >
              <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
      <//>
    `;if(r)return l`
      <${ae} padding="lg">
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
    `;if(n.error)return l`
      <${ae} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("users.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let u=a.filter(c=>tt(e,[c.id,c.display_name,c.email,c.role,c.status,c.last_active]));return l`
    <div className="space-y-5">
      <${HM}
        onCreate=${s}
        isCreating=${o}
        error=${i}
      />

      <${ae} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("users.title",{count:u.length})}
        </h3>
        ${a.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("users.noUsers")}
            </p>`:u.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("settings.noMatchingSettings",{query:e})}
            </p>`:u.map(c=>l`<${QM} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function Ok(){let e=X(),t=H({queryKey:["settings-export"],queryFn:O$,staleTime:3e4}),a=t.data?.settings||{},[n,r]=p.default.useState({}),[s,i]=p.default.useState(!1),o=G({mutationFn:async({key:m,value:f})=>gd(await Qp(m,f),"Save failed"),onSuccess:(m,{key:f,value:h})=>{e.setQueryData(["settings-export"],x=>{if(!x)return x;let y={...x,settings:{...x.settings}};return h==null?delete y.settings[f]:y.settings[f]=h,y}),r(x=>({...x,[f]:!0})),setTimeout(()=>r(x=>({...x,[f]:!1})),2e3),Ih.has(f)&&i(!0),f==="agent.auto_approve_tools"&&e.invalidateQueries({queryKey:["settings-tools"]})}}),u=p.default.useCallback((m,f)=>o.mutate({key:m,value:f}),[o]),c=G({mutationFn:L$,onSuccess:(m,f)=>{e.invalidateQueries({queryKey:["settings-export"]});let h=Object.keys(f?.settings||{});h.includes("agent.auto_approve_tools")&&e.invalidateQueries({queryKey:["settings-tools"]}),h.some(x=>Ih.has(x))&&i(!0)}}),d=p.default.useCallback(m=>c.mutateAsync(m),[c]);return{settings:a,query:t,save:u,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error||c.error}}function Qh(){let e=R(),{tab:t}=st(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=ba(),s=r?"inference":"language",i=t||s,{settings:o,query:u,save:c,savedKeys:d,needsRestart:m,importSettings:f,isImporting:h,saveError:x}=Ok(),[y,$]=p.default.useState("");p.default.useEffect(()=>{$("")},[i]);let g=u.isLoading,v={inference:l`<${gk}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${g}
      searchQuery=${y}
    />`,agent:l`<${dk}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${g}
      searchQuery=${y}
    />`,channels:l`<${pk} searchQuery=${y} />`,networking:l`<${bk}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${g}
      searchQuery=${y}
    />`,tools:l`<${Ck}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${g}
      searchQuery=${y}
    />`,skills:l`<${_k} searchQuery=${y} />`,traces:l`<${Ak} searchQuery=${y} />`,users:l`<${Mk} searchQuery=${y} />`,language:l`<${yk} searchQuery=${y} />`},b=C=>C==="users"||C==="inference",w=C=>Object.prototype.hasOwnProperty.call(v,C),S=Object.keys(v).filter(C=>r||!b(C)),k=w(s)&&S.includes(s)?s:S[0]||"language";return!w(i)||!r&&b(i)?l`<${it} to=${`/settings/${k}`} replace />`:l`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${m&&l`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${$k}
                visible=${!0}
                gatewayStatus=${a}
                gatewayStatusQuery=${n}
              />
            </div>`}

            ${x&&l`
              <div
                className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
              >
                ${e("error.saveFailed",{message:x.message})}
              </div>
            `}

            <${kk}
              settingsExport=${u.data||null}
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
  `}var Vh=Object.freeze({todo:!0});function Lk(){return Promise.resolve({users:[],total:0,...Vh})}function Pk(e){return Promise.resolve(null)}function Uk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function jk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Fk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Bk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function zk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function qk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Ik(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...Vh})}function Kk(e="day",t){return Promise.resolve({entries:[],...Vh})}function Hk(){return H({queryKey:["admin","usage-summary"],queryFn:Ik,refetchInterval:3e4})}function bd(e="day",t){return H({queryKey:["admin","usage",e,t],queryFn:()=>Kk(e,t),refetchInterval:3e4})}function Ni(){let e=X(),t=H({queryKey:["admin","users"],queryFn:Lk,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=G({mutationFn:Uk,onSuccess:s}),o=G({mutationFn:({id:f,payload:h})=>jk(f,h),onSuccess:s}),u=G({mutationFn:f=>Fk(f),onSuccess:s}),c=G({mutationFn:f=>Bk(f),onSuccess:s}),d=G({mutationFn:f=>zk(f),onSuccess:s}),m=G({mutationFn:({userId:f,name:h})=>qk(f,h)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(f,h)=>o.mutateAsync({id:f,payload:h}),deleteUser:u.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(f,h)=>m.mutateAsync({userId:f,name:h}),newToken:m.data,clearToken:()=>m.reset()}}function Qk(e){return H({queryKey:["admin","user",e],queryFn:()=>Pk(e),enabled:!!e,refetchInterval:1e4})}function tn(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function Aa(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function Vk(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function hr(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function _i(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function ki(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function Ri(e){return e==="admin"?"signal":"muted"}function Gk(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function Yk(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function Jk(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function Xk(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function Zk(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function VM({users:e,onSelectUser:t}){let a=R(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?l`
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
          ${n.map(r=>l`
              <tr key=${r.id} className="border-b border-white/[0.06] last:border-0">
                <td className="py-3 pr-4">
                  <button
                    onClick=${()=>t(r.id)}
                    className="text-sm font-medium text-signal hover:underline"
                  >
                    ${r.display_name||r.id}
                  </button>
                </td>
                <td className="py-3 pr-4"><${q} tone=${Ri(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${q} tone=${ki(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${hr(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:l`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function Wk({onSelectUser:e,onNavigateTab:t}){let a=R(),n=Hk(),{users:r,query:s}=Ni(),i=n.data||{},o=Gk(r),u=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?l`
      <div className="space-y-5">
        <${I} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            ${[1,2,3,4].map(m=>l`<div key=${m} className="v2-skeleton h-28 rounded-lg" />`)}
          </div>
        <//>
      </div>
    `:l`
    <div className="space-y-5">
      <${I} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.systemOverview")}</h3>
          ${i.uptime_seconds!=null&&l`
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:Vk(i.uptime_seconds)})}</span>
          `}
        </div>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <${et}
            label=${a("admin.dashboard.totalUsers")}
            value=${String(o.total)}
            tone=${o.total>0?"success":"muted"}
          />
          <${et}
            label=${a("admin.dashboard.activeUsers")}
            value=${String(o.active)}
            tone="success"
          />
          <${et}
            label=${a("admin.dashboard.suspended")}
            value=${String(o.suspended)}
            tone=${o.suspended>0?"danger":"muted"}
          />
          <${et}
            label=${a("admin.dashboard.admins")}
            value=${String(o.admins)}
            tone="signal"
          />
        </div>
      <//>

      <${I} className="p-5 sm:p-6">
        <h3 className="mb-5 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.usage30d")}</h3>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <${et}
            label=${a("admin.dashboard.totalJobs")}
            value=${String(c.total||0)}
            tone="muted"
          />
          <${et}
            label=${a("admin.dashboard.llmCalls")}
            value=${String(u.llm_calls||0)}
            tone="muted"
          />
          <${et}
            label=${a("admin.dashboard.totalCost")}
            value=${Aa(u.total_cost)}
            tone="signal"
          />
          <${et}
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
        <${VM} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var GM=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function YM({value:e,max:t}){let a=t>0?e/t*100:0;return l`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function eR({onSelectUser:e}){let t=R(),[a,n]=p.default.useState("day"),r=bd(a),s=r.data?.usage||[],i=Jk(s),o=Xk(s),u=Zk(i),c=i.length>0?i[0].cost:0;return r.isLoading?l`
      <${I} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
        <div className="grid gap-4 sm:grid-cols-4">
          ${[1,2,3,4].map(d=>l`<div key=${d} className="v2-skeleton h-28 rounded-lg" />`)}
        </div>
      <//>
    `:l`
    <div className="space-y-5">
      <${I} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.overview")}</h3>
          <div className="flex gap-1">
            ${GM.map(d=>l`
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

        ${s.length===0?l`<p className="py-4 text-sm text-iron-300">${t("admin.usage.noData")}</p>`:l`
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
                <${et} label=${t("admin.usage.totalCalls")} value=${u.calls.toLocaleString()} tone="muted" />
                <${et} label=${t("admin.usage.inputTokens")} value=${tn(u.input_tokens)} tone="muted" />
                <${et} label=${t("admin.usage.outputTokens")} value=${tn(u.output_tokens)} tone="muted" />
                <${et} label=${t("admin.usage.totalCost")} value=${Aa(u.cost.toFixed(2))} tone="signal" />
              </div>
            `}
      <//>

      ${i.length>0&&l`
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
                ${i.map(d=>l`
                    <tr key=${d.user_id} className="border-b border-white/[0.06] last:border-0">
                      <td className="py-3 pr-4">
                        <button
                          onClick=${()=>e(d.user_id)}
                          className="font-mono text-xs text-signal hover:underline"
                        >
                          ${_i(d.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${tn(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${tn(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${Aa(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${YM} value=${d.cost} max=${c} />
                      </td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}

      ${o.length>0&&l`
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
                ${o.map(d=>l`
                    <tr key=${d.model} className="border-b border-white/[0.06] last:border-0">
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${d.model}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${tn(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${tn(d.output_tokens)}</td>
                      <td className="py-3 font-mono text-xs text-iron-100">${Aa(d.cost.toFixed(2))}</td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}
    </div>
  `}function vr({label:e,children:t}){return l`
    <div className="flex items-start justify-between gap-4 border-t border-white/[0.06] py-3 first:border-0 first:pt-0">
      <span className="text-xs text-iron-300">${e}</span>
      <span className="text-right text-sm text-iron-100">${t}</span>
    </div>
  `}function tR({userId:e,onBack:t}){let a=R(),n=Qk(e),r=bd("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:u,createToken:c,newToken:d,clearToken:m}=Ni(),[f,h]=p.default.useState(null),[x,y]=p.default.useState(!1),$=n.data,g=r.data?.usage||[];if(p.default.useEffect(()=>{$&&f===null&&h($.role)},[$]),n.isLoading)return l`
      <div className="space-y-5">
        <${I} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-2 h-6 w-48 rounded" />
          <div className="v2-skeleton h-4 w-32 rounded" />
        <//>
      </div>
    `;if(n.error)return l`
      <${I} className="p-5 sm:p-6">
        <p className="text-sm text-red-200">${a("error.loadFailed",{what:a("admin.users.user"),message:n.error.message})}</p>
      <//>
    `;if(!$)return null;let v=async()=>{f&&f!==$.role&&await o($.id,{role:f})},b=async()=>{await u($.id),t()},w=async()=>{let S=window.prompt(a("admin.users.tokenNamePrompt",{name:$.display_name||a("admin.users.userFallback")}));S&&await c($.id,S)};return l`
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
              <${q} tone=${Ri($.role)} label=${$.role||"member"} />
              <${q} tone=${ki($.status)} label=${$.status||"active"} />
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            ${$.status==="active"?l`<${A} variant="secondary" onClick=${()=>s($.id)}>${a("admin.users.suspend")}<//>`:l`<${A} variant="secondary" onClick=${()=>i($.id)}>${a("admin.users.activate")}<//>`}
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

      ${(d?.token||d?.plaintext_token)&&l`
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
        <${I} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.profile")}</h3>
          <${vr} label=${a("admin.user.id")}>
            <span className="font-mono text-xs">${$.id}</span>
          <//>
          <${vr} label=${a("admin.user.email")}>${$.email||a("admin.user.notSet")}<//>
          <${vr} label=${a("admin.user.created")}>${hr($.created_at)}<//>
          <${vr} label=${a("admin.user.lastLogin")}>${hr($.last_login_at)}<//>
          ${$.created_by&&l`
            <${vr} label=${a("admin.user.createdBy")}>
              <span className="font-mono text-xs">${_i($.created_by)}</span>
            <//>
          `}
        <//>

        <${I} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${vr} label=${a("admin.user.jobs")}>${$.job_count??0}<//>
          <${vr} label=${a("admin.user.totalCost")}>${Aa($.total_cost)}<//>
          <${vr} label=${a("admin.user.lastActive")}>${hr($.last_active_at)}<//>
        <//>
      </div>

      <${I} className="p-5 sm:p-6">
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

      <${I} className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.usage30Days")}</h3>
        ${g.length===0?l`<p className="py-4 text-sm text-iron-300">${a("admin.user.noUsage")}</p>`:l`
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
                    ${g.map((S,E)=>l`
                        <tr key=${E} className="border-b border-white/[0.06] last:border-0">
                          <td className="py-3 pr-4 font-mono text-xs text-iron-100">${S.model}</td>
                          <td className="py-3 pr-4 font-mono text-xs text-iron-300">${(S.call_count||0).toLocaleString()}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${tn(S.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${tn(S.output_tokens)}</td>
                          <td className="py-3 font-mono text-xs text-iron-100">${Aa(S.total_cost)}</td>
                        </tr>
                      `)}
                  </tbody>
                </table>
              </div>
            `}
      <//>

      ${x&&l`
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
  `}function JM(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function XM({token:e,onDismiss:t}){let a=R(),[n,r]=p.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return l`
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
  `}function ZM({onCreate:e,isCreating:t,error:a}){let n=R(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[u,c]=p.default.useState("member"),[d,m]=p.default.useState(!1),f=async h=>{h.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:u}),s(""),o(""),m(!1))};return d?l`
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
              value=${u}
              onChange=${h=>c(h.target.value)}
              className="v2-select h-9 w-full rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            >
              <option value="member">${n("admin.users.member")}</option>
              <option value="admin">${n("admin.users.admin")}</option>
            </select>
          </div>
        </div>
        ${a&&l`<p className="text-sm text-[var(--v2-danger-text)]">${a.message}</p>`}
        <div className="flex gap-2">
          <${A} type="submit" disabled=${t}>
            ${n(t?"admin.users.creating":"admin.users.createUser")}
          <//>
          <${A} variant="ghost" type="button" onClick=${()=>m(!1)}>${n("admin.users.cancel")}<//>
        </div>
      </form>
    <//>
  `:l`
      <${A} variant="secondary" onClick=${()=>m(!0)}>
        <${M} name="plus" className="mr-2 h-4 w-4" />
        ${n("admin.users.newUser")}
      <//>
    `}function WM({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=R();return l`
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
  `}function eO({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=R();return l`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${q} tone=${Ri(e.role)} label=${e.role||"member"} />
          <${q} tone=${ki(e.status)} label=${e.status||"active"} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          ${e.email&&l`<span className="font-mono text-xs text-iron-300">${e.email}</span>`}
          <span className="font-mono text-xs text-iron-700">${_i(e.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-iron-300 sm:inline">
          ${e.job_count!=null?i("admin.users.jobsCount",{count:e.job_count}):""}
          ${e.total_cost!=null?` \xB7 ${Aa(e.total_cost)}`:""}
        </span>
        <span className="hidden text-xs text-iron-700 lg:inline">${hr(e.last_active_at)}</span>
        <div className="flex gap-1">
          ${e.status==="active"?l`<button onClick=${()=>a(e.id)} className="rounded-md border border-iron-700 px-2.5 py-1.5 text-[11px] font-medium text-iron-300 hover:border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] hover:text-[var(--v2-danger-text)]">${i("admin.users.suspend")}</button>`:l`<button onClick=${()=>n(e.id)} className="rounded-md border border-iron-700 px-2.5 py-1.5 text-[11px] font-medium text-iron-300 hover:border-signal/30 hover:text-signal">${i("admin.users.activate")}</button>`}
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
  `}function aR({selectedUserId:e,onSelectUser:t}){let a=R(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:u,updateUser:c,deleteUser:d,suspendUser:m,activateUser:f,createToken:h,newToken:x,clearToken:y}=Ni(),[$,g]=p.default.useState(""),[v,b]=p.default.useState("all"),[w,S]=p.default.useState(null),E=Yk(n,{search:$,filter:v}),k=JM(a),C=L=>{S({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{m(L),S(null)}})},O=async(L,P)=>{let U=window.prompt(a("admin.users.tokenNamePrompt",{name:P||a("admin.users.userFallback")}));U&&await h(L,U)};return r.isLoading?l`
      <${I} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-3 w-24 rounded" />
        ${[1,2,3].map(L=>l`
          <div key=${L} className="flex items-center justify-between border-t border-iron-700 py-3.5 first:border-0">
            <div className="v2-skeleton h-4 w-32 rounded" />
            <div className="v2-skeleton h-6 w-20 rounded-full" />
          </div>
        `)}
      <//>
    `:s?l`
      <${I} className="p-6 sm:p-8">
        <div className="flex items-center gap-3">
          <${M} name="lock" className="h-5 w-5 text-iron-700" />
          <h3 className="text-lg font-semibold text-iron-100">${a("users.adminRequired")}</h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${a("users.adminRequiredDesc")}
        </p>
      <//>
    `:l`
    <div className="space-y-5">
      ${x&&l`
        <${XM}
          token=${x.token||x.plaintext_token}
          onDismiss=${y}
        />
      `}

      <${ZM} onCreate=${i} isCreating=${o} error=${u} />

      <${I} className="p-5 sm:p-6">
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${a("admin.users.title",{count:E.length,total:n.length})}
          </h3>
          <div className="flex items-center gap-2">
            <input
              type="text"
              placeholder=${a("admin.users.searchPlaceholder")}
              value=${$}
              onChange=${L=>g(L.target.value)}
              className="h-8 w-48 rounded-md border border-iron-700 bg-iron-800/70 px-3 text-xs text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
            />
            <div className="flex gap-1">
              ${k.map(L=>l`
                  <button
                    key=${L.value}
                    onClick=${()=>b(L.value)}
                    className=${["rounded-md px-2.5 py-1.5 text-[11px] font-medium",v===L.value?"border border-signal/35 bg-signal/10 text-iron-100":"border border-transparent text-iron-300 hover:text-iron-100"].join(" ")}
                  >
                    ${L.label}
                  </button>
                `)}
            </div>
          </div>
        </div>

        ${E.length===0?l`<p className="py-4 text-sm text-iron-300">${a("admin.users.noMatch")}</p>`:E.map(L=>l`
                <${eO}
                  key=${L.id}
                  user=${L}
                  onSelect=${t}
                  onSuspend=${C}
                  onActivate=${f}
                  onChangeRole=${(P,U)=>c(P,{role:U})}
                  onCreateToken=${O}
                />
              `)}
      <//>

      ${w&&l`
        <${WM}
          title=${w.title}
          message=${w.message}
          confirmLabel=${w.confirmLabel}
          onConfirm=${w.onConfirm}
          onCancel=${()=>S(null)}
        />
      `}
    </div>
  `}function nR(){let{tab:e="dashboard"}=st(),t=pe(),[a,n]=p.default.useState(null),r=p.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=p.default.useCallback(()=>{n(null)},[]),i={dashboard:l`<${Wk}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?l`<${tR} userId=${a} onBack=${s} />`:l`<${aR}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:l`<${eR} onSelectUser=${r} />`};return i[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:l`<${it} to="/admin/dashboard" replace />`}var tO=2e3,aO=500,nO=2e3,rO=new Set([403,404]),sO=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function iO(e=globalThis.location,t=null){let a=new URLSearchParams(e?.search||""),n={active:[]};for(let[r,s,i]of sO){let o=a.get(s)?.trim();o?(n[r]=o,n.active.push({key:r,param:s,labelKey:i,value:o})):n[r]=null}return!n.threadId&&t&&(n.threadId=t),n}function rR({isAdmin:e=!1,defaultThreadId:t=null}={}){let a=Pe(),n=a?.search||"",r=p.default.useMemo(()=>iO(a,t),[t,n]),{runId:s,source:i,threadId:o,toolCallId:u,toolName:c,turnId:d}=r,[m,f]=p.default.useState([]),[h,x]=p.default.useState("all"),[y,$]=p.default.useState(""),[g,v]=p.default.useState(!1),[b,w]=p.default.useState(!0),[S,E]=p.default.useState(!0),[k,C]=p.default.useState(null),O=p.default.useRef(new Set),L=p.default.useRef(0),P=!e&&!o;p.default.useEffect(()=>{L.current+=1,f([]),C(null)},[e,s,i,o,u,c,d]);let U=p.default.useCallback(async()=>{if(P){E(!1);return}let te=++L.current;E(!0);try{let se={limit:aO,level:h==="all"?null:h,target:y.trim()||null,threadId:o,runId:s,turnId:d,toolCallId:u,toolName:c,source:i},ie;try{ie=await(e?Kx(se):Mp(se))}catch(ft){if(!e||!rO.has(ft?.status))throw ft;ie=await Mp(se)}if(te!==L.current)return;let Ye=O.current,gt=B2(ie).entries.filter(ft=>!Ye.has(ft.id));f(gt),C(null)}catch(se){if(te!==L.current)return;C(se)}finally{te===L.current&&E(!1)}},[e,h,P,s,i,y,o,u,c,d]);p.default.useEffect(()=>{U()},[U]),p.default.useEffect(()=>{if(g||P)return;let te=setInterval(U,tO);return()=>clearInterval(te)},[U,P,g]);let T=p.default.useCallback(()=>{v(te=>!te)},[]),K=p.default.useCallback(()=>{let te=[...O.current,...m.map(se=>se.id)].slice(-nO);O.current=new Set(te),f([])},[m]);return{entries:m,totalCount:m.length,paused:g,togglePause:T,clearEntries:K,levelFilter:h,setLevelFilter:x,targetFilter:y,setTargetFilter:$,autoScroll:b,setAutoScroll:w,serverLevel:null,changeServerLevel:async()=>{},scope:r,needsThreadScope:P,status:P?"needs_scope":k?"error":S?"loading":"ready",isLoading:S,error:k}}var oO=["all","trace","debug","info","warn","error"],lO=["trace","debug","info","warn","error"],sR={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},uO={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function cO({entry:e}){let t=R(),[a,n]=p.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=sR[e.level]||sR.info,i=uO[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(u=>!!u.value);return l`
    <div data-testid="logs-entry" className=${i}>
      <div
        data-testid="logs-entry-row"
        onClick=${()=>n(u=>!u)}
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
      ${a&&o.length>0&&l`
        <div
          data-testid="logs-entry-context"
          className="flex flex-wrap gap-1.5 px-4 pb-2 pl-[calc(7rem+3rem+2.5rem)] font-mono text-[11px] text-[var(--v2-text-muted)]"
        >
          ${o.map(u=>l`
              <span
                key=${u.key}
                data-testid="logs-context-chip"
                data-context-key=${u.key}
                className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-0.5"
              >
                <span>${t(u.labelKey)}</span>
                <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${u.value}</span>
              </span>
            `)}
        </div>
      `}
    </div>
  `}function iR({value:e,onChange:t,options:a,labelKey:n,t:r}){return l`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>l`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function dO({label:e,value:t,scopeKey:a}){return l`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function oR(){let e=R(),{isAdmin:t=!1,threadsState:a}=ba()||{},{entries:n,totalCount:r,paused:s,togglePause:i,clearEntries:o,levelFilter:u,setLevelFilter:c,targetFilter:d,setTargetFilter:m,autoScroll:f,setAutoScroll:h,serverLevel:x,changeServerLevel:y,scope:$,isLoading:g,error:v,needsThreadScope:b}=rR({isAdmin:t,defaultThreadId:t?null:a?.activeThreadId||null}),w=p.default.useRef(null),S=p.default.useRef(!0);p.default.useEffect(()=>{f&&S.current&&w.current&&(w.current.scrollTop=0)},[n,f]);let E=p.default.useCallback(O=>{S.current=O.currentTarget.scrollTop<=48},[]),k=n.length>0,C=$?.active||[];return l`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${iR}
          value=${u}
          onChange=${c}
          options=${oO}
          labelKey=${O=>O==="all"?"logs.levelAll":`logs.level.${O}`}
          t=${e}
        />

        <!-- Target filter -->
        <input
          type="text"
          value=${d}
          onInput=${O=>m(O.target.value)}
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
              onChange=${O=>h(O.target.checked)}
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

        ${C.length>0&&l`
          <div
            data-testid="logs-scope-toolbar"
            className="flex w-full flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]"
          >
            <span className="font-medium text-[var(--v2-text-strong)]">${e("logs.scoped")}</span>
            ${C.map(O=>l`<${dO} key=${O.param} scopeKey=${O.param} label=${e(O.labelKey)} value=${O.value} />`)}
            <a
              href="/v2/logs"
              className="ml-auto rounded-[6px] px-2 py-1 text-xs text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
            >
              ${e("logs.clearScope")}
            </a>
          </div>
        `}

        <!-- Server log level -->
        ${x!=null&&l`
          <div className="flex w-full items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]">
            <span>${e("logs.serverLevel")}</span>
            <${iR}
              value=${x}
              onChange=${y}
              options=${lO}
              labelKey=${O=>`logs.level.${O}`}
              t=${e}
            />
            <span className="ml-auto tabular-nums">
              ${e("logs.entryCount",{count:r})}
              ${s?l`<span className="ml-1 text-yellow-400">${e("logs.pausedBadge")}</span>`:null}
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
        ${v&&k?l`
              <div
                className="sticky top-0 z-10 border-b border-red-500/25 bg-red-950/70 px-4 py-2 text-xs text-red-100 backdrop-blur"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:v.message||v.statusText||"Request failed"})}
              </div>
            `:null}
        ${b?l`
              <div
                data-testid="logs-select-thread-state"
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("chat.selectConversation")}
              </div>
            `:v&&!k?l`
              <div
                className="flex h-full items-center justify-center px-6 text-center text-sm text-red-300"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:v.message||v.statusText||"Request failed"})}
              </div>
            `:g&&!k?l`
                <div
                  className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
                >
                  ${e("common.loading")}
                </div>
              `:k?n.map(O=>l`<${cO} key=${O.id} entry=${O} />`):l`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function uR(){return l`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function mO({auth:e}){let t=pe(),n=Pe().state?.from,r=n?`${n.pathname||Kr}${n.search||""}${n.hash||""}`:Kr,s=`/v2${r==="/"?"":r}`,i=p.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?l`<${uR} />`:e.isAuthenticated?l`<${it} to=${r} replace />`:l`<${x1}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function fO({auth:e,children:t}){let a=Pe();return e.isChecking?l`<${uR} />`:e.isAuthenticated?t:l`<${it} to="/login" replace state=${{from:a}} />`}function pO({auth:e}){return l`
    <${fO} auth=${e}>
      <${Yw}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        rebornProjectsEnabled=${e.rebornProjectsEnabled}
        onSignOut=${e.signOut}
      />
    <//>
  `}function lR({auth:e}){return e.isAdmin?l`<${nR} />`:l`<${it} to=${Kr} replace />`}function cR(){let e=C$();return l`
    <${Ep} basename="/v2">
      <${kp}>
        <${be} path="/login" element=${l`<${mO} auth=${e} />`} />
        <${be} path="/" element=${l`<${pO} auth=${e} />`}>
          <${be} index element=${l`<${it} to=${Kr} replace />`} />
          <${be} path="overview" element=${l`<${it} to=${Kr} replace />`} />
          <${be} path="welcome" element=${l`<${V2} />`} />
          <${be} path="chat" element=${l`<${$h} />`} />
          <${be} path="chat/:threadId" element=${l`<${$h} />`} />
          <${be} path="workspace" element=${l`<${Sh} />`} />
          <${be} path="workspace/*" element=${l`<${Sh} />`} />
          <${be} path="projects" element=${l`<${dl} />`} />
          <${be} path="projects/:projectId" element=${l`<${dl} />`} />
          <${be} path="projects/:projectId/missions/:missionId" element=${l`<${dl} />`} />
          <${be} path="projects/:projectId/threads/:threadId" element=${l`<${dl} />`} />
          <${be} path="missions" element=${l`<${_h} />`} />
          <${be} path="missions/:missionId" element=${l`<${_h} />`} />
          <${be} path="jobs" element=${l`<${Ch} />`} />
          <${be} path="jobs/:jobId" element=${l`<${Ch} />`} />
          <${be} path="routines" element=${l`<${Th} />`} />
          <${be} path="routines/:routineId" element=${l`<${Th} />`} />
          <${be} path="automations" element=${l`<${WN} />`} />
          <${be} path="extensions" element=${l`<${qh} />`} />
          <${be} path="extensions/:tab" element=${l`<${qh} />`} />
          <${be} path="logs" element=${l`<${oR} />`} />
          <${be} path="settings" element=${l`<${Qh} />`} />
          <${be} path="settings/:tab" element=${l`<${Qh} />`} />
          <${be} path="admin" element=${l`<${lR} auth=${e} />`} />
          <${be} path="admin/:tab" element=${l`<${lR} auth=${e} />`} />
        <//>
        <${be} path="*" element=${l`<${it} to=${Kr} replace />`} />
      <//>
    <//>
  `}Yh("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","approval.viewFullCommand":"View full command","approval.showCommandPreview":"Show preview","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.tabDeclined":"Declined","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitDeclined":"declined","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.serviceUnavailable":"Service unavailable","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveEligibleTools":"Always allow eligible tools","settings.field.autoApproveEligibleToolsDesc":"Applies to tools set to \u201CFollow global\u201D. Per-tool settings override this; hard-floor approval gates still ask.","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.followDefault":"Follow global","tools.sourceDefault":"default permission","tools.sourceGlobal":"global setting","tools.sourceOverride":"per-tool override","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","skills.defaultAutoActivationEnabled":"Default skill auto-activation enabled","skills.defaultAutoActivationDisabled":"Default skill auto-activation disabled","skills.defaultAutoActivationOnDesc":"Skills auto-activate by keyword on matching requests. Turn off to require an explicit /name.","skills.defaultAutoActivationOffDesc":"Skills run only when you type /name. Turn on to let them auto-activate by keyword.","skills.defaultAutoActivationOnButton":"Default: On","skills.defaultAutoActivationOffButton":"Default: Off","skills.autoActivateOnTitle":"Auto-activation on \u2014 runs on matching requests. Click to make it explicit-only (/name).","skills.autoActivateOffTitle":"Explicit-only \u2014 runs only when you type /name. Click to enable auto-activation.","skills.autoActivateOnLabel":"Auto-activate: On","skills.autoActivateOffLabel":"Auto-activate: Off","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Memory, files & attachments","workspace.readOnly":"Read-only","workspace.filterPlaceholder":"Filter by name\u2026","workspace.emptyDir":"This folder is empty.","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.noFiles":"No files here.","workspace.noMatches":"Nothing matches that filter.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a file","workspace.pickFileDesc":"Choose a file from the tree to preview or download it. This viewer is read-only.","workspace.parent":"Parent: {path}","workspace.download":"Download","workspace.binaryPreviewUnavailable":"No inline preview for this file type. Download it to view the contents.","workspace.fileMeta":"{mime} \xB7 {size} bytes","workspace.unableOpenDirectory":"Unable to open directory","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.filter.completed":"Completed","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.schedule.onceAt":"Once on {datetime}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.filterAction":"Show {label}","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.empty.onboardingTitle":"No automations yet","automations.empty.onboardingDescription":"Automations are created by chatting with your agent \u2014 there's no form to fill out. Ask it to do something on a schedule and it will set up a recurring automation for you.","automations.empty.examplesTitle":"Try asking your agent","automations.empty.example1":"Check the nearai/ironclaw repo every 10 minutes and summarize new issues, PRs, and commits.","automations.empty.example2":"Every weekday at 9am, send me a summary of my unread email.","automations.empty.example3":"Remind me to review open pull requests every afternoon at 3pm.","automations.empty.startInChat":"Start in chat","automations.empty.copyPrompt":"Copy prompt","automations.empty.copied":"Copied","automations.refreshing":"Refreshing\u2026","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.runs.total":"Recent runs: {count}","automations.runs.ok":"OK: {count}","automations.runs.error":"Failed: {count}","automations.runs.running":"Running: {count}","automations.runs.unknown":"Unknown: {count}","automations.runs.showingOf":"Showing {shown} of {total} recent runs","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up a project for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open project","projects.openGeneralWorkspace":"Open project","projects.noDescription":"No project description yet. The project is still being shaped by recent activity and thread history.","projects.general.label":"General project","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,dR.createRoot)(document.getElementById("v2-root")).render(l`
  <${Jh}>
    <${Dd} client=${Ct}>
      <${cR} />
    <//>
  <//>
`);
