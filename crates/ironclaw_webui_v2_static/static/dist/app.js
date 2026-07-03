import{a as Mn,b as qe,c as Qe,d as p,e as u,f as uv,g as cv,h as wl,i as k,j as Sl}from"./chunks/chunk-GE6TJDZP.js";var Ev=Mn(Dl=>{"use strict";var Vk=Symbol.for("react.transitional.element"),Gk=Symbol.for("react.fragment");function Cv(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:Vk,type:e,key:n,ref:t!==void 0?t:null,props:a}}Dl.Fragment=Gk;Dl.jsx=Cv;Dl.jsxs=Cv});var Bd=Mn((B6,Tv)=>{"use strict";Tv.exports=Ev()});var Hv=Mn(Ue=>{"use strict";function Vd(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<zl(r,t))e[n]=t,e[a]=r,a=n;else break e}}function Ia(e){return e.length===0?null:e[0]}function Il(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],l=i+1,c=e[l];if(0>zl(o,a))l<r&&0>zl(c,o)?(e[n]=c,e[l]=a,n=l):(e[n]=o,e[i]=a,n=i);else if(l<r&&0>zl(c,a))e[n]=c,e[l]=a,n=l;else break e}}return t}function zl(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}Ue.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(Lv=performance,Ue.unstable_now=function(){return Lv.now()}):(Hd=Date,Pv=Hd.now(),Ue.unstable_now=function(){return Hd.now()-Pv});var Lv,Hd,Pv,un=[],Pn=[],Wk=1,ma=null,wt=3,Gd=!1,Hi=!1,Ki=!1,Yd=!1,Fv=typeof setTimeout=="function"?setTimeout:null,Bv=typeof clearTimeout=="function"?clearTimeout:null,Uv=typeof setImmediate<"u"?setImmediate:null;function ql(e){for(var t=Ia(Pn);t!==null;){if(t.callback===null)Il(Pn);else if(t.startTime<=e)Il(Pn),t.sortIndex=t.expirationTime,Vd(un,t);else break;t=Ia(Pn)}}function Jd(e){if(Ki=!1,ql(e),!Hi)if(Ia(un)!==null)Hi=!0,cs||(cs=!0,us());else{var t=Ia(Pn);t!==null&&Xd(Jd,t.startTime-e)}}var cs=!1,Qi=-1,zv=5,qv=-1;function Iv(){return Yd?!0:!(Ue.unstable_now()-qv<zv)}function Kd(){if(Yd=!1,cs){var e=Ue.unstable_now();qv=e;var t=!0;try{e:{Hi=!1,Ki&&(Ki=!1,Bv(Qi),Qi=-1),Gd=!0;var a=wt;try{t:{for(ql(e),ma=Ia(un);ma!==null&&!(ma.expirationTime>e&&Iv());){var n=ma.callback;if(typeof n=="function"){ma.callback=null,wt=ma.priorityLevel;var r=n(ma.expirationTime<=e);if(e=Ue.unstable_now(),typeof r=="function"){ma.callback=r,ql(e),t=!0;break t}ma===Ia(un)&&Il(un),ql(e)}else Il(un);ma=Ia(un)}if(ma!==null)t=!0;else{var s=Ia(Pn);s!==null&&Xd(Jd,s.startTime-e),t=!1}}break e}finally{ma=null,wt=a,Gd=!1}t=void 0}}finally{t?us():cs=!1}}}var us;typeof Uv=="function"?us=function(){Uv(Kd)}:typeof MessageChannel<"u"?(Qd=new MessageChannel,jv=Qd.port2,Qd.port1.onmessage=Kd,us=function(){jv.postMessage(null)}):us=function(){Fv(Kd,0)};var Qd,jv;function Xd(e,t){Qi=Fv(function(){e(Ue.unstable_now())},t)}Ue.unstable_IdlePriority=5;Ue.unstable_ImmediatePriority=1;Ue.unstable_LowPriority=4;Ue.unstable_NormalPriority=3;Ue.unstable_Profiling=null;Ue.unstable_UserBlockingPriority=2;Ue.unstable_cancelCallback=function(e){e.callback=null};Ue.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):zv=0<e?Math.floor(1e3/e):5};Ue.unstable_getCurrentPriorityLevel=function(){return wt};Ue.unstable_next=function(e){switch(wt){case 1:case 2:case 3:var t=3;break;default:t=wt}var a=wt;wt=t;try{return e()}finally{wt=a}};Ue.unstable_requestPaint=function(){Yd=!0};Ue.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=wt;wt=e;try{return t()}finally{wt=a}};Ue.unstable_scheduleCallback=function(e,t,a){var n=Ue.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:Wk++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,Vd(Pn,e),Ia(un)===null&&e===Ia(Pn)&&(Ki?(Bv(Qi),Qi=-1):Ki=!0,Xd(Jd,a-n))):(e.sortIndex=r,Vd(un,e),Hi||Gd||(Hi=!0,cs||(cs=!0,us()))),e};Ue.unstable_shouldYield=Iv;Ue.unstable_wrapCallback=function(e){var t=wt;return function(){var a=wt;wt=t;try{return e.apply(this,arguments)}finally{wt=a}}}});var Qv=Mn((wP,Kv)=>{"use strict";Kv.exports=Hv()});var Gv=Mn(Tt=>{"use strict";var Zk=Qe();function Vv(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Un(){}var Et={d:{f:Un,r:function(){throw Error(Vv(522))},D:Un,C:Un,L:Un,m:Un,X:Un,S:Un,M:Un},p:0,findDOMNode:null},eC=Symbol.for("react.portal");function tC(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:eC,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var Vi=Zk.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function Hl(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}Tt.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=Et;Tt.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(Vv(299));return tC(e,t,null,a)};Tt.flushSync=function(e){var t=Vi.T,a=Et.p;try{if(Vi.T=null,Et.p=2,e)return e()}finally{Vi.T=t,Et.p=a,Et.d.f()}};Tt.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,Et.d.C(e,t))};Tt.prefetchDNS=function(e){typeof e=="string"&&Et.d.D(e)};Tt.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=Hl(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?Et.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&Et.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};Tt.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=Hl(t.as,t.crossOrigin);Et.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&Et.d.M(e)};Tt.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=Hl(a,t.crossOrigin);Et.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};Tt.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=Hl(t.as,t.crossOrigin);Et.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else Et.d.m(e)};Tt.requestFormReset=function(e){Et.d.r(e)};Tt.unstable_batchedUpdates=function(e,t){return e(t)};Tt.useFormState=function(e,t,a){return Vi.H.useFormState(e,t,a)};Tt.useFormStatus=function(){return Vi.H.useHostTransitionStatus()};Tt.version="19.1.0"});var Xv=Mn((NP,Jv)=>{"use strict";function Yv(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(Yv)}catch(e){console.error(e)}}Yv(),Jv.exports=Gv()});var Zx=Mn(mc=>{"use strict";var st=Qv(),by=Qe(),aC=Xv();function j(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function xy(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function Lo(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function $y(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function Wv(e){if(Lo(e)!==e)throw Error(j(188))}function nC(e){var t=e.alternate;if(!t){if(t=Lo(e),t===null)throw Error(j(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return Wv(r),e;if(s===n)return Wv(r),t;s=s.sibling}throw Error(j(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(j(189))}}if(a.alternate!==n)throw Error(j(190))}if(a.tag!==3)throw Error(j(188));return a.stateNode.current===a?e:t}function wy(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=wy(e),t!==null)return t;e=e.sibling}return null}var Me=Object.assign,rC=Symbol.for("react.element"),Kl=Symbol.for("react.transitional.element"),ao=Symbol.for("react.portal"),gs=Symbol.for("react.fragment"),Sy=Symbol.for("react.strict_mode"),Em=Symbol.for("react.profiler"),sC=Symbol.for("react.provider"),Ny=Symbol.for("react.consumer"),pn=Symbol.for("react.context"),_f=Symbol.for("react.forward_ref"),Tm=Symbol.for("react.suspense"),Am=Symbol.for("react.suspense_list"),Rf=Symbol.for("react.memo"),Bn=Symbol.for("react.lazy");Symbol.for("react.scope");var Dm=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var iC=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var Zv=Symbol.iterator;function Gi(e){return e===null||typeof e!="object"?null:(e=Zv&&e[Zv]||e["@@iterator"],typeof e=="function"?e:null)}var oC=Symbol.for("react.client.reference");function Mm(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===oC?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case gs:return"Fragment";case Em:return"Profiler";case Sy:return"StrictMode";case Tm:return"Suspense";case Am:return"SuspenseList";case Dm:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case ao:return"Portal";case pn:return(e.displayName||"Context")+".Provider";case Ny:return(e._context.displayName||"Context")+".Consumer";case _f:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case Rf:return t=e.displayName||null,t!==null?t:Mm(e.type)||"Memo";case Bn:t=e._payload,e=e._init;try{return Mm(e(t))}catch{}}return null}var no=Array.isArray,se=by.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,be=aC.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,Rr={pending:!1,data:null,method:null,action:null},Om=[],ys=-1;function Ja(e){return{current:e}}function mt(e){0>ys||(e.current=Om[ys],Om[ys]=null,ys--)}function Fe(e,t){ys++,Om[ys]=e.current,e.current=t}var Va=Ja(null),$o=Ja(null),Jn=Ja(null),$u=Ja(null);function wu(e,t){switch(Fe(Jn,t),Fe($o,e),Fe(Va,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?sy(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=sy(t),e=zx(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}mt(Va),Fe(Va,e)}function Us(){mt(Va),mt($o),mt(Jn)}function Lm(e){e.memoizedState!==null&&Fe($u,e);var t=Va.current,a=zx(t,e.type);t!==a&&(Fe($o,e),Fe(Va,a))}function Su(e){$o.current===e&&(mt(Va),mt($o)),$u.current===e&&(mt($u),Ao._currentValue=Rr)}var Pm=Object.prototype.hasOwnProperty,kf=st.unstable_scheduleCallback,Wd=st.unstable_cancelCallback,lC=st.unstable_shouldYield,uC=st.unstable_requestPaint,Ga=st.unstable_now,cC=st.unstable_getCurrentPriorityLevel,_y=st.unstable_ImmediatePriority,Ry=st.unstable_UserBlockingPriority,Nu=st.unstable_NormalPriority,dC=st.unstable_LowPriority,ky=st.unstable_IdlePriority,mC=st.log,fC=st.unstable_setDisableYieldValue,Po=null,Wt=null;function Qn(e){if(typeof mC=="function"&&fC(e),Wt&&typeof Wt.setStrictMode=="function")try{Wt.setStrictMode(Po,e)}catch{}}var Zt=Math.clz32?Math.clz32:vC,pC=Math.log,hC=Math.LN2;function vC(e){return e>>>=0,e===0?32:31-(pC(e)/hC|0)|0}var Ql=256,Vl=4194304;function Sr(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Xu(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=Sr(n):(i&=o,i!==0?r=Sr(i):a||(a=o&~e,a!==0&&(r=Sr(a))))):(o=n&~s,o!==0?r=Sr(o):i!==0?r=Sr(i):a||(a=n&~e,a!==0&&(r=Sr(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function Uo(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function gC(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function Cy(){var e=Ql;return Ql<<=1,(Ql&4194048)===0&&(Ql=256),e}function Ey(){var e=Vl;return Vl<<=1,(Vl&62914560)===0&&(Vl=4194304),e}function Zd(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function jo(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function yC(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,l=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Zt(a),m=1<<d;o[d]=0,l[d]=-1;var f=c[d];if(f!==null)for(c[d]=null,d=0;d<f.length;d++){var h=f[d];h!==null&&(h.lane&=-536870913)}a&=~m}n!==0&&Ty(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function Ty(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Zt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function Ay(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Zt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function Cf(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function Ef(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function Dy(){var e=be.p;return e!==0?e:(e=window.event,e===void 0?32:Xx(e.type))}function bC(e,t){var a=be.p;try{return be.p=e,t()}finally{be.p=a}}var or=Math.random().toString(36).slice(2),St="__reactFiber$"+or,qt="__reactProps$"+or,Gs="__reactContainer$"+or,Um="__reactEvents$"+or,xC="__reactListeners$"+or,$C="__reactHandles$"+or,eg="__reactResources$"+or,Fo="__reactMarker$"+or;function Tf(e){delete e[St],delete e[qt],delete e[Um],delete e[xC],delete e[$C]}function bs(e){var t=e[St];if(t)return t;for(var a=e.parentNode;a;){if(t=a[Gs]||a[St]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=ly(e);e!==null;){if(a=e[St])return a;e=ly(e)}return t}e=a,a=e.parentNode}return null}function Ys(e){if(e=e[St]||e[Gs]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function ro(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(j(33))}function Es(e){var t=e[eg];return t||(t=e[eg]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function ct(e){e[Fo]=!0}var My=new Set,Oy={};function Ur(e,t){js(e,t),js(e+"Capture",t)}function js(e,t){for(Oy[e]=t,e=0;e<t.length;e++)My.add(t[e])}var wC=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),tg={},ag={};function SC(e){return Pm.call(ag,e)?!0:Pm.call(tg,e)?!1:wC.test(e)?ag[e]=!0:(tg[e]=!0,!1)}function lu(e,t,a){if(SC(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function Gl(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function cn(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var em,ng;function ps(e){if(em===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);em=t&&t[1]||"",ng=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+em+e+ng}var tm=!1;function am(e,t){if(!e||tm)return"";tm=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var m=function(){throw Error()};if(Object.defineProperty(m.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(m,[])}catch(h){var f=h}Reflect.construct(e,[],m)}else{try{m.call()}catch(h){f=h}e.call(m.prototype)}}else{try{throw Error()}catch(h){f=h}(m=e())&&typeof m.catch=="function"&&m.catch(function(){})}}catch(h){if(h&&f&&typeof h.stack=="string")return[h.stack,f.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var l=i.split(`
`),c=o.split(`
`);for(r=n=0;n<l.length&&!l[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===l.length||r===c.length)for(n=l.length-1,r=c.length-1;1<=n&&0<=r&&l[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(l[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||l[n]!==c[r]){var d=`
`+l[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{tm=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?ps(a):""}function NC(e){switch(e.tag){case 26:case 27:case 5:return ps(e.type);case 16:return ps("Lazy");case 13:return ps("Suspense");case 19:return ps("SuspenseList");case 0:case 15:return am(e.type,!1);case 11:return am(e.type.render,!1);case 1:return am(e.type,!0);case 31:return ps("Activity");default:return""}}function rg(e){try{var t="";do t+=NC(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function pa(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function Ly(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function _C(e){var t=Ly(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function _u(e){e._valueTracker||(e._valueTracker=_C(e))}function Py(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=Ly(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function Ru(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var RC=/[\n"\\]/g;function ga(e){return e.replace(RC,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function jm(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+pa(t)):e.value!==""+pa(t)&&(e.value=""+pa(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?Fm(e,i,pa(t)):a!=null?Fm(e,i,pa(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+pa(o):e.removeAttribute("name")}function Uy(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+pa(a):"",t=t!=null?""+pa(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function Fm(e,t,a){t==="number"&&Ru(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function Ts(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+pa(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function jy(e,t,a){if(t!=null&&(t=""+pa(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+pa(a):""}function Fy(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(j(92));if(no(n)){if(1<n.length)throw Error(j(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=pa(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function Fs(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var kC=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function sg(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||kC.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function By(e,t,a){if(t!=null&&typeof t!="object")throw Error(j(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&sg(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&sg(e,s,t[s])}function Af(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var CC=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),EC=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function uu(e){return EC.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var Bm=null;function Df(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var xs=null,As=null;function ig(e){var t=Ys(e);if(t&&(e=t.stateNode)){var a=e[qt]||null;e:switch(e=t.stateNode,t.type){case"input":if(jm(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+ga(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[qt]||null;if(!r)throw Error(j(90));jm(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&Py(n)}break e;case"textarea":jy(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&Ts(e,!!a.multiple,t,!1)}}}var nm=!1;function zy(e,t,a){if(nm)return e(t,a);nm=!0;try{var n=e(t);return n}finally{if(nm=!1,(xs!==null||As!==null)&&(oc(),xs&&(t=xs,e=As,As=xs=null,ig(t),e)))for(t=0;t<e.length;t++)ig(e[t])}}function wo(e,t){var a=e.stateNode;if(a===null)return null;var n=a[qt]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(j(231,t,typeof a));return a}var $n=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),zm=!1;if($n)try{ds={},Object.defineProperty(ds,"passive",{get:function(){zm=!0}}),window.addEventListener("test",ds,ds),window.removeEventListener("test",ds,ds)}catch{zm=!1}var ds,Vn=null,Mf=null,cu=null;function qy(){if(cu)return cu;var e,t=Mf,a=t.length,n,r="value"in Vn?Vn.value:Vn.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return cu=r.slice(e,1<n?1-n:void 0)}function du(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function Yl(){return!0}function og(){return!1}function It(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?Yl:og,this.isPropagationStopped=og,this}return Me(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=Yl)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=Yl)},persist:function(){},isPersistent:Yl}),t}var jr={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Wu=It(jr),Bo=Me({},jr,{view:0,detail:0}),TC=It(Bo),rm,sm,Yi,Zu=Me({},Bo,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:Of,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==Yi&&(Yi&&e.type==="mousemove"?(rm=e.screenX-Yi.screenX,sm=e.screenY-Yi.screenY):sm=rm=0,Yi=e),rm)},movementY:function(e){return"movementY"in e?e.movementY:sm}}),lg=It(Zu),AC=Me({},Zu,{dataTransfer:0}),DC=It(AC),MC=Me({},Bo,{relatedTarget:0}),im=It(MC),OC=Me({},jr,{animationName:0,elapsedTime:0,pseudoElement:0}),LC=It(OC),PC=Me({},jr,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),UC=It(PC),jC=Me({},jr,{data:0}),ug=It(jC),FC={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},BC={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},zC={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function qC(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=zC[e])?!!t[e]:!1}function Of(){return qC}var IC=Me({},Bo,{key:function(e){if(e.key){var t=FC[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=du(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?BC[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:Of,charCode:function(e){return e.type==="keypress"?du(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?du(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),HC=It(IC),KC=Me({},Zu,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),cg=It(KC),QC=Me({},Bo,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:Of}),VC=It(QC),GC=Me({},jr,{propertyName:0,elapsedTime:0,pseudoElement:0}),YC=It(GC),JC=Me({},Zu,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),XC=It(JC),WC=Me({},jr,{newState:0,oldState:0}),ZC=It(WC),eE=[9,13,27,32],Lf=$n&&"CompositionEvent"in window,io=null;$n&&"documentMode"in document&&(io=document.documentMode);var tE=$n&&"TextEvent"in window&&!io,Iy=$n&&(!Lf||io&&8<io&&11>=io),dg=" ",mg=!1;function Hy(e,t){switch(e){case"keyup":return eE.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function Ky(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var $s=!1;function aE(e,t){switch(e){case"compositionend":return Ky(t);case"keypress":return t.which!==32?null:(mg=!0,dg);case"textInput":return e=t.data,e===dg&&mg?null:e;default:return null}}function nE(e,t){if($s)return e==="compositionend"||!Lf&&Hy(e,t)?(e=qy(),cu=Mf=Vn=null,$s=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return Iy&&t.locale!=="ko"?null:t.data;default:return null}}var rE={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function fg(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!rE[e.type]:t==="textarea"}function Qy(e,t,a,n){xs?As?As.push(n):As=[n]:xs=n,t=Hu(t,"onChange"),0<t.length&&(a=new Wu("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var oo=null,So=null;function sE(e){jx(e,0)}function ec(e){var t=ro(e);if(Py(t))return e}function pg(e,t){if(e==="change")return t}var Vy=!1;$n&&($n?(Xl="oninput"in document,Xl||(om=document.createElement("div"),om.setAttribute("oninput","return;"),Xl=typeof om.oninput=="function"),Jl=Xl):Jl=!1,Vy=Jl&&(!document.documentMode||9<document.documentMode));var Jl,Xl,om;function hg(){oo&&(oo.detachEvent("onpropertychange",Gy),So=oo=null)}function Gy(e){if(e.propertyName==="value"&&ec(So)){var t=[];Qy(t,So,e,Df(e)),zy(sE,t)}}function iE(e,t,a){e==="focusin"?(hg(),oo=t,So=a,oo.attachEvent("onpropertychange",Gy)):e==="focusout"&&hg()}function oE(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return ec(So)}function lE(e,t){if(e==="click")return ec(t)}function uE(e,t){if(e==="input"||e==="change")return ec(t)}function cE(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var aa=typeof Object.is=="function"?Object.is:cE;function No(e,t){if(aa(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!Pm.call(t,r)||!aa(e[r],t[r]))return!1}return!0}function vg(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function gg(e,t){var a=vg(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=vg(a)}}function Yy(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?Yy(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function Jy(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=Ru(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=Ru(e.document)}return t}function Pf(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var dE=$n&&"documentMode"in document&&11>=document.documentMode,ws=null,qm=null,lo=null,Im=!1;function yg(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;Im||ws==null||ws!==Ru(n)||(n=ws,"selectionStart"in n&&Pf(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),lo&&No(lo,n)||(lo=n,n=Hu(qm,"onSelect"),0<n.length&&(t=new Wu("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=ws)))}function wr(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var Ss={animationend:wr("Animation","AnimationEnd"),animationiteration:wr("Animation","AnimationIteration"),animationstart:wr("Animation","AnimationStart"),transitionrun:wr("Transition","TransitionRun"),transitionstart:wr("Transition","TransitionStart"),transitioncancel:wr("Transition","TransitionCancel"),transitionend:wr("Transition","TransitionEnd")},lm={},Xy={};$n&&(Xy=document.createElement("div").style,"AnimationEvent"in window||(delete Ss.animationend.animation,delete Ss.animationiteration.animation,delete Ss.animationstart.animation),"TransitionEvent"in window||delete Ss.transitionend.transition);function Fr(e){if(lm[e])return lm[e];if(!Ss[e])return e;var t=Ss[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in Xy)return lm[e]=t[a];return e}var Wy=Fr("animationend"),Zy=Fr("animationiteration"),eb=Fr("animationstart"),mE=Fr("transitionrun"),fE=Fr("transitionstart"),pE=Fr("transitioncancel"),tb=Fr("transitionend"),ab=new Map,Hm="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");Hm.push("scrollEnd");function Ea(e,t){ab.set(e,t),Ur(t,[e])}var bg=new WeakMap;function ya(e,t){if(typeof e=="object"&&e!==null){var a=bg.get(e);return a!==void 0?a:(t={value:e,source:t,stack:rg(t)},bg.set(e,t),t)}return{value:e,source:t,stack:rg(t)}}var fa=[],Ns=0,Uf=0;function tc(){for(var e=Ns,t=Uf=Ns=0;t<e;){var a=fa[t];fa[t++]=null;var n=fa[t];fa[t++]=null;var r=fa[t];fa[t++]=null;var s=fa[t];if(fa[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&nb(a,r,s)}}function ac(e,t,a,n){fa[Ns++]=e,fa[Ns++]=t,fa[Ns++]=a,fa[Ns++]=n,Uf|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function jf(e,t,a,n){return ac(e,t,a,n),ku(e)}function Js(e,t){return ac(e,null,null,t),ku(e)}function nb(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Zt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function ku(e){if(50<bo)throw bo=0,mf=null,Error(j(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var _s={};function hE(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Xt(e,t,a,n){return new hE(e,t,a,n)}function Ff(e){return e=e.prototype,!(!e||!e.isReactComponent)}function bn(e,t){var a=e.alternate;return a===null?(a=Xt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function rb(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function mu(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")Ff(e)&&(i=1);else if(typeof e=="string")i=h3(e,a,Va.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case Dm:return e=Xt(31,a,t,r),e.elementType=Dm,e.lanes=s,e;case gs:return kr(a.children,r,s,t);case Sy:i=8,r|=24;break;case Em:return e=Xt(12,a,t,r|2),e.elementType=Em,e.lanes=s,e;case Tm:return e=Xt(13,a,t,r),e.elementType=Tm,e.lanes=s,e;case Am:return e=Xt(19,a,t,r),e.elementType=Am,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case sC:case pn:i=10;break e;case Ny:i=9;break e;case _f:i=11;break e;case Rf:i=14;break e;case Bn:i=16,n=null;break e}i=29,a=Error(j(130,e===null?"null":typeof e,"")),n=null}return t=Xt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function kr(e,t,a,n){return e=Xt(7,e,n,t),e.lanes=a,e}function um(e,t,a){return e=Xt(6,e,null,t),e.lanes=a,e}function cm(e,t,a){return t=Xt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var Rs=[],ks=0,Cu=null,Eu=0,ha=[],va=0,Cr=null,hn=1,vn="";function Nr(e,t){Rs[ks++]=Eu,Rs[ks++]=Cu,Cu=e,Eu=t}function sb(e,t,a){ha[va++]=hn,ha[va++]=vn,ha[va++]=Cr,Cr=e;var n=hn;e=vn;var r=32-Zt(n)-1;n&=~(1<<r),a+=1;var s=32-Zt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,hn=1<<32-Zt(t)+r|a<<r|n,vn=s+e}else hn=1<<s|a<<r|n,vn=e}function Bf(e){e.return!==null&&(Nr(e,1),sb(e,1,0))}function zf(e){for(;e===Cu;)Cu=Rs[--ks],Rs[ks]=null,Eu=Rs[--ks],Rs[ks]=null;for(;e===Cr;)Cr=ha[--va],ha[va]=null,vn=ha[--va],ha[va]=null,hn=ha[--va],ha[va]=null}var At=null,Ie=null,ye=!1,Er=null,Ka=!1,Km=Error(j(519));function Mr(e){var t=Error(j(418,""));throw _o(ya(t,e)),Km}function xg(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[St]=e,t[qt]=n,a){case"dialog":de("cancel",t),de("close",t);break;case"iframe":case"object":case"embed":de("load",t);break;case"video":case"audio":for(a=0;a<Co.length;a++)de(Co[a],t);break;case"source":de("error",t);break;case"img":case"image":case"link":de("error",t),de("load",t);break;case"details":de("toggle",t);break;case"input":de("invalid",t),Uy(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),_u(t);break;case"select":de("invalid",t);break;case"textarea":de("invalid",t),Fy(t,n.value,n.defaultValue,n.children),_u(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||Bx(t.textContent,a)?(n.popover!=null&&(de("beforetoggle",t),de("toggle",t)),n.onScroll!=null&&de("scroll",t),n.onScrollEnd!=null&&de("scrollend",t),n.onClick!=null&&(t.onclick=cc),t=!0):t=!1,t||Mr(e)}function $g(e){for(At=e.return;At;)switch(At.tag){case 5:case 13:Ka=!1;return;case 27:case 3:Ka=!0;return;default:At=At.return}}function Ji(e){if(e!==At)return!1;if(!ye)return $g(e),ye=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||yf(e.type,e.memoizedProps)),a=!a),a&&Ie&&Mr(e),$g(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(j(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){Ie=Ca(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}Ie=null}}else t===27?(t=Ie,lr(e.type)?(e=$f,$f=null,Ie=e):Ie=t):Ie=At?Ca(e.stateNode.nextSibling):null;return!0}function zo(){Ie=At=null,ye=!1}function wg(){var e=Er;return e!==null&&(zt===null?zt=e:zt.push.apply(zt,e),Er=null),e}function _o(e){Er===null?Er=[e]:Er.push(e)}var Qm=Ja(null),Br=null,gn=null;function qn(e,t,a){Fe(Qm,t._currentValue),t._currentValue=a}function xn(e){e._currentValue=Qm.current,mt(Qm)}function Vm(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function Gm(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var l=0;l<t.length;l++)if(o.context===t[l]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),Vm(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(j(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),Vm(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function qo(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(j(387));if(i=i.memoizedProps,i!==null){var o=r.type;aa(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===$u.current){if(i=r.alternate,i===null)throw Error(j(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(Ao):e=[Ao])}r=r.return}e!==null&&Gm(t,e,a,n),t.flags|=262144}function Tu(e){for(e=e.firstContext;e!==null;){if(!aa(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function Or(e){Br=e,gn=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function Nt(e){return ib(Br,e)}function Wl(e,t){return Br===null&&Or(e),ib(e,t)}function ib(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},gn===null){if(e===null)throw Error(j(308));gn=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else gn=gn.next=t;return a}var vE=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},gE=st.unstable_scheduleCallback,yE=st.unstable_NormalPriority,nt={$$typeof:pn,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function qf(){return{controller:new vE,data:new Map,refCount:0}}function Io(e){e.refCount--,e.refCount===0&&gE(yE,function(){e.controller.abort()})}var uo=null,Ym=0,Bs=0,Ds=null;function bE(e,t){if(uo===null){var a=uo=[];Ym=0,Bs=dp(),Ds={status:"pending",value:void 0,then:function(n){a.push(n)}}}return Ym++,t.then(Sg,Sg),t}function Sg(){if(--Ym===0&&uo!==null){Ds!==null&&(Ds.status="fulfilled");var e=uo;uo=null,Bs=0,Ds=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function xE(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var Ng=se.S;se.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&bE(e,t),Ng!==null&&Ng(e,t)};var Tr=Ja(null);function If(){var e=Tr.current;return e!==null?e:Ee.pooledCache}function fu(e,t){t===null?Fe(Tr,Tr.current):Fe(Tr,t.pool)}function ob(){var e=If();return e===null?null:{parent:nt._currentValue,pool:e}}var Ho=Error(j(460)),lb=Error(j(474)),nc=Error(j(542)),Jm={then:function(){}};function _g(e){return e=e.status,e==="fulfilled"||e==="rejected"}function Zl(){}function ub(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(Zl,Zl),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,kg(e),e;default:if(typeof t.status=="string")t.then(Zl,Zl);else{if(e=Ee,e!==null&&100<e.shellSuspendCounter)throw Error(j(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,kg(e),e}throw co=t,Ho}}var co=null;function Rg(){if(co===null)throw Error(j(459));var e=co;return co=null,e}function kg(e){if(e===Ho||e===nc)throw Error(j(483))}var zn=!1;function Hf(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function Xm(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function Xn(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function Wn(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,(Se&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=ku(e),nb(e,null,a),t}return ac(e,n,t,a),ku(e)}function mo(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,Ay(e,a)}}function dm(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var Wm=!1;function fo(){if(Wm){var e=Ds;if(e!==null)throw e}}function po(e,t,a,n){Wm=!1;var r=e.updateQueue;zn=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var l=o,c=l.next;l.next=null,i===null?s=c:i.next=c,i=l;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=l))}if(s!==null){var m=r.baseState;i=0,d=c=l=null,o=s;do{var f=o.lane&-536870913,h=f!==o.lane;if(h?(he&f)===f:(n&f)===f){f!==0&&f===Bs&&(Wm=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var x=e,y=o;f=t;var $=a;switch(y.tag){case 1:if(x=y.payload,typeof x=="function"){m=x.call($,m,f);break e}m=x;break e;case 3:x.flags=x.flags&-65537|128;case 0:if(x=y.payload,f=typeof x=="function"?x.call($,m,f):x,f==null)break e;m=Me({},m,f);break e;case 2:zn=!0}}f=o.callback,f!==null&&(e.flags|=64,h&&(e.flags|=8192),h=r.callbacks,h===null?r.callbacks=[f]:h.push(f))}else h={lane:f,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=h,l=m):d=d.next=h,i|=f;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;h=o,o=h.next,h.next=null,r.lastBaseUpdate=h,r.shared.pending=null}}while(!0);d===null&&(l=m),r.baseState=l,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),ir|=i,e.lanes=i,e.memoizedState=m}}function cb(e,t){if(typeof e!="function")throw Error(j(191,e));e.call(t)}function db(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)cb(a[e],t)}var zs=Ja(null),Au=Ja(0);function Cg(e,t){e=Nn,Fe(Au,e),Fe(zs,t),Nn=e|t.baseLanes}function Zm(){Fe(Au,Nn),Fe(zs,zs.current)}function Kf(){Nn=Au.current,mt(zs),mt(Au)}var rr=0,ue=null,_e=null,Je=null,Du=!1,Ms=!1,Lr=!1,Mu=0,Ro=0,Os=null,$E=0;function Ve(){throw Error(j(321))}function Qf(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!aa(e[a],t[a]))return!1;return!0}function Vf(e,t,a,n,r,s){return rr=s,ue=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,se.H=e===null||e.memoizedState===null?qb:Ib,Lr=!1,s=a(n,r),Lr=!1,Ms&&(s=fb(t,a,n,r)),mb(e),s}function mb(e){se.H=Ou;var t=_e!==null&&_e.next!==null;if(rr=0,Je=_e=ue=null,Du=!1,Ro=0,Os=null,t)throw Error(j(300));e===null||dt||(e=e.dependencies,e!==null&&Tu(e)&&(dt=!0))}function fb(e,t,a,n){ue=e;var r=0;do{if(Ms&&(Os=null),Ro=0,Ms=!1,25<=r)throw Error(j(301));if(r+=1,Je=_e=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}se.H=CE,s=t(a,n)}while(Ms);return s}function wE(){var e=se.H,t=e.useState()[0];return t=typeof t.then=="function"?Ko(t):t,e=e.useState()[0],(_e!==null?_e.memoizedState:null)!==e&&(ue.flags|=1024),t}function Gf(){var e=Mu!==0;return Mu=0,e}function Yf(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function Jf(e){if(Du){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}Du=!1}rr=0,Je=_e=ue=null,Ms=!1,Ro=Mu=0,Os=null}function Ft(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return Je===null?ue.memoizedState=Je=e:Je=Je.next=e,Je}function Xe(){if(_e===null){var e=ue.alternate;e=e!==null?e.memoizedState:null}else e=_e.next;var t=Je===null?ue.memoizedState:Je.next;if(t!==null)Je=t,_e=e;else{if(e===null)throw ue.alternate===null?Error(j(467)):Error(j(310));_e=e,e={memoizedState:_e.memoizedState,baseState:_e.baseState,baseQueue:_e.baseQueue,queue:_e.queue,next:null},Je===null?ue.memoizedState=Je=e:Je=Je.next=e}return Je}function Xf(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function Ko(e){var t=Ro;return Ro+=1,Os===null&&(Os=[]),e=ub(Os,e,t),t=ue,(Je===null?t.memoizedState:Je.next)===null&&(t=t.alternate,se.H=t===null||t.memoizedState===null?qb:Ib),e}function rc(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return Ko(e);if(e.$$typeof===pn)return Nt(e)}throw Error(j(438,String(e)))}function Wf(e){var t=null,a=ue.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=ue.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=Xf(),ue.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=iC;return t.index++,a}function wn(e,t){return typeof t=="function"?t(e):t}function pu(e){var t=Xe();return Zf(t,_e,e)}function Zf(e,t,a){var n=e.queue;if(n===null)throw Error(j(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,l=null,c=t,d=!1;do{var m=c.lane&-536870913;if(m!==c.lane?(he&m)===m:(rr&m)===m){var f=c.revertLane;if(f===0)l!==null&&(l=l.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),m===Bs&&(d=!0);else if((rr&f)===f){c=c.next,f===Bs&&(d=!0);continue}else m={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},l===null?(o=l=m,i=s):l=l.next=m,ue.lanes|=f,ir|=f;m=c.action,Lr&&a(s,m),s=c.hasEagerState?c.eagerState:a(s,m)}else f={lane:m,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},l===null?(o=l=f,i=s):l=l.next=f,ue.lanes|=m,ir|=m;c=c.next}while(c!==null&&c!==t);if(l===null?i=s:l.next=o,!aa(s,e.memoizedState)&&(dt=!0,d&&(a=Ds,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=l,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function mm(e){var t=Xe(),a=t.queue;if(a===null)throw Error(j(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);aa(s,t.memoizedState)||(dt=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function pb(e,t,a){var n=ue,r=Xe(),s=ye;if(s){if(a===void 0)throw Error(j(407));a=a()}else a=t();var i=!aa((_e||r).memoizedState,a);i&&(r.memoizedState=a,dt=!0),r=r.queue;var o=gb.bind(null,n,r,e);if(Qo(2048,8,o,[e]),r.getSnapshot!==t||i||Je!==null&&Je.memoizedState.tag&1){if(n.flags|=2048,qs(9,sc(),vb.bind(null,n,r,a,t),null),Ee===null)throw Error(j(349));s||(rr&124)!==0||hb(n,t,a)}return a}function hb(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=ue.updateQueue,t===null?(t=Xf(),ue.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function vb(e,t,a,n){t.value=a,t.getSnapshot=n,yb(t)&&bb(e)}function gb(e,t,a){return a(function(){yb(t)&&bb(e)})}function yb(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!aa(e,a)}catch{return!0}}function bb(e){var t=Js(e,2);t!==null&&ta(t,e,2)}function ef(e){var t=Ft();if(typeof e=="function"){var a=e;if(e=a(),Lr){Qn(!0);try{a()}finally{Qn(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:wn,lastRenderedState:e},t}function xb(e,t,a,n){return e.baseState=a,Zf(e,_e,typeof n=="function"?n:wn)}function SE(e,t,a,n,r){if(ic(e))throw Error(j(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};se.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,$b(t,s)):(s.next=a.next,t.pending=a.next=s)}}function $b(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=se.T,i={};se.T=i;try{var o=a(r,n),l=se.S;l!==null&&l(i,o),Eg(e,t,o)}catch(c){tf(e,t,c)}finally{se.T=s}}else try{s=a(r,n),Eg(e,t,s)}catch(c){tf(e,t,c)}}function Eg(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){Tg(e,t,n)},function(n){return tf(e,t,n)}):Tg(e,t,a)}function Tg(e,t,a){t.status="fulfilled",t.value=a,wb(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,$b(e,a)))}function tf(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,wb(t),t=t.next;while(t!==n)}e.action=null}function wb(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function Sb(e,t){return t}function Ag(e,t){if(ye){var a=Ee.formState;if(a!==null){e:{var n=ue;if(ye){if(Ie){t:{for(var r=Ie,s=Ka;r.nodeType!==8;){if(!s){r=null;break t}if(r=Ca(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){Ie=Ca(r.nextSibling),n=r.data==="F!";break e}}Mr(n)}n=!1}n&&(t=a[0])}}return a=Ft(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:Sb,lastRenderedState:t},a.queue=n,a=Fb.bind(null,ue,n),n.dispatch=a,n=ef(!1),s=np.bind(null,ue,!1,n.queue),n=Ft(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=SE.bind(null,ue,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function Dg(e){var t=Xe();return Nb(t,_e,e)}function Nb(e,t,a){if(t=Zf(e,t,Sb)[0],e=pu(wn)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=Ko(t)}catch(i){throw i===Ho?nc:i}else n=t;t=Xe();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(ue.flags|=2048,qs(9,sc(),NE.bind(null,r,a),null)),[n,s,e]}function NE(e,t){e.action=t}function Mg(e){var t=Xe(),a=_e;if(a!==null)return Nb(t,a,e);Xe(),t=t.memoizedState,a=Xe();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function qs(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=ue.updateQueue,t===null&&(t=Xf(),ue.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function sc(){return{destroy:void 0,resource:void 0}}function _b(){return Xe().memoizedState}function hu(e,t,a,n){var r=Ft();n=n===void 0?null:n,ue.flags|=e,r.memoizedState=qs(1|t,sc(),a,n)}function Qo(e,t,a,n){var r=Xe();n=n===void 0?null:n;var s=r.memoizedState.inst;_e!==null&&n!==null&&Qf(n,_e.memoizedState.deps)?r.memoizedState=qs(t,s,a,n):(ue.flags|=e,r.memoizedState=qs(1|t,s,a,n))}function Og(e,t){hu(8390656,8,e,t)}function Rb(e,t){Qo(2048,8,e,t)}function kb(e,t){return Qo(4,2,e,t)}function Cb(e,t){return Qo(4,4,e,t)}function Eb(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function Tb(e,t,a){a=a!=null?a.concat([e]):null,Qo(4,4,Eb.bind(null,t,e),a)}function ep(){}function Ab(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&Qf(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function Db(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&Qf(t,n[1]))return n[0];if(n=e(),Lr){Qn(!0);try{e()}finally{Qn(!1)}}return a.memoizedState=[n,t],n}function tp(e,t,a){return a===void 0||(rr&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=wx(),ue.lanes|=e,ir|=e,a)}function Mb(e,t,a,n){return aa(a,t)?a:zs.current!==null?(e=tp(e,a,n),aa(e,t)||(dt=!0),e):(rr&42)===0?(dt=!0,e.memoizedState=a):(e=wx(),ue.lanes|=e,ir|=e,t)}function Ob(e,t,a,n,r){var s=be.p;be.p=s!==0&&8>s?s:8;var i=se.T,o={};se.T=o,np(e,!1,t,a);try{var l=r(),c=se.S;if(c!==null&&c(o,l),l!==null&&typeof l=="object"&&typeof l.then=="function"){var d=xE(l,n);ho(e,t,d,ea(e))}else ho(e,t,n,ea(e))}catch(m){ho(e,t,{then:function(){},status:"rejected",reason:m},ea())}finally{be.p=s,se.T=i}}function _E(){}function af(e,t,a,n){if(e.tag!==5)throw Error(j(476));var r=Lb(e).queue;Ob(e,r,t,Rr,a===null?_E:function(){return Pb(e),a(n)})}function Lb(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:Rr,baseState:Rr,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:wn,lastRenderedState:Rr},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:wn,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function Pb(e){var t=Lb(e).next.queue;ho(e,t,{},ea())}function ap(){return Nt(Ao)}function Ub(){return Xe().memoizedState}function jb(){return Xe().memoizedState}function RE(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=ea();e=Xn(a);var n=Wn(t,e,a);n!==null&&(ta(n,t,a),mo(n,t,a)),t={cache:qf()},e.payload=t;return}t=t.return}}function kE(e,t,a){var n=ea();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},ic(e)?Bb(t,a):(a=jf(e,t,a,n),a!==null&&(ta(a,e,n),zb(a,t,n)))}function Fb(e,t,a){var n=ea();ho(e,t,a,n)}function ho(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(ic(e))Bb(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,aa(o,i))return ac(e,t,r,0),Ee===null&&tc(),!1}catch{}finally{}if(a=jf(e,t,r,n),a!==null)return ta(a,e,n),zb(a,t,n),!0}return!1}function np(e,t,a,n){if(n={lane:2,revertLane:dp(),action:n,hasEagerState:!1,eagerState:null,next:null},ic(e)){if(t)throw Error(j(479))}else t=jf(e,a,n,2),t!==null&&ta(t,e,2)}function ic(e){var t=e.alternate;return e===ue||t!==null&&t===ue}function Bb(e,t){Ms=Du=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function zb(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,Ay(e,a)}}var Ou={readContext:Nt,use:rc,useCallback:Ve,useContext:Ve,useEffect:Ve,useImperativeHandle:Ve,useLayoutEffect:Ve,useInsertionEffect:Ve,useMemo:Ve,useReducer:Ve,useRef:Ve,useState:Ve,useDebugValue:Ve,useDeferredValue:Ve,useTransition:Ve,useSyncExternalStore:Ve,useId:Ve,useHostTransitionStatus:Ve,useFormState:Ve,useActionState:Ve,useOptimistic:Ve,useMemoCache:Ve,useCacheRefresh:Ve},qb={readContext:Nt,use:rc,useCallback:function(e,t){return Ft().memoizedState=[e,t===void 0?null:t],e},useContext:Nt,useEffect:Og,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,hu(4194308,4,Eb.bind(null,t,e),a)},useLayoutEffect:function(e,t){return hu(4194308,4,e,t)},useInsertionEffect:function(e,t){hu(4,2,e,t)},useMemo:function(e,t){var a=Ft();t=t===void 0?null:t;var n=e();if(Lr){Qn(!0);try{e()}finally{Qn(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=Ft();if(a!==void 0){var r=a(t);if(Lr){Qn(!0);try{a(t)}finally{Qn(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=kE.bind(null,ue,e),[n.memoizedState,e]},useRef:function(e){var t=Ft();return e={current:e},t.memoizedState=e},useState:function(e){e=ef(e);var t=e.queue,a=Fb.bind(null,ue,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:ep,useDeferredValue:function(e,t){var a=Ft();return tp(a,e,t)},useTransition:function(){var e=ef(!1);return e=Ob.bind(null,ue,e.queue,!0,!1),Ft().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=ue,r=Ft();if(ye){if(a===void 0)throw Error(j(407));a=a()}else{if(a=t(),Ee===null)throw Error(j(349));(he&124)!==0||hb(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,Og(gb.bind(null,n,s,e),[e]),n.flags|=2048,qs(9,sc(),vb.bind(null,n,s,a,t),null),a},useId:function(){var e=Ft(),t=Ee.identifierPrefix;if(ye){var a=vn,n=hn;a=(n&~(1<<32-Zt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=Mu++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=$E++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:ap,useFormState:Ag,useActionState:Ag,useOptimistic:function(e){var t=Ft();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=np.bind(null,ue,!0,a),a.dispatch=t,[e,t]},useMemoCache:Wf,useCacheRefresh:function(){return Ft().memoizedState=RE.bind(null,ue)}},Ib={readContext:Nt,use:rc,useCallback:Ab,useContext:Nt,useEffect:Rb,useImperativeHandle:Tb,useInsertionEffect:kb,useLayoutEffect:Cb,useMemo:Db,useReducer:pu,useRef:_b,useState:function(){return pu(wn)},useDebugValue:ep,useDeferredValue:function(e,t){var a=Xe();return Mb(a,_e.memoizedState,e,t)},useTransition:function(){var e=pu(wn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:Ko(e),t]},useSyncExternalStore:pb,useId:Ub,useHostTransitionStatus:ap,useFormState:Dg,useActionState:Dg,useOptimistic:function(e,t){var a=Xe();return xb(a,_e,e,t)},useMemoCache:Wf,useCacheRefresh:jb},CE={readContext:Nt,use:rc,useCallback:Ab,useContext:Nt,useEffect:Rb,useImperativeHandle:Tb,useInsertionEffect:kb,useLayoutEffect:Cb,useMemo:Db,useReducer:mm,useRef:_b,useState:function(){return mm(wn)},useDebugValue:ep,useDeferredValue:function(e,t){var a=Xe();return _e===null?tp(a,e,t):Mb(a,_e.memoizedState,e,t)},useTransition:function(){var e=mm(wn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:Ko(e),t]},useSyncExternalStore:pb,useId:Ub,useHostTransitionStatus:ap,useFormState:Mg,useActionState:Mg,useOptimistic:function(e,t){var a=Xe();return _e!==null?xb(a,_e,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:Wf,useCacheRefresh:jb},Ls=null,ko=0;function eu(e){var t=ko;return ko+=1,Ls===null&&(Ls=[]),ub(Ls,e,t)}function Xi(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function tu(e,t){throw t.$$typeof===rC?Error(j(525)):(e=Object.prototype.toString.call(t),Error(j(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function Lg(e){var t=e._init;return t(e._payload)}function Hb(e){function t(g,v){if(e){var b=g.deletions;b===null?(g.deletions=[v],g.flags|=16):b.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=bn(g,v),g.index=0,g.sibling=null,g}function s(g,v,b){return g.index=b,e?(b=g.alternate,b!==null?(b=b.index,b<v?(g.flags|=67108866,v):b):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,b,w){return v===null||v.tag!==6?(v=um(b,g.mode,w),v.return=g,v):(v=r(v,b),v.return=g,v)}function l(g,v,b,w){var S=b.type;return S===gs?d(g,v,b.props.children,w,b.key):v!==null&&(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Bn&&Lg(S)===v.type)?(v=r(v,b.props),Xi(v,b),v.return=g,v):(v=mu(b.type,b.key,b.props,null,g.mode,w),Xi(v,b),v.return=g,v)}function c(g,v,b,w){return v===null||v.tag!==4||v.stateNode.containerInfo!==b.containerInfo||v.stateNode.implementation!==b.implementation?(v=cm(b,g.mode,w),v.return=g,v):(v=r(v,b.children||[]),v.return=g,v)}function d(g,v,b,w,S){return v===null||v.tag!==7?(v=kr(b,g.mode,w,S),v.return=g,v):(v=r(v,b),v.return=g,v)}function m(g,v,b){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=um(""+v,g.mode,b),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case Kl:return b=mu(v.type,v.key,v.props,null,g.mode,b),Xi(b,v),b.return=g,b;case ao:return v=cm(v,g.mode,b),v.return=g,v;case Bn:var w=v._init;return v=w(v._payload),m(g,v,b)}if(no(v)||Gi(v))return v=kr(v,g.mode,b,null),v.return=g,v;if(typeof v.then=="function")return m(g,eu(v),b);if(v.$$typeof===pn)return m(g,Wl(g,v),b);tu(g,v)}return null}function f(g,v,b,w){var S=v!==null?v.key:null;if(typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint")return S!==null?null:o(g,v,""+b,w);if(typeof b=="object"&&b!==null){switch(b.$$typeof){case Kl:return b.key===S?l(g,v,b,w):null;case ao:return b.key===S?c(g,v,b,w):null;case Bn:return S=b._init,b=S(b._payload),f(g,v,b,w)}if(no(b)||Gi(b))return S!==null?null:d(g,v,b,w,null);if(typeof b.then=="function")return f(g,v,eu(b),w);if(b.$$typeof===pn)return f(g,v,Wl(g,b),w);tu(g,b)}return null}function h(g,v,b,w,S){if(typeof w=="string"&&w!==""||typeof w=="number"||typeof w=="bigint")return g=g.get(b)||null,o(v,g,""+w,S);if(typeof w=="object"&&w!==null){switch(w.$$typeof){case Kl:return g=g.get(w.key===null?b:w.key)||null,l(v,g,w,S);case ao:return g=g.get(w.key===null?b:w.key)||null,c(v,g,w,S);case Bn:var C=w._init;return w=C(w._payload),h(g,v,b,w,S)}if(no(w)||Gi(w))return g=g.get(b)||null,d(v,g,w,S,null);if(typeof w.then=="function")return h(g,v,b,eu(w),S);if(w.$$typeof===pn)return h(g,v,b,Wl(v,w),S);tu(v,w)}return null}function x(g,v,b,w){for(var S=null,C=null,R=v,_=v=0,A=null;R!==null&&_<b.length;_++){R.index>_?(A=R,R=null):A=R.sibling;var L=f(g,R,b[_],w);if(L===null){R===null&&(R=A);break}e&&R&&L.alternate===null&&t(g,R),v=s(L,v,_),C===null?S=L:C.sibling=L,C=L,R=A}if(_===b.length)return a(g,R),ye&&Nr(g,_),S;if(R===null){for(;_<b.length;_++)R=m(g,b[_],w),R!==null&&(v=s(R,v,_),C===null?S=R:C.sibling=R,C=R);return ye&&Nr(g,_),S}for(R=n(R);_<b.length;_++)A=h(R,g,_,b[_],w),A!==null&&(e&&A.alternate!==null&&R.delete(A.key===null?_:A.key),v=s(A,v,_),C===null?S=A:C.sibling=A,C=A);return e&&R.forEach(function(U){return t(g,U)}),ye&&Nr(g,_),S}function y(g,v,b,w){if(b==null)throw Error(j(151));for(var S=null,C=null,R=v,_=v=0,A=null,L=b.next();R!==null&&!L.done;_++,L=b.next()){R.index>_?(A=R,R=null):A=R.sibling;var U=f(g,R,L.value,w);if(U===null){R===null&&(R=A);break}e&&R&&U.alternate===null&&t(g,R),v=s(U,v,_),C===null?S=U:C.sibling=U,C=U,R=A}if(L.done)return a(g,R),ye&&Nr(g,_),S;if(R===null){for(;!L.done;_++,L=b.next())L=m(g,L.value,w),L!==null&&(v=s(L,v,_),C===null?S=L:C.sibling=L,C=L);return ye&&Nr(g,_),S}for(R=n(R);!L.done;_++,L=b.next())L=h(R,g,_,L.value,w),L!==null&&(e&&L.alternate!==null&&R.delete(L.key===null?_:L.key),v=s(L,v,_),C===null?S=L:C.sibling=L,C=L);return e&&R.forEach(function(F){return t(g,F)}),ye&&Nr(g,_),S}function $(g,v,b,w){if(typeof b=="object"&&b!==null&&b.type===gs&&b.key===null&&(b=b.props.children),typeof b=="object"&&b!==null){switch(b.$$typeof){case Kl:e:{for(var S=b.key;v!==null;){if(v.key===S){if(S=b.type,S===gs){if(v.tag===7){a(g,v.sibling),w=r(v,b.props.children),w.return=g,g=w;break e}}else if(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Bn&&Lg(S)===v.type){a(g,v.sibling),w=r(v,b.props),Xi(w,b),w.return=g,g=w;break e}a(g,v);break}else t(g,v);v=v.sibling}b.type===gs?(w=kr(b.props.children,g.mode,w,b.key),w.return=g,g=w):(w=mu(b.type,b.key,b.props,null,g.mode,w),Xi(w,b),w.return=g,g=w)}return i(g);case ao:e:{for(S=b.key;v!==null;){if(v.key===S)if(v.tag===4&&v.stateNode.containerInfo===b.containerInfo&&v.stateNode.implementation===b.implementation){a(g,v.sibling),w=r(v,b.children||[]),w.return=g,g=w;break e}else{a(g,v);break}else t(g,v);v=v.sibling}w=cm(b,g.mode,w),w.return=g,g=w}return i(g);case Bn:return S=b._init,b=S(b._payload),$(g,v,b,w)}if(no(b))return x(g,v,b,w);if(Gi(b)){if(S=Gi(b),typeof S!="function")throw Error(j(150));return b=S.call(b),y(g,v,b,w)}if(typeof b.then=="function")return $(g,v,eu(b),w);if(b.$$typeof===pn)return $(g,v,Wl(g,b),w);tu(g,b)}return typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint"?(b=""+b,v!==null&&v.tag===6?(a(g,v.sibling),w=r(v,b),w.return=g,g=w):(a(g,v),w=um(b,g.mode,w),w.return=g,g=w),i(g)):a(g,v)}return function(g,v,b,w){try{ko=0;var S=$(g,v,b,w);return Ls=null,S}catch(R){if(R===Ho||R===nc)throw R;var C=Xt(29,R,null,g.mode);return C.lanes=w,C.return=g,C}finally{}}}var Is=Hb(!0),Kb=Hb(!1),xa=Ja(null),Ya=null;function In(e){var t=e.alternate;Fe(rt,rt.current&1),Fe(xa,e),Ya===null&&(t===null||zs.current!==null||t.memoizedState!==null)&&(Ya=e)}function Qb(e){if(e.tag===22){if(Fe(rt,rt.current),Fe(xa,e),Ya===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(Ya=e)}}else Hn(e)}function Hn(){Fe(rt,rt.current),Fe(xa,xa.current)}function yn(e){mt(xa),Ya===e&&(Ya=null),mt(rt)}var rt=Ja(0);function Lu(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||xf(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function fm(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:Me({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var nf={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=ea(),r=Xn(n);r.payload=t,a!=null&&(r.callback=a),t=Wn(e,r,n),t!==null&&(ta(t,e,n),mo(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=ea(),r=Xn(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=Wn(e,r,n),t!==null&&(ta(t,e,n),mo(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=ea(),n=Xn(a);n.tag=2,t!=null&&(n.callback=t),t=Wn(e,n,a),t!==null&&(ta(t,e,a),mo(t,e,a))}};function Pg(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!No(a,n)||!No(r,s):!0}function Ug(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&nf.enqueueReplaceState(t,t.state,null)}function Pr(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=Me({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var Pu=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function Vb(e){Pu(e)}function Gb(e){console.error(e)}function Yb(e){Pu(e)}function Uu(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function jg(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function rf(e,t,a){return a=Xn(a),a.tag=3,a.payload={element:null},a.callback=function(){Uu(e,t)},a}function Jb(e){return e=Xn(e),e.tag=3,e}function Xb(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){jg(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){jg(t,a,n),typeof r!="function"&&(Zn===null?Zn=new Set([this]):Zn.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function EE(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&qo(t,a,r,!0),a=xa.current,a!==null){switch(a.tag){case 13:return Ya===null?ff():a.alternate===null&&He===0&&(He=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===Jm?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),Nm(e,n,r)),!1;case 22:return a.flags|=65536,n===Jm?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),Nm(e,n,r)),!1}throw Error(j(435,a.tag))}return Nm(e,n,r),ff(),!1}if(ye)return t=xa.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==Km&&(e=Error(j(422),{cause:n}),_o(ya(e,a)))):(n!==Km&&(t=Error(j(423),{cause:n}),_o(ya(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=ya(n,a),r=rf(e.stateNode,n,r),dm(e,r),He!==4&&(He=2)),!1;var s=Error(j(520),{cause:n});if(s=ya(s,a),yo===null?yo=[s]:yo.push(s),He!==4&&(He=2),t===null)return!0;n=ya(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=rf(a.stateNode,n,e),dm(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(Zn===null||!Zn.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=Jb(r),Xb(r,e,a,n),dm(a,r),!1}a=a.return}while(a!==null);return!1}var Wb=Error(j(461)),dt=!1;function yt(e,t,a,n){t.child=e===null?Kb(t,null,a,n):Is(t,e.child,a,n)}function Fg(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return Or(t),n=Vf(e,t,a,i,s,r),o=Gf(),e!==null&&!dt?(Yf(e,t,r),Sn(e,t,r)):(ye&&o&&Bf(t),t.flags|=1,yt(e,t,n,r),t.child)}function Bg(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!Ff(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,Zb(e,t,s,n,r)):(e=mu(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!rp(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:No,a(i,n)&&e.ref===t.ref)return Sn(e,t,r)}return t.flags|=1,e=bn(s,n),e.ref=t.ref,e.return=t,t.child=e}function Zb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if(No(s,n)&&e.ref===t.ref)if(dt=!1,t.pendingProps=n=s,rp(e,r))(e.flags&131072)!==0&&(dt=!0);else return t.lanes=e.lanes,Sn(e,t,r)}return sf(e,t,a,n,r)}function ex(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return zg(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&fu(t,s!==null?s.cachePool:null),s!==null?Cg(t,s):Zm(),Qb(t);else return t.lanes=t.childLanes=536870912,zg(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(fu(t,s.cachePool),Cg(t,s),Hn(t),t.memoizedState=null):(e!==null&&fu(t,null),Zm(),Hn(t));return yt(e,t,r,a),t.child}function zg(e,t,a,n){var r=If();return r=r===null?null:{parent:nt._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&fu(t,null),Zm(),Qb(t),e!==null&&qo(e,t,n,!0),null}function vu(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(j(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function sf(e,t,a,n,r){return Or(t),a=Vf(e,t,a,n,void 0,r),n=Gf(),e!==null&&!dt?(Yf(e,t,r),Sn(e,t,r)):(ye&&n&&Bf(t),t.flags|=1,yt(e,t,a,r),t.child)}function qg(e,t,a,n,r,s){return Or(t),t.updateQueue=null,a=fb(t,n,a,r),mb(e),n=Gf(),e!==null&&!dt?(Yf(e,t,s),Sn(e,t,s)):(ye&&n&&Bf(t),t.flags|=1,yt(e,t,a,s),t.child)}function Ig(e,t,a,n,r){if(Or(t),t.stateNode===null){var s=_s,i=a.contextType;typeof i=="object"&&i!==null&&(s=Nt(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=nf,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},Hf(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?Nt(i):_s,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(fm(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&nf.enqueueReplaceState(s,s.state,null),po(t,n,s,r),fo(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,l=Pr(a,o);s.props=l;var c=s.context,d=a.contextType;i=_s,typeof d=="object"&&d!==null&&(i=Nt(d));var m=a.getDerivedStateFromProps;d=typeof m=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&Ug(t,s,n,i),zn=!1;var f=t.memoizedState;s.state=f,po(t,n,s,r),fo(),c=t.memoizedState,o||f!==c||zn?(typeof m=="function"&&(fm(t,a,m,n),c=t.memoizedState),(l=zn||Pg(t,a,l,n,f,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=l):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,Xm(e,t),i=t.memoizedProps,d=Pr(a,i),s.props=d,m=t.pendingProps,f=s.context,c=a.contextType,l=_s,typeof c=="object"&&c!==null&&(l=Nt(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==m||f!==l)&&Ug(t,s,n,l),zn=!1,f=t.memoizedState,s.state=f,po(t,n,s,r),fo();var h=t.memoizedState;i!==m||f!==h||zn||e!==null&&e.dependencies!==null&&Tu(e.dependencies)?(typeof o=="function"&&(fm(t,a,o,n),h=t.memoizedState),(d=zn||Pg(t,a,d,n,f,h,l)||e!==null&&e.dependencies!==null&&Tu(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,h,l),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,h,l)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=h),s.props=n,s.state=h,s.context=l,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,vu(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=Is(t,e.child,null,r),t.child=Is(t,null,a,r)):yt(e,t,a,r),t.memoizedState=s.state,e=t.child):e=Sn(e,t,r),e}function Hg(e,t,a,n){return zo(),t.flags|=256,yt(e,t,a,n),t.child}var pm={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function hm(e){return{baseLanes:e,cachePool:ob()}}function vm(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=ba),e}function tx(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(rt.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(ye){if(r?In(t):Hn(t),ye){var o=Ie,l;if(l=o){e:{for(l=o,o=Ka;l.nodeType!==8;){if(!o){o=null;break e}if(l=Ca(l.nextSibling),l===null){o=null;break e}}o=l}o!==null?(t.memoizedState={dehydrated:o,treeContext:Cr!==null?{id:hn,overflow:vn}:null,retryLane:536870912,hydrationErrors:null},l=Xt(18,null,null,0),l.stateNode=o,l.return=t,t.child=l,At=t,Ie=null,l=!0):l=!1}l||Mr(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return xf(o)?t.lanes=32:t.lanes=536870912,null;yn(t)}return o=n.children,n=n.fallback,r?(Hn(t),r=t.mode,o=ju({mode:"hidden",children:o},r),n=kr(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=hm(a),r.childLanes=vm(e,i,a),t.memoizedState=pm,n):(In(t),of(t,o))}if(l=e.memoizedState,l!==null&&(o=l.dehydrated,o!==null)){if(s)t.flags&256?(In(t),t.flags&=-257,t=gm(e,t,a)):t.memoizedState!==null?(Hn(t),t.child=e.child,t.flags|=128,t=null):(Hn(t),r=n.fallback,o=t.mode,n=ju({mode:"visible",children:n.children},o),r=kr(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,Is(t,e.child,null,a),n=t.child,n.memoizedState=hm(a),n.childLanes=vm(e,i,a),t.memoizedState=pm,t=r);else if(In(t),xf(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(j(419)),n.stack="",n.digest=i,_o({value:n,source:null,stack:null}),t=gm(e,t,a)}else if(dt||qo(e,t,a,!1),i=(a&e.childLanes)!==0,dt||i){if(i=Ee,i!==null&&(n=a&-a,n=(n&42)!==0?1:Cf(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==l.retryLane))throw l.retryLane=n,Js(e,n),ta(i,e,n),Wb;o.data==="$?"||ff(),t=gm(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=l.treeContext,Ie=Ca(o.nextSibling),At=t,ye=!0,Er=null,Ka=!1,e!==null&&(ha[va++]=hn,ha[va++]=vn,ha[va++]=Cr,hn=e.id,vn=e.overflow,Cr=t),t=of(t,n.children),t.flags|=4096);return t}return r?(Hn(t),r=n.fallback,o=t.mode,l=e.child,c=l.sibling,n=bn(l,{mode:"hidden",children:n.children}),n.subtreeFlags=l.subtreeFlags&65011712,c!==null?r=bn(c,r):(r=kr(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=hm(a):(l=o.cachePool,l!==null?(c=nt._currentValue,l=l.parent!==c?{parent:c,pool:c}:l):l=ob(),o={baseLanes:o.baseLanes|a,cachePool:l}),r.memoizedState=o,r.childLanes=vm(e,i,a),t.memoizedState=pm,n):(In(t),a=e.child,e=a.sibling,a=bn(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function of(e,t){return t=ju({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function ju(e,t){return e=Xt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function gm(e,t,a){return Is(t,e.child,null,a),e=of(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function Kg(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),Vm(e.return,t,a)}function ym(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function ax(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(yt(e,t,n.children,a),n=rt.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&Kg(e,a,t);else if(e.tag===19)Kg(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Fe(rt,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&Lu(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),ym(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&Lu(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}ym(t,!0,a,null,s);break;case"together":ym(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function Sn(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),ir|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(qo(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(j(153));if(t.child!==null){for(e=t.child,a=bn(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=bn(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function rp(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&Tu(e)))}function TE(e,t,a){switch(t.tag){case 3:wu(t,t.stateNode.containerInfo),qn(t,nt,e.memoizedState.cache),zo();break;case 27:case 5:Lm(t);break;case 4:wu(t,t.stateNode.containerInfo);break;case 10:qn(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(In(t),t.flags|=128,null):(a&t.child.childLanes)!==0?tx(e,t,a):(In(t),e=Sn(e,t,a),e!==null?e.sibling:null);In(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(qo(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return ax(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Fe(rt,rt.current),n)break;return null;case 22:case 23:return t.lanes=0,ex(e,t,a);case 24:qn(t,nt,e.memoizedState.cache)}return Sn(e,t,a)}function nx(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)dt=!0;else{if(!rp(e,a)&&(t.flags&128)===0)return dt=!1,TE(e,t,a);dt=(e.flags&131072)!==0}else dt=!1,ye&&(t.flags&1048576)!==0&&sb(t,Eu,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")Ff(n)?(e=Pr(n,e),t.tag=1,t=Ig(null,t,n,e,a)):(t.tag=0,t=sf(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===_f){t.tag=11,t=Fg(null,t,n,e,a);break e}else if(r===Rf){t.tag=14,t=Bg(null,t,n,e,a);break e}}throw t=Mm(n)||n,Error(j(306,t,""))}}return t;case 0:return sf(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Pr(n,t.pendingProps),Ig(e,t,n,r,a);case 3:e:{if(wu(t,t.stateNode.containerInfo),e===null)throw Error(j(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,Xm(e,t),po(t,n,null,a);var i=t.memoizedState;if(n=i.cache,qn(t,nt,n),n!==s.cache&&Gm(t,[nt],a,!0),fo(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=Hg(e,t,n,a);break e}else if(n!==r){r=ya(Error(j(424)),t),_o(r),t=Hg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(Ie=Ca(e.firstChild),At=t,ye=!0,Er=null,Ka=!0,a=Kb(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if(zo(),n===r){t=Sn(e,t,a);break e}yt(e,t,n,a)}t=t.child}return t;case 26:return vu(e,t),e===null?(a=cy(t.type,null,t.pendingProps,null))?t.memoizedState=a:ye||(a=t.type,e=t.pendingProps,n=Ku(Jn.current).createElement(a),n[St]=t,n[qt]=e,xt(n,a,e),ct(n),t.stateNode=n):t.memoizedState=cy(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return Lm(t),e===null&&ye&&(n=t.stateNode=Ix(t.type,t.pendingProps,Jn.current),At=t,Ka=!0,r=Ie,lr(t.type)?($f=r,Ie=Ca(n.firstChild)):Ie=r),yt(e,t,t.pendingProps.children,a),vu(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&ye&&((r=n=Ie)&&(n=a3(n,t.type,t.pendingProps,Ka),n!==null?(t.stateNode=n,At=t,Ie=Ca(n.firstChild),Ka=!1,r=!0):r=!1),r||Mr(t)),Lm(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,yf(r,s)?n=null:i!==null&&yf(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=Vf(e,t,wE,null,null,a),Ao._currentValue=r),vu(e,t),yt(e,t,n,a),t.child;case 6:return e===null&&ye&&((e=a=Ie)&&(a=n3(a,t.pendingProps,Ka),a!==null?(t.stateNode=a,At=t,Ie=null,e=!0):e=!1),e||Mr(t)),null;case 13:return tx(e,t,a);case 4:return wu(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=Is(t,null,n,a):yt(e,t,n,a),t.child;case 11:return Fg(e,t,t.type,t.pendingProps,a);case 7:return yt(e,t,t.pendingProps,a),t.child;case 8:return yt(e,t,t.pendingProps.children,a),t.child;case 12:return yt(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,qn(t,t.type,n.value),yt(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,Or(t),r=Nt(r),n=n(r),t.flags|=1,yt(e,t,n,a),t.child;case 14:return Bg(e,t,t.type,t.pendingProps,a);case 15:return Zb(e,t,t.type,t.pendingProps,a);case 19:return ax(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=ju(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=bn(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return ex(e,t,a);case 24:return Or(t),n=Nt(nt),e===null?(r=If(),r===null&&(r=Ee,s=qf(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},Hf(t),qn(t,nt,r)):((e.lanes&a)!==0&&(Xm(e,t),po(t,null,null,a),fo()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),qn(t,nt,n)):(n=s.cache,qn(t,nt,n),n!==r.cache&&Gm(t,[nt],a,!0))),yt(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(j(156,t.tag))}function dn(e){e.flags|=4}function Qg(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!Qx(t)){if(t=xa.current,t!==null&&((he&4194048)===he?Ya!==null:(he&62914560)!==he&&(he&536870912)===0||t!==Ya))throw co=Jm,lb;e.flags|=8192}}function au(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?Ey():536870912,e.lanes|=t,Hs|=t)}function Wi(e,t){if(!ye)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function ze(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function AE(e,t,a){var n=t.pendingProps;switch(zf(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return ze(t),null;case 1:return ze(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),xn(nt),Us(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Ji(t)?dn(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,wg())),ze(t),null;case 26:return a=t.memoizedState,e===null?(dn(t),a!==null?(ze(t),Qg(t,a)):(ze(t),t.flags&=-16777217)):a?a!==e.memoizedState?(dn(t),ze(t),Qg(t,a)):(ze(t),t.flags&=-16777217):(e.memoizedProps!==n&&dn(t),ze(t),t.flags&=-16777217),null;case 27:Su(t),a=Jn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&dn(t);else{if(!n){if(t.stateNode===null)throw Error(j(166));return ze(t),null}e=Va.current,Ji(t)?xg(t,e):(e=Ix(r,n,a),t.stateNode=e,dn(t))}return ze(t),null;case 5:if(Su(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&dn(t);else{if(!n){if(t.stateNode===null)throw Error(j(166));return ze(t),null}if(e=Va.current,Ji(t))xg(t,e);else{switch(r=Ku(Jn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[St]=t,e[qt]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(xt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&dn(t)}}return ze(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&dn(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(j(166));if(e=Jn.current,Ji(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=At,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[St]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||Bx(e.nodeValue,a)),e||Mr(t)}else e=Ku(e).createTextNode(n),e[St]=t,t.stateNode=e}return ze(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Ji(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(j(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(j(317));r[St]=t}else zo(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;ze(t),r=!1}else r=wg(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(yn(t),t):(yn(t),null)}if(yn(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),au(t,t.updateQueue),ze(t),null;case 4:return Us(),e===null&&mp(t.stateNode.containerInfo),ze(t),null;case 10:return xn(t.type),ze(t),null;case 19:if(mt(rt),r=t.memoizedState,r===null)return ze(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)Wi(r,!1);else{if(He!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=Lu(e),s!==null){for(t.flags|=128,Wi(r,!1),e=s.updateQueue,t.updateQueue=e,au(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)rb(a,e),a=a.sibling;return Fe(rt,rt.current&1|2),t.child}e=e.sibling}r.tail!==null&&Ga()>Bu&&(t.flags|=128,n=!0,Wi(r,!1),t.lanes=4194304)}else{if(!n)if(e=Lu(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,au(t,e),Wi(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!ye)return ze(t),null}else 2*Ga()-r.renderingStartTime>Bu&&a!==536870912&&(t.flags|=128,n=!0,Wi(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=Ga(),t.sibling=null,e=rt.current,Fe(rt,n?e&1|2:e&1),t):(ze(t),null);case 22:case 23:return yn(t),Kf(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(ze(t),t.subtreeFlags&6&&(t.flags|=8192)):ze(t),a=t.updateQueue,a!==null&&au(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&mt(Tr),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),xn(nt),ze(t),null;case 25:return null;case 30:return null}throw Error(j(156,t.tag))}function DE(e,t){switch(zf(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return xn(nt),Us(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return Su(t),null;case 13:if(yn(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(j(340));zo()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return mt(rt),null;case 4:return Us(),null;case 10:return xn(t.type),null;case 22:case 23:return yn(t),Kf(),e!==null&&mt(Tr),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return xn(nt),null;case 25:return null;default:return null}}function rx(e,t){switch(zf(t),t.tag){case 3:xn(nt),Us();break;case 26:case 27:case 5:Su(t);break;case 4:Us();break;case 13:yn(t);break;case 19:mt(rt);break;case 10:xn(t.type);break;case 22:case 23:yn(t),Kf(),e!==null&&mt(Tr);break;case 24:xn(nt)}}function Vo(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){Re(t,t.return,o)}}function sr(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var l=a,c=o;try{c()}catch(d){Re(r,l,d)}}}n=n.next}while(n!==s)}}catch(d){Re(t,t.return,d)}}function sx(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{db(t,a)}catch(n){Re(e,e.return,n)}}}function ix(e,t,a){a.props=Pr(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){Re(e,t,n)}}function vo(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){Re(e,t,r)}}function Qa(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){Re(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){Re(e,t,r)}else a.current=null}function ox(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){Re(e,e.return,r)}}function bm(e,t,a){try{var n=e.stateNode;XE(n,e.type,a,t),n[qt]=t}catch(r){Re(e,e.return,r)}}function lx(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&lr(e.type)||e.tag===4}function xm(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||lx(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&lr(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function lf(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=cc));else if(n!==4&&(n===27&&lr(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(lf(e,t,a),e=e.sibling;e!==null;)lf(e,t,a),e=e.sibling}function Fu(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&lr(e.type)&&(a=e.stateNode),e=e.child,e!==null))for(Fu(e,t,a),e=e.sibling;e!==null;)Fu(e,t,a),e=e.sibling}function ux(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);xt(t,n,a),t[St]=e,t[qt]=a}catch(s){Re(e,e.return,s)}}var fn=!1,Ge=!1,$m=!1,Vg=typeof WeakSet=="function"?WeakSet:Set,ut=null;function ME(e,t){if(e=e.containerInfo,vf=Yu,e=Jy(e),Pf(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,l=-1,c=0,d=0,m=e,f=null;t:for(;;){for(var h;m!==a||r!==0&&m.nodeType!==3||(o=i+r),m!==s||n!==0&&m.nodeType!==3||(l=i+n),m.nodeType===3&&(i+=m.nodeValue.length),(h=m.firstChild)!==null;)f=m,m=h;for(;;){if(m===e)break t;if(f===a&&++c===r&&(o=i),f===s&&++d===n&&(l=i),(h=m.nextSibling)!==null)break;m=f,f=m.parentNode}m=h}a=o===-1||l===-1?null:{start:o,end:l}}else a=null}a=a||{start:0,end:0}}else a=null;for(gf={focusedElem:e,selectionRange:a},Yu=!1,ut=t;ut!==null;)if(t=ut,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,ut=e;else for(;ut!==null;){switch(t=ut,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var x=Pr(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(x,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(y){Re(a,a.return,y)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)bf(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":bf(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(j(163))}if(e=t.sibling,e!==null){e.return=t.return,ut=e;break}ut=t.return}}function cx(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:jn(e,a),n&4&&Vo(5,a);break;case 1:if(jn(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){Re(a,a.return,i)}else{var r=Pr(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){Re(a,a.return,i)}}n&64&&sx(a),n&512&&vo(a,a.return);break;case 3:if(jn(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{db(e,t)}catch(i){Re(a,a.return,i)}}break;case 27:t===null&&n&4&&ux(a);case 26:case 5:jn(e,a),t===null&&n&4&&ox(a),n&512&&vo(a,a.return);break;case 12:jn(e,a);break;case 13:jn(e,a),n&4&&fx(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=qE.bind(null,a),r3(e,a))));break;case 22:if(n=a.memoizedState!==null||fn,!n){t=t!==null&&t.memoizedState!==null||Ge,r=fn;var s=Ge;fn=n,(Ge=t)&&!s?Fn(e,a,(a.subtreeFlags&8772)!==0):jn(e,a),fn=r,Ge=s}break;case 30:break;default:jn(e,a)}}function dx(e){var t=e.alternate;t!==null&&(e.alternate=null,dx(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&Tf(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var je=null,Bt=!1;function mn(e,t,a){for(a=a.child;a!==null;)mx(e,t,a),a=a.sibling}function mx(e,t,a){if(Wt&&typeof Wt.onCommitFiberUnmount=="function")try{Wt.onCommitFiberUnmount(Po,a)}catch{}switch(a.tag){case 26:Ge||Qa(a,t),mn(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:Ge||Qa(a,t);var n=je,r=Bt;lr(a.type)&&(je=a.stateNode,Bt=!1),mn(e,t,a),xo(a.stateNode),je=n,Bt=r;break;case 5:Ge||Qa(a,t);case 6:if(n=je,r=Bt,je=null,mn(e,t,a),je=n,Bt=r,je!==null)if(Bt)try{(je.nodeType===9?je.body:je.nodeName==="HTML"?je.ownerDocument.body:je).removeChild(a.stateNode)}catch(s){Re(a,t,s)}else try{je.removeChild(a.stateNode)}catch(s){Re(a,t,s)}break;case 18:je!==null&&(Bt?(e=je,oy(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),Oo(e)):oy(je,a.stateNode));break;case 4:n=je,r=Bt,je=a.stateNode.containerInfo,Bt=!0,mn(e,t,a),je=n,Bt=r;break;case 0:case 11:case 14:case 15:Ge||sr(2,a,t),Ge||sr(4,a,t),mn(e,t,a);break;case 1:Ge||(Qa(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&ix(a,t,n)),mn(e,t,a);break;case 21:mn(e,t,a);break;case 22:Ge=(n=Ge)||a.memoizedState!==null,mn(e,t,a),Ge=n;break;default:mn(e,t,a)}}function fx(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{Oo(e)}catch(a){Re(t,t.return,a)}}function OE(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new Vg),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new Vg),t;default:throw Error(j(435,e.tag))}}function wm(e,t){var a=OE(e);t.forEach(function(n){var r=IE.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function Gt(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(lr(o.type)){je=o.stateNode,Bt=!1;break e}break;case 5:je=o.stateNode,Bt=!1;break e;case 3:case 4:je=o.stateNode.containerInfo,Bt=!0;break e}o=o.return}if(je===null)throw Error(j(160));mx(s,i,r),je=null,Bt=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)px(t,e),t=t.sibling}var ka=null;function px(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:Gt(t,e),Yt(e),n&4&&(sr(3,e,e.return),Vo(3,e),sr(5,e,e.return));break;case 1:Gt(t,e),Yt(e),n&512&&(Ge||a===null||Qa(a,a.return)),n&64&&fn&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=ka;if(Gt(t,e),Yt(e),n&512&&(Ge||a===null||Qa(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[Fo]||s[St]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),xt(s,n,a),s[St]=e,ct(s),n=s;break e;case"link":var i=my("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),xt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=my("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),xt(s,n,a),r.head.appendChild(s);break;default:throw Error(j(468,n))}s[St]=e,ct(s),n=s}e.stateNode=n}else fy(r,e.type,e.stateNode);else e.stateNode=dy(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?fy(r,e.type,e.stateNode):dy(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&bm(e,e.memoizedProps,a.memoizedProps)}break;case 27:Gt(t,e),Yt(e),n&512&&(Ge||a===null||Qa(a,a.return)),a!==null&&n&4&&bm(e,e.memoizedProps,a.memoizedProps);break;case 5:if(Gt(t,e),Yt(e),n&512&&(Ge||a===null||Qa(a,a.return)),e.flags&32){r=e.stateNode;try{Fs(r,"")}catch(h){Re(e,e.return,h)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,bm(e,r,a!==null?a.memoizedProps:r)),n&1024&&($m=!0);break;case 6:if(Gt(t,e),Yt(e),n&4){if(e.stateNode===null)throw Error(j(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(h){Re(e,e.return,h)}}break;case 3:if(bu=null,r=ka,ka=Qu(t.containerInfo),Gt(t,e),ka=r,Yt(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{Oo(t.containerInfo)}catch(h){Re(e,e.return,h)}$m&&($m=!1,hx(e));break;case 4:n=ka,ka=Qu(e.stateNode.containerInfo),Gt(t,e),Yt(e),ka=n;break;case 12:Gt(t,e),Yt(e);break;case 13:Gt(t,e),Yt(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(up=Ga()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,wm(e,n)));break;case 22:r=e.memoizedState!==null;var l=a!==null&&a.memoizedState!==null,c=fn,d=Ge;if(fn=c||r,Ge=d||l,Gt(t,e),Ge=d,fn=c,Yt(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||l||fn||Ge||_r(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){l=a=t;try{if(s=l.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=l.stateNode;var m=l.memoizedProps.style,f=m!=null&&m.hasOwnProperty("display")?m.display:null;o.style.display=f==null||typeof f=="boolean"?"":(""+f).trim()}}catch(h){Re(l,l.return,h)}}}else if(t.tag===6){if(a===null){l=t;try{l.stateNode.nodeValue=r?"":l.memoizedProps}catch(h){Re(l,l.return,h)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,wm(e,a))));break;case 19:Gt(t,e),Yt(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,wm(e,n)));break;case 30:break;case 21:break;default:Gt(t,e),Yt(e)}}function Yt(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(lx(n)){a=n;break}n=n.return}if(a==null)throw Error(j(160));switch(a.tag){case 27:var r=a.stateNode,s=xm(e);Fu(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(Fs(i,""),a.flags&=-33);var o=xm(e);Fu(e,o,i);break;case 3:case 4:var l=a.stateNode.containerInfo,c=xm(e);lf(e,c,l);break;default:throw Error(j(161))}}catch(d){Re(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function hx(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;hx(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function jn(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)cx(e,t.alternate,t),t=t.sibling}function _r(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:sr(4,t,t.return),_r(t);break;case 1:Qa(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&ix(t,t.return,a),_r(t);break;case 27:xo(t.stateNode);case 26:case 5:Qa(t,t.return),_r(t);break;case 22:t.memoizedState===null&&_r(t);break;case 30:_r(t);break;default:_r(t)}e=e.sibling}}function Fn(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:Fn(r,s,a),Vo(4,s);break;case 1:if(Fn(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){Re(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var l=r.shared.hiddenCallbacks;if(l!==null)for(r.shared.hiddenCallbacks=null,r=0;r<l.length;r++)cb(l[r],o)}catch(c){Re(n,n.return,c)}}a&&i&64&&sx(s),vo(s,s.return);break;case 27:ux(s);case 26:case 5:Fn(r,s,a),a&&n===null&&i&4&&ox(s),vo(s,s.return);break;case 12:Fn(r,s,a);break;case 13:Fn(r,s,a),a&&i&4&&fx(r,s);break;case 22:s.memoizedState===null&&Fn(r,s,a),vo(s,s.return);break;case 30:break;default:Fn(r,s,a)}t=t.sibling}}function sp(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&Io(a))}function ip(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&Io(e))}function Ha(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)vx(e,t,a,n),t=t.sibling}function vx(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:Ha(e,t,a,n),r&2048&&Vo(9,t);break;case 1:Ha(e,t,a,n);break;case 3:Ha(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&Io(e)));break;case 12:if(r&2048){Ha(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(l){Re(t,t.return,l)}}else Ha(e,t,a,n);break;case 13:Ha(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?Ha(e,t,a,n):go(e,t):s._visibility&2?Ha(e,t,a,n):(s._visibility|=2,hs(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&sp(i,t);break;case 24:Ha(e,t,a,n),r&2048&&ip(t.alternate,t);break;default:Ha(e,t,a,n)}}function hs(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,l=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:hs(s,i,o,l,r),Vo(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?hs(s,i,o,l,r):go(s,i):(d._visibility|=2,hs(s,i,o,l,r)),r&&c&2048&&sp(i.alternate,i);break;case 24:hs(s,i,o,l,r),r&&c&2048&&ip(i.alternate,i);break;default:hs(s,i,o,l,r)}t=t.sibling}}function go(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:go(a,n),r&2048&&sp(n.alternate,n);break;case 24:go(a,n),r&2048&&ip(n.alternate,n);break;default:go(a,n)}t=t.sibling}}var so=8192;function ms(e){if(e.subtreeFlags&so)for(e=e.child;e!==null;)gx(e),e=e.sibling}function gx(e){switch(e.tag){case 26:ms(e),e.flags&so&&e.memoizedState!==null&&g3(ka,e.memoizedState,e.memoizedProps);break;case 5:ms(e);break;case 3:case 4:var t=ka;ka=Qu(e.stateNode.containerInfo),ms(e),ka=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=so,so=16777216,ms(e),so=t):ms(e));break;default:ms(e)}}function yx(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function Zi(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ut=n,xx(n,e)}yx(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)bx(e),e=e.sibling}function bx(e){switch(e.tag){case 0:case 11:case 15:Zi(e),e.flags&2048&&sr(9,e,e.return);break;case 3:Zi(e);break;case 12:Zi(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,gu(e)):Zi(e);break;default:Zi(e)}}function gu(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ut=n,xx(n,e)}yx(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:sr(8,t,t.return),gu(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,gu(t));break;default:gu(t)}e=e.sibling}}function xx(e,t){for(;ut!==null;){var a=ut;switch(a.tag){case 0:case 11:case 15:sr(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:Io(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,ut=n;else e:for(a=e;ut!==null;){n=ut;var r=n.sibling,s=n.return;if(dx(n),n===a){ut=null;break e}if(r!==null){r.return=s,ut=r;break e}ut=s}}}var LE={getCacheForType:function(e){var t=Nt(nt),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},PE=typeof WeakMap=="function"?WeakMap:Map,Se=0,Ee=null,me=null,he=0,we=0,Jt=null,Gn=!1,Xs=!1,op=!1,Nn=0,He=0,ir=0,Ar=0,lp=0,ba=0,Hs=0,yo=null,zt=null,uf=!1,up=0,Bu=1/0,zu=null,Zn=null,bt=0,er=null,Ks=null,Ps=0,cf=0,df=null,$x=null,bo=0,mf=null;function ea(){if((Se&2)!==0&&he!==0)return he&-he;if(se.T!==null){var e=Bs;return e!==0?e:dp()}return Dy()}function wx(){ba===0&&(ba=(he&536870912)===0||ye?Cy():536870912);var e=xa.current;return e!==null&&(e.flags|=32),ba}function ta(e,t,a){(e===Ee&&(we===2||we===9)||e.cancelPendingCommit!==null)&&(Qs(e,0),Yn(e,he,ba,!1)),jo(e,a),((Se&2)===0||e!==Ee)&&(e===Ee&&((Se&2)===0&&(Ar|=a),He===4&&Yn(e,he,ba,!1)),Xa(e))}function Sx(e,t,a){if((Se&6)!==0)throw Error(j(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||Uo(e,t),r=n?FE(e,t):Sm(e,t,!0),s=n;do{if(r===0){Xs&&!n&&Yn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!UE(a)){r=Sm(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=yo;var l=o.current.memoizedState.isDehydrated;if(l&&(Qs(o,i).flags|=256),i=Sm(o,i,!1),i!==2){if(op&&!l){o.errorRecoveryDisabledLanes|=s,Ar|=s,r=4;break e}s=zt,zt=r,s!==null&&(zt===null?zt=s:zt.push.apply(zt,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){Qs(e,0),Yn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(j(345));case 4:if((t&4194048)!==t)break;case 6:Yn(n,t,ba,!Gn);break e;case 2:zt=null;break;case 3:case 5:break;default:throw Error(j(329))}if((t&62914560)===t&&(r=up+300-Ga(),10<r)){if(Yn(n,t,ba,!Gn),Xu(n,0,!0)!==0)break e;n.timeoutHandle=qx(Gg.bind(null,n,a,zt,zu,uf,t,ba,Ar,Hs,Gn,s,2,-0,0),r);break e}Gg(n,a,zt,zu,uf,t,ba,Ar,Hs,Gn,s,0,-0,0)}}break}while(!0);Xa(e)}function Gg(e,t,a,n,r,s,i,o,l,c,d,m,f,h){if(e.timeoutHandle=-1,m=t.subtreeFlags,(m&8192||(m&16785408)===16785408)&&(To={stylesheets:null,count:0,unsuspend:v3},gx(t),m=y3(),m!==null)){e.cancelPendingCommit=m(Jg.bind(null,e,t,s,a,n,r,i,o,l,d,1,f,h)),Yn(e,s,i,!c);return}Jg(e,t,s,a,n,r,i,o,l)}function UE(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!aa(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function Yn(e,t,a,n){t&=~lp,t&=~Ar,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Zt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&Ty(e,a,t)}function oc(){return(Se&6)===0?(Go(0,!1),!1):!0}function cp(){if(me!==null){if(we===0)var e=me.return;else e=me,gn=Br=null,Jf(e),Ls=null,ko=0,e=me;for(;e!==null;)rx(e.alternate,e),e=e.return;me=null}}function Qs(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,ZE(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),cp(),Ee=e,me=a=bn(e.current,null),he=t,we=0,Jt=null,Gn=!1,Xs=Uo(e,t),op=!1,Hs=ba=lp=Ar=ir=He=0,zt=yo=null,uf=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Zt(n),s=1<<r;t|=e[r],n&=~s}return Nn=t,tc(),a}function Nx(e,t){ue=null,se.H=Ou,t===Ho||t===nc?(t=Rg(),we=3):t===lb?(t=Rg(),we=4):we=t===Wb?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Jt=t,me===null&&(He=1,Uu(e,ya(t,e.current)))}function _x(){var e=se.H;return se.H=Ou,e===null?Ou:e}function Rx(){var e=se.A;return se.A=LE,e}function ff(){He=4,Gn||(he&4194048)!==he&&xa.current!==null||(Xs=!0),(ir&134217727)===0&&(Ar&134217727)===0||Ee===null||Yn(Ee,he,ba,!1)}function Sm(e,t,a){var n=Se;Se|=2;var r=_x(),s=Rx();(Ee!==e||he!==t)&&(zu=null,Qs(e,t)),t=!1;var i=He;e:do try{if(we!==0&&me!==null){var o=me,l=Jt;switch(we){case 8:cp(),i=6;break e;case 3:case 2:case 9:case 6:xa.current===null&&(t=!0);var c=we;if(we=0,Jt=null,Cs(e,o,l,c),a&&Xs){i=0;break e}break;default:c=we,we=0,Jt=null,Cs(e,o,l,c)}}jE(),i=He;break}catch(d){Nx(e,d)}while(!0);return t&&e.shellSuspendCounter++,gn=Br=null,Se=n,se.H=r,se.A=s,me===null&&(Ee=null,he=0,tc()),i}function jE(){for(;me!==null;)kx(me)}function FE(e,t){var a=Se;Se|=2;var n=_x(),r=Rx();Ee!==e||he!==t?(zu=null,Bu=Ga()+500,Qs(e,t)):Xs=Uo(e,t);e:do try{if(we!==0&&me!==null){t=me;var s=Jt;t:switch(we){case 1:we=0,Jt=null,Cs(e,t,s,1);break;case 2:case 9:if(_g(s)){we=0,Jt=null,Yg(t);break}t=function(){we!==2&&we!==9||Ee!==e||(we=7),Xa(e)},s.then(t,t);break e;case 3:we=7;break e;case 4:we=5;break e;case 7:_g(s)?(we=0,Jt=null,Yg(t)):(we=0,Jt=null,Cs(e,t,s,7));break;case 5:var i=null;switch(me.tag){case 26:i=me.memoizedState;case 5:case 27:var o=me;if(!i||Qx(i)){we=0,Jt=null;var l=o.sibling;if(l!==null)me=l;else{var c=o.return;c!==null?(me=c,lc(c)):me=null}break t}}we=0,Jt=null,Cs(e,t,s,5);break;case 6:we=0,Jt=null,Cs(e,t,s,6);break;case 8:cp(),He=6;break e;default:throw Error(j(462))}}BE();break}catch(d){Nx(e,d)}while(!0);return gn=Br=null,se.H=n,se.A=r,Se=a,me!==null?0:(Ee=null,he=0,tc(),He)}function BE(){for(;me!==null&&!lC();)kx(me)}function kx(e){var t=nx(e.alternate,e,Nn);e.memoizedProps=e.pendingProps,t===null?lc(e):me=t}function Yg(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=qg(a,t,t.pendingProps,t.type,void 0,he);break;case 11:t=qg(a,t,t.pendingProps,t.type.render,t.ref,he);break;case 5:Jf(t);default:rx(a,t),t=me=rb(t,Nn),t=nx(a,t,Nn)}e.memoizedProps=e.pendingProps,t===null?lc(e):me=t}function Cs(e,t,a,n){gn=Br=null,Jf(t),Ls=null,ko=0;var r=t.return;try{if(EE(e,r,t,a,he)){He=1,Uu(e,ya(a,e.current)),me=null;return}}catch(s){if(r!==null)throw me=r,s;He=1,Uu(e,ya(a,e.current)),me=null;return}t.flags&32768?(ye||n===1?e=!0:Xs||(he&536870912)!==0?e=!1:(Gn=e=!0,(n===2||n===9||n===3||n===6)&&(n=xa.current,n!==null&&n.tag===13&&(n.flags|=16384))),Cx(t,e)):lc(t)}function lc(e){var t=e;do{if((t.flags&32768)!==0){Cx(t,Gn);return}e=t.return;var a=AE(t.alternate,t,Nn);if(a!==null){me=a;return}if(t=t.sibling,t!==null){me=t;return}me=t=e}while(t!==null);He===0&&(He=5)}function Cx(e,t){do{var a=DE(e.alternate,e);if(a!==null){a.flags&=32767,me=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){me=e;return}me=e=a}while(e!==null);He=6,me=null}function Jg(e,t,a,n,r,s,i,o,l){e.cancelPendingCommit=null;do uc();while(bt!==0);if((Se&6)!==0)throw Error(j(327));if(t!==null){if(t===e.current)throw Error(j(177));if(s=t.lanes|t.childLanes,s|=Uf,yC(e,a,s,i,o,l),e===Ee&&(me=Ee=null,he=0),Ks=t,er=e,Ps=a,cf=s,df=r,$x=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,HE(Nu,function(){return Mx(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=se.T,se.T=null,r=be.p,be.p=2,i=Se,Se|=4;try{ME(e,t,a)}finally{Se=i,be.p=r,se.T=n}}bt=1,Ex(),Tx(),Ax()}}function Ex(){if(bt===1){bt=0;var e=er,t=Ks,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=se.T,se.T=null;var n=be.p;be.p=2;var r=Se;Se|=4;try{px(t,e);var s=gf,i=Jy(e.containerInfo),o=s.focusedElem,l=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&Yy(o.ownerDocument.documentElement,o)){if(l!==null&&Pf(o)){var c=l.start,d=l.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var m=o.ownerDocument||document,f=m&&m.defaultView||window;if(f.getSelection){var h=f.getSelection(),x=o.textContent.length,y=Math.min(l.start,x),$=l.end===void 0?y:Math.min(l.end,x);!h.extend&&y>$&&(i=$,$=y,y=i);var g=gg(o,y),v=gg(o,$);if(g&&v&&(h.rangeCount!==1||h.anchorNode!==g.node||h.anchorOffset!==g.offset||h.focusNode!==v.node||h.focusOffset!==v.offset)){var b=m.createRange();b.setStart(g.node,g.offset),h.removeAllRanges(),y>$?(h.addRange(b),h.extend(v.node,v.offset)):(b.setEnd(v.node,v.offset),h.addRange(b))}}}}for(m=[],h=o;h=h.parentNode;)h.nodeType===1&&m.push({element:h,left:h.scrollLeft,top:h.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<m.length;o++){var w=m[o];w.element.scrollLeft=w.left,w.element.scrollTop=w.top}}Yu=!!vf,gf=vf=null}finally{Se=r,be.p=n,se.T=a}}e.current=t,bt=2}}function Tx(){if(bt===2){bt=0;var e=er,t=Ks,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=se.T,se.T=null;var n=be.p;be.p=2;var r=Se;Se|=4;try{cx(e,t.alternate,t)}finally{Se=r,be.p=n,se.T=a}}bt=3}}function Ax(){if(bt===4||bt===3){bt=0,uC();var e=er,t=Ks,a=Ps,n=$x;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?bt=5:(bt=0,Ks=er=null,Dx(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(Zn=null),Ef(a),t=t.stateNode,Wt&&typeof Wt.onCommitFiberRoot=="function")try{Wt.onCommitFiberRoot(Po,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=se.T,r=be.p,be.p=2,se.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{se.T=t,be.p=r}}(Ps&3)!==0&&uc(),Xa(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===mf?bo++:(bo=0,mf=e):bo=0,Go(0,!1)}}function Dx(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,Io(t)))}function uc(e){return Ex(),Tx(),Ax(),Mx(e)}function Mx(){if(bt!==5)return!1;var e=er,t=cf;cf=0;var a=Ef(Ps),n=se.T,r=be.p;try{be.p=32>a?32:a,se.T=null,a=df,df=null;var s=er,i=Ps;if(bt=0,Ks=er=null,Ps=0,(Se&6)!==0)throw Error(j(331));var o=Se;if(Se|=4,bx(s.current),vx(s,s.current,i,a),Se=o,Go(0,!1),Wt&&typeof Wt.onPostCommitFiberRoot=="function")try{Wt.onPostCommitFiberRoot(Po,s)}catch{}return!0}finally{be.p=r,se.T=n,Dx(e,t)}}function Xg(e,t,a){t=ya(a,t),t=rf(e.stateNode,t,2),e=Wn(e,t,2),e!==null&&(jo(e,2),Xa(e))}function Re(e,t,a){if(e.tag===3)Xg(e,e,a);else for(;t!==null;){if(t.tag===3){Xg(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(Zn===null||!Zn.has(n))){e=ya(a,e),a=Jb(2),n=Wn(t,a,2),n!==null&&(Xb(a,n,t,e),jo(n,2),Xa(n));break}}t=t.return}}function Nm(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new PE;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(op=!0,r.add(a),e=zE.bind(null,e,t,a),t.then(e,e))}function zE(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,Ee===e&&(he&a)===a&&(He===4||He===3&&(he&62914560)===he&&300>Ga()-up?(Se&2)===0&&Qs(e,0):lp|=a,Hs===he&&(Hs=0)),Xa(e)}function Ox(e,t){t===0&&(t=Ey()),e=Js(e,t),e!==null&&(jo(e,t),Xa(e))}function qE(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),Ox(e,a)}function IE(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(j(314))}n!==null&&n.delete(t),Ox(e,a)}function HE(e,t){return kf(e,t)}var qu=null,vs=null,pf=!1,Iu=!1,_m=!1,Dr=0;function Xa(e){e!==vs&&e.next===null&&(vs===null?qu=vs=e:vs=vs.next=e),Iu=!0,pf||(pf=!0,QE())}function Go(e,t){if(!_m&&Iu){_m=!0;do for(var a=!1,n=qu;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Zt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,Wg(n,s))}else s=he,s=Xu(n,n===Ee?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||Uo(n,s)||(a=!0,Wg(n,s));n=n.next}while(a);_m=!1}}function KE(){Lx()}function Lx(){Iu=pf=!1;var e=0;Dr!==0&&(WE()&&(e=Dr),Dr=0);for(var t=Ga(),a=null,n=qu;n!==null;){var r=n.next,s=Px(n,t);s===0?(n.next=null,a===null?qu=r:a.next=r,r===null&&(vs=a)):(a=n,(e!==0||(s&3)!==0)&&(Iu=!0)),n=r}Go(e,!1)}function Px(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Zt(s),o=1<<i,l=r[i];l===-1?((o&a)===0||(o&n)!==0)&&(r[i]=gC(o,t)):l<=t&&(e.expiredLanes|=o),s&=~o}if(t=Ee,a=he,a=Xu(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&(we===2||we===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&Wd(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||Uo(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&Wd(n),Ef(a)){case 2:case 8:a=Ry;break;case 32:a=Nu;break;case 268435456:a=ky;break;default:a=Nu}return n=Ux.bind(null,e),a=kf(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&Wd(n),e.callbackPriority=2,e.callbackNode=null,2}function Ux(e,t){if(bt!==0&&bt!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(uc(!0)&&e.callbackNode!==a)return null;var n=he;return n=Xu(e,e===Ee?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:(Sx(e,n,t),Px(e,Ga()),e.callbackNode!=null&&e.callbackNode===a?Ux.bind(null,e):null)}function Wg(e,t){if(uc())return null;Sx(e,t,!0)}function QE(){e3(function(){(Se&6)!==0?kf(_y,KE):Lx()})}function dp(){return Dr===0&&(Dr=Cy()),Dr}function Zg(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:uu(""+e)}function ey(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function VE(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=Zg((r[qt]||null).action),i=n.submitter;i&&(t=(t=i[qt]||null)?Zg(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Wu("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(Dr!==0){var l=i?ey(r,i):new FormData(r);af(a,{pending:!0,data:l,method:r.method,action:s},null,l)}}else typeof s=="function"&&(o.preventDefault(),l=i?ey(r,i):new FormData(r),af(a,{pending:!0,data:l,method:r.method,action:s},s,l))},currentTarget:r}]})}}for(nu=0;nu<Hm.length;nu++)ru=Hm[nu],ty=ru.toLowerCase(),ay=ru[0].toUpperCase()+ru.slice(1),Ea(ty,"on"+ay);var ru,ty,ay,nu;Ea(Wy,"onAnimationEnd");Ea(Zy,"onAnimationIteration");Ea(eb,"onAnimationStart");Ea("dblclick","onDoubleClick");Ea("focusin","onFocus");Ea("focusout","onBlur");Ea(mE,"onTransitionRun");Ea(fE,"onTransitionStart");Ea(pE,"onTransitionCancel");Ea(tb,"onTransitionEnd");js("onMouseEnter",["mouseout","mouseover"]);js("onMouseLeave",["mouseout","mouseover"]);js("onPointerEnter",["pointerout","pointerover"]);js("onPointerLeave",["pointerout","pointerover"]);Ur("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Ur("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Ur("onBeforeInput",["compositionend","keypress","textInput","paste"]);Ur("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Ur("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Ur("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var Co="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),GE=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(Co));function jx(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],l=o.instance,c=o.currentTarget;if(o=o.listener,l!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Pu(d)}r.currentTarget=null,s=l}else for(i=0;i<n.length;i++){if(o=n[i],l=o.instance,c=o.currentTarget,o=o.listener,l!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Pu(d)}r.currentTarget=null,s=l}}}}function de(e,t){var a=t[Um];a===void 0&&(a=t[Um]=new Set);var n=e+"__bubble";a.has(n)||(Fx(t,e,2,!1),a.add(n))}function Rm(e,t,a){var n=0;t&&(n|=4),Fx(a,e,n,t)}var su="_reactListening"+Math.random().toString(36).slice(2);function mp(e){if(!e[su]){e[su]=!0,My.forEach(function(a){a!=="selectionchange"&&(GE.has(a)||Rm(a,!1,e),Rm(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[su]||(t[su]=!0,Rm("selectionchange",!1,t))}}function Fx(e,t,a,n){switch(Xx(t)){case 2:var r=$3;break;case 8:r=w3;break;default:r=vp}a=r.bind(null,t,a,e),r=void 0,!zm||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function km(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var l=i.tag;if((l===3||l===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=bs(o),i===null)return;if(l=i.tag,l===5||l===6||l===26||l===27){n=s=i;continue e}o=o.parentNode}}n=n.return}zy(function(){var c=s,d=Df(a),m=[];e:{var f=ab.get(e);if(f!==void 0){var h=Wu,x=e;switch(e){case"keypress":if(du(a)===0)break e;case"keydown":case"keyup":h=HC;break;case"focusin":x="focus",h=im;break;case"focusout":x="blur",h=im;break;case"beforeblur":case"afterblur":h=im;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":h=lg;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":h=DC;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":h=VC;break;case Wy:case Zy:case eb:h=LC;break;case tb:h=YC;break;case"scroll":case"scrollend":h=TC;break;case"wheel":h=XC;break;case"copy":case"cut":case"paste":h=UC;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":h=cg;break;case"toggle":case"beforetoggle":h=ZC}var y=(t&4)!==0,$=!y&&(e==="scroll"||e==="scrollend"),g=y?f!==null?f+"Capture":null:f;y=[];for(var v=c,b;v!==null;){var w=v;if(b=w.stateNode,w=w.tag,w!==5&&w!==26&&w!==27||b===null||g===null||(w=wo(v,g),w!=null&&y.push(Eo(v,w,b))),$)break;v=v.return}0<y.length&&(f=new h(f,x,null,a,d),m.push({event:f,listeners:y}))}}if((t&7)===0){e:{if(f=e==="mouseover"||e==="pointerover",h=e==="mouseout"||e==="pointerout",f&&a!==Bm&&(x=a.relatedTarget||a.fromElement)&&(bs(x)||x[Gs]))break e;if((h||f)&&(f=d.window===d?d:(f=d.ownerDocument)?f.defaultView||f.parentWindow:window,h?(x=a.relatedTarget||a.toElement,h=c,x=x?bs(x):null,x!==null&&($=Lo(x),y=x.tag,x!==$||y!==5&&y!==27&&y!==6)&&(x=null)):(h=null,x=c),h!==x)){if(y=lg,w="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(y=cg,w="onPointerLeave",g="onPointerEnter",v="pointer"),$=h==null?f:ro(h),b=x==null?f:ro(x),f=new y(w,v+"leave",h,a,d),f.target=$,f.relatedTarget=b,w=null,bs(d)===c&&(y=new y(g,v+"enter",x,a,d),y.target=b,y.relatedTarget=$,w=y),$=w,h&&x)t:{for(y=h,g=x,v=0,b=y;b;b=fs(b))v++;for(b=0,w=g;w;w=fs(w))b++;for(;0<v-b;)y=fs(y),v--;for(;0<b-v;)g=fs(g),b--;for(;v--;){if(y===g||g!==null&&y===g.alternate)break t;y=fs(y),g=fs(g)}y=null}else y=null;h!==null&&ny(m,f,h,y,!1),x!==null&&$!==null&&ny(m,$,x,y,!0)}}e:{if(f=c?ro(c):window,h=f.nodeName&&f.nodeName.toLowerCase(),h==="select"||h==="input"&&f.type==="file")var S=pg;else if(fg(f))if(Vy)S=uE;else{S=oE;var C=iE}else h=f.nodeName,!h||h.toLowerCase()!=="input"||f.type!=="checkbox"&&f.type!=="radio"?c&&Af(c.elementType)&&(S=pg):S=lE;if(S&&(S=S(e,c))){Qy(m,S,a,d);break e}C&&C(e,f,c),e==="focusout"&&c&&f.type==="number"&&c.memoizedProps.value!=null&&Fm(f,"number",f.value)}switch(C=c?ro(c):window,e){case"focusin":(fg(C)||C.contentEditable==="true")&&(ws=C,qm=c,lo=null);break;case"focusout":lo=qm=ws=null;break;case"mousedown":Im=!0;break;case"contextmenu":case"mouseup":case"dragend":Im=!1,yg(m,a,d);break;case"selectionchange":if(dE)break;case"keydown":case"keyup":yg(m,a,d)}var R;if(Lf)e:{switch(e){case"compositionstart":var _="onCompositionStart";break e;case"compositionend":_="onCompositionEnd";break e;case"compositionupdate":_="onCompositionUpdate";break e}_=void 0}else $s?Hy(e,a)&&(_="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(_="onCompositionStart");_&&(Iy&&a.locale!=="ko"&&($s||_!=="onCompositionStart"?_==="onCompositionEnd"&&$s&&(R=qy()):(Vn=d,Mf="value"in Vn?Vn.value:Vn.textContent,$s=!0)),C=Hu(c,_),0<C.length&&(_=new ug(_,e,null,a,d),m.push({event:_,listeners:C}),R?_.data=R:(R=Ky(a),R!==null&&(_.data=R)))),(R=tE?aE(e,a):nE(e,a))&&(_=Hu(c,"onBeforeInput"),0<_.length&&(C=new ug("onBeforeInput","beforeinput",null,a,d),m.push({event:C,listeners:_}),C.data=R)),VE(m,e,c,a,d)}jx(m,t)})}function Eo(e,t,a){return{instance:e,listener:t,currentTarget:a}}function Hu(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=wo(e,a),r!=null&&n.unshift(Eo(e,r,s)),r=wo(e,t),r!=null&&n.push(Eo(e,r,s))),e.tag===3)return n;e=e.return}return[]}function fs(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function ny(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,l=o.alternate,c=o.stateNode;if(o=o.tag,l!==null&&l===n)break;o!==5&&o!==26&&o!==27||c===null||(l=c,r?(c=wo(a,s),c!=null&&i.unshift(Eo(a,c,l))):r||(c=wo(a,s),c!=null&&i.push(Eo(a,c,l)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var YE=/\r\n?/g,JE=/\u0000|\uFFFD/g;function ry(e){return(typeof e=="string"?e:""+e).replace(YE,`
`).replace(JE,"")}function Bx(e,t){return t=ry(t),ry(e)===t}function cc(){}function Ne(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||Fs(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&Fs(e,""+n);break;case"className":Gl(e,"class",n);break;case"tabIndex":Gl(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":Gl(e,a,n);break;case"style":By(e,n,s);break;case"data":if(t!=="object"){Gl(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=uu(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&Ne(e,t,"name",r.name,r,null),Ne(e,t,"formEncType",r.formEncType,r,null),Ne(e,t,"formMethod",r.formMethod,r,null),Ne(e,t,"formTarget",r.formTarget,r,null)):(Ne(e,t,"encType",r.encType,r,null),Ne(e,t,"method",r.method,r,null),Ne(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=uu(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=cc);break;case"onScroll":n!=null&&de("scroll",e);break;case"onScrollEnd":n!=null&&de("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(j(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(j(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=uu(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":de("beforetoggle",e),de("toggle",e),lu(e,"popover",n);break;case"xlinkActuate":cn(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":cn(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":cn(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":cn(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":cn(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":cn(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":cn(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":cn(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":cn(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":lu(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=CC.get(a)||a,lu(e,a,n))}}function hf(e,t,a,n,r,s){switch(a){case"style":By(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(j(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(j(60));e.innerHTML=a}}break;case"children":typeof n=="string"?Fs(e,n):(typeof n=="number"||typeof n=="bigint")&&Fs(e,""+n);break;case"onScroll":n!=null&&de("scroll",e);break;case"onScrollEnd":n!=null&&de("scrollend",e);break;case"onClick":n!=null&&(e.onclick=cc);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!Oy.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[qt]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):lu(e,a,n)}}}function xt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":de("error",e),de("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(j(137,t));default:Ne(e,t,s,i,a,null)}}r&&Ne(e,t,"srcSet",a.srcSet,a,null),n&&Ne(e,t,"src",a.src,a,null);return;case"input":de("invalid",e);var o=s=i=r=null,l=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":l=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(j(137,t));break;default:Ne(e,t,n,d,a,null)}}Uy(e,s,o,l,c,i,r,!1),_u(e);return;case"select":de("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:Ne(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?Ts(e,!!n,t,!1):a!=null&&Ts(e,!!n,a,!0);return;case"textarea":de("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(j(91));break;default:Ne(e,t,i,o,a,null)}Fy(e,n,r,s),_u(e);return;case"option":for(l in a)if(a.hasOwnProperty(l)&&(n=a[l],n!=null))switch(l){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:Ne(e,t,l,n,a,null)}return;case"dialog":de("beforetoggle",e),de("toggle",e),de("cancel",e),de("close",e);break;case"iframe":case"object":de("load",e);break;case"video":case"audio":for(n=0;n<Co.length;n++)de(Co[n],e);break;case"image":de("error",e),de("load",e);break;case"details":de("toggle",e);break;case"embed":case"source":case"link":de("error",e),de("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(j(137,t));default:Ne(e,t,c,n,a,null)}return;default:if(Af(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&hf(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&Ne(e,t,o,n,a,null))}function XE(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,l=null,c=null,d=null;for(h in a){var m=a[h];if(a.hasOwnProperty(h)&&m!=null)switch(h){case"checked":break;case"value":break;case"defaultValue":l=m;default:n.hasOwnProperty(h)||Ne(e,t,h,null,n,m)}}for(var f in n){var h=n[f];if(m=a[f],n.hasOwnProperty(f)&&(h!=null||m!=null))switch(f){case"type":s=h;break;case"name":r=h;break;case"checked":c=h;break;case"defaultChecked":d=h;break;case"value":i=h;break;case"defaultValue":o=h;break;case"children":case"dangerouslySetInnerHTML":if(h!=null)throw Error(j(137,t));break;default:h!==m&&Ne(e,t,f,h,n,m)}}jm(e,i,o,l,c,d,s,r);return;case"select":h=i=o=f=null;for(s in a)if(l=a[s],a.hasOwnProperty(s)&&l!=null)switch(s){case"value":break;case"multiple":h=l;default:n.hasOwnProperty(s)||Ne(e,t,s,null,n,l)}for(r in n)if(s=n[r],l=a[r],n.hasOwnProperty(r)&&(s!=null||l!=null))switch(r){case"value":f=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==l&&Ne(e,t,r,s,n,l)}t=o,a=i,n=h,f!=null?Ts(e,!!a,f,!1):!!n!=!!a&&(t!=null?Ts(e,!!a,t,!0):Ts(e,!!a,a?[]:"",!1));return;case"textarea":h=f=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:Ne(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":f=r;break;case"defaultValue":h=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(j(91));break;default:r!==s&&Ne(e,t,i,r,n,s)}jy(e,f,h);return;case"option":for(var x in a)if(f=a[x],a.hasOwnProperty(x)&&f!=null&&!n.hasOwnProperty(x))switch(x){case"selected":e.selected=!1;break;default:Ne(e,t,x,null,n,f)}for(l in n)if(f=n[l],h=a[l],n.hasOwnProperty(l)&&f!==h&&(f!=null||h!=null))switch(l){case"selected":e.selected=f&&typeof f!="function"&&typeof f!="symbol";break;default:Ne(e,t,l,f,n,h)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var y in a)f=a[y],a.hasOwnProperty(y)&&f!=null&&!n.hasOwnProperty(y)&&Ne(e,t,y,null,n,f);for(c in n)if(f=n[c],h=a[c],n.hasOwnProperty(c)&&f!==h&&(f!=null||h!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(f!=null)throw Error(j(137,t));break;default:Ne(e,t,c,f,n,h)}return;default:if(Af(t)){for(var $ in a)f=a[$],a.hasOwnProperty($)&&f!==void 0&&!n.hasOwnProperty($)&&hf(e,t,$,void 0,n,f);for(d in n)f=n[d],h=a[d],!n.hasOwnProperty(d)||f===h||f===void 0&&h===void 0||hf(e,t,d,f,n,h);return}}for(var g in a)f=a[g],a.hasOwnProperty(g)&&f!=null&&!n.hasOwnProperty(g)&&Ne(e,t,g,null,n,f);for(m in n)f=n[m],h=a[m],!n.hasOwnProperty(m)||f===h||f==null&&h==null||Ne(e,t,m,f,n,h)}var vf=null,gf=null;function Ku(e){return e.nodeType===9?e:e.ownerDocument}function sy(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function zx(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function yf(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var Cm=null;function WE(){var e=window.event;return e&&e.type==="popstate"?e===Cm?!1:(Cm=e,!0):(Cm=null,!1)}var qx=typeof setTimeout=="function"?setTimeout:void 0,ZE=typeof clearTimeout=="function"?clearTimeout:void 0,iy=typeof Promise=="function"?Promise:void 0,e3=typeof queueMicrotask=="function"?queueMicrotask:typeof iy<"u"?function(e){return iy.resolve(null).then(e).catch(t3)}:qx;function t3(e){setTimeout(function(){throw e})}function lr(e){return e==="head"}function oy(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&xo(i.documentElement),a&2&&xo(i.body),a&4)for(a=i.head,xo(a),i=a.firstChild;i;){var o=i.nextSibling,l=i.nodeName;i[Fo]||l==="SCRIPT"||l==="STYLE"||l==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),Oo(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);Oo(t)}function bf(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":bf(a),Tf(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function a3(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[Fo])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=Ca(e.nextSibling),e===null)break}return null}function n3(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=Ca(e.nextSibling),e===null))return null;return e}function xf(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function r3(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function Ca(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var $f=null;function ly(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function Ix(e,t,a){switch(t=Ku(a),e){case"html":if(e=t.documentElement,!e)throw Error(j(452));return e;case"head":if(e=t.head,!e)throw Error(j(453));return e;case"body":if(e=t.body,!e)throw Error(j(454));return e;default:throw Error(j(451))}}function xo(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);Tf(e)}var $a=new Map,uy=new Set;function Qu(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var _n=be.d;be.d={f:s3,r:i3,D:o3,C:l3,L:u3,m:c3,X:m3,S:d3,M:f3};function s3(){var e=_n.f(),t=oc();return e||t}function i3(e){var t=Ys(e);t!==null&&t.tag===5&&t.type==="form"?Pb(t):_n.r(e)}var Ws=typeof document>"u"?null:document;function Hx(e,t,a){var n=Ws;if(n&&typeof t=="string"&&t){var r=ga(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),uy.has(r)||(uy.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),xt(t,"link",e),ct(t),n.head.appendChild(t)))}}function o3(e){_n.D(e),Hx("dns-prefetch",e,null)}function l3(e,t){_n.C(e,t),Hx("preconnect",e,t)}function u3(e,t,a){_n.L(e,t,a);var n=Ws;if(n&&e&&t){var r='link[rel="preload"][as="'+ga(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+ga(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+ga(a.imageSizes)+'"]')):r+='[href="'+ga(e)+'"]';var s=r;switch(t){case"style":s=Vs(e);break;case"script":s=Zs(e)}$a.has(s)||(e=Me({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),$a.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(Yo(s))||t==="script"&&n.querySelector(Jo(s))||(t=n.createElement("link"),xt(t,"link",e),ct(t),n.head.appendChild(t)))}}function c3(e,t){_n.m(e,t);var a=Ws;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+ga(n)+'"][href="'+ga(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=Zs(e)}if(!$a.has(s)&&(e=Me({rel:"modulepreload",href:e},t),$a.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(Jo(s)))return}n=a.createElement("link"),xt(n,"link",e),ct(n),a.head.appendChild(n)}}}function d3(e,t,a){_n.S(e,t,a);var n=Ws;if(n&&e){var r=Es(n).hoistableStyles,s=Vs(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(Yo(s)))o.loading=5;else{e=Me({rel:"stylesheet",href:e,"data-precedence":t},a),(a=$a.get(s))&&fp(e,a);var l=i=n.createElement("link");ct(l),xt(l,"link",e),l._p=new Promise(function(c,d){l.onload=c,l.onerror=d}),l.addEventListener("load",function(){o.loading|=1}),l.addEventListener("error",function(){o.loading|=2}),o.loading|=4,yu(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function m3(e,t){_n.X(e,t);var a=Ws;if(a&&e){var n=Es(a).hoistableScripts,r=Zs(e),s=n.get(r);s||(s=a.querySelector(Jo(r)),s||(e=Me({src:e,async:!0},t),(t=$a.get(r))&&pp(e,t),s=a.createElement("script"),ct(s),xt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function f3(e,t){_n.M(e,t);var a=Ws;if(a&&e){var n=Es(a).hoistableScripts,r=Zs(e),s=n.get(r);s||(s=a.querySelector(Jo(r)),s||(e=Me({src:e,async:!0,type:"module"},t),(t=$a.get(r))&&pp(e,t),s=a.createElement("script"),ct(s),xt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function cy(e,t,a,n){var r=(r=Jn.current)?Qu(r):null;if(!r)throw Error(j(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=Vs(a.href),a=Es(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=Vs(a.href);var s=Es(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(Yo(e)))&&!s._p&&(i.instance=s,i.state.loading=5),$a.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},$a.set(e,a),s||p3(r,e,a,i.state))),t&&n===null)throw Error(j(528,""));return i}if(t&&n!==null)throw Error(j(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=Zs(a),a=Es(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(j(444,e))}}function Vs(e){return'href="'+ga(e)+'"'}function Yo(e){return'link[rel="stylesheet"]['+e+"]"}function Kx(e){return Me({},e,{"data-precedence":e.precedence,precedence:null})}function p3(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),xt(t,"link",a),ct(t),e.head.appendChild(t))}function Zs(e){return'[src="'+ga(e)+'"]'}function Jo(e){return"script[async]"+e}function dy(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+ga(a.href)+'"]');if(n)return t.instance=n,ct(n),n;var r=Me({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),ct(n),xt(n,"style",r),yu(n,a.precedence,e),t.instance=n;case"stylesheet":r=Vs(a.href);var s=e.querySelector(Yo(r));if(s)return t.state.loading|=4,t.instance=s,ct(s),s;n=Kx(a),(r=$a.get(r))&&fp(n,r),s=(e.ownerDocument||e).createElement("link"),ct(s);var i=s;return i._p=new Promise(function(o,l){i.onload=o,i.onerror=l}),xt(s,"link",n),t.state.loading|=4,yu(s,a.precedence,e),t.instance=s;case"script":return s=Zs(a.src),(r=e.querySelector(Jo(s)))?(t.instance=r,ct(r),r):(n=a,(r=$a.get(s))&&(n=Me({},a),pp(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),ct(r),xt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(j(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,yu(n,a.precedence,e));return t.instance}function yu(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function fp(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function pp(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var bu=null;function my(e,t,a){if(bu===null){var n=new Map,r=bu=new Map;r.set(a,n)}else r=bu,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[Fo]||s[St]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function fy(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function h3(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function Qx(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var To=null;function v3(){}function g3(e,t,a){if(To===null)throw Error(j(475));var n=To;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=Vs(a.href),s=e.querySelector(Yo(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=Vu.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,ct(s);return}s=e.ownerDocument||e,a=Kx(a),(r=$a.get(r))&&fp(a,r),s=s.createElement("link"),ct(s);var i=s;i._p=new Promise(function(o,l){i.onload=o,i.onerror=l}),xt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=Vu.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function y3(){if(To===null)throw Error(j(475));var e=To;return e.stylesheets&&e.count===0&&wf(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&wf(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function Vu(){if(this.count--,this.count===0){if(this.stylesheets)wf(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var Gu=null;function wf(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,Gu=new Map,t.forEach(b3,e),Gu=null,Vu.call(e))}function b3(e,t){if(!(t.state.loading&4)){var a=Gu.get(e);if(a)var n=a.get(null);else{a=new Map,Gu.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=Vu.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var Ao={$$typeof:pn,Provider:null,Consumer:null,_currentValue:Rr,_currentValue2:Rr,_threadCount:0};function x3(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=Zd(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=Zd(0),this.hiddenUpdates=Zd(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function Vx(e,t,a,n,r,s,i,o,l,c,d,m){return e=new x3(e,t,a,i,o,l,c,m),t=1,s===!0&&(t|=24),s=Xt(3,null,null,t),e.current=s,s.stateNode=e,t=qf(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},Hf(s),e}function Gx(e){return e?(e=_s,e):_s}function Yx(e,t,a,n,r,s){r=Gx(r),n.context===null?n.context=r:n.pendingContext=r,n=Xn(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=Wn(e,n,t),a!==null&&(ta(a,e,t),mo(a,e,t))}function py(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function hp(e,t){py(e,t),(e=e.alternate)&&py(e,t)}function Jx(e){if(e.tag===13){var t=Js(e,67108864);t!==null&&ta(t,e,67108864),hp(e,67108864)}}var Yu=!0;function $3(e,t,a,n){var r=se.T;se.T=null;var s=be.p;try{be.p=2,vp(e,t,a,n)}finally{be.p=s,se.T=r}}function w3(e,t,a,n){var r=se.T;se.T=null;var s=be.p;try{be.p=8,vp(e,t,a,n)}finally{be.p=s,se.T=r}}function vp(e,t,a,n){if(Yu){var r=Sf(n);if(r===null)km(e,t,n,Ju,a),hy(e,n);else if(N3(r,e,t,a,n))n.stopPropagation();else if(hy(e,n),t&4&&-1<S3.indexOf(e)){for(;r!==null;){var s=Ys(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=Sr(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var l=1<<31-Zt(i);o.entanglements[1]|=l,i&=~l}Xa(s),(Se&6)===0&&(Bu=Ga()+500,Go(0,!1))}}break;case 13:o=Js(s,2),o!==null&&ta(o,s,2),oc(),hp(s,2)}if(s=Sf(n),s===null&&km(e,t,n,Ju,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else km(e,t,n,null,a)}}function Sf(e){return e=Df(e),gp(e)}var Ju=null;function gp(e){if(Ju=null,e=bs(e),e!==null){var t=Lo(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=$y(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return Ju=e,null}function Xx(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(cC()){case _y:return 2;case Ry:return 8;case Nu:case dC:return 32;case ky:return 268435456;default:return 32}default:return 32}}var Nf=!1,tr=null,ar=null,nr=null,Do=new Map,Mo=new Map,Kn=[],S3="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function hy(e,t){switch(e){case"focusin":case"focusout":tr=null;break;case"dragenter":case"dragleave":ar=null;break;case"mouseover":case"mouseout":nr=null;break;case"pointerover":case"pointerout":Do.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":Mo.delete(t.pointerId)}}function eo(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Ys(t),t!==null&&Jx(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function N3(e,t,a,n,r){switch(t){case"focusin":return tr=eo(tr,e,t,a,n,r),!0;case"dragenter":return ar=eo(ar,e,t,a,n,r),!0;case"mouseover":return nr=eo(nr,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return Do.set(s,eo(Do.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,Mo.set(s,eo(Mo.get(s)||null,e,t,a,n,r)),!0}return!1}function Wx(e){var t=bs(e.target);if(t!==null){var a=Lo(t);if(a!==null){if(t=a.tag,t===13){if(t=$y(a),t!==null){e.blockedOn=t,bC(e.priority,function(){if(a.tag===13){var n=ea();n=Cf(n);var r=Js(a,n);r!==null&&ta(r,a,n),hp(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function xu(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=Sf(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);Bm=n,a.target.dispatchEvent(n),Bm=null}else return t=Ys(a),t!==null&&Jx(t),e.blockedOn=a,!1;t.shift()}return!0}function vy(e,t,a){xu(e)&&a.delete(t)}function _3(){Nf=!1,tr!==null&&xu(tr)&&(tr=null),ar!==null&&xu(ar)&&(ar=null),nr!==null&&xu(nr)&&(nr=null),Do.forEach(vy),Mo.forEach(vy)}function iu(e,t){e.blockedOn===t&&(e.blockedOn=null,Nf||(Nf=!0,st.unstable_scheduleCallback(st.unstable_NormalPriority,_3)))}var ou=null;function gy(e){ou!==e&&(ou=e,st.unstable_scheduleCallback(st.unstable_NormalPriority,function(){ou===e&&(ou=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(gp(n||a)===null)continue;break}var s=Ys(a);s!==null&&(e.splice(t,3),t-=3,af(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function Oo(e){function t(l){return iu(l,e)}tr!==null&&iu(tr,e),ar!==null&&iu(ar,e),nr!==null&&iu(nr,e),Do.forEach(t),Mo.forEach(t);for(var a=0;a<Kn.length;a++){var n=Kn[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<Kn.length&&(a=Kn[0],a.blockedOn===null);)Wx(a),a.blockedOn===null&&Kn.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[qt]||null;if(typeof s=="function")i||gy(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[qt]||null)o=i.formAction;else if(gp(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),gy(a)}}}function yp(e){this._internalRoot=e}dc.prototype.render=yp.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(j(409));var a=t.current,n=ea();Yx(a,n,e,t,null,null)};dc.prototype.unmount=yp.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;Yx(e.current,2,null,e,null,null),oc(),t[Gs]=null}};function dc(e){this._internalRoot=e}dc.prototype.unstable_scheduleHydration=function(e){if(e){var t=Dy();e={blockedOn:null,target:e,priority:t};for(var a=0;a<Kn.length&&t!==0&&t<Kn[a].priority;a++);Kn.splice(a,0,e),a===0&&Wx(e)}};var yy=by.version;if(yy!=="19.1.0")throw Error(j(527,yy,"19.1.0"));be.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(j(188)):(e=Object.keys(e).join(","),Error(j(268,e)));return e=nC(t),e=e!==null?wy(e):null,e=e===null?null:e.stateNode,e};var R3={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:se,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(to=__REACT_DEVTOOLS_GLOBAL_HOOK__,!to.isDisabled&&to.supportsFiber))try{Po=to.inject(R3),Wt=to}catch{}var to;mc.createRoot=function(e,t){if(!xy(e))throw Error(j(299));var a=!1,n="",r=Vb,s=Gb,i=Yb,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=Vx(e,1,!1,null,null,a,n,r,s,i,o,null),e[Gs]=t.current,mp(e),new yp(t)};mc.hydrateRoot=function(e,t,a){if(!xy(e))throw Error(j(299));var n=!1,r="",s=Vb,i=Gb,o=Yb,l=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(l=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=Vx(e,1,!0,t,a??null,n,r,s,i,o,l,c),t.context=Gx(null),a=t.current,n=ea(),n=Cf(n),r=Xn(n),r.callback=null,Wn(a,r,n),a=n,t.current.lanes=a,jo(t,a),Xa(t),e[Gs]=t.current,mp(e),new dc(t)};mc.version="19.1.0"});var a0=Mn((RP,t0)=>{"use strict";function e0(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(e0)}catch(e){console.error(e)}}e0(),t0.exports=Zx()});var Pt=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var Pk={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},Uk=class{#t=Pk;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},Ba=new Uk;function dv(e){setTimeout(e,0)}var Ut=typeof window>"u"||"Deno"in globalThis;function Pe(){}function pv(e,t){return typeof e=="function"?e(t):e}function Pi(e){return typeof e=="number"&&e>=0&&e!==1/0}function Nl(e,t){return Math.max(e+(t||0)-Date.now(),0)}function Ra(e,t){return typeof e=="function"?e(t):e}function jt(e,t){return typeof e=="function"?e(t):e}function _l(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==Ui(i,t.options))return!1}else if(!xr(t.queryKey,i))return!1}if(a!=="all"){let l=t.isActive();if(a==="active"&&!l||a==="inactive"&&l)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function Rl(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(za(t.options.mutationKey)!==za(s))return!1}else if(!xr(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function Ui(e,t){return(t?.queryKeyHashFn||za)(e)}function za(e){return JSON.stringify(e,(t,a)=>Td(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function xr(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>xr(e[a],t[a])):!1}var jk=Object.prototype.hasOwnProperty;function ji(e,t){if(e===t)return e;let a=mv(e)&&mv(t);if(!a&&!(Td(e)&&Td(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},l=0;for(let c=0;c<i;c++){let d=a?c:s[c],m=e[d],f=t[d];if(m===f){o[d]=m,(a?c<r:jk.call(e,d))&&l++;continue}if(m===null||f===null||typeof m!="object"||typeof f!="object"){o[d]=f;continue}let h=ji(m,f);o[d]=h,h===m&&l++}return r===i&&l===r?e:o}function On(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function mv(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function Td(e){if(!fv(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!fv(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function fv(e){return Object.prototype.toString.call(e)==="[object Object]"}function hv(e){return new Promise(t=>{Ba.setTimeout(t,e)})}function Fi(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?ji(e,t):t}function vv(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function gv(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var rs=Symbol();function kl(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===rs?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function Bi(e,t){return typeof e=="function"?e(...t):!!e}var Fk=class extends Pt{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!Ut&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},ss=new Fk;function zi(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var yv=dv;function Bk(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=yv,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(l=>{a(l)})})})};return{batch:o=>{let l;t++;try{l=o()}finally{t--,t||i()}return l},batchCalls:o=>(...l)=>{s(()=>{o(...l)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var pe=Bk();var zk=class extends Pt{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!Ut&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},is=new zk;function qk(e){return Math.min(1e3*2**e,3e4)}function Ad(e){return(e??"online")==="online"?is.isOnline():!0}var Cl=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function El(e){let t=!1,a=0,n,r=zi(),s=()=>r.status!=="pending",i=y=>{if(!s()){let $=new Cl(y);f($),e.onCancel?.($)}},o=()=>{t=!0},l=()=>{t=!1},c=()=>ss.isFocused()&&(e.networkMode==="always"||is.isOnline())&&e.canRun(),d=()=>Ad(e.networkMode)&&e.canRun(),m=y=>{s()||(n?.(),r.resolve(y))},f=y=>{s()||(n?.(),r.reject(y))},h=()=>new Promise(y=>{n=$=>{(s()||c())&&y($)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),x=()=>{if(s())return;let y,$=a===0?e.initialPromise:void 0;try{y=$??e.fn()}catch(g){y=Promise.reject(g)}Promise.resolve(y).then(m).catch(g=>{if(s())return;let v=e.retry??(Ut?0:3),b=e.retryDelay??qk,w=typeof b=="function"?b(a,g):b,S=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!S){f(g);return}a++,e.onFail?.(a,g),hv(w).then(()=>c()?void 0:h()).then(()=>{t?f(g):x()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:l,canStart:d,start:()=>(d()?x():h().then(x),r)}}var Tl=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),Pi(this.gcTime)&&(this.#t=Ba.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(Ut?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(Ba.clearTimeout(this.#t),this.#t=void 0)}};var xv=class extends Tl{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=bv(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=bv(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=Fi(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(Pe).catch(Pe):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>jt(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===rs||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>Ra(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!Nl(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(l=>l.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=kl(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=El({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof Cl&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,l)=>{this.#i({type:"failed",failureCount:o,error:l})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof Cl){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,...Dd(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),pe.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function Dd(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:Ad(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function bv(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var $r=class extends Pt{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=zi(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),$v(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return Md(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return Md(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof jt(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!On(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&wv(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||jt(this.options.enabled,this.#e)!==jt(t.enabled,this.#e)||Ra(this.options.staleTime,this.#e)!==Ra(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||jt(this.options.enabled,this.#e)!==jt(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return Hk(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(Pe)),t}#v(){this.#x();let e=Ra(this.options.staleTime,this.#e);if(Ut||this.#n.isStale||!Pi(e))return;let a=Nl(this.#n.dataUpdatedAt,e)+1;this.#u=Ba.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(Ut||jt(this.options.enabled,this.#e)===!1||!Pi(this.#l)||this.#l===0)&&(this.#c=Ba.setInterval(()=>{(this.options.refetchIntervalInBackground||ss.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(Ba.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(Ba.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,l=e!==a?e.state:this.#a,{state:c}=e,d={...c},m=!1,f;if(t._optimisticResults){let _=this.hasListeners(),A=!_&&$v(e,t),L=_&&wv(e,a,t,n);(A||L)&&(d={...d,...Dd(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:h,errorUpdatedAt:x,status:y}=d;f=d.data;let $=!1;if(t.placeholderData!==void 0&&f===void 0&&y==="pending"){let _;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(_=r.data,$=!0):_=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,_!==void 0&&(y="success",f=Fi(r?.data,_,t),m=!0)}if(t.select&&f!==void 0&&!$)if(r&&f===s?.data&&t.select===this.#f)f=this.#d;else try{this.#f=t.select,f=t.select(f),f=Fi(r?.data,f,t),this.#d=f,this.#i=null}catch(_){this.#i=_}this.#i&&(h=this.#i,f=this.#d,x=Date.now(),y="error");let g=d.fetchStatus==="fetching",v=y==="pending",b=y==="error",w=v&&g,S=f!==void 0,R={status:y,fetchStatus:d.fetchStatus,isPending:v,isSuccess:y==="success",isError:b,isInitialLoading:w,isLoading:w,data:f,dataUpdatedAt:d.dataUpdatedAt,error:h,errorUpdatedAt:x,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>l.dataUpdateCount||d.errorUpdateCount>l.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:b&&!S,isPaused:d.fetchStatus==="paused",isPlaceholderData:m,isRefetchError:b&&S,isStale:Od(e,t),refetch:this.refetch,promise:this.#o,isEnabled:jt(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let _=U=>{R.status==="error"?U.reject(R.error):R.data!==void 0&&U.resolve(R.data)},A=()=>{let U=this.#o=R.promise=zi();_(U)},L=this.#o;switch(L.status){case"pending":e.queryHash===a.queryHash&&_(L);break;case"fulfilled":(R.status==="error"||R.data!==L.value)&&A();break;case"rejected":(R.status!=="error"||R.error!==L.reason)&&A();break}}return R}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),On(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){pe.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function Ik(e,t){return jt(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function $v(e,t){return Ik(e,t)||e.state.data!==void 0&&Md(e,t,t.refetchOnMount)}function Md(e,t,a){if(jt(t.enabled,e)!==!1&&Ra(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&Od(e,t)}return!1}function wv(e,t,a,n){return(e!==t||jt(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&Od(e,a)}function Od(e,t){return jt(t.enabled,e)!==!1&&e.isStaleByTime(Ra(t.staleTime,e))}function Hk(e,t){return!On(e.getCurrentResult(),t)}function Ld(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},l=0,c=async()=>{let d=!1,m=x=>{Object.defineProperty(x,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},f=kl(t.options,t.fetchOptions),h=async(x,y,$)=>{if(d)return Promise.reject();if(y==null&&x.pages.length)return Promise.resolve(x);let v=(()=>{let C={client:t.client,queryKey:t.queryKey,pageParam:y,direction:$?"backward":"forward",meta:t.options.meta};return m(C),C})(),b=await f(v),{maxPages:w}=t.options,S=$?gv:vv;return{pages:S(x.pages,b,w),pageParams:S(x.pageParams,y,w)}};if(r&&s.length){let x=r==="backward",y=x?Kk:Sv,$={pages:s,pageParams:i},g=y(n,$);o=await h($,g,x)}else{let x=e??s.length;do{let y=l===0?i[0]??n.initialPageParam:Sv(n,o);if(l>0&&y==null)break;o=await h(o,y),l++}while(l<x)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function Sv(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function Kk(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var Nv=class extends Tl{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||Pd(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=El({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),pe.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function Pd(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var _v=class extends Pt{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new Nv({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=Al(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=Al(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=Al(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=Al(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){pe.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>Rl(t,a))}findAll(e={}){return this.getAll().filter(t=>Rl(e,t))}notify(e){pe.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return pe.batch(()=>Promise.all(e.map(t=>t.continue().catch(Pe))))}};function Al(e){return e.options.scope?.id}var Ud=class extends Pt{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),On(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&za(t.mutationKey)!==za(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??Pd();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){pe.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function Rv(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function Qk(e,t,a){let n=e.slice(0);return n[t]=a,n}var jd=class extends Pt{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,pe.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,m)=>d!==a[m]),l=i||o,c=l?!0:s.some((d,m)=>{let f=this.#e[m];return!f||!On(d,f)});!l&&!c||(l&&(this.#r=r),this.#e=s,this.hasListeners()&&(l&&(Rv(a,r).forEach(d=>{d.destroy()}),Rv(r,a).forEach(d=>{d.subscribe(m=>{this.#c(d,m)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=ji(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new $r(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=Qk(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&pe.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var kv=class extends Pt{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??Ui(n,t),s=this.get(r);return s||(s=new xv({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){pe.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>_l(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>_l(e,a)):t}notify(e){pe.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){pe.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){pe.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var Fd=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new kv,this.#e=e.mutationCache||new _v,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=ss.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=is.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime(Ra(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=pv(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return pe.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;pe.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return pe.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=pe.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(Pe).catch(Pe)}invalidateQueries(e,t={}){return pe.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=pe.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(Pe)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(Pe)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime(Ra(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(Pe).catch(Pe)}fetchInfiniteQuery(e){return e.behavior=Ld(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(Pe).catch(Pe)}ensureInfiniteQueryData(e){return e.behavior=Ld(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return is.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(za(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{xr(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(za(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{xr(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=Ui(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===rs&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var qa=qe(Qe(),1);var os=qe(Qe(),1),Av=qe(Bd(),1),zd=os.createContext(void 0),Z=e=>{let t=os.useContext(zd);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},qd=({client:e,children:t})=>(os.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,Av.jsx)(zd.Provider,{value:e,children:t}));var Ml=qe(Qe(),1),Dv=Ml.createContext(!1),Ol=()=>Ml.useContext(Dv),q6=Dv.Provider;var qi=qe(Qe(),1),Yk=qe(Bd(),1);function Jk(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var Xk=qi.createContext(Jk()),Ll=()=>qi.useContext(Xk);var Mv=qe(Qe(),1);var Pl=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},Ul=e=>{Mv.useEffect(()=>{e.clearReset()},[e])},jl=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||Bi(a,[e.error,n]));var Fl=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},Bl=(e,t)=>e.isLoading&&e.isFetching&&!t,Ii=(e,t)=>e?.suspense&&t.isPending,ls=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function Id({queries:e,...t},a){let n=Z(a),r=Ol(),s=Ll(),i=qa.useMemo(()=>e.map(y=>{let $=n.defaultQueryOptions(y);return $._optimisticResults=r?"isRestoring":"optimistic",$}),[e,n,r]);i.forEach(y=>{Fl(y),Pl(y,s)}),Ul(s);let[o]=qa.useState(()=>new jd(n,i,t)),[l,c,d]=o.getOptimisticResult(i,t.combine),m=!r&&t.subscribed!==!1;qa.useSyncExternalStore(qa.useCallback(y=>m?o.subscribe(pe.batchCalls(y)):Pe,[o,m]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),qa.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let h=l.some((y,$)=>Ii(i[$],y))?l.flatMap((y,$)=>{let g=i[$];if(g){let v=new $r(n,g);if(Ii(g,y))return ls(g,v,s);Bl(y,r)&&ls(g,v,s)}return[]}):[];if(h.length>0)throw Promise.all(h);let x=l.find((y,$)=>{let g=i[$];return g&&jl({result:y,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(x?.error)throw x.error;return c(d())}var Ln=qe(Qe(),1);function Ov(e,t,a){let n=Ol(),r=Ll(),s=Z(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",Fl(i),Pl(i,r),Ul(r);let o=!s.getQueryCache().get(i.queryHash),[l]=Ln.useState(()=>new t(s,i)),c=l.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(Ln.useSyncExternalStore(Ln.useCallback(m=>{let f=d?l.subscribe(pe.batchCalls(m)):Pe;return l.updateResult(),f},[l,d]),()=>l.getCurrentResult(),()=>l.getCurrentResult()),Ln.useEffect(()=>{l.setOptions(i)},[i,l]),Ii(i,c))throw ls(i,l,r);if(jl({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!Ut&&Bl(c,n)&&(o?ls(i,l,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(Pe).finally(()=>{l.updateResult()}),i.notifyOnChangeProps?c:l.trackResult(c)}function K(e,t){return Ov(e,$r,t)}var ln=qe(Qe(),1);function Y(e,t){let a=Z(t),[n]=ln.useState(()=>new Ud(a,e));ln.useEffect(()=>{n.setOptions(e)},[n,e]);let r=ln.useSyncExternalStore(ln.useCallback(i=>n.subscribe(pe.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=ln.useCallback((i,o)=>{n.mutate(i,o).catch(Pe)},[n]);if(r.error&&Bi(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var Mk=qe(a0());var Ht=qe(Qe(),1),W=qe(Qe(),1),ke=qe(Qe(),1),Up=qe(Qe(),1),P0=qe(Qe(),1),fe=qe(Qe(),1),BT=qe(Qe(),1),zT=qe(Qe(),1),qT=qe(Qe(),1),te=qe(Qe(),1),G0=qe(Qe(),1);var n0="popstate";function r0(e){return typeof e=="object"&&e!=null&&"pathname"in e&&"search"in e&&"hash"in e&&"state"in e&&"key"in e}function d0(e={}){function t(n,r){let s=r.state?.masked,{pathname:i,search:o,hash:l}=s||n.location;return wp("",{pathname:i,search:o,hash:l},r.state&&r.state.usr||null,r.state&&r.state.key||"default",s?{pathname:n.location.pathname,search:n.location.search,hash:n.location.hash}:void 0)}function a(n,r){return typeof r=="string"?r:ei(r)}return C3(t,a,null,e)}function Te(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function na(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function k3(){return Math.random().toString(36).substring(2,10)}function s0(e,t){return{usr:e.state,key:e.key,idx:t,masked:e.mask?{pathname:e.pathname,search:e.search,hash:e.hash}:void 0}}function wp(e,t,a=null,n,r){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?zr(t):t,state:a,key:t&&t.key||n||k3(),mask:r}}function ei({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function zr(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function C3(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",l=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function m(){o="POP";let $=d(),g=$==null?null:$-c;c=$,l&&l({action:o,location:y.location,delta:g})}function f($,g){o="PUSH";let v=r0($)?$:wp(y.location,$,g);a&&a(v,$),c=d()+1;let b=s0(v,c),w=y.createHref(v.mask||v);try{i.pushState(b,"",w)}catch(S){if(S instanceof DOMException&&S.name==="DataCloneError")throw S;r.location.assign(w)}s&&l&&l({action:o,location:y.location,delta:1})}function h($,g){o="REPLACE";let v=r0($)?$:wp(y.location,$,g);a&&a(v,$),c=d();let b=s0(v,c),w=y.createHref(v.mask||v);i.replaceState(b,"",w),s&&l&&l({action:o,location:y.location,delta:0})}function x($){return E3($)}let y={get action(){return o},get location(){return e(r,i)},listen($){if(l)throw new Error("A history only accepts one active listener");return r.addEventListener(n0,m),l=$,()=>{r.removeEventListener(n0,m),l=null}},createHref($){return t(r,$)},createURL:x,encodeLocation($){let g=x($);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:f,replace:h,go($){return i.go($)}};return y}function E3(e,t=!1){let a="http://localhost";typeof window<"u"&&(a=window.location.origin!=="null"?window.location.origin:window.location.href),Te(a,"No window.location.(origin|href) available to create URL");let n=typeof e=="string"?e:ei(e);return n=n.replace(/ $/,"%20"),!t&&n.startsWith("//")&&(n=a+n),new URL(n,a)}var T3;T3=new WeakMap;function Rp(e,t,a="/"){return A3(e,t,a,!1)}function A3(e,t,a,n,r){let s=typeof t=="string"?zr(t):t,i=Wa(s.pathname||"/",a);if(i==null)return null;let o=r??M3(e),l=null,c=K3(i);for(let d=0;l==null&&d<o.length;++d)l=I3(o[d],c,n);return l}function D3(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function M3(e){let t=m0(e);return O3(t),t}function m0(e,t=[],a=[],n="",r=!1){let s=(i,o,l=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&l)return;Te(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let m=Ta([n,d.relativePath]),f=a.concat(d);i.children&&i.children.length>0&&(Te(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${m}".`),m0(i.children,t,f,m,l)),!(i.path==null&&!i.index)&&t.push({path:m,score:z3(m,i.index),routesMeta:f})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let l of f0(i.path))s(i,o,!0,l)}),t}function f0(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=f0(n.join("/")),o=[];return o.push(...i.map(l=>l===""?s:[s,l].join("/"))),r&&o.push(...i),o.map(l=>e.startsWith("/")&&l===""?"/":l)}function O3(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:q3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var L3=/^:[\w-]+$/,P3=3,U3=2,j3=1,F3=10,B3=-2,i0=e=>e==="*";function z3(e,t){let a=e.split("/"),n=a.length;return a.some(i0)&&(n+=B3),t&&(n+=U3),a.filter(r=>!i0(r)).reduce((r,s)=>r+(L3.test(s)?P3:s===""?j3:F3),n)}function q3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function I3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let l=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",m=Wo({path:l.relativePath,caseSensitive:l.caseSensitive,end:c},d),f=l.route;if(!m&&c&&a&&!n[n.length-1].route.index&&(m=Wo({path:l.relativePath,caseSensitive:l.caseSensitive,end:!1},d)),!m)return null;Object.assign(r,m.params),i.push({params:r,pathname:Ta([s,m.pathname]),pathnameBase:G3(Ta([s,m.pathnameBase])),route:f}),m.pathnameBase!=="/"&&(s=Ta([s,m.pathnameBase]))}return i}function Wo(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=H3(e.path,e.caseSensitive,e.end),r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:m},f)=>{if(d==="*"){let x=o[f]||"";i=s.slice(0,s.length-x.length).replace(/(.)\/+$/,"$1")}let h=o[f];return m&&!h?c[d]=void 0:c[d]=(h||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function H3(e,t=!1,a=!0){na(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,l,c,d)=>{if(n.push({paramName:o,isOptional:l!=null}),l){let m=d.charAt(c+i.length);return m&&m!=="/"?"/([^\\/]*)":"(?:/([^\\/]*))?"}return"/([^\\/]+)"}).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function K3(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return na(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function Wa(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}var Q3=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i;function p0(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?zr(e):e,s;return a?(a=h0(a),a.startsWith("/")?s=o0(a.substring(1),"/"):s=o0(a,t)):s=t,{pathname:s,search:Y3(n),hash:J3(r)}}function o0(e,t){let a=gc(t).split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function bp(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function V3(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function kp(e){let t=V3(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function bc(e,t,a,n=!1){let r;typeof e=="string"?r=zr(e):(r={...e},Te(!r.pathname||!r.pathname.includes("?"),bp("?","pathname","search",r)),Te(!r.pathname||!r.pathname.includes("#"),bp("#","pathname","hash",r)),Te(!r.search||!r.search.includes("#"),bp("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let m=t.length-1;if(!n&&i.startsWith("..")){let f=i.split("/");for(;f[0]==="..";)f.shift(),m-=1;r.pathname=f.join("/")}o=m>=0?t[m]:"/"}let l=p0(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!l.pathname.endsWith("/")&&(c||d)&&(l.pathname+="/"),l}var h0=e=>e.replace(/\/\/+/g,"/"),Ta=e=>h0(e.join("/")),gc=e=>e.replace(/\/+$/,""),G3=e=>gc(e).replace(/^\/*/,"/"),Y3=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,J3=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;var v0=class{constructor(e,t,a,n=!1){this.status=e,this.statusText=t||"",this.internal=n,a instanceof Error?(this.data=a.toString(),this.error=a):this.data=a}};function g0(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}function X3(e){let t=e.map(a=>a.route.path).filter(Boolean);return Ta(t)||"/"}var y0=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";function b0(e,t){let a=e;if(typeof a!="string"||!Q3.test(a))return{absoluteURL:void 0,isExternal:!1,to:a};let n=a,r=!1;if(y0)try{let s=new URL(window.location.href),i=a.startsWith("//")?new URL(s.protocol+a):new URL(a),o=Wa(i.pathname,t);i.origin===s.origin&&o!=null?a=o+i.search+i.hash:r=!0}catch{na(!1,`<Link to="${a}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}return{absoluteURL:n,isExternal:r,to:a}}var kP=Symbol("Uninstrumented");var CP=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");var x0=["POST","PUT","PATCH","DELETE"],EP=new Set(x0),W3=["GET",...x0],TP=new Set(W3);var AP=Symbol("ResetLoaderData"),Z3,eT,tT,aT;Z3=new WeakMap;eT=new WeakMap;tT=new WeakMap;aT=new WeakMap;var qr=Ht.createContext(null);qr.displayName="DataRouter";var ti=Ht.createContext(null);ti.displayName="DataRouterState";var $0=Ht.createContext(!1);function nT(){return Ht.useContext($0)}var Cp=Ht.createContext({isTransitioning:!1});Cp.displayName="ViewTransition";var w0=Ht.createContext(new Map);w0.displayName="Fetchers";var rT=Ht.createContext(null);rT.displayName="Await";var _t=Ht.createContext(null);_t.displayName="Navigation";var ai=Ht.createContext(null);ai.displayName="Location";var ra=Ht.createContext({outlet:null,matches:[],isDataRoute:!1});ra.displayName="Route";var Ep=Ht.createContext(null);Ep.displayName="RouteError";var Sp=!0,S0="REACT_ROUTER_ERROR",sT="REDIRECT",iT="ROUTE_ERROR_RESPONSE";function oT(e){if(e.startsWith(`${S0}:${sT}:{`))try{let t=JSON.parse(e.slice(28));if(typeof t=="object"&&t&&typeof t.status=="number"&&typeof t.statusText=="string"&&typeof t.location=="string"&&typeof t.reloadDocument=="boolean"&&typeof t.replace=="boolean")return t}catch{}}function lT(e){if(e.startsWith(`${S0}:${iT}:{`))try{let t=JSON.parse(e.slice(40));if(typeof t=="object"&&t&&typeof t.status=="number"&&typeof t.statusText=="string")return new v0(t.status,t.statusText,t.data)}catch{}}function N0(e,{relative:t}={}){Te(Ir(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=W.useContext(_t),{hash:r,pathname:s,search:i}=ni(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:Ta([a,s])),n.createHref({pathname:o,search:i,hash:r})}function Ir(){return W.useContext(ai)!=null}function Ae(){return Te(Ir(),"useLocation() may be used only in the context of a <Router> component."),W.useContext(ai).location}var _0="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function R0(e){W.useContext(_t).static||W.useLayoutEffect(e)}function ve(){let{isDataRoute:e}=W.useContext(ra);return e?yT():uT()}function uT(){Te(Ir(),"useNavigate() may be used only in the context of a <Router> component.");let e=W.useContext(qr),{basename:t,navigator:a}=W.useContext(_t),{matches:n}=W.useContext(ra),{pathname:r}=Ae(),s=JSON.stringify(kp(n)),i=W.useRef(!1);return R0(()=>{i.current=!0}),W.useCallback((l,c={})=>{if(na(i.current,_0),!i.current)return;if(typeof l=="number"){a.go(l);return}let d=bc(l,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:Ta([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var k0=W.createContext(null);function wa(){return W.useContext(k0)}function C0(e){let t=W.useContext(ra).outlet;return W.useMemo(()=>t&&W.createElement(k0.Provider,{value:e},t),[t,e])}function it(){let{matches:e}=W.useContext(ra);return e[e.length-1]?.params??{}}function ni(e,{relative:t}={}){let{matches:a}=W.useContext(ra),{pathname:n}=Ae(),r=JSON.stringify(kp(a));return W.useMemo(()=>bc(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function E0(e,t){return T0(e,t)}function T0(e,t,a){Te(Ir(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:n}=W.useContext(_t),{matches:r}=W.useContext(ra),s=r[r.length-1],i=s?s.params:{},o=s?s.pathname:"/",l=s?s.pathnameBase:"/",c=s&&s.route;if(Sp){let $=c&&c.path||"";O0(o,!c||$.endsWith("*")||$.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${o}" (under <Route path="${$}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${$}"> to <Route path="${$==="/"?"*":`${$}/*`}">.`)}let d=Ae(),m;if(t){let $=typeof t=="string"?zr(t):t;Te(l==="/"||$.pathname?.startsWith(l),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${l}" but pathname "${$.pathname}" was given in the \`location\` prop.`),m=$}else m=d;let f=m.pathname||"/",h=f;if(l!=="/"){let $=l.replace(/^\//,"").split("/");h="/"+f.replace(/^\//,"").split("/").slice($.length).join("/")}let x=a&&a.state.matches.length?a.state.matches.map($=>Object.assign($,{route:a.manifest[$.route.id]||$.route})):Rp(e,{pathname:h});Sp&&(na(c||x!=null,`No routes matched location "${m.pathname}${m.search}${m.hash}" `),na(x==null||x[x.length-1].route.element!==void 0||x[x.length-1].route.Component!==void 0||x[x.length-1].route.lazy!==void 0,`Matched leaf route at location "${m.pathname}${m.search}${m.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let y=pT(x&&x.map($=>Object.assign({},$,{params:Object.assign({},i,$.params),pathname:Ta([l,n.encodeLocation?n.encodeLocation($.pathname.replace(/%/g,"%25").replace(/\?/g,"%3F").replace(/#/g,"%23")).pathname:$.pathname]),pathnameBase:$.pathnameBase==="/"?l:Ta([l,n.encodeLocation?n.encodeLocation($.pathnameBase.replace(/%/g,"%25").replace(/\?/g,"%3F").replace(/#/g,"%23")).pathname:$.pathnameBase])})),r,a);return t&&y?W.createElement(ai.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",mask:void 0,...m},navigationType:"POP"}},y):y}function cT(){let e=M0(),t=g0(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return Sp&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=W.createElement(W.Fragment,null,W.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),W.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",W.createElement("code",{style:s},"ErrorBoundary")," or"," ",W.createElement("code",{style:s},"errorElement")," prop on your route."))),W.createElement(W.Fragment,null,W.createElement("h2",null,"Unexpected Application Error!"),W.createElement("h3",{style:{fontStyle:"italic"}},t),a?W.createElement("pre",{style:r},a):null,i)}var dT=W.createElement(cT,null),A0=class extends W.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.onError?this.props.onError(e,t):console.error("React Router caught the following error during render",e)}render(){let e=this.state.error;if(this.context&&typeof e=="object"&&e&&"digest"in e&&typeof e.digest=="string"){let a=lT(e.digest);a&&(e=a)}let t=e!==void 0?W.createElement(ra.Provider,{value:this.props.routeContext},W.createElement(Ep.Provider,{value:e,children:this.props.component})):this.props.children;return this.context?W.createElement(mT,{error:e},t):t}};A0.contextType=$0;var xp=new WeakMap;function mT({children:e,error:t}){let{basename:a}=W.useContext(_t);if(typeof t=="object"&&t&&"digest"in t&&typeof t.digest=="string"){let n=oT(t.digest);if(n){let r=xp.get(t);if(r)throw r;let s=b0(n.location,a);if(y0&&!xp.get(t))if(s.isExternal||n.reloadDocument)window.location.href=s.absoluteURL||s.to;else{let i=Promise.resolve().then(()=>window.__reactRouterDataRouter.navigate(s.to,{replace:n.replace}));throw xp.set(t,i),i}return W.createElement("meta",{httpEquiv:"refresh",content:`0;url=${s.absoluteURL||s.to}`})}}return e}function fT({routeContext:e,match:t,children:a}){let n=W.useContext(qr);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),W.createElement(ra.Provider,{value:e},a)}function pT(e,t=[],a){let n=a?.state;if(e==null){if(!n)return null;if(n.errors)e=n.matches;else if(t.length===0&&!n.initialized&&n.matches.length>0)e=n.matches;else return null}let r=e,s=n?.errors;if(s!=null){let d=r.findIndex(m=>m.route.id&&s?.[m.route.id]!==void 0);Te(d>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(s).join(",")}`),r=r.slice(0,Math.min(r.length,d+1))}let i=!1,o=-1;if(a&&n){i=n.renderFallback;for(let d=0;d<r.length;d++){let m=r[d];if((m.route.HydrateFallback||m.route.hydrateFallbackElement)&&(o=d),m.route.id){let{loaderData:f,errors:h}=n,x=m.route.loader&&!f.hasOwnProperty(m.route.id)&&(!h||h[m.route.id]===void 0);if(m.route.lazy||x){a.isStatic&&(i=!0),o>=0?r=r.slice(0,o+1):r=[r[0]];break}}}}let l=a?.onError,c=n&&l?(d,m)=>{l(d,{location:n.location,params:n.matches?.[0]?.params??{},pattern:X3(n.matches),errorInfo:m})}:void 0;return r.reduceRight((d,m,f)=>{let h,x=!1,y=null,$=null;n&&(h=s&&m.route.id?s[m.route.id]:void 0,y=m.route.errorElement||dT,i&&(o<0&&f===0?(O0("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),x=!0,$=null):o===f&&(x=!0,$=m.route.hydrateFallbackElement||null)));let g=t.concat(r.slice(0,f+1)),v=()=>{let b;return h?b=y:x?b=$:m.route.Component?b=W.createElement(m.route.Component,null):m.route.element?b=m.route.element:b=d,W.createElement(fT,{match:m,routeContext:{outlet:d,matches:g,isDataRoute:n!=null},children:b})};return n&&(m.route.ErrorBoundary||m.route.errorElement||f===0)?W.createElement(A0,{location:n.location,revalidation:n.revalidation,component:y,error:h,children:v(),routeContext:{outlet:null,matches:g,isDataRoute:!0},onError:c}):v()},null)}function Tp(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function hT(e){let t=W.useContext(qr);return Te(t,Tp(e)),t}function Ap(e){let t=W.useContext(ti);return Te(t,Tp(e)),t}function vT(e){let t=W.useContext(ra);return Te(t,Tp(e)),t}function Dp(e){let t=vT(e),a=t.matches[t.matches.length-1];return Te(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function gT(){return Dp("useRouteId")}function D0(){let e=Ap("useNavigation");return W.useMemo(()=>{let{matches:t,historyAction:a,...n}=e.navigation;return n},[e.navigation])}function Mp(){let{matches:e,loaderData:t}=Ap("useMatches");return W.useMemo(()=>e.map(a=>D3(a,t)),[e,t])}function M0(){let e=W.useContext(Ep),t=Ap("useRouteError"),a=Dp("useRouteError");return e!==void 0?e:t.errors?.[a]}function yT(){let{router:e}=hT("useNavigate"),t=Dp("useNavigate"),a=W.useRef(!1);return R0(()=>{a.current=!0}),W.useCallback(async(r,s={})=>{na(a.current,_0),a.current&&(typeof r=="number"?await e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var l0={};function O0(e,t,a){!t&&!l0[e]&&(l0[e]=!0,na(!1,a))}var bT="useOptimistic",DP=ke[bT];var MP=ke.memo(xT);function xT({routes:e,manifest:t,future:a,state:n,isStatic:r,onError:s}){return T0(e,void 0,{manifest:t,state:n,isStatic:r,onError:s,future:a})}function ot({to:e,replace:t,state:a,relative:n}){Te(Ir(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=ke.useContext(_t);na(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=ke.useContext(ra),{pathname:i}=Ae(),o=ve(),l=bc(e,kp(s),i,n==="path"),c=JSON.stringify(l);return ke.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function Op(e){return C0(e.context)}function xe(e){Te(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function Lp({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1,useTransitions:i}){Te(!Ir(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let o=e.replace(/^\/*/,"/"),l=ke.useMemo(()=>({basename:o,navigator:r,static:s,useTransitions:i,future:{}}),[o,r,s,i]);typeof a=="string"&&(a=zr(a));let{pathname:c="/",search:d="",hash:m="",state:f=null,key:h="default",mask:x}=a,y=ke.useMemo(()=>{let $=Wa(c,o);return $==null?null:{location:{pathname:$,search:d,hash:m,state:f,key:h,mask:x},navigationType:n}},[o,c,d,m,f,h,n,x]);return na(y!=null,`<Router basename="${o}"> is not able to match the URL "${c}${d}${m}" because it does not start with the basename, so the <Router> won't render anything.`),y==null?null:ke.createElement(_t.Provider,{value:l},ke.createElement(ai.Provider,{children:t,value:y}))}function Pp({children:e,location:t}){return E0(yc(e),t)}function yc(e,t=[]){let a=[];return ke.Children.forEach(e,(n,r)=>{if(!ke.isValidElement(n))return;let s=[...t,r];if(n.type===ke.Fragment){a.push.apply(a,yc(n.props.children,s));return}Te(n.type===xe,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),Te(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,middleware:n.props.middleware,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=yc(n.props.children,s)),a.push(i)}),a}var hc="get",vc="application/x-www-form-urlencoded";function xc(e){return typeof HTMLElement<"u"&&e instanceof HTMLElement}function $T(e){return xc(e)&&e.tagName.toLowerCase()==="button"}function wT(e){return xc(e)&&e.tagName.toLowerCase()==="form"}function ST(e){return xc(e)&&e.tagName.toLowerCase()==="input"}function NT(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function _T(e,t){return e.button===0&&(!t||t==="_self")&&!NT(e)}var fc=null;function RT(){if(fc===null)try{new FormData(document.createElement("form"),0),fc=!1}catch{fc=!0}return fc}var kT=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function $p(e){return e!=null&&!kT.has(e)?(na(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${vc}"`),null):e}function CT(e,t){let a,n,r,s,i;if(wT(e)){let o=e.getAttribute("action");n=o?Wa(o,t):null,a=e.getAttribute("method")||hc,r=$p(e.getAttribute("enctype"))||vc,s=new FormData(e)}else if($T(e)||ST(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let l=e.getAttribute("formaction")||o.getAttribute("action");if(n=l?Wa(l,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||hc,r=$p(e.getAttribute("formenctype"))||$p(o.getAttribute("enctype"))||vc,s=new FormData(o,e),!RT()){let{name:c,type:d,value:m}=e;if(d==="image"){let f=c?`${c}.`:"";s.append(`${f}x`,"0"),s.append(`${f}y`,"0")}else c&&s.append(c,m)}}else{if(xc(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=hc,n=null,r=vc,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var OP=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");var ET={"&":"\\u0026",">":"\\u003e","<":"\\u003c","\u2028":"\\u2028","\u2029":"\\u2029"},TT=/[&><\u2028\u2029]/g;function u0(e){return e.replace(TT,t=>ET[t])}function jp(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var AT=Symbol("SingleFetchRedirect");function L0(e,t,a,n){let r=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return a?r.pathname.endsWith("/")?r.pathname=`${r.pathname}_.${n}`:r.pathname=`${r.pathname}.${n}`:r.pathname==="/"?r.pathname=`_root.${n}`:t&&Wa(r.pathname,t)==="/"?r.pathname=`${gc(t)}/_root.${n}`:r.pathname=`${gc(r.pathname)}.${n}`,r}async function DT(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function MT(e){return e!=null&&typeof e.page=="string"}function OT(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function LT(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await DT(s,a);return i.links?i.links():[]}return[]}));return FT(n.flat(1).filter(OT).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function c0(e,t,a,n,r,s){let i=(l,c)=>a[c]?l.route.id!==a[c].route.id:!0,o=(l,c)=>a[c].pathname!==l.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==l.params["*"];return s==="assets"?t.filter((l,c)=>i(l,c)||o(l,c)):s==="data"?t.filter((l,c)=>{let d=n.routes[l.route.id];if(!d||!d.hasLoader)return!1;if(i(l,c)||o(l,c))return!0;if(l.route.shouldRevalidate){let m=l.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:l.params,defaultShouldRevalidate:!0});if(typeof m=="boolean")return m}return!0}):[]}function PT(e,t,{includeHydrateFallback:a}={}){return UT(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function UT(e){return[...new Set(e)]}function jT(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function FT(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!MT(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(jT(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function Fp(){let e=fe.useContext(qr);return jp(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function IT(){let e=fe.useContext(ti);return jp(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var Zo=fe.createContext(void 0);Zo.displayName="FrameworkContext";function Bp(){let e=fe.useContext(Zo);return jp(e,"You must render this element inside a <HydratedRouter> element"),e}function HT(e,t){let a=fe.useContext(Zo),[n,r]=fe.useState(!1),[s,i]=fe.useState(!1),{onFocus:o,onBlur:l,onMouseEnter:c,onMouseLeave:d,onTouchStart:m}=t,f=fe.useRef(null);fe.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let y=g=>{g.forEach(v=>{i(v.isIntersecting)})},$=new IntersectionObserver(y,{threshold:.5});return f.current&&$.observe(f.current),()=>{$.disconnect()}}},[e]),fe.useEffect(()=>{if(n){let y=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(y)}}},[n]);let h=()=>{r(!0)},x=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,f,{}]:[s,f,{onFocus:Xo(o,h),onBlur:Xo(l,x),onMouseEnter:Xo(c,h),onMouseLeave:Xo(d,x),onTouchStart:Xo(m,h)}]:[!1,f,{}]}function Xo(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function U0({page:e,...t}){let a=nT(),{router:n}=Fp(),r=fe.useMemo(()=>Rp(n.routes,e,n.basename),[n.routes,e,n.basename]);return r?a?fe.createElement(QT,{page:e,matches:r,...t}):fe.createElement(VT,{page:e,matches:r,...t}):null}function KT(e){let{manifest:t,routeModules:a}=Bp(),[n,r]=fe.useState([]);return fe.useEffect(()=>{let s=!1;return LT(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function QT({page:e,matches:t,...a}){let n=Ae(),{future:r}=Bp(),{basename:s}=Fp(),i=fe.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let o=L0(e,s,r.unstable_trailingSlashAwareDataRequests,"rsc"),l=!1,c=[];for(let d of t)typeof d.route.shouldRevalidate=="function"?l=!0:c.push(d.route.id);return l&&c.length>0&&o.searchParams.set("_routes",c.join(",")),[o.pathname+o.search]},[s,r.unstable_trailingSlashAwareDataRequests,e,n,t]);return fe.createElement(fe.Fragment,null,i.map(o=>fe.createElement("link",{key:o,rel:"prefetch",as:"fetch",href:o,...a})))}function VT({page:e,matches:t,...a}){let n=Ae(),{future:r,manifest:s,routeModules:i}=Bp(),{basename:o}=Fp(),{loaderData:l,matches:c}=IT(),d=fe.useMemo(()=>c0(e,t,c,s,n,"data"),[e,t,c,s,n]),m=fe.useMemo(()=>c0(e,t,c,s,n,"assets"),[e,t,c,s,n]),f=fe.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let y=new Set,$=!1;if(t.forEach(v=>{let b=s.routes[v.route.id];!b||!b.hasLoader||(!d.some(w=>w.route.id===v.route.id)&&v.route.id in l&&i[v.route.id]?.shouldRevalidate||b.hasClientLoader?$=!0:y.add(v.route.id))}),y.size===0)return[];let g=L0(e,o,r.unstable_trailingSlashAwareDataRequests,"data");return $&&y.size>0&&g.searchParams.set("_routes",t.filter(v=>y.has(v.route.id)).map(v=>v.route.id).join(",")),[g.pathname+g.search]},[o,r.unstable_trailingSlashAwareDataRequests,l,n,s,d,t,e,i]),h=fe.useMemo(()=>PT(m,s),[m,s]),x=KT(m);return fe.createElement(fe.Fragment,null,f.map(y=>fe.createElement("link",{key:y,rel:"prefetch",as:"fetch",href:y,...a})),h.map(y=>fe.createElement("link",{key:y,rel:"modulepreload",href:y,...a})),x.map(({key:y,link:$})=>fe.createElement("link",{key:y,nonce:a.nonce,...$,crossOrigin:$.crossOrigin??a.crossOrigin})))}function GT(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var YT=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{YT&&(window.__reactRouterVersion="7.15.1")}catch{}function zp({basename:e,children:t,useTransitions:a,window:n}){let r=te.useRef();r.current==null&&(r.current=d0({window:n,v5Compat:!0}));let s=r.current,[i,o]=te.useState({action:s.action,location:s.location}),l=te.useCallback(c=>{a===!1?o(c):te.startTransition(()=>o(c))},[a]);return te.useLayoutEffect(()=>s.listen(l),[s,l]),te.createElement(Lp,{basename:e,children:t,location:i.location,navigationType:i.action,navigator:s,useTransitions:a})}function j0({basename:e,children:t,history:a,useTransitions:n}){let[r,s]=te.useState({action:a.action,location:a.location}),i=te.useCallback(o=>{n===!1?s(o):te.startTransition(()=>s(o))},[n]);return te.useLayoutEffect(()=>a.listen(i),[a,i]),te.createElement(Lp,{basename:e,children:t,location:r.location,navigationType:r.action,navigator:a,useTransitions:n})}j0.displayName="unstable_HistoryRouter";var F0=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i,Rn=te.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,mask:o,state:l,target:c,to:d,preventScrollReset:m,viewTransition:f,defaultShouldRevalidate:h,...x},y){let{basename:$,navigator:g,useTransitions:v}=te.useContext(_t),b=typeof d=="string"&&F0.test(d),w=b0(d,$);d=w.to;let S=N0(d,{relative:r}),C=Ae(),R=null;if(o){let G=bc(o,[],C.mask?C.mask.pathname:"/",!0);$!=="/"&&(G.pathname=G.pathname==="/"?$:Ta([$,G.pathname])),R=g.createHref(G)}let[_,A,L]=HT(n,x),U=I0(d,{replace:i,mask:o,state:l,target:c,preventScrollReset:m,relative:r,viewTransition:f,defaultShouldRevalidate:h,useTransitions:v});function F(G){t&&t(G),G.defaultPrevented||U(G)}let B=!(w.isExternal||s),P=te.createElement("a",{...x,...L,href:(B?R:void 0)||w.absoluteURL||S,onClick:B?F:t,ref:GT(y,A),target:c,"data-discover":!b&&a==="render"?"true":void 0});return _&&!b?te.createElement(te.Fragment,null,P,te.createElement(U0,{page:S})):P});Rn.displayName="Link";var Za=te.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:l,...c},d){let m=ni(i,{relative:c.relative}),f=Ae(),h=te.useContext(ti),{navigator:x,basename:y}=te.useContext(_t),$=h!=null&&V0(m)&&o===!0,g=x.encodeLocation?x.encodeLocation(m).pathname:m.pathname,v=f.pathname,b=h&&h.navigation&&h.navigation.location?h.navigation.location.pathname:null;a||(v=v.toLowerCase(),b=b?b.toLowerCase():null,g=g.toLowerCase()),b&&y&&(b=Wa(b,y)||b);let w=g!=="/"&&g.endsWith("/")?g.length-1:g.length,S=v===g||!r&&v.startsWith(g)&&v.charAt(w)==="/",C=b!=null&&(b===g||!r&&b.startsWith(g)&&b.charAt(g.length)==="/"),R={isActive:S,isPending:C,isTransitioning:$},_=S?t:void 0,A;typeof n=="function"?A=n(R):A=[n,S?"active":null,C?"pending":null,$?"transitioning":null].filter(Boolean).join(" ");let L=typeof s=="function"?s(R):s;return te.createElement(Rn,{...c,"aria-current":_,className:A,ref:d,style:L,to:i,viewTransition:o},typeof l=="function"?l(R):l)});Za.displayName="NavLink";var B0=te.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=hc,action:o,onSubmit:l,relative:c,preventScrollReset:d,viewTransition:m,defaultShouldRevalidate:f,...h},x)=>{let{useTransitions:y}=te.useContext(_t),$=H0(),g=K0(o,{relative:c}),v=i.toLowerCase()==="get"?"get":"post",b=typeof o=="string"&&F0.test(o);return te.createElement("form",{ref:x,method:v,action:g,onSubmit:n?l:S=>{if(l&&l(S),S.defaultPrevented)return;S.preventDefault();let C=S.nativeEvent.submitter,R=C?.getAttribute("formmethod")||i,_=()=>$(C||S.currentTarget,{fetcherKey:t,method:R,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:m,defaultShouldRevalidate:f});y&&a!==!1?te.startTransition(()=>_()):_()},...h,"data-discover":!b&&e==="render"?"true":void 0})});B0.displayName="Form";function z0({getKey:e,storageKey:t,...a}){let n=te.useContext(Zo),{basename:r}=te.useContext(_t),s=Ae(),i=Mp();Q0({getKey:e,storageKey:t});let o=te.useMemo(()=>{if(!n||!e)return null;let c=_p(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let l=((c,d)=>{if(!window.history.state||!window.history.state.key){let m=Math.random().toString(32).slice(2);window.history.replaceState({key:m},"")}try{let f=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof f=="number"&&window.scrollTo(0,f)}catch(m){console.error(m),sessionStorage.removeItem(c)}}).toString();return te.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${l})(${u0(JSON.stringify(t||Np))}, ${u0(JSON.stringify(o))})`}})}z0.displayName="ScrollRestoration";function q0(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function qp(e){let t=te.useContext(qr);return Te(t,q0(e)),t}function JT(e){let t=te.useContext(ti);return Te(t,q0(e)),t}function I0(e,{target:t,replace:a,mask:n,state:r,preventScrollReset:s,relative:i,viewTransition:o,defaultShouldRevalidate:l,useTransitions:c}={}){let d=ve(),m=Ae(),f=ni(e,{relative:i});return te.useCallback(h=>{if(_T(h,t)){h.preventDefault();let x=a!==void 0?a:ei(m)===ei(f),y=()=>d(e,{replace:x,mask:n,state:r,preventScrollReset:s,relative:i,viewTransition:o,defaultShouldRevalidate:l});c?te.startTransition(()=>y()):y()}},[m,d,f,a,n,r,t,e,s,i,o,l,c])}var XT=0,WT=()=>`__${String(++XT)}__`;function H0(){let{router:e}=qp("useSubmit"),{basename:t}=te.useContext(_t),a=gT(),n=e.fetch,r=e.navigate;return te.useCallback(async(s,i={})=>{let{action:o,method:l,encType:c,formData:d,body:m}=CT(s,t);if(i.navigate===!1){let f=i.fetcherKey||WT();await n(f,a,i.action||o,{defaultShouldRevalidate:i.defaultShouldRevalidate,preventScrollReset:i.preventScrollReset,formData:d,body:m,formMethod:i.method||l,formEncType:i.encType||c,flushSync:i.flushSync})}else await r(i.action||o,{defaultShouldRevalidate:i.defaultShouldRevalidate,preventScrollReset:i.preventScrollReset,formData:d,body:m,formMethod:i.method||l,formEncType:i.encType||c,replace:i.replace,state:i.state,fromRouteId:a,flushSync:i.flushSync,viewTransition:i.viewTransition})},[n,r,t,a])}function K0(e,{relative:t}={}){let{basename:a}=te.useContext(_t),n=te.useContext(ra);Te(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...ni(e||".",{relative:t})},i=Ae();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),l=o.getAll("index");if(l.some(d=>d==="")){o.delete("index"),l.filter(m=>m).forEach(m=>o.append("index",m));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:Ta([a,s.pathname])),ei(s)}var Np="react-router-scroll-positions",pc={};function _p(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:Wa(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function Q0({getKey:e,storageKey:t}={}){let{router:a}=qp("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=JT("useScrollRestoration"),{basename:s}=te.useContext(_t),i=Ae(),o=Mp(),l=D0();te.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),ZT(te.useCallback(()=>{if(l.state==="idle"){let c=_p(i,o,s,e);pc[c]=window.scrollY}try{sessionStorage.setItem(t||Np,JSON.stringify(pc))}catch(c){na(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[l.state,e,s,i,o,t])),typeof document<"u"&&(te.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||Np);c&&(pc=JSON.parse(c))}catch{}},[t]),te.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(pc,()=>window.scrollY,e?(d,m)=>_p(d,m,s,e):void 0);return()=>c&&c()},[a,s,e]),te.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{na(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function ZT(e,t){let{capture:a}=t||{};te.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function V0(e,{relative:t}={}){let a=te.useContext(Cp);Te(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=qp("useViewTransitionState"),r=ni(e,{relative:t});if(!a.isTransitioning)return!1;let s=Wa(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=Wa(a.nextLocation.pathname,n)||a.nextLocation.pathname;return Wo(r.pathname,i)!=null||Wo(r.pathname,s)!=null}var Dt=new Fd({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var Ip="ironclaw_token",Ke="/api/webchat/v2",Hr=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function Sa(){return sessionStorage.getItem(Ip)||""}function ri(e){e?sessionStorage.setItem(Ip,e):sessionStorage.removeItem(Ip)}function $c(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function J0(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function Y0(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function X0({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=Y0(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=Y0(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function V(e,t={}){let a=Sa(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await J0(r);throw new Hr(X0({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function wc(){return V(`${Ke}/session`)}function Sc({clientActionId:e,requestedThreadId:t,projectId:a}={}){let n={client_action_id:e||$c()};return t&&(n.requested_thread_id=t),a&&(n.project_id=a),V(`${Ke}/threads`,{method:"POST",body:JSON.stringify(n)})}function Nc({limit:e,cursor:t,projectId:a}={}){let n=new URL(`${Ke}/threads`,window.location.origin);return e!=null&&n.searchParams.set("limit",String(e)),t&&n.searchParams.set("cursor",t),a&&n.searchParams.set("project_id",a),V(n.pathname+n.search)}function W0({threadId:e}={}){return e?V(`${Ke}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function Hp(e){return`${Ke}/threads/${encodeURIComponent(e)}/files`}function Z0({threadId:e,path:t}={}){if(!e)return Promise.reject(new Error("threadId is required"));let a=new URL(Hp(e),window.location.origin);return t&&a.searchParams.set("path",t),V(a.pathname+a.search)}function e$({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${Hp(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),V(a.pathname+a.search)}function _c({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${Hp(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function t$({limit:e,runLimit:t,includeCompleted:a}={}){let n=new URLSearchParams;e!=null&&n.set("limit",String(e)),t!=null&&n.set("run_limit",String(t)),a===!0&&n.set("include_completed","true");let r=n.toString();return V(`${Ke}/automations${r?`?${r}`:""}`)}function a$({automationId:e}={}){return e?V(`${Ke}/automations/${encodeURIComponent(e)}/pause`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function n$({automationId:e}={}){return e?V(`${Ke}/automations/${encodeURIComponent(e)}/resume`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function r$({automationId:e}={}){return e?V(`${Ke}/automations/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("automationId is required"))}var s$=`${Ke}/projects`;function eA(e){return`${s$}/${encodeURIComponent(e)}`}function i$({limit:e}={}){let t=new URL(s$,window.location.origin);return e!=null&&t.searchParams.set("limit",String(e)),V(t.pathname+t.search)}function o$({projectId:e}={}){return e?V(eA(e)):Promise.reject(new Error("projectId is required"))}function l$(){return V(`${Ke}/outbound/preferences`)}function u$(){return V(`${Ke}/outbound/targets`)}function c$({finalReplyTargetId:e}={}){return V(`${Ke}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function Kp({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:l,source:c,tail:d,follow:m}={}){let f=new URL(`${Ke}/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),l&&f.searchParams.set("tool_name",l),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),V(f.pathname+f.search)}function d$({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:l,source:c,tail:d,follow:m}={}){let f=new URL(`${Ke}/operator/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),l&&f.searchParams.set("tool_name",l),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),V(f.pathname+f.search)}function m$({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||$c(),content:t};return a.length>0&&(r.attachments=a),V(`${Ke}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function f$({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${Ke}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),V(n.pathname+n.search)}function p$({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${Ke}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function Aa(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new Hr("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=Sa(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await J0(r);throw new Hr(X0({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function Qp(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function Rc(e){return Qp(await Aa(e))}function h$({threadId:e,afterCursor:t}={}){let a=new URL(`${Ke}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=Sa();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function v$({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||$c()};return a&&(r.reason=a),V(`${Ke}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function Vp({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let l={client_action_id:i||$c(),resolution:n};return r!=null&&(l.always=r),s&&(l.credential_ref=s),V(`${Ke}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(l)})}function g$({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return V("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function y$(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),V(`${Ke}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function si(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function b$(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function x$(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new Hr("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new Hr("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function $$(){let e=Sa();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var kc="anon",w$=kc;function S$(e){w$=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:kc}function ft(){return w$}var N$="ironclaw:v2-thread-pins:",Gp=new Set,kn=new Set,Yp=null;function Jp(){return`${N$}${ft()}`}function tA(){try{let e=window.localStorage.getItem(Jp());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function aA(){try{kn.size===0?window.localStorage.removeItem(Jp()):window.localStorage.setItem(Jp(),JSON.stringify([...kn]))}catch{}}function _$(){let e=ft();if(e!==Yp){kn.clear();for(let t of tA())kn.add(t);Yp=e}}function R$(){return new Set(kn)}function k$(){let e=R$();for(let t of Gp)try{t(e)}catch{}}function C$(e){e&&(_$(),kn.has(e)?kn.delete(e):kn.add(e),aA(),k$())}function E$(){return _$(),R$()}function T$(e){return Gp.add(e),()=>{Gp.delete(e)}}function A$(){kn.clear(),Yp=ft();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(N$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}k$()}var nA=0,Kr={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function Xp(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function D$(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":rA(t)?"text":"download"}function rA(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function el(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function sA(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function iA(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function oA(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function M$(e,{limits:t,existing:a=[],t:n}){let r=t||Kr,s=[],i=[],o=a.length,l=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!sA(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:el(r.maxFileBytes)}));continue}if(l+c.size>r.maxTotalBytes){let y=n("chat.attachmentTotalTooLarge",{max:el(r.maxTotalBytes)});i.includes(y)||i.push(y);continue}let d;try{d=await iA(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:m,base64:f}=oA(d,c.type),h=m||"application/octet-stream",x=Xp(h);s.push({id:`staged-${nA++}`,filename:c.name||"attachment",mimeType:h,kind:x,sizeBytes:c.size,sizeLabel:el(c.size),dataBase64:f,previewUrl:x==="image"?d:null}),o+=1,l+=c.size}return{staged:s,errors:i}}function O$(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function L$(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}var Cc="__ironclaw_attachments_only_v1__";function lA(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||Xp(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?p$({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?el(n.size_bytes):"",preview_url:null,fetch_url:s}})}function U$(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let m=mA(s);if(!m)continue;let f=`tool-${m.invocationId}`;if(n.has(f))continue;n.add(f),r.push({id:f,role:"tool_activity",...m,timestamp:P$(s)||m.updatedAt||null,sequence:s.sequence,activityOrder:m.activityOrder,activityOrderSource:m.activityOrderSource,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=dA(s),l=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy"),c=lA(s,a),d=o==="user"&&c?.length>0&&s.content===Cc?"":s.content||"";r.push({id:i,role:o,content:d,attachments:c,timestamp:P$(s),kind:s.kind,status:l?"error":s.status,...l&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:cA(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=uA(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function uA(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function cA(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function dA(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function P$(e){return e.received_at||e.created_at||null}function mA(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:Wp(t)}var fA="gate_declined";function Wp(e){let t=e.status==="failed"||e.status==="killed",a=e.error_kind||null,n=B$(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:al(e.title||e.capability_id)||"tool",toolStatus:F$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(j$(a)||e.output_summary||e.output_preview||e.result_ref)||null,toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null,activityOrder:n,activityOrderSource:Number.isFinite(n)?"projection":null}}function Zp(e){let t=B$(e.activity_order),a=e.error_kind||null;return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:al(e.capability_id)||"tool",toolStatus:F$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:null,toolError:j$(a),toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null,activityOrder:t,activityOrderSource:Number.isFinite(t)?"projection":null}}function j$(e){return e||null}function tl(e){return e==="success"||e==="error"||e==="declined"}function al(e){let t=typeof e=="string"?e.trim():"";if(!t)return"";let a=t.split(".");return a[a.length-1]||t}function F$(e,t=null){if(t===fA)return"declined";switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}function B$(e){let t=Number(e);return Number.isFinite(t)?t:null}var pA=50,Da=new Map,hA=30;function nl(e,t){for(Da.delete(e),Da.set(e,t);Da.size>hA;){let a=Da.keys().next().value;Da.delete(a)}}function ii(e){return`${ft()}:${e}`}function q$(){Da.clear()}function I$(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?Da.get(ii(e)):null,[s,i]=p.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),[o,l]=p.default.useState(e);if(o!==e){let h=e?Da.get(ii(e)):null;l(e),i({messages:h?.messages||[],nextCursor:h?.nextCursor||null,isLoading:!!e&&!h,loadError:null})}let c=p.default.useRef(new Set),d=p.default.useRef(e);d.current=e;let m=p.default.useCallback(async(h,x={})=>{let{preserveClientOnly:y=!1,finalReplyTimestampByRun:$=null}=x;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(c.current.has(e))return;c.current.add(e);let g=ft(),v=ii(e);i(b=>({...b,isLoading:!0}));try{let b=await f$({threadId:e,limit:pA,cursor:h});if(ft()!==g)return;let w=h?[]:a?.()||[],S=U$(b.messages||[],w,e),C=b.next_cursor||null;if(h||n?.([]),!h){let R=Da.get(v)?.messages||[],_=z$(S,R,{preserveClientOnly:y,finalReplyTimestampByRun:$});nl(v,{messages:_,nextCursor:C})}i(R=>{if(d.current!==e)return R;let _;return h?_=vA(S,R.messages):_=z$(S,R.messages,{preserveClientOnly:y,finalReplyTimestampByRun:$}),nl(v,{messages:_,nextCursor:C}),{messages:_,nextCursor:C,isLoading:!1,loadError:null}})}catch(b){if(console.error("Failed to load timeline:",b),ft()!==g)return;i(w=>d.current===e?{...w,isLoading:!1,loadError:"Failed to load conversation history."}:w)}finally{c.current.delete(e)}},[e,a,n]);p.default.useEffect(()=>{let h=e?Da.get(ii(e)):null;i({messages:h?.messages||[],nextCursor:h?.nextCursor||null,isLoading:!!e&&!h,loadError:null}),e&&m()},[e,m]);let f=p.default.useCallback((h,x)=>{if(!h)return;let y=ii(h),$=b=>typeof x=="function"?x(b||[]):x;if(d.current===h){i(b=>{let w=$(b.messages||[]);return nl(y,{messages:w,nextCursor:b.nextCursor||null}),{...b,messages:w}});return}let g=Da.get(y)||{messages:[],nextCursor:null},v=$(g.messages||[]);nl(y,{messages:v,nextCursor:g.nextCursor||null})},[]);return{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:m,seedThreadMessages:f,setMessages:h=>i(x=>{let y=typeof h=="function"?h(x.messages):h;return e&&nl(ii(e),{messages:y,nextCursor:x.nextCursor}),{...x,messages:y}})}}function vA(e,t){let a=new Set(t.map(n=>n?.id).filter(Boolean));return[...e.filter(n=>!a.has(n?.id)),...t]}function z$(e,t,a={}){let{preserveClientOnly:n=!1,finalReplyTimestampByRun:r=null}=a,s=yA(e,t,{finalReplyTimestampByRun:r}),i=new Set(s.map(l=>l?.id).filter(Boolean)),o=t.filter(l=>!l||typeof l.id!="string"||i.has(l.id)?!1:H$(l)?!0:typeof l.timelineMessageId=="string"&&i.has(`msg-${l.timelineMessageId}`)?!1:gA(l)?!0:n&&l.id.startsWith("err-"));return o.length>0?bA(s,o,t):s}function gA(e){return e?.isOptimistic===!0&&typeof e.id=="string"&&e.id.startsWith("pending-")&&(e.role==="user"||e.role==="assistant")}function yA(e,t,a={}){let{finalReplyTimestampByRun:n=null}=a,r=new Map,s=new Map;for(let i of t||[])!i||!i.timestamp||(typeof i.id=="string"&&r.set(i.id,i),typeof i.timelineMessageId=="string"&&r.set(`msg-${i.timelineMessageId}`,i),eh(i)&&typeof i.turnRunId=="string"&&s.set(i.turnRunId,i));return r.size===0&&s.size===0&&!n?e:e.map(i=>{if(!i||i.timestamp||typeof i.id!="string")return i;let o=typeof i.turnRunId=="string"?i.turnRunId:null,l=r.get(i.id)||(eh(i)&&o?s.get(o):null),c=eh(i)&&o?n?.[o]:null,d=l?.timestamp||c;return d?{...i,timestamp:d}:i})}function eh(e){return e?.role==="assistant"&&e?.isFinalReply===!0}function H$(e){return e?.role==="tool_activity"||e?.role==="thinking"}function bA(e,t,a){let n=new Map;for(let[l,c]of e.entries())typeof c?.id=="string"&&n.set(c.id,l);let r=a.map(l=>xA(l,n)),s=new Map,i=[];for(let l of t){if(!H$(l)){i.push(l);continue}let c=a.indexOf(l),d=null;for(let m=c-1;m>=0;m-=1)if(r[m]!==null){d=r[m];break}if(d!==null){let m=s.get(d)||[];m.push(l),s.set(d,m)}else i.push(l)}let o=[];for(let[l,c]of e.entries()){o.push(c);let d=s.get(l);d&&o.push(...d)}return o.push(...i),o}function xA(e,t){if(!e)return null;if(typeof e.id=="string"&&t.has(e.id))return t.get(e.id);if(typeof e.timelineMessageId=="string"){let a=`msg-${e.timelineMessageId}`;if(t.has(a))return t.get(a)}return null}var sl="__new__",K$="ironclaw:v2-draft:";function oi(e){return`${K$}${ft()}:${e||sl}`}function th(e){try{return window.localStorage.getItem(oi(e))||""}catch{return""}}function ah(e,t){try{t?window.localStorage.setItem(oi(e),t):window.localStorage.removeItem(oi(e))}catch{}}function Q$(e){ah(e,"")}var rl=new Map;function nh(e){return rl.get(oi(e))||[]}function Ec(e,t){let a=oi(e);t&&t.length>0?rl.set(a,t):rl.delete(a)}function V$(e){rl.delete(oi(e))}function G$(){rl.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(K$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function $A(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function wA(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function SA(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=$A(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?wA(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),Sa()?"":(ri(n),n)}function NA(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var _A={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function RA(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),_A[t]||"Could not complete sign-in. Please try again."):""}function Y$(){let[e,t]=p.default.useState(()=>SA()||Sa()),[a,n]=p.default.useState(()=>RA()),[r]=p.default.useState(()=>NA()),[s,i]=p.default.useState(null),[o,l]=p.default.useState(()=>!!(r&&!Sa())),[c,d]=p.default.useState(()=>!!Sa());p.default.useEffect(()=>{if(!r||Sa()){l(!1);return}let x=!1;return x$(r).then(y=>{x||(ri(y),d(!0),t(y),i(null),n(""),l(!1),Dt.clear())}).catch(()=>{x||(n("Could not complete sign-in. Please try again."),l(!1))}),()=>{x=!0}},[r]),p.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let x=!1;return d(!0),wc().then(y=>{x||(i(y),d(!1))}).catch(y=>{x||(i(null),d(!1),(y?.status===401||y?.status===403)&&(ri(""),t(""),n("Your session expired. Please sign in again."),Dt.clear()))}),()=>{x=!0}},[e,o]),S$(s);let m=p.default.useRef(null);p.default.useEffect(()=>{let x=ft();m.current&&m.current!==kc&&m.current!==x&&(q$(),G$(),A$()),m.current=x},[s]);let f=p.default.useCallback(x=>{ri(x),d(!!x),t(x),i(null),n(""),Dt.clear()},[]),h=p.default.useCallback(()=>{$$().catch(()=>{}),ri(""),d(!1),t(""),i(null),n(""),Dt.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,rebornProjectsEnabled:!!s?.features?.reborn_projects,signIn:f,signOut:h}}var Qr="/chat",il=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace"},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"logs",path:"/logs",labelKey:"nav.logs",hidden:!0},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var kA=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"tools",labelKey:"settings.tools",icon:"tool"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],CA=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],EA=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],Tc={settings:kA,extensions:CA,admin:EA};var J$="ironclaw:v2-theme";function TA(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(J$);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function Ac(){let[e,t]=p.default.useState(TA);p.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(J$,e)}catch{}},[e]);let a=p.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function X$(e){return K({enabled:!!e,queryKey:["gateway-status",e],queryFn:si,refetchInterval:3e4})}var AA="/api/webchat/v2/operator/config",Dc="/api/webchat/v2/settings/tools",li="agent.auto_approve_tools",W$="tool.",DA=new Set(["always_allow","ask_each_time","disabled"]),MA=new Set(["default","always_allow","ask_each_time","disabled"]);function Z$(e){return e==="ask"?"ask_each_time":DA.has(e)?e:"ask_each_time"}function OA(e){return e==="ask"?"ask_each_time":MA.has(e)?e:"default"}function LA(e){return["default","global","override"].includes(e)?e:"default"}function ew(e){if(!e?.key?.startsWith(W$))return null;let t=e.value||{};return{name:t.name||e.key.slice(W$.length),description:t.description||"",state:Z$(t.state),default_state:Z$(t.default_state),locked:!!(t.locked||e.mutable===!1),effective_source:LA(t.effective_source||e.source)}}function PA(e){let t={};for(let a of e.entries||[])a?.key===li&&(t[li]=!!a.value);return t}async function tw(){let e=await V(Dc);return{settings:PA(e),diagnostics:e.diagnostics||[],precedence:e.precedence||[]}}async function rh(e,t){if(e===li){let n=await V(Dc,{method:"POST",body:JSON.stringify({enabled:!!t})});return{success:!0,entry:n.entry,value:n.entry?.value}}let a=await V(`${AA}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({value:t})});return{success:!0,entry:a.entry,value:a.entry?.value}}async function aw(e){let t=e?.settings||{},a=[];return Object.prototype.hasOwnProperty.call(t,li)&&a.push(await rh(li,!!t[li])),{success:!0,imported:a.length,results:a}}function Mc(){return V("/api/webchat/v2/llm/providers")}function nw(e){return V("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function rw(e){return V(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function ol(e){return V("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function sw(e){return V("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function iw(e){return V("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function ow(e){return V("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function lw(e){return V("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function uw(){return V("/api/webchat/v2/llm/codex/login",{method:"POST"})}async function cw(){let e=await V(Dc);return{tools:(e.entries||[]).map(ew).filter(Boolean),diagnostics:e.diagnostics||[]}}async function dw(e,t){let a=OA(t),n=await V(`${Dc}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({state:a})});return{success:!0,tool:ew(n.entry),entry:n.entry}}function mw(){return V("/api/webchat/v2/extensions")}function fw(){return V("/api/webchat/v2/extensions/registry")}function pw(){return V("/api/webchat/v2/skills")}function hw(e){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function vw(e){return V("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function gw(e,t){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function yw(e){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function bw(e,t){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}/auto-activate`,{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:t})})}function xw(e){return V("/api/webchat/v2/skills/auto-activate-learned",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function $w(){return V("/api/webchat/v2/traces/credit")}function ww(e){return V(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function Sw(){return Promise.resolve({users:[],todo:!0})}function Nw(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function _w(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var sh="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",ih=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function ll(e){return ih.find(t=>t.value===e)?.label||e}function ui(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function Rw(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function Oc(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function kw(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function Vr(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===sh||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?ui(e,t).trim().length>0:!0:!1}function UA(e,t,a){return e.id===a?"active":Vr(e,t)?"ready":"setup"}function Cw(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=UA(r,t,a);n[s]&&n[s].push(r)}return n}function Lc(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===sh||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!ui(e,t).trim()?"base_url":"ok"}function oh(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===sh&&(i.api_key=void 0),i}function Ew(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function Tw(e){return/^[a-z0-9_-]+$/.test(e)}function Aw(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var jA=Object.freeze({});function ci({settings:e,gatewayStatus:t,enabled:a=!0}){let n=Z(),r=K({queryKey:["llm-providers"],queryFn:Mc,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=jA,l=(s.providers||[]).map(w=>({...w,name:w.description,has_api_key:w.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,m=d||"nearai",f=s.active?.model||t?.llm_model||"",h=l.filter(w=>w.builtin),x=l.filter(w=>!w.builtin),y=[...l].sort((w,S)=>w.id===d?-1:S.id===d?1:(w.name||w.id).localeCompare(S.name||S.id)),$=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=Y({mutationFn:async w=>{if(!Vr(w,o)){let C=Lc(w,o);throw new Error(C==="base_url"?"base_url":"api_key")}let S=Oc(w,o);if(!S)throw new Error("model");return await ol({provider_id:w.id,model:S}),w},onSuccess:$}),v=Y({mutationFn:async({provider:w,form:S,apiKey:C,editingProvider:R})=>{let _=!!w?.builtin,L={id:(_?w.id:S.id.trim()).trim(),name:_?w.name||w.id:S.name.trim(),adapter:_?w.adapter:S.adapter,base_url:S.baseUrl.trim()||w?.base_url||"",default_model:S.model.trim()||void 0};return C.trim()&&(L.api_key=C.trim()),(R||w)?.id===m&&L.default_model&&(L.set_active=!0,L.model=L.default_model),await nw(L),L},onSuccess:$}),b=Y({mutationFn:async w=>(await rw(w.id),w),onSuccess:$});return{providers:y,builtinProviders:h,customProviders:x,builtinOverrides:o,activeProviderId:d,selectedModel:f,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:w=>g.mutateAsync(w),saveCustomProvider:w=>v.mutateAsync(w),saveBuiltinProvider:w=>v.mutateAsync(w),deleteCustomProvider:w=>b.mutateAsync(w),testConnection:sw,listModels:iw,isBusy:g.isPending||v.isPending||b.isPending}}function Dw({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}var Mw="ironclaw:v2-sidebar-open";function Ow(){return typeof window>"u"?null:window}function Lw(){try{return Ow()?.localStorage||null}catch{return null}}function Pw(e=Lw()){try{return e?.getItem(Mw)!=="false"}catch{return!0}}function Uw(e,t=Lw()){try{t?.setItem(Mw,e?"true":"false")}catch{}}function jw(e=Ow()){try{return e?.matchMedia?.("(min-width: 768px)").matches===!0}catch{return!1}}function Fw(e,t){return t?{...e,desktopOpen:!e.desktopOpen}:{...e,mobileOpen:!e.mobileOpen}}function Bw(e,t){return t?e.desktopOpen:e.mobileOpen}function zw({onNewChat:e}={}){let t=ve(),[a,n]=p.default.useState(()=>({mobileOpen:!1,desktopOpen:Pw()})),[r,s]=p.default.useState(()=>jw());p.default.useEffect(()=>{let d=window.matchMedia("(min-width: 768px)"),m=()=>s(d.matches);return m(),d.addEventListener?.("change",m),()=>d.removeEventListener?.("change",m)},[]),p.default.useEffect(()=>{Uw(a.desktopOpen)},[a.desktopOpen]);let i=p.default.useCallback(()=>{n(d=>({...d,mobileOpen:!1}))},[]),o=p.default.useCallback(()=>{n(d=>Fw(d,r))},[r]),l=p.default.useCallback(async()=>{let d=await e?.(),m=typeof d=="string"&&d.length>0?d:null;t(m?`/chat/${m}`:"/chat"),i()},[t,i,e]),c=p.default.useCallback(d=>{t(`/chat/${d}`),i()},[t,i]);return{mobileOpen:a.mobileOpen,desktopOpen:a.desktopOpen,currentOpen:Bw(a,r),close:i,toggle:o,newChat:l,selectThread:c}}var lh=new Set,FA=0;function di(e,t={}){let a={id:++FA,message:e,tone:t.tone||"info",duration:t.duration??2600};return lh.forEach(n=>n(a)),a.id}function qw(e){return lh.add(e),()=>lh.delete(e)}function BA(e){return e?.status===409&&e?.payload?.kind==="busy"}function Iw(e,t){return BA(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function Hw(){let e=K({queryKey:["threads"],queryFn:()=>Nc({})}),[t,a]=p.default.useState(null),[n,r]=p.default.useState(!1),s=p.default.useRef(new Map),i=p.default.useCallback(async c=>{let d=c||"__global__",m=s.current.get(d);if(m)return m;r(!0);let f=(async()=>{try{let h=await Sc(c?{projectId:c}:void 0);Dt.invalidateQueries({queryKey:["threads"]});let x=h?.thread?.thread_id;return x&&a(x),x}finally{s.current.delete(d),r(s.current.size>0)}})();return s.current.set(d,f),f},[]),o=p.default.useCallback(async c=>{await W0({threadId:c}),t===c&&a(null),Dt.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:p.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var Kw={attach:u`<path
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
    />`,arrowDown:u`<path d="M12 5v14" /><path d="m6 13 6 6 6-6" />`,retry:u`<path d="M3.5 12a8.5 8.5 0 1 1 2.6 6.1" /><path d="M3.2 18.5v-5h5" />`};function D({name:e,className:t="",strokeWidth:a=1.7}){return u`
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
      ${Kw[e]||Kw.spark}
    </svg>
  `}function J(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=J(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function Qw(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function zA(e){return Qw(e).trim().charAt(0).toUpperCase()||"I"}function qA(){let[e,t]=p.default.useState(!1),a=p.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function Vw({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=k(),s=qA(),i=Qw(a),o=a?.email||a?.role||r("common.gatewaySession");return u`
    <div
      className="relative flex items-center gap-2 border-t border-[var(--v2-panel-border)] px-3 py-3"
    >
      ${s.open&&u`
        <div
          className=${J("absolute bottom-full left-3 right-3 mb-2 rounded-[10px] border p-3 shadow-lg","border-[var(--v2-panel-border)] bg-[var(--v2-surface)]")}
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
            />`:u`<span className="place-self-center">${zA(a)}</span>`}
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
        <${D} name=${e==="dark"?"sun":"moon"} className="h-4 w-4" />
      </button>
      <button
        onClick=${n}
        className="grid h-8 w-8 shrink-0 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
        title=${r("header.signOut")}
      >
        <${D} name="logout" className="h-4 w-4" />
      </button>
    </div>
  `}var Gw={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",logs:"list",settings:"settings",admin:"shield"},IA=il.filter(e=>e.id!=="chat"&&!e.hidden);function HA({route:e,label:t,onNavigate:a}){return u`
    <${Za}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>J("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${D} name=${Gw[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function KA({route:e,label:t,subRoutes:a,onNavigate:n}){let r=k(),s=Ae(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return u`
    <div className="flex flex-col">
      <${Za}
        to=${o}
        onClick=${n}
        className=${()=>J("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${D}
          name=${Gw[e.id]||"bolt"}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1 truncate">${t}</span>
        <${D}
          name="chevron"
          className=${J("h-3.5 w-3.5 shrink-0 transition-transform duration-150",i&&"rotate-180")}
        />
      <//>

      ${i&&u`
        <div className="mt-0.5 flex flex-col gap-0.5 pl-3">
          ${a.map(l=>u`
              <${Za}
                key=${l.id}
                to=${e.path+"/"+l.id}
                onClick=${n}
                className=${({isActive:c})=>J("flex items-center gap-2.5 rounded-[8px] py-1.5 pl-7 pr-3 text-[12px] font-medium",c?"text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
              >
                <${D} name=${l.icon} className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate">${r(l.labelKey)}</span>
              <//>
            `)}
        </div>
      `}
    </div>
  `}function Yw({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=k(),s=p.default.useMemo(()=>IA.filter(i=>a||i.id!=="admin"),[a]);return u`
    <div className="flex flex-col px-3 py-2">
      <button
        data-testid="new-chat"
        onClick=${e}
        disabled=${t}
        className=${J("flex items-center gap-2.5 rounded-[10px] px-3 py-2","border border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]","bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]","hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)] disabled:opacity-50")}
      >
        <${D} name="plus" className="h-4 w-4 shrink-0" />
        <span className="text-[13px] font-medium">
          ${r(t?"chat.creating":"chat.newThread")}
        </span>
      </button>

      <nav className="mt-2 flex flex-col gap-1">
        ${s.map(i=>{let o=(Tc[i.id]||[]).filter(l=>a||!(i.id==="settings"&&["users","inference"].includes(l.id)));return o.length>0?u`
              <${KA}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:u`
            <${HA}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var Na=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),ul=new Set([Na.NEEDS_ATTENTION,Na.FAILED]),uh="ironclaw:v2-thread-attention",ch=new Set,mi=new Map;function QA(){try{let e=window.localStorage.getItem(uh);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&ul.has(a[1])):[]}catch{return[]}}function Jw(){let e=[];for(let[t,a]of mi)ul.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(uh):window.localStorage.setItem(uh,JSON.stringify(e))}catch{}}for(let[e,t]of QA())mi.set(e,t);function Ww(){return new Map(mi)}function Xw(){let e=Ww();for(let t of ch)try{t(e)}catch{}}function Pc(e,t){if(!e)return;let a=mi.get(e);if(t==null){if(!mi.delete(e))return;ul.has(a)&&Jw(),Xw();return}a!==t&&(mi.set(e,t),(ul.has(t)||ul.has(a))&&Jw(),Xw())}function Zw(e){Pc(e,null)}function VA(){return Ww()}function GA(e){return ch.add(e),()=>{ch.delete(e)}}function e1(){let[e,t]=p.default.useState(VA);return p.default.useEffect(()=>GA(t),[]),e}function Uc(e){return e.updated_at||e.created_at||null}function dh(e,t){let a=Uc(e)||"",n=Uc(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function t1(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function a1(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function YA(){let[e,t]=p.default.useState(E$);return p.default.useEffect(()=>T$(t),[]),e}var JA=Object.freeze({[Na.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-transparent"},[Na.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[Na.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function XA(e){return e&&JA[e]||null}function WA(e){let t=String(e?.state||"").toLowerCase();return t==="processing"||t==="running"?Na.RUNNING:t==="needs_attention"||t==="awaitingapproval"||t==="awaiting_approval"?Na.NEEDS_ATTENTION:t==="failed"||t==="interrupted"?Na.FAILED:null}function ZA({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=k(),o=Uc(e),l=t1(o),c=a1(o),d=p.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(h=>{window.alert(h?.message||"Unable to delete chat")})},[s,e.id]),m=p.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),C$(e.id)},[e.id]);return u`
    <div
      className=${J("group flex w-full items-stretch rounded-[8px] border-l-2",n?n.borderClass:t?"border-[var(--v2-accent)]":"border-transparent",t?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
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
            className=${J("h-1.5 w-1.5 shrink-0 rounded-full",n.dotClass)}
          />`}
        </div>
        ${(n||l)&&u`<span
          className=${J("block truncate text-[11px]",n?n.textClass:"text-[var(--v2-text-faint)]")}
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
        className=${J("my-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] transition",a?"text-[var(--v2-accent-text)]":"opacity-0 text-[var(--v2-text-faint)] group-hover:opacity-100 focus:opacity-100","hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-accent-text)]")}
      >
        <${D} name="pin" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>
      ${s&&u`<button
        type="button"
        onClick=${d}
        title=${i("common.deleteChat")}
        aria-label=${i("common.deleteChat")}
        className=${J("my-1 mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px]","opacity-0 transition group-hover:opacity-100 focus:opacity-100","text-[var(--v2-text-faint)] hover:bg-[var(--v2-danger-soft)] hover:text-[var(--v2-danger-text)]")}
      >
        <${D} name="trash" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>`}
    </div>
  `}function n1({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:u`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>u`
          <${ZA}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${XA(n.has(o.id)?n.get(o.id):WA(o))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function r1({threads:e,activeThreadId:t,rebornProjectsEnabled:a=!1,onSelect:n,onDelete:r,onNavigate:s}){let[i,o]=p.default.useState(!1),[l,c]=p.default.useState(""),d=e1(),m=YA(),f=k(),{pinned:h,recent:x,totalMatches:y}=p.default.useMemo(()=>{let $=l.trim().toLowerCase(),g=$?e.filter(w=>(w.title||w.id||"").toLowerCase().includes($)):e,v=[],b=[];for(let w of g)m.has(w.id)?v.push(w):b.push(w);return v.sort(dh),b.sort(dh),{pinned:v,recent:b,totalMatches:v.length+b.length}},[e,l,m]);return u`
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
        <${D}
          name="chevron"
          className=${J("h-3.5 w-3.5 text-[var(--v2-text-faint)]",i?"-rotate-90":"")}
          strokeWidth=${2.2}
        />
      </button>

      ${!i&&u`
        ${e.length>0&&u`<div className="relative mb-1 mt-1 px-1">
          <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--v2-text-faint)]">
            <${D} name="search" className="h-3.5 w-3.5" />
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
          <${Za}
            to="/projects"
            onClick=${s}
            className=${({isActive:$})=>J("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",$?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
          >
            <${D} name="folder" className="h-4 w-4 shrink-0" />
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

          <${n1}
            label=${f("common.pinned")}
            items=${h}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${m}
            onSelect=${n}
            onDelete=${r}
          />
          <${n1}
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
  `}function jc(){let e=Z(),t=K({queryKey:["trace-credits"],queryFn:$w,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=Y({mutationFn:ww,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function e4(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function s1(){let e=k(),{credits:t}=jc();if(!t||!t.enrolled)return null;let a=e4(t.final_credit),n=t.submissions_accepted||0,r=t.submissions_submitted||0,s=t.manual_review_hold_count||0;return u`
    <div className="px-3 pb-1">
      <${Rn}
        to="/settings/traces"
        className="block rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2.5 transition-colors hover:border-[var(--v2-accent-soft)] hover:bg-[var(--v2-surface-muted)]"
      >
        <div className="flex items-center gap-2 text-[var(--v2-accent-text)]">
          <${D} name="layers" className="h-3.5 w-3.5 shrink-0" />
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
  `}function i1({id:e,threadsState:t,theme:a,toggleTheme:n,profile:r,isAdmin:s,rebornProjectsEnabled:i=!1,onSignOut:o,onClose:l,onNewChat:c,onSelectThread:d,onDeleteThread:m}){return u`
    <aside
      id=${e}
      className="flex h-full w-[260px] shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
    >
      <div className="flex items-center gap-2.5 px-4 py-5">
        <${Rn}
          to="/chat"
          onClick=${l}
          className="flex items-center gap-2.5 opacity-90 hover:opacity-100"
          aria-label="IronClaw"
        >
          <img src="/v2/assets/logo.jpg" alt="IronClaw" className="h-7 w-auto" />
        <//>
      </div>

      <${Yw}
        onNewChat=${c}
        isCreating=${t.isCreating}
        isAdmin=${s}
        onNavigate=${l}
      />

      <${s1} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${r1}
          threads=${t.threads}
          activeThreadId=${t.activeThreadId}
          rebornProjectsEnabled=${i}
          onSelect=${d}
          onDelete=${m}
          onNavigate=${l}
        />
      </div>

      <${Vw}
        theme=${a}
        toggleTheme=${n}
        profile=${r}
        onSignOut=${o}
      />
    </aside>
  `}var t4="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",a4="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",o1="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",l1={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},u1={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function T({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=l1[n]??l1.md,l=r?"w-full":"";if(a==="primary")return u`
      <${s}
        style=${{background:t4,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${J(o1,o,l,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:a4}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=u1[a]??u1.outline;return u`
    <${s}
      className=${J(o1,o,l,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function c1(){let e=p.default.useMemo(()=>n4(window.location),[]),[t,a]=p.default.useState(null),[n,r]=p.default.useState(null),[s,i]=p.default.useState(!1),[o,l]=p.default.useState(""),[c,d]=p.default.useState(!1);p.default.useEffect(()=>{if(!e)return;let h=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:h.signal}).then(x=>{if(!x.ok)throw new Error(String(x.status));return x.json()}).then(a).catch(()=>{h.signal.aborted||a(null)}),()=>h.abort()},[e]);let m=p.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),l("");try{let h=await fetch(`${e.base}/attestation/report`);if(!h.ok)throw new Error(String(h.status));let x=await h.json();return r(x),x}catch(h){return l(h.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),f=p.default.useCallback(async()=>{let h=n||await m();return!h||!navigator.clipboard?!1:(await navigator.clipboard.writeText(JSON.stringify({...h,instance_attestation:t},null,2)),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[m,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:m,copyReport:f}}function n4(e){let t=e.hostname;if(!t||t==="localhost"||r4(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function r4(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}var s4=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function d1(){let e=k(),t=c1(),[a,n]=p.default.useState(!1),r=p.default.useCallback(()=>{n(o=>{let l=!o;return l&&t.loadReport(),l})},[t]),s=p.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=i4({teeInfo:t.teeInfo,report:t.report,t:e});return u`
    <div className="relative">
      <button
        type="button"
        onClick=${r}
        aria-expanded=${a}
        title=${e("tee.title")}
        className=${J("grid h-8 w-8 place-items-center rounded-[8px]","border border-[color-mix(in_srgb,var(--v2-positive-text)_28%,transparent)]","bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]","hover:border-[color-mix(in_srgb,var(--v2-positive-text)_52%,transparent)]")}
      >
        <${D} name="shield" className="h-4 w-4" />
      </button>

      ${a&&u`
        <div
          className=${J("absolute right-0 top-full z-40 mt-2 w-[min(22rem,calc(100vw-2rem))]","rounded-[14px] border border-[var(--v2-panel-border)]","bg-[var(--v2-surface)] p-3 shadow-[0_18px_48px_rgba(0,0,0,0.35)]")}
        >
          <div className="flex items-center gap-2">
            <span className="grid h-8 w-8 place-items-center rounded-[10px] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]">
              <${D} name="shield" className="h-4 w-4" />
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
            <${T}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${t.reportLoading}
              onClick=${s}
            >
              <${D} name="check" className="h-4 w-4" />
              ${t.copied?e("tee.copied"):e("tee.copyReport")}
            <//>
          </div>
        </div>
      `}
    </div>
  `}function i4({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return s4.map(([r,s])=>({label:a(s),value:o4(n[r])||a("common.unknown")}))}function o4(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var l4="https://docs.ironclaw.com";function m1({threadsState:e,onToggleSidebar:t,sidebarOpen:a=!0}){let n=k(),r=Ae(),s=p.default.useMemo(()=>{for(let o of il){let l=Tc[o.id];if(!l)continue;let c=o.path+"/";if(r.pathname.startsWith(c)){let d=r.pathname.slice(c.length).split("/")[0],m=l.find(f=>f.id===d);if(m)return{parent:n(o.labelKey),current:n(m.labelKey)}}}return null},[r.pathname,n]),i=p.default.useMemo(()=>{if(s)return null;if(r.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(c=>c.id===e.activeThreadId)?.title||n("nav.chat");let o=il.find(l=>r.pathname.startsWith(l.path));return o?n(o.labelKey):""},[r.pathname,e.activeThreadId,e.threads,n,s]);return u`
    <header
      className=${J("flex h-14 shrink-0 items-center gap-3 px-4","border-b border-[var(--v2-panel-border)]","bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] backdrop-blur-xl")}
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
        <${D} name="list" className="h-4 w-4" />
      </button>

      ${s?u`
            <div className="flex min-w-0 items-center gap-2 text-[14px] font-semibold">
              <span className="shrink-0 text-[var(--v2-text-muted)]">
                ${s.parent}
              </span>
              <${D}
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
        <${d1} />
        <${Za}
          to="/logs"
          className=${({isActive:o})=>J("inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",o&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
          title=${n("nav.logs")}
        >
          ${n("nav.logs")}
        <//>
        <a
          href=${l4}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${n("nav.docs")}
        >
          ${n("nav.docs")}
        </a>
      </div>
    </header>
  `}function f1({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=ve(),i=k(),[o,l]=p.default.useState(""),[c,d]=p.default.useState(0),m=p.default.useRef(null),f=p.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(b=>({id:`thread-${b.id}`,label:b.title||`Thread ${b.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${b.id}`)}));return[...g,...v]},[a,s,n,r]),h=p.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?f.filter(v=>v.label.toLowerCase().includes(g)):f},[f,o]);p.default.useEffect(()=>{if(!e)return;l(""),d(0);let g=window.requestAnimationFrame(()=>m.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),p.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,h.length-1)))},[h.length]);let x=p.default.useCallback(g=>{g&&(t(),g.run())},[t]),y=p.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,h.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),x(h[c])):g.key==="Escape"&&(g.preventDefault(),t())},[h,c,x,t]);if(!e)return null;let $=null;return u`
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh]" role="dialog" aria-modal="true" aria-label="Command palette">
      <button type="button" aria-label="Close" onClick=${t} className="absolute inset-0 bg-black/50"></button>
      <div className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_30px_60px_-20px_rgba(0,0,0,0.8)]">
        <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-3">
          <${D} name="search" className="h-4 w-4 text-[var(--v2-text-faint)]" />
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
                  <${D} name=${g.icon} className="h-4 w-4 shrink-0" />
                  <span className="min-w-0 truncate">${g.label}</span>
                </button>
              </li>
            `})}
        </ul>
      </div>
    </div>
  `}var p1={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},u4={info:"bolt",success:"check",error:"close"};function h1(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>qw(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:u`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${e.map(a=>u`
          <div
            key=${a.id}
            role="status"
            className=${["pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",p1[a.tone]||p1.info].join(" ")}
          >
            <${D} name=${u4[a.tone]||"bolt"} className="h-4 w-4 shrink-0" />
            <span>${a.message}</span>
          </div>
        `)}
    </div>
  `}function v1({token:e,profile:t,isChecking:a=!1,isAdmin:n,rebornProjectsEnabled:r=!1,onSignOut:s}){let i=k(),{theme:o,toggleTheme:l}=Ac(),c=X$(e),d=Hw(),m=zw({onNewChat:()=>d.setActiveThreadId(null)}),f=c.data,h=Ae(),x=ve(),y=ci({settings:{},gatewayStatus:f,enabled:n}),$=n&&Dw({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),g=h.pathname==="/welcome"||h.pathname.startsWith("/settings"),[v,b]=p.default.useState(!1);p.default.useEffect(()=>{let S=C=>{(C.metaKey||C.ctrlKey)&&C.key.toLowerCase()==="k"&&(C.preventDefault(),b(R=>!R))};return window.addEventListener("keydown",S),()=>window.removeEventListener("keydown",S)},[]);let w=p.default.useCallback(async S=>{let C=d.activeThreadId===S;try{await d.deleteThread(S),C&&x("/chat",{replace:!0})}catch(R){console.error("Failed to delete thread:",R),di(Iw(R,i),{tone:"error"})}},[x,d,i]);return $&&!g?u`<${ot} to="/welcome" replace />`:u`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${m.mobileOpen&&u`<button
        type="button"
        aria-label=${i("nav.close")}
        onClick=${m.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${J("fixed inset-y-0 left-0 z-50 md:relative md:z-auto",m.mobileOpen?"flex":"hidden",m.desktopOpen?"md:flex":"md:hidden")}
      >
        <${i1}
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
        <${m1}
          threadsState=${d}
          onToggleSidebar=${m.toggle}
          sidebarOpen=${m.currentOpen}
        />
        <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
          ${c.error&&u`
            <div
              className=${J("m-4 rounded-[14px] border px-4 py-3 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >
              ${c.error.message||i("error.gatewayConnection")}
            </div>
          `}
          <${Op}
            context=${{gatewayStatus:f,gatewayStatusQuery:c,currentUser:t,isChecking:a,isAdmin:n,threadsState:d}}
          />
        </main>
      </div>
      <${f1}
        open=${v}
        onClose=${()=>b(!1)}
        threadsState=${d}
        onNewChat=${m.newChat}
        onToggleTheme=${l}
      />
      <${h1} />
    </div>
  `}var Kt=qe(Qe(),1),pl=e=>e.type==="checkbox",Gr=e=>e instanceof Date,Mt=e=>e==null,E1=e=>typeof e=="object",Ye=e=>!Mt(e)&&!Array.isArray(e)&&E1(e)&&!Gr(e),c4=e=>Ye(e)&&e.target?pl(e.target)?e.target.checked:e.target.value:e,d4=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,m4=(e,t)=>e.has(d4(t)),f4=e=>{let t=e.constructor&&e.constructor.prototype;return Ye(t)&&t.hasOwnProperty("isPrototypeOf")},ph=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function pt(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(ph&&(e instanceof Blob||n))&&(a||Ye(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!f4(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=pt(e[r]));else return e;return t}var Ic=e=>/^\w*$/.test(e),Ze=e=>e===void 0,hh=e=>Array.isArray(e)?e.filter(Boolean):[],vh=e=>hh(e.replace(/["|']|\]/g,"").split(/\.|\[/)),ee=(e,t,a)=>{if(!t||!Ye(e))return a;let n=(Ic(t)?[t]:vh(t)).reduce((r,s)=>Mt(r)?r:r[s],e);return Ze(n)||n===e?Ze(e[t])?a:e[t]:n},en=e=>typeof e=="boolean",Be=(e,t,a)=>{let n=-1,r=Ic(t)?[t]:vh(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],l=a;if(n!==i){let c=e[o];l=Ye(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=l,e=e[o]}},g1={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},Ma={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},Cn={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},p4=Kt.default.createContext(null);p4.displayName="HookFormContext";var h4=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==Ma.all&&(t._proxyFormState[i]=!n||Ma.all),a&&(a[i]=!0),e[i]}});return r},v4=typeof window<"u"?Kt.default.useLayoutEffect:Kt.default.useEffect;var tn=e=>typeof e=="string",g4=(e,t,a,n,r)=>tn(e)?(n&&t.watch.add(e),ee(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),ee(a,s))):(n&&(t.watchAll=!0),a),fh=e=>Mt(e)||!E1(e);function ur(e,t,a=new WeakSet){if(fh(e)||fh(t))return e===t;if(Gr(e)&&Gr(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(Gr(i)&&Gr(o)||Ye(i)&&Ye(o)||Array.isArray(i)&&Array.isArray(o)?!ur(i,o,a):i!==o)return!1}}return!0}var y4=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},ml=e=>Array.isArray(e)?e:[e],y1=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},Qt=e=>Ye(e)&&!Object.keys(e).length,gh=e=>e.type==="file",Oa=e=>typeof e=="function",Bc=e=>{if(!ph)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},T1=e=>e.type==="select-multiple",yh=e=>e.type==="radio",b4=e=>yh(e)||pl(e),mh=e=>Bc(e)&&e.isConnected;function x4(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=Ze(e)?n++:e[t[n++]];return e}function $4(e){for(let t in e)if(e.hasOwnProperty(t)&&!Ze(e[t]))return!1;return!0}function We(e,t){let a=Array.isArray(t)?t:Ic(t)?[t]:vh(t),n=a.length===1?e:x4(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Ye(n)&&Qt(n)||Array.isArray(n)&&$4(n))&&We(e,a.slice(0,-1)),e}var A1=e=>{for(let t in e)if(Oa(e[t]))return!0;return!1};function zc(e,t={}){let a=Array.isArray(e);if(Ye(e)||a)for(let n in e)Array.isArray(e[n])||Ye(e[n])&&!A1(e[n])?(t[n]=Array.isArray(e[n])?[]:{},zc(e[n],t[n])):Mt(e[n])||(t[n]=!0);return t}function D1(e,t,a){let n=Array.isArray(e);if(Ye(e)||n)for(let r in e)Array.isArray(e[r])||Ye(e[r])&&!A1(e[r])?Ze(t)||fh(a[r])?a[r]=Array.isArray(e[r])?zc(e[r],[]):{...zc(e[r])}:D1(e[r],Mt(t)?{}:t[r],a[r]):a[r]=!ur(e[r],t[r]);return a}var cl=(e,t)=>D1(e,t,zc(t)),b1={value:!1,isValid:!1},x1={value:!0,isValid:!0},M1=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!Ze(e[0].attributes.value)?Ze(e[0].value)||e[0].value===""?x1:{value:e[0].value,isValid:!0}:x1:b1}return b1},O1=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>Ze(e)?e:t?e===""?NaN:e&&+e:a&&tn(e)?new Date(e):n?n(e):e,$1={isValid:!1,value:null},L1=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,$1):$1;function w1(e){let t=e.ref;return gh(t)?t.files:yh(t)?L1(e.refs).value:T1(t)?[...t.selectedOptions].map(({value:a})=>a):pl(t)?M1(e.refs).value:O1(Ze(t.value)?e.ref.value:t.value,e)}var w4=(e,t,a,n)=>{let r={};for(let s of e){let i=ee(t,s);i&&Be(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},qc=e=>e instanceof RegExp,dl=e=>Ze(e)?e:qc(e)?e.source:Ye(e)?qc(e.value)?e.value.source:e.value:e,S1=e=>({isOnSubmit:!e||e===Ma.onSubmit,isOnBlur:e===Ma.onBlur,isOnChange:e===Ma.onChange,isOnAll:e===Ma.all,isOnTouch:e===Ma.onTouched}),N1="AsyncFunction",S4=e=>!!e&&!!e.validate&&!!(Oa(e.validate)&&e.validate.constructor.name===N1||Ye(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===N1)),N4=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),_1=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),fl=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=ee(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(fl(o,t))break}else if(Ye(o)&&fl(o,t))break}}};function R1(e,t,a){let n=ee(e,a);if(n||Ic(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=ee(t,s),o=ee(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var _4=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return Qt(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||Ma.all))},R4=(e,t,a)=>!e||!t||e===t||ml(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),k4=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,C4=(e,t)=>!hh(ee(e,t)).length&&We(e,t),E4=(e,t,a)=>{let n=ml(ee(e,a));return Be(n,"root",t[a]),Be(e,a,n),e},Fc=e=>tn(e);function k1(e,t,a="validate"){if(Fc(e)||Array.isArray(e)&&e.every(Fc)||en(e)&&!e)return{type:a,message:Fc(e)?e:"",ref:t}}var fi=e=>Ye(e)&&!qc(e)?e:{value:e,message:""},C1=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:l,maxLength:c,minLength:d,min:m,max:f,pattern:h,validate:x,name:y,valueAsNumber:$,mount:g}=e._f,v=ee(a,y);if(!g||t.has(y))return{};let b=o?o[0]:i,w=F=>{r&&b.reportValidity&&(b.setCustomValidity(en(F)?"":F||""),b.reportValidity())},S={},C=yh(i),R=pl(i),_=C||R,A=($||gh(i))&&Ze(i.value)&&Ze(v)||Bc(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,L=y4.bind(null,y,n,S),U=(F,B,P,G=Cn.maxLength,ae=Cn.minLength)=>{let le=F?B:P;S[y]={type:F?G:ae,message:le,ref:i,...L(F?G:ae,le)}};if(s?!Array.isArray(v)||!v.length:l&&(!_&&(A||Mt(v))||en(v)&&!v||R&&!M1(o).isValid||C&&!L1(o).isValid)){let{value:F,message:B}=Fc(l)?{value:!!l,message:l}:fi(l);if(F&&(S[y]={type:Cn.required,message:B,ref:b,...L(Cn.required,B)},!n))return w(B),S}if(!A&&(!Mt(m)||!Mt(f))){let F,B,P=fi(f),G=fi(m);if(!Mt(v)&&!isNaN(v)){let ae=i.valueAsNumber||v&&+v;Mt(P.value)||(F=ae>P.value),Mt(G.value)||(B=ae<G.value)}else{let ae=i.valueAsDate||new Date(v),le=Oe=>new Date(new Date().toDateString()+" "+Oe),lt=i.type=="time",ht=i.type=="week";tn(P.value)&&v&&(F=lt?le(v)>le(P.value):ht?v>P.value:ae>new Date(P.value)),tn(G.value)&&v&&(B=lt?le(v)<le(G.value):ht?v<G.value:ae<new Date(G.value))}if((F||B)&&(U(!!F,P.message,G.message,Cn.max,Cn.min),!n))return w(S[y].message),S}if((c||d)&&!A&&(tn(v)||s&&Array.isArray(v))){let F=fi(c),B=fi(d),P=!Mt(F.value)&&v.length>+F.value,G=!Mt(B.value)&&v.length<+B.value;if((P||G)&&(U(P,F.message,B.message),!n))return w(S[y].message),S}if(h&&!A&&tn(v)){let{value:F,message:B}=fi(h);if(qc(F)&&!v.match(F)&&(S[y]={type:Cn.pattern,message:B,ref:i,...L(Cn.pattern,B)},!n))return w(B),S}if(x){if(Oa(x)){let F=await x(v,a),B=k1(F,b);if(B&&(S[y]={...B,...L(Cn.validate,B.message)},!n))return w(B.message),S}else if(Ye(x)){let F={};for(let B in x){if(!Qt(F)&&!n)break;let P=k1(await x[B](v,a),b,B);P&&(F={...P,...L(B,P.message)},w(P.message),n&&(S[y]=F))}if(!Qt(F)&&(S[y]={ref:b,...F},!n))return S}}return w(!0),S},T4={mode:Ma.onSubmit,reValidateMode:Ma.onChange,shouldFocusError:!0};function A4(e={}){let t={...T4,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:Oa(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Ye(t.defaultValues)||Ye(t.values)?pt(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:pt(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},l,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},m={...d},f={array:y1(),state:y1()},h=t.criteriaMode===Ma.all,x=N=>E=>{clearTimeout(c),c=setTimeout(N,E)},y=async N=>{if(!t.disabled&&(d.isValid||m.isValid||N)){let E=t.resolver?Qt((await R()).errors):await A(n,!0);E!==a.isValid&&f.state.next({isValid:E})}},$=(N,E)=>{!t.disabled&&(d.isValidating||d.validatingFields||m.isValidating||m.validatingFields)&&((N||Array.from(o.mount)).forEach(M=>{M&&(E?Be(a.validatingFields,M,E):We(a.validatingFields,M))}),f.state.next({validatingFields:a.validatingFields,isValidating:!Qt(a.validatingFields)}))},g=(N,E=[],M,q,z=!0,O=!0)=>{if(q&&M&&!t.disabled){if(i.action=!0,O&&Array.isArray(ee(n,N))){let Q=M(ee(n,N),q.argA,q.argB);z&&Be(n,N,Q)}if(O&&Array.isArray(ee(a.errors,N))){let Q=M(ee(a.errors,N),q.argA,q.argB);z&&Be(a.errors,N,Q),C4(a.errors,N)}if((d.touchedFields||m.touchedFields)&&O&&Array.isArray(ee(a.touchedFields,N))){let Q=M(ee(a.touchedFields,N),q.argA,q.argB);z&&Be(a.touchedFields,N,Q)}(d.dirtyFields||m.dirtyFields)&&(a.dirtyFields=cl(r,s)),f.state.next({name:N,isDirty:U(N,E),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else Be(s,N,E)},v=(N,E)=>{Be(a.errors,N,E),f.state.next({errors:a.errors})},b=N=>{a.errors=N,f.state.next({errors:a.errors,isValid:!1})},w=(N,E,M,q)=>{let z=ee(n,N);if(z){let O=ee(s,N,Ze(M)?ee(r,N):M);Ze(O)||q&&q.defaultChecked||E?Be(s,N,E?O:w1(z._f)):P(N,O),i.mount&&y()}},S=(N,E,M,q,z)=>{let O=!1,Q=!1,ce={name:N};if(!t.disabled){if(!M||q){(d.isDirty||m.isDirty)&&(Q=a.isDirty,a.isDirty=ce.isDirty=U(),O=Q!==ce.isDirty);let ge=ur(ee(r,N),E);Q=!!ee(a.dirtyFields,N),ge?We(a.dirtyFields,N):Be(a.dirtyFields,N,!0),ce.dirtyFields=a.dirtyFields,O=O||(d.dirtyFields||m.dirtyFields)&&Q!==!ge}if(M){let ge=ee(a.touchedFields,N);ge||(Be(a.touchedFields,N,M),ce.touchedFields=a.touchedFields,O=O||(d.touchedFields||m.touchedFields)&&ge!==M)}O&&z&&f.state.next(ce)}return O?ce:{}},C=(N,E,M,q)=>{let z=ee(a.errors,N),O=(d.isValid||m.isValid)&&en(E)&&a.isValid!==E;if(t.delayError&&M?(l=x(()=>v(N,M)),l(t.delayError)):(clearTimeout(c),l=null,M?Be(a.errors,N,M):We(a.errors,N)),(M?!ur(z,M):z)||!Qt(q)||O){let Q={...q,...O&&en(E)?{isValid:E}:{},errors:a.errors,name:N};a={...a,...Q},f.state.next(Q)}},R=async N=>{$(N,!0);let E=await t.resolver(s,t.context,w4(N||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return $(N),E},_=async N=>{let{errors:E}=await R(N);if(N)for(let M of N){let q=ee(E,M);q?Be(a.errors,M,q):We(a.errors,M)}else a.errors=E;return E},A=async(N,E,M={valid:!0})=>{for(let q in N){let z=N[q];if(z){let{_f:O,...Q}=z;if(O){let ce=o.array.has(O.name),ge=z._f&&S4(z._f);ge&&d.validatingFields&&$([q],!0);let gt=await C1(z,o.disabled,s,h,t.shouldUseNativeValidation&&!E,ce);if(ge&&d.validatingFields&&$([q]),gt[O.name]&&(M.valid=!1,E))break;!E&&(ee(gt,O.name)?ce?E4(a.errors,gt,O.name):Be(a.errors,O.name,gt[O.name]):We(a.errors,O.name))}!Qt(Q)&&await A(Q,E,M)}}return M.valid},L=()=>{for(let N of o.unMount){let E=ee(n,N);E&&(E._f.refs?E._f.refs.every(M=>!mh(M)):!mh(E._f.ref))&&la(N)}o.unMount=new Set},U=(N,E)=>!t.disabled&&(N&&E&&Be(s,N,E),!ur(Oe(),r)),F=(N,E,M)=>g4(N,o,{...i.mount?s:Ze(E)?r:tn(N)?{[N]:E}:E},M,E),B=N=>hh(ee(i.mount?s:r,N,t.shouldUnregister?ee(r,N,[]):[])),P=(N,E,M={})=>{let q=ee(n,N),z=E;if(q){let O=q._f;O&&(!O.disabled&&Be(s,N,O1(E,O)),z=Bc(O.ref)&&Mt(E)?"":E,T1(O.ref)?[...O.ref.options].forEach(Q=>Q.selected=z.includes(Q.value)):O.refs?pl(O.ref)?O.refs.forEach(Q=>{(!Q.defaultChecked||!Q.disabled)&&(Array.isArray(z)?Q.checked=!!z.find(ce=>ce===Q.value):Q.checked=z===Q.value||!!z)}):O.refs.forEach(Q=>Q.checked=Q.value===z):gh(O.ref)?O.ref.value="":(O.ref.value=z,O.ref.type||f.state.next({name:N,values:pt(s)})))}(M.shouldDirty||M.shouldTouch)&&S(N,z,M.shouldTouch,M.shouldDirty,!0),M.shouldValidate&&ht(N)},G=(N,E,M)=>{for(let q in E){if(!E.hasOwnProperty(q))return;let z=E[q],O=N+"."+q,Q=ee(n,O);(o.array.has(N)||Ye(z)||Q&&!Q._f)&&!Gr(z)?G(O,z,M):P(O,z,M)}},ae=(N,E,M={})=>{let q=ee(n,N),z=o.array.has(N),O=pt(E);Be(s,N,O),z?(f.array.next({name:N,values:pt(s)}),(d.isDirty||d.dirtyFields||m.isDirty||m.dirtyFields)&&M.shouldDirty&&f.state.next({name:N,dirtyFields:cl(r,s),isDirty:U(N,O)})):q&&!q._f&&!Mt(O)?G(N,O,M):P(N,O,M),_1(N,o)&&f.state.next({...a,name:N}),f.state.next({name:i.mount?N:void 0,values:pt(s)})},le=async N=>{i.mount=!0;let E=N.target,M=E.name,q=!0,z=ee(n,M),O=ge=>{q=Number.isNaN(ge)||Gr(ge)&&isNaN(ge.getTime())||ur(ge,ee(s,M,ge))},Q=S1(t.mode),ce=S1(t.reValidateMode);if(z){let ge,gt,Ce=E.type?w1(z._f):c4(N),Ct=N.type===g1.BLUR||N.type===g1.FOCUS_OUT,on=!N4(z._f)&&!t.resolver&&!ee(a.errors,M)&&!z._f.deps||k4(Ct,ee(a.touchedFields,M),a.isSubmitted,ce,Q),ja=_1(M,o,Ct);Be(s,M,Ce),Ct?(!E||!E.readOnly)&&(z._f.onBlur&&z._f.onBlur(N),l&&l(0)):z._f.onChange&&z._f.onChange(N);let Fa=S(M,Ce,Ct),yr=!Qt(Fa)||ja;if(!Ct&&f.state.next({name:M,type:N.type,values:pt(s)}),on)return(d.isValid||m.isValid)&&(t.mode==="onBlur"?Ct&&y():Ct||y()),yr&&f.state.next({name:M,...ja?{}:Fa});if(!Ct&&ja&&f.state.next({...a}),t.resolver){let{errors:br}=await R([M]);if(O(Ce),q){let Zr=R1(a.errors,n,M),es=R1(br,n,Zr.name||M);ge=es.error,M=es.name,gt=Qt(br)}}else $([M],!0),ge=(await C1(z,o.disabled,s,h,t.shouldUseNativeValidation))[M],$([M]),O(Ce),q&&(ge?gt=!1:(d.isValid||m.isValid)&&(gt=await A(n,!0)));q&&(z._f.deps&&ht(z._f.deps),C(M,gt,ge,Fa))}},lt=(N,E)=>{if(ee(a.errors,E)&&N.focus)return N.focus(),1},ht=async(N,E={})=>{let M,q,z=ml(N);if(t.resolver){let O=await _(Ze(N)?N:z);M=Qt(O),q=N?!z.some(Q=>ee(O,Q)):M}else N?(q=(await Promise.all(z.map(async O=>{let Q=ee(n,O);return await A(Q&&Q._f?{[O]:Q}:Q)}))).every(Boolean),!(!q&&!a.isValid)&&y()):q=M=await A(n);return f.state.next({...!tn(N)||(d.isValid||m.isValid)&&M!==a.isValid?{}:{name:N},...t.resolver||!N?{isValid:M}:{},errors:a.errors}),E.shouldFocus&&!q&&fl(n,lt,N?z:o.mount),q},Oe=N=>{let E={...i.mount?s:r};return Ze(N)?E:tn(N)?ee(E,N):N.map(M=>ee(E,M))},De=(N,E)=>({invalid:!!ee((E||a).errors,N),isDirty:!!ee((E||a).dirtyFields,N),error:ee((E||a).errors,N),isValidating:!!ee(a.validatingFields,N),isTouched:!!ee((E||a).touchedFields,N)}),at=N=>{N&&ml(N).forEach(E=>We(a.errors,E)),f.state.next({errors:N?a.errors:{}})},$t=(N,E,M)=>{let q=(ee(n,N,{_f:{}})._f||{}).ref,z=ee(a.errors,N)||{},{ref:O,message:Q,type:ce,...ge}=z;Be(a.errors,N,{...ge,...E,ref:q}),f.state.next({name:N,errors:a.errors,isValid:!1}),M&&M.shouldFocus&&q&&q.focus&&q.focus()},Le=(N,E)=>Oa(N)?f.state.subscribe({next:M=>"values"in M&&N(F(void 0,E),M)}):F(N,E,!0),Pa=N=>f.state.subscribe({next:E=>{R4(N.name,E.name,N.exact)&&_4(E,N.formState||d,X,N.reRenderRoot)&&N.callback({values:{...s},...a,...E,defaultValues:r})}}).unsubscribe,kt=N=>(i.mount=!0,m={...m,...N.formState},Pa({...N,formState:m})),la=(N,E={})=>{for(let M of N?ml(N):o.mount)o.mount.delete(M),o.array.delete(M),E.keepValue||(We(n,M),We(s,M)),!E.keepError&&We(a.errors,M),!E.keepDirty&&We(a.dirtyFields,M),!E.keepTouched&&We(a.touchedFields,M),!E.keepIsValidating&&We(a.validatingFields,M),!t.shouldUnregister&&!E.keepDefaultValue&&We(r,M);f.state.next({values:pt(s)}),f.state.next({...a,...E.keepDirty?{isDirty:U()}:{}}),!E.keepIsValid&&y()},rn=({disabled:N,name:E})=>{(en(N)&&i.mount||N||o.disabled.has(E))&&(N?o.disabled.add(E):o.disabled.delete(E))},ua=(N,E={})=>{let M=ee(n,N),q=en(E.disabled)||en(t.disabled);return Be(n,N,{...M||{},_f:{...M&&M._f?M._f:{ref:{name:N}},name:N,mount:!0,...E}}),o.mount.add(N),M?rn({disabled:en(E.disabled)?E.disabled:t.disabled,name:N}):w(N,!0,E.value),{...q?{disabled:E.disabled||t.disabled}:{},...t.progressive?{required:!!E.required,min:dl(E.min),max:dl(E.max),minLength:dl(E.minLength),maxLength:dl(E.maxLength),pattern:dl(E.pattern)}:{},name:N,onChange:le,onBlur:le,ref:z=>{if(z){ua(N,E),M=ee(n,N);let O=Ze(z.value)&&z.querySelectorAll&&z.querySelectorAll("input,select,textarea")[0]||z,Q=b4(O),ce=M._f.refs||[];if(Q?ce.find(ge=>ge===O):O===M._f.ref)return;Be(n,N,{_f:{...M._f,...Q?{refs:[...ce.filter(mh),O,...Array.isArray(ee(r,N))?[{}]:[]],ref:{type:O.type,name:N}}:{ref:O}}}),w(N,!1,void 0,O)}else M=ee(n,N,{}),M._f&&(M._f.mount=!1),(t.shouldUnregister||E.shouldUnregister)&&!(m4(o.array,N)&&i.action)&&o.unMount.add(N)}}},Vt=()=>t.shouldFocusError&&fl(n,lt,o.mount),sn=N=>{en(N)&&(f.state.next({disabled:N}),fl(n,(E,M)=>{let q=ee(n,M);q&&(E.disabled=q._f.disabled||N,Array.isArray(q._f.refs)&&q._f.refs.forEach(z=>{z.disabled=q._f.disabled||N}))},0,!1))},vt=(N,E)=>async M=>{let q;M&&(M.preventDefault&&M.preventDefault(),M.persist&&M.persist());let z=pt(s);if(f.state.next({isSubmitting:!0}),t.resolver){let{errors:O,values:Q}=await R();a.errors=O,z=pt(Q)}else await A(n);if(o.disabled.size)for(let O of o.disabled)We(z,O);if(We(a.errors,"root"),Qt(a.errors)){f.state.next({errors:{}});try{await N(z,M)}catch(O){q=O}}else E&&await E({...a.errors},M),Vt(),setTimeout(Vt);if(f.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:Qt(a.errors)&&!q,submitCount:a.submitCount+1,errors:a.errors}),q)throw q},ca=(N,E={})=>{ee(n,N)&&(Ze(E.defaultValue)?ae(N,pt(ee(r,N))):(ae(N,E.defaultValue),Be(r,N,pt(E.defaultValue))),E.keepTouched||We(a.touchedFields,N),E.keepDirty||(We(a.dirtyFields,N),a.isDirty=E.defaultValue?U(N,pt(ee(r,N))):U()),E.keepError||(We(a.errors,N),d.isValid&&y()),f.state.next({...a}))},_a=(N,E={})=>{let M=N?pt(N):r,q=pt(M),z=Qt(N),O=z?r:q;if(E.keepDefaultValues||(r=M),!E.keepValues){if(E.keepDirtyValues){let Q=new Set([...o.mount,...Object.keys(cl(r,s))]);for(let ce of Array.from(Q))ee(a.dirtyFields,ce)?Be(O,ce,ee(s,ce)):ae(ce,ee(O,ce))}else{if(ph&&Ze(N))for(let Q of o.mount){let ce=ee(n,Q);if(ce&&ce._f){let ge=Array.isArray(ce._f.refs)?ce._f.refs[0]:ce._f.ref;if(Bc(ge)){let gt=ge.closest("form");if(gt){gt.reset();break}}}}if(E.keepFieldsRef)for(let Q of o.mount)ae(Q,ee(O,Q));else n={}}s=t.shouldUnregister?E.keepDefaultValues?pt(r):{}:pt(O),f.array.next({values:{...O}}),f.state.next({values:{...O}})}o={mount:E.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!E.keepIsValid||!!E.keepDirtyValues,i.watch=!!t.shouldUnregister,f.state.next({submitCount:E.keepSubmitCount?a.submitCount:0,isDirty:z?!1:E.keepDirty?a.isDirty:!!(E.keepDefaultValues&&!ur(N,r)),isSubmitted:E.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:z?{}:E.keepDirtyValues?E.keepDefaultValues&&s?cl(r,s):a.dirtyFields:E.keepDefaultValues&&N?cl(r,N):E.keepDirty?a.dirtyFields:{},touchedFields:E.keepTouched?a.touchedFields:{},errors:E.keepErrors?a.errors:{},isSubmitSuccessful:E.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},da=(N,E)=>_a(Oa(N)?N(s):N,E),Ua=(N,E={})=>{let M=ee(n,N),q=M&&M._f;if(q){let z=q.refs?q.refs[0]:q.ref;z.focus&&(z.focus(),E.shouldSelect&&Oa(z.select)&&z.select())}},X=N=>{a={...a,...N}},ie={control:{register:ua,unregister:la,getFieldState:De,handleSubmit:vt,setError:$t,_subscribe:Pa,_runSchema:R,_focusError:Vt,_getWatch:F,_getDirty:U,_setValid:y,_setFieldArray:g,_setDisabledField:rn,_setErrors:b,_getFieldArray:B,_reset:_a,_resetDefaultValues:()=>Oa(t.defaultValues)&&t.defaultValues().then(N=>{da(N,t.resetOptions),f.state.next({isLoading:!1})}),_removeUnmounted:L,_disableForm:sn,_subjects:f,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(N){i=N},get _defaultValues(){return r},get _names(){return o},set _names(N){o=N},get _formState(){return a},get _options(){return t},set _options(N){t={...t,...N}}},subscribe:kt,trigger:ht,register:ua,handleSubmit:vt,watch:Le,setValue:ae,getValues:Oe,reset:da,resetField:ca,clearErrors:at,unregister:la,setError:$t,setFocus:Ua,getFieldState:De};return{...ie,formControl:ie}}function P1(e={}){let t=Kt.default.useRef(void 0),a=Kt.default.useRef(void 0),[n,r]=Kt.default.useState({isDirty:!1,isValidating:!1,isLoading:Oa(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:Oa(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!Oa(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=A4(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,v4(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),Kt.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),Kt.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),Kt.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),Kt.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),Kt.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),Kt.default.useEffect(()=>{e.values&&!ur(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),Kt.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=h4(n,s),t.current}var U1={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},j1={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},D4={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function ne({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return u`
    <${s}
      className=${J(U1[a]??U1.default,j1[n]??j1.md,D4[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var bh="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",Hc={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function Ot({className:e="",size:t="md",error:a=!1,...n}){return u`
    <input
      className=${J(bh,Hc[t]??Hc.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Kc({className:e="",error:t=!1,rows:a=4,...n}){return u`
    <textarea
      rows=${a}
      className=${J(bh,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function xh({children:e,className:t="",size:a="md",error:n=!1,...r}){return u`
    <div className="relative w-full">
      <select
        className=${J(bh,Hc[a]??Hc.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
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
  `}function M4({children:e,className:t="",required:a=!1,...n}){return u`
    <label
      className=${J("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&u`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function En({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return u`
    <div className=${J("flex flex-col gap-2",s)}>
      ${e&&u`<${M4} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&u`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&u`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var O4={google:"Google",github:"GitHub",apple:"Apple"};function L4(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function F1({providers:e,redirectAfter:t}){let a=k();return e.length?u`
    <div className="mt-6 space-y-3">
      <div className="flex items-center gap-3 text-[11px] uppercase text-[var(--v2-text-faint)]">
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
        <span>${a("login.oauthDivider")}</span>
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
      </div>
      <div className="grid gap-2">
        ${e.map(n=>u`
            <${T}
              key=${n}
              as="a"
              href=${L4(n,t)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <${D} name="shield" className="h-4 w-4" />
              ${a("login.oauthProvider",{provider:O4[n]||n})}
            <//>
          `)}
      </div>
    </div>
  `:null}var P4=["google","github","apple"];function B1(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>{let a=!1;return b$().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(P4.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function z1({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=k(),{theme:s,toggleTheme:i}=Ac(),o=B1(),{formState:{errors:l,isSubmitting:c},handleSubmit:d,register:m}=P1({defaultValues:{token:e||""}});return u`
    <main
      className="relative flex min-h-[100dvh] items-center justify-center bg-[var(--v2-canvas)] px-4 py-8 sm:px-6 lg:px-12"
    >
      <!-- Theme toggle -->
      <${T}
        variant="secondary"
        size="icon"
        onClick=${i}
        aria-label=${r(s==="dark"?"theme.switchToLight":"theme.switchToDark")}
        title=${r(s==="dark"?"theme.light":"theme.dark")}
        className="absolute right-4 top-4 z-10 sm:right-6 sm:top-6"
      >
        <${D} name=${s==="dark"?"sun":"moon"} className="h-4 w-4" />
      <//>

      <!-- Login form (centered) -->
      <${ne}
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
          <${En}
            label=${r("login.tokenLabel")}
            htmlFor="v2-token"
            error=${l.token?.message??""}
            hint=${r("login.tokenHint")}
          >
            <${Ot}
              id="v2-token"
              type="password"
              error=${!!l.token}
              ...${m("token",{required:r("login.tokenRequired"),setValueAs:f=>f.trim()})}
              placeholder=${r("login.tokenPlaceholder")}
              autocomplete="current-password"
            />
          <//>

          ${t&&u`<p
              className=${J("rounded-[10px] border px-3 py-2 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >${t}</p>`}

          <${T}
            type="submit"
            variant="primary"
            fullWidth
            disabled=${c}
          >
            ${r("login.connect")}
          <//>
        </form>

        <${F1}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var q1={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},I1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function I({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return u`
    <span
      className=${J("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",I1[n]??I1.md,q1[e]??q1.muted,r)}
    >
      ${a&&u`<span
          className=${J("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var U4=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,H1=/(bash|shell|exec|run|command|terminal|spawn|process)/,K1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function Q1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return U4.test(n)?{tone:"danger",key:"tool.riskWrite"}:H1.test(n)?{tone:"warning",key:"tool.riskExec"}:K1.test(n)?{tone:"info",key:"tool.riskNetwork"}:H1.test(r)?{tone:"warning",key:"tool.riskExec"}:K1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}var Qc=480;function j4(e,t){return t&&t.length>0?t.some(a=>typeof a?.value=="string"&&a.value.length>Qc):typeof e=="string"&&e.length>Qc}function V1(e,t){return typeof e!="string"||t||e.length<=Qc?e:`${e.slice(0,Qc).trimEnd()}
...`}function G1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=k(),{toolName:s,description:i,parameters:o,allowAlways:l,approvalDetails:c=[]}=e,[d,m]=p.default.useState(!1),[f,h]=p.default.useState(!1),[x,y]=p.default.useState(!1),$=p.default.useRef(!1),g=p.default.useRef(e);g.current=e,p.default.useEffect(()=>{h(!1),$.current=!1,y(!1)},[e]);let v=p.default.useMemo(()=>Q1(s,i,o),[s,i,o]),b=s||r("approval.thisTool"),w=j4(o,c),S=f?"max-h-72":"max-h-36",C=p.default.useCallback(async _=>{if($.current)return;let A=g.current;$.current=!0,y(!0);try{await _?.()}finally{g.current===A&&($.current=!1,y(!1))}},[]),R=p.default.useCallback(()=>{C(d&&l?n:t)},[d,l,n,t,C]);return u`
    <div
      data-testid="approval-card"
      className="mx-auto max-w-lg rounded-xl border border-copper/30 bg-copper/10 p-4"
    >
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-copper/25 bg-copper/10 text-copper">
          <${D} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">${r("approval.title")}</span>
        <${I}
          tone=${v.tone}
          label=${r(v.key)}
          dot=${!1}
          size="sm"
          className="ml-auto"
        />
      </div>
      ${s&&u`<div className="mb-1 break-all font-mono text-sm font-medium text-iron-100">${s}</div>`}
      ${i&&u`<div className="mb-3 break-words text-sm text-iron-200">${i}</div>`}
      ${c.length>0?u`
            <dl className=${`mb-2 ${S} overflow-y-auto rounded-md border border-iron-800 bg-iron-950/80 text-xs`}>
              ${c.map(_=>u`
                  <div className="grid gap-1 border-b border-iron-800/70 px-3 py-2 last:border-b-0 sm:grid-cols-[7rem_1fr]">
                    <dt className="font-medium text-iron-400">${_.label}</dt>
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${V1(_.value,f)}</dd>
                  </div>
                `)}
            </dl>
          `:o&&u`<pre className=${`mb-2 ${S} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${V1(o,f)}</pre>`}

      ${w&&u`
        <${T}
          variant="ghost"
          size="sm"
          className="mb-3 px-0 text-[var(--v2-accent)] hover:bg-transparent"
          onClick=${()=>h(_=>!_)}
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
            onChange=${_=>m(_.currentTarget.checked)}
            disabled=${x}
            className="h-3.5 w-3.5 accent-[var(--v2-accent)]"
          />
          ${r("approval.alwaysAllowToolLabel",{tool:b})}
        </label>
      `}

      <div className="flex flex-wrap gap-2">
        <${T} variant="primary" onClick=${R} disabled=${x}>
          ${r(d&&l?"approval.approveAndAlways":"approval.approve")}
        <//>
        <${T}
          variant="secondary"
          onClick=${()=>C(a)}
          disabled=${x}
        >
          ${r("approval.deny")}
        <//>
      </div>
    </div>
  `}function pi({icon:e="lock",headline:t,provider:a,accountLabel:n,body:r,expiresAt:s,pillHint:i,defaultExpanded:o=!0,testId:l="auth-gate",challengeKind:c="",children:d}){let m=k(),[f,h]=p.default.useState(o),x=p.default.useId(),y=n||a||"";return u`
    <div
      data-testid=${l}
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
          <${D} name=${e} className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold text-white">
            ${t||m("authGate.title")}
          </span>
          ${y&&u`<span className="block truncate text-xs text-iron-300">${y}</span>`}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 text-xs font-medium text-[#8fc8f2]">
          ${i&&u`<span className="hidden sm:inline">${i}</span>`}
          <${D}
            name="chevron"
            className=${["h-4 w-4",f?"rotate-180":""].join(" ")}
          />
        </span>
      </button>

      ${f&&u`
        <div
          id=${x}
          className="border-t border-[rgba(76,167,230,0.2)] px-4 pb-4 pt-3"
        >
          ${r&&u`<div className="mb-3 text-sm text-iron-200">${r}</div>`}
          ${d}
          ${s&&u`
            <p className="mt-2 text-xs text-iron-300">
              ${m("authGate.expiresAt")}: ${new Date(s).toLocaleString()}
            </p>
          `}
        </div>
      `}
    </div>
  `}function Y1({gate:e,onCancel:t}){let a=k();return u`
    <${pi}
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
          <${T} type="button" variant="secondary" onClick=${()=>t?.()}>
            ${a("authGate.cancel")}
          <//>
        </div>
      </form>
    <//>
  `}function J1({gate:e,onCancel:t}){let a=k(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),o=p.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]);p.default.useEffect(()=>{i("")},[e.authorizationUrl,e.gateRef,e.runId]);let l=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),c=p.default.useCallback(()=>{if(!o){i(a("authGate.serviceUnavailable"));return}i(""),window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0)},[e.authorizationUrl,o]),d=n?a("authGate.reopenAuthorization",{provider:l}):a("authGate.openAuthorization",{provider:l});return u`
    <${pi}
      icon="link"
      headline=${e?.headline||a("authGate.oauthTitle")}
      provider=${e?.provider?l:""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      expiresAt=${e?.expiresAt||""}
      pillHint=${a("authGate.pillAuthorize")}
      challengeKind="oauth_url"
    >
      <div className="flex flex-wrap gap-2">
        <${T}
          as="a"
          href=${o?e.authorizationUrl:void 0}
          target="_blank"
          rel="noopener noreferrer"
          className="auth-oauth"
          data-testid="auth-oauth-open"
          variant="primary"
          onClick=${m=>{m.preventDefault(),c()}}
        >
          <${D} name="link" className="h-4 w-4" />
          ${d}
        <//>
        <${T}
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
  `}function X1({gate:e,onSubmit:t,onCancel:a}){let n=k(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState(!1),d=p.default.useCallback(async m=>{m.preventDefault();let f=r.trim();if(!f){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(f),s("")}catch(h){o(h?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return u`
    <${pi}
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
          <${Ot}
            type="password"
            autoComplete="off"
            spellCheck=${!1}
            value=${r}
            disabled=${l}
            placeholder=${n("authGate.tokenPlaceholder")}
            aria-label=${n("authGate.tokenLabel")}
            data-testid="auth-token-input"
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
          <${T} type="submit" variant="primary" disabled=${l}>
            ${n(l?"authGate.submitting":"authGate.submit")}
          <//>
          <${T}
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
  `}var F4="/api/webchat/v2/extensions/pairing/redeem";function W1({channel:e,action:t}){let a=k(),n=Z(),[r,s]=p.default.useState(""),i=z4(t,a),o=Y({mutationFn:({code:c})=>B4(e,c),onSuccess:()=>{s(""),n.invalidateQueries({queryKey:["extensions"]}),n.invalidateQueries({queryKey:["connectable-channels"]}),n.invalidateQueries({queryKey:["pairing",e]})}}),l=()=>{if(o.isPending)return;let c=r.trim().toUpperCase();c&&o.mutate({code:c})};return u`
    <div
      data-testid="pairing-section"
      className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4"
    >
      <h4 className="mb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        ${i.title}
      </h4>
      <p className="mb-4 text-xs leading-5 text-iron-300">${i.instructions}</p>

      <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          type="text"
          value=${r}
          onChange=${c=>s(c.target.value)}
          onKeyDown=${c=>c.key==="Enter"&&l()}
          placeholder=${i.placeholder}
          data-testid="pairing-code-input"
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${T}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${l}
          disabled=${o.isPending||!r.trim()}
          data-testid="pairing-submit"
        >
          ${i.submitLabel}
        <//>
      </div>

      ${o.isSuccess&&u`<p data-testid="pairing-success" className="text-xs text-emerald-300">
        ${o.data?.message||i.successMessage}
      </p>`}
      ${o.isError&&u`<p data-testid="pairing-error" className="text-xs text-red-300">
        ${q4(o.error,i.errorMessage)}
      </p>`}
    </div>
  `}function B4(e,t){return V(F4,{method:"POST",body:JSON.stringify({channel:e,code:t})}).then(a=>({...a,success:!0}))}function z4(e,t){return{title:e?.title||t("pairing.title"),instructions:e?.instructions||t("pairing.instructions"),placeholder:e?.input_placeholder||e?.code_placeholder||t("pairing.placeholder"),submitLabel:e?.submit_label||t("pairing.approve"),successMessage:e?.success_message||t("pairing.success"),errorMessage:e?.error_message||t("pairing.error")}}function q4(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var I4="/api/webchat/v2/extensions/pairing/redeem";function Z1(e){return V(I4,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function Vc({action:e}){let t=k(),a=Z(),n=Y({mutationFn:({code:l})=>Z1(l),onSuccess:()=>{s(""),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=p.default.useState(""),i=H4(e,t),o=()=>{if(n.isPending)return;let l=r.trim().toUpperCase();l&&n.mutate({code:l})};return u`
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
          onChange=${l=>s(l.target.value)}
          onKeyDown=${l=>l.key==="Enter"&&o()}
          placeholder=${i.codePlaceholder}
          data-testid="slack-pairing-code-input"
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${T}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          data-testid="slack-pairing-submit"
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
        ${K4(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function H4(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function K4(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function Q4(e,t){return e?.channel==="slack"&&e.strategy===t}function eS({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return u`
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
        ${t&&u`
          <button
            type="button"
            aria-label="Dismiss connect action"
            data-testid="channel-connect-dismiss"
            onClick=${t}
            className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-iron-400 hover:bg-white/[0.04] hover:text-iron-100"
          >
            <${D} name="close" className="h-4 w-4" />
          </button>
        `}
      </div>

      ${Q4(e,"inbound_proof_code")?u`<${Vc} action=${e.action} />`:e.strategy==="inbound_proof_code"?u`
              <${W1}
                channel=${a}
                action=${e.action}
              />
            `:u`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function V4(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):Kr.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:Kr.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:Kr.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:Kr.maxTotalBytes}:Kr}function tS(){let e=Sa(),t=K({enabled:!!e,queryKey:["session"],queryFn:wc,staleTime:5*6e4});return V4(t.data)}function Gc({onSend:e,onCancel:t,disabled:a,sendDisabled:n=a,canCancel:r=!1,initialText:s="",resetKey:i="",draftKey:o=sl,variant:l="dock",context:c={},statusText:d=""}){let m=k(),f=ft(),h=l==="hero",x=tS(),[y,$]=p.default.useState(()=>th(o)),[g,v]=p.default.useState(()=>nh(o)),[b,w]=p.default.useState(""),[S,C]=p.default.useState(!1),[R,_]=p.default.useState(!1),[A,L]=p.default.useState(!1),U=p.default.useRef(null),F=p.default.useRef(null),B=p.default.useRef(!1),P=a||n||S,G=p.default.useRef(a||n);G.current=a||n,B.current=P;let ae=p.default.useRef([]),le=p.default.useRef(Promise.resolve()),lt=p.default.useRef({draftKey:o,storageScope:f});lt.current={draftKey:o,storageScope:f},p.default.useEffect(()=>{ae.current=g},[g]);let ht=p.default.useRef(null),Oe=p.default.useRef(null),De=p.default.useCallback(()=>{Oe.current&&(window.clearTimeout(Oe.current),Oe.current=null);let O=ht.current;ht.current=null,O&&O.scope===ft()&&ah(O.key,O.text)},[]),at=p.default.useCallback(()=>{Oe.current&&(window.clearTimeout(Oe.current),Oe.current=null),ht.current=null},[]),$t=p.default.useCallback(()=>{let O=U.current;O&&(O.style.height="auto",O.style.height=`${Math.min(O.scrollHeight,200)}px`)},[]);p.default.useEffect(()=>{$t()},[y,$t]),p.default.useEffect(()=>($(th(o)),()=>De()),[o,f,De]);let Le=p.default.useRef(o),Pa=p.default.useRef(f);p.default.useEffect(()=>{if(Le.current!==o||Pa.current!==f){Le.current=o,Pa.current=f,v(nh(o)),w("");return}Ec(o,g)},[o,f,g]),p.default.useEffect(()=>{s&&($(s),window.requestAnimationFrame(()=>{U.current&&(U.current.focus(),U.current.setSelectionRange(s.length,s.length))}))},[s,i]);let kt=p.default.useCallback(O=>{if(a||!O||O.length===0)return;let Q=o,ce=f;le.current=le.current.then(async()=>{let ge=o,gt=f,{staged:Ce,errors:Ct}=await M$(O,{limits:x,existing:ae.current,t:m}),on=lt.current;if(!(on.draftKey!==ge||on.storageScope!==gt||ft()!==gt)){if(Ce.length>0){let ja=[...ae.current,...Ce];ae.current=ja,Ec(ge,ja),v(ja)}w(Ct.length>0?Ct.join(" "):"")}}).catch(()=>{w(m("chat.attachmentStagingFailed"))})},[a,o,x,f,m]),la=p.default.useCallback(O=>{let Q=ae.current.filter(ce=>ce.id!==O);ae.current=Q,Ec(o,Q),v(Q),w("")},[o]),rn=p.default.useCallback(()=>{a||F.current?.click()},[a]),ua=p.default.useCallback(O=>{let Q=Array.from(O.target.files||[]);kt(Q),O.target.value=""},[kt]),Vt=p.default.useCallback(async()=>{let O=y.trim(),Q=g.length>0,ce=O||(Q?Cc:"");if(!(!ce||B.current)){B.current=!0,C(!0);try{if(await e(ce,{attachments:g,displayContent:O})===null)return;$(""),v([]),ae.current=[],w(""),at(),Q$(o),V$(o),U.current&&(U.current.style.height="auto")}catch{}finally{B.current=G.current,C(!1)}}},[y,g,e,o,at,a,n]),sn=p.default.useCallback(O=>{let Q=O.target.value;$(Q),ht.current={key:o,text:Q,scope:ft()},Oe.current&&window.clearTimeout(Oe.current),Oe.current=window.setTimeout(De,300)},[o,De]),vt=p.default.useCallback(async()=>{if(!(!r||R||!t)){_(!0);try{await t()}finally{_(!1)}}},[r,R,t]),ca=p.default.useCallback(O=>{if(O.key==="Enter"&&!O.shiftKey){if(O.preventDefault(),U.current?.dataset?.sendDisabled==="true"||B.current)return;Vt()}},[Vt]),_a=p.default.useCallback(O=>{let Q=Array.from(O.clipboardData?.files||[]);Q.length>0&&(O.preventDefault(),kt(Q))},[kt]),da=p.default.useCallback(O=>{O.preventDefault(),L(!1);let Q=Array.from(O.dataTransfer?.files||[]);Q.length>0&&kt(Q)},[kt]),Ua=p.default.useCallback(O=>{O.preventDefault(),!a&&L(!0)},[a]),X=p.default.useCallback(O=>{O.currentTarget.contains(O.relatedTarget)||L(!1)},[]),re=y.trim()||g.length>0,ie=a||n,N=m(h?"chat.heroPlaceholder":"chat.followUpPlaceholder"),E=x.accept.length>0?x.accept.join(","):void 0,M=h?"w-full":"px-4 py-3 sm:px-5 lg:px-8",q=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",h?"min-h-[120px]":"",a?"opacity-70":""].join(" "),z=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",h?"min-h-[72px]":"min-h-[40px]"].join(" ");return u`
    <div className=${M}>
      <div
        className=${q}
        onDrop=${da}
        onDragOver=${Ua}
        onDragLeave=${X}
      >
        ${A&&u`
          <div className="pointer-events-none absolute inset-1 z-10 flex items-center justify-center rounded-[16px] border border-dashed border-[color-mix(in_srgb,var(--v2-accent)_55%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-canvas)_82%,transparent)] text-sm font-medium text-[var(--v2-accent-text)]">
            ${m("chat.attachmentDropHint")}
          </div>
        `}
        ${b&&u`
          <div
            role="alert"
            className="mb-3 flex items-start gap-2 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-2 text-xs leading-5 text-[var(--v2-danger-text)]"
          >
            <span className="min-w-0 flex-1">${b}</span>
            <button
              type="button"
              onClick=${()=>w("")}
              aria-label=${m("common.dismiss")}
              title=${m("common.dismiss")}
              className="-mr-1 -mt-0.5 shrink-0 rounded p-0.5 text-[color-mix(in_srgb,var(--v2-danger-text)_80%,transparent)] transition hover:bg-[color-mix(in_srgb,var(--v2-danger-text)_14%,transparent)] hover:text-[var(--v2-danger-text)]"
            >
              <${D} name="close" className="h-3.5 w-3.5" strokeWidth=${2} />
            </button>
          </div>
        `}

        ${g.length>0&&u`
          <div className="mb-2 flex flex-wrap gap-2 px-1">
            ${g.map(O=>u`
                <div
                  key=${O.id}
                  className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
                >
                  ${O.previewUrl?u`<img
                        src=${O.previewUrl}
                        alt=${O.filename}
                        className="h-9 w-9 shrink-0 rounded object-cover"
                      />`:u`<span
                        className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                      >
                        <${D} name="file" className="h-4 w-4" />
                      </span>`}
                  <span className="flex min-w-0 flex-col">
                    <span className="max-w-[12rem] truncate font-medium">
                      ${O.filename}
                    </span>
                    <span className="text-[10px] text-iron-400">${O.sizeLabel}</span>
                  </span>
                  <button
                    type="button"
                    onClick=${()=>la(O.id)}
                    aria-label=${m("chat.attachmentRemove")}
                    title=${m("chat.attachmentRemove")}
                    className="absolute right-1 top-1 grid h-5 w-5 place-items-center rounded-full text-iron-400 hover:bg-iron-700 hover:text-white"
                  >
                    <${D} name="close" className="h-3 w-3" />
                  </button>
                </div>
              `)}
          </div>
        `}

        <textarea
          ref=${U}
          data-testid="chat-composer"
          value=${y}
          onChange=${sn}
          onKeyDown=${ca}
          onPaste=${_a}
          data-send-disabled=${ie?"true":"false"}
          placeholder=${N}
          rows=${1}
          disabled=${a}
          className=${z}
        />

        <input
          ref=${F}
          type="file"
          multiple
          accept=${E}
          className="hidden"
          onChange=${ua}
        />

        <div className="mt-2 flex items-center gap-2">
          ${ie&&u`
            <span className="inline-flex items-center gap-2 text-xs text-[var(--v2-text-muted)]">
              <span className="h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
              ${d||m("chat.statusWorking")}
            </span>
          `}
          <div className="ml-auto flex items-center gap-1.5">
            <button
              type="button"
              onClick=${rn}
              disabled=${a}
              aria-label=${m("chat.attachFiles")}
              title=${m("chat.attachFiles")}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <${D} name="plus" className="h-5 w-5" />
            </button>
            ${r?u`
                <${T}
                  type="button"
                  variant="danger"
                  size="icon-sm"
                  onClick=${vt}
                  disabled=${R}
                  aria-label=${m("common.cancel")}
                  title=${m("common.cancel")}
                  className="rounded-full"
                >
                  <${D} name="close" className="h-5 w-5" />
                <//>
              `:u`
                <${T}
                  type="button"
                  variant="primary"
                  size="icon-sm"
                  onClick=${Vt}
                  disabled=${ie||S||!re}
                  aria-label=${m("chat.send")}
                  className="rounded-full"
                >
                  <${D} name="send" className="h-5 w-5" />
                <//>
              `}
          </div>
        </div>
      </div>
    </div>
  `}var aS={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function nS({status:e}){let t=k();if(e==="idle"||e==="connecting"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return u`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",aS[e]||aS.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function rS({onSuggestion:e,onSend:t,disabled:a,sendDisabled:n,initialText:r,resetKey:s,draftKey:i,context:o,statusText:l,canCancel:c,onCancel:d}){let m=k(),f=[{icon:"tool",title:m("chat.suggestion1"),detail:m("chat.suggestion1Desc")},{icon:"shield",title:m("chat.suggestion2"),detail:m("chat.suggestion2Desc")},{icon:"plug",title:m("chat.suggestion3"),detail:m("chat.suggestion3Desc")}];return u`
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
        <${Gc}
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
                <${D} name=${h.icon} className="h-4 w-4" />
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
  `}var G4=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function sS({open:e,onClose:t}){let a=k();return e?u`
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
            <${D} name="bolt" className="h-4 w-4" />
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
            <${D} name="close" className="h-4 w-4" />
          </button>
        </div>
        <ul className="flex flex-col gap-2">
          ${G4.map((n,r)=>u`
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
  `:null}function oS(e){let t=0,a=0,n=0,r=0,s=0;for(let o of e){if(o.role==="thinking"&&(t+=1),o.role==="tool_activity"){let l=iS([o]);a+=l.tools,n+=l.failed,r+=l.declined,s+=l.running}if(Y4(o)){let l=iS(o.toolCalls);a+=l.tools,n+=l.failed,r+=l.declined,s+=l.running}}let i=[];return t&&i.push(`${t} reasoning`),a&&i.push(`${a} ${a===1?"tool":"tools"}`),n&&i.push(`${n} failed`),r&&i.push(`${r} declined`),!n&&!r&&s&&i.push("running"),{hasError:n>0,hasDeclined:r>0,label:`Activity${i.length?` - ${i.join(", ")}`:""}`}}function iS(e){let t=0,a=0,n=0;for(let r of e)r.toolStatus==="error"&&(t+=1),r.toolStatus==="declined"&&(a+=1),r.toolStatus==="running"&&(n+=1);return{tools:e.length,failed:t,declined:a,running:n}}function Y4(e){return e.toolCalls&&e.toolCalls.length>0}var lS=!1;function J4(){lS||!window.DOMPurify||(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),lS=!0)}function uS(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}J4();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var $h=360;function X4(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let l=s("Copy");if(l.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),l.textContent="Copied",di("Code copied",{tone:"success"}),setTimeout(()=>l.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(l),n.appendChild(r),t.scrollHeight>$h){t.style.maxHeight=`${$h}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${$h}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function W4({content:e,className:t=""}){let a=p.default.useRef(null),n=p.default.useMemo(()=>uS(e),[e]);return p.default.useEffect(()=>{X4(a.current)},[n]),u`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var sa=p.default.memo(W4);var cS={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",declined:"bg-iron-400",error:"bg-[var(--v2-danger-text)]"},Z4={success:"ok",declined:"declined",error:"err",running:"run"},e5=2;function hi({activity:e}){return e.toolCalls&&e.toolCalls.length>0?u`<${a5} tools=${e.toolCalls} />`:u`<${n5} activity=${e} />`}function t5(e,t){let a=0,n=0,r=0,s=0;for(let l of t){let c=String(l.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function a5({tools:e}){let t=k(),a=e.some(o=>o.toolStatus==="error"),n=e.some(o=>o.toolStatus==="error"||o.toolStatus==="declined"),[r,s]=p.default.useState(n);if(p.default.useEffect(()=>{n&&s(!0)},[n]),e.length<=e5)return u`
      <div className="flex flex-col gap-3">
        ${e.map((o,l)=>u`<${hi}
            key=${o.id||o.callId||`${o.toolName}-${l}`}
            activity=${o}
          />`)}
      </div>
    `;let i=t5(t,e);return u`
    <div className="flex flex-col">
      <button
        type="button"
        onClick=${()=>s(o=>!o)}
        aria-expanded=${r?"true":"false"}
        className=${["v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",a?"text-[var(--v2-danger-text)]":"text-iron-400 hover:text-iron-200"].join(" ")}
      >
        <${D} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${i}</span>
        <${D}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0",r?"rotate-180":""].join(" ")}
        />
      </button>

      ${r&&u`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((o,l)=>u`<${hi}
              key=${o.id||o.callId||`${o.toolName}-${l}`}
              activity=${o}
            />`)}
        </div>
      `}
    </div>
  `}function n5({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:l}=e,[c,d]=p.default.useState(n==="error"||n==="declined");p.default.useEffect(()=>{(n==="error"||n==="declined")&&d(!0)},[n]);let m=cS[n]||cS.running,f=i!=null,h=p.default.useId(),x=u`
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
        >${Z4[n]||"run"}</span
      >
      <span className="shrink-0 truncate font-mono text-[13px] font-medium text-iron-100"
        >${a}</span
      >
      ${r&&u`<span className="min-w-0 truncate font-mono text-xs text-iron-400"
        >${r}</span
      >`}
      <span className="ml-auto flex shrink-0 items-center gap-2">
        ${f&&u`<span className="font-mono text-[11px] text-iron-300">${i}ms</span>`}
        <${D}
          name="chevron"
          className=${["h-3.5 w-3.5 text-iron-400",c?"rotate-180":""].join(" ")}
        />
      </span>
    </button>
  `;return u`
    <div
      className=${t?"":"flex gap-3"}
      data-testid="tool-activity-card"
      data-tool-name=${a||""}
      data-tool-status=${n||""}
    >
      ${!t&&u`
        <div
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
        >
          <${D} name="tool" className="h-4 w-4" />
        </div>
      `}
      <div className=${t?"min-w-0 flex-1":"min-w-0 max-w-[85%] flex-1"}>
        ${x}
        ${c&&u`<${r5}
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
  `}function r5({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=k(),l=p.default.useMemo(()=>{let f=[];return r&&f.push({id:s==="declined"?"declined":"error",label:o(s==="declined"?"tool.tabDeclined":"tool.tabError")}),t&&f.push({id:"details",label:o("tool.tabDetails")}),a&&f.push({id:"params",label:o("tool.tabParameters")}),n&&f.push({id:"result",label:o("tool.tabResult")}),f},[o,r,t,a,n,s]),[c,d]=p.default.useState(null),m=c&&l.some(f=>f.id===c)?c:l[0]?.id;return p.default.useEffect(()=>{r&&d(s==="declined"?"declined":"error")},[r,s]),l.length===0?u`
      <div
        id=${e}
        className="rounded-b-lg border-x border-b border-iron-700/40 bg-iron-950 px-3 py-2 font-mono text-xs text-iron-400"
      >
        ${o("tool.noDetail")}
      </div>
    `:u`
    <div
      id=${e}
      data-testid="tool-activity-detail"
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
        ${m==="result"&&u`<${s5} text=${n} />`}
        ${(m==="error"||m==="declined")&&u`<pre
          className=${["overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono",m==="declined"?"text-iron-300":"text-[var(--v2-danger-text)]"].join(" ")}
        >${r}</pre>`}
      </div>
    </div>
  `}function s5({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return u`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(i5)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return u`
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
                  >${o5(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?u`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:u`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function i5(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function o5(e){return e==null?"":String(e)}function dS({activity:e}){let t=oS(e),a=c5(e),[n,r]=p.default.useState(a);return p.default.useEffect(()=>{a&&r(!0)},[a]),u`
    <div className="mr-auto flex w-full max-w-[85%] flex-col" data-testid="activity-run">
      <button
        type="button"
        onClick=${()=>r(s=>!s)}
        aria-expanded=${n?"true":"false"}
        data-testid="activity-run-toggle"
        className=${["v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",t.hasError?"text-[var(--v2-danger-text)]":"text-iron-400 hover:text-iron-200"].join(" ")}
      >
        <${D} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${t.label}</span>
        <${D}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0",n?"rotate-180":""].join(" ")}
        />
      </button>

      ${n&&u`
        <div className="mt-2 flex flex-col gap-3" data-testid="activity-run-items">
          ${e.map((s,i)=>u`
            <${l5}
              key=${s.id||`${s.role||"activity"}-${i}`}
              item=${s}
            />
          `)}
        </div>
      `}
    </div>
  `}function l5({item:e}){if(e.role==="thinking")return u`<${u5} content=${e.content} />`;if(e.role==="tool_activity"||wh(e)){let t=wh(e)?{id:e.id,toolCalls:e.toolCalls}:e;return u`<${hi} activity=${t} />`}return null}function u5({content:e}){return e?u`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <${D} name="spark" className="h-4 w-4" />
      </div>
      <div className="min-w-0 max-w-[85%] flex-1 border-l-2 border-white/10 pl-3 text-iron-300">
        <${sa} content=${e} className="text-[13px]" />
      </div>
    </div>
  `:null}function wh(e){return e?.toolCalls&&e.toolCalls.length>0}function c5(e){return(e||[]).some(t=>t?.role==="thinking"||t?.toolStatus==="running"||t?.toolStatus==="error"||t?.toolStatus==="declined"?!0:wh(t)?t.toolCalls.some(a=>a?.toolStatus==="running"||a?.toolStatus==="error"||a?.toolStatus==="declined"):!1)}function vi(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function d5({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=p.default.useState(()=>t&&e.preview_url||null);return p.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return Rc(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?u`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:u`<${D} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var mS="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",fS="px-3 py-2";function Yc({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await Aa(e.fetch_url);vi(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),l=u`
    <${d5} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?u`<div
      className=${`${mS} ${fS} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${l}
    </div>`:u`<div className=${`${mS} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${fS} text-left transition-colors hover:bg-iron-900/80`}
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
      <${D} name="download" className="h-3.5 w-3.5" />
    </button>`}
  </div>`}var pS={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function gi({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return p.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),p.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?u`
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
        className=${J("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",pS[n]??pS.md,r)}
      >
        ${a?u`<${Sh} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function Sh({children:e,onClose:t,className:a=""}){return u`
    <div
      className=${J("flex shrink-0 items-center justify-between gap-4","px-5 py-4 md:px-7 md:py-5","border-b border-[var(--v2-panel-border)]",a)}
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
            <${D} name="close" className="h-4 w-4" />
          </button>
        `}
    </div>
  `}function yi({children:e,className:t=""}){return u`
    <div className=${J("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function bi({children:e,className:t=""}){return u`
    <div
      className=${J("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var hS=1e5;function Jc({attachment:e,onClose:t}){let a=!!e,[n,r]=p.default.useState("loading"),[s,i]=p.default.useState({}),o=e?D$(e.mime_type):"download";if(p.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return Aa(e.fetch_url).then(async m=>{d=URL.createObjectURL(m);let f={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")f.dataUrl=await Qp(m);else if(o==="pdf")f.frameUrl=d;else if(o==="text"){let h=await m.text();f.truncated=h.length>hS,f.text=f.truncated?h.slice(0,hS):h}if(c){URL.revokeObjectURL(d);return}i(f),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let l=e.filename||"attachment";return u`
    <${gi} open=${a} onClose=${t} size="xl">
      <${Sh} onClose=${t}>
        <span className="block truncate">${l}</span>
      <//>
      <${yi} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&u`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&u`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&u`<${m5} mode=${o} view=${s} filename=${l} />`}
      <//>
      <${bi}>
        ${s.downloadUrl&&u`<a
          href=${s.downloadUrl}
          download=${l}
          data-testid="attachment-download"
          className="v2-button inline-flex items-center gap-1.5 rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-200 hover:border-signal/35 hover:text-white"
        >
          <${D} name="download" className="h-3.5 w-3.5" />
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
  `}function m5({mode:e,view:t,filename:a}){switch(e){case"image":return u`<img
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
        <${D} name="file" className="h-10 w-10 text-signal" />
        <div className="text-sm">This file type can't be previewed.</div>
      </div>`}}var f5=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function p5(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function vS(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of p5(e).matchAll(f5)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function gS(e){return e.split("/").filter(Boolean).pop()||e}function yS(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function h5({threadId:e,path:t,onPreview:a}){let[n,r]=p.default.useState({mime_type:"",size_label:""});p.default.useEffect(()=>{let i=!0;return e$({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:yS(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:gS(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:_c({threadId:e,path:t})};return u`<${Yc}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function bS({threadId:e,content:t}){let a=p.default.useMemo(()=>vS(t),[t]),[n,r]=p.default.useState(null);return!e||a.length===0?null:u`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>u`<${h5}
          key=${s}
          threadId=${e}
          path=${s}
          onPreview=${r}
        />`)}
      <${Jc}
        attachment=${n}
        onClose=${()=>r(null)}
      />
    </div>
  `}var xS={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function v5(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function g5({content:e}){let[t,a]=p.default.useState(!1);return e?u`
    <div className="flex flex-col items-start">
      <button
        type="button"
        onClick=${()=>a(n=>!n)}
        aria-expanded=${t?"true":"false"}
        className="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent px-1 py-1 text-xs font-medium text-iron-400 hover:text-iron-200"
      >
        <${D} name="spark" className="h-3.5 w-3.5" />
        <span>${t?"Hide reasoning":"Reasoning"}</span>
        <${D}
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
  `:null}function y5({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:l,status:c,error:d,toolCalls:m,timestamp:f}=e,h=n==="user",[x,y]=p.default.useState(!1),[$,g]=p.default.useState(null),v=p.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),y(!0),di("Copied to clipboard",{tone:"success"}),setTimeout(()=>y(!1),1400)}catch{}},[r]);if(n==="tool_activity"||m&&m.length>0){let L=m&&m.length>0?{id:e.id,toolCalls:m}:e;return u`<${hi} activity=${L} />`}if(n==="thinking")return u`<${g5} content=${r} />`;if(n==="image")return u`
      <div className="flex">
        <div className="flex flex-wrap gap-2">
          ${(o||[]).map((U,F)=>U.data_url?u`<img key=${F} src=${U.data_url} className="max-h-64 rounded-lg border border-iron-700 object-cover" alt="Generated result" />`:u`
                  <div key=${F} className="rounded-lg border border-iron-700 bg-iron-900/70 px-4 py-3 text-sm text-iron-200">
                    <div>Generated image unavailable in history payload</div>
                    ${U.path&&u`<div className="mt-1 font-mono text-xs text-iron-300">${U.path}</div>`}
                  </div>
                `)}
        </div>
      </div>
    `;let b=v5(f),w=n==="user"||n==="assistant"&&!l,S=n==="system"||n==="error",C=h?"max-w-[85%]":S?"mx-auto max-w-[85%]":"w-full max-w-[85%]",R=h?"":"w-full min-w-0 max-w-full",_=c==="error"&&t,A=w||_||b;return u`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",h?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col",C].join(" ")}>
        <div
          className=${["text-base leading-7",R,xS[n]||xS.assistant,l?"opacity-70":""].join(" ")}
        >
          ${n==="assistant"||n==="system"||n==="error"?u`<${sa} content=${r} />`:u`<div className="whitespace-pre-wrap">${r}</div>`}

          ${c==="error"&&u`
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-300">
              <span>${d}</span>
            </div>
          `}

          ${s&&s.length>0&&u`
            <div className="mt-2 flex flex-wrap gap-2">
              ${s.map((L,U)=>u`<img key=${U} src=${L} className="max-h-48 rounded-lg border border-iron-700 object-cover" alt="Message attachment" />`)}
            </div>
          `}

          ${i&&i.length>0&&u`
            <div className="mt-2 flex flex-col gap-1.5">
              ${i.map((L,U)=>u`<${Yc}
                key=${L.id||U}
                att=${L}
                onPreview=${g}
              />`)}
            </div>
            <${Jc}
              attachment=${$}
              onClose=${()=>g(null)}
            />
          `}

          ${n==="assistant"&&u`<${bS}
            threadId=${a}
            content=${typeof r=="string"?r:""}
          />`}
        </div>
      </div>

      ${A&&u`
        <div
          className=${["mt-1 flex min-h-7 w-max max-w-[85%] flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100",h?"self-end justify-end":S?"self-center justify-center":"self-start justify-start"].join(" ")}
        >
          ${b&&u`<time dateTime=${f} className="shrink-0 font-mono text-[11px] text-iron-500">${b}</time>`}
          ${(w||_)&&u`
            <div className="flex shrink-0 items-center gap-1">
            ${w&&u`
              <button
                type="button"
                onClick=${v}
                title=${x?"Copied":"Copy message"}
                aria-label=${x?"Copied":"Copy message"}
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 hover:text-iron-100"
              >
                <${D} name=${x?"check":"copy"} className="h-3.5 w-3.5" />
              </button>
            `}
            ${_&&u`
              <button
                type="button"
                onClick=${()=>t(e)}
                title="Retry message"
                aria-label="Retry message"
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 text-red-300 hover:text-red-200"
              >
                <${D} name="retry" className="h-3.5 w-3.5" />
              </button>
            `}
            </div>
          `}
        </div>
      `}
    </div>
  `}var $S=p.default.memo(y5);function kS(e){let t=b5(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(CS(r)){let s=wS(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){SS(a,s),NS(a,r),n+=s.length;continue}}if(Nh(r)){let s=wS(t,n);SS(a,s),n+=s.length-1;continue}NS(a,r)}return a}function b5(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=Xc(i);o&&CS(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!Nh(i))continue;let o=Xc(i),l=o?t.get(o):void 0;if(l===void 0||l>=s)continue;let c=a.get(l)||[];c.push(i),a.set(l,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function wS(e,t){let a=t,n=Xc(e[t]);for(;a<e.length&&Nh(e[a])&&x5(n,e[a]);)a+=1;return e.slice(t,a)}function x5(e,t){let a=Xc(t);return!e||!a||a===e}function SS(e,t){if(t.length===0)return;let a=$5(t);e.push({type:"activity-run",id:`activity-run-${a[0].id}`,activity:a})}function NS(e,t){e.push({type:"message",id:t.id,message:t})}function CS(e){return e.role==="assistant"&&!ES(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function Nh(e){return e.role==="thinking"||e.role==="tool_activity"||ES(e)}function ES(e){return e?.toolCalls&&e.toolCalls.length>0}function Xc(e){return e?.turnRunId||null}function $5(e){return[...e].sort((t,a)=>t?.role!=="tool_activity"||a?.role!=="tool_activity"?0:w5(t,a))}function w5(e,t){if(Number.isFinite(e.activityOrder)&&Number.isFinite(t.activityOrder)){let n=e.activityOrder-t.activityOrder;if(n!==0)return n}let a=_S(RS(e.updatedAt||e.timestamp),RS(t.updatedAt||t.timestamp));return a!==0?a:_S(e.sequence,t.sequence)}function _S(e,t){let a=Number.isFinite(e)?e:null,n=Number.isFinite(t)?t:null;return a===null&&n===null?0:a===null?1:n===null?-1:a-n}function RS(e){if(!e)return null;let t=Date.parse(e);return Number.isFinite(t)?t:null}var S5=100,N5=100;function _5(e){return e?e.scrollHeight-e.scrollTop-e.clientHeight:Number.POSITIVE_INFINITY}function TS(e,t=S5){return _5(e)<=t}function AS(e){e&&(e.scrollTop=Math.max(0,e.scrollHeight-e.clientHeight))}function DS(e){return e?.id?`${e.role||""}:${e.id}`:null}function R5(e,t){let a=DS(t);return!!(a&&t?.role==="user"&&a!==e)}function k5(){return typeof window>"u"||!window.getSelection?"":String(window.getSelection()?.toString?.()||"")}function MS({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let l=k(),c=p.default.useRef(null),d=p.default.useRef(null),m=p.default.useRef(!0),f=p.default.useRef(null),h=p.default.useRef(null),x=p.default.useRef(null),y=p.default.useRef(0),$=p.default.useRef(!1),[g,v]=p.default.useState(!0),b=p.default.useCallback(()=>{h.current!==null&&(window.cancelAnimationFrame(h.current),h.current=null)},[]),w=p.default.useCallback((B=!1)=>{c.current&&(B&&(m.current=!0,$.current=!1),m.current&&(b(),h.current=window.requestAnimationFrame(()=>{h.current=null;let G=c.current;!G||!B&&!m.current||(AS(G),y.current=G.scrollTop,$.current=!1,v(!0))})))},[b]),S=p.default.useCallback(()=>{x.current!==null&&(window.cancelAnimationFrame(x.current),x.current=null)},[]);p.default.useLayoutEffect(()=>{let B=e.length>0?e[e.length-1]:null,P=DS(B),G=R5(f.current,B);return f.current=P,w(G),b},[e,i,w,b]),p.default.useLayoutEffect(()=>{let B=d.current;if(!B||typeof ResizeObserver!="function")return;let P=new ResizeObserver(()=>{w()});return P.observe(B),()=>{P.disconnect(),b()}},[w,b]);let C=p.default.useCallback(()=>{x.current=null;let B=c.current;if(!B)return;let P=TS(B);y.current=B.scrollTop,P?(m.current=!0,$.current=!1,v(!0)):$.current?(m.current=!1,v(!1)):(m.current=!0,v(!0),w()),a&&B.scrollTop<N5&&n&&!t&&n()},[a,n,t,w]),R=p.default.useCallback(()=>{$.current=!0},[]),_=p.default.useCallback(B=>{let P=c.current;if(!P||typeof B?.clientX!="number")return;let G=P.offsetWidth-P.clientWidth;if(G<=0)return;let ae=P.getBoundingClientRect().right;B.clientX>=ae-G-2&&($.current=!0)},[]),A=p.default.useCallback(()=>{let B=c.current;if(!B)return;let P=TS(B),G=B.scrollTop<y.current;y.current=B.scrollTop,!P&&G&&($.current=!0),P?(m.current=!0,$.current=!1):$.current?(m.current=!1,b()):m.current=!0,x.current===null&&(x.current=window.requestAnimationFrame(C))},[b,C]),L=p.default.useCallback(()=>{let B=c.current;B&&(AS(B),y.current=B.scrollTop,m.current=!0,$.current=!1,v(!0))},[]),U=p.default.useCallback(B=>{let P=k5();!P||!B.clipboardData||(B.preventDefault(),B.clipboardData.clearData(),B.clipboardData.setData("text/plain",P))},[]);p.default.useEffect(()=>S,[S]);let F=p.default.useMemo(()=>kS(e),[e]);return u`
    <div className="relative flex min-h-0 min-w-0 flex-1">
    <div
      ref=${c}
      onScroll=${A}
      onWheel=${R}
      onTouchMove=${R}
      onPointerDown=${_}
      onCopy=${U}
      data-testid="message-list-scroll"
      className="flex min-w-0 flex-1 overflow-y-auto px-4 pt-6 pb-14 sm:px-5 lg:px-8"
    >
      <div
        ref=${d}
        data-testid="message-list-content"
        className="mx-auto flex w-full min-w-0 max-w-5xl flex-col gap-5"
      >
        ${a&&u`
          <div className="text-center">
            <button
              onClick=${n}
              disabled=${t}
              data-testid="message-list-load-older"
              className="v2-button rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-300 hover:border-signal/35 hover:text-white disabled:opacity-50"
            >
              ${l(t?"chat.history.loading":"chat.history.loadOlder")}
            </button>
          </div>
        `}
        ${F.map(B=>B.type==="activity-run"?u`<${dS} key=${B.id} activity=${B.activity} />`:u`<${$S}
                key=${B.id}
                message=${B.message}
                onRetry=${r}
                threadId=${s}
              />`)}
        ${o}
      </div>
    </div>
    ${!g&&u`
      <button
        type="button"
        onClick=${L}
        aria-label=${l("chat.jumpToLatest")}
        className="absolute bottom-4 left-1/2 inline-flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-3 py-1.5 text-xs font-medium text-[var(--v2-text-strong)] shadow-[0_10px_30px_-12px_rgba(0,0,0,0.7)] hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]"
      >
        <${D} name="arrowDown" className="h-3.5 w-3.5" />
        ${l("chat.jumpToLatest")}
      </button>
    `}
    </div>
  `}function OS({notice:e,onRecover:t}){return u`
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
  `}function LS({suggestions:e,onSelect:t,disabled:a=!1}){return!e||e.length===0?null:u`
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
  `}function PS(){return u`
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
  `}function Wc(){return V("/api/webchat/v2/channels/connectable")}function US(e,t){if(!_h(e))return null;let a=Zc(e),n=A5(a),r=null;for(let s of t||[]){if(!T5(s))continue;let i=D5(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function _h(e){let t=Zc(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function C5(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function E5(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>jS(Zc(n))):a}function T5(e){return e?.strategy!=="admin_managed_channels"}function A5(e){return FS(e,"slack")&&jS(e)}function jS(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function Zc(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function D5(e,t,a={}){return(a.commandAliasesOnly?E5(t,{channelManagementOnly:!0}):C5(t)).reduce((r,s)=>{let i=Zc(s);return FS(e,i)?Math.max(r,i.length):r},0)}function FS(e,t){return t?` ${e} `.includes(` ${t} `):!1}function BS(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n={kind:"gate",gateKind:"approval",runId:t.turn_run_id,gateRef:t.gate_ref,invocationId:t.invocation_id||null,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return M5(n,a,t.body)}return e==="auth_required"?{kind:"auth_required",gateKind:"auth",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,invocationId:t.invocation_id||null,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function zS(e){if(!e?.run_id||!e.gate_ref)return null;let t=e.gate_kind||"generic",a={gateKind:t,runId:e.run_id,gateRef:e.gate_ref,invocationId:e.invocation_id||null,headline:e.headline,body:e.body||"",allowAlways:e.allow_always===!0};if(t==="auth"){let n=e.auth_context||{};return{...a,kind:"auth_required",challengeKind:n.challenge_kind||"other",provider:n.provider||null,accountLabel:n.account_label||"",authorizationUrl:n.authorization_url||null,expiresAt:n.expires_at||null}}return{...a,kind:"gate"}}function M5(e,t,a){if(!t)return e;let n=O5(t);return{...e,toolName:t.tool_name||null,description:t.reason||a,actionLabel:t.action?.label||null,destination:t.destination||null,approvalScope:t.scope||null,approvalDetails:n,parameters:n.length?n.map(r=>`${r.label}: ${r.value}`).join(`
`):null}}function O5(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function qS({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function IS(){return{terminalByInvocation:new Map}}function HS(e){e?.current?.terminalByInvocation?.clear()}function kh(e,t,a){let n=QS(t,{toolStatus:"running"});n&&xi(e,n,a)}function KS(e,t,a,n="gate_declined"){let r=QS(t,{toolStatus:"declined",toolError:n,toolErrorKind:"gate_declined"});r&&xi(e,r,a)}function xi(e,t,a){if(!t)return;let n=B5(t);n=F5(n,a),e(r=>{let s=VS(n),i=P5(r,n,s);if(i>=0){let l=[...r];return l[i]=U5(l[i],n),Rh(l[i],a),l}let o={id:s,role:"tool_activity",...n};return Rh(o,a),[...r,o]})}function QS(e,t={}){let a=e?.kind==="gate",n=e?.kind==="auth_required",r=n&&t.toolStatus==="declined";if(!e?.runId||!e?.gateRef||!e.invocationId&&!r||!a&&!n)return null;let s=e.invocationId||L5(e),i=e.toolName||e.headline||e.gateKind||"gate";return{invocationId:s,callId:s,capabilityId:e.toolName||e.gateKind||null,toolName:al(i)||i,toolStatus:t.toolStatus||"running",toolDetail:null,toolParameters:null,toolResultPreview:null,toolError:t.toolError||null,toolErrorKind:t.toolErrorKind||null,toolDurationMs:null,updatedAt:t.updatedAt||new Date().toISOString(),resultRef:null,truncated:!1,outputBytes:null,outputKind:null,turnRunId:e.runId,gateRef:e.gateRef,gateActivity:!0}}function L5(e){return`gate:${e.runId}:${e.kind}:${e.gateRef}`}function VS(e){return`tool-${e.invocationId}`}function P5(e,t,a){let n=e.findIndex(s=>s?.id===a);if(n>=0)return n;let r=t.gateRef||null;if(r){let s=e.findIndex(i=>i?.role==="tool_activity"&&i.turnRunId===t.turnRunId&&i.gateRef===r);if(s>=0)return s}return-1}function U5(e,t){let a=tl(e.toolStatus),n=tl(t.toolStatus),r=a&&!n,s=t.gateActivity&&!e.gateActivity,i={...e,...t,id:e.id,role:"tool_activity",invocationId:e.gateActivity&&!t.gateActivity?t.invocationId:e.invocationId||t.invocationId,callId:e.gateActivity&&!t.gateActivity?t.callId:e.callId||t.callId,toolName:s?e.toolName:t.toolName||e.toolName,toolStatus:r?e.toolStatus:t.toolStatus,toolError:t.toolError||e.toolError,toolErrorKind:t.toolErrorKind||e.toolErrorKind||null,updatedAt:r?e.updatedAt||t.updatedAt:t.updatedAt||e.updatedAt,turnRunId:t.turnRunId||e.turnRunId||null,gateRef:t.gateRef||e.gateRef||null,gateActivity:e.gateActivity&&t.gateActivity,capabilityId:s?e.capabilityId||t.capabilityId||null:t.capabilityId||e.capabilityId||null,activityOrder:j5(e,t),activityOrderSource:t.activityOrderSource||e.activityOrderSource||null};return e.gateActivity&&!t.gateActivity&&(i.id=VS(t),i.gateActivity=!1),i}function j5(e,t){return Number.isFinite(t.activityOrder)?t.activityOrder:e.activityOrder}function F5(e,t){if(!e?.invocationId)return e;if(tl(e.toolStatus))return Rh(e,t),e;let a=t?.current?.terminalByInvocation?.get(e.invocationId);return a?Number.isFinite(e.activityOrder)?{...a,activityOrder:e.activityOrder,activityOrderSource:e.activityOrderSource||a.activityOrderSource||null}:a:e}function Rh(e,t){!e?.invocationId||!tl(e.toolStatus)||t?.current?.terminalByInvocation?.set(e.invocationId,e)}function B5(e){let t=al(e.toolName||e.capabilityId);return{...e,toolName:t||e.toolName||"tool"}}function WS({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o,onRunSettled:l}){let c=p.default.useRef(new Set),d=p.default.useRef(null),m=p.default.useRef(null);return p.default.useCallback(f=>{let{type:h,frame:x}=f||{};if(!(!h||!x))switch(h){case"accepted":{let y=x.ack||{};y.run_id&&(d.current=y.run_id),r?.({runId:y.run_id||null,threadId:y.thread_id||e,status:y.status||null}),a(!0);return}case"running":case"capability_progress":{let y=x.progress||{};y.turn_run_id&&(d.current=y.turn_run_id,r?.($=>$&&$.runId===y.turn_run_id?{...$,status:"running"}:{runId:y.turn_run_id,threadId:e,status:"running"}),z5(n,y.turn_run_id,m)),a(!0);return}case"capability_activity":{let y=x.activity;if(!y||!y.invocation_id)return;xi(t,Zp(y),o);return}case"capability_display_preview":{let y=x.preview;if(!y||!y.invocation_id)return;let $=Wp(y);xi(t,$,o);return}case"gate":case"auth_required":{let y=BS(h,x.prompt);y&&(kh(t,y,o),n(y),r?.({runId:y.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let y=x.reply||{};t($=>[...$,{id:`reply-${y.turn_run_id||Date.now()}`,role:"assistant",content:y.text||"",timestamp:y.generated_at||new Date().toISOString(),turnRunId:y.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let y=x.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),ad(c,l,y,!1);return}case"failed":{let y=x.run_state||{},$=y.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Eh(t,{runId:$,status:y.status||"failed",failureCategory:K5(y),failureSummary:null}),ad(c,l,$,!1);return}case"projection_snapshot":case"projection_update":{let y=x.state?.items||[];I5({items:y,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:l,settledRunsRef:c,latestRunIdRef:d,promptRunIdRef:m,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i,o,l])}function ad(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var GS=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),YS=new Set(["completed","succeeded"]),ed=new Set(["blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]),td=new Set(["awaiting_gate","blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]);function JS(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function z5(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function q5(e,t,a,n,r,s){let i=t?.runId||null;if(!i||r?.has(i))return!0;let o=a?.get(i);if(o)return!td.has(o);let l=e?.current,c=l?.runId||n?.current||null;if(c&&i!==c)return!0;let d=s?.current===c;return c&&i===c&&!d&&l?.status&&!td.has(l.status)?!0:!l?.runId||!l.status?!1:!td.has(l.status)}function I5({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:l,promptRunIdRef:c,activeRunRef:d,locallyResolvedGatesRef:m,toolActivityStateRef:f}){let h=new Map,x=new Set,y=d?.current||null,$=y?.runId||l?.current||null;for(let v of e){let b=v.run_status;b?.run_id&&b.status&&(h.set(b.run_id,b.status),$&&$!==b.run_id&&y?.status&&!GS.has(y.status)&&ed.has(b.status)&&x.add(b.run_id))}let g=l?.current??null;for(let v of e){if(v.run_status){let{run_id:b,status:w,failure_category:S,failure_summary:C}=v.run_status,R=GS.has(w),_=d?.current?.source==="local"?d.current.runId:null,A=!!(b&&_&&_!==b),L=g??l?.current??null,U=!!(R&&b&&L&&L!==b),F=b&&ed.has(w)?XS(m,b):null;if(b&&x.has(b)||A)continue;if(U){XS(m,d?.current?.runId)?.outcome==="resumed"&&(H5({runId:b,activePromptRunId:d?.current?.runId,success:YS.has(w),status:w,failureCategory:S,failureSummary:C,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:l,promptRunIdRef:c,locallyResolvedGatesRef:m}),g=null);continue}if(F){JS(r,b,c),F.outcome==="resumed"?(n(!0),s?.(B=>B&&B.runId===b?{...B,status:B.status==="awaiting_gate"?"queued":B.status||"queued"}:{runId:b,threadId:t,status:"queued"}),g=b,l&&(l.current=b)):(n(!1),d?.current?.runId===b&&s?.(null),g=null,l?.current===b&&(l.current=null));continue}b&&(g=b,!R&&l&&(l.current=b),s?.(B=>B&&B.runId===b?{...B,status:w}:{runId:b,threadId:t,status:w})),b&&ed.has(w)?c&&(c.current=b):b&&c?.current===b&&(c.current=null),R?(n(!1),r(null),s?.(null),Ch(m,b),g=null,l&&(l.current=null),b&&c?.current===b&&(c.current=null),ad(o,i,b,YS.has(w)),(w==="failed"||w==="recovery_required")&&Eh(a,{runId:b,status:w,failureCategory:S,failureSummary:C})):ed.has(w)||(JS(r,b,c),Ch(m,b),n(!0))}if(v.text){let b=`text-${v.text.id}`;a(w=>{let S=v.text.id?`msg-${v.text.id}`:null,C=w.findIndex(_=>_.id===b||S&&_.id===S),R={...C>=0?w[C]:{},id:b,role:"assistant",content:v.text.body||"",timestamp:w[C]?.timestamp||new Date().toISOString(),isFinalReply:!0};if(C>=0){let _=[...w];return _[C]=R,_}return[...w,R]}),n(!1)}if(v.thinking){let b=`thinking-${v.thinking.id}`;a(w=>{let S=w.findIndex(R=>R.id===b),C={id:b,role:"thinking",content:v.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:v.thinking.run_id||null};if(S>=0){let R=[...w];return R[S]=C,R}return[...w,C]})}if(v.capability_activity){let b=v.capability_activity;b.invocation_id&&xi(a,Zp(b),f)}if(v.gate){let b=zS(v.gate),w=b?.runId||null;w&&!q5(d,b,h,l,x,c)&&!V5(m,w,b.gateRef)&&(kh(a,b,f),r(S=>S||b),s?.(S=>S&&S.runId===w?{...S,status:td.has(S.status)?S.status:"awaiting_gate"}:{runId:w,threadId:t,status:"awaiting_gate"}),c&&(c.current=w),n(!1))}if(v.skill_activation){let{id:b,skill_names:w=[],feedback:S=[]}=v.skill_activation;if(w.length||S.length){let C=`skill-${b||w.join("-")||"activation"}`,R=[w.length?`Skill activated: ${w.join(", ")}`:"",...S].filter(Boolean).join(`
`);a(_=>_.some(A=>A.id===C)?_:[..._,{id:C,role:"system",content:R,timestamp:new Date().toISOString()}])}}}l&&g&&(l.current=g)}function H5({runId:e,activePromptRunId:t,success:a,status:n,failureCategory:r,failureSummary:s,setMessages:i,setIsProcessing:o,setPendingGate:l,setActiveRun:c,onRunSettled:d,settledRunsRef:m,latestRunIdRef:f,promptRunIdRef:h,locallyResolvedGatesRef:x}){o(!1),l(null),c?.(null),Ch(x,t),f&&(f.current=null),h?.current===t&&(h.current=null),ad(m,d,e,a),(n==="failed"||n==="recovery_required")&&Eh(i,{runId:e,status:n,failureCategory:r,failureSummary:s})}function K5(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function Eh(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`;e(i=>{let o=i.findIndex(c=>c.id===s),l=qS({status:a,failureCategory:n,failureSummary:r});if(o>=0){if(!!!(r||n)||i[o].content===l)return i;let d=[...i];return d[o]={...d[o],content:l},d}return[...i,{id:s,role:"error",content:l,timestamp:new Date().toISOString()}]})}function XS(e,t){if(!t)return null;let a=e?.current;if(!a)return null;for(let[n,r]of a.entries())if(n.startsWith(`${t}
`))return Q5(r);return null}function Q5(e){return e&&typeof e=="object"?{resolution:e.resolution||null,outcome:e.outcome||null}:{resolution:e||null,outcome:null}}function Ch(e,t){if(!t)return;let a=e?.current;if(a)for(let n of Array.from(a.keys()))n.startsWith(`${t}
`)&&a.delete(n)}function V5(e,t,a){return!t||!a?!1:!!e?.current?.has(`${t}
${a}`)}function ZS(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function e2(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function t2(e,t,a,n){let r=Th(n);return r?(G5(e,t,a,{timelineMessageId:r}),r):null}function G5(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function Th(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var Y5=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function a2({threadId:e,onEvent:t,enabled:a}){let[n,r]=p.default.useState("idle"),s=p.default.useRef(t);s.current=t;let i=p.default.useRef(null);return p.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,l=null,c=0,d=3e4;function m(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=h$({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let y=Math.min(1e3*2**c,d);l=setTimeout(m,y)};let x=(y,$)=>{let g=null;try{g=JSON.parse(y.data)}catch{return}!g||typeof g!="object"||(y.lastEventId&&(i.current=y.lastEventId),s.current?.({type:g.type||$,frame:g,lastEventId:y.lastEventId||null}))};o.onmessage=y=>x(y,"message");for(let y of Y5)o.addEventListener(y,$=>x($,y))}function f(){l&&(clearTimeout(l),l=null),o&&(o.close(),o=null),r("paused")}function h(){document.visibilityState==="hidden"?f():o||m()}return m(),document.addEventListener("visibilitychange",h),()=>{document.removeEventListener("visibilitychange",h),l&&clearTimeout(l),o&&o.close()}},[a,e]),{status:n}}var J5=3e4,X5="credential_stored_gate_resolution_failed",W5="approval_gate_pending_send_blocked",Z5="ironclaw-product-auth",Ah="ironclaw:product-auth:oauth-complete",eD="ironclaw:product-auth:oauth-complete";async function n2(e){let t=new AbortController,a=setTimeout(()=>t.abort(),J5);try{return await e(t.signal)}finally{clearTimeout(a)}}function tD(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=X5,t.cause=e,t}function aD(){let e=new Error("Resolve the approval request before sending another message.");return e.safeErrorCode=W5,e}function nD(e){let a=Dt.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function r2(e,t){return!e||!t?.runId||!t?.gateRef?null:`${e}
${t.runId}
${t.gateRef}`}function rD(e){return e?.continuation?.type==="turn_gate_resume"}function sD(e){if(e?.outcome)return e.outcome;let t=String(e?.status||"").toLowerCase();return t==="queued"||t==="running"?"resumed":t==="cancelled"||e?.already_terminal===!0?"cancelled":e?.already_terminal===!1?"resumed":null}function s2(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function iD(e){return e?.type===eD&&e?.status==="completed"}function oD(e,t,a){if(!iD(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function Dh(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function lD(e){if(!_h(e))return null;try{let a=(await Dt.fetchQuery({queryKey:["connectable-channels"],queryFn:Wc}))?.channels||[];return US(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function i2(e){let t=p.default.useRef(e),a=p.default.useRef(new Map),n=p.default.useRef(1),[r,s]=p.default.useState(0),[i,o]=p.default.useState(Date.now()),[l,c]=p.default.useState(null),d=p.default.useRef(l),m=p.default.useCallback(X=>{let re=typeof X=="function"?X(d.current):X;d.current=re,c(re)},[]);p.default.useEffect(()=>{d.current=l},[l]);let[f,h]=p.default.useState(null),x=p.default.useCallback(()=>a.current.get(e||"__new__")||[],[e]),y=p.default.useCallback(X=>{let re=e||"__new__";X.length>0?a.current.set(re,X):a.current.delete(re)},[e]),{messages:$,hasMore:g,nextCursor:v,isLoading:b,loadError:w,loadHistory:S,seedThreadMessages:C,setMessages:R}=I$(e,{getPendingMessages:x,setPendingMessages:y}),[_,A]=p.default.useState(!1),L=p.default.useRef(_),U=p.default.useCallback(X=>{let re=typeof X=="function"?X(L.current):X;L.current=re,A(re)},[]),[F,B]=p.default.useState(null),P=p.default.useRef(F),[G,ae]=p.default.useState(null),le=p.default.useCallback(X=>{let re=P.current,ie=typeof X=="function"?X(re):X;Object.is(ie,re)||(P.current=ie,B(ie))},[]),[lt,ht]=p.default.useState(e),Oe=p.default.useRef(IS()),De=p.default.useRef(new Map),at=p.default.useRef({gateKey:null,credentialRef:null,inFlight:!1}),$t=p.default.useRef(!1),Le=p.default.useRef(null);lt!==e&&(ht(e),A(!1),B(null),ae(null),c(null),h(null)),p.default.useEffect(()=>{t.current=e},[e]),p.default.useEffect(()=>()=>{Le.current?.threadId===e&&(Le.current=null)},[e]),p.default.useEffect(()=>{P.current=F},[F]),p.default.useEffect(()=>{L.current=_},[_]),p.default.useEffect(()=>{let X=r2(e,F);ae(re=>re&&re.gateKey!==X?null:re)},[F,e]),p.default.useEffect(()=>{HS(Oe),De.current.clear()},[e]);let Pa=Math.max(0,Math.ceil((r-i)/1e3)),kt=F?.runId&&F?.gateRef?`${F.runId}
${F.gateRef}`:null;p.default.useEffect(()=>{if(!r)return;let X=setInterval(()=>o(Date.now()),250);return()=>clearInterval(X)},[r]),p.default.useEffect(()=>{at.current.gateKey!==kt&&(at.current={gateKey:kt,credentialRef:null,inFlight:!1})},[kt]),p.default.useEffect(()=>{if(!s2(F))return;let X=Date.now(),re=M=>{oD(M,F,X)&&(le(q=>s2(q)?null:q),U(!0))},ie=null;typeof window.BroadcastChannel=="function"&&(ie=new window.BroadcastChannel(Z5),ie.onmessage=M=>re(M.data));let N=M=>{M.key===Ah&&re(Dh(M.newValue))};window.addEventListener("storage",N),re(Dh(window.localStorage?.getItem?.(Ah)));let E=window.setInterval(()=>{re(Dh(window.localStorage?.getItem?.(Ah)))},500);return()=>{window.clearInterval(E),ie&&ie.close(),window.removeEventListener("storage",N)}},[F]);let la=WS({threadId:e,setMessages:R,setIsProcessing:U,setPendingGate:le,setActiveRun:m,activeRunRef:d,locallyResolvedGatesRef:De,toolActivityStateRef:Oe,onRunSettled:(X,{success:re})=>{let ie=Le.current;ie?.runId===X?Le.current=null:X&&ie&&!ie.runId&&(Le.current={...ie,runId:X,settledBeforeResponse:!0}),re&&y([]),S(void 0,{preserveClientOnly:!0,finalReplyTimestampByRun:X&&re?{[X]:new Date().toISOString()}:null})}}),{status:rn}=a2({threadId:e,onEvent:la,enabled:!!e}),ua=p.default.useCallback(async(X,re={})=>{let{threadId:ie,attachments:N=[],displayContent:E}=re,M=N.map(O$),q=N.map(L$),z=typeof E=="string"?E:X;if(F||P.current)throw aD();let O=ie||e,Q=d.current,ce=!!Q&&!!O&&Q.threadId===O,ge=L.current&&!!O&&O===e,gt=!!O&&Le.current?.threadId===O;if($t.current||ge||ce||gt)return null;if(N.length===0){let oe=await lD(X);if(oe)return h(oe),{channel_connect_action:oe}}h(null);let Ce=ie||e;if(!Ce){let oe=await Sc();if(Dt.invalidateQueries({queryKey:["threads"]}),Ce=oe?.thread?.thread_id,!Ce)throw new Error("createThread returned no thread_id")}let Ct=Ce,on={id:`pending-${n.current++}`,role:"user",content:z,attachments:q,retryContent:X,retryDisplayContent:z,retryAttachments:N,timestamp:new Date().toISOString(),isOptimistic:!0},ja={id:on.id,role:"user",content:z,attachments:q,retryContent:X,retryDisplayContent:z,retryAttachments:N,timestamp:on.timestamp,isOptimistic:!0};ZS(a.current,Ct,on);let Fa=on.id,yr=!e||Ce===e,br=oe=>{yr&&R(oe)},Zr=oe=>{Ce!==e&&C(Ce,oe)},es=oe=>{yr&&oe()},ts=yr;ts&&(Le.current={threadId:Ce,runId:null,settledBeforeResponse:!1}),$t.current=!0,br(oe=>[...oe,ja]),Zr(oe=>[...oe,ja]),es(()=>{U(!0),P.current||le(null)});try{let oe=await m$({threadId:Ce,content:X,attachments:M});nD(Ce)&&Dt.invalidateQueries({queryKey:["threads"]});let as=!1;if(oe?.run_id&&ts){let Lt=Le.current;as=!!(Lt&&Lt.threadId===Ce&&Lt.runId===oe.run_id&&Lt.settledBeforeResponse),as?Le.current=null:Le.current={threadId:Ce,runId:oe.run_id,settledBeforeResponse:!1}}else ts&&(Le.current=null);oe?.run_id&&yr&&!as&&m({runId:oe.run_id,threadId:oe.thread_id||Ce,status:oe.status||null,source:"local"});let $l=t2(a.current,Ct,Fa,oe?.accepted_message_ref)||Th(oe?.accepted_message_ref);if($l){let Lt=ns=>ns.map(Dn=>Dn.id===Fa?{...Dn,timelineMessageId:$l}:Dn);br(Lt),Zr(Lt)}if(oe?.outcome==="rejected_busy"){ts&&(Le.current=null);let Lt=ns=>ns.map(Dn=>Dn.id===Fa?{...Dn,isOptimistic:!1,status:"error"}:Dn);if(br(Lt),Zr(Lt),oe?.notice){let ns=(Li=yr)=>{let Ok={id:`system-rejected-${n.current++}`,role:"system",content:oe.notice,timestamp:new Date().toISOString(),isOptimistic:!1},lv=Lk=>[...Lk,Ok];Li&&R(lv),(!Li||Ce!==e)&&C(Ce,lv)};if(!t.current||t.current===Ce){let Li=r2(Ce,P.current);Li?ae({gateKey:Li,content:oe.notice}):ns()}else ns(!1)}es(()=>U(!1)),$t.current=!1}else oe?.run_id||(ts&&(Le.current=null),$t.current=!1);return oe}catch(oe){ts&&(Le.current=null),oe.status===429&&s(Date.now()+cD(oe));let as=$l=>$l.map(Lt=>Lt.id===Fa?{...Lt,isOptimistic:!1,status:"error",error:oe.message}:Lt);throw br(as),Zr(as),es(()=>U(!1)),$t.current=!1,oe&&typeof oe=="object"&&(oe.optimisticMessageId=Fa,oe.optimisticThreadId=Ce),oe}finally{$t.current=!1,e2(a.current,Ct,Fa)}},[e,F,R,C,U,le,m]),Vt=p.default.useCallback(async(X,re={})=>{if(!F)return;let{runId:ie,gateRef:N}=F;if(!ie||!N)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");let E=await Vp({threadId:e,runId:ie,gateRef:N,resolution:X,always:re.always,credentialRef:re.credentialRef}),M=sD(E);if(De.current.set(`${ie}
${N}`,{resolution:X,outcome:M}),uD(X)&&M==="resumed"&&KS(R,F,Oe),le(null),M==="resumed"){U(!0),m({runId:E?.run_id||ie,threadId:E?.thread_id||e,status:E?.status||"queued"});return}U(!1),m(null)},[F,e,R,m]),sn=p.default.useCallback(async X=>{if(!F)throw new Error("auth gate is no longer pending");let{runId:re,gateRef:ie,provider:N}=F;if(!re||!ie||!N)throw new Error("auth gate is missing required credential metadata");let E=F.accountLabel||`${N} credential`,M=`${re}
${ie}`;if(at.current.gateKey!==M&&(at.current={gateKey:M,credentialRef:null,inFlight:!1}),at.current.inFlight)throw new Error("auth token submission already in progress");at.current.inFlight=!0;try{let q=at.current.credentialRef,z=null;if(!q){if(z=await n2(O=>g$({provider:N,accountLabel:E,token:X,threadId:e,runId:re,gateRef:ie,signal:O})),q=z?.credential_ref,!q)throw new Error("manual token submit returned no credential_ref");at.current.credentialRef=q}if(!rD(z))try{await n2(O=>Vp({threadId:e,runId:re,gateRef:ie,resolution:"credential_provided",credentialRef:q,signal:O}))}catch(O){throw tD(O)}at.current={gateKey:null,credentialRef:null,inFlight:!1},le(null),U(!0)}catch(q){throw at.current.gateKey===M&&(at.current.inFlight=!1),q}},[F,e]),vt=p.default.useCallback(async X=>{let re=l?.runId;if(!re||!e)return;le(null),U(!1),m(null),$t.current=!1;let ie=Le.current;(ie?.runId===re||ie?.threadId===e)&&(Le.current=null),await v$({threadId:e,runId:re,reason:X})},[l,e]),ca=p.default.useCallback(()=>{g&&v&&S(v)},[g,v,S]),_a=p.default.useCallback(async(X,re,ie)=>{let N="approved",E=!1;re==="deny"?N="denied":re==="cancel"?N="cancelled":re==="always"&&(N="approved",E=!0),await Vt(N,{always:E})},[Vt]),da=p.default.useCallback(()=>{},[]),Ua=p.default.useCallback(async X=>{if(!X||X.status!=="error")return;let re=typeof X.retryContent=="string"?X.retryContent:typeof X.content=="string"?X.content:"",ie=Array.isArray(X.retryAttachments)?X.retryAttachments:[];if(!re&&ie.length===0)return;let N=M=>M.filter(q=>q.id!==X.id),E=M=>M.some(z=>z.id!==X.id&&z.role==="user"&&z.status==="error"&&z.retryContent===re)||M.some(z=>z.id===X.id)?M:[...M,X];R(N),e&&C(e,N);try{await ua(re,{threadId:e,attachments:ie,displayContent:typeof X.retryDisplayContent=="string"?X.retryDisplayContent:X.content})===null&&(R(E),e&&C(e,E))}catch(M){if(M?.optimisticMessageId){R(N),e&&C(e,N);return}R(E),e&&C(e,E)}},[ua,C,R,e]);return{messages:$,isProcessing:_,pendingGate:F,busyGateNotice:G,channelConnectAction:f,activeRun:l,sseStatus:rn,historyLoading:b,historyLoadError:w,hasMore:g,cooldownSeconds:Pa,send:ua,resolveGate:Vt,submitAuthToken:sn,cancelRun:vt,loadMore:ca,dismissChannelConnectAction:()=>h(null),suggestions:[],setSuggestions:da,retryMessage:Ua,approve:_a,recoverHistory:da,recoveryNotice:null}}function uD(e){return e==="denied"||e==="cancelled"}function cD(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function o2({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}function dD(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function nd({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let l=o.toString(),c=`/logs${l?`?${l}`:""}`;return i?`/v2${c}`:c}function l2(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(dD),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}var mD=1500;function u2({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let o=k(),{messages:l,isProcessing:c,pendingGate:d,busyGateNotice:m,channelConnectAction:f,suggestions:h,sseStatus:x,historyLoading:y,historyLoadError:$,hasMore:g,cooldownSeconds:v,recoveryNotice:b,activeRun:w,send:S,cancelRun:C,retryMessage:R,approve:_,recoverHistory:A,loadMore:L,setSuggestions:U,submitAuthToken:F,dismissChannelConnectAction:B}=i2(t),P=p.default.useMemo(()=>e.find(vt=>vt.id===t)||null,[e,t]),G=p.default.useMemo(()=>o2({gatewayStatus:i,activeThread:P}),[i,P]),ae=!!t&&!!d,le=!!t&&c,lt=l.length>0||le||ae||!!f,ht=!y&&!lt&&!$,Oe=ae?"Resolve the approval request before sending another message.":"",De=ae||le&&!ae||v>0,at=p.default.useRef(De);at.current=De;let $t=Oe||(v>0?`Retry in ${v}s`:void 0),Le=t||sl,Pa=!!(t&&w?.runId&&w.threadId===t&&le&&!ae),kt=t&&w?.runId&&w.threadId===t?nd({threadId:t,runId:w.runId},{absolute:!0}):null,la=p.default.useCallback(async(vt,{images:ca=[],attachments:_a=[],displayContent:da}={})=>{if(ae)throw new Error(Oe);if(at.current)return null;let Ua=await S(vt,{images:ca,attachments:_a,displayContent:da,threadId:t}),X=Ua?.thread_id||t;return!t&&X&&a&&a(X,{replace:!0}),Ua},[t,ae,Oe,De,a,S]),rn=p.default.useCallback(async vt=>{De||(U([]),await la(vt))},[De,la,U]),ua=p.default.useCallback(()=>C("user_requested"),[C]);p.default.useEffect(()=>{if(!t)return;if(d){Pc(t,Na.NEEDS_ATTENTION);return}if(c){Pc(t,Na.RUNNING);return}let vt=setTimeout(()=>Zw(t),mD);return()=>clearTimeout(vt)},[t,d,c]);let[Vt,sn]=p.default.useState(!1);return p.default.useEffect(()=>{let vt=ca=>{if(ca.key==="Escape"){sn(!1);return}if(ca.key!=="?")return;let _a=ca.target,da=_a?.tagName;da==="INPUT"||da==="TEXTAREA"||_a?.isContentEditable||(ca.preventDefault(),sn(Ua=>!Ua))};return window.addEventListener("keydown",vt),()=>window.removeEventListener("keydown",vt)},[]),u`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${nS} status=${x} />

        ${c&&!d&&kt&&u`
          <div className="flex justify-end border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-1.5">
            <${Rn}
              to=${kt}
              className="inline-flex h-8 items-center gap-1.5 rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
              title=${o("nav.logs")}
            >
              <${D} name="list" className="h-3.5 w-3.5" />
              ${o("nav.logs")}
            <//>
          </div>
        `}

        ${$&&u`
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            ${$}
          </div>
        `}

        ${ht&&u`
          <${rS}
            onSuggestion=${rn}
            onSend=${la}
            disabled=${!1}
            sendDisabled=${De}
            initialText=${r}
            resetKey=${s}
            draftKey=${Le}
            context=${G}
            statusText=${$t}
            canCancel=${Pa}
            onCancel=${ua}
          />
        `}
        ${!ht&&u`
          <${MS}
            messages=${l}
            isLoading=${y}
            hasMore=${g}
            onLoadMore=${L}
            onRetryMessage=${R}
            threadId=${t}
            pending=${le}
          >
            ${b&&u`
              <${OS}
                notice=${b}
                onRecover=${A}
              />
            `}
            ${le&&!ae&&u`<${PS} />`}
            ${f&&u`
              <${eS}
                connectAction=${f}
                onDismiss=${B}
              />
            `}
            ${d&&(d.kind==="auth_required"?d.challengeKind==="oauth_url"?u`
                  <${J1}
                    gate=${d}
                    onCancel=${()=>_(d.requestId,"cancel",d.kind)}
                  />
                `:d.challengeKind==="manual_token"?u`
                  <${X1}
                    gate=${d}
                    onSubmit=${F}
                    onCancel=${()=>_(d.requestId,"cancel",d.kind)}
                  />
                `:u`
                  <${Y1}
                    gate=${d}
                    onCancel=${()=>_(d.requestId,"cancel",d.kind)}
                  />
                `:u`
              <${G1}
                gate=${d}
                onApprove=${()=>_(d.requestId,"approve",d.kind)}
                onDeny=${()=>_(d.requestId,"deny",d.kind)}
                onAlways=${()=>_(d.requestId,"always",d.kind)}
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

          <${LS}
            suggestions=${h}
            onSelect=${rn}
            disabled=${De}
          />

          <${Gc}
            onSend=${la}
            disabled=${!1}
            sendDisabled=${De}
            initialText=${r}
            resetKey=${s}
            draftKey=${Le}
            context=${G}
            statusText=${$t}
            canCancel=${Pa}
            onCancel=${ua}
          />
        `}
      </div>
      <${sS}
        open=${Vt}
        onClose=${()=>sn(!1)}
      />
    </div>
  `}function Mh(){let{threadsState:e,gatewayStatus:t}=wa(),{threadId:a}=it(),n=ve(),r=Ae(),s=r.state?.composerDraft||"",i=a||null;p.default.useEffect(()=>{i&&i!==e.activeThreadId?e.setActiveThreadId(i):i||e.setActiveThreadId(null)},[i]);let o=p.default.useCallback((l,c={})=>{if(!l){e.setActiveThreadId(null),n("/chat",c);return}e.setActiveThreadId(l),n(`/chat/${l}`,c)},[e,n]);return u`
    <${u2}
      threads=${e.threads}
      activeThreadId=${i}
      onSelectThread=${o}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function c2(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?ui(e,t):"",model:e?Oc(e,t):""}}function d2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:l}){let[c,d]=p.default.useState(()=>c2(e,a)),[m,f]=p.default.useState(""),[h,x]=p.default.useState([]),[y,$]=p.default.useState(null),[g,v]=p.default.useState(""),b=p.default.useRef(!!e);p.default.useEffect(()=>{n&&(d(c2(e,a)),f(""),x([]),$(null),v(""),b.current=!!e)},[n,e,a]);let w=e?.builtin===!0,S=e&&!e.builtin,C=p.default.useCallback((U,F)=>{d(B=>{let P={...B,[U]:F};return U==="name"&&!b.current&&(P.id=Ew(F)),P})},[]),R=p.default.useCallback(()=>!w&&(!c.name.trim()||!c.id.trim())?l("llm.fieldsRequired"):!w&&!Tw(c.id.trim())?l("llm.invalidId"):!S&&!w&&t.includes(c.id.trim())?l("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,w,S,l]),_=p.default.useCallback(async()=>{let U=R();if(U){$({tone:"error",text:U});return}v("save");try{await s({form:c,apiKey:m,provider:e}),r()}catch(F){$({tone:"error",text:F.message})}finally{v("")}},[m,c,r,s,e,R]),A=p.default.useCallback(async()=>{if(!c.model.trim()){$({tone:"error",text:l("llm.modelRequired")});return}v("test");try{let U=await i(oh(e,c,m,a));$({tone:U.ok?"success":"error",text:U.message})}catch(U){$({tone:"error",text:U.message})}finally{v("")}},[m,a,c,i,e,l]),L=p.default.useCallback(async()=>{if((w?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){$({tone:"error",text:l("llm.baseUrlRequired")});return}v("models");try{let F=await o(oh(e,c,m,a));if(!F.ok||!Array.isArray(F.models)||!F.models.length)$({tone:"error",text:F.message||l("llm.modelsFetchFailed")});else{x(F.models);let B=Aw(c.model,F.models);B!==null&&C("model",B),$({tone:"success",text:l("llm.modelsFetched",{count:F.models.length})})}}catch(F){$({tone:"error",text:F.message})}finally{v("")}},[m,a,c,w,o,e,l,C]);return{form:c,apiKey:m,models:h,message:y,busy:g,isBuiltin:w,isEditing:S,setApiKey:f,update:C,submit:_,runTest:A,fetchModels:L,markIdEdited:()=>{b.current=!0}}}function rd({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let l=k(),c=d2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:l});if(!n)return null;let{form:d,apiKey:m,models:f,message:h,busy:x,isBuiltin:y,isEditing:$}=c,g=y?l("llm.configureProvider",{name:e.name||e.id}):l($?"llm.editProvider":"llm.newProvider");return u`
    <${gi} open=${n} onClose=${r} title=${g} size="lg">
      <${yi} className="space-y-4">
        ${!y&&u`
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${l("llm.providerName")}
              <${Ot} value=${d.name} onChange=${v=>c.update("name",v.target.value)} />
            </label>
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${l("llm.providerId")}
              <${Ot}
                value=${d.id}
                disabled=${$}
                onChange=${v=>{c.markIdEdited(),c.update("id",v.target.value)}}
              />
            </label>
          </div>
          <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
            ${l("llm.adapter")}
            <${xh} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${ih.map(v=>u`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
            <//>
          </label>
        `}

        ${y&&u`
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            ${ll(e.adapter)}
          </div>
        `}

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${l("llm.baseUrl")}
          <${Ot} value=${d.baseUrl} placeholder=${e?.base_url||""} onChange=${v=>c.update("baseUrl",v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${l("llm.apiKey")}
          <${Ot} type="password" value=${m} placeholder=${l("llm.apiKeyPlaceholder")} onChange=${v=>c.setApiKey(v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${l("llm.defaultModel")}
          <div className="flex items-stretch gap-2">
            <${Ot} value=${d.model} onChange=${v=>c.update("model",v.target.value)} />
            <${T} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${x!==""} onClick=${c.fetchModels}>
              ${l(x==="models"?"llm.fetchingModels":"llm.fetchModels")}
            <//>
          </div>
        </label>

        ${f.length>0&&u`
          <${xh} value=${d.model} onChange=${v=>c.update("model",v.target.value)}>
            ${f.map(v=>u`<option key=${v} value=${v}>${v}</option>`)}
          <//>
        `}

        ${h&&u`
          <div className=${h.tone==="error"?"text-sm text-red-200":"text-sm text-mint"} role="status">
            ${h.text}
          </div>
        `}
      <//>
      <${bi}>
        <${T} type="button" variant="secondary" disabled=${x!==""} onClick=${c.runTest}>
          ${l(x==="test"?"llm.testing":"llm.testConnection")}
        <//>
        <${T} type="button" variant="ghost" disabled=${x!==""} onClick=${r}>${l("common.cancel")}<//>
        <${T} type="button" disabled=${x!==""} onClick=${c.submit}>
          ${l(x==="save"?"common.saving":"common.save")}
        <//>
      <//>
    <//>
  `}function sd({login:e}){let t=k(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return u`
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
  `}function fD(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function id({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=ci({settings:e,gatewayStatus:t}),[s,i]=p.default.useState(null),[o,l]=p.default.useState(!1),[c,d]=p.default.useState(null),m=p.default.useRef(null),f=p.default.useCallback((g,v)=>{m.current&&window.clearTimeout(m.current),d({tone:g,text:v}),m.current=window.setTimeout(()=>d(null),3500)},[]);p.default.useEffect(()=>()=>{m.current&&window.clearTimeout(m.current)},[]);let h=p.default.useCallback((g=null)=>{i(g),l(!0)},[]),x=p.default.useCallback(async g=>{try{await r.setActiveProvider(g),f("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(h(g),f("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):f("error",v.message)}},[h,r,f,n]),y=p.default.useCallback(async({form:g,apiKey:v,provider:b})=>{if(b?.builtin){await r.saveBuiltinProvider({provider:b,form:g,apiKey:v}),f("success",n("llm.providerConfigured",{name:b.name||b.id}));return}let w=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:b});f("success",n(b?"llm.providerUpdated":"llm.providerAdded",{name:w.name||w.id}))},[r,f,n]),$=p.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),f("success",n("llm.providerDeleted"))}catch(v){f("error",v.message)}},[r,f,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>fD(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:h,closeDialog:()=>l(!1),handleUse:x,handleSave:y,handleDelete:$}}var pD=3e5;function hD(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function vD(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function gD(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=l=>{let c=l.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},pD);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var yD=3e5,bD=9e5,xD=2e3;async function m2(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,xD)),(await Mc().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function od({onSuccess:e}={}){let t=k(),a=Z(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),[o,l]=p.default.useState(!1),[c,d]=p.default.useState(""),[m,f]=p.default.useState(null),h=p.default.useCallback(()=>{i(""),d(""),f(null)},[]),x=p.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),y=p.default.useCallback(async v=>{if(h(),hD()){i(t("onboarding.nearaiLocalSso"));return}let b=window.open("about:blank","_blank");if(!b){i(t("onboarding.nearaiFailed"));return}try{b.opener=null}catch{}r(!0);try{let{auth_url:w}=await ow({provider:v,origin:window.location.origin});b.location.href=w;let S=await m2("nearai",yD,b);if(S==="active"){await x();return}b.close(),i(t(S==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{b.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,h,t]),$=p.default.useCallback(async()=>{h(),r(!0);try{let v=vD(),b=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!b){i(t("onboarding.nearaiFailed"));return}b.opener=null;let w=await gD(b,v);if(!w){i(t("onboarding.nearaiFailed"));return}await lw({account_id:w.accountId,public_key:w.publicKey,signature:w.signature,message:w.message,recipient:w.recipient,nonce:w.nonce}),await x()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,h,t]),g=p.default.useCallback(async()=>{h();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}l(!0);try{let{user_code:b,verification_uri:w}=await uw();f({userCode:b,verificationUri:w}),v&&(v.location.href=w);let S=await m2("openai_codex",bD,v);if(S==="active"){await x();return}v&&v.close(),d(t(S==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{l(!1)}},[x,h,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:m,startNearai:y,startNearaiWallet:$,startCodex:g}}var f2="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",$D="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",wD="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",SD="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",ND={nearai:{color:"#00ec97",path:$D},openai_codex:{color:"#10a37f",path:f2},openai:{color:"#10a37f",path:f2},anthropic:{color:"#d97757",path:wD},ollama:{color:null,path:SD}};function p2({id:e,name:t}){let a=ND[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return u`
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
  `}var _D=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function RD({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=p.default.useState(!1),o=p.default.useRef(null),l=t||a.nearaiBusy;p.default.useEffect(()=>{if(!s)return;let d=f=>{o.current&&!o.current.contains(f.target)&&i(!1)},m=f=>{f.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",m),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",m)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return u`
    <div ref=${o} className="relative shrink-0">
      <${T}
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
        <${D} name="chevron" className="h-3.5 w-3.5" />
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
  `}function kD({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let l=s(e.nameKey),c;return e.auth==="nearai"?c=u`<${RD} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=u`
      <${T} type="button" variant="secondary" size="sm" disabled=${r.codexBusy} onClick=${r.startCodex}>
        ${s("onboarding.signIn")}
      <//>
    `:a?c=u`<${T} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>i(t)}>
      ${s("llm.use")}
    <//>`:c=u`<${T} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>o(t)}>
      ${s("onboarding.setUp")}
    <//>`,u`
    <${ne} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-4">
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <${p2} id=${e.id} name=${l} />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">${l}</span>
            ${a&&u`<${I} tone="positive" label=${s("onboarding.ready")} size="sm" />`}
          </div>
          <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">${s(e.descKey)}</div>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap gap-2 sm:justify-end">${c}</div>
    <//>
  `}function h2(){let{isAdmin:e=!1,isChecking:t=!1}=wa();return t?null:e?u`<${CD} />`:u`<${ot} to="/chat" replace />`}function CD(){let e=k(),t=ve(),a=Z(),{gatewayStatus:n}=wa(),r=id({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=_D.map(m=>({entry:m,provider:s.providers.find(f=>f.id===m.id)})).filter(m=>m.provider),o=p.default.useCallback(()=>t("/chat"),[t]),l=od({onSuccess:o}),c=p.default.useCallback(async m=>{let f=m.active_model||m.default_model||"";await ol({provider_id:m.id,model:f}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=p.default.useCallback(async({form:m,apiKey:f,provider:h})=>{await r.handleSave({form:m,apiKey:f,provider:h});let x=h?.id||m.id.trim(),y=m.model?.trim()||h?.default_model||"";await ol({provider_id:x,model:y}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?u`
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
              <${kD}
                key=${m.id}
                entry=${m}
                provider=${f}
                configured=${Vr(f,s.builtinOverrides)}
                isBusy=${s.isBusy}
                login=${l}
                t=${e}
                onUse=${c}
                onSetUp=${r.openDialog}
              />
            `)}
        </div>

        <${sd} login=${l} />

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

      <${rd}
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
  `}function H({children:e,className:t="",...a}){return u`<${ne} className=${t} ...${a}>${e}<//>`}function et({label:e,value:t,tone:a="muted",badgeLabel:n,detail:r,showDivider:s=!0,className:i="",valueClassName:o="text-[1.75rem] md:text-[2rem]"}){return u`
    <div
      className=${J("px-1 py-4",s&&"border-t border-[var(--v2-panel-border)]",i)}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div
            className="font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]"
          >
            ${e}
          </div>
          <div
            className=${J("mt-3 truncate font-medium tracking-[-0.05em] text-[var(--v2-text-strong)]",o)}
          >
            ${t}
          </div>
          ${r&&u`<div className="mt-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            ${r}
          </div>`}
        </div>
        <${I} tone=${a} label=${n??a} />
      </div>
    </div>
  `}function v2({items:e}){return u`
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
  `}function $e({title:e,description:t,children:a,boxed:n=!0}){let r=u`
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
  `;return n?u`<${ne} padding="lg">${r}<//>`:u`<div className="py-8">${r}</div>`}var g2={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function an({result:e,onDismiss:t}){return e?u`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",g2[e.type]||g2.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}var y2="",ED={workspace:"home"};function ld(e){return ED[e]||e}function hl(e){return[...e||[]].sort((t,a)=>t.is_dir!==a.is_dir?t.is_dir?-1:1:t.name.localeCompare(a.name,void 0,{sensitivity:"base"}))}function $i(e){return e?e.split("/").filter(Boolean):[]}function ud(e){return e?`/workspace/${$i(e).map(encodeURIComponent).join("/")}`:"/workspace"}function Oh(e){let t=$i(e);return t.pop(),t.join("/")}function b2(e){return/\.mdx?$/i.test(e||"")}function cd({path:e,onNavigate:t}){let a=k(),n=$i(e),r="";return u`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button
        type="button"
        onClick=${()=>t("/workspace")}
        className="text-signal hover:underline"
      >
        ${a("workspace.breadcrumbRoot")}
      </button>
      ${n.map((s,i)=>{r=r?`${r}/${s}`:s;let o=r,l=i===0?ld(s):s;return u`
          <span key=${o} className="text-iron-400">/</span>
          <button
            key=${`${o}-button`}
            type="button"
            onClick=${()=>t(ud(o))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${l}
          </button>
        `})}
    </div>
  `}function TD(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function x2({path:e,entries:t,isLoading:a,filter:n,onOpen:r,onNavigate:s}){let i=k();if(a)return u`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;let o=(t||[]).filter(f=>!TD(f.path)),l=String(n||"").trim().toLowerCase(),c=l?o.filter(f=>f.name.toLowerCase().includes(l)):o,d=hl(c),m;return o.length?d.length?m=u`
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
    <${H} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${cd} path=${e} onNavigate=${s} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">${m}</div>
    <//>
  `}var dd="/api/webchat/v2/fs",AD=1024*1024,DD=8*1024*1024;function $2(e){let t=String(e||"").split("/").filter(Boolean);return{mount:t.shift()||"",path:t.join("/")}}function MD(e,t){return t?`${e}/${t}`:e}function OD(e){let t=String(e||"").toLowerCase();return t.startsWith("text/")||t==="application/json"||t==="application/javascript"||t==="application/xml"||t.endsWith("+json")||t.endsWith("+xml")}function LD(e){return String(e||"").toLowerCase().startsWith("image/")}function PD(e){let t=String(e||"").toLowerCase();return t.startsWith("audio/")||t.startsWith("video/")||t.startsWith("font/")||t==="application/pdf"||t==="application/zip"||t==="application/gzip"}function UD(e){if(e.subarray(0,Math.min(e.length,8192)).indexOf(0)!==-1)return!0;try{return new TextDecoder("utf-8",{fatal:!0}).decode(e),!1}catch{return!0}}function jD(e,t){let a=new URL(`${dd}/content`,window.location.origin);return a.searchParams.set("mount",e),a.searchParams.set("path",t),a.pathname+a.search}async function FD(){return(await V(`${dd}/mounts`))?.mounts||[]}async function wi(e=""){if(!e)return{entries:(await FD()).map(o=>({name:ld(o.mount),path:o.mount,is_dir:!0}))};let{mount:t,path:a}=$2(e),n=new URL(`${dd}/list`,window.location.origin);return n.searchParams.set("mount",t),a&&n.searchParams.set("path",a),{entries:((await V(n.pathname+n.search))?.entries||[]).map(i=>({name:i.name,path:MD(t,i.path),is_dir:i.kind==="directory"}))}}async function w2(e){let{mount:t,path:a}=$2(e);if(!t||!a)return{kind:"directory",path:e};let n=new URL(`${dd}/stat`,window.location.origin);n.searchParams.set("mount",t),n.searchParams.set("path",a);let s=(await V(n.pathname+n.search))?.stat||{},i=s.mime_type||"application/octet-stream",o=Number(s.size_bytes||0),l=jD(t,a),c={path:e,mime:i,size_bytes:o,download_path:l};if(s.kind&&s.kind!=="file")return{...c,kind:"directory"};if(LD(i)){if(o>DD)return{...c,kind:"binary"};let h=await Rc(l);return{...c,kind:"image",image_data_url:h}}if(PD(i)||o>AD)return{...c,kind:"binary"};let d=await Aa(l),m=new Uint8Array(await d.arrayBuffer());if(!OD(i)&&UD(m))return{...c,kind:"binary"};let f=new TextDecoder("utf-8").decode(m);return{...c,kind:"text",content:f}}function S2(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function BD(e,t,a){let n=String(t||"").trim().toLowerCase(),r=(e||[]).filter(s=>!S2(s.path)).filter(s=>!n||s.name.toLowerCase().includes(n)?!0:s.is_dir&&a.has(s.path));return hl(r)}function N2({entry:e,depth:t,selectedPath:a,expandedPaths:n,filter:r,onToggleDirectory:s,onSelectFile:i}){let o=k(),l=n.has(e.path),c=K({queryKey:["workspace-list",e.path],queryFn:()=>wi(e.path),enabled:e.is_dir&&l});if(e.is_dir){let d=BD(c.data?.entries,r,n);return u`
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
                  <${N2}
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
  `}function _2({entries:e,selectedPath:t,expandedPaths:a,filter:n,onToggleDirectory:r,onSelectFile:s,isLoading:i}){let o=k();if(i)return u`<div className="space-y-2 p-3">${[1,2,3,4].map(c=>u`<div key=${c} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let l=hl(e.filter(c=>!S2(c.path)));return l.length?u`
    <div className="space-y-1 p-2">
      ${l.map(c=>u`
        <${N2}
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
  `:u`<div className="px-4 py-8 text-sm text-iron-300">${o("workspace.noFiles")}</div>`}function R2({rootEntries:e,selectedPath:t,expandedPaths:a,filter:n,onFilterChange:r,isLoadingTree:s,onToggleDirectory:i,onSelectFile:o}){let l=k();return u`
    <${H} className="flex min-h-[420px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="border-b border-white/10 p-3">
        <input
          value=${n}
          onInput=${c=>r(c.target.value)}
          placeholder=${l("workspace.filterPlaceholder")}
          className="h-9 w-full rounded-md border border-white/10 bg-iron-950/80 px-3 text-sm text-white outline-none placeholder:text-iron-400 focus:border-signal/45"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <${_2}
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
  `}function k2(e){return $i(e).pop()||"download"}function zD({path:e,file:t}){let a=k();return t.kind==="image"?u`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${t.image_data_url}
          alt=${k2(e)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `:t.kind==="text"?u`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${b2(e)?u`<${sa} content=${t.content} className="max-w-4xl text-base leading-7" />`:u`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
      </div>
    `:u`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${a("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `}function C2({path:e,file:t,isLoading:a,onNavigate:n}){let r=k(),[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(t?.download_path){i(!0);try{let c=await Aa(t.download_path);vi(c,k2(e))}catch{}finally{i(!1)}}},[t,e]);if(a)return u`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;if(!t||t.kind==="directory")return u`
      <${$e}
        title=${r("workspace.pickFileTitle")}
        description=${r("workspace.pickFileDesc")}
      />
    `;let l=r("workspace.fileMeta",{mime:t.mime||"application/octet-stream",size:Number(t.size_bytes||0)});return u`
    <${H} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${cd} path=${e} onNavigate=${n} />
        <div className="flex items-center gap-2">
          <${I} tone="muted" label=${l} />
          <${T}
            variant="secondary"
            size="sm"
            onClick=${o}
            disabled=${s}
          >${r("workspace.download")}<//>
        </div>
      </div>

      <${zD} path=${e} file=${t} />

      ${Oh(e)&&u`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${r("workspace.parent",{path:Oh(e)})}
        </div>
      `}
    <//>
  `}function E2(e){let t=k(),a=Z(),[n,r]=p.default.useState(new Set),[s,i]=p.default.useState(""),[o,l]=p.default.useState(null),c=K({queryKey:["workspace-list",""],queryFn:()=>wi("")}),d=K({queryKey:["workspace-file",e],queryFn:()=>w2(e),enabled:!!e}),m=e===""||d.data?.kind==="directory",f=K({queryKey:["workspace-list",e],queryFn:()=>wi(e),enabled:m});p.default.useEffect(()=>{l(null)},[e]);let h=p.default.useCallback(y=>a.fetchQuery({queryKey:["workspace-list",y],queryFn:()=>wi(y)}),[a]),x=p.default.useCallback(async y=>{let $=new Set(n);if($.has(y)){$.delete(y),r($);return}$.add(y),r($);try{await h(y)}catch(g){l({type:"error",message:g.message||t("workspace.unableOpenDirectory")})}},[n,h,t]);return{rootEntries:c.data?.entries||[],file:d.data||null,selectionIsDirectory:m,currentEntries:f.data?.entries||[],expandedPaths:n,filter:s,setFilter:i,result:o,clearResult:()=>l(null),isLoadingTree:c.isLoading,isLoadingFile:d.isLoading,isLoadingListing:f.isLoading,isFetching:c.isFetching||d.isFetching||f.isFetching,error:c.error||d.error||f.error||null,loadDirectory:h,toggleDirectory:x,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function Lh(){let e=k(),t=ve(),n=it()["*"]||y2,r=E2(n),s=p.default.useCallback(i=>{t(ud(i))},[t]);return u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="flex h-full min-h-0 flex-col space-y-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1 className="text-lg font-semibold text-white">${e("workspace.title")}</h1>
                <${I} tone="muted" label=${e("workspace.readOnly")} />
              </div>
              <p className="mt-0.5 text-sm text-iron-400">${e("workspace.subtitle")}</p>
            </div>
            <${T}
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
          <${an}
            result=${r.result}
            onDismiss=${r.clearResult}
          />

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <${R2}
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
                  <${x2}
                    path=${n}
                    entries=${r.currentEntries}
                    isLoading=${r.isLoadingListing}
                    filter=${r.filter}
                    onOpen=${s}
                    onNavigate=${t}
                  />
                `:u`
                  <${C2}
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
  `}function T2(e){if(!e)return null;let t=e.metadata&&typeof e.metadata=="object"&&!Array.isArray(e.metadata)?e.metadata:{};return{id:e.project_id,name:e.name,description:e.description,goals:Array.isArray(t.goals)?t.goals:[],icon:e.icon||null,color:e.color||null,state:e.state,role:e.role,metadata:t,created_at:e.created_at,updated_at:e.updated_at,health:e.state==="archived"?"muted":"green"}}function qD(e){return e?{...e,id:e.thread_id,state:e.state||null,turn_count:e.turn_count||0,updated_at:e.updated_at||null}:null}async function A2(){let t=((await i$({limit:200}))?.projects||[]).map(T2);return{attention:[],projects:t}}async function D2(e){if(!e)return null;let t=await o$({projectId:e});return T2(t?.project)}function M2(e){return Promise.resolve({missions:[],todo:!0})}async function O2(e){if(!e)return{threads:[]};let t=await Nc({projectId:e,limit:200});return{threads:(t?.threads||[]).map(qD).filter(Boolean),next_cursor:t?.next_cursor||null}}function L2(e){return Promise.resolve({widgets:[],todo:!0})}function P2(e){return Promise.resolve(null)}function U2(e){return Promise.resolve(null)}function j2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function F2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function B2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function z2(){let e=Z(),t=K({queryKey:["projects-overview"],queryFn:A2,refetchInterval:5e3}),a=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function q2(e){let t=Z(),a=!!e,n=K({queryKey:["project-detail",e],queryFn:()=>D2(e),enabled:a,refetchInterval:a?7e3:!1}),r=K({queryKey:["project-missions",e],queryFn:()=>M2(e),enabled:a,refetchInterval:a?5e3:!1}),s=K({queryKey:["project-threads",e],queryFn:()=>O2(e),enabled:a,refetchInterval:a?4e3:!1}),i=K({queryKey:["project-widgets",e],queryFn:()=>L2(e),enabled:a,refetchInterval:a?15e3:!1}),o=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function I2({projectId:e,missionId:t,threadId:a}){let n=Z(),[r,s]=p.default.useState(null),i=K({queryKey:["project-mission-detail",t],queryFn:()=>P2(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=K({queryKey:["project-thread-detail",a],queryFn:()=>U2(a),enabled:!!a,refetchInterval:a?4e3:!1}),l=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=Y({mutationFn:({targetMissionId:f})=>j2(f),onSuccess:f=>{s({type:"success",message:f?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to fire mission"})}}),d=Y({mutationFn:({targetMissionId:f})=>F2(f),onSuccess:()=>{s({type:"success",message:"Mission paused."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to pause mission"})}}),m=Y({mutationFn:({targetMissionId:f})=>B2(f),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending}}function md(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function fd(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function H2(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function K2(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function ID(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function Q2(e){let t=ID(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function V2(e){let t=e?.projects||[],a=t.reduce((o,l)=>o+Number(l.cost_today_usd||0),0),n=t.reduce((o,l)=>o+Number(l.active_missions||0),0),r=t.reduce((o,l)=>o+Number(l.threads_today||0),0),s=t.reduce((o,l)=>o+Number(l.pending_gates||0),0),i=t.reduce((o,l)=>o+Number(l.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function vl(e,t){return`${e} ${t}${e===1?"":"s"}`}var HD={projects:"muted",attention:"warning",spend:"success"};function G2({overview:e}){let t=V2(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:fd(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return u`
    <${H} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>u`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${I} tone=${HD[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function KD(e){return e?.type==="failure"?"danger":"warning"}function QD(e){return e?.type==="failure"?"failure":"gate"}function Y2({items:e,onOpenItem:t}){return e?.length?u`
    <${H} className="overflow-hidden border-amber-300/10 p-0">
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
              <${I} tone=${KD(a)} label=${QD(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function VD({project:e,onOpen:t,t:a}){return u`
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
        <${I} tone=${H2(e.health)} label=${e.health||"unknown"} />
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
            ${a("projects.card.threadsToday",{count:vl(e.threads_today||0,"thread")})}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${a("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">${vl(e.pending_gates||0,"gate")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${a("projects.card.failures24h",{count:vl(e.failures_24h||0,"failure")})}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>${a("projects.card.spendToday",{value:fd(e.cost_today_usd||0)})}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${md(e.last_activity)}</div>
        </div>
        <${T}
          data-testid="project-open-workspace"
          variant="secondary"
          onClick=${n=>{n.stopPropagation(),t(e.id)}}
        >${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function GD({project:e,onOpen:t,t:a}){return u`
    <${H}
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
            ${vl(e.threads_today||0,"thread")} today
          </div>
          <${T}
            data-testid="project-open-workspace"
            variant="secondary"
            onClick=${n=>{n.stopPropagation(),t(e.id)}}
          >${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function J2({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=k(),l=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return t?u`
    <div data-testid="projects-grid" className="space-y-5">
      ${l&&u`<${GD} project=${l} onOpen=${r} t=${o} />`}

      <${H} className="p-4 sm:p-5">
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
            <${T} onClick=${s}>${o(i?"projects.preparingChat":"projects.newProject")}<//>
          </div>
        </div>
      <//>

      ${c.length?u`<div className="grid gap-4 xl:grid-cols-2 2xl:grid-cols-3">
            ${c.map(d=>u`<${VD} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
          </div>`:e.length?u`
            <${$e}
              title=${o("projects.scoped.onlyGeneralTitle")}
              description=${o("projects.scoped.onlyGeneralDesc")}
            >
              <${T} onClick=${s}>${o(i?"projects.preparingChat":"projects.startProject")}<//>
            <//>
          `:u`
              <${$e}
                title=${o("projects.empty.noMatchTitle")}
                description=${o("projects.empty.noMatchDesc")}
              />
            `}
    </div>
  `:u`
      <${$e}
        title=${o("projects.empty.noneTitle")}
        description=${o("projects.empty.noneDesc")}
      >
        <${T} onClick=${s}>${o("projects.createFromChat")}<//>
      <//>
    `}function X2({threads:e,selectedThreadId:t,onSelectThread:a,onNewConversation:n,isStartingConversation:r}){let s=[...e].sort((i,o)=>new Date(o.updated_at||o.created_at)-new Date(i.updated_at||i.created_at));return u`
    <${H} className="p-4 sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Conversations</div>
          <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">Project conversations</h2>
        </div>
        ${n&&u`
          <${T} onClick=${n} disabled=${r}>
            ${r?"Starting\u2026":"New conversation"}
          <//>
        `}
      </div>

      <div className="mt-5 space-y-3">
        ${s.length?s.slice(0,18).map(i=>{let o=Q2(i);return u`
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
                    <${I} tone=${K2(i.state)} label=${i.state} />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                    <span>${i.step_count||0} steps</span>
                    <span>${i.total_tokens||0} tokens</span>
                    <span>${md(i.updated_at||i.created_at)}</span>
                  </div>
                </button>
              `}):u`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                No project threads yet. When an automation runs or scoped chat work happens inside this project, activity will appear here.
              </div>
            `}
      </div>
    <//>
  `}var YD="/workspace";function JD(e){let t=a=>a.kind==="directory"?0:1;return[...e].sort((a,n)=>t(a)-t(n)||a.name.localeCompare(n.name,void 0,{sensitivity:"base"}))}function XD(e){return e?String(e).replace(/^\/workspace\/?/,"").split("/").filter(Boolean):[]}function W2({threadId:e}){let t=k(),[a,n]=p.default.useState(void 0),[r,s]=p.default.useState(null),i=K({queryKey:["project-files",e||"",a||""],queryFn:()=>Z0({threadId:e,path:a}),enabled:!!e}),o=p.default.useMemo(()=>JD(i.data?.entries||[]),[i.data]),l=p.default.useCallback(async m=>{if(m.kind==="directory"){s(null),n(m.path);return}try{s(null);let f=await Aa(_c({threadId:e,path:m.path})),h=URL.createObjectURL(f),x=document.createElement("a");x.href=h,x.download=m.name,document.body.appendChild(x),x.click(),x.remove(),URL.revokeObjectURL(h)}catch(f){s(f?.message||"Unable to download file")}},[e]),c=XD(a),d=u`
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
          ${"Files"}
        </div>
        <${I} tone="muted" label=${t("workspace.readOnly")} />
      </div>
      <${T}
        variant="secondary"
        size="sm"
        onClick=${()=>i.refetch()}
        disabled=${!e||i.isFetching}
      >
        ${i.isFetching?t("workspace.refreshing"):t("workspace.refresh")}
      <//>
    </div>
  `;return e?u`
    <${H} className="p-4 sm:p-5">
      ${d}

      <div className="mt-3 flex min-w-0 flex-wrap items-center gap-1.5 font-mono text-xs text-iron-400">
        <button
          type="button"
          onClick=${()=>n(void 0)}
          className="text-signal hover:underline"
        >
          ${"workspace"}
        </button>
        ${c.map((m,f)=>{let h=`${YD}/${c.slice(0,f+1).join("/")}`;return u`
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
                  data-testid="project-filesystem-entry"
                  data-entry-kind=${m.kind}
                  data-entry-path=${m.path}
                  className="flex w-full items-center gap-3 rounded-[12px] border border-transparent px-3 py-2 text-left hover:border-white/10 hover:bg-white/[0.04]"
                >
                  <${D}
                    name=${m.kind==="directory"?"folder":"file"}
                    className="h-4 w-4 shrink-0 text-iron-300"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm text-white">${m.name}</span>
                  ${m.kind==="directory"?u`<${D} name="chevron" className="h-3.5 w-3.5 shrink-0 -rotate-90 text-iron-500" />`:u`<${D} name="download" className="h-3.5 w-3.5 shrink-0 text-iron-500" />`}
                </button>
              `):u`
              <div className="rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                ${"This folder is empty."}
              </div>
            `}
      </div>
    <//>
  `:u`
      <${H} className="p-4 sm:p-5">
        ${d}
        <div className="mt-4 rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
          ${"No files yet \u2014 they appear once a thread has run in this project."}
        </div>
      <//>
    `}function WD(e){return[...e||[]].sort((a,n)=>new Date(n.updated_at||n.created_at)-new Date(a.updated_at||a.created_at))[0]?.id||null}function Z2({project:e,threads:t,selectedThreadId:a,onSelectThread:n,onNewConversation:r,isStartingConversation:s}){let i=WD(t);return u`
    <div
      data-testid="project-workspace"
      data-project-id=${e.id}
      className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]"
    >
      <div className="space-y-5">
        <div className="min-w-0">
          <h2 data-testid="project-workspace-title" className="text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
          ${e.description?u`<p className="mt-1 text-sm leading-6 text-iron-300">${e.description}</p>`:null}
        </div>

        <${X2}
          threads=${t}
          selectedThreadId=${a}
          onSelectThread=${n}
          onNewConversation=${r}
          isStartingConversation=${s}
        />
      </div>

      <${W2} threadId=${i} />
    </div>
  `}function gl(){let e=k(),t=ve(),{threadsState:a}=wa(),{projectId:n=null,threadId:r=null}=it(),[s,i]=p.default.useState(""),[o,l]=p.default.useState(null),c=z2(),d=q2(n),m=I2({projectId:n,threadId:r}),f=p.default.useMemo(()=>{let R=s.trim().toLowerCase();return R?c.overview.projects.filter(_=>[_.name,_.description,..._.goals||[]].some(A=>String(A||"").toLowerCase().includes(R))):c.overview.projects},[c.overview.projects,s]),h=p.default.useMemo(()=>c.overview.projects.find(R=>R.id===n)||null,[c.overview.projects,n]),x=p.default.useCallback(()=>{c.invalidate(),d.invalidate()},[c,d]),y=p.default.useCallback(R=>{t(`/projects/${R}`)},[t]),$=p.default.useCallback(R=>{if(R.thread_id){t(`/projects/${R.project_id}/threads/${R.thread_id}`);return}t(`/projects/${R.project_id}`)},[t]),g=p.default.useCallback(async()=>{let R=null;l(null);try{R=await a.createThread()}catch(_){l({type:"error",message:_.message||e("projects.chatAutoFail")});return}t(R?`/chat/${R}`:"/chat",{state:{composerDraft:e("projects.creationDraft")}})},[t,a,e]),v=p.default.useCallback(R=>{t(`/projects/${n}/threads/${R}`)},[t,n]),b=p.default.useCallback(async()=>{l(null);try{let R=await a.createThread(n);t(R?`/chat/${R}`:"/chat"),d.invalidate()}catch(R){l({type:"error",message:R.message||e("projects.chatAutoFail")})}},[t,a,n,d,e]),w=p.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),S=u`
    ${n&&u`<${T} variant="ghost" onClick=${()=>t("/projects")}>${e("projects.allProjects")}<//>`}
  `,C=null;return n?d.isLoading?C=u`
        <div className="space-y-4">
          ${[1,2,3].map(R=>u`<div key=${R} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `:d.error||!d.project&&!h?C=u`
        <${$e}
          title=${e("projects.unavailable")}
          description=${d.error?.message||e("projects.unavailableDesc")}
        >
          <${T} variant="secondary" onClick=${()=>t("/projects")}>${e("projects.returnToProjects")}<//>
        <//>
      `:C=u`
        <${Z2}
          project=${d.project||h}
          threads=${d.threads}
          selectedThreadId=${r}
          onSelectThread=${v}
          onNewConversation=${b}
          isStartingConversation=${a.isCreating}
        />
      `:C=c.isLoading?u`
          <div className="space-y-4">
            ${[1,2,3].map(R=>u`<div key=${R} className="v2-skeleton h-40 rounded-[20px]" />`)}
          </div>
        `:u`
          <${J2}
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
          <${an} result=${o} onDismiss=${()=>l(null)} />
          <${an} result=${m.actionResult} onDismiss=${m.clearActionResult} />
          ${!n&&u`
            <${G2} overview=${c.overview} />
            <${Y2} items=${c.overview.attention} onOpenItem=${$} />
          `}
          ${C}
        </div>
      </div>
    </div>
  `}function yl(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not scheduled"}function bl(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function eN(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function tN(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);return r!==0?r:new Date(n.updated_at||0).getTime()-new Date(a.updated_at||0).getTime()})}function pd({label:e,value:t}){return u`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function ZD({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=k();return e.status==="Active"?u`
      <${T} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.fireNow")}<//>
      <${T} variant="secondary" onClick=${()=>n(e.id)} disabled=${t}>${s("missions.action.pause")}<//>
    `:e.status==="Paused"?u`
      <${T} onClick=${()=>r(e.id)} disabled=${t}>${s("missions.action.resume")}<//>
      <${T} variant="secondary" onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runOnce")}<//>
    `:u`<${T} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runAgain")}<//>`}function aN({mission:e,isLoading:t,error:a,isBusy:n,onFire:r,onPause:s,onResume:i,onOpenProject:o,onOpenThread:l}){let c=k();return t?u`
      <div className="space-y-4">
        ${[1,2,3].map(d=>u`<div key=${d} className="v2-skeleton h-36 rounded-xl" />`)}
      </div>
    `:a||!e?u`
      <${$e}
        title=${c("missions.unavailable")}
        description=${a?.message||c("missions.unavailableDesc")}
      />
    `:u`
    <div className="space-y-4">
      <${H} className="p-4 sm:p-5">
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
          <${I} tone=${bl(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${pd} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${pd} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${pd} label=${c("missions.meta.nextFire")} value=${yl(e.next_fire_at)} />
          <${pd} label=${c("missions.meta.updated")} value=${yl(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${ZD}
            mission=${e}
            isBusy=${n}
            onFire=${r}
            onPause=${s}
            onResume=${i}
          />
        </div>
      <//>

      <${H} className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.brief")}</div>
        <div className="mt-4 text-sm leading-6 text-iron-200">
          <${sa} content=${e.goal||c("missions.noGoal")} />
        </div>
      <//>

      ${e.current_focus&&u`
        <${H} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${sa} content=${e.current_focus} />
          </div>
        <//>
      `}

      ${e.success_criteria&&u`
        <${H} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${sa} content=${e.success_criteria} />
          </div>
        <//>
      `}

      ${e.threads?.length?u`
        <${H} className="p-4 sm:p-5">
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
                  <${I} tone=${bl(d.state==="Running"?"Active":d.state==="Failed"?"Failed":"Completed")} label=${d.state} />
                </div>
              </button>
            `)}
          </div>
        <//>
      `:null}
    </div>
  `}function eM(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function nN({value:e,onChange:t,children:a,label:n}){return u`
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
  `}function tM({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=k(),s=t===e.id;return u`
    <div
      className=${["w-full rounded-xl border p-4 text-left",s?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80"].join(" ")}
    >
      <button type="button" onClick=${()=>a(e.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${e.name}</div>
              <${I} tone=${bl(e.status)} label=${e.status} />
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
          ${r("missions.updated",{value:yl(e.updated_at)})}
        </span>
        <${T}
          variant="ghost"
          onClick=${i=>{i.stopPropagation(),n(e.project.id)}}
        >
          ${e.project.name}
        <//>
      </div>
    </div>
  `}function Ph({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:l,projectOptions:c,onSelectMission:d,onOpenProject:m}){let f=k(),h=eM(f);return u`
    <${H} className="p-4 sm:p-5">
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
        <${nN} value=${s} onChange=${i} label=${f("missions.filter.status")}>
          ${h.map(x=>u`<option key=${x.value} value=${x.value}>${x.label}<//>`)}
        <//>
        <${nN} value=${o} onChange=${l} label=${f("missions.filter.project")}>
          <option value="all">${f("missions.filter.allProjects")}</option>
          ${c.map(x=>u`<option key=${x.id} value=${x.id}>${x.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(x=>u`
              <${tM}
                key=${x.id}
                mission=${x}
                selectedMissionId=${a}
                onSelectMission=${d}
                onOpenProject=${m}
              />
            `):u`
              <${$e}
                title=${f("missions.emptyTitle")}
                description=${f("missions.emptyDesc")}
                boxed=${!1}
              />
            `}
      </div>
    <//>
  `}function aM(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function rN({summary:e}){let t=k(),a=aM(t);return u`
    <${H} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>u`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${I} tone=${n.tone} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${e[n.key]||0}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">
              ${n.key==="total"?t("missions.summary.completedFailed",{completed:e.completed||0,failed:e.failed||0}):t("missions.summary.acrossProjects")}
            </p>
          </div>
        `)}
      </div>
    <//>
  `}function sN(){return Promise.resolve({projects:[],todo:!0})}function iN({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function oN(e){return Promise.resolve(null)}function lN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function uN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function cN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function dN(e){let t=K({queryKey:["mission-detail",e],queryFn:()=>oN(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function nM(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function mN(){let e=Z(),[t,a]=p.default.useState(null),n=K({queryKey:["projects-overview"],queryFn:sN,refetchInterval:7e3}),r=n.data?.projects||[],s=Id({queries:r.map(f=>({queryKey:["missions","project",f.id],queryFn:()=>iN({projectId:f.id}),refetchInterval:5e3,select:h=>h?.missions||[]}))}),i=s.flatMap((f,h)=>{let x=r[h];return(f.data||[]).map(y=>nM(y,x))}),o=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),l=(f,h)=>({mutationFn:({missionId:x})=>f(x),onSuccess:()=>{a({type:"success",message:h}),o()},onError:x=>{a({type:"error",message:x.message||"Unable to update mission"})}}),c=Y(l(lN,"Mission fired and a run was queued.")),d=Y(l(uN,"Mission paused.")),m=Y(l(cN,"Mission resumed."));return{projects:r,missions:i,summary:eN(i),isLoading:n.isLoading||s.some(f=>f.isLoading),isRefreshing:n.isFetching||s.some(f=>f.isFetching),error:n.error||s.find(f=>f.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending,invalidate:o}}function Uh(){let e=k(),t=ve(),{missionId:a=null}=it(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,l]=p.default.useState("all"),c=mN(),d=dN(a),m=p.default.useMemo(()=>{let g=n.trim().toLowerCase();return tN(c.missions).filter(v=>{let b=!g||[v.name,v.goal,v.project?.name].some(C=>String(C||"").toLowerCase().includes(g)),w=s==="all"||v.status===s,S=o==="all"||v.project?.id===o;return b&&w&&S})},[c.missions,o,n,s]),f=p.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),h=d.mission?{...f,...d.mission,project:f?.project||null}:f,x=p.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),y=p.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),$=a?u`
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <${Ph}
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
          <${aN}
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
        <${Ph}
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
            <${T}
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

          <${an}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${rN} summary=${c.summary} />

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
  `}var fN=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],rM=new Set(["pending","in_progress"]),pN=new Set(["failed","interrupted","stuck","cancelled"]);function cr(e){return e?String(e).replace(/_/g," "):"unknown"}function Si(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":pN.has(e)?"danger":"muted":"muted"}function sM(e){return rM.has(e)}function hd(e){return sM(e?.state)}function hN(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":pN.has(e.state):!1}function Yr(e,t=8){return e?String(e).slice(0,t):"unknown"}function ia(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function vN(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function jh(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${ia(e.started_at)}`:null].filter(Boolean).join(" / ")}var iM=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function gN(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function oM({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?u`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${t==="tool_use"?a.tool_name||"Tool call":a.tool_name||"Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${gN(t==="tool_use"?a.input:a.output||a.error||a)}</pre>
      </details>
    `:t==="message"?u`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a.role||"assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${a.content||""}</div>
      </div>
    `:u`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t.replace(/_/g," ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${a.message||a.status||gN(a)}</div>
    </div>
  `}function yN({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=k(),[s,i]=p.default.useState("all"),[o,l]=p.default.useState(""),[c,d]=p.default.useState(!0),m=p.default.useRef(null),f=p.default.useMemo(()=>s==="all"?t:t.filter(x=>x.event_type===s),[t,s]);p.default.useEffect(()=>{c&&m.current&&(m.current.scrollTop=m.current.scrollHeight)},[c,f.length]);let h=p.default.useCallback(async(x=!1)=>{let y=o.trim();if(!(!y&&!x))try{await a({content:y||"(done)",done:x}),l("")}catch{}},[o,a]);return u`
    <${H} className="p-5 sm:p-6">
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
            ${iM.map(x=>u`<option key=${x.value} value=${x.value}>${x.label}</option>`)}
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
                <${oM} event=${x} />
              </div>
            `):u`
              <${$e}
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
          <${T} variant="secondary" disabled=${n} onClick=${()=>h(!0)}>${r("common.done")}<//>
          <${T} variant="primary" disabled=${n} onClick=${()=>h(!1)}>${r("common.send")}<//>
        </div>
      `}
    <//>
  `}function bN({job:e,activeTab:t,onTabChange:a,onBack:n,onCancel:r,onRestart:s,isBusy:i,children:o}){return u`
    <div className="space-y-5">
      <${H} className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick=${n} className="text-sm text-signal hover:text-white">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-semibold tracking-tight text-white">${e.title||"Untitled job"}</h2>
              <${I} tone=${Si(e.state)} label=${cr(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${Yr(e.id)}</span>
              <span>created ${ia(e.created_at)}</span>
              ${jh(e)&&u`<span>${jh(e)}</span>`}
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
            ${hd(e)&&u`
              <${T} variant="secondary" disabled=${i} onClick=${()=>r(e.id)}>Cancel<//>
            `}
            ${hN(e)&&u`
              <${T} variant="primary" disabled=${i} onClick=${()=>s(e.id)}>Restart<//>
            `}
          </div>
        </div>
      <//>

      <div className="flex flex-wrap gap-2">
        ${fN.map(l=>u`
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
  `}function xN({nodes:e,depth:t=0,selectedPath:a,expandingPath:n,onToggleDirectory:r,onSelectPath:s}){return u`
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
        ${i.isDir&&i.expanded&&i.children?.length?u`<${xN}
              nodes=${i.children}
              depth=${t+1}
              selectedPath=${a}
              expandingPath=${n}
              onToggleDirectory=${r}
              onSelectPath=${s}
            />`:null}
      </div>
    `)}
  `}function $N({canBrowse:e,tree:t,selectedPath:a,selectedFile:n,fileError:r,isLoadingTree:s,isLoadingFile:i,expandingPath:o,treeError:l,onToggleDirectory:c,onSelectPath:d}){return e?u`
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <${H} className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          ${l&&u`<div className="mx-2 mb-3 rounded-md border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${l}</div>`}
          ${s?u`<div className="space-y-2 px-2">${[1,2,3,4].map(m=>u`<div key=${m} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?u`
                  <${xN}
                    nodes=${t}
                    selectedPath=${a}
                    expandingPath=${o}
                    onToggleDirectory=${c}
                    onSelectPath=${d}
                  />
                `:u`<div className="px-2 py-6 text-sm text-iron-300">No files were recorded for this workspace.</div>`}
        </div>
      <//>

      <${H} className="min-h-[440px] p-5 sm:p-6">
        <div className="border-b border-white/10 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">File preview</div>
          <p className="mt-2 break-all text-sm leading-6 text-iron-300">${n?.path||a||"Select a file from the tree to inspect its contents."}</p>
        </div>

        ${r&&!i?u`<div className="mt-5 rounded-md border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">${r}</div>`:i?u`<div className="mt-5 space-y-3">${[1,2,3,4,5].map(m=>u`<div key=${m} className="v2-skeleton h-4 rounded" />`)}</div>`:n?u`<pre className="mt-5 max-h-[60vh] overflow-auto whitespace-pre-wrap rounded-[18px] border border-white/10 bg-iron-950/90 p-4 font-mono text-xs leading-6 text-iron-100">${n.content}</pre>`:u`
                <${$e}
                  title="No file selected"
                  description="Pick a concrete file from the workspace tree to render it here."
                />
              `}
      <//>
    </div>
  `:u`
      <${$e}
        title="No project workspace"
        description="File browsing is only available for sandbox jobs that produced a mounted project directory."
      />
    `}function Ni({label:e,value:t}){return u`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function wN({job:e}){let t=(e.transitions||[]).map(a=>({title:`${cr(a.from)} -> ${cr(a.to)}`,description:[ia(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return u`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${H} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${I} tone=${Si(e.state)} label=${cr(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${Ni} label="Created" value=${ia(e.created_at)} />
          <${Ni} label="Started" value=${ia(e.started_at)} />
          <${Ni} label="Completed" value=${ia(e.completed_at)} />
          <${Ni} label="Duration" value=${vN(e.elapsed_secs)} />
          <${Ni} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${Ni} label="Mode" value=${e.job_mode||"Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${H} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${e.description?u`<${sa} content=${e.description} className="mt-4 text-sm leading-7 text-iron-200" />`:u`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
        <//>

        ${t.length?u`
              <${H} className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Transitions</div>
                <h3 className="mt-2 text-xl font-semibold text-white">State timeline</h3>
                <div className="mt-3">
                  <${v2} items=${t} />
                </div>
              <//>
            `:u`
              <${$e}
                title="No state history yet"
                description="Transitions appear here once the job advances or records a recovery event."
              />
            `}
      </div>
    </div>
  `}function SN({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:l,isBusy:c,isRefreshing:d}){let m=k(),f=[{value:"all",label:m("jobs.list.filter.all")},{value:"pending",label:m("jobs.list.filter.pending")},{value:"in_progress",label:m("jobs.list.filter.inProgress")},{value:"completed",label:m("jobs.list.filter.completed")},{value:"failed",label:m("jobs.list.filter.failed")},{value:"stuck",label:m("jobs.list.filter.stuck")}];if(!e.length){let h=!!n.trim()||s!=="all";return u`
      <${$e}
        title=${m(t&&h?"jobs.list.empty.noMatchTitle":"jobs.list.empty.noJobsTitle")}
        description=${m(t&&h?"jobs.list.empty.noMatchDesc":"jobs.list.empty.noJobsDesc")}
      />
    `}return u`
    <div className="space-y-5">
      <${H} className="p-4 sm:p-5">
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
                  <${I} tone=${Si(h.state)} label=${cr(h.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${Yr(h.id)}</span>
                  <span>${m("jobs.list.created",{value:ia(h.created_at)})}</span>
                  ${h.started_at&&u`<span>${m("jobs.list.started",{value:ia(h.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${hd(h)&&u`
                  <${T}
                    variant="secondary"
                    className="h-9 px-3 text-xs"
                    disabled=${c}
                    onClick=${()=>l(h.id)}
                  >
                    ${m("jobs.action.cancel")}
                  <//>
                `}
                <${T} variant="ghost" className="h-9 px-3 text-xs" onClick=${()=>o(h.id)}>${m("jobs.action.open")}<//>
              </div>
            </div>
          </article>
        `)}
      </div>
    </div>
  `}var lM=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function NN({summary:e}){return u`
    <${H} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${lM.map(t=>u`
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
  `}function _N(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function RN(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function kN(e){return Promise.resolve(null)}function CN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function EN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function TN(e){return Promise.resolve({events:[],todo:!0})}function AN(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function Fh(e,t=""){return Promise.resolve({entries:[],todo:!0})}function DN(e,t){return Promise.resolve({content:"",todo:!0})}function MN(e){let t=Z(),[a,n]=p.default.useState(null),r=K({queryKey:["job-detail",e],queryFn:()=>kN(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=K({queryKey:["job-events",e],queryFn:()=>TN(e),enabled:!!e,refetchInterval:e?2500:!1}),i=Y({mutationFn:({content:o,done:l})=>AN(e,{content:o,done:l}),onSuccess:(o,{done:l})=>{n({type:"success",message:l?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return p.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function ON(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function LN(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=LN(a.children,t);if(n)return n}}return null}function vd(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:vd(n.children,t,a)}:n)}function PN(e){let[t,a]=p.default.useState([]),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState(""),c=!!(e?.project_dir&&e?.id),d=K({queryKey:["job-files-root",e?.id],queryFn:()=>Fh(e.id,""),enabled:c}),m=K({queryKey:["job-file",e?.id,n],queryFn:()=>DN(e.id,n),enabled:!!(c&&n)});p.default.useEffect(()=>{a([]),r(""),i(""),l("")},[e?.id]),p.default.useEffect(()=>{d.data?.entries?(a(ON(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let f=p.default.useCallback(async h=>{let x=LN(t,h);if(!(!x||!e?.id)){if(x.expanded){a(y=>vd(y,h,$=>({...$,expanded:!1})));return}if(x.loaded){a(y=>vd(y,h,$=>({...$,expanded:!0})));return}l(h);try{let y=await Fh(e.id,h);a($=>vd($,h,g=>({...g,expanded:!0,loaded:!0,children:ON(y.entries)}))),i("")}catch(y){i(y.message||"Unable to open folder")}finally{l("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:m.data||null,fileError:m.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:m.isLoading||m.isFetching,expandingPath:o,treeError:s,toggleDirectory:f}}function UN(){let e=Z(),[t,a]=p.default.useState(null),n=K({queryKey:["jobs-summary"],queryFn:RN,refetchInterval:5e3}),r=K({queryKey:["jobs"],queryFn:_N,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=Y({mutationFn:({jobId:l})=>CN(l),onSuccess:(l,{jobId:c})=>{a({type:"success",message:`Job ${Yr(c)} cancelled`}),s()},onError:l=>{a({type:"error",message:l.message||"Unable to cancel job"})}}),o=Y({mutationFn:({jobId:l})=>EN(l),onSuccess:l=>{a({type:"success",message:`Restart queued as ${Yr(l?.new_job_id)}`}),s()},onError:l=>{a({type:"error",message:l.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function jN({result:e,onDismiss:t}){let a=k();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return u`
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
  `}function Bh(){let e=k(),t=ve(),{jobId:a=null}=it(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,l]=p.default.useState(a?"activity":"overview"),c=UN(),d=MN(a),m=PN(d.job);p.default.useEffect(()=>{l(a?"activity":"overview")},[a]);let f=p.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(b=>{let w=!v||b.title.toLowerCase().includes(v)||b.id.toLowerCase().includes(v),S=s==="all"||b.state===s;return w&&S})},[c.jobs,n,s]),h=p.default.useCallback(v=>t(`/jobs/${v}`),[t]),x=p.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),y=p.default.useCallback(async v=>{try{let b=await c.restartJob({jobId:v});b?.new_job_id&&t(`/jobs/${b.new_job_id}`)}catch{}},[c,t]),$=u`
    ${a&&u`<${T} variant="ghost" onClick=${()=>t("/jobs")}
      >${e("jobs.allJobs")}<//
    >`}
  `,g=null;if(a)if(d.isLoading)g=u`
        <div className="space-y-4">
          ${[1,2,3].map(v=>u`<div key=${v} className="v2-skeleton h-32 rounded-[18px]" />`)}
        </div>
      `;else if(d.error||!d.job)g=u`
        <${$e}
          title=${e("jobs.unavailable")}
          description=${d.error?.message||e("jobs.unavailableDesc")}
        >
          <${T} variant="secondary" onClick=${()=>t("/jobs")}
            >${e("jobs.returnToJobs")}<//
          >
        <//>
      `;else{let v={overview:u`<${wN} job=${d.job} />`,activity:u`
          <${yN}
            job=${d.job}
            events=${d.events}
            onSendPrompt=${d.sendPrompt}
            isSendingPrompt=${d.isSendingPrompt}
          />
        `,files:u`
          <${$N}
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
        <${bN}
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
          <${SN}
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
          <${jN}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${jN}
            result=${d.promptResult}
            onDismiss=${d.clearPromptResult}
          />
          <${NN} summary=${c.summary} />
          ${g}
        </div>
      </div>
    </div>
  `}function dr(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not scheduled"}function gd(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function yd(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function FN(e=[]){return[...e].sort((t,a)=>t.enabled!==a.enabled?t.enabled?-1:1:new Date(a.next_fire_at||a.last_run_at||0).getTime()-new Date(t.next_fire_at||t.last_run_at||0).getTime())}function BN(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function uM(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function zN({runs:e}){return e?.length?u`
    <div className="space-y-3">
      ${e.map(t=>u`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${I} tone=${uM(t.status)} label=${t.status} />
              <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                ${dr(t.started_at)}
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
    `}function mr({label:e,value:t}){return u`
    <div className="rounded-xl border border-iron-700 bg-iron-950/50 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div className="mt-2 min-w-0 break-words text-sm text-iron-100">
        ${t||"\u2014"}
      </div>
    </div>
  `}function qN({title:e,value:t}){return u`
    <div>
      <h3 className="text-sm font-semibold text-iron-100">${e}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >${JSON.stringify(t||{},null,2)}</pre>
    </div>
  `}function IN({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=ve(),l=k();return t?u`
      <div className="space-y-4">
        ${[1,2,3].map(c=>u`<div key=${c} className="v2-skeleton h-32 rounded-xl" />`)}
      </div>
    `:a||!e?u`
      <${$e}
        title=${l("routine.unavailable")}
        description=${a?.message||l("routine.unavailableDesc")}
      />
    `:u`
    <${H} className="p-4 sm:p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="truncate text-2xl font-semibold tracking-tight text-iron-100">
              ${e.name}
            </h2>
            <${I}
              tone=${gd(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${I}
              tone=${yd(e.verification_status)}
              label=${e.verification_status||"unknown"}
            />
          </div>
          <p className="mt-2 max-w-3xl text-sm leading-6 text-iron-300">
            ${e.description||e.trigger_summary||"No description"}
          </p>
        </div>

        <div className="flex flex-wrap gap-2">
          <${T} variant="secondary" disabled=${n} onClick=${r}>Run<//>
          <${T} variant="ghost" disabled=${n} onClick=${s}>
            ${e.enabled?"Disable":"Enable"}
          <//>
          <${T} variant="ghost" onClick=${i}>Delete<//>
        </div>
      </div>

      <div className="mt-5 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <${mr} label="Trigger" value=${e.trigger_summary||e.trigger_type} />
        <${mr} label="Action" value=${BN(e.action)} />
        <${mr} label="Next fire" value=${dr(e.next_fire_at)} />
        <${mr} label="Last run" value=${dr(e.last_run_at)} />
        <${mr} label="Run count" value=${e.run_count} />
        <${mr} label="Failures" value=${e.consecutive_failures} />
        <${mr} label="Created" value=${dr(e.created_at)} />
        <${mr} label="Routine ID" value=${e.id} />
      </div>

      ${e.conversation_id&&u`
        <div className="mt-5">
          <${T} variant="secondary" onClick=${()=>o(`/chat/${e.conversation_id}`)}>
            Open routine thread
          <//>
        </div>
      `}

      <div className="mt-6 grid gap-6 xl:grid-cols-2">
        <${qN} title=${l("routine.triggerPayload")} value=${e.trigger} />
        <${qN} title=${l("routine.actionPayload")} value=${e.action} />
      </div>

      <div className="mt-6">
        <h3 className="mb-3 text-sm font-semibold text-iron-100">Recent runs</h3>
        <${zN} runs=${e.recent_runs} />
      </div>
    <//>
  `}function HN({routine:e,selectedRoutineId:t,onSelectRoutine:a,onTriggerRoutine:n,onToggleRoutine:r,isBusy:s}){let i=t===e.id;return u`
    <article
      className=${["group flex flex-col gap-4 rounded-[18px] border p-5",i?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick=${()=>a(e.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">${e.name}</h3>
            <${I}
              tone=${gd(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${I}
              tone=${yd(e.verification_status)}
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
            <span>next ${dr(e.next_fire_at)}</span>
          </div>
        </button>

        <div className="flex shrink-0 flex-wrap gap-2">
          <${T}
            variant="secondary"
            className="h-9 px-3 text-xs"
            disabled=${s}
            onClick=${()=>n(e.id)}
          >
            Run
          <//>
          <${T}
            variant="ghost"
            className="h-9 px-3 text-xs"
            disabled=${s}
            onClick=${()=>r(e.id)}
          >
            ${e.enabled?"Disable":"Enable"}
          <//>
          <${T}
            variant="ghost"
            className="h-9 px-3 text-xs"
            onClick=${()=>a(e.id)}
          >
            Open
          <//>
        </div>
      </div>
    </article>
  `}var cM=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function zh({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:l,onToggleRoutine:c,isBusy:d,isRefreshing:m}){let f=k();if(!e.length){let h=!!n.trim()||s!=="all";return u`
      <${$e}
        title=${t&&h?"No routines match":"No routines yet"}
        description=${t&&h?"Adjust the search or status filter to find a saved routine.":"Routines created from chat will appear here after they are saved."}
      />
    `}return u`
    <div className="space-y-5">
      <${H} className="p-4 sm:p-5">
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
            ${cM.map(h=>u`<option key=${h.value} value=${h.value}>${h.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(h=>u`
            <${HN}
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
  `}var dM=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function KN({summary:e}){return u`
    <${H} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${dM.map(t=>u`
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
  `}function QN(e){let[t,a]=p.default.useState(""),[n,r]=p.default.useState("all");return{filteredRoutines:p.default.useMemo(()=>{let i=t.trim().toLowerCase();return FN(e).filter(o=>{let l=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||l.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function VN(){return Promise.resolve({routines:[],todo:!0})}function GN(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function YN(e){return Promise.resolve(null)}function bd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function xd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function JN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function XN(e){let t=Z(),[a,n]=p.default.useState(null),r=K({queryKey:["routine-detail",e],queryFn:()=>YN(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:m=>{n({type:"error",message:m.message||"Unable to update routine"})}}),o=Y(i(bd,"Routine run queued.")),l=Y(i(xd,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:l.mutateAsync,isBusy:o.isPending||l.isPending}}function WN(){let e=Z(),[t,a]=p.default.useState(null),n=K({queryKey:["routines-summary"],queryFn:GN,refetchInterval:5e3}),r=K({queryKey:["routines"],queryFn:VN,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,m)=>({mutationFn:({routineId:f})=>d(f),onSuccess:()=>{a({type:"success",message:m}),s()},onError:f=>{a({type:"error",message:f.message||"Unable to update routine"})}}),o=Y(i(bd,"Routine run queued.")),l=Y(i(xd,"Routine status updated.")),c=Y(i(JN,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:l.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||l.isPending||c.isPending,invalidate:s}}function qh(){let e=ve(),{routineId:t=null}=it(),a=WN(),n=XN(t),r=QN(a.routines),s=p.default.useCallback(async(l,c)=>{try{await l({routineId:c})}catch{}},[]),i=p.default.useCallback(async(l,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:l}),e("/routines")}catch{}},[e,a]),o=t?u`
        <div className="grid gap-5 xl:grid-cols-[minmax(0,0.9fr)_minmax(440px,1.1fr)]">
          <${zh}
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
          <${IN}
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
        <${zh}
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
            <${T} variant="ghost" onClick=${()=>e("/routines")}>
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

          <${an}
            result=${a.actionResult}
            onDismiss=${a.clearActionResult}
          />
          <${an}
            result=${n.actionResult}
            onDismiss=${n.clearActionResult}
          />
          <${KN} summary=${a.summary} />

          ${a.isLoading?u`
                <div className="space-y-4">
                  ${[1,2,3].map(l=>u`<div key=${l} className="v2-skeleton h-32 rounded-xl" />`)}
                </div>
              `:o}
        </div>
      </div>
    </div>
  `}function mM(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function fM(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function ZN({deliveryState:e}){let t=k(),a=e.currentTarget?.target_id||"",[n,r]=p.default.useState(a),[s,i]=p.default.useState(!1),o=p.default.useRef(null);p.default.useEffect(()=>{r(a)},[a]),p.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let l=n!==a,c=e.isLoading||e.isSaving,d=l&&!c,m=!!a&&!c,f=e.finalReplyTargets.length>0,h=e.targets.some(A=>A?.capabilities?.final_replies&&A?.target?.status==="unavailable"),x=f||h,y=A=>(o.current&&clearTimeout(o.current),i(!1),A.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),$=()=>{d&&y(e.saveFinalReplyTarget(n||null))},g=()=>{m&&(r(""),y(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),b=e.currentStatus,w=b==="available"?"success":b==="unavailable"?"warning":"muted",S=t(b==="available"?"automations.delivery.pill.ready":b==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),C=!!e.currentTarget,R=t(C?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),_=fM(t("automations.delivery.footnote"),{command:u`<code
        key="cmd"
        className="rounded px-1.5 py-0.5 font-mono text-[0.6875rem] bg-[var(--v2-surface-muted)] text-[var(--v2-accent-text)]"
      >
        approve &lt;code&gt;
      </code>`});return u`
    <${H} className="p-5 sm:p-6">
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
              <${I} tone=${w} label=${S} />
            </div>
          </div>
        `}

        <!-- ── Radio option rows ────────────────────────────────────── -->
        <div>
          <span className="mb-1.5 block font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
            ${R}
          </span>
          <div
            className="flex flex-col gap-3"
            role="radiogroup"
            aria-label=${t("automations.delivery.title")}
          >

            <!-- Available external targets -->
            ${e.finalReplyTargets.map(A=>{let L=A?.target?.target_id??"",U=A?.target?.display_name||A?.target?.target_id||"",F=A?.target?.description||"",B=A?.target?.status??"available",P=n===L;return u`
                <label
                  key=${L}
                  className=${J("flex items-start gap-3.5 rounded-xl border px-4 py-3.5 cursor-pointer","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]","hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",P&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
                >
                  <input
                    type="radio"
                    name="delivery-target"
                    value=${L}
                    checked=${P}
                    disabled=${c}
                    onChange=${()=>r(L)}
                    className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-semibold text-[var(--v2-text-strong)] leading-snug">
                      ${U}
                    </div>
                    ${F&&u`<div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                      ${F}
                    </div>`}
                  </div>
                  <${I}
                    tone=${mM(B)}
                    label=${t(B==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.ready")}
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
                <${I}
                  tone="warning"
                  label=${t("automations.delivery.pill.notPaired")}
                  className="shrink-0"
                />
              </div>
            `}

            <!-- Web app only / fallback row -->
            <label
              className=${J("flex items-start gap-3.5 rounded-xl border px-4 py-3.5","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]",f?"cursor-pointer hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]":"cursor-default",n===""&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
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
              <${I}
                tone="muted"
                label=${t("automations.delivery.pill.fallback")}
                className="self-center shrink-0"
              />
            </label>

          </div>
        </div>

        <!-- ── Save row ─────────────────────────────────────────────── -->
        <div className="flex flex-wrap items-center gap-3">
          <${T}
            variant="primary"
            size="sm"
            disabled=${!d}
            onClick=${$}
          >
            <${D} name="check" className="h-3.5 w-3.5" />
            ${t("automations.delivery.save")}
          <//>
          <${T}
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
              <${D} name="check" className="h-3 w-3" />
              ${t("automations.delivery.saved")}
            </span>
          `}
          ${e.saveError&&!s&&u`
            <span
              role="alert"
              className="flex items-center gap-1.5 text-xs font-semibold text-red-300"
            >
              <${D} name="close" className="h-3 w-3" />
              ${t("automations.delivery.saveFailed")}
            </span>
          `}
        </div>

        <!-- ── Footnote (only when an external Slack-style target exists) ── -->
        ${x&&u`
          <div
            className="rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-faint)]"
          >
            ${_}
          </div>
        `}

      </div>
    <//>
  `}var pM=["schedule","once"],t_={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},a_={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},n_={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function oa(e){return typeof e=="function"?e:t=>t}var Hh=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:Tn},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:EM},{value:"completed",labelKey:"automations.filter.completed",predicate:TM}];function r_(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>pM.includes(r?.source?.type)).map(r=>NM(r,t,a)).sort(CM)}function s_(e,t){let a=Hh.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function i_(e){let t=e.filter(i=>i.state!=="completed"),a=t.filter(i=>Tn(i)).length,n=t.filter(i=>i.has_running_run).length,r=t.filter(i=>i.has_failed_runs).length,s=t.filter(i=>Tn(i)&&Ih(i)!=null).sort((i,o)=>(i.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(o.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:t.length,active:a,running:n,failures:r,nextRun:s?.next_run_label||null}}function hM(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=OM(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:l,month:c,dayOfWeek:d,year:m}=s,f=t&&typeof t=="string"?t:null,h=f?` (${f})`:"",x=m==="*"&&l==="*"&&c==="*"&&d==="*";if(x&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=LM(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(fr(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let y=AM(o,i,n);if(!y)return r("automations.schedule.custom");if(x)return r("automations.schedule.everyDayAt",{time:y})+h;let $=PM(d);if(m==="*"&&l==="*"&&c==="*"&&$==="1-5")return r("automations.schedule.weekdaysAt",{time:y})+h;if(m==="*"&&l==="*"&&c==="*"&&fr($,0,7)){let g=DM(Number($)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:y})+h}if(m==="*"&&fr(l,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(l),time:y})+h;if(fr(l,1,31)&&fr(c,1,12)&&d==="*"&&(m==="*"||fr(m,1970,9999))){let g=MM(Number(c),Number(l),m==="*"?null:Number(m),n);return r("automations.schedule.dateAt",{date:g,time:y})+h}return r("automations.schedule.custom")}function Jr(e,t="Unknown",a,n){if(!e)return t;let r=new Date(e);if(Number.isNaN(r.getTime()))return t;let s=n&&typeof n=="string"?{timeZone:n}:{};try{return r.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...s})}catch{return r.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function o_(e,t){let a=t_[e]?.labelKey||"automations.state.unknown";return oa(t)(a)}function l_(e){return t_[e]?.tone||"muted"}function vM(e,t){return Tn(e)&&e?.has_running_run?oa(t)("automations.status.running"):Tn(e)&&e?.has_failed_runs?oa(t)("automations.status.needsReview"):o_(e?.state,t)}function gM(e){return Tn(e)&&e?.has_running_run?"info":Tn(e)&&e?.has_failed_runs?"danger":l_(e?.state)}function yM(e,t){let a=a_[e]?.labelKey||"automations.lastStatus.none";return oa(t)(a)}function bM(e){return a_[e]?.tone||"muted"}function xM(e,t){let a=n_[$d(e)]?.labelKey||"automations.runStatus.unknown";return oa(t)(a)}function $M(e){return n_[$d(e)]?.tone||"muted"}function wM(e,t,a,n){if(!e)return oa(a)("automations.schedule.custom");let r=Jr(e,null,n,t);if(!r)return oa(a)("automations.schedule.custom");let s=t&&typeof t=="string"?` (${t})`:"";return oa(a)("automations.schedule.onceAt",{datetime:r})+s}function SM(e,t,a){return e?.type==="once"?wM(e.at,e.timezone,t,a):e?.type==="schedule"?hM(e.cron,e.timezone||"UTC",t,a):oa(t)("automations.schedule.custom")}function NM(e,t,a){let n=oa(t),r=_M(e.recent_runs,t,a),s=r[0]||null,i=r.find(m=>m.status==="running")||null,o=r.find(m=>m.status==="ok"||m.status==="error")||null,l=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null,d={...e,recent_runs:r,has_running_run:r.some(m=>m.status==="running"),has_failed_runs:r.some(m=>m.status==="error")};return{...d,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:SM(e.source,t,a),state_label:o_(e.state,t),state_tone:l_(e.state),primary_status_label:vM(d,t),primary_status_tone:gM(d),next_run_timestamp:Kh(e.next_run_at),next_run_label:Jr(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:Jr(c,n("automations.date.noRuns"),a),last_status_label:yM(l,t),last_status_tone:bM(l),created_label:Jr(e.created_at,n("automations.date.unknown"),a),latest_run:s,current_run:i,success_rate_label:kM(r,t)}}function _M(e,t,a){let n=oa(t);return Array.isArray(e)?e.map(r=>{let s=$d(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=Kh(i);return{...r,status:s,status_label:xM(s,t),status_tone:$M(s),timestamp:o,timestamp_source:i,fired_label:Jr(i,n("automations.date.unscheduled"),a),submitted_label:Jr(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:Jr(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function $d(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function u_(e){let t=Array.isArray(e)?e:[],a={total:t.length,ok:0,error:0,running:0,unknown:0};for(let n of t){let r=$d(n?.status);Object.prototype.hasOwnProperty.call(a,r)?a[r]+=1:a.unknown+=1}return a}function RM(e){let t=u_(e);return[{key:"ok",tone:"text-emerald-300",count:t.ok},{key:"error",tone:"text-red-300",count:t.error},{key:"running",tone:"text-sky-300",count:t.running},{key:"unknown",tone:"text-iron-400",count:t.unknown}].filter(a=>a.count>0)}function c_(e,t){let a=oa(t),n=u_(e),r=RM(e).map(s=>({...s,text:a(`automations.runs.${s.key}`,{count:s.count})}));return{total:n.total,totalText:a("automations.runs.total",{count:n.total}),chips:r}}function kM(e,t){let a=oa(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function CM(e,t){let a=Tn(e),n=Tn(t);return a!==n?a?-1:1:(Ih(e)??Number.MAX_SAFE_INTEGER)-(Ih(t)??Number.MAX_SAFE_INTEGER)}function Kh(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function Tn(e){return e?.state==="active"||e?.state==="scheduled"}function EM(e){return["paused","disabled","inactive"].includes(e?.state)}function TM(e){return e?.state==="completed"}function Ih(e){return e?.next_run_timestamp??Kh(e?.next_run_at)}function Qh(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function AM(e,t,a){return!fr(e,0,23)||!fr(t,0,59)?null:Qh(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function DM(e,t){return Qh(t,{weekday:"long"},new Date(2001,0,7+e))}function MM(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return Qh(n,r,new Date(a??2e3,e-1,t))}function OM(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&e_(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&e_(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function e_(e){return/^0+$/.test(e)}function fr(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function LM(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function PM(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}var UM=8;function Vh(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function wd({runs:e=[]}){let t=k(),a=Array.isArray(e)?e:[],n=a.slice(0,UM);if(!n.length)return u`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;let r=a.length-n.length,s=`+${Math.min(r,999)}`;return u`
    <div
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
    >
      ${n.map(i=>u`
        <span
          key=${Vh(i)}
          title=${`${i.status_label} \xB7 ${i.fired_label}`}
          className=${J("h-3 w-3 rounded-full border",i.status==="ok"&&"border-emerald-300/50 bg-emerald-400",i.status==="error"&&"border-red-300/50 bg-red-400",i.status==="running"&&"border-sky-300/60 bg-sky-400",i.status==="unknown"&&"border-iron-500 bg-iron-600")}
        />
      `)}
      ${r>0&&u`<span
        className="ml-0.5 font-mono text-[11px] text-iron-400"
        title=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
      >
        ${s}
      </span>`}
    </div>
  `}function Sd({runs:e=[],className:t=""}){let a=k(),n=c_(e,a);return n.total?u`
    <div className=${J("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]",t)}>
      <span className="text-iron-300">${n.totalText}</span>
      ${n.chips.map(r=>u`<span key=${r.key} className=${r.tone}>${r.text}</span>`)}
    </div>
  `:u`<span className=${J("text-[11px] text-iron-400",t)}>
      ${a("automations.table.noRuns")}
    </span>`}function d_({run:e,onOpenRun:t,onOpenLogs:a}){let n=k(),r=!!e.chat_path,s=nd({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return u`
    <div className="grid gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto] sm:items-center">
      <div>
        <${I} tone=${e.status_tone} label=${e.status_label} />
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
        <${T}
          variant="secondary"
          size="sm"
          disabled=${!r}
          onClick=${r?()=>t(e.chat_path):void 0}
        >
          <${D} name="chat" className="mr-1.5 h-4 w-4" />
          ${n("automations.detail.openRun")}
        <//>
        <${T}
          variant="ghost"
          size="sm"
          disabled=${!i}
          onClick=${i?()=>a(s):void 0}
        >
          <${D} name="file" className="mr-1.5 h-4 w-4" />
          ${n("nav.logs")}
        <//>
      </div>
    </div>
  `}function Nd({label:e,value:t,tone:a}){return u`
    <div className="min-w-0 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div
        className=${J("mt-2 min-w-0 break-words text-sm text-iron-100",a==="success"&&"text-emerald-200",a==="danger"&&"text-red-200",a==="info"&&"text-sky-200")}
      >
        ${t||"\u2014"}
      </div>
    </div>
  `}function m_({automation:e,isMutating:t=!1,onPauseAutomation:a,onResumeAutomation:n,onDeleteAutomation:r}){let s=k(),i=ve();if(!e)return u`
      <${H} className="p-4 sm:p-5">
        <${$e}
          boxed=${!1}
          title=${s("automations.detail.emptyTitle")}
          description=${s("automations.detail.emptyDescription")}
        />
      <//>
    `;let o=e.current_run,l=e.state==="paused",c=e.state==="active"||e.state==="scheduled",m=`${s(l?"missions.action.resume":"missions.action.pause")}: ${e.display_name}`,f=()=>{if(l){n?.(e.automation_id);return}c&&a?.(e.automation_id)},h=`${s("common.delete")}: ${e.display_name}`,x=()=>{window.confirm(h)&&r?.(e.automation_id)};return u`
    <${H} className="overflow-hidden">
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
            <${I}
              tone=${e.primary_status_tone}
              label=${e.primary_status_label}
            />
            ${(c||l)&&u`
              <${T}
                type="button"
                variant=${l?"primary":"secondary"}
                size="icon-sm"
                aria-label=${m}
                title=${m}
                disabled=${t}
                onClick=${f}
              >
                <${D} name=${l?"play":"pause"} className="h-4 w-4" />
              <//>
            `}
            <${T}
              type="button"
              variant="danger"
              size="icon-sm"
              aria-label=${h}
              title=${h}
              disabled=${t}
              onClick=${x}
            >
              <${D} name="trash" className="h-4 w-4" />
            <//>
          </div>
        </div>
      </div>

      <div className="space-y-5 p-4 sm:p-5">
        <div className="grid gap-3 sm:grid-cols-2">
          <${Nd} label=${s("automations.detail.schedule")} value=${e.schedule_label} />
          <${Nd}
            label=${s("automations.detail.successRate")}
            value=${e.success_rate_label}
            tone=${e.has_failed_runs?"danger":"success"}
          />
          <${Nd} label=${s("automations.detail.lastCompleted")} value=${e.last_run_label} />
          <${Nd}
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
              <${wd} runs=${e.recent_runs} />
              <${Sd} runs=${e.recent_runs} />
            </div>
          </div>

          ${e.recent_runs.length?u`
                <div>
                  ${e.recent_runs.map(y=>u`
                    <${d_}
                      key=${Vh(y)}
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
  `}var jM=["automations.empty.example1","automations.empty.example2","automations.empty.example3"];function FM({promptKey:e}){let t=k(),a=t(e),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>()=>clearTimeout(s.current),[]),u`
    <li
      className="flex items-center gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3"
    >
      <span className="min-w-0 flex-1 text-sm leading-6 text-iron-200">${a}</span>
      <button
        type="button"
        onClick=${async()=>{let o=typeof navigator>"u"?null:navigator.clipboard;if(o?.writeText)try{await o.writeText(a),r(!0),clearTimeout(s.current),s.current=setTimeout(()=>r(!1),1500)}catch{}}}
        aria-label=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        title=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        className=${J("inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-[var(--v2-panel-border)] text-iron-300 hover:text-iron-100 hover:border-white/20","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",n&&"text-emerald-300")}
      >
        <${D} name=${n?"check":"copy"} className="h-4 w-4" />
      </button>
    </li>
  `}function f_(){let e=k(),t=ve();return u`
    <${H} className="p-6 sm:p-8">
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
            ${jM.map(a=>u`<${FM} key=${a} promptKey=${a} />`)}
          </ul>
        </div>

        <div className="mt-6">
          <${T} variant="primary" size="sm" onClick=${()=>t("/chat")}>
            <${D} name="chat" className="mr-1.5 h-4 w-4" />
            ${e("automations.empty.startInChat")}
          <//>
        </div>
      </div>
    <//>
  `}function p_({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,isMutating:s,selectedAutomationId:i,onSelectAutomation:o,onPauseAutomation:l,onResumeAutomation:c,onDeleteAutomation:d}){let m=k(),f=s_(e,t),h=e.length>0,x=f.find(y=>y.automation_id===i)||f[0]||null;return u`
    <div className="space-y-5">
      <${H} className="p-4 sm:p-5">
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
              ${Hh.map(y=>u`
                <button
                  key=${y.value}
                  type="button"
                  aria-pressed=${t===y.value}
                  onClick=${()=>a(y.value)}
                  className=${J("min-h-9 shrink-0 whitespace-nowrap px-3 py-2 text-xs font-semibold leading-tight",t===y.value?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
                >
                  ${m(y.labelKey)}
                </button>
              `)}
            </div>
            <${T}
              variant="secondary"
              size="icon-sm"
              aria-label=${m("automations.refresh")}
              title=${m(r?"automations.refreshing":"automations.refresh")}
              disabled=${r}
              onClick=${n}
            >
              <${D}
                name="retry"
                className=${J("h-4 w-4",r&&"v2-spin")}
              />
            <//>
          </div>
        </div>
      <//>

      ${f.length?u`
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1.12fr)_minmax(22rem,0.88fr)]">
              <${H} className="overflow-hidden">
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
                            className=${J("border-b border-[var(--v2-panel-border)] last:border-0 hover:bg-white/[0.03]",$&&"bg-[var(--v2-accent-soft)]/30")}
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
                                <${wd} runs=${y.recent_runs} />
                                <${Sd} runs=${y.recent_runs} />
                              </div>
                            </td>
                            <td className="px-5 py-4 align-top">
                              <${I}
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

              <${m_}
                automation=${x}
                isMutating=${s}
                onPauseAutomation=${l}
                onResumeAutomation=${c}
                onDeleteAutomation=${d}
              />
            </div>
          `:h?u`
              <${$e}
                title=${m("automations.empty.matchingTitle")}
                description=${m("automations.empty.matchingDescription")}
              />
            `:u`<${f_} />`}
    </div>
  `}function h_({summary:e,activeFilter:t,onSelectFilter:a}){let n=k(),r=[{key:"scheduled",label:n("automations.summary.scheduled"),value:e?.scheduled??0,tone:"muted",detail:n("automations.summary.scheduledDetail"),filter:"all"},{key:"active",label:n("automations.summary.active"),value:e?.active??0,tone:"signal",detail:n("automations.summary.activeDetail"),filter:"active"},{key:"running",label:n("automations.summary.running"),value:e?.running??0,tone:"info",detail:n("automations.summary.runningDetail"),filter:"running"},{key:"failures",label:n("automations.summary.failures"),value:e?.failures??0,tone:(e?.failures??0)>0?"danger":"success",detail:n("automations.summary.failuresDetail"),filter:(e?.failures??0)>0?"failures":null},{key:"nextRun",label:n("automations.summary.nextRun"),value:e?.nextRun||n("automations.summary.none"),tone:"info",detail:n("automations.summary.nextRunDetail"),valueClassName:"text-lg md:text-xl"}];return u`
    <${H} className="p-4 sm:p-5">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        ${r.map(s=>{let i=!!(s.filter&&a),o=i&&t===s.filter,l=u`
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
          `,c="rounded-[14px] border border-white/8 bg-white/[0.03] p-4 text-left";return i?u`
            <button
              key=${s.key}
              type="button"
              aria-pressed=${o}
              title=${n("automations.summary.filterAction",{label:s.label})}
              onClick=${()=>a(s.filter)}
              className=${J(c,"transition-colors hover:border-white/20 hover:bg-white/[0.05]","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",o&&"border-[var(--v2-accent)]/60 bg-[var(--v2-accent-soft)]/30")}
            >
              ${l}
            </button>
          `:u`<div key=${s.key} className=${c}>${l}</div>`})}
      </div>
    <//>
  `}function BM(e){return e==="active"||e==="scheduled"}function zM(e){return Number.isFinite(e)?e:null}function v_(e,t=Date.now()){let a=Array.isArray(e)?e:[],n=null;for(let r of a){if(!r||(r.has_running_run&&(n=n==null?5e3:Math.min(n,5e3)),!BM(r.state)))continue;let s=zM(r.next_run_timestamp);if(s==null)continue;let i=s-t,o=i<=0?2e3:i<3e4?Math.max(1e3,i+1200):null;o!=null&&(n=n==null?o:Math.min(n,o))}return n}var IM=50,HM=25;function g_(e=!1){let{t,lang:a}=wl(),n=Z(),r=K({queryKey:["automations",{includeCompleted:e}],queryFn:()=>t$({limit:IM,runLimit:HM,includeCompleted:e}),refetchInterval:3e4,refetchIntervalInBackground:!1}),s=p.default.useMemo(()=>r_(r.data,t,a),[r.data,t,a]),i=p.default.useMemo(()=>i_(s),[s]),o=p.default.useMemo(()=>v_(s),[s]);p.default.useEffect(()=>{if(o==null)return;let h=setTimeout(()=>{r.refetch()},o);return()=>clearTimeout(h)},[o,r.refetch]);let l=r.data?.scheduler_enabled!==!1,c=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["automations"]})},[n]),d=Y({mutationFn:h=>a$({automationId:h}),onSuccess:c}),m=Y({mutationFn:h=>n$({automationId:h}),onSuccess:c}),f=Y({mutationFn:h=>r$({automationId:h}),onSuccess:c});return{automations:s,summary:i,schedulerEnabled:l,isLoading:r.isLoading,isRefreshing:r.isFetching,isMutating:d.isPending||m.isPending||f.isPending,error:r.error||null,actionError:d.error||m.error||f.error||null,pauseAutomation:d.mutate,resumeAutomation:m.mutate,deleteAutomation:f.mutate,refetch:r.refetch}}var y_=["outbound-delivery","preferences"],b_=["outbound-delivery","targets"];function x_(){let e=Z(),t=K({queryKey:y_,queryFn:l$}),a=K({queryKey:b_,queryFn:u$}),n=Y({mutationFn:({finalReplyTargetId:i})=>c$({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(y_,i),e.invalidateQueries({queryKey:b_})}}),r=p.default.useMemo(()=>a.data?.targets??[],[a.data]),s=p.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function $_(){let e=k(),[t,a]=p.default.useState("all"),[n,r]=p.default.useState(null),i=g_(t==="completed"),o=x_(),[l,c]=p.default.useState(!1),d=p.default.useRef(null);p.default.useEffect(()=>()=>clearTimeout(d.current),[]);let m=p.default.useCallback(()=>{c(!0),clearTimeout(d.current),d.current=setTimeout(()=>c(!1),1e3),i.refetch()},[i.refetch]),f=i.isRefreshing||l,h=i.error&&!i.isLoading&&i.automations.length===0;return p.default.useEffect(()=>{if(!i.automations.length){r(null);return}i.automations.some(y=>y.automation_id===n)||r(i.automations[0].automation_id)},[i.automations,n]),u`
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
                <${h_}
                  summary=${i.summary}
                  activeFilter=${t}
                  onSelectFilter=${a}
                />
                <${ZN} deliveryState=${o} />

                ${i.isLoading?u`
                      <div className="space-y-4">
                        ${[1,2,3].map(x=>u`<div
                              key=${x}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`)}
                      </div>
                    `:u`
                      <${p_}
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
  `}var w_={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function S_({result:e,onDismiss:t}){return p.default.useEffect(()=>{if(!e)return;let a=setTimeout(t,4e3);return()=>clearTimeout(a)},[e,t]),e?u`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",w_[e.type]||w_.info].join(" ")}>
      <${D}
        name=${e.type==="success"?"check":e.type==="error"?"close":"bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">
        <${D} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `:null}var __="/api/webchat/v2/channels/slack/setup";function R_(){return V(__)}function k_(e){let t={installation_id:String(e.installation_id||"").trim(),team_id:String(e.team_id||"").trim(),api_app_id:String(e.api_app_id||"").trim(),user_id:N_(e.user_id),shared_subject_user_id:N_(e.shared_subject_user_id)},a=String(e.bot_token||"").trim(),n=String(e.signing_secret||"").trim(),r=String(e.oauth_client_id||"").trim(),s=String(e.oauth_client_secret||"").trim();return a&&(t.bot_token=a),n&&(t.signing_secret=n),r&&(t.oauth_client_id=r),s&&(t.oauth_client_secret=s),V(__,{method:"PUT",body:JSON.stringify(t)})}function Gh(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function N_(e){let t=String(e||"").trim();return t||null}var C_="/api/webchat/v2/channels/slack/allowed",KM="/api/webchat/v2/channels/slack/subjects";function E_(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function T_(){return V(C_)}function A_(){return V(KM)}function D_(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return V(C_,{method:"PUT",body:JSON.stringify(n)})}function M_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var O_=["slack-allowed-channels"];function P_({action:e}){let t=k(),a=Z(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState([]),c=VM(e,t),d=K({queryKey:O_,queryFn:T_}),m=K({queryKey:["slack-routable-subjects"],queryFn:A_}),f=m.data?.subjects||[],h=L_(f),x=m.isSuccess||m.isError,y=f.length>0;p.default.useEffect(()=>{d.data&&l(Yh(d.data.channels||[]))},[d.data]);let $=Y({mutationFn:({channels:C})=>D_(C),onSuccess:C=>{l(Yh(C.channels||[])),a.invalidateQueries({queryKey:O_}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let C=n.trim();!C||!m.isSuccess||(l(R=>Yh([...R,{channel_id:C,subject_user_id:s}])),r(""))},v=C=>{l(R=>R.filter(_=>_.channel_id!==C))},b=(C,R)=>{l(_=>_.map(A=>A.channel_id===C?{...A,subject_user_id:R}:A))},w=()=>{$.mutate({channels:QM(o)})},S=m.isError&&o.some(C=>!C.subject_user_id);return u`
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
        <${T}
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
                      onChange=${R=>b(C.channel_id,R.target.value)}
                      className="h-8 rounded-md border border-white/10 bg-white/[0.04] px-2 text-xs text-iron-100 outline-none focus:border-signal/45"
                    >
                      <option value="">${c.autoSubjectLabel}</option>
                      ${L_(f,C).map(R=>u`
                          <option key=${R.subject_user_id} value=${R.subject_user_id}>
                            ${R.display_name}
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
        <${T}
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
          ${M_($.error||d.error||m.error,c.errorMessage)}
        </p>`}
      </div>
    </div>
  `}function L_(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function Yh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return E_(Array.from(t.keys())).map(a=>t.get(a))}function QM(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function VM(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var Xh=["slack-setup"],An={installationId:{body:"Local IronClaw name for this Slack install. Choose one and keep it stable.",example:"Example: local-slack"},teamId:{body:"Slack workspace/team ID from the workspace that installed the app.",example:"Example: T0123456789"},appId:{body:"Slack app Basic Information > App Credentials.",example:"Example: A0123456789"},botUser:{body:"Optional Reborn user. Blank uses the current WebUI operator.",example:"Example: user:operator"},sharedSubject:{body:"Optional default team agent for shared channel turns. Usually blank.",example:"Example: user:slack-shared"},botToken:{body:"Slack app OAuth & Permissions > Bot User OAuth Token.",example:"Example: xoxb-..."},signingSecret:{body:"Slack app Basic Information > App Credentials > Signing Secret.",example:""},oauthClientId:{body:"Slack app OAuth & Permissions > App Credentials > Client ID. Required for personal (user-token) OAuth.",example:"Example: 123456789012.123456789012"},oauthClientSecret:{body:"Slack app OAuth & Permissions > App Credentials > Client Secret. Required for personal (user-token) OAuth.",example:""}};function j_({action:e}){let t=K({queryKey:Xh,queryFn:R_}),a=t.data?.configured===!0;return u`
    <div className="space-y-3">
      <${GM} action=${e} setupQuery=${t} />
      ${a&&u`<${P_} action=${e} />`}
    </div>
  `}function GM({action:e,setupQuery:t}){let a=Z(),[n,r]=p.default.useState(YM()),s=p.default.useRef(!1),i=p.default.useRef(!1),o=t.data,l=JM(e);p.default.useEffect(()=>{!o||s.current||i.current||(r(U_(o)),s.current=!0)},[o]);let c=Y({mutationFn:k_,onSuccess:h=>{i.current=!1,r(U_(h)),s.current=!0,a.setQueryData(Xh,h),a.invalidateQueries({queryKey:Xh}),a.invalidateQueries({queryKey:["slack-allowed-channels"]}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["extensions"]})}}),d=h=>x=>{i.current=!0,r(y=>({...y,[h]:x.target.value}))},m=()=>c.mutate(n),f=n.installation_id.trim()&&n.team_id.trim()&&n.api_app_id.trim()&&(o?.bot_token_configured||n.bot_token.trim())&&(o?.signing_secret_configured||n.signing_secret.trim())&&(o?.oauth_client_secret_configured||!n.oauth_client_id.trim()||n.oauth_client_secret.trim());return u`
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
        ${_i("Installation ID",n.installation_id,d("installation_id"),"",An.installationId)}
        ${_i("Team ID",n.team_id,d("team_id"),"",An.teamId)}
        ${_i("App ID",n.api_app_id,d("api_app_id"),"",An.appId)}
        ${_i("Bot user",n.user_id,d("user_id"),"default operator",An.botUser)}
        ${_i("Shared subject",n.shared_subject_user_id,d("shared_subject_user_id"),"optional",An.sharedSubject)}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        ${Jh("Bot token",n.bot_token,d("bot_token"),o?.bot_token_configured,An.botToken)}
        ${Jh("Signing secret",n.signing_secret,d("signing_secret"),o?.signing_secret_configured,An.signingSecret)}
        ${_i("OAuth client ID",n.oauth_client_id,d("oauth_client_id"),"optional",An.oauthClientId)}
        ${Jh("OAuth client secret",n.oauth_client_secret,d("oauth_client_secret"),o?.oauth_client_secret_configured,An.oauthClientSecret)}
      </div>

      <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <${T}
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick=${m}
          disabled=${!f||c.isPending}
        >
          ${c.isPending?"Saving...":l.submitLabel}
        <//>
        ${t.isError&&u`<p className="text-xs text-red-300">
          ${Gh(t.error,l.errorMessage)}
        </p>`}
        ${c.isError&&u`<p className="text-xs text-red-300">
          ${Gh(c.error,l.errorMessage)}
        </p>`}
        ${c.isSuccess&&u`<p className="text-xs text-emerald-300">${l.successMessage}</p>`}
      </div>
    </div>
  `}function U_(e){return{installation_id:e.installation_id||"",team_id:e.team_id||"",api_app_id:e.api_app_id||"",user_id:e.user_id||"",shared_subject_user_id:e.shared_subject_user_id||"",bot_token:"",signing_secret:"",oauth_client_id:e.oauth_client_id||"",oauth_client_secret:""}}function YM(){return{installation_id:"",team_id:"",api_app_id:"",user_id:"",shared_subject_user_id:"",bot_token:"",signing_secret:"",oauth_client_id:"",oauth_client_secret:""}}function _i(e,t,a,n="",r=null){return u`
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-iron-500">${e}</span>
      <input
        type="text"
        value=${t}
        onChange=${a}
        placeholder=${n}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <${F_} help=${r} />
    </label>
  `}function Jh(e,t,a,n,r=null){return u`
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
      <${F_} help=${r} />
    </label>
  `}function F_({help:e}){return e?u`
    <p className="mt-1.5 min-h-8 text-[11px] leading-4 text-iron-400">
      <span className="block">${e.body}</span>
      ${e.example&&u`<span className="mt-0.5 block font-mono text-iron-300">${e.example}</span>`}
    </p>
  `:null}function JM(e){return{title:"Slack setup",instructions:e?.instructions||"Configure the Slack app before assigning channels.",submitLabel:"Save setup",successMessage:"Slack setup saved.",errorMessage:"Slack setup update failed."}}var Wh={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function pr(e){return e==="wasm_channel"||e==="channel"}var B_={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},z_={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function q_(e){let t=I_(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||pr(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function I_(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function Zh(e){let t=I_(e);return t==="active"||t==="ready"}function H_({extension:e,secrets:t=[],fields:a=[]}={}){return Zh(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var K_="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",Q_="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",V_="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",G_="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",Y_="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",XM="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function J_(e){return e.package_ref?.id||""}function WM({actions:e,isBusy:t}){let a=k(),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>{if(!n)return;let i=o=>{s.current&&!s.current.contains(o.target)&&r(!1)};return document.addEventListener("mousedown",i),()=>document.removeEventListener("mousedown",i)},[n]),u`
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
        <${D} name="more" className="h-4 w-4" strokeWidth=${2.4} />
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
                <${D} name=${i.icon||"settings"} className="h-3.5 w-3.5" />
                ${i.label}
              </button>
            `)}
        </div>
      `}
    </div>
  `}function X_({items:e}){return!e||e.length===0?null:u`
    <div className="mt-3 flex flex-wrap gap-1">
      ${e.map(t=>u`<span key=${t} className=${XM}>${t}</span>`)}
    </div>
  `}function Ri({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=k(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=B_[i]||"muted",l=s(`extensions.state.${i}`)||z_[i]||i,c=s(`extensions.kind.${e.kind}`)||Wh[e.kind]||e.kind,d=e.display_name||J_(e),m=!!e.package_ref,f=e.tools||[],[h,x]=p.default.useState(!1),$=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],b=[],w=q_(e);w==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):w==="activate"&&v.push({id:"activate",label:s("extensions.activate"),run:()=>t(g)}),m&&(e.needs_setup||e.has_auth)&&w!=="configure"&&b.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)});let S=b.some(R=>R.id==="configure");m&&w!=="configure"&&pr(e.kind)&&(i==="setup_required"||i==="failed")&&b.push({id:"setup",label:s("extensions.setup"),icon:"settings",run:()=>a(g)}),m&&pr(e.kind)&&!S&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&b.push({id:"reconfigure",label:s("extensions.reconfigure"),icon:"settings",run:()=>a(g)}),m&&b.push({id:"remove",label:s("common.remove"),icon:"trash",danger:!0,run:()=>n(g)});let C=v[0];return u`
    <div className=${K_}>
      <div className="flex items-start gap-2">
        <${I} tone=${o} label=${l} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${b.length>0&&u`<${WM} actions=${b} isBusy=${r} />`}
      </div>

      <div className=${Q_}>
        <span>${c}</span>
        ${e.version&&u`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&u`<p className=${V_}>${e.description}</p>`}

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

      <div className=${G_}>
        ${f.length>0?u`
              <button
                type="button"
                aria-expanded=${h?"true":"false"}
                onClick=${()=>x(R=>!R)}
                className=${Y_}
              >
                <${D} name="layers" className="h-3.5 w-3.5" />
                <span>${f.length===1?s("extensions.oneCapability"):s("extensions.pluralCapabilities",{count:f.length})}</span>
                <${D}
                  name="chevron"
                  className=${["h-3 w-3",h?"rotate-180":""].join(" ")}
                />
              </button>
            `:u`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">${s("extensions.noCapabilities")}</span>`}
        <span className="flex-1"></span>
        ${C&&u`
          <${T} variant="secondary" size="sm" onClick=${C.run} disabled=${r}>
            ${C.label}
          <//>
        `}
      </div>

      ${h&&u`<${X_} items=${f} />`}
    </div>
  `}function Xr({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=k(),s=r(`extensions.kind.${e.kind}`)||Wh[e.kind]||e.kind,i=e.display_name||J_(e),o=!!(e.package_ref&&t),l=!!(e.needs_setup||e.has_auth||pr(e.kind)),c=e.keywords||[],[d,m]=p.default.useState(!1);return u`
    <div className=${K_}>
      <div className="flex items-start gap-2">
        <${I}
          tone="muted"
          label=${n||r("extensions.state.available")||"available"}
          size="sm"
        />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${i}
        </span>
      </div>

      <div className=${Q_}>
        <span>${s}</span>
        ${e.version&&u`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&u`<p className=${V_}>${e.description}</p>`}

      <div className=${G_}>
        ${c.length>0?u`
              <button
                type="button"
                aria-expanded=${d?"true":"false"}
                onClick=${()=>m(f=>!f)}
                className=${Y_}
              >
                <${D} name="list" className="h-3.5 w-3.5" />
                <span>${c.length===1?r("extensions.oneKeyword"):r("extensions.pluralKeywords",{count:c.length})}</span>
                <${D}
                  name="chevron"
                  className=${["h-3 w-3",d?"rotate-180":""].join(" ")}
                />
              </button>
            `:u`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]"></span>`}
        <span className="flex-1"></span>
        ${o&&u`
          <${T}
            variant="outline"
            size="sm"
            onClick=${()=>t({packageRef:e.package_ref,displayName:i,configureAfterInstall:l})}
            disabled=${a}
          >
            <${D} name="plus" className="mr-1.5 h-3.5 w-3.5" />
            ${r("extensions.install")}
          <//>
        `}
      </div>

      ${d&&u`<${X_} items=${c} />`}
    </div>
  `}var ZM="/api/webchat/v2/extensions/pairing/redeem";function W_(e,t){return V(ZM,{method:"POST",body:JSON.stringify({channel:e,code:t})}).then(a=>({success:!0,provider:a.provider,provider_user_id:a.provider_user_id}))}function Z_(){return V("/api/webchat/v2/extensions")}function eR(){return V("/api/webchat/v2/extensions/registry")}function tR(e){return V("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function aR(e){return V(`/api/webchat/v2/extensions/${encodeURIComponent(xl(e))}/activate`,{method:"POST"})}function nR(e){return V(`/api/webchat/v2/extensions/${encodeURIComponent(xl(e))}/remove`,{method:"POST"})}function rR(e){return V(`/api/webchat/v2/extensions/${encodeURIComponent(xl(e))}/setup`)}function sR(e,t,a){return y$(xl(e),{action:"submit",payload:{secrets:t,fields:a}})}function iR(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return V(`/api/webchat/v2/extensions/${encodeURIComponent(xl(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function oR(){return Promise.resolve({requests:[]})}function lR(e,t){return W_(e,t)}function xl(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var eO=2e3,tO=10*60*1e3;function cR(e){try{return new URL(e).protocol==="https:"}catch{return!1}}function _d(e,t=null){return cR(e)?t&&!t.closed?(t.location.href=e,{ok:!0,popup:t}):{ok:!0,popup:window.open(e,"_blank","noopener,noreferrer")}:{ok:!1,popup:null}}function ki(e){return e?.package_ref?.id||null}function ev(e){return e?.display_name||ki(e)||""}function uR(e,t,a){return ki(t)||`${e}:${ev(t)||"unknown"}:${a}`}function aO(e,t){return e.installed!==t.installed?e.installed?-1:1:ev(e.entry||e.extension).localeCompare(ev(t.entry||t.extension))}function dR(){let e=k(),t=Z(),a=K({queryKey:["gateway-status-extensions"],queryFn:si,staleTime:1e4}),n=K({queryKey:["extensions"],queryFn:Z_,refetchOnMount:"always"}),r=K({queryKey:["extension-registry"],queryFn:eR,refetchOnMount:"always"}),s=K({queryKey:["connectable-channels"],queryFn:Wc,refetchOnMount:"always"}),i=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["gateway-status-extensions"]}),t.invalidateQueries({queryKey:["connectable-channels"]})},[t]),[o,l]=p.default.useState(null),c=p.default.useCallback(()=>l(null),[]),d=Y({mutationFn:({packageRef:P})=>tR(P),onSuccess:(P,{displayName:G,configureAfterInstall:ae,onNeedsSetup:le,packageRef:lt})=>{P.success?(l({type:"success",message:P.message||P.instructions||e("extensions.installedSuccess",{name:G||e("extensions.defaultName")})}),P.auth_url&&!_d(P.auth_url).ok?l({type:"error",message:"Authentication URL must use HTTPS."}):!P.auth_url&&ae&&typeof le=="function"&&le({packageRef:lt,displayName:G,active:!1,activationStatus:"setup_required",onboardingState:"setup_required"})):l({type:"error",message:P.message||e("extensions.installFailed")}),i()},onError:P=>{l({type:"error",message:P.message}),i()}}),m=Y({mutationFn:({packageRef:P})=>aR(P),onSuccess:(P,{displayName:G})=>{P.success?(l({type:"success",message:P.message||P.instructions||e("extensions.activatedSuccess",{name:G||e("extensions.defaultName")})}),P.auth_url&&!_d(P.auth_url).ok&&l({type:"error",message:"Authentication URL must use HTTPS."})):P.auth_url?_d(P.auth_url).ok?l({type:"info",message:e("extensions.openingAuth")}):l({type:"error",message:"Authentication URL must use HTTPS."}):P.awaiting_token?l({type:"info",message:e("extensions.configurationRequired")}):l({type:"error",message:P.message||e("extensions.activationFailed")}),i()},onError:P=>{l({type:"error",message:P.message})}}),f=Y({mutationFn:({packageRef:P})=>nR(P),onSuccess:(P,{displayName:G})=>{P.success?l({type:"success",message:e("extensions.removedSuccess",{name:G||e("extensions.defaultName")})}):l({type:"error",message:P.message||e("extensions.removeFailed")}),i()},onError:P=>{l({type:"error",message:P.message})}}),h=a.data||{},x=n.data?.extensions||[],y=r.data?.entries||[],$=s.data?.channels||[],g=new Map(x.map(P=>[ki(P),P]).filter(([P])=>!!P)),v=new Set(y.map(P=>ki(P)).filter(Boolean)),b=[...y.map((P,G)=>{let ae=ki(P),le=ae&&g.get(ae)||null;return{id:uR("registry",P,G),installed:!!(le||P.installed),entry:P,extension:le}}),...x.filter(P=>{let G=ki(P);return!G||!v.has(G)}).map((P,G)=>({id:uR("installed",P,G),installed:!0,entry:null,extension:P}))].sort(aO),w=P=>pr(P.kind),S=x.filter(w),C=x.filter(P=>P.kind==="mcp_server"),R=x.filter(P=>!w(P)&&P.kind!=="mcp_server"),_=y.filter(P=>w(P)&&!P.installed),A=y.filter(P=>P.kind==="mcp_server"&&!P.installed),L=y.filter(P=>P.kind!=="mcp_server"&&!w(P)&&!P.installed),U=n.isLoading||r.isLoading,F=d.isPending||m.isPending||f.isPending,B=p.default.useCallback(P=>{let G=P?.displayName||P?.packageRef?.id||"this extension";window.confirm(`Remove ${G}?`)&&f.mutate(P)},[f]);return{status:h,extensions:x,channels:S,mcpServers:C,tools:R,channelRegistry:_,mcpRegistry:A,toolRegistry:L,registry:y,catalogEntries:b,connectableChannels:$,isLoading:U,isBusy:F,actionResult:o,clearResult:c,install:d.mutate,activate:m.mutate,remove:B,invalidate:i}}function mR(e){let t=K({queryKey:["extension-setup",e?.id||e],queryFn:()=>rR(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function fR(e,t){let a=Z(),n=e?.id||e;return Y({mutationFn:({secrets:r,fields:s})=>sR(e,r,s).then(i=>{if(i.success===!1)throw new Error(i.message||"Setup failed");return i}),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function pR(e){let t=Z(),a=e?.id||e,n=p.default.useRef(null),r=p.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=p.default.useCallback(()=>{let l=t.getQueryData(["extension-setup",a]);if(l?.secrets?.length>0&&l.secrets.every(f=>f.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(f=>f.package_ref?.id===a),m=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return m==="active"||m==="ready"},[a,t]),o=p.default.useCallback(l=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||l&&l.closed||Date.now()-c>tO)&&(r(),s())},eO)},[r,s,i]);return p.default.useEffect(()=>r,[r]),Y({mutationFn:({secret:l,popup:c})=>iR(e,l).then(d=>{if(d.success===!1)throw new Error(d.message||"OAuth setup failed");if(d.authorization_url&&!cR(d.authorization_url))throw new Error("Authorization URL must use HTTPS.");return{res:d,popup:c}}),onSuccess:({res:l,popup:c})=>{let d=c;l.authorization_url?d=_d(l.authorization_url,c).popup:c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(l,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function hR(e,t={}){let a=K({queryKey:["pairing",e],queryFn:()=>oR(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=Z(),r=Y({mutationFn:({code:s})=>lR(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function vR(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var nO={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function gR({channel:e,redeemFn:t,i18nKeys:a=nO,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=k(),o=typeof t=="function",l=hR(e,{enabled:!o}),c=Z(),[d,m]=p.default.useState(""),f=rO(i,a,r),h=Y({mutationFn:({code:S})=>t(e,S),onSuccess:()=>{m("");for(let S of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:S})}}),x=p.default.useCallback(S=>l.approve({code:S}),[l.approve]),y=p.default.useCallback(()=>{let S=d.trim().toUpperCase();S&&(o?h.mutate({code:S}):l.approve({code:S}))},[o,d,l.approve,h]),$=o?[]:l.requests,g=o?!1:l.isLoading,v=o?h.isPending:l.isApproving,b=o?h.isSuccess?h.data:null:l.result,w=o?h.isError?h.error:null:l.error;return p.default.useEffect(()=>{b?.success&&m("")},[b?.success]),g?u`
      <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
        <div className="v2-skeleton h-3 w-24 rounded" />
      </div>
    `:u`
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
        <${T}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${y}
          disabled=${v||!d.trim()}
          data-testid="pairing-submit"
        >
          ${f.action}
        <//>
      </div>

      ${b?.success&&u`<p data-testid="pairing-success" className="mb-3 text-xs text-emerald-300">
        ${b.message||f.success}
      </p>`}
      ${b&&!b.success&&u`<p data-testid="pairing-error" className="mb-3 text-xs text-red-300">
        ${b.message||f.error}
      </p>`}
      ${w&&u`<p data-testid="pairing-error" className="mb-3 text-xs text-red-300">
        ${vR(w,f.error)}
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
                  <${T}
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
  `}function rO(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function Rd(e){return e.package_ref?.id||""}function yR(e){return Rd(e)==="slack"}function xR(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function $R(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function sO(e){let t=e||[],a=[t.find(xR),t.find($R)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function bR({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>xR(r)?u`<${j_} action=${r.action} />`:$R(r)?u`<${Vc} action=${r.action} />`:null).filter(Boolean);return n.length>0?u`<div className="space-y-3">${n}</div>`:null}function wR({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:l}){let c=k(),d=t||[],m=e.enabled_channels||[],f=sO(a),h=d.some(yR),x=f.length>0&&!h;return u`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          ${c("channels.builtIn")}
        </h3>
        <${Ci}
          name=${c("channels.webGateway")}
          description=${c("channels.webGatewayDesc")}
          enabled=${!0}
          detail=${"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)}
        />
        <${Ci}
          name=${c("channels.httpWebhook")}
          description=${c("channels.httpWebhookDesc")}
          enabled=${m.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${Ci}
          name=${c("channels.cli")}
          description=${c("channels.cliDesc")}
          enabled=${m.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${Ci}
          name=${c("channels.repl")}
          description=${c("channels.replDesc")}
          enabled=${m.includes("repl")}
          detail="ironclaw run --repl"
        />
        ${x&&u`
          <${Ci}
            name=${c("channels.slack")}
            description=${c("channels.slackDesc")}
            enabled=${!1}
            statusLabel=${c("channels.setup")}
            statusTone="muted"
            detail=${c("channels.slackDetail")}
          >
            <${bR}
              slackConnectActions=${f}
            />
          </${Ci}>
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
                <div key=${Rd(y)} className="flex flex-col gap-3">
                  <${Ri}
                    ext=${y}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${l}
                  />
                  ${yR(y)&&u`<${bR}
                    slackConnectActions=${f}
                  />`}
                  ${(y.onboarding_state==="pairing_required"||y.onboarding_state==="pairing")&&u` <${gR} channel=${Rd(y)} /> `}
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
                  key=${Rd(y)}
                  entry=${y}
                  onInstall=${o}
                  isBusy=${l}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function Ci({name:e,description:t,enabled:a,detail:n,children:r,statusLabel:s=a?"on":"off",statusTone:i=a?"success":"muted"}){return u`
    <div
      className="border-t border-white/[0.06] py-4 first:border-0 first:pt-0"
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-iron-200">${e}</span>
            <${I}
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
  `}function SR({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=k(),s=e?.displayName||e?.packageRef?.id||r("extensions.defaultName"),{secrets:i=[],fields:o=[],onboarding:l,isLoading:c,error:d}=mR(e?.packageRef),[m,f]=p.default.useState({}),[h,x]=p.default.useState({}),y=pR(e?.packageRef),$=fR(e?.packageRef,_=>{_.success!==!1&&(n&&n(_),a())}),g=p.default.useCallback(()=>{let _={};for(let[A,L]of Object.entries(m)){let U=(L||"").trim();U&&(_[A]=U)}$.mutate({secrets:_,fields:h})},[m,h,$]),v=p.default.useCallback(_=>{let A=window.open("about:blank","_blank","width=600,height=600");A&&(A.opener=null),y.mutate({secret:_,popup:A})},[y]),w=i.filter(_=>(_.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,S=Zh(e),C=H_({extension:e,secrets:i,fields:o}),R=iO(l?.setup_url);return c?u`
      <${kd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(_=>u`<div
                key=${_}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?u`
      <${kd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?u`
      <${kd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")}
        </p>
      <//>
    `:u`
    <${kd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
      ${l?.credential_instructions&&u`
        <p className="mb-4 text-sm leading-6 text-iron-300">
          ${l.credential_instructions}
        </p>
      `}
      ${R&&u`
        <a
          href=${R}
          target="_blank"
          rel="noopener noreferrer"
          className="mb-4 inline-flex items-center gap-1.5 text-sm text-signal hover:underline"
        >
          ${r("extensions.getCredentials")}
          <${D} name="bolt" className="h-3.5 w-3.5" />
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
                        ${_.provided?r("extensions.authConfigured"):r("extensions.authPopup")}
                      </span>
                      <${T}
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
                placeholder=${_.provided?r("extensions.keepSecretPlaceholder"):""}
                value=${m[_.name]||""}
                onChange=${A=>f(L=>({...L,[_.name]:A.target.value}))}
                onKeyDown=${A=>A.key==="Enter"&&g()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
              ${_.auto_generate&&!_.provided&&u`
                <p className="mt-1 text-xs text-iron-700">
                  ${r("extensions.autoGenerated")}
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
        <${T} variant="ghost" onClick=${a}>${r("common.cancel")}<//>
        ${C&&u`
        <${T}
          variant="primary"
          onClick=${()=>t?.(e)}
        >
          ${r("extensions.activate")}
        <//>
        `}
        ${w&&u`
        <${T}
          variant=${C?"secondary":"primary"}
          onClick=${g}
          disabled=${$.isPending}
        >
          ${$.isPending?r("common.saving"):r("common.save")}
        <//>
        `}
      </div>
    <//>
  `}function iO(e){if(!e)return null;try{let t=new URL(String(e));return t.protocol==="https:"?t.href:null}catch{return null}}function kd({onClose:e,title:t,children:a}){let n=p.default.useId();return p.default.useEffect(()=>{let r=s=>{s.key==="Escape"&&e()};return window.addEventListener("keydown",r),()=>window.removeEventListener("keydown",r)},[e]),u`
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
            <${D} name="close" className="h-4 w-4" />
          </button>
        </div>
        ${a}
      </div>
    </div>
  `}function NR(e){return e.package_ref?.id||""}function _R({mcpServers:e,mcpRegistry:t,onActivate:a,onConfigure:n,onRemove:r,onInstall:s,isBusy:i}){let o=k();return e.length===0&&t.length===0?u`
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
                <${Ri}
                  key=${NR(l)}
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
                  key=${NR(l)}
                  entry=${l}
                  onInstall=${s}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function oO(e){return e?.package_ref?.id||""}function lO(e){return e.entry||e.extension||{}}function RR({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=k(),[o,l]=p.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(y=>{let $=lO(y);return($.display_name||oO($)).toLowerCase().includes(c)||($.description||"").toLowerCase().includes(c)||($.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,m=d.filter(y=>y.installed&&y.extension),f=d.filter(y=>y.installed&&!y.extension&&y.entry),h=m.length+f.length,x=d.filter(y=>!y.installed&&y.entry);return e.length===0?u`
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
                      <${Ri}
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
  `}function tv(){let{tab:e="registry"}=it(),[t,a]=p.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:l,connectableChannels:c,isLoading:d,isBusy:m,actionResult:f,clearResult:h,install:x,activate:y,remove:$,invalidate:g}=dR(),v=p.default.useCallback(_=>a(_),[]),b=p.default.useCallback(_=>x({..._,onNeedsSetup:v}),[v,x]),w=p.default.useCallback(()=>a(null),[]),S=p.default.useCallback(()=>g(),[g]),C=p.default.useCallback(_=>{_&&(y(_),a(null))},[y]);if(d)return u`
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
    `;if(e==="installed")return u`<${ot} to="/extensions/registry" replace />`;let R={channels:u`<${wR}
      status=${n}
      channels=${r}
      connectableChannels=${c}
      channelRegistry=${i}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${b}
      isBusy=${m}
    />`,mcp:u`<${_R}
      mcpServers=${s}
      mcpRegistry=${o}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${b}
      isBusy=${m}
    />`,registry:u`<${RR}
      catalogEntries=${l}
      onInstall=${b}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      isBusy=${m}
    />`};return R[e]?u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <${S_} result=${f} onDismiss=${h} />
          ${R[e]}
        </div>
      </div>

      ${t&&u`
        <${SR}
          extension=${t}
          onActivate=${C}
          onClose=${w}
          onSaved=${S}
        />
      `}
    </div>
  `:u`<${ot} to="/extensions/registry" replace />`}var kR=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],CR=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],ER=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],av=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function TR(e){return String(e||"").trim().toLowerCase()}function AR(e){if(e==null)return"";if(Array.isArray(e))return e.map(AR).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function tt(e,t){let a=TR(e);return a?t.map(AR).join(" ").toLowerCase().includes(a):!0}function Ei(e,t,a,n){let r=TR(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(l=>tt(r,[i,l.key,l.labelKey?n(l.labelKey):l.label,l.descKey?n(l.descKey):l.description,t[l.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function uO({visible:e}){let t=k();return e?u`
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      ${t("tools.saved")}
    </span>
  `:null}function cO({checked:e,onChange:t,label:a}){return u`
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
  `}function dO({field:e,value:t,onSave:a,isSaved:n}){let r=k(),[s,i]=p.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",l=e.descKey?r(e.descKey):e.description||"";p.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=p.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let m=parseInt(d,10);isNaN(m)||a(e.key,m)}else if(e.type==="float"){let m=parseFloat(d);isNaN(m)||a(e.key,m)}else a(e.key,d)},[e.key,e.type,a]);return u`
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">${o}</div>
        ${l&&u`<div className="mt-1 text-xs leading-5 text-iron-300">${l}</div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${e.type==="boolean"?u`
              <${cO}
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
        <${uO} visible=${n} />
      </div>
    </div>
  `}function Ti({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=k(),o=t?i(t):e||"";return u`
    <${ne} className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${o}</h3>
      <div>
        ${a.map(l=>u`
              <${dO}
                key=${l.key}
                field=${l}
                value=${n[l.key]}
                onSave=${r}
                isSaved=${s[l.key]}
              />
            `)}
      </div>
    <//>
  `}function Rt({query:e}){let t=k();return u`
    <${ne} padding="lg">
      <div className="flex items-center gap-3">
        <span
          className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-faint)]"
        >
          <${D} name="search" className="h-4 w-4" />
        </span>
        <div className="min-w-0">
          <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
            ${t("settings.noMatchingSettings",{query:e})}
          </h3>
        </div>
      </div>
    <//>
  `}function DR({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=k();if(n)return u`<${mO} />`;let i=Ei(CR,e,r,s);return i.length===0?u`<${Rt} query=${r} />`:u`
    <div className="space-y-5">
      ${i.map(o=>u`
            <${Ti}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function mO(){return u`
    <div className="space-y-5">
      ${[1,2,3].map(e=>u`
            <${ne} key=${e} padding="md">
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
  `}function MR(){let e=K({queryKey:["gateway-status-settings"],queryFn:si,staleTime:1e4}),t=K({queryKey:["extensions"],queryFn:mw}),a=K({queryKey:["extension-registry"],queryFn:fw}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(m=>m.kind==="wasm_channel"||m.kind==="channel"),o=s.filter(m=>(m.kind==="wasm_channel"||m.kind==="channel")&&!m.installed),l=r.filter(m=>m.kind==="mcp_server"),c=s.filter(m=>m.kind==="mcp_server"&&!m.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:l,mcpRegistry:c,extensions:r,isLoading:d}}function fO({name:e,description:t,enabled:a,detail:n}){let r=k();return u`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${e}</span>
          <${I}
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
  `}function OR({channel:e,registryEntry:t}){let a=k(),n=t?.display_name||e?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},l={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return u`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${n}</span>
          ${s?u`<${I}
                tone=${o[i]||"muted"}
                label=${l[i]||i}
                size="sm"
              />`:u`<${I}
                tone="muted"
                label=${a("channels.available")}
                size="sm"
              />`}
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${r}</div>
      </div>
    </div>
  `}function pO(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function hO({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=pO(e,i).filter(x=>tt(s,[i("channels.builtIn"),x.id,x.name,x.description,x.detail])),l=new Set(t.map(x=>x.name)),c=t.filter(x=>tt(s,[i("channels.messaging"),x.name,x.display_name,x.description,x.onboarding_state])),d=a.filter(x=>!l.has(x.name)).filter(x=>tt(s,[i("channels.messaging"),x.name,x.display_name,x.description])),m=new Set(n.map(x=>x.name)),f=n.filter(x=>tt(s,[i("channels.mcpServers"),x.name,x.display_name,x.description,x.active?i("channels.active"):i("channels.inactive")])),h=r.filter(x=>!m.has(x.name)).filter(x=>tt(s,[i("channels.mcpServers"),x.name,x.display_name,x.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:h}}function LR({searchQuery:e=""}){let t=k(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=MR();if(o)return u`
      <div className="space-y-5">
        <${ne} padding="md">
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
    `;let{builtInChannels:l,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:f}=hO({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return l.length===0&&c.length===0&&d.length===0&&m.length===0&&f.length===0?u`<${Rt} query=${e} />`:u`
    <div className="space-y-5">
      ${l.length>0&&u`
      <${ne} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${l.map(h=>u`
            <${fO}
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
        <${ne} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.messaging")}
          </h3>
          ${c.map(h=>u`
              <${OR}
                key=${h.name}
                channel=${h}
                registryEntry=${r.find(x=>x.name===h.name)}
              />
            `)}
          ${d.map(h=>u`
              <${OR} key=${h.name} registryEntry=${h} />
            `)}
        <//>
      `}
      ${(m.length>0||f.length>0)&&u`
        <${ne} padding="md">
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
                      <${I}
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
                      <${I}
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
  `}function PR({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:l,onNearaiWallet:c,onCodexLogin:d,loginBusy:m}){let f=k(),h=e.id===t,x=Vr(e,n),y=ui(e,n),$=Rw(e,n,t,a),g=Lc(e,n),v=kw(e),b=f(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[w,S]=p.default.useState(h),C=p.default.useCallback(()=>S(lt=>!lt),[]);p.default.useEffect(()=>{S(h)},[h]);let R=x?u`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${ll(e.adapter)} · ${$||e.default_model||f("llm.none")}
      </span>`:u`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${b}
      </span>`,_=e.id==="nearai"||e.id==="openai_codex",A=e.api_key_set===!0||e.has_api_key===!0,L=e.builtin?e.id==="nearai"&&v&&!A?f("llm.addApiKey"):f("llm.configure"):f("common.edit"),U=v&&e.builtin?u`
          <${T}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${r}
            onClick=${()=>i(e)}
          >
            ${L}
          <//>
        `:null,F=!h&&e.id==="nearai"?u`
          ${U}
          <${T} type="button" variant="secondary" size="sm" disabled=${m} onClick=${c}>
            ${f("onboarding.nearWallet")}
          <//>
          <${T} type="button" variant="secondary" size="sm" disabled=${m} onClick=${()=>l("github")}>
            GitHub
          <//>
          <${T} type="button" variant="secondary" size="sm" disabled=${m} onClick=${()=>l("google")}>
            Google
          <//>
        `:!h&&e.id==="openai_codex"?u`
          <${T} type="button" variant="secondary" size="sm" disabled=${m} onClick=${d}>
            ${f("onboarding.codexSignIn")}
          <//>
        `:null,P=!h&&x&&(!_||e.id==="nearai"&&e.has_api_key===!0)?u`
        <${T}
          type="button"
          variant="primary"
          size="sm"
          disabled=${r}
          onClick=${()=>s(e)}
        >
          ${f("llm.use")}
        <//>
      `:null,G=x?null:u`
        <${T}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${r}
          onClick=${()=>i(e)}
        >
          ${f(g==="api_key"?"llm.addApiKey":"llm.configure")}
        <//>
      `,ae=h?null:P||(_?F:G),le=!_&&(e.builtin&&e.id!=="bedrock"||!e.builtin)||e.id==="nearai"&&v;return u`
    <${ne}
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
            ${h&&u`<${I} tone="positive" label=${f("llm.active")} size="sm" />`}
            ${e.builtin&&!h&&u`<${I} tone="muted" label=${f("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${R}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 py-3 pr-4 sm:pr-5">
          ${ae}
          <button
            type="button"
            onClick=${C}
            data-testid="llm-provider-chevron"
            aria-label=${f(w?"llm.collapseDetails":"llm.expandDetails")}
            className=${["grid h-7 w-7 place-items-center rounded-md text-[var(--v2-text-faint)] transition-transform hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]",w?"rotate-180":""].join(" ")}
          >
            <${D} name="chevron" className="h-4 w-4" />
          </button>
        </div>
      </div>

      ${w&&u`
        <div data-testid="llm-provider-details" className="border-t border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-4 sm:px-5">
          <div className="grid gap-3 text-xs text-[var(--v2-text-muted)] sm:grid-cols-3">
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${f("llm.adapter")}</div>
              <div className="mt-1 truncate">${ll(e.adapter)}</div>
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
            ${le&&u`
              <${T}
                type="button"
                variant="secondary"
                size="sm"
                disabled=${r}
                onClick=${()=>i(e)}
              >
                ${L}
              <//>
            `}
            ${!e.builtin&&!h&&u`
              <${T}
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
  `}var vO=[{key:"active",labelKey:"llm.groupActive",dotClass:"bg-[var(--v2-positive-text)]"},{key:"ready",labelKey:"llm.groupReady",dotClass:"bg-[var(--v2-accent)]"},{key:"setup",labelKey:"llm.groupSetup",dotClass:"bg-[var(--v2-warning-text)]"}];function gO({label:e,count:t,dotClass:a}){return u`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full "+a} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${t}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `}function UR({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=k(),r=id({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=od(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return u`<${Rt} query=${a} />`;let l=Cw(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return u`
    <${ne} className="p-4 sm:p-6">
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${n("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">${n("llm.providersDesc")}</p>
        </div>
        <${T} type="button" variant="secondary" size="sm" className="gap-2" onClick=${()=>r.openDialog(null)}>
          <${D} name="plus" className="h-3.5 w-3.5" />
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

      <${sd} login=${i} />

      ${s.isLoading?u`<div className="text-sm text-[var(--v2-text-muted)]">${n("common.loading")}</div>`:s.error?u`<div className="text-sm text-red-200">${n("error.loadFailed",{what:n("llm.providers"),message:s.error.message})}</div>`:u`
            <div className="space-y-1">
              ${vO.flatMap(c=>{let d=l[c.key];return d.length?[u`
                    <section
                      key=${c.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${c.key}
                      className="mb-3"
                    >
                      <${gO}
                        label=${n(c.labelKey)}
                        count=${d.length}
                        dotClass=${c.dotClass}
                      />
                      <div className="space-y-2">
                      ${d.map(m=>u`
                          <${PR}
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

      <${rd}
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
  `}function jR({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=k(),{activeProviderId:o,selectedModel:l,providers:c,hasActiveProvider:d}=ci({settings:e,gatewayStatus:t});if(r)return u`<${yO} />`;let m=d?o:"",f=c.find(g=>g.id===o),h=d&&(l||f?.default_model||e.selected_model)||"",x=Ei(kR,e,s,i),y=tt(s,[i("inference.provider"),i("inference.backend"),m,i("inference.model"),h]),$=tt(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!y&&!$&&x.length===0?u`<${Rt} query=${s} />`:u`
    <div className="space-y-5">
      ${y&&u`
      <${ne} padding="none" className="p-4 sm:p-5">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${i("inference.provider")}</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${i("inference.backend")}</div>
            <div className="mt-1 flex items-center gap-2">
              <span className="font-mono text-lg font-semibold text-[var(--v2-text-strong)]">${m||i("inference.none")}</span>
              ${d?u`<${I} tone="positive" label=${i("inference.active")} size="sm" />`:u`<${I} tone="muted" label=${i("llm.notConfigured")} size="sm" />`}
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
        <${UR}
          settings=${e}
          gatewayStatus=${t}
          searchQuery=${s}
        />
      `}

      ${x.map(g=>u`
            <${Ti}
              key=${g.groupKey}
              groupKey=${g.groupKey}
              fields=${g.fields}
              settings=${e}
              onSave=${a}
              savedKeys=${n}
            />
          `)}
    </div>
  `}function hr({className:e=""}){return u`
    <div
      className=${"rounded animate-pulse bg-[var(--v2-surface-muted)] "+e}
    />
  `}function yO(){return u`
    <div className="space-y-5">
      <${ne} padding="md">
        <${hr} className="mb-4 h-3 w-24" />
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${hr} className="h-3 w-16" />
            <${hr} className="mt-2 h-6 w-28" />
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${hr} className="h-3 w-16" />
            <${hr} className="mt-2 h-6 w-40" />
          </div>
        </div>
      <//>
      ${[1,2].map(e=>u`
            <${ne} key=${e} padding="md">
              <${hr} className="mb-4 h-3 w-20" />
              ${[1,2,3].map(t=>u`
                    <div key=${t} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                      <${hr} className="h-4 w-32" />
                      <${hr} className="h-9 w-36" />
                    </div>
                  `)}
            <//>
          `)}
    </div>
  `}function FR({searchQuery:e=""}){let t=k(),{lang:a,setLang:n}=wl(),r=Sl.find(i=>i.code===a)||Sl[0],s=Sl.filter(i=>tt(e,[i.code,i.name,i.native]));return s.length===0?u`<${Rt} query=${e} />`:u`
    <${ne} padding="md">
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
  `}function BR({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=k();if(n)return u`
      <div className="space-y-5">
        ${[1,2].map(o=>u`
              <${ne} key=${o} padding="md">
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
    `;let i=Ei(ER,e,r,s);return i.length===0?u`<${Rt} query=${r} />`:u`
    <div className="space-y-5">
      ${i.map(o=>u`
            <${Ti}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function zR(){let e=k(),[t,a]=p.default.useState(!1),n=p.default.useCallback(()=>a(!0),[]),r=p.default.useCallback(()=>a(!1),[]),s=p.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function qR({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=k(),r=zR({gatewayStatus:t,gatewayStatusQuery:a});return e?u`
    <div className="space-y-3">
      <div
        role="alert"
        className="flex flex-col gap-3 rounded-xl border border-copper/30 bg-copper/10 px-4 py-3 sm:flex-row sm:items-center"
      >
        <div className="flex min-w-0 flex-1 items-start gap-3">
          <${D} name="bolt" className="mt-0.5 h-4 w-4 shrink-0 text-copper" />
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

        <${T}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${!r.restartEnabled||r.isRestarting}
          onClick=${r.openConfirm}
          title=${r.restartEnabled?void 0:r.unavailableReason}
          className="w-full sm:w-auto"
        >
          <${D} name=${r.isRestarting?"pulse":"bolt"} className="h-4 w-4" />
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

    <${gi}
      open=${r.confirmOpen}
      onClose=${r.closeConfirm}
      title=${n("restart.title")}
      size="sm"
    >
      <${yi} className="space-y-3">
        <p className="text-sm text-[var(--v2-text)]">
          ${n("restart.description")}
        </p>
        <div className="rounded-xl border border-copper/25 bg-copper/10 px-3 py-2 text-xs text-copper">
          ${n("restart.warning")}
        </div>
      <//>
      <${bi}>
        <${T}
          type="button"
          variant="ghost"
          size="sm"
          disabled=${r.isRestarting}
          onClick=${r.closeConfirm}
        >
          ${n("restart.cancel")}
        <//>
        <${T}
          type="button"
          variant="danger"
          size="sm"
          disabled=${r.isRestarting}
          onClick=${r.confirmRestart}
        >
          <${D} name="bolt" className="h-4 w-4" />
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
            <${D} name="pulse" className="h-5 w-5 animate-pulse" />
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
  `:null}function IR(){let e=Z(),t=K({queryKey:["skills"],queryFn:pw}),a=Y({mutationFn:vw,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),n=Y({mutationFn:yw,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),r=Y({mutationFn:({name:c,content:d})=>gw(c,{content:d}),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),s=Y({mutationFn:({name:c,enabled:d})=>bw(c,d),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),i=Y({mutationFn:c=>xw(c),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),o=t.data?.skills||[],l=t.data?.auto_activate_learned!==!1;return{skills:o,query:t,autoActivateLearned:l,fetchSkillContent:hw,installSkill:a.mutateAsync,removeSkill:n.mutateAsync,updateSkill:r.mutateAsync,setSkillAutoActivate:s.mutateAsync,setAutoActivateLearned:i.mutateAsync,isInstalling:a.isPending,isRemoving:n.isPending,isUpdating:r.isPending,isSettingAutoActivate:s.isPending,isSettingAutoActivateLearned:i.isPending}}function HR({skill:e,onEdit:t,onRemove:a,onUpdate:n,onSetAutoActivate:r,isRemoving:s,isUpdating:i,isSettingAutoActivate:o}){let l=k(),c=e.name||e.id,d=e.trust||e.trust_level||"installed",m=e.source_kind||"installed",f=!!e.can_edit,h=!!e.can_delete,x=e.auto_activate!==!1,[y,$]=p.default.useState(!1),[g,v]=p.default.useState(""),[b,w]=p.default.useState(""),[S,C]=p.default.useState(!1);p.default.useEffect(()=>{y||(v(""),w(""))},[y]);let R=p.default.useCallback(async()=>{C(!0),w("");try{let A=await t(c);v(A?.content||""),$(!0)}catch(A){w(A.message||l("skills.contentLoadFailed"))}finally{C(!1)}},[c,t,l]),_=p.default.useCallback(async()=>{(await n(c,g))?.success&&$(!1)},[g,c,n]);return u`
    <div className="ext-card border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text)]">${c}</span>
            <${I}
              tone=${String(d).toLowerCase()==="trusted"?"positive":"muted"}
              label=${d}
              size="sm"
            />
            <${I}
              tone=${m==="system"?"positive":"muted"}
              label=${l(`skills.source.${m}`)}
              size="sm"
            />
            ${e.version&&u`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v${e.version}</span>`}
          </div>

          ${e.description&&u`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${e.description}</div>`}

          ${y?u`
                <div className="mt-3">
                  <${Kc}
                    rows=${12}
                    value=${g}
                    className="font-mono text-xs leading-5"
                    onInput=${A=>v(A.currentTarget.value)}
                  />
                </div>
              `:u`<${bO} skill=${e} />`}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          ${f&&!y&&u`
            <${T}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${i||S}
              title=${l("skills.edit")}
              onClick=${R}
            >
              <${D} name="file" className="h-4 w-4" />
              ${l(S?"skills.loading":"skills.edit")}
            <//>
          `}
          ${y&&u`
            <${T}
              type="button"
              variant="ghost"
              size="sm"
              disabled=${i}
              onClick=${()=>{v(""),$(!1)}}
            >
              <${D} name="close" className="h-4 w-4" />
              ${l("skills.cancel")}
            <//>
            <${T}
              type="button"
              variant="primary"
              size="sm"
              disabled=${i}
              onClick=${_}
            >
              <${D} name="check" className="h-4 w-4" />
              ${l(i?"skills.saving":"skills.save")}
            <//>
          `}
          ${f&&!y&&u`
            <${T}
              type="button"
              variant=${x?"secondary":"ghost"}
              size="sm"
              disabled=${o}
              title=${l(x?"skills.autoActivateOnTitle":"skills.autoActivateOffTitle")}
              onClick=${()=>r(c,!x)}
            >
              <${D} name=${x?"check":"close"} className="h-4 w-4" />
              ${l(x?"skills.autoActivateOnLabel":"skills.autoActivateOffLabel")}
            <//>
          `}
          ${h&&!y&&u`
            <${T}
              type="button"
              variant="danger"
              size="sm"
              disabled=${s}
              title=${l("skills.delete")}
              onClick=${()=>a(c)}
            >
              <${D} name="trash" className="h-4 w-4" />
              ${l("skills.delete")}
            <//>
          `}
        </div>
      </div>
      ${b&&u`<p className="mt-2 text-xs text-[var(--v2-danger-text)]">${b}</p>`}
    </div>
  `}function bO({skill:e}){let t=k();return u`
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
        ${e.has_requirements&&u`<${nv}>requirements.txt<//>`}
        ${e.has_scripts&&u`<${nv}>scripts/<//>`}
        ${e.install_source_url&&u`<${nv}>${t("skills.imported")}<//>`}
      </div>
    `}
  `}function nv({children:e}){return u`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${e}
    </span>
  `}function KR({onInstall:e,isInstalling:t}){let a=k(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState({name:"",content:""}),[c,d]=p.default.useState(""),[m,f]=p.default.useState(""),h=p.default.useCallback((y,$)=>{l(g=>!g[y]||!$.trim()?g:{...g,[y]:""})},[]),x=p.default.useCallback(async()=>{let y=xO({name:n,content:s}),$=$O(y,a);if($.name||$.content){l($),d(""),f("");return}l({name:"",content:""}),d(""),f("");try{let g=await e(y);if(!g?.success){d(g?.message||a("skills.installFailed"));return}r(""),i(""),f(g.message||a("skills.installedSuccess",{name:y.name}))}catch(g){d(g.message||a("skills.installFailed"))}},[s,n,e,a]);return u`
    <${ne} padding="md">
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

      <${En} label=${a("skills.name")} error=${o.name} required>
        <${Ot}
          size="sm"
          error=${!!o.name}
          aria-invalid=${o.name?"true":void 0}
          value=${n}
          placeholder=${a("skills.namePlaceholder")}
          onInput=${y=>{let $=y.currentTarget.value;r($),h("name",$)}}
        />
      <//>

      <${En}
        className="mt-3"
        label=${a("skills.content")}
        error=${o.content}
        hint=${a("skills.contentHint")}
        required
      >
        <${Kc}
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
        <${T} type="button" size="sm" disabled=${t} onClick=${x}>
          <${D} name="upload" className="h-4 w-4" />
          ${a(t?"skills.installing":"skills.install")}
        <//>
      </div>
    <//>
  `}function xO({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function $O(e,t){return{name:e.name?"":t("skills.nameRequired"),content:e.content?"":t("skills.contentRequired")}}function QR({searchQuery:e=""}){let t=k(),{skills:a,query:n,autoActivateLearned:r,fetchSkillContent:s,installSkill:i,removeSkill:o,updateSkill:l,setSkillAutoActivate:c,setAutoActivateLearned:d,isInstalling:m,isRemoving:f,isUpdating:h,isSettingAutoActivate:x,isSettingAutoActivateLearned:y}=IR(),[$,g]=p.default.useState(""),[v,b]=p.default.useState(""),w=p.default.useCallback(async A=>{if(window.confirm(t("skills.confirmDelete",{name:A}))){g(""),b("");try{let L=await o(A);if(!L?.success){g(L?.message||t("skills.removeFailed"));return}b(L.message||t("skills.removed",{name:A}))}catch(L){g(L.message||t("skills.removeFailed"))}}},[o,t]),S=p.default.useCallback(async(A,L)=>{if(!L.trim())return g(t("skills.contentRequired")),b(""),{success:!1,message:t("skills.contentRequired")};g(""),b("");try{let U=await l({name:A,content:L});return U?.success?(b(U.message||t("skills.updated",{name:A})),U):(g(U?.message||t("skills.updateFailed")),U)}catch(U){let F=U.message||t("skills.updateFailed");return g(F),{success:!1,message:F}}},[t,l]),C=p.default.useCallback(async(A,L)=>{g(""),b("");try{let U=await c({name:A,enabled:L});if(!U?.success){g(U?.message||t("skills.updateFailed"));return}b(U.message)}catch(U){g(U.message||t("skills.updateFailed"))}},[c,t]),R=p.default.useCallback(async A=>{g(""),b("");try{let L=await d(A);if(!L?.success){g(L?.message||t("skills.updateFailed"));return}b(L.message)}catch(L){g(L.message||t("skills.updateFailed"))}},[d,t]),_;if(n.isLoading)_=u`
      <${ne} padding="md">
          <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(A=>u`
            <div key=${A} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
              <div>
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              </div>
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
        <//>
    `;else if(n.error)_=u`
      <${ne} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">${t("skills.failedLoad",{message:n.error.message})}</p>
        <//>
    `;else{let A=a.filter(U=>tt(e,[U.name,U.id,U.description,U.keywords,U.trust_level,U.source_kind,U.version])),L=NO(A);a.length===0?_=u`
        <${ne} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:A.length===0?_=u`<${Rt} query=${e} />`:_=u`
        <div id="skills-list">
          ${L.map(U=>u`
              <${SO}
                key=${U.id}
                title=${t(U.labelKey)}
                skills=${U.skills}
                onEdit=${s}
                onRemove=${w}
                onUpdate=${S}
                onSetAutoActivate=${C}
                isRemoving=${f}
                isUpdating=${h}
                isSettingAutoActivate=${x}
              />
            `)}
        </div>
      `}return u`
    <div className="space-y-4">
      <${wO}
        enabled=${r}
        isSaving=${y}
        onToggle=${R}
      />
      <${KR} onInstall=${i} isInstalling=${m} />
      <${_O} error=${$} result=${v} />
      ${_}
    </div>
  `}function wO({enabled:e,isSaving:t,onToggle:a}){let n=k();return u`
    <${ne} padding="md" style=${e?void 0:{background:"var(--v2-danger-soft)"}}>
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
          <${T}
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
  `}function SO({title:e,skills:t,onEdit:a,onRemove:n,onUpdate:r,onSetAutoActivate:s,isRemoving:i,isUpdating:o,isSettingAutoActivate:l}){return t.length===0?null:u`
    <${ne} padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${e}
      </h3>
      ${t.map(c=>u`
          <${HR}
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
  `}function NO(e){let t=[{id:"user",labelKey:"skills.group.user",skills:[]},{id:"system",labelKey:"skills.group.system",skills:[]},{id:"workspace",labelKey:"skills.group.workspace",skills:[]}],a=t[0];for(let n of e){let r=n.source_kind||"";(r==="system"?t[1]:r==="workspace"?t[2]:a).skills.push(n)}return t.filter(n=>n.skills.length>0)}function _O({error:e,result:t}){return!e&&!t?null:u`
    <div
      className=${e?"rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200":"rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200"}
    >
      ${e||t}
    </div>
  `}function RO(e,t){vi(new Blob([JSON.stringify(t,null,2)],{type:"application/json"}),e)}function kO(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{try{t(JSON.parse(n.result))}catch(r){a(r)}},n.onerror=()=>a(n.error||new Error("Unable to read file")),n.readAsText(e)})}function VR({settingsExport:e,onImport:t,isImporting:a,searchQuery:n,onSearchChange:r,onSearchClear:s,onBack:i,canGoBack:o}){let l=k(),c=p.default.useRef(null),d=p.default.useRef(null),[m,f]=p.default.useState(null),h=p.default.useCallback(($,g)=>{d.current&&window.clearTimeout(d.current),f({tone:$,text:g}),d.current=window.setTimeout(()=>f(null),3500)},[]);p.default.useEffect(()=>()=>{d.current&&window.clearTimeout(d.current)},[]);let x=p.default.useCallback(()=>{e&&(RO("ironclaw-settings.json",e),h("success",l("settings.exportSuccess")))},[e,h,l]),y=p.default.useCallback(async $=>{let g=$.target.files?.[0];if($.target.value="",!!g)try{let v=await kO(g);if(!v||typeof v!="object"||!v.settings||typeof v.settings!="object"||Array.isArray(v.settings))throw new Error(l("settings.importInvalid"));await t(v),h("success",l("settings.importSuccess"))}catch(v){h("error",l("settings.importFailed",{message:v.message}))}},[t,h,l]);return u`
    <div className="rounded-md border border-white/10 bg-white/[0.03] px-3 py-3">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center">
        <div className="flex min-w-0 flex-1 flex-col gap-3 sm:flex-row sm:items-center">
          ${o&&u`
            <${T}
              type="button"
              variant="ghost"
              size="sm"
              onClick=${i}
              className="w-fit gap-2"
            >
              <${D} name="chevron" className="h-3.5 w-3.5 rotate-90" />
              ${l("settings.back")}
            <//>
          `}

          <label className="relative min-w-0 flex-1">
            <span className="sr-only">${l("settings.searchPlaceholder")}</span>
            <${D}
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
                <${D} name="close" className="h-3.5 w-3.5" />
              </button>
            `}
          </label>
        </div>

        <div className="flex shrink-0 flex-wrap gap-2">
          <${T}
            type="button"
            variant="secondary"
            size="sm"
            onClick=${x}
            disabled=${!e||a}
            className="gap-2"
          >
            <${D} name="download" className="h-3.5 w-3.5" />
            ${l("settings.export")}
          <//>
          <${T}
            type="button"
            variant="secondary"
            size="sm"
            onClick=${()=>c.current?.click()}
            disabled=${a}
            className="gap-2"
          >
            <${D} name="upload" className="h-3.5 w-3.5" />
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
  `}function Cd(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function GR(){let e=Z(),t=K({queryKey:["settings-tools"],queryFn:cw}),a=t.data?.tools||[],[n,r]=p.default.useState({}),s=Y({mutationFn:async({name:o,state:l})=>Cd(await dw(o,l),"Save failed"),onSuccess:(o,{name:l,state:c})=>{e.setQueryData(["settings-tools"],d=>{if(!d)return d;let m=o?.tool;return{...d,tools:d.tools.map(f=>f.name===l?{...f,state:c,...m||{}}:f)}}),r(d=>({...d,[l]:!0})),setTimeout(()=>r(d=>({...d,[l]:!1})),2e3)}}),i=p.default.useCallback((o,l)=>s.mutate({name:o,state:l}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}var rv="agent.auto_approve_tools";function YR(e,t){let a=`tools.description.${t.name}`,n=e(a);return n&&n!==a?n:t.description||""}function CO({visible:e}){let t=k();return e?u`
    <span className="font-mono text-[11px] text-[var(--v2-accent-text)]" role="status">
      ${t("tools.saved")}
    </span>
  `:null}function EO({checked:e,disabled:t=!1,label:a,onChange:n}){return u`
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
  `}function sv({settings:e,onSave:t,savedKeys:a,isLoading:n}){let r=k(),s=r("settings.field.autoApproveEligibleTools"),i=e?.[rv],o=i==null?!0:i===!0||i==="true";return u`
    <${ne} padding="md" className="flex items-center justify-between gap-6">
      <div className="min-w-0">
        <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
          ${s}
        </h3>
        <p className="mt-1 text-sm text-[var(--v2-text-muted)]">
          ${r("settings.field.autoApproveEligibleToolsDesc")}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-3">
        <${CO} visible=${a?.[rv]} />
        <${EO}
          checked=${o}
          disabled=${n}
          label=${s}
          onChange=${l=>t(rv,l)}
        />
      </div>
    <//>
  `}function TO({tool:e,onPermissionChange:t,isSaved:a}){let n=k(),r=YR(n,e),s=[{value:"default",label:n("tools.followDefault"),tone:"neutral"},{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask_each_time",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],i={default:n("tools.sourceDefault"),global:n("tools.sourceGlobal"),override:n("tools.sourceOverride")},o=e.locked,l=s.find(f=>f.value===e.state)||s[1],c=e.effective_source||"default",d=c==="override"?e.state:"default",m=c==="default"&&e.state===e.default_state;return u`
    <div
      data-testid="settings-tool-row"
      data-tool-name=${e.name}
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="flex min-w-0 items-center gap-3">
        ${o&&u`<span data-testid="settings-tool-lock" className="shrink-0">
          <${D}
            name="lock"
            className="h-3.5 w-3.5 text-[var(--v2-text-faint)]"
          />
        </span>`}
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate font-mono text-sm text-[var(--v2-text)]"
              >${e.name}</span
            >
            ${m&&u`
              <span
                className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]"
              >
                ${n("tools.default")}
              </span>
            `}
            <span
              className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]"
            >
              ${i[c]||i.default}
            </span>
          </div>
          ${r&&u`
            <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">
              ${r}
            </div>
          `}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${o?u`<${I} tone=${l.tone} label=${l.label} size="sm" />`:u`
              <select
                value=${d}
                onChange=${f=>t(e.name,f.target.value)}
                aria-label=${n("tools.permissionFor",{name:e.name})}
                className="v2-select h-8 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2.5 font-mono text-xs text-[var(--v2-text-strong)] outline-none focus:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]"
              >
                ${s.map(f=>u`<option key=${f.value} value=${f.value}>
                      ${f.label}
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
  `}function JR({settings:e={},onSave:t=()=>{},savedKeys:a={},isLoading:n=!1,searchQuery:r=""}){let s=k(),{tools:i,query:o,setPermission:l,savedTools:c}=GR();if(o.isLoading)return u`
      <div className="space-y-4">
        <${sv}
          settings=${e}
          onSave=${t}
          savedKeys=${a}
          isLoading=${n}
        />
        <${ne} padding="md">
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
        <${sv}
          settings=${e}
          onSave=${t}
          savedKeys=${a}
          isLoading=${n}
        />
        <${ne} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">
            ${s("tools.failedLoad",{message:o.error.message})}
          </p>
        <//>
      </div>
    `;let d=i.filter(m=>{let f=YR(s,m);return tt(r,[m.name,m.description,f,m.state,m.default_state,m.effective_source,m.state==="disabled"?s("tools.disabled"):""])});return u`
    <div className="space-y-4">
      <${sv}
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

      <${ne} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${s("tools.permissions")}
        </h3>
        ${d.length===0?u`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${s("tools.noMatch")}
            </p>`:d.map(m=>u`
                  <${TO}
                    key=${m.name}
                    tool=${m}
                    onPermissionChange=${l}
                    isSaved=${c[m.name]}
                  />
                `)}
      <//>
    </div>
  `}function XR(e){return(Number(e)||0).toFixed(2)}function AO(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function WR(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function Wr({label:e,value:t,description:a}){return u`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&u`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function ZR({searchQuery:e=""}){let t=k(),{credits:a,query:n,authorize:r}=jc();if(!tt(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return u`<${Rt} query=${e} />`;let s;if(n.isLoading)s=u`
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
          value=${XR(a.pending_credit)}
        />
        <${Wr}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${XR(a.final_credit)}
        />
        <${Wr}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${AO(a.delayed_credit_delta)}
        />
        <${Wr}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${Wr}
          label=${t("traceCommons.lastSubmission")}
          value=${WR(a.last_submission_at,t)}
        />
        <${Wr}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${WR(a.last_credit_sync_at,t)}
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
    <${ne} padding="md">
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
  `}function ek(){let e=Z(),t=K({queryKey:["admin-users"],queryFn:Sw,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=Y({mutationFn:Nw,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=Y({mutationFn:({id:i,payload:o})=>_w(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function DO({onCreate:e,isCreating:t,error:a}){let n=k(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState("member"),[d,m]=p.default.useState(!1),f=h=>{h.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:l},{onSuccess:()=>{s(""),o(""),m(!1)}})};return d?u`
    <${ne} padding="md">
      <h3
        className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${n("users.newUser")}
      </h3>
      <form onSubmit=${f} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <${En} label=${n("users.displayName")} htmlFor="user-name">
            <${Ot}
              id="user-name"
              type="text"
              value=${r}
              onChange=${h=>s(h.target.value)}
              required
            />
          <//>
          <${En} label=${n("users.email")} htmlFor="user-email">
            <${Ot}
              id="user-email"
              type="email"
              value=${i}
              onChange=${h=>o(h.target.value)}
            />
          <//>
        </div>
        <${En} label=${n("users.role")} htmlFor="user-role">
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
          <${T} type="submit" disabled=${t}>
            ${n(t?"users.creating":"users.createUser")}
          <//>
          <${T}
            variant="ghost"
            type="button"
            onClick=${()=>m(!1)}
            >${n("users.cancel")}<//
          >
        </div>
      </form>
    <//>
  `:u`
      <${T} variant="secondary" onClick=${()=>m(!0)}>
        <${D} name="plus" className="mr-2 h-4 w-4" />
        ${n("users.addUser")}
      <//>
    `}function MO({user:e}){let t=k(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return u`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]"
            >${e.display_name||e.id}</span
          >
          <${I}
            tone=${n}
            label=${e.role==="admin"?t("users.admin"):t("users.member")}
            size="sm"
          />
          <${I} tone=${a} label=${e.status||"active"} size="sm" />
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
  `}function tk({searchQuery:e=""}){let t=k(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=ek();if(n.isLoading)return u`
      <${ne} padding="md">
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
      <${ne} padding="lg">
        <div className="flex items-center gap-3">
          <${D} name="lock" className="h-5 w-5 text-[var(--v2-text-faint)]" />
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">
            ${t("users.adminRequired")}
          </h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
          ${t("users.adminRequiredDesc")}
        </p>
      <//>
    `;if(n.error)return u`
      <${ne} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("users.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let l=a.filter(c=>tt(e,[c.id,c.display_name,c.email,c.role,c.status,c.last_active]));return u`
    <div className="space-y-5">
      <${DO}
        onCreate=${s}
        isCreating=${o}
        error=${i}
      />

      <${ne} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("users.title",{count:l.length})}
        </h3>
        ${a.length===0?u`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("users.noUsers")}
            </p>`:l.length===0?u`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("settings.noMatchingSettings",{query:e})}
            </p>`:l.map(c=>u`<${MO} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function ak(){let e=Z(),t=K({queryKey:["settings-export"],queryFn:tw,staleTime:3e4}),a=t.data?.settings||{},[n,r]=p.default.useState({}),[s,i]=p.default.useState(!1),o=Y({mutationFn:async({key:m,value:f})=>Cd(await rh(m,f),"Save failed"),onSuccess:(m,{key:f,value:h})=>{e.setQueryData(["settings-export"],x=>{if(!x)return x;let y={...x,settings:{...x.settings}};return h==null?delete y.settings[f]:y.settings[f]=h,y}),r(x=>({...x,[f]:!0})),setTimeout(()=>r(x=>({...x,[f]:!1})),2e3),av.has(f)&&i(!0),f==="agent.auto_approve_tools"&&e.invalidateQueries({queryKey:["settings-tools"]})}}),l=p.default.useCallback((m,f)=>o.mutate({key:m,value:f}),[o]),c=Y({mutationFn:aw,onSuccess:(m,f)=>{e.invalidateQueries({queryKey:["settings-export"]});let h=Object.keys(f?.settings||{});h.includes("agent.auto_approve_tools")&&e.invalidateQueries({queryKey:["settings-tools"]}),h.some(x=>av.has(x))&&i(!0)}}),d=p.default.useCallback(m=>c.mutateAsync(m),[c]);return{settings:a,query:t,save:l,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error,importError:c.error}}function iv(){let e=k(),{tab:t}=it(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=wa(),s=r?"inference":"language",i=t||s,{settings:o,query:l,save:c,savedKeys:d,needsRestart:m,importSettings:f,isImporting:h,saveError:x,importError:y}=ak(),[$,g]=p.default.useState("");p.default.useEffect(()=>{g("")},[i]);let v=l.isLoading,b={inference:u`<${jR}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${v}
      searchQuery=${$}
    />`,agent:u`<${DR}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${v}
      searchQuery=${$}
    />`,channels:u`<${LR} searchQuery=${$} />`,networking:u`<${BR}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${v}
      searchQuery=${$}
    />`,tools:u`<${JR}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${v}
      searchQuery=${$}
    />`,skills:u`<${QR} searchQuery=${$} />`,traces:u`<${ZR} searchQuery=${$} />`,users:u`<${tk} searchQuery=${$} />`,language:u`<${FR} searchQuery=${$} />`},w=A=>A==="users"||A==="inference",S=A=>Object.prototype.hasOwnProperty.call(b,A),C=Object.keys(b).filter(A=>r||!w(A)),_=S(s)&&C.includes(s)?s:C[0]||"language";return!S(i)||!r&&w(i)?u`<${ot} to=${`/settings/${_}`} replace />`:u`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${m&&u`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${qR}
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

            ${y&&u`
              <div
                className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
              >
                ${e("settings.importFailed",{message:y.message})}
              </div>
            `}

            <${VR}
              settingsExport=${l.data||null}
              onImport=${f}
              isImporting=${h}
              searchQuery=${$}
              onSearchChange=${g}
              onSearchClear=${()=>g("")}
              canGoBack=${!1}
            />

            ${b[i]}
          </div>
        </div>
      </div>
    </div>
  `}var ov=Object.freeze({todo:!0});function nk(){return Promise.resolve({users:[],total:0,...ov})}function rk(e){return Promise.resolve(null)}function sk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function ik(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function ok(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function lk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function uk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function ck(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function dk(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...ov})}function mk(e="day",t){return Promise.resolve({entries:[],...ov})}function fk(){return K({queryKey:["admin","usage-summary"],queryFn:dk,refetchInterval:3e4})}function Ed(e="day",t){return K({queryKey:["admin","usage",e,t],queryFn:()=>mk(e,t),refetchInterval:3e4})}function Ai(){let e=Z(),t=K({queryKey:["admin","users"],queryFn:nk,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=Y({mutationFn:sk,onSuccess:s}),o=Y({mutationFn:({id:f,payload:h})=>ik(f,h),onSuccess:s}),l=Y({mutationFn:f=>ok(f),onSuccess:s}),c=Y({mutationFn:f=>lk(f),onSuccess:s}),d=Y({mutationFn:f=>uk(f),onSuccess:s}),m=Y({mutationFn:({userId:f,name:h})=>ck(f,h)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(f,h)=>o.mutateAsync({id:f,payload:h}),deleteUser:l.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(f,h)=>m.mutateAsync({userId:f,name:h}),newToken:m.data,clearToken:()=>m.reset()}}function pk(e){return K({queryKey:["admin","user",e],queryFn:()=>rk(e),enabled:!!e,refetchInterval:1e4})}function nn(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function La(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function hk(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function vr(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function Di(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function Mi(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function Oi(e){return e==="admin"?"signal":"muted"}function vk(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function gk(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function yk(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function bk(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function xk(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function OO({users:e,onSelectUser:t}){let a=k(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?u`
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
                <td className="py-3 pr-4"><${I} tone=${Oi(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${I} tone=${Mi(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${vr(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:u`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function $k({onSelectUser:e,onNavigateTab:t}){let a=k(),n=fk(),{users:r,query:s}=Ai(),i=n.data||{},o=vk(r),l=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?u`
      <div className="space-y-5">
        <${H} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            ${[1,2,3,4].map(m=>u`<div key=${m} className="v2-skeleton h-28 rounded-lg" />`)}
          </div>
        <//>
      </div>
    `:u`
    <div className="space-y-5">
      <${H} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.systemOverview")}</h3>
          ${i.uptime_seconds!=null&&u`
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:hk(i.uptime_seconds)})}</span>
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

      <${H} className="p-5 sm:p-6">
        <h3 className="mb-5 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.usage30d")}</h3>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <${et}
            label=${a("admin.dashboard.totalJobs")}
            value=${String(c.total||0)}
            tone="muted"
          />
          <${et}
            label=${a("admin.dashboard.llmCalls")}
            value=${String(l.llm_calls||0)}
            tone="muted"
          />
          <${et}
            label=${a("admin.dashboard.totalCost")}
            value=${La(l.total_cost)}
            tone="signal"
          />
          <${et}
            label=${a("admin.dashboard.activeJobs")}
            value=${String(c.in_progress||0)}
            tone=${(c.in_progress||0)>0?"success":"muted"}
          />
        </div>
      <//>

      <${H} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.recentUsers")}</h3>
          <button
            onClick=${()=>t("users")}
            className="text-xs text-signal hover:underline"
          >
            ${a("admin.dashboard.viewAll")}
          </button>
        </div>
        <${OO} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var LO=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function PO({value:e,max:t}){let a=t>0?e/t*100:0;return u`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function wk({onSelectUser:e}){let t=k(),[a,n]=p.default.useState("day"),r=Ed(a),s=r.data?.usage||[],i=yk(s),o=bk(s),l=xk(i),c=i.length>0?i[0].cost:0;return r.isLoading?u`
      <${H} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
        <div className="grid gap-4 sm:grid-cols-4">
          ${[1,2,3,4].map(d=>u`<div key=${d} className="v2-skeleton h-28 rounded-lg" />`)}
        </div>
      <//>
    `:u`
    <div className="space-y-5">
      <${H} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.overview")}</h3>
          <div className="flex gap-1">
            ${LO.map(d=>u`
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
                <${et} label=${t("admin.usage.totalCalls")} value=${l.calls.toLocaleString()} tone="muted" />
                <${et} label=${t("admin.usage.inputTokens")} value=${nn(l.input_tokens)} tone="muted" />
                <${et} label=${t("admin.usage.outputTokens")} value=${nn(l.output_tokens)} tone="muted" />
                <${et} label=${t("admin.usage.totalCost")} value=${La(l.cost.toFixed(2))} tone="signal" />
              </div>
            `}
      <//>

      ${i.length>0&&u`
        <${H} className="p-5 sm:p-6">
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
                          ${Di(d.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${nn(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${nn(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${La(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${PO} value=${d.cost} max=${c} />
                      </td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}

      ${o.length>0&&u`
        <${H} className="p-5 sm:p-6">
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
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${nn(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${nn(d.output_tokens)}</td>
                      <td className="py-3 font-mono text-xs text-iron-100">${La(d.cost.toFixed(2))}</td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}
    </div>
  `}function gr({label:e,children:t}){return u`
    <div className="flex items-start justify-between gap-4 border-t border-white/[0.06] py-3 first:border-0 first:pt-0">
      <span className="text-xs text-iron-300">${e}</span>
      <span className="text-right text-sm text-iron-100">${t}</span>
    </div>
  `}function Sk({userId:e,onBack:t}){let a=k(),n=pk(e),r=Ed("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:l,createToken:c,newToken:d,clearToken:m}=Ai(),[f,h]=p.default.useState(null),[x,y]=p.default.useState(!1),$=n.data,g=r.data?.usage||[];if(p.default.useEffect(()=>{$&&f===null&&h($.role)},[$]),n.isLoading)return u`
      <div className="space-y-5">
        <${H} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-2 h-6 w-48 rounded" />
          <div className="v2-skeleton h-4 w-32 rounded" />
        <//>
      </div>
    `;if(n.error)return u`
      <${H} className="p-5 sm:p-6">
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

      <${H} className="p-5 sm:p-6">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <h2 className="text-2xl font-semibold tracking-tight text-white">${$.display_name||$.id}</h2>
            <div className="mt-2 flex items-center gap-2">
              <${I} tone=${Oi($.role)} label=${$.role||"member"} />
              <${I} tone=${Mi($.status)} label=${$.status||"active"} />
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            ${$.status==="active"?u`<${T} variant="secondary" onClick=${()=>s($.id)}>${a("admin.users.suspend")}<//>`:u`<${T} variant="secondary" onClick=${()=>i($.id)}>${a("admin.users.activate")}<//>`}
            <${T} variant="secondary" onClick=${w}>${a("admin.users.createToken")}<//>
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
              <${D} name="close" className="h-4 w-4" />
            </button>
          </div>
        </div>
      `}

      <div className="grid gap-5 lg:grid-cols-2">
        <${H} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.profile")}</h3>
          <${gr} label=${a("admin.user.id")}>
            <span className="font-mono text-xs">${$.id}</span>
          <//>
          <${gr} label=${a("admin.user.email")}>${$.email||a("admin.user.notSet")}<//>
          <${gr} label=${a("admin.user.created")}>${vr($.created_at)}<//>
          <${gr} label=${a("admin.user.lastLogin")}>${vr($.last_login_at)}<//>
          ${$.created_by&&u`
            <${gr} label=${a("admin.user.createdBy")}>
              <span className="font-mono text-xs">${Di($.created_by)}</span>
            <//>
          `}
        <//>

        <${H} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${gr} label=${a("admin.user.jobs")}>${$.job_count??0}<//>
          <${gr} label=${a("admin.user.totalCost")}>${La($.total_cost)}<//>
          <${gr} label=${a("admin.user.lastActive")}>${vr($.last_active_at)}<//>
        <//>
      </div>

      <${H} className="p-5 sm:p-6">
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
          <${T} onClick=${v} disabled=${!f||f===$.role}>
            ${a("admin.user.saveRole")}
          <//>
        </div>
      <//>

      <${H} className="p-5 sm:p-6">
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
                    ${g.map((S,C)=>u`
                        <tr key=${C} className="border-b border-white/[0.06] last:border-0">
                          <td className="py-3 pr-4 font-mono text-xs text-iron-100">${S.model}</td>
                          <td className="py-3 pr-4 font-mono text-xs text-iron-300">${(S.call_count||0).toLocaleString()}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${nn(S.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${nn(S.output_tokens)}</td>
                          <td className="py-3 font-mono text-xs text-iron-100">${La(S.total_cost)}</td>
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
              <${T} variant="ghost" onClick=${()=>y(!1)}>${a("admin.users.cancel")}<//>
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
  `}function UO(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function jO({token:e,onDismiss:t}){let a=k(),[n,r]=p.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return u`
    <div className="rounded-xl border border-signal/30 bg-signal/10 p-4 sm:p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-iron-100">${a("admin.users.tokenCreated")}</p>
          <p className="mt-1 text-xs text-iron-300">${a("admin.users.tokenCreatedDesc")}</p>
          <div className="mt-3 flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-md border border-iron-700 bg-iron-800/70 px-3 py-2 font-mono text-xs text-iron-100">
              ${e}
            </code>
            <${T} variant="secondary" onClick=${s}>
              ${a(n?"admin.users.copied":"admin.users.copy")}
            <//>
          </div>
        </div>
        <button onClick=${t} className="text-iron-300 hover:text-iron-100">
          <${D} name="close" className="h-4 w-4" />
        </button>
      </div>
    </div>
  `}function FO({onCreate:e,isCreating:t,error:a}){let n=k(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState("member"),[d,m]=p.default.useState(!1),f=async h=>{h.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:l}),s(""),o(""),m(!1))};return d?u`
    <${H} className="p-5 sm:p-6">
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
          <${T} type="submit" disabled=${t}>
            ${n(t?"admin.users.creating":"admin.users.createUser")}
          <//>
          <${T} variant="ghost" type="button" onClick=${()=>m(!1)}>${n("admin.users.cancel")}<//>
        </div>
      </form>
    <//>
  `:u`
      <${T} variant="secondary" onClick=${()=>m(!0)}>
        <${D} name="plus" className="mr-2 h-4 w-4" />
        ${n("admin.users.newUser")}
      <//>
    `}function BO({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=k();return u`
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick=${r}>
      <div className="w-full max-w-md rounded-xl border border-iron-700 bg-iron-900 p-6" onClick=${i=>i.stopPropagation()}>
        <h3 className="text-lg font-semibold text-iron-100">${e}</h3>
        <p className="mt-2 text-sm text-iron-300">${t}</p>
        <div className="mt-5 flex justify-end gap-2">
          <${T} variant="ghost" onClick=${r}>${s("admin.users.cancel")}<//>
          <button
            onClick=${n}
            className="v2-button inline-flex h-10 items-center justify-center rounded-md bg-[var(--v2-danger-soft)] px-4 text-sm font-semibold text-[var(--v2-danger-text)] hover:bg-[color-mix(in_srgb,var(--v2-danger-soft)_65%,var(--v2-danger-text))]"
          >
            ${a}
          </button>
        </div>
      </div>
    </div>
  `}function zO({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=k();return u`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${I} tone=${Oi(e.role)} label=${e.role||"member"} />
          <${I} tone=${Mi(e.status)} label=${e.status||"active"} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          ${e.email&&u`<span className="font-mono text-xs text-iron-300">${e.email}</span>`}
          <span className="font-mono text-xs text-iron-700">${Di(e.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-iron-300 sm:inline">
          ${e.job_count!=null?i("admin.users.jobsCount",{count:e.job_count}):""}
          ${e.total_cost!=null?` \xB7 ${La(e.total_cost)}`:""}
        </span>
        <span className="hidden text-xs text-iron-700 lg:inline">${vr(e.last_active_at)}</span>
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
  `}function Nk({selectedUserId:e,onSelectUser:t}){let a=k(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:l,updateUser:c,deleteUser:d,suspendUser:m,activateUser:f,createToken:h,newToken:x,clearToken:y}=Ai(),[$,g]=p.default.useState(""),[v,b]=p.default.useState("all"),[w,S]=p.default.useState(null),C=gk(n,{search:$,filter:v}),R=UO(a),_=L=>{S({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{m(L),S(null)}})},A=async(L,U)=>{let F=window.prompt(a("admin.users.tokenNamePrompt",{name:U||a("admin.users.userFallback")}));F&&await h(L,F)};return r.isLoading?u`
      <${H} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-3 w-24 rounded" />
        ${[1,2,3].map(L=>u`
          <div key=${L} className="flex items-center justify-between border-t border-iron-700 py-3.5 first:border-0">
            <div className="v2-skeleton h-4 w-32 rounded" />
            <div className="v2-skeleton h-6 w-20 rounded-full" />
          </div>
        `)}
      <//>
    `:s?u`
      <${H} className="p-6 sm:p-8">
        <div className="flex items-center gap-3">
          <${D} name="lock" className="h-5 w-5 text-iron-700" />
          <h3 className="text-lg font-semibold text-iron-100">${a("users.adminRequired")}</h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${a("users.adminRequiredDesc")}
        </p>
      <//>
    `:u`
    <div className="space-y-5">
      ${x&&u`
        <${jO}
          token=${x.token||x.plaintext_token}
          onDismiss=${y}
        />
      `}

      <${FO} onCreate=${i} isCreating=${o} error=${l} />

      <${H} className="p-5 sm:p-6">
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${a("admin.users.title",{count:C.length,total:n.length})}
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
              ${R.map(L=>u`
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

        ${C.length===0?u`<p className="py-4 text-sm text-iron-300">${a("admin.users.noMatch")}</p>`:C.map(L=>u`
                <${zO}
                  key=${L.id}
                  user=${L}
                  onSelect=${t}
                  onSuspend=${_}
                  onActivate=${f}
                  onChangeRole=${(U,F)=>c(U,{role:F})}
                  onCreateToken=${A}
                />
              `)}
      <//>

      ${w&&u`
        <${BO}
          title=${w.title}
          message=${w.message}
          confirmLabel=${w.confirmLabel}
          onConfirm=${w.onConfirm}
          onCancel=${()=>S(null)}
        />
      `}
    </div>
  `}function _k(){let{tab:e="dashboard"}=it(),t=ve(),[a,n]=p.default.useState(null),r=p.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=p.default.useCallback(()=>{n(null)},[]),i={dashboard:u`<${$k}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?u`<${Sk} userId=${a} onBack=${s} />`:u`<${Nk}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:u`<${wk} onSelectUser=${r} />`};return i[e]?u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:u`<${ot} to="/admin/dashboard" replace />`}var qO=2e3,IO=500,HO=2e3,KO=new Set([403,404]),QO=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function VO(e=globalThis.location,t=null){let a=new URLSearchParams(e?.search||""),n={active:[]};for(let[r,s,i]of QO){let o=a.get(s)?.trim();o?(n[r]=o,n.active.push({key:r,param:s,labelKey:i,value:o})):n[r]=null}return!n.threadId&&t&&(n.threadId=t),n}function Rk({isAdmin:e=!1,defaultThreadId:t=null}={}){let a=Ae(),n=a?.search||"",r=p.default.useMemo(()=>VO(a,t),[t,n]),{runId:s,source:i,threadId:o,toolCallId:l,toolName:c,turnId:d}=r,[m,f]=p.default.useState([]),[h,x]=p.default.useState("all"),[y,$]=p.default.useState(""),[g,v]=p.default.useState(!1),[b,w]=p.default.useState(!0),[S,C]=p.default.useState(!0),[R,_]=p.default.useState(null),A=p.default.useRef(new Set),L=p.default.useRef(0),U=!e&&!o;p.default.useEffect(()=>{L.current+=1,f([]),_(null)},[e,s,i,o,l,c,d]);let F=p.default.useCallback(async()=>{if(U){C(!1);return}let G=++L.current;C(!0);try{let ae={limit:IO,level:h==="all"?null:h,target:y.trim()||null,threadId:o,runId:s,turnId:d,toolCallId:l,toolName:c,source:i},le;try{le=await(e?d$(ae):Kp(ae))}catch(De){if(!e||!KO.has(De?.status))throw De;le=await Kp(ae)}if(G!==L.current)return;let lt=A.current,Oe=l2(le).entries.filter(De=>!lt.has(De.id));f(Oe),_(null)}catch(ae){if(G!==L.current)return;_(ae)}finally{G===L.current&&C(!1)}},[e,h,U,s,i,y,o,l,c,d]);p.default.useEffect(()=>{F()},[F]),p.default.useEffect(()=>{if(g||U)return;let G=setInterval(F,qO);return()=>clearInterval(G)},[F,U,g]);let B=p.default.useCallback(()=>{v(G=>!G)},[]),P=p.default.useCallback(()=>{let G=[...A.current,...m.map(ae=>ae.id)].slice(-HO);A.current=new Set(G),f([])},[m]);return{entries:m,totalCount:m.length,paused:g,togglePause:B,clearEntries:P,levelFilter:h,setLevelFilter:x,targetFilter:y,setTargetFilter:$,autoScroll:b,setAutoScroll:w,serverLevel:null,changeServerLevel:async()=>{},scope:r,needsThreadScope:U,status:U?"needs_scope":R?"error":S?"loading":"ready",isLoading:S,error:R}}var GO=["all","trace","debug","info","warn","error"],YO=["trace","debug","info","warn","error"],kk={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},JO={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function XO({entry:e}){let t=k(),[a,n]=p.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=kk[e.level]||kk.info,i=JO[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(l=>!!l.value);return u`
    <div data-testid="logs-entry" className=${i}>
      <div
        data-testid="logs-entry-row"
        onClick=${l=>{let c=typeof window<"u"&&window.getSelection?.();c&&!c.isCollapsed&&l.currentTarget.contains(c.anchorNode)&&l.currentTarget.contains(c.focusNode)||n(d=>!d)}}
        className=${["grid cursor-pointer select-text gap-x-3 px-4 py-1 font-mono text-xs hover:bg-[var(--v2-surface-muted)]","grid-cols-[7rem_3rem_minmax(10rem,18rem)_1fr]"].join(" ")}
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
  `}function Ck({value:e,onChange:t,options:a,labelKey:n,t:r}){return u`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>u`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function WO({label:e,value:t,scopeKey:a}){return u`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function Ek(){let e=k(),{isAdmin:t=!1,threadsState:a}=wa()||{},{entries:n,totalCount:r,paused:s,togglePause:i,clearEntries:o,levelFilter:l,setLevelFilter:c,targetFilter:d,setTargetFilter:m,autoScroll:f,setAutoScroll:h,serverLevel:x,changeServerLevel:y,scope:$,isLoading:g,error:v,needsThreadScope:b}=Rk({isAdmin:t,defaultThreadId:t?null:a?.activeThreadId||null}),w=p.default.useRef(null),S=p.default.useRef(!0);p.default.useEffect(()=>{f&&S.current&&w.current&&(w.current.scrollTop=0)},[n,f]);let C=p.default.useCallback(A=>{S.current=A.currentTarget.scrollTop<=48},[]),R=n.length>0,_=$?.active||[];return u`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${Ck}
          value=${l}
          onChange=${c}
          options=${GO}
          labelKey=${A=>A==="all"?"logs.levelAll":`logs.level.${A}`}
          t=${e}
        />

        <!-- Target filter -->
        <input
          type="text"
          value=${d}
          onInput=${A=>m(A.target.value)}
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
              onChange=${A=>h(A.target.checked)}
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

        ${_.length>0&&u`
          <div
            data-testid="logs-scope-toolbar"
            className="flex w-full flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]"
          >
            <span className="font-medium text-[var(--v2-text-strong)]">${e("logs.scoped")}</span>
            ${_.map(A=>u`<${WO} key=${A.param} scopeKey=${A.param} label=${e(A.labelKey)} value=${A.value} />`)}
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
            <${Ck}
              value=${x}
              onChange=${y}
              options=${YO}
              labelKey=${A=>`logs.level.${A}`}
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
        ${v&&R?u`
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
            `:v&&!R?u`
              <div
                className="flex h-full items-center justify-center px-6 text-center text-sm text-red-300"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:v.message||v.statusText||"Request failed"})}
              </div>
            `:g&&!R?u`
                <div
                  className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
                >
                  ${e("common.loading")}
                </div>
              `:R?n.map(A=>u`<${XO} key=${A.id} entry=${A} />`):u`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function Ak(){return u`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function ZO({auth:e}){let t=ve(),n=Ae().state?.from,r=n?`${n.pathname||Qr}${n.search||""}${n.hash||""}`:Qr,s=`/v2${r==="/"?"":r}`,i=p.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?u`<${Ak} />`:e.isAuthenticated?u`<${ot} to=${r} replace />`:u`<${z1}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function eL({auth:e,children:t}){let a=Ae();return e.isChecking?u`<${Ak} />`:e.isAuthenticated?t:u`<${ot} to="/login" replace state=${{from:a}} />`}function tL({auth:e}){return u`
    <${eL} auth=${e}>
      <${v1}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        rebornProjectsEnabled=${e.rebornProjectsEnabled}
        onSignOut=${e.signOut}
      />
    <//>
  `}function Tk({auth:e}){return e.isAdmin?u`<${_k} />`:u`<${ot} to=${Qr} replace />`}function Dk(){let e=Y$();return u`
    <${zp} basename="/v2">
      <${Pp}>
        <${xe} path="/login" element=${u`<${ZO} auth=${e} />`} />
        <${xe} path="/" element=${u`<${tL} auth=${e} />`}>
          <${xe} index element=${u`<${ot} to=${Qr} replace />`} />
          <${xe} path="overview" element=${u`<${ot} to=${Qr} replace />`} />
          <${xe} path="welcome" element=${u`<${h2} />`} />
          <${xe} path="chat" element=${u`<${Mh} />`} />
          <${xe} path="chat/:threadId" element=${u`<${Mh} />`} />
          <${xe} path="workspace" element=${u`<${Lh} />`} />
          <${xe} path="workspace/*" element=${u`<${Lh} />`} />
          <${xe} path="projects" element=${u`<${gl} />`} />
          <${xe} path="projects/:projectId" element=${u`<${gl} />`} />
          <${xe} path="projects/:projectId/missions/:missionId" element=${u`<${gl} />`} />
          <${xe} path="projects/:projectId/threads/:threadId" element=${u`<${gl} />`} />
          <${xe} path="missions" element=${u`<${Uh} />`} />
          <${xe} path="missions/:missionId" element=${u`<${Uh} />`} />
          <${xe} path="jobs" element=${u`<${Bh} />`} />
          <${xe} path="jobs/:jobId" element=${u`<${Bh} />`} />
          <${xe} path="routines" element=${u`<${qh} />`} />
          <${xe} path="routines/:routineId" element=${u`<${qh} />`} />
          <${xe} path="automations" element=${u`<${$_} />`} />
          <${xe} path="extensions" element=${u`<${tv} />`} />
          <${xe} path="extensions/:tab" element=${u`<${tv} />`} />
          <${xe} path="logs" element=${u`<${Ek} />`} />
          <${xe} path="settings" element=${u`<${iv} />`} />
          <${xe} path="settings/:tab" element=${u`<${iv} />`} />
          <${xe} path="admin" element=${u`<${Tk} auth=${e} />`} />
          <${xe} path="admin/:tab" element=${u`<${Tk} auth=${e} />`} />
        <//>
        <${xe} path="*" element=${u`<${ot} to=${Qr} replace />`} />
      <//>
    <//>
  `}uv("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","approval.viewFullCommand":"View full command","approval.showCommandPreview":"Show preview","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.tabDeclined":"Declined","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitDeclined":"declined","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.serviceUnavailable":"Service unavailable","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveEligibleTools":"Always allow eligible tools","settings.field.autoApproveEligibleToolsDesc":"Applies to tools set to \u201CFollow global\u201D. Per-tool settings override this; hard-floor approval gates still ask.","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.followDefault":"Follow global","tools.sourceDefault":"default permission","tools.sourceGlobal":"global setting","tools.sourceOverride":"per-tool override","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","tools.description.builtin.echo":"Echo a message","tools.description.builtin.time":"Get, parse, format, convert, or diff timestamps","tools.description.builtin.json":"Parse, query, stringify, and validate JSON","tools.description.builtin.http":"Perform an outbound HTTP request through host egress. Redirect responses are returned; the host transport does not follow them. Prefer GitHub extension capabilities for GitHub repository, issue, pull request, release, or workflow data when available.","tools.description.builtin.http.save":"Perform an outbound HTTP request through host egress and save the sanitized response body through scoped filesystem authority. Prefer GitHub extension capabilities for GitHub repository, issue, pull request, release, or workflow data when available.","tools.description.builtin.shell":"Execute shell commands with validation and saved-file references for large local output","tools.description.builtin.spawn_subagent":"Authorize a scoped child subagent run","tools.description.builtin.trace_commons.onboard":"Enroll this IronClaw in Trace Commons using an operator-issued invite link after explicit user consent.","tools.description.builtin.trace_commons.status":"Report Trace Commons enrollment state for the current user.","tools.description.builtin.trace_commons.credits":"Report the current user's Trace Commons credit state, balances, submission counts, and recent explanations.","tools.description.builtin.trace_commons.profile_token":"Mint a short-lived Trace Commons profile-management value for browser or manual profile setup.","tools.description.builtin.trace_commons.profile_set":"Create or update the current user's public Trace Commons community profile after explicit consent.","tools.description.builtin.profile_set":"Record a private local fact about the user's agent context: timezone, locale, or location.","tools.description.builtin.memory_search":"Search Reborn persistent memory documents in the current scope","tools.description.builtin.memory_write":"Write, append, or patch Reborn persistent memory documents in the current scope","tools.description.builtin.memory_read":"Read a Reborn persistent memory document in the current scope","tools.description.builtin.memory_tree":"List Reborn persistent memory documents as a compact tree","tools.description.builtin.read_file":"Read text files and extract text from supported document files through scoped mounts","tools.description.builtin.write_file":"Write content through scoped mounts","tools.description.builtin.list_dir":"List directory contents through scoped mounts","tools.description.builtin.glob":"Find files under a scoped directory with a glob pattern","tools.description.builtin.grep":"Search scoped file contents with grep output modes","tools.description.builtin.apply_patch":"Apply exact or fuzzy search-replace edits through scoped mounts","tools.description.builtin.skill_list":"List Reborn filesystem skills visible to the current local-dev agent","tools.description.builtin.skill_install":"Install a SKILL.md document, URL, ZIP bundle, or GitHub skill repository into the current user's skill root","tools.description.builtin.skill_remove":"Remove a user-installed Reborn filesystem skill","tools.description.builtin.trigger_create":"Create a caller-scoped scheduled trigger, either one-time or recurring","tools.description.builtin.trigger_list":"List scheduled triggers owned by the current caller scope","tools.description.builtin.trigger_remove":"Remove a caller-scoped scheduled trigger","tools.description.builtin.trigger_pause":"Pause a caller-scoped scheduled trigger so it remains retained but does not fire","tools.description.builtin.trigger_resume":"Resume a caller-scoped paused trigger so it may fire on its stored schedule","tools.description.builtin.extension_search":"Search the local Reborn extension catalog by extension, product, provider, or service name","tools.description.builtin.extension_install":"Install a searched Reborn extension into durable local-dev lifecycle state","tools.description.builtin.extension_activate":"Activate an installed Reborn extension for the model-visible local-dev capability surface","tools.description.builtin.extension_remove":"Remove an installed Reborn extension from durable local-dev lifecycle state","tools.description.nearai.web_search":"Search through the NEAR AI MCP server","tools.description.builtin.outbound_delivery_target_set":"Set the current user's final-reply outbound delivery target, such as a Slack DM or Slack channel","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","skills.defaultAutoActivationEnabled":"Default skill auto-activation enabled","skills.defaultAutoActivationDisabled":"Default skill auto-activation disabled","skills.defaultAutoActivationOnDesc":"Skills auto-activate by keyword on matching requests. Turn off to require an explicit /name.","skills.defaultAutoActivationOffDesc":"Skills run only when you type /name. Turn on to let them auto-activate by keyword.","skills.defaultAutoActivationOnButton":"Default: On","skills.defaultAutoActivationOffButton":"Default: Off","skills.autoActivateOnTitle":"Auto-activation on \u2014 runs on matching requests. Click to make it explicit-only (/name).","skills.autoActivateOffTitle":"Explicit-only \u2014 runs only when you type /name. Click to enable auto-activation.","skills.autoActivateOnLabel":"Auto-activate: On","skills.autoActivateOffLabel":"Auto-activate: Off","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Memory, files & attachments","workspace.readOnly":"Read-only","workspace.filterPlaceholder":"Filter by name\u2026","workspace.emptyDir":"This folder is empty.","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.noFiles":"No files here.","workspace.noMatches":"Nothing matches that filter.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a file","workspace.pickFileDesc":"Choose a file from the tree to preview or download it. This viewer is read-only.","workspace.parent":"Parent: {path}","workspace.download":"Download","workspace.binaryPreviewUnavailable":"No inline preview for this file type. Download it to view the contents.","workspace.fileMeta":"{mime} \xB7 {size} bytes","workspace.unableOpenDirectory":"Unable to open directory","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.filter.completed":"Completed","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.schedule.onceAt":"Once on {datetime}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.filterAction":"Show {label}","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.empty.onboardingTitle":"No automations yet","automations.empty.onboardingDescription":"Automations are created by chatting with your agent \u2014 there's no form to fill out. Ask it to do something on a schedule and it will set up a recurring automation for you.","automations.empty.examplesTitle":"Try asking your agent","automations.empty.example1":"Check the nearai/ironclaw repo every 10 minutes and summarize new issues, PRs, and commits.","automations.empty.example2":"Every weekday at 9am, send me a summary of my unread email.","automations.empty.example3":"Remind me to review open pull requests every afternoon at 3pm.","automations.empty.startInChat":"Start in chat","automations.empty.copyPrompt":"Copy prompt","automations.empty.copied":"Copied","automations.refreshing":"Refreshing\u2026","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.runs.total":"Recent runs: {count}","automations.runs.ok":"OK: {count}","automations.runs.error":"Failed: {count}","automations.runs.running":"Running: {count}","automations.runs.unknown":"Unknown: {count}","automations.runs.showingOf":"Showing {shown} of {total} recent runs","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up a project for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open project","projects.openGeneralWorkspace":"Open project","projects.noDescription":"No project description yet. The project is still being shaped by recent activity and thread history.","projects.general.label":"General project","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.activate":"Activate","extensions.setup":"Setup","extensions.install":"Install","extensions.noCapabilities":"No capabilities","extensions.defaultName":"Extension","extensions.installedSuccess":"{name} installed","extensions.activatedSuccess":"{name} activated","extensions.removedSuccess":"{name} removed","extensions.installFailed":"Install failed","extensions.activationFailed":"Activation failed","extensions.removeFailed":"Remove failed","extensions.openingAuth":"Opening authentication...","extensions.configurationRequired":"Configuration required","extensions.getCredentials":"Get credentials","extensions.keepSecretPlaceholder":"\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (leave blank to keep)","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,Mk.createRoot)(document.getElementById("v2-root")).render(u`
  <${cv}>
    <${qd} client=${Dt}>
      <${Dk} />
    <//>
  <//>
`);
