import{a as Dn,b as qe,c as Qe,d as p,e as u,f as iv,g as ov,h as $l,i as R,j as wl}from"./chunks/chunk-GE6TJDZP.js";var Rv=Dn(Al=>{"use strict";var Ik=Symbol.for("react.transitional.element"),Hk=Symbol.for("react.fragment");function _v(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:Ik,type:e,key:n,ref:t!==void 0?t:null,props:a}}Al.Fragment=Hk;Al.jsx=_v;Al.jsxs=_v});var jd=Dn((A6,kv)=>{"use strict";kv.exports=Rv()});var zv=Dn(Ue=>{"use strict";function Kd(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<Bl(r,t))e[n]=t,e[a]=r,a=n;else break e}}function Ia(e){return e.length===0?null:e[0]}function ql(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],l=i+1,c=e[l];if(0>Bl(o,a))l<r&&0>Bl(c,o)?(e[n]=c,e[l]=a,n=l):(e[n]=o,e[i]=a,n=i);else if(l<r&&0>Bl(c,a))e[n]=c,e[l]=a,n=l;else break e}}return t}function Bl(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}Ue.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(Dv=performance,Ue.unstable_now=function(){return Dv.now()}):(qd=Date,Mv=qd.now(),Ue.unstable_now=function(){return qd.now()-Mv});var Dv,qd,Mv,un=[],Ln=[],Gk=1,ma=null,wt=3,Qd=!1,qi=!1,Ii=!1,Vd=!1,Pv=typeof setTimeout=="function"?setTimeout:null,Uv=typeof clearTimeout=="function"?clearTimeout:null,Ov=typeof setImmediate<"u"?setImmediate:null;function zl(e){for(var t=Ia(Ln);t!==null;){if(t.callback===null)ql(Ln);else if(t.startTime<=e)ql(Ln),t.sortIndex=t.expirationTime,Kd(un,t);else break;t=Ia(Ln)}}function Gd(e){if(Ii=!1,zl(e),!qi)if(Ia(un)!==null)qi=!0,cs||(cs=!0,us());else{var t=Ia(Ln);t!==null&&Yd(Gd,t.startTime-e)}}var cs=!1,Hi=-1,jv=5,Fv=-1;function Bv(){return Vd?!0:!(Ue.unstable_now()-Fv<jv)}function Id(){if(Vd=!1,cs){var e=Ue.unstable_now();Fv=e;var t=!0;try{e:{qi=!1,Ii&&(Ii=!1,Uv(Hi),Hi=-1),Qd=!0;var a=wt;try{t:{for(zl(e),ma=Ia(un);ma!==null&&!(ma.expirationTime>e&&Bv());){var n=ma.callback;if(typeof n=="function"){ma.callback=null,wt=ma.priorityLevel;var r=n(ma.expirationTime<=e);if(e=Ue.unstable_now(),typeof r=="function"){ma.callback=r,zl(e),t=!0;break t}ma===Ia(un)&&ql(un),zl(e)}else ql(un);ma=Ia(un)}if(ma!==null)t=!0;else{var s=Ia(Ln);s!==null&&Yd(Gd,s.startTime-e),t=!1}}break e}finally{ma=null,wt=a,Qd=!1}t=void 0}}finally{t?us():cs=!1}}}var us;typeof Ov=="function"?us=function(){Ov(Id)}:typeof MessageChannel<"u"?(Hd=new MessageChannel,Lv=Hd.port2,Hd.port1.onmessage=Id,us=function(){Lv.postMessage(null)}):us=function(){Pv(Id,0)};var Hd,Lv;function Yd(e,t){Hi=Pv(function(){e(Ue.unstable_now())},t)}Ue.unstable_IdlePriority=5;Ue.unstable_ImmediatePriority=1;Ue.unstable_LowPriority=4;Ue.unstable_NormalPriority=3;Ue.unstable_Profiling=null;Ue.unstable_UserBlockingPriority=2;Ue.unstable_cancelCallback=function(e){e.callback=null};Ue.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):jv=0<e?Math.floor(1e3/e):5};Ue.unstable_getCurrentPriorityLevel=function(){return wt};Ue.unstable_next=function(e){switch(wt){case 1:case 2:case 3:var t=3;break;default:t=wt}var a=wt;wt=t;try{return e()}finally{wt=a}};Ue.unstable_requestPaint=function(){Vd=!0};Ue.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=wt;wt=e;try{return t()}finally{wt=a}};Ue.unstable_scheduleCallback=function(e,t,a){var n=Ue.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:Gk++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,Kd(Ln,e),Ia(un)===null&&e===Ia(Ln)&&(Ii?(Uv(Hi),Hi=-1):Ii=!0,Yd(Gd,a-n))):(e.sortIndex=r,Kd(un,e),qi||Qd||(qi=!0,cs||(cs=!0,us()))),e};Ue.unstable_shouldYield=Bv;Ue.unstable_wrapCallback=function(e){var t=wt;return function(){var a=wt;wt=t;try{return e.apply(this,arguments)}finally{wt=a}}}});var Iv=Dn((fP,qv)=>{"use strict";qv.exports=zv()});var Kv=Dn(Tt=>{"use strict";var Yk=Qe();function Hv(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Pn(){}var Et={d:{f:Pn,r:function(){throw Error(Hv(522))},D:Pn,C:Pn,L:Pn,m:Pn,X:Pn,S:Pn,M:Pn},p:0,findDOMNode:null},Jk=Symbol.for("react.portal");function Xk(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:Jk,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var Ki=Yk.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function Il(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}Tt.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=Et;Tt.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(Hv(299));return Xk(e,t,null,a)};Tt.flushSync=function(e){var t=Ki.T,a=Et.p;try{if(Ki.T=null,Et.p=2,e)return e()}finally{Ki.T=t,Et.p=a,Et.d.f()}};Tt.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,Et.d.C(e,t))};Tt.prefetchDNS=function(e){typeof e=="string"&&Et.d.D(e)};Tt.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=Il(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?Et.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&Et.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};Tt.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=Il(t.as,t.crossOrigin);Et.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&Et.d.M(e)};Tt.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=Il(a,t.crossOrigin);Et.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};Tt.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=Il(t.as,t.crossOrigin);Et.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else Et.d.m(e)};Tt.requestFormReset=function(e){Et.d.r(e)};Tt.unstable_batchedUpdates=function(e,t){return e(t)};Tt.useFormState=function(e,t,a){return Ki.H.useFormState(e,t,a)};Tt.useFormStatus=function(){return Ki.H.useHostTransitionStatus()};Tt.version="19.1.0"});var Gv=Dn((hP,Vv)=>{"use strict";function Qv(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(Qv)}catch(e){console.error(e)}}Qv(),Vv.exports=Kv()});var Jx=Dn(dc=>{"use strict";var st=Iv(),vy=Qe(),Wk=Gv();function j(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function gy(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function Mo(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function yy(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function Yv(e){if(Mo(e)!==e)throw Error(j(188))}function Zk(e){var t=e.alternate;if(!t){if(t=Mo(e),t===null)throw Error(j(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return Yv(r),e;if(s===n)return Yv(r),t;s=s.sibling}throw Error(j(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(j(189))}}if(a.alternate!==n)throw Error(j(190))}if(a.tag!==3)throw Error(j(188));return a.stateNode.current===a?e:t}function by(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=by(e),t!==null)return t;e=e.sibling}return null}var Me=Object.assign,eC=Symbol.for("react.element"),Hl=Symbol.for("react.transitional.element"),eo=Symbol.for("react.portal"),gs=Symbol.for("react.fragment"),xy=Symbol.for("react.strict_mode"),km=Symbol.for("react.profiler"),tC=Symbol.for("react.provider"),$y=Symbol.for("react.consumer"),pn=Symbol.for("react.context"),Sf=Symbol.for("react.forward_ref"),Cm=Symbol.for("react.suspense"),Em=Symbol.for("react.suspense_list"),Nf=Symbol.for("react.memo"),Fn=Symbol.for("react.lazy");Symbol.for("react.scope");var Tm=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var aC=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var Jv=Symbol.iterator;function Qi(e){return e===null||typeof e!="object"?null:(e=Jv&&e[Jv]||e["@@iterator"],typeof e=="function"?e:null)}var nC=Symbol.for("react.client.reference");function Am(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===nC?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case gs:return"Fragment";case km:return"Profiler";case xy:return"StrictMode";case Cm:return"Suspense";case Em:return"SuspenseList";case Tm:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case eo:return"Portal";case pn:return(e.displayName||"Context")+".Provider";case $y:return(e._context.displayName||"Context")+".Consumer";case Sf:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case Nf:return t=e.displayName||null,t!==null?t:Am(e.type)||"Memo";case Fn:t=e._payload,e=e._init;try{return Am(e(t))}catch{}}return null}var to=Array.isArray,se=vy.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,be=Wk.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,Nr={pending:!1,data:null,method:null,action:null},Dm=[],ys=-1;function Ja(e){return{current:e}}function dt(e){0>ys||(e.current=Dm[ys],Dm[ys]=null,ys--)}function Fe(e,t){ys++,Dm[ys]=e.current,e.current=t}var Va=Ja(null),bo=Ja(null),Yn=Ja(null),xu=Ja(null);function $u(e,t){switch(Fe(Yn,t),Fe(bo,e),Fe(Va,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?ay(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=ay(t),e=jx(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}dt(Va),Fe(Va,e)}function Us(){dt(Va),dt(bo),dt(Yn)}function Mm(e){e.memoizedState!==null&&Fe(xu,e);var t=Va.current,a=jx(t,e.type);t!==a&&(Fe(bo,e),Fe(Va,a))}function wu(e){bo.current===e&&(dt(Va),dt(bo)),xu.current===e&&(dt(xu),Eo._currentValue=Nr)}var Om=Object.prototype.hasOwnProperty,_f=st.unstable_scheduleCallback,Jd=st.unstable_cancelCallback,rC=st.unstable_shouldYield,sC=st.unstable_requestPaint,Ga=st.unstable_now,iC=st.unstable_getCurrentPriorityLevel,wy=st.unstable_ImmediatePriority,Sy=st.unstable_UserBlockingPriority,Su=st.unstable_NormalPriority,oC=st.unstable_LowPriority,Ny=st.unstable_IdlePriority,lC=st.log,uC=st.unstable_setDisableYieldValue,Oo=null,Wt=null;function Kn(e){if(typeof lC=="function"&&uC(e),Wt&&typeof Wt.setStrictMode=="function")try{Wt.setStrictMode(Oo,e)}catch{}}var Zt=Math.clz32?Math.clz32:mC,cC=Math.log,dC=Math.LN2;function mC(e){return e>>>=0,e===0?32:31-(cC(e)/dC|0)|0}var Kl=256,Ql=4194304;function $r(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Ju(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=$r(n):(i&=o,i!==0?r=$r(i):a||(a=o&~e,a!==0&&(r=$r(a))))):(o=n&~s,o!==0?r=$r(o):i!==0?r=$r(i):a||(a=n&~e,a!==0&&(r=$r(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function Lo(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function fC(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function _y(){var e=Kl;return Kl<<=1,(Kl&4194048)===0&&(Kl=256),e}function Ry(){var e=Ql;return Ql<<=1,(Ql&62914560)===0&&(Ql=4194304),e}function Xd(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function Po(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function pC(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,l=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Zt(a),m=1<<d;o[d]=0,l[d]=-1;var f=c[d];if(f!==null)for(c[d]=null,d=0;d<f.length;d++){var h=f[d];h!==null&&(h.lane&=-536870913)}a&=~m}n!==0&&ky(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function ky(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Zt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function Cy(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Zt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function Rf(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function kf(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function Ey(){var e=be.p;return e!==0?e:(e=window.event,e===void 0?32:Gx(e.type))}function hC(e,t){var a=be.p;try{return be.p=e,t()}finally{be.p=a}}var ir=Math.random().toString(36).slice(2),St="__reactFiber$"+ir,qt="__reactProps$"+ir,Gs="__reactContainer$"+ir,Lm="__reactEvents$"+ir,vC="__reactListeners$"+ir,gC="__reactHandles$"+ir,Xv="__reactResources$"+ir,Uo="__reactMarker$"+ir;function Cf(e){delete e[St],delete e[qt],delete e[Lm],delete e[vC],delete e[gC]}function bs(e){var t=e[St];if(t)return t;for(var a=e.parentNode;a;){if(t=a[Gs]||a[St]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=sy(e);e!==null;){if(a=e[St])return a;e=sy(e)}return t}e=a,a=e.parentNode}return null}function Ys(e){if(e=e[St]||e[Gs]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function ao(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(j(33))}function Es(e){var t=e[Xv];return t||(t=e[Xv]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function ut(e){e[Uo]=!0}var Ty=new Set,Ay={};function Lr(e,t){js(e,t),js(e+"Capture",t)}function js(e,t){for(Ay[e]=t,e=0;e<t.length;e++)Ty.add(t[e])}var yC=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),Wv={},Zv={};function bC(e){return Om.call(Zv,e)?!0:Om.call(Wv,e)?!1:yC.test(e)?Zv[e]=!0:(Wv[e]=!0,!1)}function ou(e,t,a){if(bC(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function Vl(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function cn(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var Wd,eg;function ps(e){if(Wd===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);Wd=t&&t[1]||"",eg=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+Wd+e+eg}var Zd=!1;function em(e,t){if(!e||Zd)return"";Zd=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var m=function(){throw Error()};if(Object.defineProperty(m.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(m,[])}catch(h){var f=h}Reflect.construct(e,[],m)}else{try{m.call()}catch(h){f=h}e.call(m.prototype)}}else{try{throw Error()}catch(h){f=h}(m=e())&&typeof m.catch=="function"&&m.catch(function(){})}}catch(h){if(h&&f&&typeof h.stack=="string")return[h.stack,f.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var l=i.split(`
`),c=o.split(`
`);for(r=n=0;n<l.length&&!l[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===l.length||r===c.length)for(n=l.length-1,r=c.length-1;1<=n&&0<=r&&l[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(l[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||l[n]!==c[r]){var d=`
`+l[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{Zd=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?ps(a):""}function xC(e){switch(e.tag){case 26:case 27:case 5:return ps(e.type);case 16:return ps("Lazy");case 13:return ps("Suspense");case 19:return ps("SuspenseList");case 0:case 15:return em(e.type,!1);case 11:return em(e.type.render,!1);case 1:return em(e.type,!0);case 31:return ps("Activity");default:return""}}function tg(e){try{var t="";do t+=xC(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function pa(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function Dy(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function $C(e){var t=Dy(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function Nu(e){e._valueTracker||(e._valueTracker=$C(e))}function My(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=Dy(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function _u(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var wC=/[\n"\\]/g;function ga(e){return e.replace(wC,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function Pm(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+pa(t)):e.value!==""+pa(t)&&(e.value=""+pa(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?Um(e,i,pa(t)):a!=null?Um(e,i,pa(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+pa(o):e.removeAttribute("name")}function Oy(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+pa(a):"",t=t!=null?""+pa(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function Um(e,t,a){t==="number"&&_u(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function Ts(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+pa(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function Ly(e,t,a){if(t!=null&&(t=""+pa(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+pa(a):""}function Py(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(j(92));if(to(n)){if(1<n.length)throw Error(j(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=pa(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function Fs(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var SC=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function ag(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||SC.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function Uy(e,t,a){if(t!=null&&typeof t!="object")throw Error(j(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&ag(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&ag(e,s,t[s])}function Ef(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var NC=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),_C=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function lu(e){return _C.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var jm=null;function Tf(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var xs=null,As=null;function ng(e){var t=Ys(e);if(t&&(e=t.stateNode)){var a=e[qt]||null;e:switch(e=t.stateNode,t.type){case"input":if(Pm(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+ga(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[qt]||null;if(!r)throw Error(j(90));Pm(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&My(n)}break e;case"textarea":Ly(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&Ts(e,!!a.multiple,t,!1)}}}var tm=!1;function jy(e,t,a){if(tm)return e(t,a);tm=!0;try{var n=e(t);return n}finally{if(tm=!1,(xs!==null||As!==null)&&(ic(),xs&&(t=xs,e=As,As=xs=null,ng(t),e)))for(t=0;t<e.length;t++)ng(e[t])}}function xo(e,t){var a=e.stateNode;if(a===null)return null;var n=a[qt]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(j(231,t,typeof a));return a}var $n=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),Fm=!1;if($n)try{ds={},Object.defineProperty(ds,"passive",{get:function(){Fm=!0}}),window.addEventListener("test",ds,ds),window.removeEventListener("test",ds,ds)}catch{Fm=!1}var ds,Qn=null,Af=null,uu=null;function Fy(){if(uu)return uu;var e,t=Af,a=t.length,n,r="value"in Qn?Qn.value:Qn.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return uu=r.slice(e,1<n?1-n:void 0)}function cu(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function Gl(){return!0}function rg(){return!1}function It(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?Gl:rg,this.isPropagationStopped=rg,this}return Me(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=Gl)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=Gl)},persist:function(){},isPersistent:Gl}),t}var Pr={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Xu=It(Pr),jo=Me({},Pr,{view:0,detail:0}),RC=It(jo),am,nm,Vi,Wu=Me({},jo,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:Df,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==Vi&&(Vi&&e.type==="mousemove"?(am=e.screenX-Vi.screenX,nm=e.screenY-Vi.screenY):nm=am=0,Vi=e),am)},movementY:function(e){return"movementY"in e?e.movementY:nm}}),sg=It(Wu),kC=Me({},Wu,{dataTransfer:0}),CC=It(kC),EC=Me({},jo,{relatedTarget:0}),rm=It(EC),TC=Me({},Pr,{animationName:0,elapsedTime:0,pseudoElement:0}),AC=It(TC),DC=Me({},Pr,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),MC=It(DC),OC=Me({},Pr,{data:0}),ig=It(OC),LC={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},PC={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},UC={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function jC(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=UC[e])?!!t[e]:!1}function Df(){return jC}var FC=Me({},jo,{key:function(e){if(e.key){var t=LC[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=cu(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?PC[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:Df,charCode:function(e){return e.type==="keypress"?cu(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?cu(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),BC=It(FC),zC=Me({},Wu,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),og=It(zC),qC=Me({},jo,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:Df}),IC=It(qC),HC=Me({},Pr,{propertyName:0,elapsedTime:0,pseudoElement:0}),KC=It(HC),QC=Me({},Wu,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),VC=It(QC),GC=Me({},Pr,{newState:0,oldState:0}),YC=It(GC),JC=[9,13,27,32],Mf=$n&&"CompositionEvent"in window,ro=null;$n&&"documentMode"in document&&(ro=document.documentMode);var XC=$n&&"TextEvent"in window&&!ro,By=$n&&(!Mf||ro&&8<ro&&11>=ro),lg=" ",ug=!1;function zy(e,t){switch(e){case"keyup":return JC.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function qy(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var $s=!1;function WC(e,t){switch(e){case"compositionend":return qy(t);case"keypress":return t.which!==32?null:(ug=!0,lg);case"textInput":return e=t.data,e===lg&&ug?null:e;default:return null}}function ZC(e,t){if($s)return e==="compositionend"||!Mf&&zy(e,t)?(e=Fy(),uu=Af=Qn=null,$s=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return By&&t.locale!=="ko"?null:t.data;default:return null}}var eE={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function cg(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!eE[e.type]:t==="textarea"}function Iy(e,t,a,n){xs?As?As.push(n):As=[n]:xs=n,t=Iu(t,"onChange"),0<t.length&&(a=new Xu("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var so=null,$o=null;function tE(e){Lx(e,0)}function Zu(e){var t=ao(e);if(My(t))return e}function dg(e,t){if(e==="change")return t}var Hy=!1;$n&&($n?(Jl="oninput"in document,Jl||(sm=document.createElement("div"),sm.setAttribute("oninput","return;"),Jl=typeof sm.oninput=="function"),Yl=Jl):Yl=!1,Hy=Yl&&(!document.documentMode||9<document.documentMode));var Yl,Jl,sm;function mg(){so&&(so.detachEvent("onpropertychange",Ky),$o=so=null)}function Ky(e){if(e.propertyName==="value"&&Zu($o)){var t=[];Iy(t,$o,e,Tf(e)),jy(tE,t)}}function aE(e,t,a){e==="focusin"?(mg(),so=t,$o=a,so.attachEvent("onpropertychange",Ky)):e==="focusout"&&mg()}function nE(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return Zu($o)}function rE(e,t){if(e==="click")return Zu(t)}function sE(e,t){if(e==="input"||e==="change")return Zu(t)}function iE(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var aa=typeof Object.is=="function"?Object.is:iE;function wo(e,t){if(aa(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!Om.call(t,r)||!aa(e[r],t[r]))return!1}return!0}function fg(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function pg(e,t){var a=fg(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=fg(a)}}function Qy(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?Qy(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function Vy(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=_u(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=_u(e.document)}return t}function Of(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var oE=$n&&"documentMode"in document&&11>=document.documentMode,ws=null,Bm=null,io=null,zm=!1;function hg(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;zm||ws==null||ws!==_u(n)||(n=ws,"selectionStart"in n&&Of(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),io&&wo(io,n)||(io=n,n=Iu(Bm,"onSelect"),0<n.length&&(t=new Xu("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=ws)))}function xr(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var Ss={animationend:xr("Animation","AnimationEnd"),animationiteration:xr("Animation","AnimationIteration"),animationstart:xr("Animation","AnimationStart"),transitionrun:xr("Transition","TransitionRun"),transitionstart:xr("Transition","TransitionStart"),transitioncancel:xr("Transition","TransitionCancel"),transitionend:xr("Transition","TransitionEnd")},im={},Gy={};$n&&(Gy=document.createElement("div").style,"AnimationEvent"in window||(delete Ss.animationend.animation,delete Ss.animationiteration.animation,delete Ss.animationstart.animation),"TransitionEvent"in window||delete Ss.transitionend.transition);function Ur(e){if(im[e])return im[e];if(!Ss[e])return e;var t=Ss[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in Gy)return im[e]=t[a];return e}var Yy=Ur("animationend"),Jy=Ur("animationiteration"),Xy=Ur("animationstart"),lE=Ur("transitionrun"),uE=Ur("transitionstart"),cE=Ur("transitioncancel"),Wy=Ur("transitionend"),Zy=new Map,qm="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");qm.push("scrollEnd");function Ea(e,t){Zy.set(e,t),Lr(t,[e])}var vg=new WeakMap;function ya(e,t){if(typeof e=="object"&&e!==null){var a=vg.get(e);return a!==void 0?a:(t={value:e,source:t,stack:tg(t)},vg.set(e,t),t)}return{value:e,source:t,stack:tg(t)}}var fa=[],Ns=0,Lf=0;function ec(){for(var e=Ns,t=Lf=Ns=0;t<e;){var a=fa[t];fa[t++]=null;var n=fa[t];fa[t++]=null;var r=fa[t];fa[t++]=null;var s=fa[t];if(fa[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&eb(a,r,s)}}function tc(e,t,a,n){fa[Ns++]=e,fa[Ns++]=t,fa[Ns++]=a,fa[Ns++]=n,Lf|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function Pf(e,t,a,n){return tc(e,t,a,n),Ru(e)}function Js(e,t){return tc(e,null,null,t),Ru(e)}function eb(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Zt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function Ru(e){if(50<go)throw go=0,cf=null,Error(j(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var _s={};function dE(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Xt(e,t,a,n){return new dE(e,t,a,n)}function Uf(e){return e=e.prototype,!(!e||!e.isReactComponent)}function bn(e,t){var a=e.alternate;return a===null?(a=Xt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function tb(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function du(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")Uf(e)&&(i=1);else if(typeof e=="string")i=d3(e,a,Va.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case Tm:return e=Xt(31,a,t,r),e.elementType=Tm,e.lanes=s,e;case gs:return _r(a.children,r,s,t);case xy:i=8,r|=24;break;case km:return e=Xt(12,a,t,r|2),e.elementType=km,e.lanes=s,e;case Cm:return e=Xt(13,a,t,r),e.elementType=Cm,e.lanes=s,e;case Em:return e=Xt(19,a,t,r),e.elementType=Em,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case tC:case pn:i=10;break e;case $y:i=9;break e;case Sf:i=11;break e;case Nf:i=14;break e;case Fn:i=16,n=null;break e}i=29,a=Error(j(130,e===null?"null":typeof e,"")),n=null}return t=Xt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function _r(e,t,a,n){return e=Xt(7,e,n,t),e.lanes=a,e}function om(e,t,a){return e=Xt(6,e,null,t),e.lanes=a,e}function lm(e,t,a){return t=Xt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var Rs=[],ks=0,ku=null,Cu=0,ha=[],va=0,Rr=null,hn=1,vn="";function wr(e,t){Rs[ks++]=Cu,Rs[ks++]=ku,ku=e,Cu=t}function ab(e,t,a){ha[va++]=hn,ha[va++]=vn,ha[va++]=Rr,Rr=e;var n=hn;e=vn;var r=32-Zt(n)-1;n&=~(1<<r),a+=1;var s=32-Zt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,hn=1<<32-Zt(t)+r|a<<r|n,vn=s+e}else hn=1<<s|a<<r|n,vn=e}function jf(e){e.return!==null&&(wr(e,1),ab(e,1,0))}function Ff(e){for(;e===ku;)ku=Rs[--ks],Rs[ks]=null,Cu=Rs[--ks],Rs[ks]=null;for(;e===Rr;)Rr=ha[--va],ha[va]=null,vn=ha[--va],ha[va]=null,hn=ha[--va],ha[va]=null}var At=null,Ie=null,ye=!1,kr=null,Ka=!1,Im=Error(j(519));function Ar(e){var t=Error(j(418,""));throw So(ya(t,e)),Im}function gg(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[St]=e,t[qt]=n,a){case"dialog":ce("cancel",t),ce("close",t);break;case"iframe":case"object":case"embed":ce("load",t);break;case"video":case"audio":for(a=0;a<Ro.length;a++)ce(Ro[a],t);break;case"source":ce("error",t);break;case"img":case"image":case"link":ce("error",t),ce("load",t);break;case"details":ce("toggle",t);break;case"input":ce("invalid",t),Oy(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),Nu(t);break;case"select":ce("invalid",t);break;case"textarea":ce("invalid",t),Py(t,n.value,n.defaultValue,n.children),Nu(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||Ux(t.textContent,a)?(n.popover!=null&&(ce("beforetoggle",t),ce("toggle",t)),n.onScroll!=null&&ce("scroll",t),n.onScrollEnd!=null&&ce("scrollend",t),n.onClick!=null&&(t.onclick=uc),t=!0):t=!1,t||Ar(e)}function yg(e){for(At=e.return;At;)switch(At.tag){case 5:case 13:Ka=!1;return;case 27:case 3:Ka=!0;return;default:At=At.return}}function Gi(e){if(e!==At)return!1;if(!ye)return yg(e),ye=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||vf(e.type,e.memoizedProps)),a=!a),a&&Ie&&Ar(e),yg(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(j(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){Ie=Ca(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}Ie=null}}else t===27?(t=Ie,or(e.type)?(e=bf,bf=null,Ie=e):Ie=t):Ie=At?Ca(e.stateNode.nextSibling):null;return!0}function Fo(){Ie=At=null,ye=!1}function bg(){var e=kr;return e!==null&&(zt===null?zt=e:zt.push.apply(zt,e),kr=null),e}function So(e){kr===null?kr=[e]:kr.push(e)}var Hm=Ja(null),jr=null,gn=null;function zn(e,t,a){Fe(Hm,t._currentValue),t._currentValue=a}function xn(e){e._currentValue=Hm.current,dt(Hm)}function Km(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function Qm(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var l=0;l<t.length;l++)if(o.context===t[l]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),Km(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(j(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),Km(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function Bo(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(j(387));if(i=i.memoizedProps,i!==null){var o=r.type;aa(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===xu.current){if(i=r.alternate,i===null)throw Error(j(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(Eo):e=[Eo])}r=r.return}e!==null&&Qm(t,e,a,n),t.flags|=262144}function Eu(e){for(e=e.firstContext;e!==null;){if(!aa(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function Dr(e){jr=e,gn=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function Nt(e){return nb(jr,e)}function Xl(e,t){return jr===null&&Dr(e),nb(e,t)}function nb(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},gn===null){if(e===null)throw Error(j(308));gn=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else gn=gn.next=t;return a}var mE=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},fE=st.unstable_scheduleCallback,pE=st.unstable_NormalPriority,nt={$$typeof:pn,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function Bf(){return{controller:new mE,data:new Map,refCount:0}}function zo(e){e.refCount--,e.refCount===0&&fE(pE,function(){e.controller.abort()})}var oo=null,Vm=0,Bs=0,Ds=null;function hE(e,t){if(oo===null){var a=oo=[];Vm=0,Bs=up(),Ds={status:"pending",value:void 0,then:function(n){a.push(n)}}}return Vm++,t.then(xg,xg),t}function xg(){if(--Vm===0&&oo!==null){Ds!==null&&(Ds.status="fulfilled");var e=oo;oo=null,Bs=0,Ds=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function vE(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var $g=se.S;se.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&hE(e,t),$g!==null&&$g(e,t)};var Cr=Ja(null);function zf(){var e=Cr.current;return e!==null?e:Ee.pooledCache}function mu(e,t){t===null?Fe(Cr,Cr.current):Fe(Cr,t.pool)}function rb(){var e=zf();return e===null?null:{parent:nt._currentValue,pool:e}}var qo=Error(j(460)),sb=Error(j(474)),ac=Error(j(542)),Gm={then:function(){}};function wg(e){return e=e.status,e==="fulfilled"||e==="rejected"}function Wl(){}function ib(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(Wl,Wl),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,Ng(e),e;default:if(typeof t.status=="string")t.then(Wl,Wl);else{if(e=Ee,e!==null&&100<e.shellSuspendCounter)throw Error(j(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,Ng(e),e}throw lo=t,qo}}var lo=null;function Sg(){if(lo===null)throw Error(j(459));var e=lo;return lo=null,e}function Ng(e){if(e===qo||e===ac)throw Error(j(483))}var Bn=!1;function qf(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function Ym(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function Jn(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function Xn(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,(Se&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=Ru(e),eb(e,null,a),t}return tc(e,n,t,a),Ru(e)}function uo(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,Cy(e,a)}}function um(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var Jm=!1;function co(){if(Jm){var e=Ds;if(e!==null)throw e}}function mo(e,t,a,n){Jm=!1;var r=e.updateQueue;Bn=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var l=o,c=l.next;l.next=null,i===null?s=c:i.next=c,i=l;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=l))}if(s!==null){var m=r.baseState;i=0,d=c=l=null,o=s;do{var f=o.lane&-536870913,h=f!==o.lane;if(h?(pe&f)===f:(n&f)===f){f!==0&&f===Bs&&(Jm=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var b=e,y=o;f=t;var $=a;switch(y.tag){case 1:if(b=y.payload,typeof b=="function"){m=b.call($,m,f);break e}m=b;break e;case 3:b.flags=b.flags&-65537|128;case 0:if(b=y.payload,f=typeof b=="function"?b.call($,m,f):b,f==null)break e;m=Me({},m,f);break e;case 2:Bn=!0}}f=o.callback,f!==null&&(e.flags|=64,h&&(e.flags|=8192),h=r.callbacks,h===null?r.callbacks=[f]:h.push(f))}else h={lane:f,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=h,l=m):d=d.next=h,i|=f;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;h=o,o=h.next,h.next=null,r.lastBaseUpdate=h,r.shared.pending=null}}while(!0);d===null&&(l=m),r.baseState=l,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),sr|=i,e.lanes=i,e.memoizedState=m}}function ob(e,t){if(typeof e!="function")throw Error(j(191,e));e.call(t)}function lb(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)ob(a[e],t)}var zs=Ja(null),Tu=Ja(0);function _g(e,t){e=Nn,Fe(Tu,e),Fe(zs,t),Nn=e|t.baseLanes}function Xm(){Fe(Tu,Nn),Fe(zs,zs.current)}function If(){Nn=Tu.current,dt(zs),dt(Tu)}var nr=0,le=null,_e=null,Je=null,Au=!1,Ms=!1,Mr=!1,Du=0,No=0,Os=null,gE=0;function Ve(){throw Error(j(321))}function Hf(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!aa(e[a],t[a]))return!1;return!0}function Kf(e,t,a,n,r,s){return nr=s,le=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,se.H=e===null||e.memoizedState===null?Fb:Bb,Mr=!1,s=a(n,r),Mr=!1,Ms&&(s=cb(t,a,n,r)),ub(e),s}function ub(e){se.H=Mu;var t=_e!==null&&_e.next!==null;if(nr=0,Je=_e=le=null,Au=!1,No=0,Os=null,t)throw Error(j(300));e===null||ct||(e=e.dependencies,e!==null&&Eu(e)&&(ct=!0))}function cb(e,t,a,n){le=e;var r=0;do{if(Ms&&(Os=null),No=0,Ms=!1,25<=r)throw Error(j(301));if(r+=1,Je=_e=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}se.H=NE,s=t(a,n)}while(Ms);return s}function yE(){var e=se.H,t=e.useState()[0];return t=typeof t.then=="function"?Io(t):t,e=e.useState()[0],(_e!==null?_e.memoizedState:null)!==e&&(le.flags|=1024),t}function Qf(){var e=Du!==0;return Du=0,e}function Vf(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function Gf(e){if(Au){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}Au=!1}nr=0,Je=_e=le=null,Ms=!1,No=Du=0,Os=null}function Ft(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return Je===null?le.memoizedState=Je=e:Je=Je.next=e,Je}function Xe(){if(_e===null){var e=le.alternate;e=e!==null?e.memoizedState:null}else e=_e.next;var t=Je===null?le.memoizedState:Je.next;if(t!==null)Je=t,_e=e;else{if(e===null)throw le.alternate===null?Error(j(467)):Error(j(310));_e=e,e={memoizedState:_e.memoizedState,baseState:_e.baseState,baseQueue:_e.baseQueue,queue:_e.queue,next:null},Je===null?le.memoizedState=Je=e:Je=Je.next=e}return Je}function Yf(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function Io(e){var t=No;return No+=1,Os===null&&(Os=[]),e=ib(Os,e,t),t=le,(Je===null?t.memoizedState:Je.next)===null&&(t=t.alternate,se.H=t===null||t.memoizedState===null?Fb:Bb),e}function nc(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return Io(e);if(e.$$typeof===pn)return Nt(e)}throw Error(j(438,String(e)))}function Jf(e){var t=null,a=le.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=le.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=Yf(),le.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=aC;return t.index++,a}function wn(e,t){return typeof t=="function"?t(e):t}function fu(e){var t=Xe();return Xf(t,_e,e)}function Xf(e,t,a){var n=e.queue;if(n===null)throw Error(j(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,l=null,c=t,d=!1;do{var m=c.lane&-536870913;if(m!==c.lane?(pe&m)===m:(nr&m)===m){var f=c.revertLane;if(f===0)l!==null&&(l=l.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),m===Bs&&(d=!0);else if((nr&f)===f){c=c.next,f===Bs&&(d=!0);continue}else m={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},l===null?(o=l=m,i=s):l=l.next=m,le.lanes|=f,sr|=f;m=c.action,Mr&&a(s,m),s=c.hasEagerState?c.eagerState:a(s,m)}else f={lane:m,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},l===null?(o=l=f,i=s):l=l.next=f,le.lanes|=m,sr|=m;c=c.next}while(c!==null&&c!==t);if(l===null?i=s:l.next=o,!aa(s,e.memoizedState)&&(ct=!0,d&&(a=Ds,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=l,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function cm(e){var t=Xe(),a=t.queue;if(a===null)throw Error(j(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);aa(s,t.memoizedState)||(ct=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function db(e,t,a){var n=le,r=Xe(),s=ye;if(s){if(a===void 0)throw Error(j(407));a=a()}else a=t();var i=!aa((_e||r).memoizedState,a);i&&(r.memoizedState=a,ct=!0),r=r.queue;var o=pb.bind(null,n,r,e);if(Ho(2048,8,o,[e]),r.getSnapshot!==t||i||Je!==null&&Je.memoizedState.tag&1){if(n.flags|=2048,qs(9,rc(),fb.bind(null,n,r,a,t),null),Ee===null)throw Error(j(349));s||(nr&124)!==0||mb(n,t,a)}return a}function mb(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=le.updateQueue,t===null?(t=Yf(),le.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function fb(e,t,a,n){t.value=a,t.getSnapshot=n,hb(t)&&vb(e)}function pb(e,t,a){return a(function(){hb(t)&&vb(e)})}function hb(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!aa(e,a)}catch{return!0}}function vb(e){var t=Js(e,2);t!==null&&ta(t,e,2)}function Wm(e){var t=Ft();if(typeof e=="function"){var a=e;if(e=a(),Mr){Kn(!0);try{a()}finally{Kn(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:wn,lastRenderedState:e},t}function gb(e,t,a,n){return e.baseState=a,Xf(e,_e,typeof n=="function"?n:wn)}function bE(e,t,a,n,r){if(sc(e))throw Error(j(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};se.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,yb(t,s)):(s.next=a.next,t.pending=a.next=s)}}function yb(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=se.T,i={};se.T=i;try{var o=a(r,n),l=se.S;l!==null&&l(i,o),Rg(e,t,o)}catch(c){Zm(e,t,c)}finally{se.T=s}}else try{s=a(r,n),Rg(e,t,s)}catch(c){Zm(e,t,c)}}function Rg(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){kg(e,t,n)},function(n){return Zm(e,t,n)}):kg(e,t,a)}function kg(e,t,a){t.status="fulfilled",t.value=a,bb(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,yb(e,a)))}function Zm(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,bb(t),t=t.next;while(t!==n)}e.action=null}function bb(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function xb(e,t){return t}function Cg(e,t){if(ye){var a=Ee.formState;if(a!==null){e:{var n=le;if(ye){if(Ie){t:{for(var r=Ie,s=Ka;r.nodeType!==8;){if(!s){r=null;break t}if(r=Ca(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){Ie=Ca(r.nextSibling),n=r.data==="F!";break e}}Ar(n)}n=!1}n&&(t=a[0])}}return a=Ft(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:xb,lastRenderedState:t},a.queue=n,a=Pb.bind(null,le,n),n.dispatch=a,n=Wm(!1),s=tp.bind(null,le,!1,n.queue),n=Ft(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=bE.bind(null,le,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function Eg(e){var t=Xe();return $b(t,_e,e)}function $b(e,t,a){if(t=Xf(e,t,xb)[0],e=fu(wn)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=Io(t)}catch(i){throw i===qo?ac:i}else n=t;t=Xe();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(le.flags|=2048,qs(9,rc(),xE.bind(null,r,a),null)),[n,s,e]}function xE(e,t){e.action=t}function Tg(e){var t=Xe(),a=_e;if(a!==null)return $b(t,a,e);Xe(),t=t.memoizedState,a=Xe();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function qs(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=le.updateQueue,t===null&&(t=Yf(),le.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function rc(){return{destroy:void 0,resource:void 0}}function wb(){return Xe().memoizedState}function pu(e,t,a,n){var r=Ft();n=n===void 0?null:n,le.flags|=e,r.memoizedState=qs(1|t,rc(),a,n)}function Ho(e,t,a,n){var r=Xe();n=n===void 0?null:n;var s=r.memoizedState.inst;_e!==null&&n!==null&&Hf(n,_e.memoizedState.deps)?r.memoizedState=qs(t,s,a,n):(le.flags|=e,r.memoizedState=qs(1|t,s,a,n))}function Ag(e,t){pu(8390656,8,e,t)}function Sb(e,t){Ho(2048,8,e,t)}function Nb(e,t){return Ho(4,2,e,t)}function _b(e,t){return Ho(4,4,e,t)}function Rb(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function kb(e,t,a){a=a!=null?a.concat([e]):null,Ho(4,4,Rb.bind(null,t,e),a)}function Wf(){}function Cb(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&Hf(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function Eb(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&Hf(t,n[1]))return n[0];if(n=e(),Mr){Kn(!0);try{e()}finally{Kn(!1)}}return a.memoizedState=[n,t],n}function Zf(e,t,a){return a===void 0||(nr&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=bx(),le.lanes|=e,sr|=e,a)}function Tb(e,t,a,n){return aa(a,t)?a:zs.current!==null?(e=Zf(e,a,n),aa(e,t)||(ct=!0),e):(nr&42)===0?(ct=!0,e.memoizedState=a):(e=bx(),le.lanes|=e,sr|=e,t)}function Ab(e,t,a,n,r){var s=be.p;be.p=s!==0&&8>s?s:8;var i=se.T,o={};se.T=o,tp(e,!1,t,a);try{var l=r(),c=se.S;if(c!==null&&c(o,l),l!==null&&typeof l=="object"&&typeof l.then=="function"){var d=vE(l,n);fo(e,t,d,ea(e))}else fo(e,t,n,ea(e))}catch(m){fo(e,t,{then:function(){},status:"rejected",reason:m},ea())}finally{be.p=s,se.T=i}}function $E(){}function ef(e,t,a,n){if(e.tag!==5)throw Error(j(476));var r=Db(e).queue;Ab(e,r,t,Nr,a===null?$E:function(){return Mb(e),a(n)})}function Db(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:Nr,baseState:Nr,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:wn,lastRenderedState:Nr},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:wn,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function Mb(e){var t=Db(e).next.queue;fo(e,t,{},ea())}function ep(){return Nt(Eo)}function Ob(){return Xe().memoizedState}function Lb(){return Xe().memoizedState}function wE(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=ea();e=Jn(a);var n=Xn(t,e,a);n!==null&&(ta(n,t,a),uo(n,t,a)),t={cache:Bf()},e.payload=t;return}t=t.return}}function SE(e,t,a){var n=ea();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},sc(e)?Ub(t,a):(a=Pf(e,t,a,n),a!==null&&(ta(a,e,n),jb(a,t,n)))}function Pb(e,t,a){var n=ea();fo(e,t,a,n)}function fo(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(sc(e))Ub(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,aa(o,i))return tc(e,t,r,0),Ee===null&&ec(),!1}catch{}finally{}if(a=Pf(e,t,r,n),a!==null)return ta(a,e,n),jb(a,t,n),!0}return!1}function tp(e,t,a,n){if(n={lane:2,revertLane:up(),action:n,hasEagerState:!1,eagerState:null,next:null},sc(e)){if(t)throw Error(j(479))}else t=Pf(e,a,n,2),t!==null&&ta(t,e,2)}function sc(e){var t=e.alternate;return e===le||t!==null&&t===le}function Ub(e,t){Ms=Au=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function jb(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,Cy(e,a)}}var Mu={readContext:Nt,use:nc,useCallback:Ve,useContext:Ve,useEffect:Ve,useImperativeHandle:Ve,useLayoutEffect:Ve,useInsertionEffect:Ve,useMemo:Ve,useReducer:Ve,useRef:Ve,useState:Ve,useDebugValue:Ve,useDeferredValue:Ve,useTransition:Ve,useSyncExternalStore:Ve,useId:Ve,useHostTransitionStatus:Ve,useFormState:Ve,useActionState:Ve,useOptimistic:Ve,useMemoCache:Ve,useCacheRefresh:Ve},Fb={readContext:Nt,use:nc,useCallback:function(e,t){return Ft().memoizedState=[e,t===void 0?null:t],e},useContext:Nt,useEffect:Ag,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,pu(4194308,4,Rb.bind(null,t,e),a)},useLayoutEffect:function(e,t){return pu(4194308,4,e,t)},useInsertionEffect:function(e,t){pu(4,2,e,t)},useMemo:function(e,t){var a=Ft();t=t===void 0?null:t;var n=e();if(Mr){Kn(!0);try{e()}finally{Kn(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=Ft();if(a!==void 0){var r=a(t);if(Mr){Kn(!0);try{a(t)}finally{Kn(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=SE.bind(null,le,e),[n.memoizedState,e]},useRef:function(e){var t=Ft();return e={current:e},t.memoizedState=e},useState:function(e){e=Wm(e);var t=e.queue,a=Pb.bind(null,le,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:Wf,useDeferredValue:function(e,t){var a=Ft();return Zf(a,e,t)},useTransition:function(){var e=Wm(!1);return e=Ab.bind(null,le,e.queue,!0,!1),Ft().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=le,r=Ft();if(ye){if(a===void 0)throw Error(j(407));a=a()}else{if(a=t(),Ee===null)throw Error(j(349));(pe&124)!==0||mb(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,Ag(pb.bind(null,n,s,e),[e]),n.flags|=2048,qs(9,rc(),fb.bind(null,n,s,a,t),null),a},useId:function(){var e=Ft(),t=Ee.identifierPrefix;if(ye){var a=vn,n=hn;a=(n&~(1<<32-Zt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=Du++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=gE++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:ep,useFormState:Cg,useActionState:Cg,useOptimistic:function(e){var t=Ft();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=tp.bind(null,le,!0,a),a.dispatch=t,[e,t]},useMemoCache:Jf,useCacheRefresh:function(){return Ft().memoizedState=wE.bind(null,le)}},Bb={readContext:Nt,use:nc,useCallback:Cb,useContext:Nt,useEffect:Sb,useImperativeHandle:kb,useInsertionEffect:Nb,useLayoutEffect:_b,useMemo:Eb,useReducer:fu,useRef:wb,useState:function(){return fu(wn)},useDebugValue:Wf,useDeferredValue:function(e,t){var a=Xe();return Tb(a,_e.memoizedState,e,t)},useTransition:function(){var e=fu(wn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:Io(e),t]},useSyncExternalStore:db,useId:Ob,useHostTransitionStatus:ep,useFormState:Eg,useActionState:Eg,useOptimistic:function(e,t){var a=Xe();return gb(a,_e,e,t)},useMemoCache:Jf,useCacheRefresh:Lb},NE={readContext:Nt,use:nc,useCallback:Cb,useContext:Nt,useEffect:Sb,useImperativeHandle:kb,useInsertionEffect:Nb,useLayoutEffect:_b,useMemo:Eb,useReducer:cm,useRef:wb,useState:function(){return cm(wn)},useDebugValue:Wf,useDeferredValue:function(e,t){var a=Xe();return _e===null?Zf(a,e,t):Tb(a,_e.memoizedState,e,t)},useTransition:function(){var e=cm(wn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:Io(e),t]},useSyncExternalStore:db,useId:Ob,useHostTransitionStatus:ep,useFormState:Tg,useActionState:Tg,useOptimistic:function(e,t){var a=Xe();return _e!==null?gb(a,_e,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:Jf,useCacheRefresh:Lb},Ls=null,_o=0;function Zl(e){var t=_o;return _o+=1,Ls===null&&(Ls=[]),ib(Ls,e,t)}function Yi(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function eu(e,t){throw t.$$typeof===eC?Error(j(525)):(e=Object.prototype.toString.call(t),Error(j(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function Dg(e){var t=e._init;return t(e._payload)}function zb(e){function t(g,v){if(e){var x=g.deletions;x===null?(g.deletions=[v],g.flags|=16):x.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=bn(g,v),g.index=0,g.sibling=null,g}function s(g,v,x){return g.index=x,e?(x=g.alternate,x!==null?(x=x.index,x<v?(g.flags|=67108866,v):x):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,x,w){return v===null||v.tag!==6?(v=om(x,g.mode,w),v.return=g,v):(v=r(v,x),v.return=g,v)}function l(g,v,x,w){var S=x.type;return S===gs?d(g,v,x.props.children,w,x.key):v!==null&&(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Fn&&Dg(S)===v.type)?(v=r(v,x.props),Yi(v,x),v.return=g,v):(v=du(x.type,x.key,x.props,null,g.mode,w),Yi(v,x),v.return=g,v)}function c(g,v,x,w){return v===null||v.tag!==4||v.stateNode.containerInfo!==x.containerInfo||v.stateNode.implementation!==x.implementation?(v=lm(x,g.mode,w),v.return=g,v):(v=r(v,x.children||[]),v.return=g,v)}function d(g,v,x,w,S){return v===null||v.tag!==7?(v=_r(x,g.mode,w,S),v.return=g,v):(v=r(v,x),v.return=g,v)}function m(g,v,x){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=om(""+v,g.mode,x),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case Hl:return x=du(v.type,v.key,v.props,null,g.mode,x),Yi(x,v),x.return=g,x;case eo:return v=lm(v,g.mode,x),v.return=g,v;case Fn:var w=v._init;return v=w(v._payload),m(g,v,x)}if(to(v)||Qi(v))return v=_r(v,g.mode,x,null),v.return=g,v;if(typeof v.then=="function")return m(g,Zl(v),x);if(v.$$typeof===pn)return m(g,Xl(g,v),x);eu(g,v)}return null}function f(g,v,x,w){var S=v!==null?v.key:null;if(typeof x=="string"&&x!==""||typeof x=="number"||typeof x=="bigint")return S!==null?null:o(g,v,""+x,w);if(typeof x=="object"&&x!==null){switch(x.$$typeof){case Hl:return x.key===S?l(g,v,x,w):null;case eo:return x.key===S?c(g,v,x,w):null;case Fn:return S=x._init,x=S(x._payload),f(g,v,x,w)}if(to(x)||Qi(x))return S!==null?null:d(g,v,x,w,null);if(typeof x.then=="function")return f(g,v,Zl(x),w);if(x.$$typeof===pn)return f(g,v,Xl(g,x),w);eu(g,x)}return null}function h(g,v,x,w,S){if(typeof w=="string"&&w!==""||typeof w=="number"||typeof w=="bigint")return g=g.get(x)||null,o(v,g,""+w,S);if(typeof w=="object"&&w!==null){switch(w.$$typeof){case Hl:return g=g.get(w.key===null?x:w.key)||null,l(v,g,w,S);case eo:return g=g.get(w.key===null?x:w.key)||null,c(v,g,w,S);case Fn:var k=w._init;return w=k(w._payload),h(g,v,x,w,S)}if(to(w)||Qi(w))return g=g.get(x)||null,d(v,g,w,S,null);if(typeof w.then=="function")return h(g,v,x,Zl(w),S);if(w.$$typeof===pn)return h(g,v,x,Xl(v,w),S);eu(v,w)}return null}function b(g,v,x,w){for(var S=null,k=null,N=v,C=v=0,P=null;N!==null&&C<x.length;C++){N.index>C?(P=N,N=null):P=N.sibling;var L=f(g,N,x[C],w);if(L===null){N===null&&(N=P);break}e&&N&&L.alternate===null&&t(g,N),v=s(L,v,C),k===null?S=L:k.sibling=L,k=L,N=P}if(C===x.length)return a(g,N),ye&&wr(g,C),S;if(N===null){for(;C<x.length;C++)N=m(g,x[C],w),N!==null&&(v=s(N,v,C),k===null?S=N:k.sibling=N,k=N);return ye&&wr(g,C),S}for(N=n(N);C<x.length;C++)P=h(N,g,C,x[C],w),P!==null&&(e&&P.alternate!==null&&N.delete(P.key===null?C:P.key),v=s(P,v,C),k===null?S=P:k.sibling=P,k=P);return e&&N.forEach(function(U){return t(g,U)}),ye&&wr(g,C),S}function y(g,v,x,w){if(x==null)throw Error(j(151));for(var S=null,k=null,N=v,C=v=0,P=null,L=x.next();N!==null&&!L.done;C++,L=x.next()){N.index>C?(P=N,N=null):P=N.sibling;var U=f(g,N,L.value,w);if(U===null){N===null&&(N=P);break}e&&N&&U.alternate===null&&t(g,N),v=s(U,v,C),k===null?S=U:k.sibling=U,k=U,N=P}if(L.done)return a(g,N),ye&&wr(g,C),S;if(N===null){for(;!L.done;C++,L=x.next())L=m(g,L.value,w),L!==null&&(v=s(L,v,C),k===null?S=L:k.sibling=L,k=L);return ye&&wr(g,C),S}for(N=n(N);!L.done;C++,L=x.next())L=h(N,g,C,L.value,w),L!==null&&(e&&L.alternate!==null&&N.delete(L.key===null?C:L.key),v=s(L,v,C),k===null?S=L:k.sibling=L,k=L);return e&&N.forEach(function(F){return t(g,F)}),ye&&wr(g,C),S}function $(g,v,x,w){if(typeof x=="object"&&x!==null&&x.type===gs&&x.key===null&&(x=x.props.children),typeof x=="object"&&x!==null){switch(x.$$typeof){case Hl:e:{for(var S=x.key;v!==null;){if(v.key===S){if(S=x.type,S===gs){if(v.tag===7){a(g,v.sibling),w=r(v,x.props.children),w.return=g,g=w;break e}}else if(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Fn&&Dg(S)===v.type){a(g,v.sibling),w=r(v,x.props),Yi(w,x),w.return=g,g=w;break e}a(g,v);break}else t(g,v);v=v.sibling}x.type===gs?(w=_r(x.props.children,g.mode,w,x.key),w.return=g,g=w):(w=du(x.type,x.key,x.props,null,g.mode,w),Yi(w,x),w.return=g,g=w)}return i(g);case eo:e:{for(S=x.key;v!==null;){if(v.key===S)if(v.tag===4&&v.stateNode.containerInfo===x.containerInfo&&v.stateNode.implementation===x.implementation){a(g,v.sibling),w=r(v,x.children||[]),w.return=g,g=w;break e}else{a(g,v);break}else t(g,v);v=v.sibling}w=lm(x,g.mode,w),w.return=g,g=w}return i(g);case Fn:return S=x._init,x=S(x._payload),$(g,v,x,w)}if(to(x))return b(g,v,x,w);if(Qi(x)){if(S=Qi(x),typeof S!="function")throw Error(j(150));return x=S.call(x),y(g,v,x,w)}if(typeof x.then=="function")return $(g,v,Zl(x),w);if(x.$$typeof===pn)return $(g,v,Xl(g,x),w);eu(g,x)}return typeof x=="string"&&x!==""||typeof x=="number"||typeof x=="bigint"?(x=""+x,v!==null&&v.tag===6?(a(g,v.sibling),w=r(v,x),w.return=g,g=w):(a(g,v),w=om(x,g.mode,w),w.return=g,g=w),i(g)):a(g,v)}return function(g,v,x,w){try{_o=0;var S=$(g,v,x,w);return Ls=null,S}catch(N){if(N===qo||N===ac)throw N;var k=Xt(29,N,null,g.mode);return k.lanes=w,k.return=g,k}finally{}}}var Is=zb(!0),qb=zb(!1),xa=Ja(null),Ya=null;function qn(e){var t=e.alternate;Fe(rt,rt.current&1),Fe(xa,e),Ya===null&&(t===null||zs.current!==null||t.memoizedState!==null)&&(Ya=e)}function Ib(e){if(e.tag===22){if(Fe(rt,rt.current),Fe(xa,e),Ya===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(Ya=e)}}else In(e)}function In(){Fe(rt,rt.current),Fe(xa,xa.current)}function yn(e){dt(xa),Ya===e&&(Ya=null),dt(rt)}var rt=Ja(0);function Ou(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||yf(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function dm(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:Me({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var tf={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=ea(),r=Jn(n);r.payload=t,a!=null&&(r.callback=a),t=Xn(e,r,n),t!==null&&(ta(t,e,n),uo(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=ea(),r=Jn(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=Xn(e,r,n),t!==null&&(ta(t,e,n),uo(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=ea(),n=Jn(a);n.tag=2,t!=null&&(n.callback=t),t=Xn(e,n,a),t!==null&&(ta(t,e,a),uo(t,e,a))}};function Mg(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!wo(a,n)||!wo(r,s):!0}function Og(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&tf.enqueueReplaceState(t,t.state,null)}function Or(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=Me({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var Lu=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function Hb(e){Lu(e)}function Kb(e){console.error(e)}function Qb(e){Lu(e)}function Pu(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function Lg(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function af(e,t,a){return a=Jn(a),a.tag=3,a.payload={element:null},a.callback=function(){Pu(e,t)},a}function Vb(e){return e=Jn(e),e.tag=3,e}function Gb(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){Lg(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){Lg(t,a,n),typeof r!="function"&&(Wn===null?Wn=new Set([this]):Wn.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function _E(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&Bo(t,a,r,!0),a=xa.current,a!==null){switch(a.tag){case 13:return Ya===null?df():a.alternate===null&&He===0&&(He=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===Gm?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),wm(e,n,r)),!1;case 22:return a.flags|=65536,n===Gm?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),wm(e,n,r)),!1}throw Error(j(435,a.tag))}return wm(e,n,r),df(),!1}if(ye)return t=xa.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==Im&&(e=Error(j(422),{cause:n}),So(ya(e,a)))):(n!==Im&&(t=Error(j(423),{cause:n}),So(ya(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=ya(n,a),r=af(e.stateNode,n,r),um(e,r),He!==4&&(He=2)),!1;var s=Error(j(520),{cause:n});if(s=ya(s,a),vo===null?vo=[s]:vo.push(s),He!==4&&(He=2),t===null)return!0;n=ya(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=af(a.stateNode,n,e),um(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(Wn===null||!Wn.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=Vb(r),Gb(r,e,a,n),um(a,r),!1}a=a.return}while(a!==null);return!1}var Yb=Error(j(461)),ct=!1;function gt(e,t,a,n){t.child=e===null?qb(t,null,a,n):Is(t,e.child,a,n)}function Pg(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return Dr(t),n=Kf(e,t,a,i,s,r),o=Qf(),e!==null&&!ct?(Vf(e,t,r),Sn(e,t,r)):(ye&&o&&jf(t),t.flags|=1,gt(e,t,n,r),t.child)}function Ug(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!Uf(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,Jb(e,t,s,n,r)):(e=du(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!ap(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:wo,a(i,n)&&e.ref===t.ref)return Sn(e,t,r)}return t.flags|=1,e=bn(s,n),e.ref=t.ref,e.return=t,t.child=e}function Jb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if(wo(s,n)&&e.ref===t.ref)if(ct=!1,t.pendingProps=n=s,ap(e,r))(e.flags&131072)!==0&&(ct=!0);else return t.lanes=e.lanes,Sn(e,t,r)}return nf(e,t,a,n,r)}function Xb(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return jg(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&mu(t,s!==null?s.cachePool:null),s!==null?_g(t,s):Xm(),Ib(t);else return t.lanes=t.childLanes=536870912,jg(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(mu(t,s.cachePool),_g(t,s),In(t),t.memoizedState=null):(e!==null&&mu(t,null),Xm(),In(t));return gt(e,t,r,a),t.child}function jg(e,t,a,n){var r=zf();return r=r===null?null:{parent:nt._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&mu(t,null),Xm(),Ib(t),e!==null&&Bo(e,t,n,!0),null}function hu(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(j(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function nf(e,t,a,n,r){return Dr(t),a=Kf(e,t,a,n,void 0,r),n=Qf(),e!==null&&!ct?(Vf(e,t,r),Sn(e,t,r)):(ye&&n&&jf(t),t.flags|=1,gt(e,t,a,r),t.child)}function Fg(e,t,a,n,r,s){return Dr(t),t.updateQueue=null,a=cb(t,n,a,r),ub(e),n=Qf(),e!==null&&!ct?(Vf(e,t,s),Sn(e,t,s)):(ye&&n&&jf(t),t.flags|=1,gt(e,t,a,s),t.child)}function Bg(e,t,a,n,r){if(Dr(t),t.stateNode===null){var s=_s,i=a.contextType;typeof i=="object"&&i!==null&&(s=Nt(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=tf,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},qf(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?Nt(i):_s,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(dm(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&tf.enqueueReplaceState(s,s.state,null),mo(t,n,s,r),co(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,l=Or(a,o);s.props=l;var c=s.context,d=a.contextType;i=_s,typeof d=="object"&&d!==null&&(i=Nt(d));var m=a.getDerivedStateFromProps;d=typeof m=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&Og(t,s,n,i),Bn=!1;var f=t.memoizedState;s.state=f,mo(t,n,s,r),co(),c=t.memoizedState,o||f!==c||Bn?(typeof m=="function"&&(dm(t,a,m,n),c=t.memoizedState),(l=Bn||Mg(t,a,l,n,f,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=l):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,Ym(e,t),i=t.memoizedProps,d=Or(a,i),s.props=d,m=t.pendingProps,f=s.context,c=a.contextType,l=_s,typeof c=="object"&&c!==null&&(l=Nt(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==m||f!==l)&&Og(t,s,n,l),Bn=!1,f=t.memoizedState,s.state=f,mo(t,n,s,r),co();var h=t.memoizedState;i!==m||f!==h||Bn||e!==null&&e.dependencies!==null&&Eu(e.dependencies)?(typeof o=="function"&&(dm(t,a,o,n),h=t.memoizedState),(d=Bn||Mg(t,a,d,n,f,h,l)||e!==null&&e.dependencies!==null&&Eu(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,h,l),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,h,l)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=h),s.props=n,s.state=h,s.context=l,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,hu(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=Is(t,e.child,null,r),t.child=Is(t,null,a,r)):gt(e,t,a,r),t.memoizedState=s.state,e=t.child):e=Sn(e,t,r),e}function zg(e,t,a,n){return Fo(),t.flags|=256,gt(e,t,a,n),t.child}var mm={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function fm(e){return{baseLanes:e,cachePool:rb()}}function pm(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=ba),e}function Wb(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(rt.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(ye){if(r?qn(t):In(t),ye){var o=Ie,l;if(l=o){e:{for(l=o,o=Ka;l.nodeType!==8;){if(!o){o=null;break e}if(l=Ca(l.nextSibling),l===null){o=null;break e}}o=l}o!==null?(t.memoizedState={dehydrated:o,treeContext:Rr!==null?{id:hn,overflow:vn}:null,retryLane:536870912,hydrationErrors:null},l=Xt(18,null,null,0),l.stateNode=o,l.return=t,t.child=l,At=t,Ie=null,l=!0):l=!1}l||Ar(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return yf(o)?t.lanes=32:t.lanes=536870912,null;yn(t)}return o=n.children,n=n.fallback,r?(In(t),r=t.mode,o=Uu({mode:"hidden",children:o},r),n=_r(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=fm(a),r.childLanes=pm(e,i,a),t.memoizedState=mm,n):(qn(t),rf(t,o))}if(l=e.memoizedState,l!==null&&(o=l.dehydrated,o!==null)){if(s)t.flags&256?(qn(t),t.flags&=-257,t=hm(e,t,a)):t.memoizedState!==null?(In(t),t.child=e.child,t.flags|=128,t=null):(In(t),r=n.fallback,o=t.mode,n=Uu({mode:"visible",children:n.children},o),r=_r(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,Is(t,e.child,null,a),n=t.child,n.memoizedState=fm(a),n.childLanes=pm(e,i,a),t.memoizedState=mm,t=r);else if(qn(t),yf(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(j(419)),n.stack="",n.digest=i,So({value:n,source:null,stack:null}),t=hm(e,t,a)}else if(ct||Bo(e,t,a,!1),i=(a&e.childLanes)!==0,ct||i){if(i=Ee,i!==null&&(n=a&-a,n=(n&42)!==0?1:Rf(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==l.retryLane))throw l.retryLane=n,Js(e,n),ta(i,e,n),Yb;o.data==="$?"||df(),t=hm(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=l.treeContext,Ie=Ca(o.nextSibling),At=t,ye=!0,kr=null,Ka=!1,e!==null&&(ha[va++]=hn,ha[va++]=vn,ha[va++]=Rr,hn=e.id,vn=e.overflow,Rr=t),t=rf(t,n.children),t.flags|=4096);return t}return r?(In(t),r=n.fallback,o=t.mode,l=e.child,c=l.sibling,n=bn(l,{mode:"hidden",children:n.children}),n.subtreeFlags=l.subtreeFlags&65011712,c!==null?r=bn(c,r):(r=_r(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=fm(a):(l=o.cachePool,l!==null?(c=nt._currentValue,l=l.parent!==c?{parent:c,pool:c}:l):l=rb(),o={baseLanes:o.baseLanes|a,cachePool:l}),r.memoizedState=o,r.childLanes=pm(e,i,a),t.memoizedState=mm,n):(qn(t),a=e.child,e=a.sibling,a=bn(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function rf(e,t){return t=Uu({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function Uu(e,t){return e=Xt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function hm(e,t,a){return Is(t,e.child,null,a),e=rf(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function qg(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),Km(e.return,t,a)}function vm(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function Zb(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(gt(e,t,n.children,a),n=rt.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&qg(e,a,t);else if(e.tag===19)qg(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Fe(rt,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&Ou(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),vm(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&Ou(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}vm(t,!0,a,null,s);break;case"together":vm(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function Sn(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),sr|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(Bo(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(j(153));if(t.child!==null){for(e=t.child,a=bn(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=bn(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function ap(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&Eu(e)))}function RE(e,t,a){switch(t.tag){case 3:$u(t,t.stateNode.containerInfo),zn(t,nt,e.memoizedState.cache),Fo();break;case 27:case 5:Mm(t);break;case 4:$u(t,t.stateNode.containerInfo);break;case 10:zn(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(qn(t),t.flags|=128,null):(a&t.child.childLanes)!==0?Wb(e,t,a):(qn(t),e=Sn(e,t,a),e!==null?e.sibling:null);qn(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(Bo(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return Zb(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Fe(rt,rt.current),n)break;return null;case 22:case 23:return t.lanes=0,Xb(e,t,a);case 24:zn(t,nt,e.memoizedState.cache)}return Sn(e,t,a)}function ex(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)ct=!0;else{if(!ap(e,a)&&(t.flags&128)===0)return ct=!1,RE(e,t,a);ct=(e.flags&131072)!==0}else ct=!1,ye&&(t.flags&1048576)!==0&&ab(t,Cu,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")Uf(n)?(e=Or(n,e),t.tag=1,t=Bg(null,t,n,e,a)):(t.tag=0,t=nf(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===Sf){t.tag=11,t=Pg(null,t,n,e,a);break e}else if(r===Nf){t.tag=14,t=Ug(null,t,n,e,a);break e}}throw t=Am(n)||n,Error(j(306,t,""))}}return t;case 0:return nf(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Or(n,t.pendingProps),Bg(e,t,n,r,a);case 3:e:{if($u(t,t.stateNode.containerInfo),e===null)throw Error(j(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,Ym(e,t),mo(t,n,null,a);var i=t.memoizedState;if(n=i.cache,zn(t,nt,n),n!==s.cache&&Qm(t,[nt],a,!0),co(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=zg(e,t,n,a);break e}else if(n!==r){r=ya(Error(j(424)),t),So(r),t=zg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(Ie=Ca(e.firstChild),At=t,ye=!0,kr=null,Ka=!0,a=qb(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if(Fo(),n===r){t=Sn(e,t,a);break e}gt(e,t,n,a)}t=t.child}return t;case 26:return hu(e,t),e===null?(a=oy(t.type,null,t.pendingProps,null))?t.memoizedState=a:ye||(a=t.type,e=t.pendingProps,n=Hu(Yn.current).createElement(a),n[St]=t,n[qt]=e,bt(n,a,e),ut(n),t.stateNode=n):t.memoizedState=oy(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return Mm(t),e===null&&ye&&(n=t.stateNode=Bx(t.type,t.pendingProps,Yn.current),At=t,Ka=!0,r=Ie,or(t.type)?(bf=r,Ie=Ca(n.firstChild)):Ie=r),gt(e,t,t.pendingProps.children,a),hu(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&ye&&((r=n=Ie)&&(n=WE(n,t.type,t.pendingProps,Ka),n!==null?(t.stateNode=n,At=t,Ie=Ca(n.firstChild),Ka=!1,r=!0):r=!1),r||Ar(t)),Mm(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,vf(r,s)?n=null:i!==null&&vf(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=Kf(e,t,yE,null,null,a),Eo._currentValue=r),hu(e,t),gt(e,t,n,a),t.child;case 6:return e===null&&ye&&((e=a=Ie)&&(a=ZE(a,t.pendingProps,Ka),a!==null?(t.stateNode=a,At=t,Ie=null,e=!0):e=!1),e||Ar(t)),null;case 13:return Wb(e,t,a);case 4:return $u(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=Is(t,null,n,a):gt(e,t,n,a),t.child;case 11:return Pg(e,t,t.type,t.pendingProps,a);case 7:return gt(e,t,t.pendingProps,a),t.child;case 8:return gt(e,t,t.pendingProps.children,a),t.child;case 12:return gt(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,zn(t,t.type,n.value),gt(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,Dr(t),r=Nt(r),n=n(r),t.flags|=1,gt(e,t,n,a),t.child;case 14:return Ug(e,t,t.type,t.pendingProps,a);case 15:return Jb(e,t,t.type,t.pendingProps,a);case 19:return Zb(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=Uu(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=bn(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return Xb(e,t,a);case 24:return Dr(t),n=Nt(nt),e===null?(r=zf(),r===null&&(r=Ee,s=Bf(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},qf(t),zn(t,nt,r)):((e.lanes&a)!==0&&(Ym(e,t),mo(t,null,null,a),co()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),zn(t,nt,n)):(n=s.cache,zn(t,nt,n),n!==r.cache&&Qm(t,[nt],a,!0))),gt(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(j(156,t.tag))}function dn(e){e.flags|=4}function Ig(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!Ix(t)){if(t=xa.current,t!==null&&((pe&4194048)===pe?Ya!==null:(pe&62914560)!==pe&&(pe&536870912)===0||t!==Ya))throw lo=Gm,sb;e.flags|=8192}}function tu(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?Ry():536870912,e.lanes|=t,Hs|=t)}function Ji(e,t){if(!ye)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function ze(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function kE(e,t,a){var n=t.pendingProps;switch(Ff(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return ze(t),null;case 1:return ze(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),xn(nt),Us(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Gi(t)?dn(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,bg())),ze(t),null;case 26:return a=t.memoizedState,e===null?(dn(t),a!==null?(ze(t),Ig(t,a)):(ze(t),t.flags&=-16777217)):a?a!==e.memoizedState?(dn(t),ze(t),Ig(t,a)):(ze(t),t.flags&=-16777217):(e.memoizedProps!==n&&dn(t),ze(t),t.flags&=-16777217),null;case 27:wu(t),a=Yn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&dn(t);else{if(!n){if(t.stateNode===null)throw Error(j(166));return ze(t),null}e=Va.current,Gi(t)?gg(t,e):(e=Bx(r,n,a),t.stateNode=e,dn(t))}return ze(t),null;case 5:if(wu(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&dn(t);else{if(!n){if(t.stateNode===null)throw Error(j(166));return ze(t),null}if(e=Va.current,Gi(t))gg(t,e);else{switch(r=Hu(Yn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[St]=t,e[qt]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(bt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&dn(t)}}return ze(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&dn(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(j(166));if(e=Yn.current,Gi(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=At,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[St]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||Ux(e.nodeValue,a)),e||Ar(t)}else e=Hu(e).createTextNode(n),e[St]=t,t.stateNode=e}return ze(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Gi(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(j(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(j(317));r[St]=t}else Fo(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;ze(t),r=!1}else r=bg(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(yn(t),t):(yn(t),null)}if(yn(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),tu(t,t.updateQueue),ze(t),null;case 4:return Us(),e===null&&cp(t.stateNode.containerInfo),ze(t),null;case 10:return xn(t.type),ze(t),null;case 19:if(dt(rt),r=t.memoizedState,r===null)return ze(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)Ji(r,!1);else{if(He!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=Ou(e),s!==null){for(t.flags|=128,Ji(r,!1),e=s.updateQueue,t.updateQueue=e,tu(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)tb(a,e),a=a.sibling;return Fe(rt,rt.current&1|2),t.child}e=e.sibling}r.tail!==null&&Ga()>Fu&&(t.flags|=128,n=!0,Ji(r,!1),t.lanes=4194304)}else{if(!n)if(e=Ou(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,tu(t,e),Ji(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!ye)return ze(t),null}else 2*Ga()-r.renderingStartTime>Fu&&a!==536870912&&(t.flags|=128,n=!0,Ji(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=Ga(),t.sibling=null,e=rt.current,Fe(rt,n?e&1|2:e&1),t):(ze(t),null);case 22:case 23:return yn(t),If(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(ze(t),t.subtreeFlags&6&&(t.flags|=8192)):ze(t),a=t.updateQueue,a!==null&&tu(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&dt(Cr),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),xn(nt),ze(t),null;case 25:return null;case 30:return null}throw Error(j(156,t.tag))}function CE(e,t){switch(Ff(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return xn(nt),Us(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return wu(t),null;case 13:if(yn(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(j(340));Fo()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return dt(rt),null;case 4:return Us(),null;case 10:return xn(t.type),null;case 22:case 23:return yn(t),If(),e!==null&&dt(Cr),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return xn(nt),null;case 25:return null;default:return null}}function tx(e,t){switch(Ff(t),t.tag){case 3:xn(nt),Us();break;case 26:case 27:case 5:wu(t);break;case 4:Us();break;case 13:yn(t);break;case 19:dt(rt);break;case 10:xn(t.type);break;case 22:case 23:yn(t),If(),e!==null&&dt(Cr);break;case 24:xn(nt)}}function Ko(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){Re(t,t.return,o)}}function rr(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var l=a,c=o;try{c()}catch(d){Re(r,l,d)}}}n=n.next}while(n!==s)}}catch(d){Re(t,t.return,d)}}function ax(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{lb(t,a)}catch(n){Re(e,e.return,n)}}}function nx(e,t,a){a.props=Or(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){Re(e,t,n)}}function po(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){Re(e,t,r)}}function Qa(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){Re(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){Re(e,t,r)}else a.current=null}function rx(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){Re(e,e.return,r)}}function gm(e,t,a){try{var n=e.stateNode;VE(n,e.type,a,t),n[qt]=t}catch(r){Re(e,e.return,r)}}function sx(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&or(e.type)||e.tag===4}function ym(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||sx(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&or(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function sf(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=uc));else if(n!==4&&(n===27&&or(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(sf(e,t,a),e=e.sibling;e!==null;)sf(e,t,a),e=e.sibling}function ju(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&or(e.type)&&(a=e.stateNode),e=e.child,e!==null))for(ju(e,t,a),e=e.sibling;e!==null;)ju(e,t,a),e=e.sibling}function ix(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);bt(t,n,a),t[St]=e,t[qt]=a}catch(s){Re(e,e.return,s)}}var fn=!1,Ge=!1,bm=!1,Hg=typeof WeakSet=="function"?WeakSet:Set,lt=null;function EE(e,t){if(e=e.containerInfo,pf=Gu,e=Vy(e),Of(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,l=-1,c=0,d=0,m=e,f=null;t:for(;;){for(var h;m!==a||r!==0&&m.nodeType!==3||(o=i+r),m!==s||n!==0&&m.nodeType!==3||(l=i+n),m.nodeType===3&&(i+=m.nodeValue.length),(h=m.firstChild)!==null;)f=m,m=h;for(;;){if(m===e)break t;if(f===a&&++c===r&&(o=i),f===s&&++d===n&&(l=i),(h=m.nextSibling)!==null)break;m=f,f=m.parentNode}m=h}a=o===-1||l===-1?null:{start:o,end:l}}else a=null}a=a||{start:0,end:0}}else a=null;for(hf={focusedElem:e,selectionRange:a},Gu=!1,lt=t;lt!==null;)if(t=lt,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,lt=e;else for(;lt!==null;){switch(t=lt,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var b=Or(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(b,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(y){Re(a,a.return,y)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)gf(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":gf(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(j(163))}if(e=t.sibling,e!==null){e.return=t.return,lt=e;break}lt=t.return}}function ox(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:Un(e,a),n&4&&Ko(5,a);break;case 1:if(Un(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){Re(a,a.return,i)}else{var r=Or(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){Re(a,a.return,i)}}n&64&&ax(a),n&512&&po(a,a.return);break;case 3:if(Un(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{lb(e,t)}catch(i){Re(a,a.return,i)}}break;case 27:t===null&&n&4&&ix(a);case 26:case 5:Un(e,a),t===null&&n&4&&rx(a),n&512&&po(a,a.return);break;case 12:Un(e,a);break;case 13:Un(e,a),n&4&&cx(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=jE.bind(null,a),e3(e,a))));break;case 22:if(n=a.memoizedState!==null||fn,!n){t=t!==null&&t.memoizedState!==null||Ge,r=fn;var s=Ge;fn=n,(Ge=t)&&!s?jn(e,a,(a.subtreeFlags&8772)!==0):Un(e,a),fn=r,Ge=s}break;case 30:break;default:Un(e,a)}}function lx(e){var t=e.alternate;t!==null&&(e.alternate=null,lx(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&Cf(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var je=null,Bt=!1;function mn(e,t,a){for(a=a.child;a!==null;)ux(e,t,a),a=a.sibling}function ux(e,t,a){if(Wt&&typeof Wt.onCommitFiberUnmount=="function")try{Wt.onCommitFiberUnmount(Oo,a)}catch{}switch(a.tag){case 26:Ge||Qa(a,t),mn(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:Ge||Qa(a,t);var n=je,r=Bt;or(a.type)&&(je=a.stateNode,Bt=!1),mn(e,t,a),yo(a.stateNode),je=n,Bt=r;break;case 5:Ge||Qa(a,t);case 6:if(n=je,r=Bt,je=null,mn(e,t,a),je=n,Bt=r,je!==null)if(Bt)try{(je.nodeType===9?je.body:je.nodeName==="HTML"?je.ownerDocument.body:je).removeChild(a.stateNode)}catch(s){Re(a,t,s)}else try{je.removeChild(a.stateNode)}catch(s){Re(a,t,s)}break;case 18:je!==null&&(Bt?(e=je,ry(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),Do(e)):ry(je,a.stateNode));break;case 4:n=je,r=Bt,je=a.stateNode.containerInfo,Bt=!0,mn(e,t,a),je=n,Bt=r;break;case 0:case 11:case 14:case 15:Ge||rr(2,a,t),Ge||rr(4,a,t),mn(e,t,a);break;case 1:Ge||(Qa(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&nx(a,t,n)),mn(e,t,a);break;case 21:mn(e,t,a);break;case 22:Ge=(n=Ge)||a.memoizedState!==null,mn(e,t,a),Ge=n;break;default:mn(e,t,a)}}function cx(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{Do(e)}catch(a){Re(t,t.return,a)}}function TE(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new Hg),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new Hg),t;default:throw Error(j(435,e.tag))}}function xm(e,t){var a=TE(e);t.forEach(function(n){var r=FE.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function Gt(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(or(o.type)){je=o.stateNode,Bt=!1;break e}break;case 5:je=o.stateNode,Bt=!1;break e;case 3:case 4:je=o.stateNode.containerInfo,Bt=!0;break e}o=o.return}if(je===null)throw Error(j(160));ux(s,i,r),je=null,Bt=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)dx(t,e),t=t.sibling}var ka=null;function dx(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:Gt(t,e),Yt(e),n&4&&(rr(3,e,e.return),Ko(3,e),rr(5,e,e.return));break;case 1:Gt(t,e),Yt(e),n&512&&(Ge||a===null||Qa(a,a.return)),n&64&&fn&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=ka;if(Gt(t,e),Yt(e),n&512&&(Ge||a===null||Qa(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[Uo]||s[St]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),bt(s,n,a),s[St]=e,ut(s),n=s;break e;case"link":var i=uy("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),bt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=uy("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),bt(s,n,a),r.head.appendChild(s);break;default:throw Error(j(468,n))}s[St]=e,ut(s),n=s}e.stateNode=n}else cy(r,e.type,e.stateNode);else e.stateNode=ly(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?cy(r,e.type,e.stateNode):ly(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&gm(e,e.memoizedProps,a.memoizedProps)}break;case 27:Gt(t,e),Yt(e),n&512&&(Ge||a===null||Qa(a,a.return)),a!==null&&n&4&&gm(e,e.memoizedProps,a.memoizedProps);break;case 5:if(Gt(t,e),Yt(e),n&512&&(Ge||a===null||Qa(a,a.return)),e.flags&32){r=e.stateNode;try{Fs(r,"")}catch(h){Re(e,e.return,h)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,gm(e,r,a!==null?a.memoizedProps:r)),n&1024&&(bm=!0);break;case 6:if(Gt(t,e),Yt(e),n&4){if(e.stateNode===null)throw Error(j(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(h){Re(e,e.return,h)}}break;case 3:if(yu=null,r=ka,ka=Ku(t.containerInfo),Gt(t,e),ka=r,Yt(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{Do(t.containerInfo)}catch(h){Re(e,e.return,h)}bm&&(bm=!1,mx(e));break;case 4:n=ka,ka=Ku(e.stateNode.containerInfo),Gt(t,e),Yt(e),ka=n;break;case 12:Gt(t,e),Yt(e);break;case 13:Gt(t,e),Yt(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(op=Ga()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,xm(e,n)));break;case 22:r=e.memoizedState!==null;var l=a!==null&&a.memoizedState!==null,c=fn,d=Ge;if(fn=c||r,Ge=d||l,Gt(t,e),Ge=d,fn=c,Yt(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||l||fn||Ge||Sr(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){l=a=t;try{if(s=l.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=l.stateNode;var m=l.memoizedProps.style,f=m!=null&&m.hasOwnProperty("display")?m.display:null;o.style.display=f==null||typeof f=="boolean"?"":(""+f).trim()}}catch(h){Re(l,l.return,h)}}}else if(t.tag===6){if(a===null){l=t;try{l.stateNode.nodeValue=r?"":l.memoizedProps}catch(h){Re(l,l.return,h)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,xm(e,a))));break;case 19:Gt(t,e),Yt(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,xm(e,n)));break;case 30:break;case 21:break;default:Gt(t,e),Yt(e)}}function Yt(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(sx(n)){a=n;break}n=n.return}if(a==null)throw Error(j(160));switch(a.tag){case 27:var r=a.stateNode,s=ym(e);ju(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(Fs(i,""),a.flags&=-33);var o=ym(e);ju(e,o,i);break;case 3:case 4:var l=a.stateNode.containerInfo,c=ym(e);sf(e,c,l);break;default:throw Error(j(161))}}catch(d){Re(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function mx(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;mx(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function Un(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)ox(e,t.alternate,t),t=t.sibling}function Sr(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:rr(4,t,t.return),Sr(t);break;case 1:Qa(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&nx(t,t.return,a),Sr(t);break;case 27:yo(t.stateNode);case 26:case 5:Qa(t,t.return),Sr(t);break;case 22:t.memoizedState===null&&Sr(t);break;case 30:Sr(t);break;default:Sr(t)}e=e.sibling}}function jn(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:jn(r,s,a),Ko(4,s);break;case 1:if(jn(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){Re(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var l=r.shared.hiddenCallbacks;if(l!==null)for(r.shared.hiddenCallbacks=null,r=0;r<l.length;r++)ob(l[r],o)}catch(c){Re(n,n.return,c)}}a&&i&64&&ax(s),po(s,s.return);break;case 27:ix(s);case 26:case 5:jn(r,s,a),a&&n===null&&i&4&&rx(s),po(s,s.return);break;case 12:jn(r,s,a);break;case 13:jn(r,s,a),a&&i&4&&cx(r,s);break;case 22:s.memoizedState===null&&jn(r,s,a),po(s,s.return);break;case 30:break;default:jn(r,s,a)}t=t.sibling}}function np(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&zo(a))}function rp(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&zo(e))}function Ha(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)fx(e,t,a,n),t=t.sibling}function fx(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:Ha(e,t,a,n),r&2048&&Ko(9,t);break;case 1:Ha(e,t,a,n);break;case 3:Ha(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&zo(e)));break;case 12:if(r&2048){Ha(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(l){Re(t,t.return,l)}}else Ha(e,t,a,n);break;case 13:Ha(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?Ha(e,t,a,n):ho(e,t):s._visibility&2?Ha(e,t,a,n):(s._visibility|=2,hs(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&np(i,t);break;case 24:Ha(e,t,a,n),r&2048&&rp(t.alternate,t);break;default:Ha(e,t,a,n)}}function hs(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,l=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:hs(s,i,o,l,r),Ko(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?hs(s,i,o,l,r):ho(s,i):(d._visibility|=2,hs(s,i,o,l,r)),r&&c&2048&&np(i.alternate,i);break;case 24:hs(s,i,o,l,r),r&&c&2048&&rp(i.alternate,i);break;default:hs(s,i,o,l,r)}t=t.sibling}}function ho(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:ho(a,n),r&2048&&np(n.alternate,n);break;case 24:ho(a,n),r&2048&&rp(n.alternate,n);break;default:ho(a,n)}t=t.sibling}}var no=8192;function ms(e){if(e.subtreeFlags&no)for(e=e.child;e!==null;)px(e),e=e.sibling}function px(e){switch(e.tag){case 26:ms(e),e.flags&no&&e.memoizedState!==null&&f3(ka,e.memoizedState,e.memoizedProps);break;case 5:ms(e);break;case 3:case 4:var t=ka;ka=Ku(e.stateNode.containerInfo),ms(e),ka=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=no,no=16777216,ms(e),no=t):ms(e));break;default:ms(e)}}function hx(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function Xi(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];lt=n,gx(n,e)}hx(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)vx(e),e=e.sibling}function vx(e){switch(e.tag){case 0:case 11:case 15:Xi(e),e.flags&2048&&rr(9,e,e.return);break;case 3:Xi(e);break;case 12:Xi(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,vu(e)):Xi(e);break;default:Xi(e)}}function vu(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];lt=n,gx(n,e)}hx(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:rr(8,t,t.return),vu(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,vu(t));break;default:vu(t)}e=e.sibling}}function gx(e,t){for(;lt!==null;){var a=lt;switch(a.tag){case 0:case 11:case 15:rr(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:zo(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,lt=n;else e:for(a=e;lt!==null;){n=lt;var r=n.sibling,s=n.return;if(lx(n),n===a){lt=null;break e}if(r!==null){r.return=s,lt=r;break e}lt=s}}}var AE={getCacheForType:function(e){var t=Nt(nt),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},DE=typeof WeakMap=="function"?WeakMap:Map,Se=0,Ee=null,de=null,pe=0,we=0,Jt=null,Vn=!1,Xs=!1,sp=!1,Nn=0,He=0,sr=0,Er=0,ip=0,ba=0,Hs=0,vo=null,zt=null,of=!1,op=0,Fu=1/0,Bu=null,Wn=null,yt=0,Zn=null,Ks=null,Ps=0,lf=0,uf=null,yx=null,go=0,cf=null;function ea(){if((Se&2)!==0&&pe!==0)return pe&-pe;if(se.T!==null){var e=Bs;return e!==0?e:up()}return Ey()}function bx(){ba===0&&(ba=(pe&536870912)===0||ye?_y():536870912);var e=xa.current;return e!==null&&(e.flags|=32),ba}function ta(e,t,a){(e===Ee&&(we===2||we===9)||e.cancelPendingCommit!==null)&&(Qs(e,0),Gn(e,pe,ba,!1)),Po(e,a),((Se&2)===0||e!==Ee)&&(e===Ee&&((Se&2)===0&&(Er|=a),He===4&&Gn(e,pe,ba,!1)),Xa(e))}function xx(e,t,a){if((Se&6)!==0)throw Error(j(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||Lo(e,t),r=n?LE(e,t):$m(e,t,!0),s=n;do{if(r===0){Xs&&!n&&Gn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!ME(a)){r=$m(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=vo;var l=o.current.memoizedState.isDehydrated;if(l&&(Qs(o,i).flags|=256),i=$m(o,i,!1),i!==2){if(sp&&!l){o.errorRecoveryDisabledLanes|=s,Er|=s,r=4;break e}s=zt,zt=r,s!==null&&(zt===null?zt=s:zt.push.apply(zt,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){Qs(e,0),Gn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(j(345));case 4:if((t&4194048)!==t)break;case 6:Gn(n,t,ba,!Vn);break e;case 2:zt=null;break;case 3:case 5:break;default:throw Error(j(329))}if((t&62914560)===t&&(r=op+300-Ga(),10<r)){if(Gn(n,t,ba,!Vn),Ju(n,0,!0)!==0)break e;n.timeoutHandle=Fx(Kg.bind(null,n,a,zt,Bu,of,t,ba,Er,Hs,Vn,s,2,-0,0),r);break e}Kg(n,a,zt,Bu,of,t,ba,Er,Hs,Vn,s,0,-0,0)}}break}while(!0);Xa(e)}function Kg(e,t,a,n,r,s,i,o,l,c,d,m,f,h){if(e.timeoutHandle=-1,m=t.subtreeFlags,(m&8192||(m&16785408)===16785408)&&(Co={stylesheets:null,count:0,unsuspend:m3},px(t),m=p3(),m!==null)){e.cancelPendingCommit=m(Vg.bind(null,e,t,s,a,n,r,i,o,l,d,1,f,h)),Gn(e,s,i,!c);return}Vg(e,t,s,a,n,r,i,o,l)}function ME(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!aa(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function Gn(e,t,a,n){t&=~ip,t&=~Er,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Zt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&ky(e,a,t)}function ic(){return(Se&6)===0?(Qo(0,!1),!1):!0}function lp(){if(de!==null){if(we===0)var e=de.return;else e=de,gn=jr=null,Gf(e),Ls=null,_o=0,e=de;for(;e!==null;)tx(e.alternate,e),e=e.return;de=null}}function Qs(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,YE(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),lp(),Ee=e,de=a=bn(e.current,null),pe=t,we=0,Jt=null,Vn=!1,Xs=Lo(e,t),sp=!1,Hs=ba=ip=Er=sr=He=0,zt=vo=null,of=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Zt(n),s=1<<r;t|=e[r],n&=~s}return Nn=t,ec(),a}function $x(e,t){le=null,se.H=Mu,t===qo||t===ac?(t=Sg(),we=3):t===sb?(t=Sg(),we=4):we=t===Yb?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Jt=t,de===null&&(He=1,Pu(e,ya(t,e.current)))}function wx(){var e=se.H;return se.H=Mu,e===null?Mu:e}function Sx(){var e=se.A;return se.A=AE,e}function df(){He=4,Vn||(pe&4194048)!==pe&&xa.current!==null||(Xs=!0),(sr&134217727)===0&&(Er&134217727)===0||Ee===null||Gn(Ee,pe,ba,!1)}function $m(e,t,a){var n=Se;Se|=2;var r=wx(),s=Sx();(Ee!==e||pe!==t)&&(Bu=null,Qs(e,t)),t=!1;var i=He;e:do try{if(we!==0&&de!==null){var o=de,l=Jt;switch(we){case 8:lp(),i=6;break e;case 3:case 2:case 9:case 6:xa.current===null&&(t=!0);var c=we;if(we=0,Jt=null,Cs(e,o,l,c),a&&Xs){i=0;break e}break;default:c=we,we=0,Jt=null,Cs(e,o,l,c)}}OE(),i=He;break}catch(d){$x(e,d)}while(!0);return t&&e.shellSuspendCounter++,gn=jr=null,Se=n,se.H=r,se.A=s,de===null&&(Ee=null,pe=0,ec()),i}function OE(){for(;de!==null;)Nx(de)}function LE(e,t){var a=Se;Se|=2;var n=wx(),r=Sx();Ee!==e||pe!==t?(Bu=null,Fu=Ga()+500,Qs(e,t)):Xs=Lo(e,t);e:do try{if(we!==0&&de!==null){t=de;var s=Jt;t:switch(we){case 1:we=0,Jt=null,Cs(e,t,s,1);break;case 2:case 9:if(wg(s)){we=0,Jt=null,Qg(t);break}t=function(){we!==2&&we!==9||Ee!==e||(we=7),Xa(e)},s.then(t,t);break e;case 3:we=7;break e;case 4:we=5;break e;case 7:wg(s)?(we=0,Jt=null,Qg(t)):(we=0,Jt=null,Cs(e,t,s,7));break;case 5:var i=null;switch(de.tag){case 26:i=de.memoizedState;case 5:case 27:var o=de;if(!i||Ix(i)){we=0,Jt=null;var l=o.sibling;if(l!==null)de=l;else{var c=o.return;c!==null?(de=c,oc(c)):de=null}break t}}we=0,Jt=null,Cs(e,t,s,5);break;case 6:we=0,Jt=null,Cs(e,t,s,6);break;case 8:lp(),He=6;break e;default:throw Error(j(462))}}PE();break}catch(d){$x(e,d)}while(!0);return gn=jr=null,se.H=n,se.A=r,Se=a,de!==null?0:(Ee=null,pe=0,ec(),He)}function PE(){for(;de!==null&&!rC();)Nx(de)}function Nx(e){var t=ex(e.alternate,e,Nn);e.memoizedProps=e.pendingProps,t===null?oc(e):de=t}function Qg(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=Fg(a,t,t.pendingProps,t.type,void 0,pe);break;case 11:t=Fg(a,t,t.pendingProps,t.type.render,t.ref,pe);break;case 5:Gf(t);default:tx(a,t),t=de=tb(t,Nn),t=ex(a,t,Nn)}e.memoizedProps=e.pendingProps,t===null?oc(e):de=t}function Cs(e,t,a,n){gn=jr=null,Gf(t),Ls=null,_o=0;var r=t.return;try{if(_E(e,r,t,a,pe)){He=1,Pu(e,ya(a,e.current)),de=null;return}}catch(s){if(r!==null)throw de=r,s;He=1,Pu(e,ya(a,e.current)),de=null;return}t.flags&32768?(ye||n===1?e=!0:Xs||(pe&536870912)!==0?e=!1:(Vn=e=!0,(n===2||n===9||n===3||n===6)&&(n=xa.current,n!==null&&n.tag===13&&(n.flags|=16384))),_x(t,e)):oc(t)}function oc(e){var t=e;do{if((t.flags&32768)!==0){_x(t,Vn);return}e=t.return;var a=kE(t.alternate,t,Nn);if(a!==null){de=a;return}if(t=t.sibling,t!==null){de=t;return}de=t=e}while(t!==null);He===0&&(He=5)}function _x(e,t){do{var a=CE(e.alternate,e);if(a!==null){a.flags&=32767,de=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){de=e;return}de=e=a}while(e!==null);He=6,de=null}function Vg(e,t,a,n,r,s,i,o,l){e.cancelPendingCommit=null;do lc();while(yt!==0);if((Se&6)!==0)throw Error(j(327));if(t!==null){if(t===e.current)throw Error(j(177));if(s=t.lanes|t.childLanes,s|=Lf,pC(e,a,s,i,o,l),e===Ee&&(de=Ee=null,pe=0),Ks=t,Zn=e,Ps=a,lf=s,uf=r,yx=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,BE(Su,function(){return Tx(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=se.T,se.T=null,r=be.p,be.p=2,i=Se,Se|=4;try{EE(e,t,a)}finally{Se=i,be.p=r,se.T=n}}yt=1,Rx(),kx(),Cx()}}function Rx(){if(yt===1){yt=0;var e=Zn,t=Ks,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=se.T,se.T=null;var n=be.p;be.p=2;var r=Se;Se|=4;try{dx(t,e);var s=hf,i=Vy(e.containerInfo),o=s.focusedElem,l=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&Qy(o.ownerDocument.documentElement,o)){if(l!==null&&Of(o)){var c=l.start,d=l.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var m=o.ownerDocument||document,f=m&&m.defaultView||window;if(f.getSelection){var h=f.getSelection(),b=o.textContent.length,y=Math.min(l.start,b),$=l.end===void 0?y:Math.min(l.end,b);!h.extend&&y>$&&(i=$,$=y,y=i);var g=pg(o,y),v=pg(o,$);if(g&&v&&(h.rangeCount!==1||h.anchorNode!==g.node||h.anchorOffset!==g.offset||h.focusNode!==v.node||h.focusOffset!==v.offset)){var x=m.createRange();x.setStart(g.node,g.offset),h.removeAllRanges(),y>$?(h.addRange(x),h.extend(v.node,v.offset)):(x.setEnd(v.node,v.offset),h.addRange(x))}}}}for(m=[],h=o;h=h.parentNode;)h.nodeType===1&&m.push({element:h,left:h.scrollLeft,top:h.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<m.length;o++){var w=m[o];w.element.scrollLeft=w.left,w.element.scrollTop=w.top}}Gu=!!pf,hf=pf=null}finally{Se=r,be.p=n,se.T=a}}e.current=t,yt=2}}function kx(){if(yt===2){yt=0;var e=Zn,t=Ks,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=se.T,se.T=null;var n=be.p;be.p=2;var r=Se;Se|=4;try{ox(e,t.alternate,t)}finally{Se=r,be.p=n,se.T=a}}yt=3}}function Cx(){if(yt===4||yt===3){yt=0,sC();var e=Zn,t=Ks,a=Ps,n=yx;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?yt=5:(yt=0,Ks=Zn=null,Ex(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(Wn=null),kf(a),t=t.stateNode,Wt&&typeof Wt.onCommitFiberRoot=="function")try{Wt.onCommitFiberRoot(Oo,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=se.T,r=be.p,be.p=2,se.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{se.T=t,be.p=r}}(Ps&3)!==0&&lc(),Xa(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===cf?go++:(go=0,cf=e):go=0,Qo(0,!1)}}function Ex(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,zo(t)))}function lc(e){return Rx(),kx(),Cx(),Tx(e)}function Tx(){if(yt!==5)return!1;var e=Zn,t=lf;lf=0;var a=kf(Ps),n=se.T,r=be.p;try{be.p=32>a?32:a,se.T=null,a=uf,uf=null;var s=Zn,i=Ps;if(yt=0,Ks=Zn=null,Ps=0,(Se&6)!==0)throw Error(j(331));var o=Se;if(Se|=4,vx(s.current),fx(s,s.current,i,a),Se=o,Qo(0,!1),Wt&&typeof Wt.onPostCommitFiberRoot=="function")try{Wt.onPostCommitFiberRoot(Oo,s)}catch{}return!0}finally{be.p=r,se.T=n,Ex(e,t)}}function Gg(e,t,a){t=ya(a,t),t=af(e.stateNode,t,2),e=Xn(e,t,2),e!==null&&(Po(e,2),Xa(e))}function Re(e,t,a){if(e.tag===3)Gg(e,e,a);else for(;t!==null;){if(t.tag===3){Gg(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(Wn===null||!Wn.has(n))){e=ya(a,e),a=Vb(2),n=Xn(t,a,2),n!==null&&(Gb(a,n,t,e),Po(n,2),Xa(n));break}}t=t.return}}function wm(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new DE;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(sp=!0,r.add(a),e=UE.bind(null,e,t,a),t.then(e,e))}function UE(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,Ee===e&&(pe&a)===a&&(He===4||He===3&&(pe&62914560)===pe&&300>Ga()-op?(Se&2)===0&&Qs(e,0):ip|=a,Hs===pe&&(Hs=0)),Xa(e)}function Ax(e,t){t===0&&(t=Ry()),e=Js(e,t),e!==null&&(Po(e,t),Xa(e))}function jE(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),Ax(e,a)}function FE(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(j(314))}n!==null&&n.delete(t),Ax(e,a)}function BE(e,t){return _f(e,t)}var zu=null,vs=null,mf=!1,qu=!1,Sm=!1,Tr=0;function Xa(e){e!==vs&&e.next===null&&(vs===null?zu=vs=e:vs=vs.next=e),qu=!0,mf||(mf=!0,qE())}function Qo(e,t){if(!Sm&&qu){Sm=!0;do for(var a=!1,n=zu;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Zt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,Yg(n,s))}else s=pe,s=Ju(n,n===Ee?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||Lo(n,s)||(a=!0,Yg(n,s));n=n.next}while(a);Sm=!1}}function zE(){Dx()}function Dx(){qu=mf=!1;var e=0;Tr!==0&&(GE()&&(e=Tr),Tr=0);for(var t=Ga(),a=null,n=zu;n!==null;){var r=n.next,s=Mx(n,t);s===0?(n.next=null,a===null?zu=r:a.next=r,r===null&&(vs=a)):(a=n,(e!==0||(s&3)!==0)&&(qu=!0)),n=r}Qo(e,!1)}function Mx(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Zt(s),o=1<<i,l=r[i];l===-1?((o&a)===0||(o&n)!==0)&&(r[i]=fC(o,t)):l<=t&&(e.expiredLanes|=o),s&=~o}if(t=Ee,a=pe,a=Ju(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&(we===2||we===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&Jd(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||Lo(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&Jd(n),kf(a)){case 2:case 8:a=Sy;break;case 32:a=Su;break;case 268435456:a=Ny;break;default:a=Su}return n=Ox.bind(null,e),a=_f(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&Jd(n),e.callbackPriority=2,e.callbackNode=null,2}function Ox(e,t){if(yt!==0&&yt!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(lc(!0)&&e.callbackNode!==a)return null;var n=pe;return n=Ju(e,e===Ee?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:(xx(e,n,t),Mx(e,Ga()),e.callbackNode!=null&&e.callbackNode===a?Ox.bind(null,e):null)}function Yg(e,t){if(lc())return null;xx(e,t,!0)}function qE(){JE(function(){(Se&6)!==0?_f(wy,zE):Dx()})}function up(){return Tr===0&&(Tr=_y()),Tr}function Jg(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:lu(""+e)}function Xg(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function IE(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=Jg((r[qt]||null).action),i=n.submitter;i&&(t=(t=i[qt]||null)?Jg(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Xu("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(Tr!==0){var l=i?Xg(r,i):new FormData(r);ef(a,{pending:!0,data:l,method:r.method,action:s},null,l)}}else typeof s=="function"&&(o.preventDefault(),l=i?Xg(r,i):new FormData(r),ef(a,{pending:!0,data:l,method:r.method,action:s},s,l))},currentTarget:r}]})}}for(au=0;au<qm.length;au++)nu=qm[au],Wg=nu.toLowerCase(),Zg=nu[0].toUpperCase()+nu.slice(1),Ea(Wg,"on"+Zg);var nu,Wg,Zg,au;Ea(Yy,"onAnimationEnd");Ea(Jy,"onAnimationIteration");Ea(Xy,"onAnimationStart");Ea("dblclick","onDoubleClick");Ea("focusin","onFocus");Ea("focusout","onBlur");Ea(lE,"onTransitionRun");Ea(uE,"onTransitionStart");Ea(cE,"onTransitionCancel");Ea(Wy,"onTransitionEnd");js("onMouseEnter",["mouseout","mouseover"]);js("onMouseLeave",["mouseout","mouseover"]);js("onPointerEnter",["pointerout","pointerover"]);js("onPointerLeave",["pointerout","pointerover"]);Lr("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Lr("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Lr("onBeforeInput",["compositionend","keypress","textInput","paste"]);Lr("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Lr("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Lr("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var Ro="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),HE=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(Ro));function Lx(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],l=o.instance,c=o.currentTarget;if(o=o.listener,l!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Lu(d)}r.currentTarget=null,s=l}else for(i=0;i<n.length;i++){if(o=n[i],l=o.instance,c=o.currentTarget,o=o.listener,l!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Lu(d)}r.currentTarget=null,s=l}}}}function ce(e,t){var a=t[Lm];a===void 0&&(a=t[Lm]=new Set);var n=e+"__bubble";a.has(n)||(Px(t,e,2,!1),a.add(n))}function Nm(e,t,a){var n=0;t&&(n|=4),Px(a,e,n,t)}var ru="_reactListening"+Math.random().toString(36).slice(2);function cp(e){if(!e[ru]){e[ru]=!0,Ty.forEach(function(a){a!=="selectionchange"&&(HE.has(a)||Nm(a,!1,e),Nm(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[ru]||(t[ru]=!0,Nm("selectionchange",!1,t))}}function Px(e,t,a,n){switch(Gx(t)){case 2:var r=g3;break;case 8:r=y3;break;default:r=pp}a=r.bind(null,t,a,e),r=void 0,!Fm||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function _m(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var l=i.tag;if((l===3||l===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=bs(o),i===null)return;if(l=i.tag,l===5||l===6||l===26||l===27){n=s=i;continue e}o=o.parentNode}}n=n.return}jy(function(){var c=s,d=Tf(a),m=[];e:{var f=Zy.get(e);if(f!==void 0){var h=Xu,b=e;switch(e){case"keypress":if(cu(a)===0)break e;case"keydown":case"keyup":h=BC;break;case"focusin":b="focus",h=rm;break;case"focusout":b="blur",h=rm;break;case"beforeblur":case"afterblur":h=rm;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":h=sg;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":h=CC;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":h=IC;break;case Yy:case Jy:case Xy:h=AC;break;case Wy:h=KC;break;case"scroll":case"scrollend":h=RC;break;case"wheel":h=VC;break;case"copy":case"cut":case"paste":h=MC;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":h=og;break;case"toggle":case"beforetoggle":h=YC}var y=(t&4)!==0,$=!y&&(e==="scroll"||e==="scrollend"),g=y?f!==null?f+"Capture":null:f;y=[];for(var v=c,x;v!==null;){var w=v;if(x=w.stateNode,w=w.tag,w!==5&&w!==26&&w!==27||x===null||g===null||(w=xo(v,g),w!=null&&y.push(ko(v,w,x))),$)break;v=v.return}0<y.length&&(f=new h(f,b,null,a,d),m.push({event:f,listeners:y}))}}if((t&7)===0){e:{if(f=e==="mouseover"||e==="pointerover",h=e==="mouseout"||e==="pointerout",f&&a!==jm&&(b=a.relatedTarget||a.fromElement)&&(bs(b)||b[Gs]))break e;if((h||f)&&(f=d.window===d?d:(f=d.ownerDocument)?f.defaultView||f.parentWindow:window,h?(b=a.relatedTarget||a.toElement,h=c,b=b?bs(b):null,b!==null&&($=Mo(b),y=b.tag,b!==$||y!==5&&y!==27&&y!==6)&&(b=null)):(h=null,b=c),h!==b)){if(y=sg,w="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(y=og,w="onPointerLeave",g="onPointerEnter",v="pointer"),$=h==null?f:ao(h),x=b==null?f:ao(b),f=new y(w,v+"leave",h,a,d),f.target=$,f.relatedTarget=x,w=null,bs(d)===c&&(y=new y(g,v+"enter",b,a,d),y.target=x,y.relatedTarget=$,w=y),$=w,h&&b)t:{for(y=h,g=b,v=0,x=y;x;x=fs(x))v++;for(x=0,w=g;w;w=fs(w))x++;for(;0<v-x;)y=fs(y),v--;for(;0<x-v;)g=fs(g),x--;for(;v--;){if(y===g||g!==null&&y===g.alternate)break t;y=fs(y),g=fs(g)}y=null}else y=null;h!==null&&ey(m,f,h,y,!1),b!==null&&$!==null&&ey(m,$,b,y,!0)}}e:{if(f=c?ao(c):window,h=f.nodeName&&f.nodeName.toLowerCase(),h==="select"||h==="input"&&f.type==="file")var S=dg;else if(cg(f))if(Hy)S=sE;else{S=nE;var k=aE}else h=f.nodeName,!h||h.toLowerCase()!=="input"||f.type!=="checkbox"&&f.type!=="radio"?c&&Ef(c.elementType)&&(S=dg):S=rE;if(S&&(S=S(e,c))){Iy(m,S,a,d);break e}k&&k(e,f,c),e==="focusout"&&c&&f.type==="number"&&c.memoizedProps.value!=null&&Um(f,"number",f.value)}switch(k=c?ao(c):window,e){case"focusin":(cg(k)||k.contentEditable==="true")&&(ws=k,Bm=c,io=null);break;case"focusout":io=Bm=ws=null;break;case"mousedown":zm=!0;break;case"contextmenu":case"mouseup":case"dragend":zm=!1,hg(m,a,d);break;case"selectionchange":if(oE)break;case"keydown":case"keyup":hg(m,a,d)}var N;if(Mf)e:{switch(e){case"compositionstart":var C="onCompositionStart";break e;case"compositionend":C="onCompositionEnd";break e;case"compositionupdate":C="onCompositionUpdate";break e}C=void 0}else $s?zy(e,a)&&(C="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(C="onCompositionStart");C&&(By&&a.locale!=="ko"&&($s||C!=="onCompositionStart"?C==="onCompositionEnd"&&$s&&(N=Fy()):(Qn=d,Af="value"in Qn?Qn.value:Qn.textContent,$s=!0)),k=Iu(c,C),0<k.length&&(C=new ig(C,e,null,a,d),m.push({event:C,listeners:k}),N?C.data=N:(N=qy(a),N!==null&&(C.data=N)))),(N=XC?WC(e,a):ZC(e,a))&&(C=Iu(c,"onBeforeInput"),0<C.length&&(k=new ig("onBeforeInput","beforeinput",null,a,d),m.push({event:k,listeners:C}),k.data=N)),IE(m,e,c,a,d)}Lx(m,t)})}function ko(e,t,a){return{instance:e,listener:t,currentTarget:a}}function Iu(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=xo(e,a),r!=null&&n.unshift(ko(e,r,s)),r=xo(e,t),r!=null&&n.push(ko(e,r,s))),e.tag===3)return n;e=e.return}return[]}function fs(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function ey(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,l=o.alternate,c=o.stateNode;if(o=o.tag,l!==null&&l===n)break;o!==5&&o!==26&&o!==27||c===null||(l=c,r?(c=xo(a,s),c!=null&&i.unshift(ko(a,c,l))):r||(c=xo(a,s),c!=null&&i.push(ko(a,c,l)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var KE=/\r\n?/g,QE=/\u0000|\uFFFD/g;function ty(e){return(typeof e=="string"?e:""+e).replace(KE,`
`).replace(QE,"")}function Ux(e,t){return t=ty(t),ty(e)===t}function uc(){}function Ne(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||Fs(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&Fs(e,""+n);break;case"className":Vl(e,"class",n);break;case"tabIndex":Vl(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":Vl(e,a,n);break;case"style":Uy(e,n,s);break;case"data":if(t!=="object"){Vl(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=lu(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&Ne(e,t,"name",r.name,r,null),Ne(e,t,"formEncType",r.formEncType,r,null),Ne(e,t,"formMethod",r.formMethod,r,null),Ne(e,t,"formTarget",r.formTarget,r,null)):(Ne(e,t,"encType",r.encType,r,null),Ne(e,t,"method",r.method,r,null),Ne(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=lu(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=uc);break;case"onScroll":n!=null&&ce("scroll",e);break;case"onScrollEnd":n!=null&&ce("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(j(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(j(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=lu(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":ce("beforetoggle",e),ce("toggle",e),ou(e,"popover",n);break;case"xlinkActuate":cn(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":cn(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":cn(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":cn(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":cn(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":cn(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":cn(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":cn(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":cn(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":ou(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=NC.get(a)||a,ou(e,a,n))}}function ff(e,t,a,n,r,s){switch(a){case"style":Uy(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(j(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(j(60));e.innerHTML=a}}break;case"children":typeof n=="string"?Fs(e,n):(typeof n=="number"||typeof n=="bigint")&&Fs(e,""+n);break;case"onScroll":n!=null&&ce("scroll",e);break;case"onScrollEnd":n!=null&&ce("scrollend",e);break;case"onClick":n!=null&&(e.onclick=uc);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!Ay.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[qt]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):ou(e,a,n)}}}function bt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":ce("error",e),ce("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(j(137,t));default:Ne(e,t,s,i,a,null)}}r&&Ne(e,t,"srcSet",a.srcSet,a,null),n&&Ne(e,t,"src",a.src,a,null);return;case"input":ce("invalid",e);var o=s=i=r=null,l=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":l=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(j(137,t));break;default:Ne(e,t,n,d,a,null)}}Oy(e,s,o,l,c,i,r,!1),Nu(e);return;case"select":ce("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:Ne(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?Ts(e,!!n,t,!1):a!=null&&Ts(e,!!n,a,!0);return;case"textarea":ce("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(j(91));break;default:Ne(e,t,i,o,a,null)}Py(e,n,r,s),Nu(e);return;case"option":for(l in a)if(a.hasOwnProperty(l)&&(n=a[l],n!=null))switch(l){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:Ne(e,t,l,n,a,null)}return;case"dialog":ce("beforetoggle",e),ce("toggle",e),ce("cancel",e),ce("close",e);break;case"iframe":case"object":ce("load",e);break;case"video":case"audio":for(n=0;n<Ro.length;n++)ce(Ro[n],e);break;case"image":ce("error",e),ce("load",e);break;case"details":ce("toggle",e);break;case"embed":case"source":case"link":ce("error",e),ce("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(j(137,t));default:Ne(e,t,c,n,a,null)}return;default:if(Ef(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&ff(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&Ne(e,t,o,n,a,null))}function VE(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,l=null,c=null,d=null;for(h in a){var m=a[h];if(a.hasOwnProperty(h)&&m!=null)switch(h){case"checked":break;case"value":break;case"defaultValue":l=m;default:n.hasOwnProperty(h)||Ne(e,t,h,null,n,m)}}for(var f in n){var h=n[f];if(m=a[f],n.hasOwnProperty(f)&&(h!=null||m!=null))switch(f){case"type":s=h;break;case"name":r=h;break;case"checked":c=h;break;case"defaultChecked":d=h;break;case"value":i=h;break;case"defaultValue":o=h;break;case"children":case"dangerouslySetInnerHTML":if(h!=null)throw Error(j(137,t));break;default:h!==m&&Ne(e,t,f,h,n,m)}}Pm(e,i,o,l,c,d,s,r);return;case"select":h=i=o=f=null;for(s in a)if(l=a[s],a.hasOwnProperty(s)&&l!=null)switch(s){case"value":break;case"multiple":h=l;default:n.hasOwnProperty(s)||Ne(e,t,s,null,n,l)}for(r in n)if(s=n[r],l=a[r],n.hasOwnProperty(r)&&(s!=null||l!=null))switch(r){case"value":f=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==l&&Ne(e,t,r,s,n,l)}t=o,a=i,n=h,f!=null?Ts(e,!!a,f,!1):!!n!=!!a&&(t!=null?Ts(e,!!a,t,!0):Ts(e,!!a,a?[]:"",!1));return;case"textarea":h=f=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:Ne(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":f=r;break;case"defaultValue":h=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(j(91));break;default:r!==s&&Ne(e,t,i,r,n,s)}Ly(e,f,h);return;case"option":for(var b in a)if(f=a[b],a.hasOwnProperty(b)&&f!=null&&!n.hasOwnProperty(b))switch(b){case"selected":e.selected=!1;break;default:Ne(e,t,b,null,n,f)}for(l in n)if(f=n[l],h=a[l],n.hasOwnProperty(l)&&f!==h&&(f!=null||h!=null))switch(l){case"selected":e.selected=f&&typeof f!="function"&&typeof f!="symbol";break;default:Ne(e,t,l,f,n,h)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var y in a)f=a[y],a.hasOwnProperty(y)&&f!=null&&!n.hasOwnProperty(y)&&Ne(e,t,y,null,n,f);for(c in n)if(f=n[c],h=a[c],n.hasOwnProperty(c)&&f!==h&&(f!=null||h!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(f!=null)throw Error(j(137,t));break;default:Ne(e,t,c,f,n,h)}return;default:if(Ef(t)){for(var $ in a)f=a[$],a.hasOwnProperty($)&&f!==void 0&&!n.hasOwnProperty($)&&ff(e,t,$,void 0,n,f);for(d in n)f=n[d],h=a[d],!n.hasOwnProperty(d)||f===h||f===void 0&&h===void 0||ff(e,t,d,f,n,h);return}}for(var g in a)f=a[g],a.hasOwnProperty(g)&&f!=null&&!n.hasOwnProperty(g)&&Ne(e,t,g,null,n,f);for(m in n)f=n[m],h=a[m],!n.hasOwnProperty(m)||f===h||f==null&&h==null||Ne(e,t,m,f,n,h)}var pf=null,hf=null;function Hu(e){return e.nodeType===9?e:e.ownerDocument}function ay(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function jx(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function vf(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var Rm=null;function GE(){var e=window.event;return e&&e.type==="popstate"?e===Rm?!1:(Rm=e,!0):(Rm=null,!1)}var Fx=typeof setTimeout=="function"?setTimeout:void 0,YE=typeof clearTimeout=="function"?clearTimeout:void 0,ny=typeof Promise=="function"?Promise:void 0,JE=typeof queueMicrotask=="function"?queueMicrotask:typeof ny<"u"?function(e){return ny.resolve(null).then(e).catch(XE)}:Fx;function XE(e){setTimeout(function(){throw e})}function or(e){return e==="head"}function ry(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&yo(i.documentElement),a&2&&yo(i.body),a&4)for(a=i.head,yo(a),i=a.firstChild;i;){var o=i.nextSibling,l=i.nodeName;i[Uo]||l==="SCRIPT"||l==="STYLE"||l==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),Do(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);Do(t)}function gf(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":gf(a),Cf(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function WE(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[Uo])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=Ca(e.nextSibling),e===null)break}return null}function ZE(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=Ca(e.nextSibling),e===null))return null;return e}function yf(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function e3(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function Ca(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var bf=null;function sy(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function Bx(e,t,a){switch(t=Hu(a),e){case"html":if(e=t.documentElement,!e)throw Error(j(452));return e;case"head":if(e=t.head,!e)throw Error(j(453));return e;case"body":if(e=t.body,!e)throw Error(j(454));return e;default:throw Error(j(451))}}function yo(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);Cf(e)}var $a=new Map,iy=new Set;function Ku(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var _n=be.d;be.d={f:t3,r:a3,D:n3,C:r3,L:s3,m:i3,X:l3,S:o3,M:u3};function t3(){var e=_n.f(),t=ic();return e||t}function a3(e){var t=Ys(e);t!==null&&t.tag===5&&t.type==="form"?Mb(t):_n.r(e)}var Ws=typeof document>"u"?null:document;function zx(e,t,a){var n=Ws;if(n&&typeof t=="string"&&t){var r=ga(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),iy.has(r)||(iy.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),bt(t,"link",e),ut(t),n.head.appendChild(t)))}}function n3(e){_n.D(e),zx("dns-prefetch",e,null)}function r3(e,t){_n.C(e,t),zx("preconnect",e,t)}function s3(e,t,a){_n.L(e,t,a);var n=Ws;if(n&&e&&t){var r='link[rel="preload"][as="'+ga(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+ga(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+ga(a.imageSizes)+'"]')):r+='[href="'+ga(e)+'"]';var s=r;switch(t){case"style":s=Vs(e);break;case"script":s=Zs(e)}$a.has(s)||(e=Me({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),$a.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(Vo(s))||t==="script"&&n.querySelector(Go(s))||(t=n.createElement("link"),bt(t,"link",e),ut(t),n.head.appendChild(t)))}}function i3(e,t){_n.m(e,t);var a=Ws;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+ga(n)+'"][href="'+ga(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=Zs(e)}if(!$a.has(s)&&(e=Me({rel:"modulepreload",href:e},t),$a.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(Go(s)))return}n=a.createElement("link"),bt(n,"link",e),ut(n),a.head.appendChild(n)}}}function o3(e,t,a){_n.S(e,t,a);var n=Ws;if(n&&e){var r=Es(n).hoistableStyles,s=Vs(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(Vo(s)))o.loading=5;else{e=Me({rel:"stylesheet",href:e,"data-precedence":t},a),(a=$a.get(s))&&dp(e,a);var l=i=n.createElement("link");ut(l),bt(l,"link",e),l._p=new Promise(function(c,d){l.onload=c,l.onerror=d}),l.addEventListener("load",function(){o.loading|=1}),l.addEventListener("error",function(){o.loading|=2}),o.loading|=4,gu(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function l3(e,t){_n.X(e,t);var a=Ws;if(a&&e){var n=Es(a).hoistableScripts,r=Zs(e),s=n.get(r);s||(s=a.querySelector(Go(r)),s||(e=Me({src:e,async:!0},t),(t=$a.get(r))&&mp(e,t),s=a.createElement("script"),ut(s),bt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function u3(e,t){_n.M(e,t);var a=Ws;if(a&&e){var n=Es(a).hoistableScripts,r=Zs(e),s=n.get(r);s||(s=a.querySelector(Go(r)),s||(e=Me({src:e,async:!0,type:"module"},t),(t=$a.get(r))&&mp(e,t),s=a.createElement("script"),ut(s),bt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function oy(e,t,a,n){var r=(r=Yn.current)?Ku(r):null;if(!r)throw Error(j(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=Vs(a.href),a=Es(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=Vs(a.href);var s=Es(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(Vo(e)))&&!s._p&&(i.instance=s,i.state.loading=5),$a.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},$a.set(e,a),s||c3(r,e,a,i.state))),t&&n===null)throw Error(j(528,""));return i}if(t&&n!==null)throw Error(j(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=Zs(a),a=Es(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(j(444,e))}}function Vs(e){return'href="'+ga(e)+'"'}function Vo(e){return'link[rel="stylesheet"]['+e+"]"}function qx(e){return Me({},e,{"data-precedence":e.precedence,precedence:null})}function c3(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),bt(t,"link",a),ut(t),e.head.appendChild(t))}function Zs(e){return'[src="'+ga(e)+'"]'}function Go(e){return"script[async]"+e}function ly(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+ga(a.href)+'"]');if(n)return t.instance=n,ut(n),n;var r=Me({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),ut(n),bt(n,"style",r),gu(n,a.precedence,e),t.instance=n;case"stylesheet":r=Vs(a.href);var s=e.querySelector(Vo(r));if(s)return t.state.loading|=4,t.instance=s,ut(s),s;n=qx(a),(r=$a.get(r))&&dp(n,r),s=(e.ownerDocument||e).createElement("link"),ut(s);var i=s;return i._p=new Promise(function(o,l){i.onload=o,i.onerror=l}),bt(s,"link",n),t.state.loading|=4,gu(s,a.precedence,e),t.instance=s;case"script":return s=Zs(a.src),(r=e.querySelector(Go(s)))?(t.instance=r,ut(r),r):(n=a,(r=$a.get(s))&&(n=Me({},a),mp(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),ut(r),bt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(j(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,gu(n,a.precedence,e));return t.instance}function gu(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function dp(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function mp(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var yu=null;function uy(e,t,a){if(yu===null){var n=new Map,r=yu=new Map;r.set(a,n)}else r=yu,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[Uo]||s[St]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function cy(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function d3(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function Ix(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var Co=null;function m3(){}function f3(e,t,a){if(Co===null)throw Error(j(475));var n=Co;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=Vs(a.href),s=e.querySelector(Vo(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=Qu.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,ut(s);return}s=e.ownerDocument||e,a=qx(a),(r=$a.get(r))&&dp(a,r),s=s.createElement("link"),ut(s);var i=s;i._p=new Promise(function(o,l){i.onload=o,i.onerror=l}),bt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=Qu.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function p3(){if(Co===null)throw Error(j(475));var e=Co;return e.stylesheets&&e.count===0&&xf(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&xf(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function Qu(){if(this.count--,this.count===0){if(this.stylesheets)xf(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var Vu=null;function xf(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,Vu=new Map,t.forEach(h3,e),Vu=null,Qu.call(e))}function h3(e,t){if(!(t.state.loading&4)){var a=Vu.get(e);if(a)var n=a.get(null);else{a=new Map,Vu.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=Qu.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var Eo={$$typeof:pn,Provider:null,Consumer:null,_currentValue:Nr,_currentValue2:Nr,_threadCount:0};function v3(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=Xd(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=Xd(0),this.hiddenUpdates=Xd(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function Hx(e,t,a,n,r,s,i,o,l,c,d,m){return e=new v3(e,t,a,i,o,l,c,m),t=1,s===!0&&(t|=24),s=Xt(3,null,null,t),e.current=s,s.stateNode=e,t=Bf(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},qf(s),e}function Kx(e){return e?(e=_s,e):_s}function Qx(e,t,a,n,r,s){r=Kx(r),n.context===null?n.context=r:n.pendingContext=r,n=Jn(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=Xn(e,n,t),a!==null&&(ta(a,e,t),uo(a,e,t))}function dy(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function fp(e,t){dy(e,t),(e=e.alternate)&&dy(e,t)}function Vx(e){if(e.tag===13){var t=Js(e,67108864);t!==null&&ta(t,e,67108864),fp(e,67108864)}}var Gu=!0;function g3(e,t,a,n){var r=se.T;se.T=null;var s=be.p;try{be.p=2,pp(e,t,a,n)}finally{be.p=s,se.T=r}}function y3(e,t,a,n){var r=se.T;se.T=null;var s=be.p;try{be.p=8,pp(e,t,a,n)}finally{be.p=s,se.T=r}}function pp(e,t,a,n){if(Gu){var r=$f(n);if(r===null)_m(e,t,n,Yu,a),my(e,n);else if(x3(r,e,t,a,n))n.stopPropagation();else if(my(e,n),t&4&&-1<b3.indexOf(e)){for(;r!==null;){var s=Ys(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=$r(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var l=1<<31-Zt(i);o.entanglements[1]|=l,i&=~l}Xa(s),(Se&6)===0&&(Fu=Ga()+500,Qo(0,!1))}}break;case 13:o=Js(s,2),o!==null&&ta(o,s,2),ic(),fp(s,2)}if(s=$f(n),s===null&&_m(e,t,n,Yu,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else _m(e,t,n,null,a)}}function $f(e){return e=Tf(e),hp(e)}var Yu=null;function hp(e){if(Yu=null,e=bs(e),e!==null){var t=Mo(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=yy(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return Yu=e,null}function Gx(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(iC()){case wy:return 2;case Sy:return 8;case Su:case oC:return 32;case Ny:return 268435456;default:return 32}default:return 32}}var wf=!1,er=null,tr=null,ar=null,To=new Map,Ao=new Map,Hn=[],b3="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function my(e,t){switch(e){case"focusin":case"focusout":er=null;break;case"dragenter":case"dragleave":tr=null;break;case"mouseover":case"mouseout":ar=null;break;case"pointerover":case"pointerout":To.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":Ao.delete(t.pointerId)}}function Wi(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Ys(t),t!==null&&Vx(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function x3(e,t,a,n,r){switch(t){case"focusin":return er=Wi(er,e,t,a,n,r),!0;case"dragenter":return tr=Wi(tr,e,t,a,n,r),!0;case"mouseover":return ar=Wi(ar,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return To.set(s,Wi(To.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,Ao.set(s,Wi(Ao.get(s)||null,e,t,a,n,r)),!0}return!1}function Yx(e){var t=bs(e.target);if(t!==null){var a=Mo(t);if(a!==null){if(t=a.tag,t===13){if(t=yy(a),t!==null){e.blockedOn=t,hC(e.priority,function(){if(a.tag===13){var n=ea();n=Rf(n);var r=Js(a,n);r!==null&&ta(r,a,n),fp(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function bu(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=$f(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);jm=n,a.target.dispatchEvent(n),jm=null}else return t=Ys(a),t!==null&&Vx(t),e.blockedOn=a,!1;t.shift()}return!0}function fy(e,t,a){bu(e)&&a.delete(t)}function $3(){wf=!1,er!==null&&bu(er)&&(er=null),tr!==null&&bu(tr)&&(tr=null),ar!==null&&bu(ar)&&(ar=null),To.forEach(fy),Ao.forEach(fy)}function su(e,t){e.blockedOn===t&&(e.blockedOn=null,wf||(wf=!0,st.unstable_scheduleCallback(st.unstable_NormalPriority,$3)))}var iu=null;function py(e){iu!==e&&(iu=e,st.unstable_scheduleCallback(st.unstable_NormalPriority,function(){iu===e&&(iu=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(hp(n||a)===null)continue;break}var s=Ys(a);s!==null&&(e.splice(t,3),t-=3,ef(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function Do(e){function t(l){return su(l,e)}er!==null&&su(er,e),tr!==null&&su(tr,e),ar!==null&&su(ar,e),To.forEach(t),Ao.forEach(t);for(var a=0;a<Hn.length;a++){var n=Hn[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<Hn.length&&(a=Hn[0],a.blockedOn===null);)Yx(a),a.blockedOn===null&&Hn.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[qt]||null;if(typeof s=="function")i||py(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[qt]||null)o=i.formAction;else if(hp(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),py(a)}}}function vp(e){this._internalRoot=e}cc.prototype.render=vp.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(j(409));var a=t.current,n=ea();Qx(a,n,e,t,null,null)};cc.prototype.unmount=vp.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;Qx(e.current,2,null,e,null,null),ic(),t[Gs]=null}};function cc(e){this._internalRoot=e}cc.prototype.unstable_scheduleHydration=function(e){if(e){var t=Ey();e={blockedOn:null,target:e,priority:t};for(var a=0;a<Hn.length&&t!==0&&t<Hn[a].priority;a++);Hn.splice(a,0,e),a===0&&Yx(e)}};var hy=vy.version;if(hy!=="19.1.0")throw Error(j(527,hy,"19.1.0"));be.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(j(188)):(e=Object.keys(e).join(","),Error(j(268,e)));return e=Zk(t),e=e!==null?by(e):null,e=e===null?null:e.stateNode,e};var w3={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:se,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(Zi=__REACT_DEVTOOLS_GLOBAL_HOOK__,!Zi.isDisabled&&Zi.supportsFiber))try{Oo=Zi.inject(w3),Wt=Zi}catch{}var Zi;dc.createRoot=function(e,t){if(!gy(e))throw Error(j(299));var a=!1,n="",r=Hb,s=Kb,i=Qb,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=Hx(e,1,!1,null,null,a,n,r,s,i,o,null),e[Gs]=t.current,cp(e),new vp(t)};dc.hydrateRoot=function(e,t,a){if(!gy(e))throw Error(j(299));var n=!1,r="",s=Hb,i=Kb,o=Qb,l=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(l=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=Hx(e,1,!0,t,a??null,n,r,s,i,o,l,c),t.context=Kx(null),a=t.current,n=ea(),n=Rf(n),r=Jn(n),r.callback=null,Xn(a,r,n),a=n,t.current.lanes=a,Po(t,a),Xa(t),e[Gs]=t.current,cp(e),new cc(t)};dc.version="19.1.0"});var Zx=Dn((gP,Wx)=>{"use strict";function Xx(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(Xx)}catch(e){console.error(e)}}Xx(),Wx.exports=Jx()});var Pt=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var Dk={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},Mk=class{#t=Dk;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},Ba=new Mk;function lv(e){setTimeout(e,0)}var Ut=typeof window>"u"||"Deno"in globalThis;function Pe(){}function dv(e,t){return typeof e=="function"?e(t):e}function Oi(e){return typeof e=="number"&&e>=0&&e!==1/0}function Sl(e,t){return Math.max(e+(t||0)-Date.now(),0)}function Ra(e,t){return typeof e=="function"?e(t):e}function jt(e,t){return typeof e=="function"?e(t):e}function Nl(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==Li(i,t.options))return!1}else if(!yr(t.queryKey,i))return!1}if(a!=="all"){let l=t.isActive();if(a==="active"&&!l||a==="inactive"&&l)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function _l(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(za(t.options.mutationKey)!==za(s))return!1}else if(!yr(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function Li(e,t){return(t?.queryKeyHashFn||za)(e)}function za(e){return JSON.stringify(e,(t,a)=>Cd(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function yr(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>yr(e[a],t[a])):!1}var Ok=Object.prototype.hasOwnProperty;function Pi(e,t){if(e===t)return e;let a=uv(e)&&uv(t);if(!a&&!(Cd(e)&&Cd(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},l=0;for(let c=0;c<i;c++){let d=a?c:s[c],m=e[d],f=t[d];if(m===f){o[d]=m,(a?c<r:Ok.call(e,d))&&l++;continue}if(m===null||f===null||typeof m!="object"||typeof f!="object"){o[d]=f;continue}let h=Pi(m,f);o[d]=h,h===m&&l++}return r===i&&l===r?e:o}function Mn(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function uv(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function Cd(e){if(!cv(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!cv(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function cv(e){return Object.prototype.toString.call(e)==="[object Object]"}function mv(e){return new Promise(t=>{Ba.setTimeout(t,e)})}function Ui(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?Pi(e,t):t}function fv(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function pv(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var rs=Symbol();function Rl(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===rs?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function ji(e,t){return typeof e=="function"?e(...t):!!e}var Lk=class extends Pt{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!Ut&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},ss=new Lk;function Fi(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var hv=lv;function Pk(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=hv,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(l=>{a(l)})})})};return{batch:o=>{let l;t++;try{l=o()}finally{t--,t||i()}return l},batchCalls:o=>(...l)=>{s(()=>{o(...l)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var fe=Pk();var Uk=class extends Pt{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!Ut&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},is=new Uk;function jk(e){return Math.min(1e3*2**e,3e4)}function Ed(e){return(e??"online")==="online"?is.isOnline():!0}var kl=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function Cl(e){let t=!1,a=0,n,r=Fi(),s=()=>r.status!=="pending",i=y=>{if(!s()){let $=new kl(y);f($),e.onCancel?.($)}},o=()=>{t=!0},l=()=>{t=!1},c=()=>ss.isFocused()&&(e.networkMode==="always"||is.isOnline())&&e.canRun(),d=()=>Ed(e.networkMode)&&e.canRun(),m=y=>{s()||(n?.(),r.resolve(y))},f=y=>{s()||(n?.(),r.reject(y))},h=()=>new Promise(y=>{n=$=>{(s()||c())&&y($)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),b=()=>{if(s())return;let y,$=a===0?e.initialPromise:void 0;try{y=$??e.fn()}catch(g){y=Promise.reject(g)}Promise.resolve(y).then(m).catch(g=>{if(s())return;let v=e.retry??(Ut?0:3),x=e.retryDelay??jk,w=typeof x=="function"?x(a,g):x,S=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!S){f(g);return}a++,e.onFail?.(a,g),mv(w).then(()=>c()?void 0:h()).then(()=>{t?f(g):b()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:l,canStart:d,start:()=>(d()?b():h().then(b),r)}}var El=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),Oi(this.gcTime)&&(this.#t=Ba.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(Ut?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(Ba.clearTimeout(this.#t),this.#t=void 0)}};var gv=class extends El{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=vv(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=vv(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=Ui(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(Pe).catch(Pe):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>jt(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===rs||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>Ra(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!Sl(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(l=>l.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=Rl(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=Cl({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof kl&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,l)=>{this.#i({type:"failed",failureCount:o,error:l})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof kl){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,...Td(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),fe.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function Td(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:Ed(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function vv(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var br=class extends Pt{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=Fi(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),yv(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return Ad(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return Ad(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof jt(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!Mn(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&bv(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||jt(this.options.enabled,this.#e)!==jt(t.enabled,this.#e)||Ra(this.options.staleTime,this.#e)!==Ra(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||jt(this.options.enabled,this.#e)!==jt(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return Bk(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(Pe)),t}#v(){this.#x();let e=Ra(this.options.staleTime,this.#e);if(Ut||this.#n.isStale||!Oi(e))return;let a=Sl(this.#n.dataUpdatedAt,e)+1;this.#u=Ba.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(Ut||jt(this.options.enabled,this.#e)===!1||!Oi(this.#l)||this.#l===0)&&(this.#c=Ba.setInterval(()=>{(this.options.refetchIntervalInBackground||ss.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(Ba.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(Ba.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,l=e!==a?e.state:this.#a,{state:c}=e,d={...c},m=!1,f;if(t._optimisticResults){let C=this.hasListeners(),P=!C&&yv(e,t),L=C&&bv(e,a,t,n);(P||L)&&(d={...d,...Td(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:h,errorUpdatedAt:b,status:y}=d;f=d.data;let $=!1;if(t.placeholderData!==void 0&&f===void 0&&y==="pending"){let C;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(C=r.data,$=!0):C=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,C!==void 0&&(y="success",f=Ui(r?.data,C,t),m=!0)}if(t.select&&f!==void 0&&!$)if(r&&f===s?.data&&t.select===this.#f)f=this.#d;else try{this.#f=t.select,f=t.select(f),f=Ui(r?.data,f,t),this.#d=f,this.#i=null}catch(C){this.#i=C}this.#i&&(h=this.#i,f=this.#d,b=Date.now(),y="error");let g=d.fetchStatus==="fetching",v=y==="pending",x=y==="error",w=v&&g,S=f!==void 0,N={status:y,fetchStatus:d.fetchStatus,isPending:v,isSuccess:y==="success",isError:x,isInitialLoading:w,isLoading:w,data:f,dataUpdatedAt:d.dataUpdatedAt,error:h,errorUpdatedAt:b,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>l.dataUpdateCount||d.errorUpdateCount>l.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:x&&!S,isPaused:d.fetchStatus==="paused",isPlaceholderData:m,isRefetchError:x&&S,isStale:Dd(e,t),refetch:this.refetch,promise:this.#o,isEnabled:jt(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let C=U=>{N.status==="error"?U.reject(N.error):N.data!==void 0&&U.resolve(N.data)},P=()=>{let U=this.#o=N.promise=Fi();C(U)},L=this.#o;switch(L.status){case"pending":e.queryHash===a.queryHash&&C(L);break;case"fulfilled":(N.status==="error"||N.data!==L.value)&&P();break;case"rejected":(N.status!=="error"||N.error!==L.reason)&&P();break}}return N}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),Mn(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){fe.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function Fk(e,t){return jt(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function yv(e,t){return Fk(e,t)||e.state.data!==void 0&&Ad(e,t,t.refetchOnMount)}function Ad(e,t,a){if(jt(t.enabled,e)!==!1&&Ra(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&Dd(e,t)}return!1}function bv(e,t,a,n){return(e!==t||jt(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&Dd(e,a)}function Dd(e,t){return jt(t.enabled,e)!==!1&&e.isStaleByTime(Ra(t.staleTime,e))}function Bk(e,t){return!Mn(e.getCurrentResult(),t)}function Md(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},l=0,c=async()=>{let d=!1,m=b=>{Object.defineProperty(b,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},f=Rl(t.options,t.fetchOptions),h=async(b,y,$)=>{if(d)return Promise.reject();if(y==null&&b.pages.length)return Promise.resolve(b);let v=(()=>{let k={client:t.client,queryKey:t.queryKey,pageParam:y,direction:$?"backward":"forward",meta:t.options.meta};return m(k),k})(),x=await f(v),{maxPages:w}=t.options,S=$?pv:fv;return{pages:S(b.pages,x,w),pageParams:S(b.pageParams,y,w)}};if(r&&s.length){let b=r==="backward",y=b?zk:xv,$={pages:s,pageParams:i},g=y(n,$);o=await h($,g,b)}else{let b=e??s.length;do{let y=l===0?i[0]??n.initialPageParam:xv(n,o);if(l>0&&y==null)break;o=await h(o,y),l++}while(l<b)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function xv(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function zk(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var $v=class extends El{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||Od(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=Cl({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),fe.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function Od(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var wv=class extends Pt{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new $v({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=Tl(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=Tl(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=Tl(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=Tl(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){fe.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>_l(t,a))}findAll(e={}){return this.getAll().filter(t=>_l(e,t))}notify(e){fe.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return fe.batch(()=>Promise.all(e.map(t=>t.continue().catch(Pe))))}};function Tl(e){return e.options.scope?.id}var Ld=class extends Pt{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),Mn(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&za(t.mutationKey)!==za(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??Od();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){fe.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function Sv(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function qk(e,t,a){let n=e.slice(0);return n[t]=a,n}var Pd=class extends Pt{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,fe.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,m)=>d!==a[m]),l=i||o,c=l?!0:s.some((d,m)=>{let f=this.#e[m];return!f||!Mn(d,f)});!l&&!c||(l&&(this.#r=r),this.#e=s,this.hasListeners()&&(l&&(Sv(a,r).forEach(d=>{d.destroy()}),Sv(r,a).forEach(d=>{d.subscribe(m=>{this.#c(d,m)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=Pi(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new br(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=qk(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&fe.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var Nv=class extends Pt{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??Li(n,t),s=this.get(r);return s||(s=new gv({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){fe.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>Nl(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>Nl(e,a)):t}notify(e){fe.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){fe.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){fe.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var Ud=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new Nv,this.#e=e.mutationCache||new wv,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=ss.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=is.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime(Ra(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=dv(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return fe.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;fe.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return fe.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=fe.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(Pe).catch(Pe)}invalidateQueries(e,t={}){return fe.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=fe.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(Pe)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(Pe)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime(Ra(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(Pe).catch(Pe)}fetchInfiniteQuery(e){return e.behavior=Md(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(Pe).catch(Pe)}ensureInfiniteQueryData(e){return e.behavior=Md(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return is.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(za(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{yr(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(za(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{yr(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=Li(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===rs&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var qa=qe(Qe(),1);var os=qe(Qe(),1),Cv=qe(jd(),1),Fd=os.createContext(void 0),W=e=>{let t=os.useContext(Fd);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},Bd=({client:e,children:t})=>(os.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,Cv.jsx)(Fd.Provider,{value:e,children:t}));var Dl=qe(Qe(),1),Ev=Dl.createContext(!1),Ml=()=>Dl.useContext(Ev),M6=Ev.Provider;var Bi=qe(Qe(),1),Kk=qe(jd(),1);function Qk(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var Vk=Bi.createContext(Qk()),Ol=()=>Bi.useContext(Vk);var Tv=qe(Qe(),1);var Ll=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},Pl=e=>{Tv.useEffect(()=>{e.clearReset()},[e])},Ul=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||ji(a,[e.error,n]));var jl=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},Fl=(e,t)=>e.isLoading&&e.isFetching&&!t,zi=(e,t)=>e?.suspense&&t.isPending,ls=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function zd({queries:e,...t},a){let n=W(a),r=Ml(),s=Ol(),i=qa.useMemo(()=>e.map(y=>{let $=n.defaultQueryOptions(y);return $._optimisticResults=r?"isRestoring":"optimistic",$}),[e,n,r]);i.forEach(y=>{jl(y),Ll(y,s)}),Pl(s);let[o]=qa.useState(()=>new Pd(n,i,t)),[l,c,d]=o.getOptimisticResult(i,t.combine),m=!r&&t.subscribed!==!1;qa.useSyncExternalStore(qa.useCallback(y=>m?o.subscribe(fe.batchCalls(y)):Pe,[o,m]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),qa.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let h=l.some((y,$)=>zi(i[$],y))?l.flatMap((y,$)=>{let g=i[$];if(g){let v=new br(n,g);if(zi(g,y))return ls(g,v,s);Fl(y,r)&&ls(g,v,s)}return[]}):[];if(h.length>0)throw Promise.all(h);let b=l.find((y,$)=>{let g=i[$];return g&&Ul({result:y,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(b?.error)throw b.error;return c(d())}var On=qe(Qe(),1);function Av(e,t,a){let n=Ml(),r=Ol(),s=W(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",jl(i),Ll(i,r),Pl(r);let o=!s.getQueryCache().get(i.queryHash),[l]=On.useState(()=>new t(s,i)),c=l.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(On.useSyncExternalStore(On.useCallback(m=>{let f=d?l.subscribe(fe.batchCalls(m)):Pe;return l.updateResult(),f},[l,d]),()=>l.getCurrentResult(),()=>l.getCurrentResult()),On.useEffect(()=>{l.setOptions(i)},[i,l]),zi(i,c))throw ls(i,l,r);if(Ul({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!Ut&&Fl(c,n)&&(o?ls(i,l,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(Pe).finally(()=>{l.updateResult()}),i.notifyOnChangeProps?c:l.trackResult(c)}function H(e,t){return Av(e,br,t)}var ln=qe(Qe(),1);function G(e,t){let a=W(t),[n]=ln.useState(()=>new Ld(a,e));ln.useEffect(()=>{n.setOptions(e)},[n,e]);let r=ln.useSyncExternalStore(ln.useCallback(i=>n.subscribe(fe.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=ln.useCallback((i,o)=>{n.mutate(i,o).catch(Pe)},[n]);if(r.error&&ji(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var Ek=qe(Zx());var Ht=qe(Qe(),1),X=qe(Qe(),1),ke=qe(Qe(),1),Lp=qe(Qe(),1),M0=qe(Qe(),1),me=qe(Qe(),1),PT=qe(Qe(),1),UT=qe(Qe(),1),jT=qe(Qe(),1),te=qe(Qe(),1),K0=qe(Qe(),1);var e0="popstate";function t0(e){return typeof e=="object"&&e!=null&&"pathname"in e&&"search"in e&&"hash"in e&&"state"in e&&"key"in e}function l0(e={}){function t(n,r){let s=r.state?.masked,{pathname:i,search:o,hash:l}=s||n.location;return xp("",{pathname:i,search:o,hash:l},r.state&&r.state.usr||null,r.state&&r.state.key||"default",s?{pathname:n.location.pathname,search:n.location.search,hash:n.location.hash}:void 0)}function a(n,r){return typeof r=="string"?r:ei(r)}return N3(t,a,null,e)}function Te(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function na(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function S3(){return Math.random().toString(36).substring(2,10)}function a0(e,t){return{usr:e.state,key:e.key,idx:t,masked:e.mask?{pathname:e.pathname,search:e.search,hash:e.hash}:void 0}}function xp(e,t,a=null,n,r){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?Fr(t):t,state:a,key:t&&t.key||n||S3(),mask:r}}function ei({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function Fr(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function N3(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",l=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function m(){o="POP";let $=d(),g=$==null?null:$-c;c=$,l&&l({action:o,location:y.location,delta:g})}function f($,g){o="PUSH";let v=t0($)?$:xp(y.location,$,g);a&&a(v,$),c=d()+1;let x=a0(v,c),w=y.createHref(v.mask||v);try{i.pushState(x,"",w)}catch(S){if(S instanceof DOMException&&S.name==="DataCloneError")throw S;r.location.assign(w)}s&&l&&l({action:o,location:y.location,delta:1})}function h($,g){o="REPLACE";let v=t0($)?$:xp(y.location,$,g);a&&a(v,$),c=d();let x=a0(v,c),w=y.createHref(v.mask||v);i.replaceState(x,"",w),s&&l&&l({action:o,location:y.location,delta:0})}function b($){return _3($)}let y={get action(){return o},get location(){return e(r,i)},listen($){if(l)throw new Error("A history only accepts one active listener");return r.addEventListener(e0,m),l=$,()=>{r.removeEventListener(e0,m),l=null}},createHref($){return t(r,$)},createURL:b,encodeLocation($){let g=b($);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:f,replace:h,go($){return i.go($)}};return y}function _3(e,t=!1){let a="http://localhost";typeof window<"u"&&(a=window.location.origin!=="null"?window.location.origin:window.location.href),Te(a,"No window.location.(origin|href) available to create URL");let n=typeof e=="string"?e:ei(e);return n=n.replace(/ $/,"%20"),!t&&n.startsWith("//")&&(n=a+n),new URL(n,a)}var R3;R3=new WeakMap;function Np(e,t,a="/"){return k3(e,t,a,!1)}function k3(e,t,a,n,r){let s=typeof t=="string"?Fr(t):t,i=Wa(s.pathname||"/",a);if(i==null)return null;let o=r??E3(e),l=null,c=z3(i);for(let d=0;l==null&&d<o.length;++d)l=F3(o[d],c,n);return l}function C3(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function E3(e){let t=u0(e);return T3(t),t}function u0(e,t=[],a=[],n="",r=!1){let s=(i,o,l=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&l)return;Te(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let m=Ta([n,d.relativePath]),f=a.concat(d);i.children&&i.children.length>0&&(Te(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${m}".`),u0(i.children,t,f,m,l)),!(i.path==null&&!i.index)&&t.push({path:m,score:U3(m,i.index),routesMeta:f})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let l of c0(i.path))s(i,o,!0,l)}),t}function c0(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=c0(n.join("/")),o=[];return o.push(...i.map(l=>l===""?s:[s,l].join("/"))),r&&o.push(...i),o.map(l=>e.startsWith("/")&&l===""?"/":l)}function T3(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:j3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var A3=/^:[\w-]+$/,D3=3,M3=2,O3=1,L3=10,P3=-2,n0=e=>e==="*";function U3(e,t){let a=e.split("/"),n=a.length;return a.some(n0)&&(n+=P3),t&&(n+=M3),a.filter(r=>!n0(r)).reduce((r,s)=>r+(A3.test(s)?D3:s===""?O3:L3),n)}function j3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function F3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let l=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",m=Jo({path:l.relativePath,caseSensitive:l.caseSensitive,end:c},d),f=l.route;if(!m&&c&&a&&!n[n.length-1].route.index&&(m=Jo({path:l.relativePath,caseSensitive:l.caseSensitive,end:!1},d)),!m)return null;Object.assign(r,m.params),i.push({params:r,pathname:Ta([s,m.pathname]),pathnameBase:H3(Ta([s,m.pathnameBase])),route:f}),m.pathnameBase!=="/"&&(s=Ta([s,m.pathnameBase]))}return i}function Jo(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=B3(e.path,e.caseSensitive,e.end),r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:m},f)=>{if(d==="*"){let b=o[f]||"";i=s.slice(0,s.length-b.length).replace(/(.)\/+$/,"$1")}let h=o[f];return m&&!h?c[d]=void 0:c[d]=(h||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function B3(e,t=!1,a=!0){na(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,l,c,d)=>{if(n.push({paramName:o,isOptional:l!=null}),l){let m=d.charAt(c+i.length);return m&&m!=="/"?"/([^\\/]*)":"(?:/([^\\/]*))?"}return"/([^\\/]+)"}).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function z3(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return na(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function Wa(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}var q3=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i;function d0(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?Fr(e):e,s;return a?(a=m0(a),a.startsWith("/")?s=r0(a.substring(1),"/"):s=r0(a,t)):s=t,{pathname:s,search:K3(n),hash:Q3(r)}}function r0(e,t){let a=vc(t).split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function gp(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function I3(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function _p(e){let t=I3(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function yc(e,t,a,n=!1){let r;typeof e=="string"?r=Fr(e):(r={...e},Te(!r.pathname||!r.pathname.includes("?"),gp("?","pathname","search",r)),Te(!r.pathname||!r.pathname.includes("#"),gp("#","pathname","hash",r)),Te(!r.search||!r.search.includes("#"),gp("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let m=t.length-1;if(!n&&i.startsWith("..")){let f=i.split("/");for(;f[0]==="..";)f.shift(),m-=1;r.pathname=f.join("/")}o=m>=0?t[m]:"/"}let l=d0(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!l.pathname.endsWith("/")&&(c||d)&&(l.pathname+="/"),l}var m0=e=>e.replace(/\/\/+/g,"/"),Ta=e=>m0(e.join("/")),vc=e=>e.replace(/\/+$/,""),H3=e=>vc(e).replace(/^\/*/,"/"),K3=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,Q3=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;var f0=class{constructor(e,t,a,n=!1){this.status=e,this.statusText=t||"",this.internal=n,a instanceof Error?(this.data=a.toString(),this.error=a):this.data=a}};function p0(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}function V3(e){let t=e.map(a=>a.route.path).filter(Boolean);return Ta(t)||"/"}var h0=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";function v0(e,t){let a=e;if(typeof a!="string"||!q3.test(a))return{absoluteURL:void 0,isExternal:!1,to:a};let n=a,r=!1;if(h0)try{let s=new URL(window.location.href),i=a.startsWith("//")?new URL(s.protocol+a):new URL(a),o=Wa(i.pathname,t);i.origin===s.origin&&o!=null?a=o+i.search+i.hash:r=!0}catch{na(!1,`<Link to="${a}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}return{absoluteURL:n,isExternal:r,to:a}}var yP=Symbol("Uninstrumented");var bP=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");var g0=["POST","PUT","PATCH","DELETE"],xP=new Set(g0),G3=["GET",...g0],$P=new Set(G3);var wP=Symbol("ResetLoaderData"),Y3,J3,X3,W3;Y3=new WeakMap;J3=new WeakMap;X3=new WeakMap;W3=new WeakMap;var Br=Ht.createContext(null);Br.displayName="DataRouter";var ti=Ht.createContext(null);ti.displayName="DataRouterState";var y0=Ht.createContext(!1);function Z3(){return Ht.useContext(y0)}var Rp=Ht.createContext({isTransitioning:!1});Rp.displayName="ViewTransition";var b0=Ht.createContext(new Map);b0.displayName="Fetchers";var eT=Ht.createContext(null);eT.displayName="Await";var _t=Ht.createContext(null);_t.displayName="Navigation";var ai=Ht.createContext(null);ai.displayName="Location";var ra=Ht.createContext({outlet:null,matches:[],isDataRoute:!1});ra.displayName="Route";var kp=Ht.createContext(null);kp.displayName="RouteError";var $p=!0,x0="REACT_ROUTER_ERROR",tT="REDIRECT",aT="ROUTE_ERROR_RESPONSE";function nT(e){if(e.startsWith(`${x0}:${tT}:{`))try{let t=JSON.parse(e.slice(28));if(typeof t=="object"&&t&&typeof t.status=="number"&&typeof t.statusText=="string"&&typeof t.location=="string"&&typeof t.reloadDocument=="boolean"&&typeof t.replace=="boolean")return t}catch{}}function rT(e){if(e.startsWith(`${x0}:${aT}:{`))try{let t=JSON.parse(e.slice(40));if(typeof t=="object"&&t&&typeof t.status=="number"&&typeof t.statusText=="string")return new f0(t.status,t.statusText,t.data)}catch{}}function $0(e,{relative:t}={}){Te(zr(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=X.useContext(_t),{hash:r,pathname:s,search:i}=ni(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:Ta([a,s])),n.createHref({pathname:o,search:i,hash:r})}function zr(){return X.useContext(ai)!=null}function Ae(){return Te(zr(),"useLocation() may be used only in the context of a <Router> component."),X.useContext(ai).location}var w0="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function S0(e){X.useContext(_t).static||X.useLayoutEffect(e)}function ve(){let{isDataRoute:e}=X.useContext(ra);return e?pT():sT()}function sT(){Te(zr(),"useNavigate() may be used only in the context of a <Router> component.");let e=X.useContext(Br),{basename:t,navigator:a}=X.useContext(_t),{matches:n}=X.useContext(ra),{pathname:r}=Ae(),s=JSON.stringify(_p(n)),i=X.useRef(!1);return S0(()=>{i.current=!0}),X.useCallback((l,c={})=>{if(na(i.current,w0),!i.current)return;if(typeof l=="number"){a.go(l);return}let d=yc(l,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:Ta([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var N0=X.createContext(null);function wa(){return X.useContext(N0)}function _0(e){let t=X.useContext(ra).outlet;return X.useMemo(()=>t&&X.createElement(N0.Provider,{value:e},t),[t,e])}function it(){let{matches:e}=X.useContext(ra);return e[e.length-1]?.params??{}}function ni(e,{relative:t}={}){let{matches:a}=X.useContext(ra),{pathname:n}=Ae(),r=JSON.stringify(_p(a));return X.useMemo(()=>yc(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function R0(e,t){return k0(e,t)}function k0(e,t,a){Te(zr(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:n}=X.useContext(_t),{matches:r}=X.useContext(ra),s=r[r.length-1],i=s?s.params:{},o=s?s.pathname:"/",l=s?s.pathnameBase:"/",c=s&&s.route;if($p){let $=c&&c.path||"";A0(o,!c||$.endsWith("*")||$.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${o}" (under <Route path="${$}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${$}"> to <Route path="${$==="/"?"*":`${$}/*`}">.`)}let d=Ae(),m;if(t){let $=typeof t=="string"?Fr(t):t;Te(l==="/"||$.pathname?.startsWith(l),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${l}" but pathname "${$.pathname}" was given in the \`location\` prop.`),m=$}else m=d;let f=m.pathname||"/",h=f;if(l!=="/"){let $=l.replace(/^\//,"").split("/");h="/"+f.replace(/^\//,"").split("/").slice($.length).join("/")}let b=a&&a.state.matches.length?a.state.matches.map($=>Object.assign($,{route:a.manifest[$.route.id]||$.route})):Np(e,{pathname:h});$p&&(na(c||b!=null,`No routes matched location "${m.pathname}${m.search}${m.hash}" `),na(b==null||b[b.length-1].route.element!==void 0||b[b.length-1].route.Component!==void 0||b[b.length-1].route.lazy!==void 0,`Matched leaf route at location "${m.pathname}${m.search}${m.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let y=cT(b&&b.map($=>Object.assign({},$,{params:Object.assign({},i,$.params),pathname:Ta([l,n.encodeLocation?n.encodeLocation($.pathname.replace(/%/g,"%25").replace(/\?/g,"%3F").replace(/#/g,"%23")).pathname:$.pathname]),pathnameBase:$.pathnameBase==="/"?l:Ta([l,n.encodeLocation?n.encodeLocation($.pathnameBase.replace(/%/g,"%25").replace(/\?/g,"%3F").replace(/#/g,"%23")).pathname:$.pathnameBase])})),r,a);return t&&y?X.createElement(ai.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",mask:void 0,...m},navigationType:"POP"}},y):y}function iT(){let e=T0(),t=p0(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return $p&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=X.createElement(X.Fragment,null,X.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),X.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",X.createElement("code",{style:s},"ErrorBoundary")," or"," ",X.createElement("code",{style:s},"errorElement")," prop on your route."))),X.createElement(X.Fragment,null,X.createElement("h2",null,"Unexpected Application Error!"),X.createElement("h3",{style:{fontStyle:"italic"}},t),a?X.createElement("pre",{style:r},a):null,i)}var oT=X.createElement(iT,null),C0=class extends X.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.onError?this.props.onError(e,t):console.error("React Router caught the following error during render",e)}render(){let e=this.state.error;if(this.context&&typeof e=="object"&&e&&"digest"in e&&typeof e.digest=="string"){let a=rT(e.digest);a&&(e=a)}let t=e!==void 0?X.createElement(ra.Provider,{value:this.props.routeContext},X.createElement(kp.Provider,{value:e,children:this.props.component})):this.props.children;return this.context?X.createElement(lT,{error:e},t):t}};C0.contextType=y0;var yp=new WeakMap;function lT({children:e,error:t}){let{basename:a}=X.useContext(_t);if(typeof t=="object"&&t&&"digest"in t&&typeof t.digest=="string"){let n=nT(t.digest);if(n){let r=yp.get(t);if(r)throw r;let s=v0(n.location,a);if(h0&&!yp.get(t))if(s.isExternal||n.reloadDocument)window.location.href=s.absoluteURL||s.to;else{let i=Promise.resolve().then(()=>window.__reactRouterDataRouter.navigate(s.to,{replace:n.replace}));throw yp.set(t,i),i}return X.createElement("meta",{httpEquiv:"refresh",content:`0;url=${s.absoluteURL||s.to}`})}}return e}function uT({routeContext:e,match:t,children:a}){let n=X.useContext(Br);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),X.createElement(ra.Provider,{value:e},a)}function cT(e,t=[],a){let n=a?.state;if(e==null){if(!n)return null;if(n.errors)e=n.matches;else if(t.length===0&&!n.initialized&&n.matches.length>0)e=n.matches;else return null}let r=e,s=n?.errors;if(s!=null){let d=r.findIndex(m=>m.route.id&&s?.[m.route.id]!==void 0);Te(d>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(s).join(",")}`),r=r.slice(0,Math.min(r.length,d+1))}let i=!1,o=-1;if(a&&n){i=n.renderFallback;for(let d=0;d<r.length;d++){let m=r[d];if((m.route.HydrateFallback||m.route.hydrateFallbackElement)&&(o=d),m.route.id){let{loaderData:f,errors:h}=n,b=m.route.loader&&!f.hasOwnProperty(m.route.id)&&(!h||h[m.route.id]===void 0);if(m.route.lazy||b){a.isStatic&&(i=!0),o>=0?r=r.slice(0,o+1):r=[r[0]];break}}}}let l=a?.onError,c=n&&l?(d,m)=>{l(d,{location:n.location,params:n.matches?.[0]?.params??{},pattern:V3(n.matches),errorInfo:m})}:void 0;return r.reduceRight((d,m,f)=>{let h,b=!1,y=null,$=null;n&&(h=s&&m.route.id?s[m.route.id]:void 0,y=m.route.errorElement||oT,i&&(o<0&&f===0?(A0("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),b=!0,$=null):o===f&&(b=!0,$=m.route.hydrateFallbackElement||null)));let g=t.concat(r.slice(0,f+1)),v=()=>{let x;return h?x=y:b?x=$:m.route.Component?x=X.createElement(m.route.Component,null):m.route.element?x=m.route.element:x=d,X.createElement(uT,{match:m,routeContext:{outlet:d,matches:g,isDataRoute:n!=null},children:x})};return n&&(m.route.ErrorBoundary||m.route.errorElement||f===0)?X.createElement(C0,{location:n.location,revalidation:n.revalidation,component:y,error:h,children:v(),routeContext:{outlet:null,matches:g,isDataRoute:!0},onError:c}):v()},null)}function Cp(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function dT(e){let t=X.useContext(Br);return Te(t,Cp(e)),t}function Ep(e){let t=X.useContext(ti);return Te(t,Cp(e)),t}function mT(e){let t=X.useContext(ra);return Te(t,Cp(e)),t}function Tp(e){let t=mT(e),a=t.matches[t.matches.length-1];return Te(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function fT(){return Tp("useRouteId")}function E0(){let e=Ep("useNavigation");return X.useMemo(()=>{let{matches:t,historyAction:a,...n}=e.navigation;return n},[e.navigation])}function Ap(){let{matches:e,loaderData:t}=Ep("useMatches");return X.useMemo(()=>e.map(a=>C3(a,t)),[e,t])}function T0(){let e=X.useContext(kp),t=Ep("useRouteError"),a=Tp("useRouteError");return e!==void 0?e:t.errors?.[a]}function pT(){let{router:e}=dT("useNavigate"),t=Tp("useNavigate"),a=X.useRef(!1);return S0(()=>{a.current=!0}),X.useCallback(async(r,s={})=>{na(a.current,w0),a.current&&(typeof r=="number"?await e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var s0={};function A0(e,t,a){!t&&!s0[e]&&(s0[e]=!0,na(!1,a))}var hT="useOptimistic",SP=ke[hT];var NP=ke.memo(vT);function vT({routes:e,manifest:t,future:a,state:n,isStatic:r,onError:s}){return k0(e,void 0,{manifest:t,state:n,isStatic:r,onError:s,future:a})}function ot({to:e,replace:t,state:a,relative:n}){Te(zr(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=ke.useContext(_t);na(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=ke.useContext(ra),{pathname:i}=Ae(),o=ve(),l=yc(e,_p(s),i,n==="path"),c=JSON.stringify(l);return ke.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function Dp(e){return _0(e.context)}function xe(e){Te(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function Mp({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1,useTransitions:i}){Te(!zr(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let o=e.replace(/^\/*/,"/"),l=ke.useMemo(()=>({basename:o,navigator:r,static:s,useTransitions:i,future:{}}),[o,r,s,i]);typeof a=="string"&&(a=Fr(a));let{pathname:c="/",search:d="",hash:m="",state:f=null,key:h="default",mask:b}=a,y=ke.useMemo(()=>{let $=Wa(c,o);return $==null?null:{location:{pathname:$,search:d,hash:m,state:f,key:h,mask:b},navigationType:n}},[o,c,d,m,f,h,n,b]);return na(y!=null,`<Router basename="${o}"> is not able to match the URL "${c}${d}${m}" because it does not start with the basename, so the <Router> won't render anything.`),y==null?null:ke.createElement(_t.Provider,{value:l},ke.createElement(ai.Provider,{children:t,value:y}))}function Op({children:e,location:t}){return R0(gc(e),t)}function gc(e,t=[]){let a=[];return ke.Children.forEach(e,(n,r)=>{if(!ke.isValidElement(n))return;let s=[...t,r];if(n.type===ke.Fragment){a.push.apply(a,gc(n.props.children,s));return}Te(n.type===xe,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),Te(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,middleware:n.props.middleware,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=gc(n.props.children,s)),a.push(i)}),a}var pc="get",hc="application/x-www-form-urlencoded";function bc(e){return typeof HTMLElement<"u"&&e instanceof HTMLElement}function gT(e){return bc(e)&&e.tagName.toLowerCase()==="button"}function yT(e){return bc(e)&&e.tagName.toLowerCase()==="form"}function bT(e){return bc(e)&&e.tagName.toLowerCase()==="input"}function xT(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function $T(e,t){return e.button===0&&(!t||t==="_self")&&!xT(e)}var mc=null;function wT(){if(mc===null)try{new FormData(document.createElement("form"),0),mc=!1}catch{mc=!0}return mc}var ST=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function bp(e){return e!=null&&!ST.has(e)?(na(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${hc}"`),null):e}function NT(e,t){let a,n,r,s,i;if(yT(e)){let o=e.getAttribute("action");n=o?Wa(o,t):null,a=e.getAttribute("method")||pc,r=bp(e.getAttribute("enctype"))||hc,s=new FormData(e)}else if(gT(e)||bT(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let l=e.getAttribute("formaction")||o.getAttribute("action");if(n=l?Wa(l,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||pc,r=bp(e.getAttribute("formenctype"))||bp(o.getAttribute("enctype"))||hc,s=new FormData(o,e),!wT()){let{name:c,type:d,value:m}=e;if(d==="image"){let f=c?`${c}.`:"";s.append(`${f}x`,"0"),s.append(`${f}y`,"0")}else c&&s.append(c,m)}}else{if(bc(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=pc,n=null,r=hc,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var _P=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");var _T={"&":"\\u0026",">":"\\u003e","<":"\\u003c","\u2028":"\\u2028","\u2029":"\\u2029"},RT=/[&><\u2028\u2029]/g;function i0(e){return e.replace(RT,t=>_T[t])}function Pp(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var kT=Symbol("SingleFetchRedirect");function D0(e,t,a,n){let r=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return a?r.pathname.endsWith("/")?r.pathname=`${r.pathname}_.${n}`:r.pathname=`${r.pathname}.${n}`:r.pathname==="/"?r.pathname=`_root.${n}`:t&&Wa(r.pathname,t)==="/"?r.pathname=`${vc(t)}/_root.${n}`:r.pathname=`${vc(r.pathname)}.${n}`,r}async function CT(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function ET(e){return e!=null&&typeof e.page=="string"}function TT(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function AT(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await CT(s,a);return i.links?i.links():[]}return[]}));return LT(n.flat(1).filter(TT).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function o0(e,t,a,n,r,s){let i=(l,c)=>a[c]?l.route.id!==a[c].route.id:!0,o=(l,c)=>a[c].pathname!==l.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==l.params["*"];return s==="assets"?t.filter((l,c)=>i(l,c)||o(l,c)):s==="data"?t.filter((l,c)=>{let d=n.routes[l.route.id];if(!d||!d.hasLoader)return!1;if(i(l,c)||o(l,c))return!0;if(l.route.shouldRevalidate){let m=l.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:l.params,defaultShouldRevalidate:!0});if(typeof m=="boolean")return m}return!0}):[]}function DT(e,t,{includeHydrateFallback:a}={}){return MT(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function MT(e){return[...new Set(e)]}function OT(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function LT(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!ET(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(OT(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function Up(){let e=me.useContext(Br);return Pp(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function FT(){let e=me.useContext(ti);return Pp(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var Xo=me.createContext(void 0);Xo.displayName="FrameworkContext";function jp(){let e=me.useContext(Xo);return Pp(e,"You must render this element inside a <HydratedRouter> element"),e}function BT(e,t){let a=me.useContext(Xo),[n,r]=me.useState(!1),[s,i]=me.useState(!1),{onFocus:o,onBlur:l,onMouseEnter:c,onMouseLeave:d,onTouchStart:m}=t,f=me.useRef(null);me.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let y=g=>{g.forEach(v=>{i(v.isIntersecting)})},$=new IntersectionObserver(y,{threshold:.5});return f.current&&$.observe(f.current),()=>{$.disconnect()}}},[e]),me.useEffect(()=>{if(n){let y=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(y)}}},[n]);let h=()=>{r(!0)},b=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,f,{}]:[s,f,{onFocus:Yo(o,h),onBlur:Yo(l,b),onMouseEnter:Yo(c,h),onMouseLeave:Yo(d,b),onTouchStart:Yo(m,h)}]:[!1,f,{}]}function Yo(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function O0({page:e,...t}){let a=Z3(),{router:n}=Up(),r=me.useMemo(()=>Np(n.routes,e,n.basename),[n.routes,e,n.basename]);return r?a?me.createElement(qT,{page:e,matches:r,...t}):me.createElement(IT,{page:e,matches:r,...t}):null}function zT(e){let{manifest:t,routeModules:a}=jp(),[n,r]=me.useState([]);return me.useEffect(()=>{let s=!1;return AT(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function qT({page:e,matches:t,...a}){let n=Ae(),{future:r}=jp(),{basename:s}=Up(),i=me.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let o=D0(e,s,r.unstable_trailingSlashAwareDataRequests,"rsc"),l=!1,c=[];for(let d of t)typeof d.route.shouldRevalidate=="function"?l=!0:c.push(d.route.id);return l&&c.length>0&&o.searchParams.set("_routes",c.join(",")),[o.pathname+o.search]},[s,r.unstable_trailingSlashAwareDataRequests,e,n,t]);return me.createElement(me.Fragment,null,i.map(o=>me.createElement("link",{key:o,rel:"prefetch",as:"fetch",href:o,...a})))}function IT({page:e,matches:t,...a}){let n=Ae(),{future:r,manifest:s,routeModules:i}=jp(),{basename:o}=Up(),{loaderData:l,matches:c}=FT(),d=me.useMemo(()=>o0(e,t,c,s,n,"data"),[e,t,c,s,n]),m=me.useMemo(()=>o0(e,t,c,s,n,"assets"),[e,t,c,s,n]),f=me.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let y=new Set,$=!1;if(t.forEach(v=>{let x=s.routes[v.route.id];!x||!x.hasLoader||(!d.some(w=>w.route.id===v.route.id)&&v.route.id in l&&i[v.route.id]?.shouldRevalidate||x.hasClientLoader?$=!0:y.add(v.route.id))}),y.size===0)return[];let g=D0(e,o,r.unstable_trailingSlashAwareDataRequests,"data");return $&&y.size>0&&g.searchParams.set("_routes",t.filter(v=>y.has(v.route.id)).map(v=>v.route.id).join(",")),[g.pathname+g.search]},[o,r.unstable_trailingSlashAwareDataRequests,l,n,s,d,t,e,i]),h=me.useMemo(()=>DT(m,s),[m,s]),b=zT(m);return me.createElement(me.Fragment,null,f.map(y=>me.createElement("link",{key:y,rel:"prefetch",as:"fetch",href:y,...a})),h.map(y=>me.createElement("link",{key:y,rel:"modulepreload",href:y,...a})),b.map(({key:y,link:$})=>me.createElement("link",{key:y,nonce:a.nonce,...$,crossOrigin:$.crossOrigin??a.crossOrigin})))}function HT(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var KT=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{KT&&(window.__reactRouterVersion="7.15.1")}catch{}function Fp({basename:e,children:t,useTransitions:a,window:n}){let r=te.useRef();r.current==null&&(r.current=l0({window:n,v5Compat:!0}));let s=r.current,[i,o]=te.useState({action:s.action,location:s.location}),l=te.useCallback(c=>{a===!1?o(c):te.startTransition(()=>o(c))},[a]);return te.useLayoutEffect(()=>s.listen(l),[s,l]),te.createElement(Mp,{basename:e,children:t,location:i.location,navigationType:i.action,navigator:s,useTransitions:a})}function L0({basename:e,children:t,history:a,useTransitions:n}){let[r,s]=te.useState({action:a.action,location:a.location}),i=te.useCallback(o=>{n===!1?s(o):te.startTransition(()=>s(o))},[n]);return te.useLayoutEffect(()=>a.listen(i),[a,i]),te.createElement(Mp,{basename:e,children:t,location:r.location,navigationType:r.action,navigator:a,useTransitions:n})}L0.displayName="unstable_HistoryRouter";var P0=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i,Rn=te.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,mask:o,state:l,target:c,to:d,preventScrollReset:m,viewTransition:f,defaultShouldRevalidate:h,...b},y){let{basename:$,navigator:g,useTransitions:v}=te.useContext(_t),x=typeof d=="string"&&P0.test(d),w=v0(d,$);d=w.to;let S=$0(d,{relative:r}),k=Ae(),N=null;if(o){let ee=yc(o,[],k.mask?k.mask.pathname:"/",!0);$!=="/"&&(ee.pathname=ee.pathname==="/"?$:Ta([$,ee.pathname])),N=g.createHref(ee)}let[C,P,L]=BT(n,b),U=B0(d,{replace:i,mask:o,state:l,target:c,preventScrollReset:m,relative:r,viewTransition:f,defaultShouldRevalidate:h,useTransitions:v});function F(ee){t&&t(ee),ee.defaultPrevented||U(ee)}let T=!(w.isExternal||s),K=te.createElement("a",{...b,...L,href:(T?N:void 0)||w.absoluteURL||S,onClick:T?F:t,ref:HT(y,P),target:c,"data-discover":!x&&a==="render"?"true":void 0});return C&&!x?te.createElement(te.Fragment,null,K,te.createElement(O0,{page:S})):K});Rn.displayName="Link";var Za=te.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:l,...c},d){let m=ni(i,{relative:c.relative}),f=Ae(),h=te.useContext(ti),{navigator:b,basename:y}=te.useContext(_t),$=h!=null&&H0(m)&&o===!0,g=b.encodeLocation?b.encodeLocation(m).pathname:m.pathname,v=f.pathname,x=h&&h.navigation&&h.navigation.location?h.navigation.location.pathname:null;a||(v=v.toLowerCase(),x=x?x.toLowerCase():null,g=g.toLowerCase()),x&&y&&(x=Wa(x,y)||x);let w=g!=="/"&&g.endsWith("/")?g.length-1:g.length,S=v===g||!r&&v.startsWith(g)&&v.charAt(w)==="/",k=x!=null&&(x===g||!r&&x.startsWith(g)&&x.charAt(g.length)==="/"),N={isActive:S,isPending:k,isTransitioning:$},C=S?t:void 0,P;typeof n=="function"?P=n(N):P=[n,S?"active":null,k?"pending":null,$?"transitioning":null].filter(Boolean).join(" ");let L=typeof s=="function"?s(N):s;return te.createElement(Rn,{...c,"aria-current":C,className:P,ref:d,style:L,to:i,viewTransition:o},typeof l=="function"?l(N):l)});Za.displayName="NavLink";var U0=te.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=pc,action:o,onSubmit:l,relative:c,preventScrollReset:d,viewTransition:m,defaultShouldRevalidate:f,...h},b)=>{let{useTransitions:y}=te.useContext(_t),$=z0(),g=q0(o,{relative:c}),v=i.toLowerCase()==="get"?"get":"post",x=typeof o=="string"&&P0.test(o);return te.createElement("form",{ref:b,method:v,action:g,onSubmit:n?l:S=>{if(l&&l(S),S.defaultPrevented)return;S.preventDefault();let k=S.nativeEvent.submitter,N=k?.getAttribute("formmethod")||i,C=()=>$(k||S.currentTarget,{fetcherKey:t,method:N,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:m,defaultShouldRevalidate:f});y&&a!==!1?te.startTransition(()=>C()):C()},...h,"data-discover":!x&&e==="render"?"true":void 0})});U0.displayName="Form";function j0({getKey:e,storageKey:t,...a}){let n=te.useContext(Xo),{basename:r}=te.useContext(_t),s=Ae(),i=Ap();I0({getKey:e,storageKey:t});let o=te.useMemo(()=>{if(!n||!e)return null;let c=Sp(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let l=((c,d)=>{if(!window.history.state||!window.history.state.key){let m=Math.random().toString(32).slice(2);window.history.replaceState({key:m},"")}try{let f=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof f=="number"&&window.scrollTo(0,f)}catch(m){console.error(m),sessionStorage.removeItem(c)}}).toString();return te.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${l})(${i0(JSON.stringify(t||wp))}, ${i0(JSON.stringify(o))})`}})}j0.displayName="ScrollRestoration";function F0(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function Bp(e){let t=te.useContext(Br);return Te(t,F0(e)),t}function QT(e){let t=te.useContext(ti);return Te(t,F0(e)),t}function B0(e,{target:t,replace:a,mask:n,state:r,preventScrollReset:s,relative:i,viewTransition:o,defaultShouldRevalidate:l,useTransitions:c}={}){let d=ve(),m=Ae(),f=ni(e,{relative:i});return te.useCallback(h=>{if($T(h,t)){h.preventDefault();let b=a!==void 0?a:ei(m)===ei(f),y=()=>d(e,{replace:b,mask:n,state:r,preventScrollReset:s,relative:i,viewTransition:o,defaultShouldRevalidate:l});c?te.startTransition(()=>y()):y()}},[m,d,f,a,n,r,t,e,s,i,o,l,c])}var VT=0,GT=()=>`__${String(++VT)}__`;function z0(){let{router:e}=Bp("useSubmit"),{basename:t}=te.useContext(_t),a=fT(),n=e.fetch,r=e.navigate;return te.useCallback(async(s,i={})=>{let{action:o,method:l,encType:c,formData:d,body:m}=NT(s,t);if(i.navigate===!1){let f=i.fetcherKey||GT();await n(f,a,i.action||o,{defaultShouldRevalidate:i.defaultShouldRevalidate,preventScrollReset:i.preventScrollReset,formData:d,body:m,formMethod:i.method||l,formEncType:i.encType||c,flushSync:i.flushSync})}else await r(i.action||o,{defaultShouldRevalidate:i.defaultShouldRevalidate,preventScrollReset:i.preventScrollReset,formData:d,body:m,formMethod:i.method||l,formEncType:i.encType||c,replace:i.replace,state:i.state,fromRouteId:a,flushSync:i.flushSync,viewTransition:i.viewTransition})},[n,r,t,a])}function q0(e,{relative:t}={}){let{basename:a}=te.useContext(_t),n=te.useContext(ra);Te(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...ni(e||".",{relative:t})},i=Ae();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),l=o.getAll("index");if(l.some(d=>d==="")){o.delete("index"),l.filter(m=>m).forEach(m=>o.append("index",m));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:Ta([a,s.pathname])),ei(s)}var wp="react-router-scroll-positions",fc={};function Sp(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:Wa(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function I0({getKey:e,storageKey:t}={}){let{router:a}=Bp("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=QT("useScrollRestoration"),{basename:s}=te.useContext(_t),i=Ae(),o=Ap(),l=E0();te.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),YT(te.useCallback(()=>{if(l.state==="idle"){let c=Sp(i,o,s,e);fc[c]=window.scrollY}try{sessionStorage.setItem(t||wp,JSON.stringify(fc))}catch(c){na(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[l.state,e,s,i,o,t])),typeof document<"u"&&(te.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||wp);c&&(fc=JSON.parse(c))}catch{}},[t]),te.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(fc,()=>window.scrollY,e?(d,m)=>Sp(d,m,s,e):void 0);return()=>c&&c()},[a,s,e]),te.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{na(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function YT(e,t){let{capture:a}=t||{};te.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function H0(e,{relative:t}={}){let a=te.useContext(Rp);Te(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=Bp("useViewTransitionState"),r=ni(e,{relative:t});if(!a.isTransitioning)return!1;let s=Wa(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=Wa(a.nextLocation.pathname,n)||a.nextLocation.pathname;return Jo(r.pathname,i)!=null||Jo(r.pathname,s)!=null}var Dt=new Ud({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var zp="ironclaw_token",Ke="/api/webchat/v2",qr=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function Sa(){return sessionStorage.getItem(zp)||""}function ri(e){e?sessionStorage.setItem(zp,e):sessionStorage.removeItem(zp)}function xc(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function V0(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function Q0(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function G0({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=Q0(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=Q0(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function V(e,t={}){let a=Sa(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await V0(r);throw new qr(G0({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function $c(){return V(`${Ke}/session`)}function wc({clientActionId:e,requestedThreadId:t,projectId:a}={}){let n={client_action_id:e||xc()};return t&&(n.requested_thread_id=t),a&&(n.project_id=a),V(`${Ke}/threads`,{method:"POST",body:JSON.stringify(n)})}function Y0({limit:e,cursor:t,projectId:a}={}){let n=new URL(`${Ke}/threads`,window.location.origin);return e!=null&&n.searchParams.set("limit",String(e)),t&&n.searchParams.set("cursor",t),a&&n.searchParams.set("project_id",a),V(n.pathname+n.search)}function J0({threadId:e}={}){return e?V(`${Ke}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function qp(e){return`${Ke}/threads/${encodeURIComponent(e)}/files`}function X0({threadId:e,path:t}={}){if(!e)return Promise.reject(new Error("threadId is required"));let a=new URL(qp(e),window.location.origin);return t&&a.searchParams.set("path",t),V(a.pathname+a.search)}function W0({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${qp(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),V(a.pathname+a.search)}function Sc({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${qp(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function Z0({limit:e,runLimit:t,includeCompleted:a}={}){let n=new URLSearchParams;e!=null&&n.set("limit",String(e)),t!=null&&n.set("run_limit",String(t)),a===!0&&n.set("include_completed","true");let r=n.toString();return V(`${Ke}/automations${r?`?${r}`:""}`)}function e$({automationId:e}={}){return e?V(`${Ke}/automations/${encodeURIComponent(e)}/pause`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function t$({automationId:e}={}){return e?V(`${Ke}/automations/${encodeURIComponent(e)}/resume`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function a$({automationId:e}={}){return e?V(`${Ke}/automations/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("automationId is required"))}var n$=`${Ke}/projects`;function JT(e){return`${n$}/${encodeURIComponent(e)}`}function r$({limit:e}={}){let t=new URL(n$,window.location.origin);return e!=null&&t.searchParams.set("limit",String(e)),V(t.pathname+t.search)}function s$({projectId:e}={}){return e?V(JT(e)):Promise.reject(new Error("projectId is required"))}function i$(){return V(`${Ke}/outbound/preferences`)}function o$(){return V(`${Ke}/outbound/targets`)}function l$({finalReplyTargetId:e}={}){return V(`${Ke}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function Ip({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:l,source:c,tail:d,follow:m}={}){let f=new URL(`${Ke}/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),l&&f.searchParams.set("tool_name",l),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),V(f.pathname+f.search)}function u$({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:l,source:c,tail:d,follow:m}={}){let f=new URL(`${Ke}/operator/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),l&&f.searchParams.set("tool_name",l),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),V(f.pathname+f.search)}function c$({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||xc(),content:t};return a.length>0&&(r.attachments=a),V(`${Ke}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function d$({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${Ke}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),V(n.pathname+n.search)}function m$({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${Ke}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function Aa(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new qr("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=Sa(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await V0(r);throw new qr(G0({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function Hp(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function Nc(e){return Hp(await Aa(e))}function f$({threadId:e,afterCursor:t}={}){let a=new URL(`${Ke}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=Sa();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function p$({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||xc()};return a&&(r.reason=a),V(`${Ke}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function Kp({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let l={client_action_id:i||xc(),resolution:n};return r!=null&&(l.always=r),s&&(l.credential_ref=s),V(`${Ke}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(l)})}function h$({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return V("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function v$(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),V(`${Ke}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function si(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function g$(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function y$(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new qr("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new qr("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function b$(){let e=Sa();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var _c="anon",x$=_c;function $$(e){x$=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:_c}function mt(){return x$}var w$="ironclaw:v2-thread-pins:",Qp=new Set,kn=new Set,Vp=null;function Gp(){return`${w$}${mt()}`}function XT(){try{let e=window.localStorage.getItem(Gp());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function WT(){try{kn.size===0?window.localStorage.removeItem(Gp()):window.localStorage.setItem(Gp(),JSON.stringify([...kn]))}catch{}}function S$(){let e=mt();if(e!==Vp){kn.clear();for(let t of XT())kn.add(t);Vp=e}}function N$(){return new Set(kn)}function _$(){let e=N$();for(let t of Qp)try{t(e)}catch{}}function R$(e){e&&(S$(),kn.has(e)?kn.delete(e):kn.add(e),WT(),_$())}function k$(){return S$(),N$()}function C$(e){return Qp.add(e),()=>{Qp.delete(e)}}function E$(){kn.clear(),Vp=mt();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(w$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}_$()}var ZT=0,Ir={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function Yp(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function T$(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":eA(t)?"text":"download"}function eA(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function Wo(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function tA(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function aA(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function nA(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function A$(e,{limits:t,existing:a=[],t:n}){let r=t||Ir,s=[],i=[],o=a.length,l=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!tA(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:Wo(r.maxFileBytes)}));continue}if(l+c.size>r.maxTotalBytes){let y=n("chat.attachmentTotalTooLarge",{max:Wo(r.maxTotalBytes)});i.includes(y)||i.push(y);continue}let d;try{d=await aA(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:m,base64:f}=nA(d,c.type),h=m||"application/octet-stream",b=Yp(h);s.push({id:`staged-${ZT++}`,filename:c.name||"attachment",mimeType:h,kind:b,sizeBytes:c.size,sizeLabel:Wo(c.size),dataBase64:f,previewUrl:b==="image"?d:null}),o+=1,l+=c.size}return{staged:s,errors:i}}function D$(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function M$(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}var Rc="__ironclaw_attachments_only_v1__";function rA(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||Yp(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?m$({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?Wo(n.size_bytes):"",preview_url:null,fetch_url:s}})}function L$(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let m=lA(s);if(!m)continue;let f=`tool-${m.invocationId}`;if(n.has(f))continue;n.add(f),r.push({id:f,role:"tool_activity",...m,timestamp:O$(s)||m.updatedAt||null,sequence:s.sequence,activityOrder:m.activityOrder,activityOrderSource:m.activityOrderSource,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=oA(s),l=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy"),c=rA(s,a),d=o==="user"&&c?.length>0&&s.content===Rc?"":s.content||"";r.push({id:i,role:o,content:d,attachments:c,timestamp:O$(s),kind:s.kind,status:l?"error":s.status,...l&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:iA(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=sA(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function sA(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function iA(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function oA(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function O$(e){return e.received_at||e.created_at||null}function lA(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:Jp(t)}var uA="gate_declined";function Jp(e){let t=e.status==="failed"||e.status==="killed",a=e.error_kind||null,n=j$(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:el(e.title||e.capability_id)||"tool",toolStatus:U$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(P$(a)||e.output_summary||e.output_preview||e.result_ref)||null,toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null,activityOrder:n,activityOrderSource:Number.isFinite(n)?"projection":null}}function Xp(e){let t=j$(e.activity_order),a=e.error_kind||null;return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:el(e.capability_id)||"tool",toolStatus:U$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:null,toolError:P$(a),toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null,activityOrder:t,activityOrderSource:Number.isFinite(t)?"projection":null}}function P$(e){return e||null}function Zo(e){return e==="success"||e==="error"||e==="declined"}function el(e){let t=typeof e=="string"?e.trim():"";if(!t)return"";let a=t.split(".");return a[a.length-1]||t}function U$(e,t=null){if(t===uA)return"declined";switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}function j$(e){let t=Number(e);return Number.isFinite(t)?t:null}var cA=50,Da=new Map,dA=30;function tl(e,t){for(Da.delete(e),Da.set(e,t);Da.size>dA;){let a=Da.keys().next().value;Da.delete(a)}}function ii(e){return`${mt()}:${e}`}function B$(){Da.clear()}function z$(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?Da.get(ii(e)):null,[s,i]=p.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),[o,l]=p.default.useState(e);if(o!==e){let h=e?Da.get(ii(e)):null;l(e),i({messages:h?.messages||[],nextCursor:h?.nextCursor||null,isLoading:!!e&&!h,loadError:null})}let c=p.default.useRef(new Set),d=p.default.useRef(e);d.current=e;let m=p.default.useCallback(async(h,b={})=>{let{preserveClientOnly:y=!1,finalReplyTimestampByRun:$=null}=b;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(c.current.has(e))return;c.current.add(e);let g=mt(),v=ii(e);i(x=>({...x,isLoading:!0}));try{let x=await d$({threadId:e,limit:cA,cursor:h});if(mt()!==g)return;let w=h?[]:a?.()||[],S=L$(x.messages||[],w,e),k=x.next_cursor||null;if(h||n?.([]),!h){let N=Da.get(v)?.messages||[],C=F$(S,N,{preserveClientOnly:y,finalReplyTimestampByRun:$});tl(v,{messages:C,nextCursor:k})}i(N=>{if(d.current!==e)return N;let C;return h?C=mA(S,N.messages):C=F$(S,N.messages,{preserveClientOnly:y,finalReplyTimestampByRun:$}),tl(v,{messages:C,nextCursor:k}),{messages:C,nextCursor:k,isLoading:!1,loadError:null}})}catch(x){if(console.error("Failed to load timeline:",x),mt()!==g)return;i(w=>d.current===e?{...w,isLoading:!1,loadError:"Failed to load conversation history."}:w)}finally{c.current.delete(e)}},[e,a,n]);p.default.useEffect(()=>{let h=e?Da.get(ii(e)):null;i({messages:h?.messages||[],nextCursor:h?.nextCursor||null,isLoading:!!e&&!h,loadError:null}),e&&m()},[e,m]);let f=p.default.useCallback((h,b)=>{if(!h)return;let y=ii(h),$=x=>typeof b=="function"?b(x||[]):b;if(d.current===h){i(x=>{let w=$(x.messages||[]);return tl(y,{messages:w,nextCursor:x.nextCursor||null}),{...x,messages:w}});return}let g=Da.get(y)||{messages:[],nextCursor:null},v=$(g.messages||[]);tl(y,{messages:v,nextCursor:g.nextCursor||null})},[]);return{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:m,seedThreadMessages:f,setMessages:h=>i(b=>{let y=typeof h=="function"?h(b.messages):h;return e&&tl(ii(e),{messages:y,nextCursor:b.nextCursor}),{...b,messages:y}})}}function mA(e,t){let a=new Set(t.map(n=>n?.id).filter(Boolean));return[...e.filter(n=>!a.has(n?.id)),...t]}function F$(e,t,a={}){let{preserveClientOnly:n=!1,finalReplyTimestampByRun:r=null}=a,s=pA(e,t,{finalReplyTimestampByRun:r}),i=new Set(s.map(l=>l?.id).filter(Boolean)),o=t.filter(l=>!l||typeof l.id!="string"||i.has(l.id)?!1:q$(l)?!0:typeof l.timelineMessageId=="string"&&i.has(`msg-${l.timelineMessageId}`)?!1:fA(l)?!0:n&&l.id.startsWith("err-"));return o.length>0?hA(s,o,t):s}function fA(e){return e?.isOptimistic===!0&&typeof e.id=="string"&&e.id.startsWith("pending-")&&(e.role==="user"||e.role==="assistant")}function pA(e,t,a={}){let{finalReplyTimestampByRun:n=null}=a,r=new Map,s=new Map;for(let i of t||[])!i||!i.timestamp||(typeof i.id=="string"&&r.set(i.id,i),typeof i.timelineMessageId=="string"&&r.set(`msg-${i.timelineMessageId}`,i),Wp(i)&&typeof i.turnRunId=="string"&&s.set(i.turnRunId,i));return r.size===0&&s.size===0&&!n?e:e.map(i=>{if(!i||i.timestamp||typeof i.id!="string")return i;let o=typeof i.turnRunId=="string"?i.turnRunId:null,l=r.get(i.id)||(Wp(i)&&o?s.get(o):null),c=Wp(i)&&o?n?.[o]:null,d=l?.timestamp||c;return d?{...i,timestamp:d}:i})}function Wp(e){return e?.role==="assistant"&&e?.isFinalReply===!0}function q$(e){return e?.role==="tool_activity"||e?.role==="thinking"}function hA(e,t,a){let n=new Map;for(let[l,c]of e.entries())typeof c?.id=="string"&&n.set(c.id,l);let r=a.map(l=>vA(l,n)),s=new Map,i=[];for(let l of t){if(!q$(l)){i.push(l);continue}let c=a.indexOf(l),d=null;for(let m=c-1;m>=0;m-=1)if(r[m]!==null){d=r[m];break}if(d!==null){let m=s.get(d)||[];m.push(l),s.set(d,m)}else i.push(l)}let o=[];for(let[l,c]of e.entries()){o.push(c);let d=s.get(l);d&&o.push(...d)}return o.push(...i),o}function vA(e,t){if(!e)return null;if(typeof e.id=="string"&&t.has(e.id))return t.get(e.id);if(typeof e.timelineMessageId=="string"){let a=`msg-${e.timelineMessageId}`;if(t.has(a))return t.get(a)}return null}var nl="__new__",I$="ironclaw:v2-draft:";function oi(e){return`${I$}${mt()}:${e||nl}`}function Zp(e){try{return window.localStorage.getItem(oi(e))||""}catch{return""}}function eh(e,t){try{t?window.localStorage.setItem(oi(e),t):window.localStorage.removeItem(oi(e))}catch{}}function H$(e){eh(e,"")}var al=new Map;function th(e){return al.get(oi(e))||[]}function kc(e,t){let a=oi(e);t&&t.length>0?al.set(a,t):al.delete(a)}function K$(e){al.delete(oi(e))}function Q$(){al.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(I$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function gA(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function yA(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function bA(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=gA(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?yA(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),Sa()?"":(ri(n),n)}function xA(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var $A={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function wA(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),$A[t]||"Could not complete sign-in. Please try again."):""}function V$(){let[e,t]=p.default.useState(()=>bA()||Sa()),[a,n]=p.default.useState(()=>wA()),[r]=p.default.useState(()=>xA()),[s,i]=p.default.useState(null),[o,l]=p.default.useState(()=>!!(r&&!Sa())),[c,d]=p.default.useState(()=>!!Sa());p.default.useEffect(()=>{if(!r||Sa()){l(!1);return}let b=!1;return y$(r).then(y=>{b||(ri(y),d(!0),t(y),i(null),n(""),l(!1),Dt.clear())}).catch(()=>{b||(n("Could not complete sign-in. Please try again."),l(!1))}),()=>{b=!0}},[r]),p.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let b=!1;return d(!0),$c().then(y=>{b||(i(y),d(!1))}).catch(y=>{b||(i(null),d(!1),(y?.status===401||y?.status===403)&&(ri(""),t(""),n("Your session expired. Please sign in again."),Dt.clear()))}),()=>{b=!0}},[e,o]),$$(s);let m=p.default.useRef(null);p.default.useEffect(()=>{let b=mt();m.current&&m.current!==_c&&m.current!==b&&(B$(),Q$(),E$()),m.current=b},[s]);let f=p.default.useCallback(b=>{ri(b),d(!!b),t(b),i(null),n(""),Dt.clear()},[]),h=p.default.useCallback(()=>{b$().catch(()=>{}),ri(""),d(!1),t(""),i(null),n(""),Dt.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,rebornProjectsEnabled:!!s?.features?.reborn_projects,signIn:f,signOut:h}}var Hr="/chat",rl=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace"},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"logs",path:"/logs",labelKey:"nav.logs",hidden:!0},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var SA=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"tools",labelKey:"settings.tools",icon:"tool"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],NA=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],_A=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],Cc={settings:SA,extensions:NA,admin:_A};var G$="ironclaw:v2-theme";function RA(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(G$);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function Ec(){let[e,t]=p.default.useState(RA);p.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(G$,e)}catch{}},[e]);let a=p.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function Y$(e){return H({enabled:!!e,queryKey:["gateway-status",e],queryFn:si,refetchInterval:3e4})}var kA="/api/webchat/v2/operator/config",Tc="/api/webchat/v2/settings/tools",li="agent.auto_approve_tools",J$="tool.",CA=new Set(["always_allow","ask_each_time","disabled"]),EA=new Set(["default","always_allow","ask_each_time","disabled"]);function X$(e){return e==="ask"?"ask_each_time":CA.has(e)?e:"ask_each_time"}function TA(e){return e==="ask"?"ask_each_time":EA.has(e)?e:"default"}function AA(e){return["default","global","override"].includes(e)?e:"default"}function W$(e){if(!e?.key?.startsWith(J$))return null;let t=e.value||{};return{name:t.name||e.key.slice(J$.length),description:t.description||"",state:X$(t.state),default_state:X$(t.default_state),locked:!!(t.locked||e.mutable===!1),effective_source:AA(t.effective_source||e.source)}}function DA(e){let t={};for(let a of e.entries||[])a?.key===li&&(t[li]=!!a.value);return t}async function Z$(){let e=await V(Tc);return{settings:DA(e),diagnostics:e.diagnostics||[],precedence:e.precedence||[]}}async function ah(e,t){if(e===li){let n=await V(Tc,{method:"POST",body:JSON.stringify({enabled:!!t})});return{success:!0,entry:n.entry,value:n.entry?.value}}let a=await V(`${kA}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({value:t})});return{success:!0,entry:a.entry,value:a.entry?.value}}async function ew(e){let t=e?.settings||{},a=[];return Object.prototype.hasOwnProperty.call(t,li)&&a.push(await ah(li,!!t[li])),{success:!0,imported:a.length,results:a}}function Ac(){return V("/api/webchat/v2/llm/providers")}function tw(e){return V("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function aw(e){return V(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function sl(e){return V("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function nw(e){return V("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function rw(e){return V("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function sw(e){return V("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function iw(e){return V("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function ow(){return V("/api/webchat/v2/llm/codex/login",{method:"POST"})}async function lw(){let e=await V(Tc);return{tools:(e.entries||[]).map(W$).filter(Boolean),diagnostics:e.diagnostics||[]}}async function uw(e,t){let a=TA(t),n=await V(`${Tc}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({state:a})});return{success:!0,tool:W$(n.entry),entry:n.entry}}function cw(){return V("/api/webchat/v2/extensions")}function dw(){return V("/api/webchat/v2/extensions/registry")}function mw(){return V("/api/webchat/v2/skills")}function fw(e){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function pw(e){return V("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function hw(e,t){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function vw(e){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function gw(e,t){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}/auto-activate`,{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:t})})}function yw(e){return V("/api/webchat/v2/skills/auto-activate-learned",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function bw(){return V("/api/webchat/v2/traces/credit")}function xw(e){return V(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function $w(){return Promise.resolve({users:[],todo:!0})}function ww(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function Sw(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var nh="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",rh=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function il(e){return rh.find(t=>t.value===e)?.label||e}function ui(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function Nw(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function Dc(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function _w(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function Kr(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===nh||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?ui(e,t).trim().length>0:!0:!1}function MA(e,t,a){return e.id===a?"active":Kr(e,t)?"ready":"setup"}function Rw(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=MA(r,t,a);n[s]&&n[s].push(r)}return n}function Mc(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===nh||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!ui(e,t).trim()?"base_url":"ok"}function sh(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===nh&&(i.api_key=void 0),i}function kw(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function Cw(e){return/^[a-z0-9_-]+$/.test(e)}function Ew(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var OA=Object.freeze({});function ci({settings:e,gatewayStatus:t,enabled:a=!0}){let n=W(),r=H({queryKey:["llm-providers"],queryFn:Ac,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=OA,l=(s.providers||[]).map(w=>({...w,name:w.description,has_api_key:w.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,m=d||"nearai",f=s.active?.model||t?.llm_model||"",h=l.filter(w=>w.builtin),b=l.filter(w=>!w.builtin),y=[...l].sort((w,S)=>w.id===d?-1:S.id===d?1:(w.name||w.id).localeCompare(S.name||S.id)),$=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=G({mutationFn:async w=>{if(!Kr(w,o)){let k=Mc(w,o);throw new Error(k==="base_url"?"base_url":"api_key")}let S=Dc(w,o);if(!S)throw new Error("model");return await sl({provider_id:w.id,model:S}),w},onSuccess:$}),v=G({mutationFn:async({provider:w,form:S,apiKey:k,editingProvider:N})=>{let C=!!w?.builtin,L={id:(C?w.id:S.id.trim()).trim(),name:C?w.name||w.id:S.name.trim(),adapter:C?w.adapter:S.adapter,base_url:S.baseUrl.trim()||w?.base_url||"",default_model:S.model.trim()||void 0};return k.trim()&&(L.api_key=k.trim()),(N||w)?.id===m&&L.default_model&&(L.set_active=!0,L.model=L.default_model),await tw(L),L},onSuccess:$}),x=G({mutationFn:async w=>(await aw(w.id),w),onSuccess:$});return{providers:y,builtinProviders:h,customProviders:b,builtinOverrides:o,activeProviderId:d,selectedModel:f,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:w=>g.mutateAsync(w),saveCustomProvider:w=>v.mutateAsync(w),saveBuiltinProvider:w=>v.mutateAsync(w),deleteCustomProvider:w=>x.mutateAsync(w),testConnection:nw,listModels:rw,isBusy:g.isPending||v.isPending||x.isPending}}function Tw({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}var Aw="ironclaw:v2-sidebar-open";function Dw(){return typeof window>"u"?null:window}function Mw(){try{return Dw()?.localStorage||null}catch{return null}}function Ow(e=Mw()){try{return e?.getItem(Aw)!=="false"}catch{return!0}}function Lw(e,t=Mw()){try{t?.setItem(Aw,e?"true":"false")}catch{}}function Pw(e=Dw()){try{return e?.matchMedia?.("(min-width: 768px)").matches===!0}catch{return!1}}function Uw(e,t){return t?{...e,desktopOpen:!e.desktopOpen}:{...e,mobileOpen:!e.mobileOpen}}function jw(e,t){return t?e.desktopOpen:e.mobileOpen}function Fw({onNewChat:e}={}){let t=ve(),[a,n]=p.default.useState(()=>({mobileOpen:!1,desktopOpen:Ow()})),[r,s]=p.default.useState(()=>Pw());p.default.useEffect(()=>{let d=window.matchMedia("(min-width: 768px)"),m=()=>s(d.matches);return m(),d.addEventListener?.("change",m),()=>d.removeEventListener?.("change",m)},[]),p.default.useEffect(()=>{Lw(a.desktopOpen)},[a.desktopOpen]);let i=p.default.useCallback(()=>{n(d=>({...d,mobileOpen:!1}))},[]),o=p.default.useCallback(()=>{n(d=>Uw(d,r))},[r]),l=p.default.useCallback(async()=>{let d=await e?.(),m=typeof d=="string"&&d.length>0?d:null;t(m?`/chat/${m}`:"/chat"),i()},[t,i,e]),c=p.default.useCallback(d=>{t(`/chat/${d}`),i()},[t,i]);return{mobileOpen:a.mobileOpen,desktopOpen:a.desktopOpen,currentOpen:jw(a,r),close:i,toggle:o,newChat:l,selectThread:c}}var ih=new Set,LA=0;function di(e,t={}){let a={id:++LA,message:e,tone:t.tone||"info",duration:t.duration??2600};return ih.forEach(n=>n(a)),a.id}function Bw(e){return ih.add(e),()=>ih.delete(e)}function PA(e){return e?.status===409&&e?.payload?.kind==="busy"}function zw(e,t){return PA(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function qw(){let e=H({queryKey:["threads"],queryFn:()=>Y0({})}),[t,a]=p.default.useState(null),[n,r]=p.default.useState(!1),s=p.default.useRef(new Map),i=p.default.useCallback(async c=>{let d=c||"__global__",m=s.current.get(d);if(m)return m;r(!0);let f=(async()=>{try{let h=await wc(c?{projectId:c}:void 0);Dt.invalidateQueries({queryKey:["threads"]});let b=h?.thread?.thread_id;return b&&a(b),b}finally{s.current.delete(d),r(s.current.size>0)}})();return s.current.set(d,f),f},[]),o=p.default.useCallback(async c=>{await J0({threadId:c}),t===c&&a(null),Dt.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:p.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var Iw={attach:u`<path
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
      ${Iw[e]||Iw.spark}
    </svg>
  `}function Y(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=Y(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function Hw(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function UA(e){return Hw(e).trim().charAt(0).toUpperCase()||"I"}function jA(){let[e,t]=p.default.useState(!1),a=p.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function Kw({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=R(),s=jA(),i=Hw(a),o=a?.email||a?.role||r("common.gatewaySession");return u`
    <div
      className="relative flex items-center gap-2 border-t border-[var(--v2-panel-border)] px-3 py-3"
    >
      ${s.open&&u`
        <div
          className=${Y("absolute bottom-full left-3 right-3 mb-2 rounded-[10px] border p-3 shadow-lg","border-[var(--v2-panel-border)] bg-[var(--v2-surface)]")}
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
            />`:u`<span className="place-self-center">${UA(a)}</span>`}
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
  `}var Qw={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",logs:"list",settings:"settings",admin:"shield"},FA=rl.filter(e=>e.id!=="chat"&&!e.hidden);function BA({route:e,label:t,onNavigate:a}){return u`
    <${Za}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>Y("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${M} name=${Qw[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function zA({route:e,label:t,subRoutes:a,onNavigate:n}){let r=R(),s=Ae(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return u`
    <div className="flex flex-col">
      <${Za}
        to=${o}
        onClick=${n}
        className=${()=>Y("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${M}
          name=${Qw[e.id]||"bolt"}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1 truncate">${t}</span>
        <${M}
          name="chevron"
          className=${Y("h-3.5 w-3.5 shrink-0 transition-transform duration-150",i&&"rotate-180")}
        />
      <//>

      ${i&&u`
        <div className="mt-0.5 flex flex-col gap-0.5 pl-3">
          ${a.map(l=>u`
              <${Za}
                key=${l.id}
                to=${e.path+"/"+l.id}
                onClick=${n}
                className=${({isActive:c})=>Y("flex items-center gap-2.5 rounded-[8px] py-1.5 pl-7 pr-3 text-[12px] font-medium",c?"text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
              >
                <${M} name=${l.icon} className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate">${r(l.labelKey)}</span>
              <//>
            `)}
        </div>
      `}
    </div>
  `}function Vw({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=R(),s=p.default.useMemo(()=>FA.filter(i=>a||i.id!=="admin"),[a]);return u`
    <div className="flex flex-col px-3 py-2">
      <button
        data-testid="new-chat"
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
        ${s.map(i=>{let o=(Cc[i.id]||[]).filter(l=>a||!(i.id==="settings"&&["users","inference"].includes(l.id)));return o.length>0?u`
              <${zA}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:u`
            <${BA}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var Na=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),ol=new Set([Na.NEEDS_ATTENTION,Na.FAILED]),oh="ironclaw:v2-thread-attention",lh=new Set,mi=new Map;function qA(){try{let e=window.localStorage.getItem(oh);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&ol.has(a[1])):[]}catch{return[]}}function Gw(){let e=[];for(let[t,a]of mi)ol.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(oh):window.localStorage.setItem(oh,JSON.stringify(e))}catch{}}for(let[e,t]of qA())mi.set(e,t);function Jw(){return new Map(mi)}function Yw(){let e=Jw();for(let t of lh)try{t(e)}catch{}}function Oc(e,t){if(!e)return;let a=mi.get(e);if(t==null){if(!mi.delete(e))return;ol.has(a)&&Gw(),Yw();return}a!==t&&(mi.set(e,t),(ol.has(t)||ol.has(a))&&Gw(),Yw())}function Xw(e){Oc(e,null)}function IA(){return Jw()}function HA(e){return lh.add(e),()=>{lh.delete(e)}}function Ww(){let[e,t]=p.default.useState(IA);return p.default.useEffect(()=>HA(t),[]),e}function Lc(e){return e.updated_at||e.created_at||null}function uh(e,t){let a=Lc(e)||"",n=Lc(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function Zw(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function e1(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function KA(){let[e,t]=p.default.useState(k$);return p.default.useEffect(()=>C$(t),[]),e}var QA=Object.freeze({[Na.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-transparent"},[Na.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[Na.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function VA(e){return e&&QA[e]||null}function GA(e){let t=String(e?.state||"").toLowerCase();return t==="processing"||t==="running"?Na.RUNNING:t==="needs_attention"||t==="awaitingapproval"||t==="awaiting_approval"?Na.NEEDS_ATTENTION:t==="failed"||t==="interrupted"?Na.FAILED:null}function YA({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=R(),o=Lc(e),l=Zw(o),c=e1(o),d=p.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(h=>{window.alert(h?.message||"Unable to delete chat")})},[s,e.id]),m=p.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),R$(e.id)},[e.id]);return u`
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
          ${n&&u`<span
            aria-label=${n.label}
            className=${Y("h-1.5 w-1.5 shrink-0 rounded-full",n.dotClass)}
          />`}
        </div>
        ${(n||l)&&u`<span
          className=${Y("block truncate text-[11px]",n?n.textClass:"text-[var(--v2-text-faint)]")}
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
        className=${Y("my-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] transition",a?"text-[var(--v2-accent-text)]":"opacity-0 text-[var(--v2-text-faint)] group-hover:opacity-100 focus:opacity-100","hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-accent-text)]")}
      >
        <${M} name="pin" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>
      ${s&&u`<button
        type="button"
        onClick=${d}
        title=${i("common.deleteChat")}
        aria-label=${i("common.deleteChat")}
        className=${Y("my-1 mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px]","opacity-0 transition group-hover:opacity-100 focus:opacity-100","text-[var(--v2-text-faint)] hover:bg-[var(--v2-danger-soft)] hover:text-[var(--v2-danger-text)]")}
      >
        <${M} name="trash" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>`}
    </div>
  `}function t1({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:u`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>u`
          <${YA}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${VA(n.has(o.id)?n.get(o.id):GA(o))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function a1({threads:e,activeThreadId:t,rebornProjectsEnabled:a=!1,onSelect:n,onDelete:r,onNavigate:s}){let[i,o]=p.default.useState(!1),[l,c]=p.default.useState(""),d=Ww(),m=KA(),f=R(),{pinned:h,recent:b,totalMatches:y}=p.default.useMemo(()=>{let $=l.trim().toLowerCase(),g=$?e.filter(w=>(w.title||w.id||"").toLowerCase().includes($)):e,v=[],x=[];for(let w of g)m.has(w.id)?v.push(w):x.push(w);return v.sort(uh),x.sort(uh),{pinned:v,recent:x,totalMatches:v.length+x.length}},[e,l,m]);return u`
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
          <${Za}
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
          ${e.length===0&&u`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            ${f("chat.noConversations")}
          </div>`}
          ${e.length>0&&y===0&&u`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            ${f("common.noChatsMatch").replace("{query}",l)}
          </div>`}

          <${t1}
            label=${f("common.pinned")}
            items=${h}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${m}
            onSelect=${n}
            onDelete=${r}
          />
          <${t1}
            label=${f("common.recent")}
            items=${b}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${m}
            onSelect=${n}
            onDelete=${r}
          />
        </div>
      `}
    </div>
  `}function Pc(){let e=W(),t=H({queryKey:["trace-credits"],queryFn:bw,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=G({mutationFn:xw,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function JA(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function n1(){let e=R(),{credits:t}=Pc();if(!t||!t.enrolled)return null;let a=JA(t.final_credit),n=t.submissions_accepted||0,r=t.submissions_submitted||0,s=t.manual_review_hold_count||0;return u`
    <div className="px-3 pb-1">
      <${Rn}
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
        ${s>0&&u`
          <div className="mt-1 text-[11px] font-medium text-[var(--v2-accent-text)]">
            ${e("traceCommons.cardHeld",{count:s})}
          </div>
        `}
      <//>
    </div>
  `}function r1({id:e,threadsState:t,theme:a,toggleTheme:n,profile:r,isAdmin:s,rebornProjectsEnabled:i=!1,onSignOut:o,onClose:l,onNewChat:c,onSelectThread:d,onDeleteThread:m}){return u`
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

      <${Vw}
        onNewChat=${c}
        isCreating=${t.isCreating}
        isAdmin=${s}
        onNavigate=${l}
      />

      <${n1} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${a1}
          threads=${t.threads}
          activeThreadId=${t.activeThreadId}
          rebornProjectsEnabled=${i}
          onSelect=${d}
          onDelete=${m}
          onNavigate=${l}
        />
      </div>

      <${Kw}
        theme=${a}
        toggleTheme=${n}
        profile=${r}
        onSignOut=${o}
      />
    </aside>
  `}var XA="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",WA="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",s1="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",i1={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},o1={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function A({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=i1[n]??i1.md,l=r?"w-full":"";if(a==="primary")return u`
      <${s}
        style=${{background:XA,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${Y(s1,o,l,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:WA}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=o1[a]??o1.outline;return u`
    <${s}
      className=${Y(s1,o,l,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function l1(){let e=p.default.useMemo(()=>ZA(window.location),[]),[t,a]=p.default.useState(null),[n,r]=p.default.useState(null),[s,i]=p.default.useState(!1),[o,l]=p.default.useState(""),[c,d]=p.default.useState(!1);p.default.useEffect(()=>{if(!e)return;let h=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:h.signal}).then(b=>{if(!b.ok)throw new Error(String(b.status));return b.json()}).then(a).catch(()=>{h.signal.aborted||a(null)}),()=>h.abort()},[e]);let m=p.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),l("");try{let h=await fetch(`${e.base}/attestation/report`);if(!h.ok)throw new Error(String(h.status));let b=await h.json();return r(b),b}catch(h){return l(h.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),f=p.default.useCallback(async()=>{let h=n||await m();return!h||!navigator.clipboard?!1:(await navigator.clipboard.writeText(JSON.stringify({...h,instance_attestation:t},null,2)),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[m,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:m,copyReport:f}}function ZA(e){let t=e.hostname;if(!t||t==="localhost"||e4(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function e4(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}var t4=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function u1(){let e=R(),t=l1(),[a,n]=p.default.useState(!1),r=p.default.useCallback(()=>{n(o=>{let l=!o;return l&&t.loadReport(),l})},[t]),s=p.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=a4({teeInfo:t.teeInfo,report:t.report,t:e});return u`
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

      ${a&&u`
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
  `}function a4({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return t4.map(([r,s])=>({label:a(s),value:n4(n[r])||a("common.unknown")}))}function n4(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var r4="https://docs.ironclaw.com";function c1({threadsState:e,onToggleSidebar:t,sidebarOpen:a=!0}){let n=R(),r=Ae(),s=p.default.useMemo(()=>{for(let o of rl){let l=Cc[o.id];if(!l)continue;let c=o.path+"/";if(r.pathname.startsWith(c)){let d=r.pathname.slice(c.length).split("/")[0],m=l.find(f=>f.id===d);if(m)return{parent:n(o.labelKey),current:n(m.labelKey)}}}return null},[r.pathname,n]),i=p.default.useMemo(()=>{if(s)return null;if(r.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(c=>c.id===e.activeThreadId)?.title||n("nav.chat");let o=rl.find(l=>r.pathname.startsWith(l.path));return o?n(o.labelKey):""},[r.pathname,e.activeThreadId,e.threads,n,s]);return u`
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
        <${Za}
          to="/logs"
          className=${({isActive:o})=>Y("inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",o&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
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
  `}function d1({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=ve(),i=R(),[o,l]=p.default.useState(""),[c,d]=p.default.useState(0),m=p.default.useRef(null),f=p.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(x=>({id:`thread-${x.id}`,label:x.title||`Thread ${x.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${x.id}`)}));return[...g,...v]},[a,s,n,r]),h=p.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?f.filter(v=>v.label.toLowerCase().includes(g)):f},[f,o]);p.default.useEffect(()=>{if(!e)return;l(""),d(0);let g=window.requestAnimationFrame(()=>m.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),p.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,h.length-1)))},[h.length]);let b=p.default.useCallback(g=>{g&&(t(),g.run())},[t]),y=p.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,h.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),b(h[c])):g.key==="Escape"&&(g.preventDefault(),t())},[h,c,b,t]);if(!e)return null;let $=null;return u`
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
          ${h.map((g,v)=>{let x=g.group!==$;return $=g.group,u`
              ${x&&u`<li key=${`g-${g.group}`} className="px-2 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">${g.group}</li>`}
              <li key=${g.id}>
                <button
                  type="button"
                  onMouseEnter=${()=>d(v)}
                  onClick=${()=>b(g)}
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
  `}var m1={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},s4={info:"bolt",success:"check",error:"close"};function f1(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>Bw(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:u`
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
  `}function p1({token:e,profile:t,isChecking:a=!1,isAdmin:n,rebornProjectsEnabled:r=!1,onSignOut:s}){let i=R(),{theme:o,toggleTheme:l}=Ec(),c=Y$(e),d=qw(),m=Fw({onNewChat:()=>d.setActiveThreadId(null)}),f=c.data,h=Ae(),b=ve(),y=ci({settings:{},gatewayStatus:f,enabled:n}),$=n&&Tw({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),g=h.pathname==="/welcome"||h.pathname.startsWith("/settings"),[v,x]=p.default.useState(!1);p.default.useEffect(()=>{let S=k=>{(k.metaKey||k.ctrlKey)&&k.key.toLowerCase()==="k"&&(k.preventDefault(),x(N=>!N))};return window.addEventListener("keydown",S),()=>window.removeEventListener("keydown",S)},[]);let w=p.default.useCallback(async S=>{let k=d.activeThreadId===S;try{await d.deleteThread(S),k&&b("/chat",{replace:!0})}catch(N){console.error("Failed to delete thread:",N),di(zw(N,i),{tone:"error"})}},[b,d,i]);return $&&!g?u`<${ot} to="/welcome" replace />`:u`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${m.mobileOpen&&u`<button
        type="button"
        aria-label=${i("nav.close")}
        onClick=${m.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${Y("fixed inset-y-0 left-0 z-50 md:relative md:z-auto",m.mobileOpen?"flex":"hidden",m.desktopOpen?"md:flex":"md:hidden")}
      >
        <${r1}
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
              className=${Y("m-4 rounded-[14px] border px-4 py-3 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >
              ${c.error.message||i("error.gatewayConnection")}
            </div>
          `}
          <${Dp}
            context=${{gatewayStatus:f,gatewayStatusQuery:c,currentUser:t,isChecking:a,isAdmin:n,threadsState:d}}
          />
        </main>
      </div>
      <${d1}
        open=${v}
        onClose=${()=>x(!1)}
        threadsState=${d}
        onNewChat=${m.newChat}
        onToggleTheme=${l}
      />
      <${f1} />
    </div>
  `}var Kt=qe(Qe(),1),ml=e=>e.type==="checkbox",Qr=e=>e instanceof Date,Mt=e=>e==null,k1=e=>typeof e=="object",Ye=e=>!Mt(e)&&!Array.isArray(e)&&k1(e)&&!Qr(e),i4=e=>Ye(e)&&e.target?ml(e.target)?e.target.checked:e.target.value:e,o4=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,l4=(e,t)=>e.has(o4(t)),u4=e=>{let t=e.constructor&&e.constructor.prototype;return Ye(t)&&t.hasOwnProperty("isPrototypeOf")},mh=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function ft(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(mh&&(e instanceof Blob||n))&&(a||Ye(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!u4(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=ft(e[r]));else return e;return t}var zc=e=>/^\w*$/.test(e),Ze=e=>e===void 0,fh=e=>Array.isArray(e)?e.filter(Boolean):[],ph=e=>fh(e.replace(/["|']|\]/g,"").split(/\.|\[/)),Z=(e,t,a)=>{if(!t||!Ye(e))return a;let n=(zc(t)?[t]:ph(t)).reduce((r,s)=>Mt(r)?r:r[s],e);return Ze(n)||n===e?Ze(e[t])?a:e[t]:n},en=e=>typeof e=="boolean",Be=(e,t,a)=>{let n=-1,r=zc(t)?[t]:ph(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],l=a;if(n!==i){let c=e[o];l=Ye(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=l,e=e[o]}},h1={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},Ma={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},Cn={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},c4=Kt.default.createContext(null);c4.displayName="HookFormContext";var d4=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==Ma.all&&(t._proxyFormState[i]=!n||Ma.all),a&&(a[i]=!0),e[i]}});return r},m4=typeof window<"u"?Kt.default.useLayoutEffect:Kt.default.useEffect;var tn=e=>typeof e=="string",f4=(e,t,a,n,r)=>tn(e)?(n&&t.watch.add(e),Z(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),Z(a,s))):(n&&(t.watchAll=!0),a),dh=e=>Mt(e)||!k1(e);function lr(e,t,a=new WeakSet){if(dh(e)||dh(t))return e===t;if(Qr(e)&&Qr(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(Qr(i)&&Qr(o)||Ye(i)&&Ye(o)||Array.isArray(i)&&Array.isArray(o)?!lr(i,o,a):i!==o)return!1}}return!0}var p4=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},cl=e=>Array.isArray(e)?e:[e],v1=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},Qt=e=>Ye(e)&&!Object.keys(e).length,hh=e=>e.type==="file",Oa=e=>typeof e=="function",jc=e=>{if(!mh)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},C1=e=>e.type==="select-multiple",vh=e=>e.type==="radio",h4=e=>vh(e)||ml(e),ch=e=>jc(e)&&e.isConnected;function v4(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=Ze(e)?n++:e[t[n++]];return e}function g4(e){for(let t in e)if(e.hasOwnProperty(t)&&!Ze(e[t]))return!1;return!0}function We(e,t){let a=Array.isArray(t)?t:zc(t)?[t]:ph(t),n=a.length===1?e:v4(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Ye(n)&&Qt(n)||Array.isArray(n)&&g4(n))&&We(e,a.slice(0,-1)),e}var E1=e=>{for(let t in e)if(Oa(e[t]))return!0;return!1};function Fc(e,t={}){let a=Array.isArray(e);if(Ye(e)||a)for(let n in e)Array.isArray(e[n])||Ye(e[n])&&!E1(e[n])?(t[n]=Array.isArray(e[n])?[]:{},Fc(e[n],t[n])):Mt(e[n])||(t[n]=!0);return t}function T1(e,t,a){let n=Array.isArray(e);if(Ye(e)||n)for(let r in e)Array.isArray(e[r])||Ye(e[r])&&!E1(e[r])?Ze(t)||dh(a[r])?a[r]=Array.isArray(e[r])?Fc(e[r],[]):{...Fc(e[r])}:T1(e[r],Mt(t)?{}:t[r],a[r]):a[r]=!lr(e[r],t[r]);return a}var ll=(e,t)=>T1(e,t,Fc(t)),g1={value:!1,isValid:!1},y1={value:!0,isValid:!0},A1=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!Ze(e[0].attributes.value)?Ze(e[0].value)||e[0].value===""?y1:{value:e[0].value,isValid:!0}:y1:g1}return g1},D1=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>Ze(e)?e:t?e===""?NaN:e&&+e:a&&tn(e)?new Date(e):n?n(e):e,b1={isValid:!1,value:null},M1=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,b1):b1;function x1(e){let t=e.ref;return hh(t)?t.files:vh(t)?M1(e.refs).value:C1(t)?[...t.selectedOptions].map(({value:a})=>a):ml(t)?A1(e.refs).value:D1(Ze(t.value)?e.ref.value:t.value,e)}var y4=(e,t,a,n)=>{let r={};for(let s of e){let i=Z(t,s);i&&Be(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},Bc=e=>e instanceof RegExp,ul=e=>Ze(e)?e:Bc(e)?e.source:Ye(e)?Bc(e.value)?e.value.source:e.value:e,$1=e=>({isOnSubmit:!e||e===Ma.onSubmit,isOnBlur:e===Ma.onBlur,isOnChange:e===Ma.onChange,isOnAll:e===Ma.all,isOnTouch:e===Ma.onTouched}),w1="AsyncFunction",b4=e=>!!e&&!!e.validate&&!!(Oa(e.validate)&&e.validate.constructor.name===w1||Ye(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===w1)),x4=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),S1=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),dl=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=Z(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(dl(o,t))break}else if(Ye(o)&&dl(o,t))break}}};function N1(e,t,a){let n=Z(e,a);if(n||zc(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=Z(t,s),o=Z(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var $4=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return Qt(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||Ma.all))},w4=(e,t,a)=>!e||!t||e===t||cl(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),S4=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,N4=(e,t)=>!fh(Z(e,t)).length&&We(e,t),_4=(e,t,a)=>{let n=cl(Z(e,a));return Be(n,"root",t[a]),Be(e,a,n),e},Uc=e=>tn(e);function _1(e,t,a="validate"){if(Uc(e)||Array.isArray(e)&&e.every(Uc)||en(e)&&!e)return{type:a,message:Uc(e)?e:"",ref:t}}var fi=e=>Ye(e)&&!Bc(e)?e:{value:e,message:""},R1=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:l,maxLength:c,minLength:d,min:m,max:f,pattern:h,validate:b,name:y,valueAsNumber:$,mount:g}=e._f,v=Z(a,y);if(!g||t.has(y))return{};let x=o?o[0]:i,w=F=>{r&&x.reportValidity&&(x.setCustomValidity(en(F)?"":F||""),x.reportValidity())},S={},k=vh(i),N=ml(i),C=k||N,P=($||hh(i))&&Ze(i.value)&&Ze(v)||jc(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,L=p4.bind(null,y,n,S),U=(F,T,K,ee=Cn.maxLength,ne=Cn.minLength)=>{let he=F?T:K;S[y]={type:F?ee:ne,message:he,ref:i,...L(F?ee:ne,he)}};if(s?!Array.isArray(v)||!v.length:l&&(!C&&(P||Mt(v))||en(v)&&!v||N&&!A1(o).isValid||k&&!M1(o).isValid)){let{value:F,message:T}=Uc(l)?{value:!!l,message:l}:fi(l);if(F&&(S[y]={type:Cn.required,message:T,ref:x,...L(Cn.required,T)},!n))return w(T),S}if(!P&&(!Mt(m)||!Mt(f))){let F,T,K=fi(f),ee=fi(m);if(!Mt(v)&&!isNaN(v)){let ne=i.valueAsNumber||v&&+v;Mt(K.value)||(F=ne>K.value),Mt(ee.value)||(T=ne<ee.value)}else{let ne=i.valueAsDate||new Date(v),he=Oe=>new Date(new Date().toDateString()+" "+Oe),xt=i.type=="time",pt=i.type=="week";tn(K.value)&&v&&(F=xt?he(v)>he(K.value):pt?v>K.value:ne>new Date(K.value)),tn(ee.value)&&v&&(T=xt?he(v)<he(ee.value):pt?v<ee.value:ne<new Date(ee.value))}if((F||T)&&(U(!!F,K.message,ee.message,Cn.max,Cn.min),!n))return w(S[y].message),S}if((c||d)&&!P&&(tn(v)||s&&Array.isArray(v))){let F=fi(c),T=fi(d),K=!Mt(F.value)&&v.length>+F.value,ee=!Mt(T.value)&&v.length<+T.value;if((K||ee)&&(U(K,F.message,T.message),!n))return w(S[y].message),S}if(h&&!P&&tn(v)){let{value:F,message:T}=fi(h);if(Bc(F)&&!v.match(F)&&(S[y]={type:Cn.pattern,message:T,ref:i,...L(Cn.pattern,T)},!n))return w(T),S}if(b){if(Oa(b)){let F=await b(v,a),T=_1(F,x);if(T&&(S[y]={...T,...L(Cn.validate,T.message)},!n))return w(T.message),S}else if(Ye(b)){let F={};for(let T in b){if(!Qt(F)&&!n)break;let K=_1(await b[T](v,a),x,T);K&&(F={...K,...L(T,K.message)},w(K.message),n&&(S[y]=F))}if(!Qt(F)&&(S[y]={ref:x,...F},!n))return S}}return w(!0),S},R4={mode:Ma.onSubmit,reValidateMode:Ma.onChange,shouldFocusError:!0};function k4(e={}){let t={...R4,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:Oa(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Ye(t.defaultValues)||Ye(t.values)?ft(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:ft(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},l,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},m={...d},f={array:v1(),state:v1()},h=t.criteriaMode===Ma.all,b=_=>E=>{clearTimeout(c),c=setTimeout(_,E)},y=async _=>{if(!t.disabled&&(d.isValid||m.isValid||_)){let E=t.resolver?Qt((await N()).errors):await P(n,!0);E!==a.isValid&&f.state.next({isValid:E})}},$=(_,E)=>{!t.disabled&&(d.isValidating||d.validatingFields||m.isValidating||m.validatingFields)&&((_||Array.from(o.mount)).forEach(D=>{D&&(E?Be(a.validatingFields,D,E):We(a.validatingFields,D))}),f.state.next({validatingFields:a.validatingFields,isValidating:!Qt(a.validatingFields)}))},g=(_,E=[],D,z,B=!0,O=!0)=>{if(z&&D&&!t.disabled){if(i.action=!0,O&&Array.isArray(Z(n,_))){let Q=D(Z(n,_),z.argA,z.argB);B&&Be(n,_,Q)}if(O&&Array.isArray(Z(a.errors,_))){let Q=D(Z(a.errors,_),z.argA,z.argB);B&&Be(a.errors,_,Q),N4(a.errors,_)}if((d.touchedFields||m.touchedFields)&&O&&Array.isArray(Z(a.touchedFields,_))){let Q=D(Z(a.touchedFields,_),z.argA,z.argB);B&&Be(a.touchedFields,_,Q)}(d.dirtyFields||m.dirtyFields)&&(a.dirtyFields=ll(r,s)),f.state.next({name:_,isDirty:U(_,E),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else Be(s,_,E)},v=(_,E)=>{Be(a.errors,_,E),f.state.next({errors:a.errors})},x=_=>{a.errors=_,f.state.next({errors:a.errors,isValid:!1})},w=(_,E,D,z)=>{let B=Z(n,_);if(B){let O=Z(s,_,Ze(D)?Z(r,_):D);Ze(O)||z&&z.defaultChecked||E?Be(s,_,E?O:x1(B._f)):K(_,O),i.mount&&y()}},S=(_,E,D,z,B)=>{let O=!1,Q=!1,ue={name:_};if(!t.disabled){if(!D||z){(d.isDirty||m.isDirty)&&(Q=a.isDirty,a.isDirty=ue.isDirty=U(),O=Q!==ue.isDirty);let ge=lr(Z(r,_),E);Q=!!Z(a.dirtyFields,_),ge?We(a.dirtyFields,_):Be(a.dirtyFields,_,!0),ue.dirtyFields=a.dirtyFields,O=O||(d.dirtyFields||m.dirtyFields)&&Q!==!ge}if(D){let ge=Z(a.touchedFields,_);ge||(Be(a.touchedFields,_,D),ue.touchedFields=a.touchedFields,O=O||(d.touchedFields||m.touchedFields)&&ge!==D)}O&&B&&f.state.next(ue)}return O?ue:{}},k=(_,E,D,z)=>{let B=Z(a.errors,_),O=(d.isValid||m.isValid)&&en(E)&&a.isValid!==E;if(t.delayError&&D?(l=b(()=>v(_,D)),l(t.delayError)):(clearTimeout(c),l=null,D?Be(a.errors,_,D):We(a.errors,_)),(D?!lr(B,D):B)||!Qt(z)||O){let Q={...z,...O&&en(E)?{isValid:E}:{},errors:a.errors,name:_};a={...a,...Q},f.state.next(Q)}},N=async _=>{$(_,!0);let E=await t.resolver(s,t.context,y4(_||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return $(_),E},C=async _=>{let{errors:E}=await N(_);if(_)for(let D of _){let z=Z(E,D);z?Be(a.errors,D,z):We(a.errors,D)}else a.errors=E;return E},P=async(_,E,D={valid:!0})=>{for(let z in _){let B=_[z];if(B){let{_f:O,...Q}=B;if(O){let ue=o.array.has(O.name),ge=B._f&&b4(B._f);ge&&d.validatingFields&&$([z],!0);let vt=await R1(B,o.disabled,s,h,t.shouldUseNativeValidation&&!E,ue);if(ge&&d.validatingFields&&$([z]),vt[O.name]&&(D.valid=!1,E))break;!E&&(Z(vt,O.name)?ue?_4(a.errors,vt,O.name):Be(a.errors,O.name,vt[O.name]):We(a.errors,O.name))}!Qt(Q)&&await P(Q,E,D)}}return D.valid},L=()=>{for(let _ of o.unMount){let E=Z(n,_);E&&(E._f.refs?E._f.refs.every(D=>!ch(D)):!ch(E._f.ref))&&la(_)}o.unMount=new Set},U=(_,E)=>!t.disabled&&(_&&E&&Be(s,_,E),!lr(Oe(),r)),F=(_,E,D)=>f4(_,o,{...i.mount?s:Ze(E)?r:tn(_)?{[_]:E}:E},D,E),T=_=>fh(Z(i.mount?s:r,_,t.shouldUnregister?Z(r,_,[]):[])),K=(_,E,D={})=>{let z=Z(n,_),B=E;if(z){let O=z._f;O&&(!O.disabled&&Be(s,_,D1(E,O)),B=jc(O.ref)&&Mt(E)?"":E,C1(O.ref)?[...O.ref.options].forEach(Q=>Q.selected=B.includes(Q.value)):O.refs?ml(O.ref)?O.refs.forEach(Q=>{(!Q.defaultChecked||!Q.disabled)&&(Array.isArray(B)?Q.checked=!!B.find(ue=>ue===Q.value):Q.checked=B===Q.value||!!B)}):O.refs.forEach(Q=>Q.checked=Q.value===B):hh(O.ref)?O.ref.value="":(O.ref.value=B,O.ref.type||f.state.next({name:_,values:ft(s)})))}(D.shouldDirty||D.shouldTouch)&&S(_,B,D.shouldTouch,D.shouldDirty,!0),D.shouldValidate&&pt(_)},ee=(_,E,D)=>{for(let z in E){if(!E.hasOwnProperty(z))return;let B=E[z],O=_+"."+z,Q=Z(n,O);(o.array.has(_)||Ye(B)||Q&&!Q._f)&&!Qr(B)?ee(O,B,D):K(O,B,D)}},ne=(_,E,D={})=>{let z=Z(n,_),B=o.array.has(_),O=ft(E);Be(s,_,O),B?(f.array.next({name:_,values:ft(s)}),(d.isDirty||d.dirtyFields||m.isDirty||m.dirtyFields)&&D.shouldDirty&&f.state.next({name:_,dirtyFields:ll(r,s),isDirty:U(_,O)})):z&&!z._f&&!Mt(O)?ee(_,O,D):K(_,O,D),S1(_,o)&&f.state.next({...a,name:_}),f.state.next({name:i.mount?_:void 0,values:ft(s)})},he=async _=>{i.mount=!0;let E=_.target,D=E.name,z=!0,B=Z(n,D),O=ge=>{z=Number.isNaN(ge)||Qr(ge)&&isNaN(ge.getTime())||lr(ge,Z(s,D,ge))},Q=$1(t.mode),ue=$1(t.reValidateMode);if(B){let ge,vt,Ce=E.type?x1(B._f):i4(_),Ct=_.type===h1.BLUR||_.type===h1.FOCUS_OUT,on=!x4(B._f)&&!t.resolver&&!Z(a.errors,D)&&!B._f.deps||S4(Ct,Z(a.touchedFields,D),a.isSubmitted,ue,Q),ja=S1(D,o,Ct);Be(s,D,Ce),Ct?(!E||!E.readOnly)&&(B._f.onBlur&&B._f.onBlur(_),l&&l(0)):B._f.onChange&&B._f.onChange(_);let Fa=S(D,Ce,Ct),vr=!Qt(Fa)||ja;if(!Ct&&f.state.next({name:D,type:_.type,values:ft(s)}),on)return(d.isValid||m.isValid)&&(t.mode==="onBlur"?Ct&&y():Ct||y()),vr&&f.state.next({name:D,...ja?{}:Fa});if(!Ct&&ja&&f.state.next({...a}),t.resolver){let{errors:gr}=await N([D]);if(O(Ce),z){let Zr=N1(a.errors,n,D),es=N1(gr,n,Zr.name||D);ge=es.error,D=es.name,vt=Qt(gr)}}else $([D],!0),ge=(await R1(B,o.disabled,s,h,t.shouldUseNativeValidation))[D],$([D]),O(Ce),z&&(ge?vt=!1:(d.isValid||m.isValid)&&(vt=await P(n,!0)));z&&(B._f.deps&&pt(B._f.deps),k(D,vt,ge,Fa))}},xt=(_,E)=>{if(Z(a.errors,E)&&_.focus)return _.focus(),1},pt=async(_,E={})=>{let D,z,B=cl(_);if(t.resolver){let O=await C(Ze(_)?_:B);D=Qt(O),z=_?!B.some(Q=>Z(O,Q)):D}else _?(z=(await Promise.all(B.map(async O=>{let Q=Z(n,O);return await P(Q&&Q._f?{[O]:Q}:Q)}))).every(Boolean),!(!z&&!a.isValid)&&y()):z=D=await P(n);return f.state.next({...!tn(_)||(d.isValid||m.isValid)&&D!==a.isValid?{}:{name:_},...t.resolver||!_?{isValid:D}:{},errors:a.errors}),E.shouldFocus&&!z&&dl(n,xt,_?B:o.mount),z},Oe=_=>{let E={...i.mount?s:r};return Ze(_)?E:tn(_)?Z(E,_):_.map(D=>Z(E,D))},De=(_,E)=>({invalid:!!Z((E||a).errors,_),isDirty:!!Z((E||a).dirtyFields,_),error:Z((E||a).errors,_),isValidating:!!Z(a.validatingFields,_),isTouched:!!Z((E||a).touchedFields,_)}),at=_=>{_&&cl(_).forEach(E=>We(a.errors,E)),f.state.next({errors:_?a.errors:{}})},$t=(_,E,D)=>{let z=(Z(n,_,{_f:{}})._f||{}).ref,B=Z(a.errors,_)||{},{ref:O,message:Q,type:ue,...ge}=B;Be(a.errors,_,{...ge,...E,ref:z}),f.state.next({name:_,errors:a.errors,isValid:!1}),D&&D.shouldFocus&&z&&z.focus&&z.focus()},Le=(_,E)=>Oa(_)?f.state.subscribe({next:D=>"values"in D&&_(F(void 0,E),D)}):F(_,E,!0),Pa=_=>f.state.subscribe({next:E=>{w4(_.name,E.name,_.exact)&&$4(E,_.formState||d,J,_.reRenderRoot)&&_.callback({values:{...s},...a,...E,defaultValues:r})}}).unsubscribe,kt=_=>(i.mount=!0,m={...m,..._.formState},Pa({..._,formState:m})),la=(_,E={})=>{for(let D of _?cl(_):o.mount)o.mount.delete(D),o.array.delete(D),E.keepValue||(We(n,D),We(s,D)),!E.keepError&&We(a.errors,D),!E.keepDirty&&We(a.dirtyFields,D),!E.keepTouched&&We(a.touchedFields,D),!E.keepIsValidating&&We(a.validatingFields,D),!t.shouldUnregister&&!E.keepDefaultValue&&We(r,D);f.state.next({values:ft(s)}),f.state.next({...a,...E.keepDirty?{isDirty:U()}:{}}),!E.keepIsValid&&y()},rn=({disabled:_,name:E})=>{(en(_)&&i.mount||_||o.disabled.has(E))&&(_?o.disabled.add(E):o.disabled.delete(E))},ua=(_,E={})=>{let D=Z(n,_),z=en(E.disabled)||en(t.disabled);return Be(n,_,{...D||{},_f:{...D&&D._f?D._f:{ref:{name:_}},name:_,mount:!0,...E}}),o.mount.add(_),D?rn({disabled:en(E.disabled)?E.disabled:t.disabled,name:_}):w(_,!0,E.value),{...z?{disabled:E.disabled||t.disabled}:{},...t.progressive?{required:!!E.required,min:ul(E.min),max:ul(E.max),minLength:ul(E.minLength),maxLength:ul(E.maxLength),pattern:ul(E.pattern)}:{},name:_,onChange:he,onBlur:he,ref:B=>{if(B){ua(_,E),D=Z(n,_);let O=Ze(B.value)&&B.querySelectorAll&&B.querySelectorAll("input,select,textarea")[0]||B,Q=h4(O),ue=D._f.refs||[];if(Q?ue.find(ge=>ge===O):O===D._f.ref)return;Be(n,_,{_f:{...D._f,...Q?{refs:[...ue.filter(ch),O,...Array.isArray(Z(r,_))?[{}]:[]],ref:{type:O.type,name:_}}:{ref:O}}}),w(_,!1,void 0,O)}else D=Z(n,_,{}),D._f&&(D._f.mount=!1),(t.shouldUnregister||E.shouldUnregister)&&!(l4(o.array,_)&&i.action)&&o.unMount.add(_)}}},Vt=()=>t.shouldFocusError&&dl(n,xt,o.mount),sn=_=>{en(_)&&(f.state.next({disabled:_}),dl(n,(E,D)=>{let z=Z(n,D);z&&(E.disabled=z._f.disabled||_,Array.isArray(z._f.refs)&&z._f.refs.forEach(B=>{B.disabled=z._f.disabled||_}))},0,!1))},ht=(_,E)=>async D=>{let z;D&&(D.preventDefault&&D.preventDefault(),D.persist&&D.persist());let B=ft(s);if(f.state.next({isSubmitting:!0}),t.resolver){let{errors:O,values:Q}=await N();a.errors=O,B=ft(Q)}else await P(n);if(o.disabled.size)for(let O of o.disabled)We(B,O);if(We(a.errors,"root"),Qt(a.errors)){f.state.next({errors:{}});try{await _(B,D)}catch(O){z=O}}else E&&await E({...a.errors},D),Vt(),setTimeout(Vt);if(f.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:Qt(a.errors)&&!z,submitCount:a.submitCount+1,errors:a.errors}),z)throw z},ca=(_,E={})=>{Z(n,_)&&(Ze(E.defaultValue)?ne(_,ft(Z(r,_))):(ne(_,E.defaultValue),Be(r,_,ft(E.defaultValue))),E.keepTouched||We(a.touchedFields,_),E.keepDirty||(We(a.dirtyFields,_),a.isDirty=E.defaultValue?U(_,ft(Z(r,_))):U()),E.keepError||(We(a.errors,_),d.isValid&&y()),f.state.next({...a}))},_a=(_,E={})=>{let D=_?ft(_):r,z=ft(D),B=Qt(_),O=B?r:z;if(E.keepDefaultValues||(r=D),!E.keepValues){if(E.keepDirtyValues){let Q=new Set([...o.mount,...Object.keys(ll(r,s))]);for(let ue of Array.from(Q))Z(a.dirtyFields,ue)?Be(O,ue,Z(s,ue)):ne(ue,Z(O,ue))}else{if(mh&&Ze(_))for(let Q of o.mount){let ue=Z(n,Q);if(ue&&ue._f){let ge=Array.isArray(ue._f.refs)?ue._f.refs[0]:ue._f.ref;if(jc(ge)){let vt=ge.closest("form");if(vt){vt.reset();break}}}}if(E.keepFieldsRef)for(let Q of o.mount)ne(Q,Z(O,Q));else n={}}s=t.shouldUnregister?E.keepDefaultValues?ft(r):{}:ft(O),f.array.next({values:{...O}}),f.state.next({values:{...O}})}o={mount:E.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!E.keepIsValid||!!E.keepDirtyValues,i.watch=!!t.shouldUnregister,f.state.next({submitCount:E.keepSubmitCount?a.submitCount:0,isDirty:B?!1:E.keepDirty?a.isDirty:!!(E.keepDefaultValues&&!lr(_,r)),isSubmitted:E.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:B?{}:E.keepDirtyValues?E.keepDefaultValues&&s?ll(r,s):a.dirtyFields:E.keepDefaultValues&&_?ll(r,_):E.keepDirty?a.dirtyFields:{},touchedFields:E.keepTouched?a.touchedFields:{},errors:E.keepErrors?a.errors:{},isSubmitSuccessful:E.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},da=(_,E)=>_a(Oa(_)?_(s):_,E),Ua=(_,E={})=>{let D=Z(n,_),z=D&&D._f;if(z){let B=z.refs?z.refs[0]:z.ref;B.focus&&(B.focus(),E.shouldSelect&&Oa(B.select)&&B.select())}},J=_=>{a={...a,..._}},ie={control:{register:ua,unregister:la,getFieldState:De,handleSubmit:ht,setError:$t,_subscribe:Pa,_runSchema:N,_focusError:Vt,_getWatch:F,_getDirty:U,_setValid:y,_setFieldArray:g,_setDisabledField:rn,_setErrors:x,_getFieldArray:T,_reset:_a,_resetDefaultValues:()=>Oa(t.defaultValues)&&t.defaultValues().then(_=>{da(_,t.resetOptions),f.state.next({isLoading:!1})}),_removeUnmounted:L,_disableForm:sn,_subjects:f,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(_){i=_},get _defaultValues(){return r},get _names(){return o},set _names(_){o=_},get _formState(){return a},get _options(){return t},set _options(_){t={...t,..._}}},subscribe:kt,trigger:pt,register:ua,handleSubmit:ht,watch:Le,setValue:ne,getValues:Oe,reset:da,resetField:ca,clearErrors:at,unregister:la,setError:$t,setFocus:Ua,getFieldState:De};return{...ie,formControl:ie}}function O1(e={}){let t=Kt.default.useRef(void 0),a=Kt.default.useRef(void 0),[n,r]=Kt.default.useState({isDirty:!1,isValidating:!1,isLoading:Oa(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:Oa(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!Oa(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=k4(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,m4(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),Kt.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),Kt.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),Kt.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),Kt.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),Kt.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),Kt.default.useEffect(()=>{e.values&&!lr(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),Kt.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=d4(n,s),t.current}var L1={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},P1={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},C4={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function ae({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return u`
    <${s}
      className=${Y(L1[a]??L1.default,P1[n]??P1.md,C4[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var gh="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",qc={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function Ot({className:e="",size:t="md",error:a=!1,...n}){return u`
    <input
      className=${Y(gh,qc[t]??qc.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Ic({className:e="",error:t=!1,rows:a=4,...n}){return u`
    <textarea
      rows=${a}
      className=${Y(gh,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function yh({children:e,className:t="",size:a="md",error:n=!1,...r}){return u`
    <div className="relative w-full">
      <select
        className=${Y(gh,qc[a]??qc.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
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
      className=${Y("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&u`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function En({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return u`
    <div className=${Y("flex flex-col gap-2",s)}>
      ${e&&u`<${E4} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&u`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&u`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var T4={google:"Google",github:"GitHub",apple:"Apple"};function A4(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function U1({providers:e,redirectAfter:t}){let a=R();return e.length?u`
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
  `:null}var D4=["google","github","apple"];function j1(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>{let a=!1;return g$().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(D4.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function F1({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=R(),{theme:s,toggleTheme:i}=Ec(),o=j1(),{formState:{errors:l,isSubmitting:c},handleSubmit:d,register:m}=O1({defaultValues:{token:e||""}});return u`
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

        <${U1}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var B1={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},z1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function q({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return u`
    <span
      className=${Y("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",z1[n]??z1.md,B1[e]??B1.muted,r)}
    >
      ${a&&u`<span
          className=${Y("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var M4=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,q1=/(bash|shell|exec|run|command|terminal|spawn|process)/,I1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function H1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return M4.test(n)?{tone:"danger",key:"tool.riskWrite"}:q1.test(n)?{tone:"warning",key:"tool.riskExec"}:I1.test(n)?{tone:"info",key:"tool.riskNetwork"}:q1.test(r)?{tone:"warning",key:"tool.riskExec"}:I1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}var Hc=480;function O4(e,t){return t&&t.length>0?t.some(a=>typeof a?.value=="string"&&a.value.length>Hc):typeof e=="string"&&e.length>Hc}function K1(e,t){return typeof e!="string"||t||e.length<=Hc?e:`${e.slice(0,Hc).trimEnd()}
...`}function Q1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=R(),{toolName:s,description:i,parameters:o,allowAlways:l,approvalDetails:c=[]}=e,[d,m]=p.default.useState(!1),[f,h]=p.default.useState(!1),[b,y]=p.default.useState(!1),$=p.default.useRef(!1),g=p.default.useRef(e);g.current=e,p.default.useEffect(()=>{h(!1),$.current=!1,y(!1)},[e]);let v=p.default.useMemo(()=>H1(s,i,o),[s,i,o]),x=s||r("approval.thisTool"),w=O4(o,c),S=f?"max-h-72":"max-h-36",k=p.default.useCallback(async C=>{if($.current)return;let P=g.current;$.current=!0,y(!0);try{await C?.()}finally{g.current===P&&($.current=!1,y(!1))}},[]),N=p.default.useCallback(()=>{k(d&&l?n:t)},[d,l,n,t,k]);return u`
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
              ${c.map(C=>u`
                  <div className="grid gap-1 border-b border-iron-800/70 px-3 py-2 last:border-b-0 sm:grid-cols-[7rem_1fr]">
                    <dt className="font-medium text-iron-400">${C.label}</dt>
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${K1(C.value,f)}</dd>
                  </div>
                `)}
            </dl>
          `:o&&u`<pre className=${`mb-2 ${S} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${K1(o,f)}</pre>`}

      ${w&&u`
        <${A}
          variant="ghost"
          size="sm"
          className="mb-3 px-0 text-[var(--v2-accent)] hover:bg-transparent"
          onClick=${()=>h(C=>!C)}
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
            onChange=${C=>m(C.currentTarget.checked)}
            disabled=${b}
            className="h-3.5 w-3.5 accent-[var(--v2-accent)]"
          />
          ${r("approval.alwaysAllowToolLabel",{tool:x})}
        </label>
      `}

      <div className="flex flex-wrap gap-2">
        <${A} variant="primary" onClick=${N} disabled=${b}>
          ${r(d&&l?"approval.approveAndAlways":"approval.approve")}
        <//>
        <${A}
          variant="secondary"
          onClick=${()=>k(a)}
          disabled=${b}
        >
          ${r("approval.deny")}
        <//>
      </div>
    </div>
  `}function pi({icon:e="lock",headline:t,provider:a,accountLabel:n,body:r,expiresAt:s,pillHint:i,defaultExpanded:o=!0,testId:l="auth-gate",challengeKind:c="",children:d}){let m=R(),[f,h]=p.default.useState(o),b=p.default.useId(),y=n||a||"";return u`
    <div
      data-testid=${l}
      data-auth-challenge=${c||void 0}
      className="mx-auto w-full max-w-lg rounded-xl border border-[rgba(76,167,230,0.34)] bg-[rgba(76,167,230,0.08)]"
    >
      <button
        type="button"
        onClick=${()=>h($=>!$)}
        aria-expanded=${f?"true":"false"}
        aria-controls=${b}
        className="flex w-full items-center gap-3 rounded-xl border-0 bg-transparent px-4 py-3 text-left"
      >
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-md border border-[rgba(76,167,230,0.28)] bg-[rgba(76,167,230,0.1)] text-[#8fc8f2]">
          <${M} name=${e} className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold text-white">
            ${t||m("authGate.title")}
          </span>
          ${y&&u`<span className="block truncate text-xs text-iron-300">${y}</span>`}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 text-xs font-medium text-[#8fc8f2]">
          ${i&&u`<span className="hidden sm:inline">${i}</span>`}
          <${M}
            name="chevron"
            className=${["h-4 w-4",f?"rotate-180":""].join(" ")}
          />
        </span>
      </button>

      ${f&&u`
        <div
          id=${b}
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
  `}function V1({gate:e,onCancel:t}){let a=R();return u`
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
          <${A} type="button" variant="secondary" onClick=${()=>t?.()}>
            ${a("authGate.cancel")}
          <//>
        </div>
      </form>
    <//>
  `}function G1({gate:e,onCancel:t}){let a=R(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),o=p.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]);p.default.useEffect(()=>{i("")},[e.authorizationUrl,e.gateRef,e.runId]);let l=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),c=p.default.useCallback(()=>{if(!o){i(a("authGate.serviceUnavailable"));return}i(""),window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0)},[e.authorizationUrl,o]),d=n?a("authGate.reopenAuthorization",{provider:l}):a("authGate.openAuthorization",{provider:l});return u`
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
  `}function Y1({gate:e,onSubmit:t,onCancel:a}){let n=R(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState(!1),d=p.default.useCallback(async m=>{m.preventDefault();let f=r.trim();if(!f){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(f),s("")}catch(h){o(h?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return u`
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
  `}var L4="/api/webchat/v2/extensions/pairing/redeem";function J1({channel:e,action:t}){let a=R(),n=W(),[r,s]=p.default.useState(""),i=U4(t,a),o=G({mutationFn:({code:c})=>P4(e,c),onSuccess:()=>{s(""),n.invalidateQueries({queryKey:["extensions"]}),n.invalidateQueries({queryKey:["connectable-channels"]}),n.invalidateQueries({queryKey:["pairing",e]})}}),l=()=>{if(o.isPending)return;let c=r.trim().toUpperCase();c&&o.mutate({code:c})};return u`
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
        <${A}
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
        ${j4(o.error,i.errorMessage)}
      </p>`}
    </div>
  `}function P4(e,t){return V(L4,{method:"POST",body:JSON.stringify({channel:e,code:t})}).then(a=>({...a,success:!0}))}function U4(e,t){return{title:e?.title||t("pairing.title"),instructions:e?.instructions||t("pairing.instructions"),placeholder:e?.input_placeholder||e?.code_placeholder||t("pairing.placeholder"),submitLabel:e?.submit_label||t("pairing.approve"),successMessage:e?.success_message||t("pairing.success"),errorMessage:e?.error_message||t("pairing.error")}}function j4(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var F4="/api/webchat/v2/extensions/pairing/redeem";function X1(e){return V(F4,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function Kc({action:e}){let t=R(),a=W(),n=G({mutationFn:({code:l})=>X1(l),onSuccess:()=>{s(""),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=p.default.useState(""),i=B4(e,t),o=()=>{if(n.isPending)return;let l=r.trim().toUpperCase();l&&n.mutate({code:l})};return u`
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

      ${n.isSuccess&&u`<p data-testid="slack-pairing-success" className="text-xs text-emerald-300">
        ${n.data?.message||i.successMessage}
      </p>`}
      ${n.isError&&u`<p data-testid="slack-pairing-error" className="text-xs text-red-300">
        ${z4(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function B4(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function z4(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function q4(e,t){return e?.channel==="slack"&&e.strategy===t}function W1({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return u`
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
            <${M} name="close" className="h-4 w-4" />
          </button>
        `}
      </div>

      ${q4(e,"inbound_proof_code")?u`<${Kc} action=${e.action} />`:e.strategy==="inbound_proof_code"?u`
              <${J1}
                channel=${a}
                action=${e.action}
              />
            `:u`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function I4(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):Ir.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:Ir.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:Ir.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:Ir.maxTotalBytes}:Ir}function Z1(){let e=Sa(),t=H({enabled:!!e,queryKey:["session"],queryFn:$c,staleTime:5*6e4});return I4(t.data)}function Qc({onSend:e,onCancel:t,disabled:a,sendDisabled:n=a,canCancel:r=!1,initialText:s="",resetKey:i="",draftKey:o=nl,variant:l="dock",context:c={},statusText:d=""}){let m=R(),f=mt(),h=l==="hero",b=Z1(),[y,$]=p.default.useState(()=>Zp(o)),[g,v]=p.default.useState(()=>th(o)),[x,w]=p.default.useState(""),[S,k]=p.default.useState(!1),[N,C]=p.default.useState(!1),[P,L]=p.default.useState(!1),U=p.default.useRef(null),F=p.default.useRef(null),T=p.default.useRef(!1),K=a||n||S,ee=p.default.useRef(a||n);ee.current=a||n,T.current=K;let ne=p.default.useRef([]),he=p.default.useRef(Promise.resolve()),xt=p.default.useRef({draftKey:o,storageScope:f});xt.current={draftKey:o,storageScope:f},p.default.useEffect(()=>{ne.current=g},[g]);let pt=p.default.useRef(null),Oe=p.default.useRef(null),De=p.default.useCallback(()=>{Oe.current&&(window.clearTimeout(Oe.current),Oe.current=null);let O=pt.current;pt.current=null,O&&O.scope===mt()&&eh(O.key,O.text)},[]),at=p.default.useCallback(()=>{Oe.current&&(window.clearTimeout(Oe.current),Oe.current=null),pt.current=null},[]),$t=p.default.useCallback(()=>{let O=U.current;O&&(O.style.height="auto",O.style.height=`${Math.min(O.scrollHeight,200)}px`)},[]);p.default.useEffect(()=>{$t()},[y,$t]),p.default.useEffect(()=>($(Zp(o)),()=>De()),[o,f,De]);let Le=p.default.useRef(o),Pa=p.default.useRef(f);p.default.useEffect(()=>{if(Le.current!==o||Pa.current!==f){Le.current=o,Pa.current=f,v(th(o)),w("");return}kc(o,g)},[o,f,g]),p.default.useEffect(()=>{s&&($(s),window.requestAnimationFrame(()=>{U.current&&(U.current.focus(),U.current.setSelectionRange(s.length,s.length))}))},[s,i]);let kt=p.default.useCallback(O=>{if(a||!O||O.length===0)return;let Q=o,ue=f;he.current=he.current.then(async()=>{let ge=o,vt=f,{staged:Ce,errors:Ct}=await A$(O,{limits:b,existing:ne.current,t:m}),on=xt.current;if(!(on.draftKey!==ge||on.storageScope!==vt||mt()!==vt)){if(Ce.length>0){let ja=[...ne.current,...Ce];ne.current=ja,kc(ge,ja),v(ja)}w(Ct.length>0?Ct.join(" "):"")}}).catch(()=>{w(m("chat.attachmentStagingFailed"))})},[a,o,b,f,m]),la=p.default.useCallback(O=>{let Q=ne.current.filter(ue=>ue.id!==O);ne.current=Q,kc(o,Q),v(Q),w("")},[o]),rn=p.default.useCallback(()=>{a||F.current?.click()},[a]),ua=p.default.useCallback(O=>{let Q=Array.from(O.target.files||[]);kt(Q),O.target.value=""},[kt]),Vt=p.default.useCallback(async()=>{let O=y.trim(),Q=g.length>0,ue=O||(Q?Rc:"");if(!(!ue||T.current)){T.current=!0,k(!0);try{if(await e(ue,{attachments:g,displayContent:O})===null)return;$(""),v([]),ne.current=[],w(""),at(),H$(o),K$(o),U.current&&(U.current.style.height="auto")}catch{}finally{T.current=ee.current,k(!1)}}},[y,g,e,o,at,a,n]),sn=p.default.useCallback(O=>{let Q=O.target.value;$(Q),pt.current={key:o,text:Q,scope:mt()},Oe.current&&window.clearTimeout(Oe.current),Oe.current=window.setTimeout(De,300)},[o,De]),ht=p.default.useCallback(async()=>{if(!(!r||N||!t)){C(!0);try{await t()}finally{C(!1)}}},[r,N,t]),ca=p.default.useCallback(O=>{if(O.key==="Enter"&&!O.shiftKey){if(O.preventDefault(),U.current?.dataset?.sendDisabled==="true"||T.current)return;Vt()}},[Vt]),_a=p.default.useCallback(O=>{let Q=Array.from(O.clipboardData?.files||[]);Q.length>0&&(O.preventDefault(),kt(Q))},[kt]),da=p.default.useCallback(O=>{O.preventDefault(),L(!1);let Q=Array.from(O.dataTransfer?.files||[]);Q.length>0&&kt(Q)},[kt]),Ua=p.default.useCallback(O=>{O.preventDefault(),!a&&L(!0)},[a]),J=p.default.useCallback(O=>{O.currentTarget.contains(O.relatedTarget)||L(!1)},[]),re=y.trim()||g.length>0,ie=a||n,_=m(h?"chat.heroPlaceholder":"chat.followUpPlaceholder"),E=b.accept.length>0?b.accept.join(","):void 0,D=h?"w-full":"px-4 py-3 sm:px-5 lg:px-8",z=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",h?"min-h-[120px]":"",a?"opacity-70":""].join(" "),B=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",h?"min-h-[72px]":"min-h-[40px]"].join(" ");return u`
    <div className=${D}>
      <div
        className=${z}
        onDrop=${da}
        onDragOver=${Ua}
        onDragLeave=${J}
      >
        ${P&&u`
          <div className="pointer-events-none absolute inset-1 z-10 flex items-center justify-center rounded-[16px] border border-dashed border-[color-mix(in_srgb,var(--v2-accent)_55%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-canvas)_82%,transparent)] text-sm font-medium text-[var(--v2-accent-text)]">
            ${m("chat.attachmentDropHint")}
          </div>
        `}
        ${x&&u`
          <div
            role="alert"
            className="mb-3 flex items-start gap-2 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-2 text-xs leading-5 text-[var(--v2-danger-text)]"
          >
            <span className="min-w-0 flex-1">${x}</span>
            <button
              type="button"
              onClick=${()=>w("")}
              aria-label=${m("common.dismiss")}
              title=${m("common.dismiss")}
              className="-mr-1 -mt-0.5 shrink-0 rounded p-0.5 text-[color-mix(in_srgb,var(--v2-danger-text)_80%,transparent)] transition hover:bg-[color-mix(in_srgb,var(--v2-danger-text)_14%,transparent)] hover:text-[var(--v2-danger-text)]"
            >
              <${M} name="close" className="h-3.5 w-3.5" strokeWidth=${2} />
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
                        <${M} name="file" className="h-4 w-4" />
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
                    <${M} name="close" className="h-3 w-3" />
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
          placeholder=${_}
          rows=${1}
          disabled=${a}
          className=${B}
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
              <${M} name="plus" className="h-5 w-5" />
            </button>
            ${r?u`
                <${A}
                  type="button"
                  variant="danger"
                  size="icon-sm"
                  onClick=${ht}
                  disabled=${N}
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
                  onClick=${Vt}
                  disabled=${ie||S||!re}
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
  `}var eS={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function tS({status:e}){let t=R();if(e==="idle"||e==="connecting"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return u`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",eS[e]||eS.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function aS({onSuggestion:e,onSend:t,disabled:a,sendDisabled:n,initialText:r,resetKey:s,draftKey:i,context:o,statusText:l,canCancel:c,onCancel:d}){let m=R(),f=[{icon:"tool",title:m("chat.suggestion1"),detail:m("chat.suggestion1Desc")},{icon:"shield",title:m("chat.suggestion2"),detail:m("chat.suggestion2Desc")},{icon:"plug",title:m("chat.suggestion3"),detail:m("chat.suggestion3Desc")}];return u`
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
        <${Qc}
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
  `}var H4=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function nS({open:e,onClose:t}){let a=R();return e?u`
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
          ${H4.map((n,r)=>u`
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
  `:null}function sS(e){let t=0,a=0,n=0,r=0,s=0;for(let o of e){if(o.role==="thinking"&&(t+=1),o.role==="tool_activity"){let l=rS([o]);a+=l.tools,n+=l.failed,r+=l.declined,s+=l.running}if(K4(o)){let l=rS(o.toolCalls);a+=l.tools,n+=l.failed,r+=l.declined,s+=l.running}}let i=[];return t&&i.push(`${t} reasoning`),a&&i.push(`${a} ${a===1?"tool":"tools"}`),n&&i.push(`${n} failed`),r&&i.push(`${r} declined`),!n&&!r&&s&&i.push("running"),{hasError:n>0,hasDeclined:r>0,label:`Activity${i.length?` - ${i.join(", ")}`:""}`}}function rS(e){let t=0,a=0,n=0;for(let r of e)r.toolStatus==="error"&&(t+=1),r.toolStatus==="declined"&&(a+=1),r.toolStatus==="running"&&(n+=1);return{tools:e.length,failed:t,declined:a,running:n}}function K4(e){return e.toolCalls&&e.toolCalls.length>0}var iS=!1;function Q4(){iS||!window.DOMPurify||(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),iS=!0)}function oS(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}Q4();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var bh=360;function V4(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let l=s("Copy");if(l.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),l.textContent="Copied",di("Code copied",{tone:"success"}),setTimeout(()=>l.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(l),n.appendChild(r),t.scrollHeight>bh){t.style.maxHeight=`${bh}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${bh}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function G4({content:e,className:t=""}){let a=p.default.useRef(null),n=p.default.useMemo(()=>oS(e),[e]);return p.default.useEffect(()=>{V4(a.current)},[n]),u`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var sa=p.default.memo(G4);var lS={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",declined:"bg-iron-400",error:"bg-[var(--v2-danger-text)]"},Y4={success:"ok",declined:"declined",error:"err",running:"run"},J4=2;function hi({activity:e}){return e.toolCalls&&e.toolCalls.length>0?u`<${W4} tools=${e.toolCalls} />`:u`<${Z4} activity=${e} />`}function X4(e,t){let a=0,n=0,r=0,s=0;for(let l of t){let c=String(l.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function W4({tools:e}){let t=R(),a=e.some(o=>o.toolStatus==="error"),n=e.some(o=>o.toolStatus==="error"||o.toolStatus==="declined"),[r,s]=p.default.useState(n);if(p.default.useEffect(()=>{n&&s(!0)},[n]),e.length<=J4)return u`
      <div className="flex flex-col gap-3">
        ${e.map((o,l)=>u`<${hi}
            key=${o.id||o.callId||`${o.toolName}-${l}`}
            activity=${o}
          />`)}
      </div>
    `;let i=X4(t,e);return u`
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
          ${e.map((o,l)=>u`<${hi}
              key=${o.id||o.callId||`${o.toolName}-${l}`}
              activity=${o}
            />`)}
        </div>
      `}
    </div>
  `}function Z4({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:l}=e,[c,d]=p.default.useState(n==="error"||n==="declined");p.default.useEffect(()=>{(n==="error"||n==="declined")&&d(!0)},[n]);let m=lS[n]||lS.running,f=i!=null,h=p.default.useId(),b=u`
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
        >${Y4[n]||"run"}</span
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
          <${M} name="tool" className="h-4 w-4" />
        </div>
      `}
      <div className=${t?"min-w-0 flex-1":"min-w-0 max-w-[85%] flex-1"}>
        ${b}
        ${c&&u`<${e5}
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
  `}function e5({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=R(),l=p.default.useMemo(()=>{let f=[];return r&&f.push({id:s==="declined"?"declined":"error",label:o(s==="declined"?"tool.tabDeclined":"tool.tabError")}),t&&f.push({id:"details",label:o("tool.tabDetails")}),a&&f.push({id:"params",label:o("tool.tabParameters")}),n&&f.push({id:"result",label:o("tool.tabResult")}),f},[o,r,t,a,n,s]),[c,d]=p.default.useState(null),m=c&&l.some(f=>f.id===c)?c:l[0]?.id;return p.default.useEffect(()=>{r&&d(s==="declined"?"declined":"error")},[r,s]),l.length===0?u`
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
        ${m==="result"&&u`<${t5} text=${n} />`}
        ${(m==="error"||m==="declined")&&u`<pre
          className=${["overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono",m==="declined"?"text-iron-300":"text-[var(--v2-danger-text)]"].join(" ")}
        >${r}</pre>`}
      </div>
    </div>
  `}function t5({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return u`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(a5)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return u`
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
                  >${n5(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?u`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:u`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function a5(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function n5(e){return e==null?"":String(e)}function uS({activity:e}){let t=sS(e),a=i5(e),[n,r]=p.default.useState(a);return p.default.useEffect(()=>{a&&r(!0)},[a]),u`
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

      ${n&&u`
        <div className="mt-2 flex flex-col gap-3" data-testid="activity-run-items">
          ${e.map((s,i)=>u`
            <${r5}
              key=${s.id||`${s.role||"activity"}-${i}`}
              item=${s}
            />
          `)}
        </div>
      `}
    </div>
  `}function r5({item:e}){if(e.role==="thinking")return u`<${s5} content=${e.content} />`;if(e.role==="tool_activity"||xh(e)){let t=xh(e)?{id:e.id,toolCalls:e.toolCalls}:e;return u`<${hi} activity=${t} />`}return null}function s5({content:e}){return e?u`
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
  `:null}function xh(e){return e?.toolCalls&&e.toolCalls.length>0}function i5(e){return(e||[]).some(t=>t?.role==="thinking"||t?.toolStatus==="running"||t?.toolStatus==="error"||t?.toolStatus==="declined"?!0:xh(t)?t.toolCalls.some(a=>a?.toolStatus==="running"||a?.toolStatus==="error"||a?.toolStatus==="declined"):!1)}function Vc(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function o5({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=p.default.useState(()=>t&&e.preview_url||null);return p.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return Nc(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?u`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:u`<${M} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var cS="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",dS="px-3 py-2";function Gc({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await Aa(e.fetch_url);Vc(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),l=u`
    <${o5} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?u`<div
      className=${`${cS} ${dS} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${l}
    </div>`:u`<div className=${`${cS} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${dS} text-left transition-colors hover:bg-iron-900/80`}
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
  </div>`}var mS={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function vi({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return p.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),p.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?u`
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
        className=${Y("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",mS[n]??mS.md,r)}
      >
        ${a?u`<${$h} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function $h({children:e,onClose:t,className:a=""}){return u`
    <div
      className=${Y("flex shrink-0 items-center justify-between gap-4","px-5 py-4 md:px-7 md:py-5","border-b border-[var(--v2-panel-border)]",a)}
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
  `}function gi({children:e,className:t=""}){return u`
    <div className=${Y("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function yi({children:e,className:t=""}){return u`
    <div
      className=${Y("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var fS=1e5;function Yc({attachment:e,onClose:t}){let a=!!e,[n,r]=p.default.useState("loading"),[s,i]=p.default.useState({}),o=e?T$(e.mime_type):"download";if(p.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return Aa(e.fetch_url).then(async m=>{d=URL.createObjectURL(m);let f={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")f.dataUrl=await Hp(m);else if(o==="pdf")f.frameUrl=d;else if(o==="text"){let h=await m.text();f.truncated=h.length>fS,f.text=f.truncated?h.slice(0,fS):h}if(c){URL.revokeObjectURL(d);return}i(f),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let l=e.filename||"attachment";return u`
    <${vi} open=${a} onClose=${t} size="xl">
      <${$h} onClose=${t}>
        <span className="block truncate">${l}</span>
      <//>
      <${gi} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&u`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&u`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&u`<${l5} mode=${o} view=${s} filename=${l} />`}
      <//>
      <${yi}>
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
  `}function l5({mode:e,view:t,filename:a}){switch(e){case"image":return u`<img
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
      </div>`}}var u5=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function c5(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function pS(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of c5(e).matchAll(u5)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function hS(e){return e.split("/").filter(Boolean).pop()||e}function vS(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function d5({threadId:e,path:t,onPreview:a}){let[n,r]=p.default.useState({mime_type:"",size_label:""});p.default.useEffect(()=>{let i=!0;return W0({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:vS(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:hS(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:Sc({threadId:e,path:t})};return u`<${Gc}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function gS({threadId:e,content:t}){let a=p.default.useMemo(()=>pS(t),[t]),[n,r]=p.default.useState(null);return!e||a.length===0?null:u`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>u`<${d5}
          key=${s}
          threadId=${e}
          path=${s}
          onPreview=${r}
        />`)}
      <${Yc}
        attachment=${n}
        onClose=${()=>r(null)}
      />
    </div>
  `}var yS={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function m5(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function f5({content:e}){let[t,a]=p.default.useState(!1);return e?u`
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
  `:null}function p5({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:l,status:c,error:d,toolCalls:m,timestamp:f}=e,h=n==="user",[b,y]=p.default.useState(!1),[$,g]=p.default.useState(null),v=p.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),y(!0),di("Copied to clipboard",{tone:"success"}),setTimeout(()=>y(!1),1400)}catch{}},[r]);if(n==="tool_activity"||m&&m.length>0){let L=m&&m.length>0?{id:e.id,toolCalls:m}:e;return u`<${hi} activity=${L} />`}if(n==="thinking")return u`<${f5} content=${r} />`;if(n==="image")return u`
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
    `;let x=m5(f),w=n==="user"||n==="assistant"&&!l,S=n==="system"||n==="error",k=h?"max-w-[85%]":S?"mx-auto max-w-[85%]":"w-full max-w-[85%]",N=h?"":"w-full min-w-0 max-w-full",C=c==="error"&&t,P=w||C||x;return u`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",h?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col",k].join(" ")}>
        <div
          className=${["text-base leading-7",N,yS[n]||yS.assistant,l?"opacity-70":""].join(" ")}
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
              ${i.map((L,U)=>u`<${Gc}
                key=${L.id||U}
                att=${L}
                onPreview=${g}
              />`)}
            </div>
            <${Yc}
              attachment=${$}
              onClose=${()=>g(null)}
            />
          `}

          ${n==="assistant"&&u`<${gS}
            threadId=${a}
            content=${typeof r=="string"?r:""}
          />`}
        </div>
      </div>

      ${P&&u`
        <div
          className=${["mt-1 flex min-h-7 w-max max-w-[85%] flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100",h?"self-end justify-end":S?"self-center justify-center":"self-start justify-start"].join(" ")}
        >
          ${x&&u`<time dateTime=${f} className="shrink-0 font-mono text-[11px] text-iron-500">${x}</time>`}
          ${(w||C)&&u`
            <div className="flex shrink-0 items-center gap-1">
            ${w&&u`
              <button
                type="button"
                onClick=${v}
                title=${b?"Copied":"Copy message"}
                aria-label=${b?"Copied":"Copy message"}
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 hover:text-iron-100"
              >
                <${M} name=${b?"check":"copy"} className="h-3.5 w-3.5" />
              </button>
            `}
            ${C&&u`
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
  `}var bS=p.default.memo(p5);function _S(e){let t=h5(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(RS(r)){let s=xS(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){$S(a,s),wS(a,r),n+=s.length;continue}}if(wh(r)){let s=xS(t,n);$S(a,s),n+=s.length-1;continue}wS(a,r)}return a}function h5(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=Jc(i);o&&RS(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!wh(i))continue;let o=Jc(i),l=o?t.get(o):void 0;if(l===void 0||l>=s)continue;let c=a.get(l)||[];c.push(i),a.set(l,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function xS(e,t){let a=t,n=Jc(e[t]);for(;a<e.length&&wh(e[a])&&v5(n,e[a]);)a+=1;return e.slice(t,a)}function v5(e,t){let a=Jc(t);return!e||!a||a===e}function $S(e,t){if(t.length===0)return;let a=g5(t);e.push({type:"activity-run",id:`activity-run-${a[0].id}`,activity:a})}function wS(e,t){e.push({type:"message",id:t.id,message:t})}function RS(e){return e.role==="assistant"&&!kS(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function wh(e){return e.role==="thinking"||e.role==="tool_activity"||kS(e)}function kS(e){return e?.toolCalls&&e.toolCalls.length>0}function Jc(e){return e?.turnRunId||null}function g5(e){return[...e].sort((t,a)=>t?.role!=="tool_activity"||a?.role!=="tool_activity"?0:y5(t,a))}function y5(e,t){if(Number.isFinite(e.activityOrder)&&Number.isFinite(t.activityOrder)){let n=e.activityOrder-t.activityOrder;if(n!==0)return n}let a=SS(NS(e.updatedAt||e.timestamp),NS(t.updatedAt||t.timestamp));return a!==0?a:SS(e.sequence,t.sequence)}function SS(e,t){let a=Number.isFinite(e)?e:null,n=Number.isFinite(t)?t:null;return a===null&&n===null?0:a===null?1:n===null?-1:a-n}function NS(e){if(!e)return null;let t=Date.parse(e);return Number.isFinite(t)?t:null}var b5=100,x5=100;function $5(e){return e?e.scrollHeight-e.scrollTop-e.clientHeight:Number.POSITIVE_INFINITY}function CS(e,t=b5){return $5(e)<=t}function ES(e){e&&(e.scrollTop=Math.max(0,e.scrollHeight-e.clientHeight))}function TS(e){return e?.id?`${e.role||""}:${e.id}`:null}function w5(e,t){let a=TS(t);return!!(a&&t?.role==="user"&&a!==e)}function S5(){return typeof window>"u"||!window.getSelection?"":String(window.getSelection()?.toString?.()||"")}function AS({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let l=R(),c=p.default.useRef(null),d=p.default.useRef(null),m=p.default.useRef(!0),f=p.default.useRef(null),h=p.default.useRef(null),b=p.default.useRef(null),y=p.default.useRef(0),$=p.default.useRef(!1),[g,v]=p.default.useState(!0),x=p.default.useCallback(()=>{h.current!==null&&(window.cancelAnimationFrame(h.current),h.current=null)},[]),w=p.default.useCallback((T=!1)=>{c.current&&(T&&(m.current=!0,$.current=!1),m.current&&(x(),h.current=window.requestAnimationFrame(()=>{h.current=null;let ee=c.current;!ee||!T&&!m.current||(ES(ee),y.current=ee.scrollTop,$.current=!1,v(!0))})))},[x]),S=p.default.useCallback(()=>{b.current!==null&&(window.cancelAnimationFrame(b.current),b.current=null)},[]);p.default.useLayoutEffect(()=>{let T=e.length>0?e[e.length-1]:null,K=TS(T),ee=w5(f.current,T);return f.current=K,w(ee),x},[e,i,w,x]),p.default.useLayoutEffect(()=>{let T=d.current;if(!T||typeof ResizeObserver!="function")return;let K=new ResizeObserver(()=>{w()});return K.observe(T),()=>{K.disconnect(),x()}},[w,x]);let k=p.default.useCallback(()=>{b.current=null;let T=c.current;if(!T)return;let K=CS(T);y.current=T.scrollTop,K?(m.current=!0,$.current=!1,v(!0)):$.current?(m.current=!1,v(!1)):(m.current=!0,v(!0),w()),a&&T.scrollTop<x5&&n&&!t&&n()},[a,n,t,w]),N=p.default.useCallback(()=>{$.current=!0},[]),C=p.default.useCallback(T=>{let K=c.current;if(!K||typeof T?.clientX!="number")return;let ee=K.offsetWidth-K.clientWidth;if(ee<=0)return;let ne=K.getBoundingClientRect().right;T.clientX>=ne-ee-2&&($.current=!0)},[]),P=p.default.useCallback(()=>{let T=c.current;if(!T)return;let K=CS(T),ee=T.scrollTop<y.current;y.current=T.scrollTop,!K&&ee&&($.current=!0),K?(m.current=!0,$.current=!1):$.current?(m.current=!1,x()):m.current=!0,b.current===null&&(b.current=window.requestAnimationFrame(k))},[x,k]),L=p.default.useCallback(()=>{let T=c.current;T&&(ES(T),y.current=T.scrollTop,m.current=!0,$.current=!1,v(!0))},[]),U=p.default.useCallback(T=>{let K=S5();!K||!T.clipboardData||(T.preventDefault(),T.clipboardData.clearData(),T.clipboardData.setData("text/plain",K))},[]);p.default.useEffect(()=>S,[S]);let F=p.default.useMemo(()=>_S(e),[e]);return u`
    <div className="relative flex min-h-0 min-w-0 flex-1">
    <div
      ref=${c}
      onScroll=${P}
      onWheel=${N}
      onTouchMove=${N}
      onPointerDown=${C}
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
        ${F.map(T=>T.type==="activity-run"?u`<${uS} key=${T.id} activity=${T.activity} />`:u`<${bS}
                key=${T.id}
                message=${T.message}
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
        <${M} name="arrowDown" className="h-3.5 w-3.5" />
        ${l("chat.jumpToLatest")}
      </button>
    `}
    </div>
  `}function DS({notice:e,onRecover:t}){return u`
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
  `}function MS({suggestions:e,onSelect:t,disabled:a=!1}){return!e||e.length===0?null:u`
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
  `}function OS(){return u`
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
  `}function Xc(){return V("/api/webchat/v2/channels/connectable")}function LS(e,t){if(!Sh(e))return null;let a=Wc(e),n=k5(a),r=null;for(let s of t||[]){if(!R5(s))continue;let i=C5(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function Sh(e){let t=Wc(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function N5(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function _5(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>PS(Wc(n))):a}function R5(e){return e?.strategy!=="admin_managed_channels"}function k5(e){return US(e,"slack")&&PS(e)}function PS(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function Wc(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function C5(e,t,a={}){return(a.commandAliasesOnly?_5(t,{channelManagementOnly:!0}):N5(t)).reduce((r,s)=>{let i=Wc(s);return US(e,i)?Math.max(r,i.length):r},0)}function US(e,t){return t?` ${e} `.includes(` ${t} `):!1}function jS(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n={kind:"gate",gateKind:"approval",runId:t.turn_run_id,gateRef:t.gate_ref,invocationId:t.invocation_id||null,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return E5(n,a,t.body)}return e==="auth_required"?{kind:"auth_required",gateKind:"auth",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,invocationId:t.invocation_id||null,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function FS(e){if(!e?.run_id||!e.gate_ref)return null;let t=e.gate_kind||"generic",a={gateKind:t,runId:e.run_id,gateRef:e.gate_ref,invocationId:e.invocation_id||null,headline:e.headline,body:e.body||"",allowAlways:e.allow_always===!0};if(t==="auth"){let n=e.auth_context||{};return{...a,kind:"auth_required",challengeKind:n.challenge_kind||"other",provider:n.provider||null,accountLabel:n.account_label||"",authorizationUrl:n.authorization_url||null,expiresAt:n.expires_at||null}}return{...a,kind:"gate"}}function E5(e,t,a){if(!t)return e;let n=T5(t);return{...e,toolName:t.tool_name||null,description:t.reason||a,actionLabel:t.action?.label||null,destination:t.destination||null,approvalScope:t.scope||null,approvalDetails:n,parameters:n.length?n.map(r=>`${r.label}: ${r.value}`).join(`
`):null}}function T5(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function BS({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function zS(){return{terminalByInvocation:new Map}}function qS(e){e?.current?.terminalByInvocation?.clear()}function _h(e,t,a){let n=HS(t,{toolStatus:"running"});n&&bi(e,n,a)}function IS(e,t,a,n="gate_declined"){let r=HS(t,{toolStatus:"declined",toolError:n,toolErrorKind:"gate_declined"});r&&bi(e,r,a)}function bi(e,t,a){if(!t)return;let n=P5(t);n=L5(n,a),e(r=>{let s=KS(n),i=D5(r,n,s);if(i>=0){let l=[...r];return l[i]=M5(l[i],n),Nh(l[i],a),l}let o={id:s,role:"tool_activity",...n};return Nh(o,a),[...r,o]})}function HS(e,t={}){let a=e?.kind==="gate",n=e?.kind==="auth_required",r=n&&t.toolStatus==="declined";if(!e?.runId||!e?.gateRef||!e.invocationId&&!r||!a&&!n)return null;let s=e.invocationId||A5(e),i=e.toolName||e.headline||e.gateKind||"gate";return{invocationId:s,callId:s,capabilityId:e.toolName||e.gateKind||null,toolName:el(i)||i,toolStatus:t.toolStatus||"running",toolDetail:null,toolParameters:null,toolResultPreview:null,toolError:t.toolError||null,toolErrorKind:t.toolErrorKind||null,toolDurationMs:null,updatedAt:t.updatedAt||new Date().toISOString(),resultRef:null,truncated:!1,outputBytes:null,outputKind:null,turnRunId:e.runId,gateRef:e.gateRef,gateActivity:!0}}function A5(e){return`gate:${e.runId}:${e.kind}:${e.gateRef}`}function KS(e){return`tool-${e.invocationId}`}function D5(e,t,a){let n=e.findIndex(s=>s?.id===a);if(n>=0)return n;let r=t.gateRef||null;if(r){let s=e.findIndex(i=>i?.role==="tool_activity"&&i.turnRunId===t.turnRunId&&i.gateRef===r);if(s>=0)return s}return-1}function M5(e,t){let a=Zo(e.toolStatus),n=Zo(t.toolStatus),r=a&&!n,s=t.gateActivity&&!e.gateActivity,i={...e,...t,id:e.id,role:"tool_activity",invocationId:e.gateActivity&&!t.gateActivity?t.invocationId:e.invocationId||t.invocationId,callId:e.gateActivity&&!t.gateActivity?t.callId:e.callId||t.callId,toolName:s?e.toolName:t.toolName||e.toolName,toolStatus:r?e.toolStatus:t.toolStatus,toolError:t.toolError||e.toolError,toolErrorKind:t.toolErrorKind||e.toolErrorKind||null,updatedAt:r?e.updatedAt||t.updatedAt:t.updatedAt||e.updatedAt,turnRunId:t.turnRunId||e.turnRunId||null,gateRef:t.gateRef||e.gateRef||null,gateActivity:e.gateActivity&&t.gateActivity,capabilityId:s?e.capabilityId||t.capabilityId||null:t.capabilityId||e.capabilityId||null,activityOrder:O5(e,t),activityOrderSource:t.activityOrderSource||e.activityOrderSource||null};return e.gateActivity&&!t.gateActivity&&(i.id=KS(t),i.gateActivity=!1),i}function O5(e,t){return Number.isFinite(t.activityOrder)?t.activityOrder:e.activityOrder}function L5(e,t){if(!e?.invocationId)return e;if(Zo(e.toolStatus))return Nh(e,t),e;let a=t?.current?.terminalByInvocation?.get(e.invocationId);return a?Number.isFinite(e.activityOrder)?{...a,activityOrder:e.activityOrder,activityOrderSource:e.activityOrderSource||a.activityOrderSource||null}:a:e}function Nh(e,t){!e?.invocationId||!Zo(e.toolStatus)||t?.current?.terminalByInvocation?.set(e.invocationId,e)}function P5(e){let t=el(e.toolName||e.capabilityId);return{...e,toolName:t||e.toolName||"tool"}}function JS({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o,onRunSettled:l}){let c=p.default.useRef(new Set),d=p.default.useRef(null),m=p.default.useRef(null);return p.default.useCallback(f=>{let{type:h,frame:b}=f||{};if(!(!h||!b))switch(h){case"accepted":{let y=b.ack||{};y.run_id&&(d.current=y.run_id),r?.({runId:y.run_id||null,threadId:y.thread_id||e,status:y.status||null}),a(!0);return}case"running":case"capability_progress":{let y=b.progress||{};y.turn_run_id&&(d.current=y.turn_run_id,r?.($=>$&&$.runId===y.turn_run_id?{...$,status:"running"}:{runId:y.turn_run_id,threadId:e,status:"running"}),U5(n,y.turn_run_id,m)),a(!0);return}case"capability_activity":{let y=b.activity;if(!y||!y.invocation_id)return;bi(t,Xp(y),o);return}case"capability_display_preview":{let y=b.preview;if(!y||!y.invocation_id)return;let $=Jp(y);bi(t,$,o);return}case"gate":case"auth_required":{let y=jS(h,b.prompt);y&&(_h(t,y,o),n(y),r?.({runId:y.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let y=b.reply||{};t($=>[...$,{id:`reply-${y.turn_run_id||Date.now()}`,role:"assistant",content:y.text||"",timestamp:y.generated_at||new Date().toISOString(),turnRunId:y.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let y=b.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),td(c,l,y,!1);return}case"failed":{let y=b.run_state||{},$=y.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),kh(t,{runId:$,status:y.status||"failed",failureCategory:z5(y),failureSummary:null}),td(c,l,$,!1);return}case"projection_snapshot":case"projection_update":{let y=b.state?.items||[];F5({items:y,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:l,settledRunsRef:c,latestRunIdRef:d,promptRunIdRef:m,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i,o,l])}function td(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var QS=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),VS=new Set(["completed","succeeded"]),Zc=new Set(["blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]),ed=new Set(["awaiting_gate","blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]);function GS(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function U5(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function j5(e,t,a,n,r,s){let i=t?.runId||null;if(!i||r?.has(i))return!0;let o=a?.get(i);if(o)return!ed.has(o);let l=e?.current,c=l?.runId||n?.current||null;if(c&&i!==c)return!0;let d=s?.current===c;return c&&i===c&&!d&&l?.status&&!ed.has(l.status)?!0:!l?.runId||!l.status?!1:!ed.has(l.status)}function F5({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:l,promptRunIdRef:c,activeRunRef:d,locallyResolvedGatesRef:m,toolActivityStateRef:f}){let h=new Map,b=new Set,y=d?.current||null,$=y?.runId||l?.current||null;for(let v of e){let x=v.run_status;x?.run_id&&x.status&&(h.set(x.run_id,x.status),$&&$!==x.run_id&&y?.status&&!QS.has(y.status)&&Zc.has(x.status)&&b.add(x.run_id))}let g=l?.current??null;for(let v of e){if(v.run_status){let{run_id:x,status:w,failure_category:S,failure_summary:k}=v.run_status,N=QS.has(w),C=d?.current?.source==="local"?d.current.runId:null,P=!!(x&&C&&C!==x),L=g??l?.current??null,U=!!(N&&x&&L&&L!==x),F=x&&Zc.has(w)?YS(m,x):null;if(x&&b.has(x)||P)continue;if(U){YS(m,d?.current?.runId)?.outcome==="resumed"&&(B5({runId:x,activePromptRunId:d?.current?.runId,success:VS.has(w),status:w,failureCategory:S,failureSummary:k,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:l,promptRunIdRef:c,locallyResolvedGatesRef:m}),g=null);continue}if(F){GS(r,x,c),F.outcome==="resumed"?(n(!0),s?.(T=>T&&T.runId===x?{...T,status:T.status==="awaiting_gate"?"queued":T.status||"queued"}:{runId:x,threadId:t,status:"queued"}),g=x,l&&(l.current=x)):(n(!1),d?.current?.runId===x&&s?.(null),g=null,l?.current===x&&(l.current=null));continue}x&&(g=x,!N&&l&&(l.current=x),s?.(T=>T&&T.runId===x?{...T,status:w}:{runId:x,threadId:t,status:w})),x&&Zc.has(w)?c&&(c.current=x):x&&c?.current===x&&(c.current=null),N?(n(!1),r(null),s?.(null),Rh(m,x),g=null,l&&(l.current=null),x&&c?.current===x&&(c.current=null),td(o,i,x,VS.has(w)),(w==="failed"||w==="recovery_required")&&kh(a,{runId:x,status:w,failureCategory:S,failureSummary:k})):Zc.has(w)||(GS(r,x,c),Rh(m,x),n(!0))}if(v.text){let x=`text-${v.text.id}`;a(w=>{let S=v.text.id?`msg-${v.text.id}`:null,k=w.findIndex(C=>C.id===x||S&&C.id===S),N={...k>=0?w[k]:{},id:x,role:"assistant",content:v.text.body||"",timestamp:w[k]?.timestamp||new Date().toISOString(),isFinalReply:!0};if(k>=0){let C=[...w];return C[k]=N,C}return[...w,N]}),n(!1)}if(v.thinking){let x=`thinking-${v.thinking.id}`;a(w=>{let S=w.findIndex(N=>N.id===x),k={id:x,role:"thinking",content:v.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:v.thinking.run_id||null};if(S>=0){let N=[...w];return N[S]=k,N}return[...w,k]})}if(v.capability_activity){let x=v.capability_activity;x.invocation_id&&bi(a,Xp(x),f)}if(v.gate){let x=FS(v.gate),w=x?.runId||null;w&&!j5(d,x,h,l,b,c)&&!I5(m,w,x.gateRef)&&(_h(a,x,f),r(S=>S||x),s?.(S=>S&&S.runId===w?{...S,status:ed.has(S.status)?S.status:"awaiting_gate"}:{runId:w,threadId:t,status:"awaiting_gate"}),c&&(c.current=w),n(!1))}if(v.skill_activation){let{id:x,skill_names:w=[],feedback:S=[]}=v.skill_activation;if(w.length||S.length){let k=`skill-${x||w.join("-")||"activation"}`,N=[w.length?`Skill activated: ${w.join(", ")}`:"",...S].filter(Boolean).join(`
`);a(C=>C.some(P=>P.id===k)?C:[...C,{id:k,role:"system",content:N,timestamp:new Date().toISOString()}])}}}l&&g&&(l.current=g)}function B5({runId:e,activePromptRunId:t,success:a,status:n,failureCategory:r,failureSummary:s,setMessages:i,setIsProcessing:o,setPendingGate:l,setActiveRun:c,onRunSettled:d,settledRunsRef:m,latestRunIdRef:f,promptRunIdRef:h,locallyResolvedGatesRef:b}){o(!1),l(null),c?.(null),Rh(b,t),f&&(f.current=null),h?.current===t&&(h.current=null),td(m,d,e,a),(n==="failed"||n==="recovery_required")&&kh(i,{runId:e,status:n,failureCategory:r,failureSummary:s})}function z5(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function kh(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`;e(i=>{let o=i.findIndex(c=>c.id===s),l=BS({status:a,failureCategory:n,failureSummary:r});if(o>=0){if(!!!(r||n)||i[o].content===l)return i;let d=[...i];return d[o]={...d[o],content:l},d}return[...i,{id:s,role:"error",content:l,timestamp:new Date().toISOString()}]})}function YS(e,t){if(!t)return null;let a=e?.current;if(!a)return null;for(let[n,r]of a.entries())if(n.startsWith(`${t}
`))return q5(r);return null}function q5(e){return e&&typeof e=="object"?{resolution:e.resolution||null,outcome:e.outcome||null}:{resolution:e||null,outcome:null}}function Rh(e,t){if(!t)return;let a=e?.current;if(a)for(let n of Array.from(a.keys()))n.startsWith(`${t}
`)&&a.delete(n)}function I5(e,t,a){return!t||!a?!1:!!e?.current?.has(`${t}
${a}`)}function XS(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function WS(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function ZS(e,t,a,n){let r=Ch(n);return r?(H5(e,t,a,{timelineMessageId:r}),r):null}function H5(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function Ch(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var K5=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function e2({threadId:e,onEvent:t,enabled:a}){let[n,r]=p.default.useState("idle"),s=p.default.useRef(t);s.current=t;let i=p.default.useRef(null);return p.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,l=null,c=0,d=3e4;function m(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=f$({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let y=Math.min(1e3*2**c,d);l=setTimeout(m,y)};let b=(y,$)=>{let g=null;try{g=JSON.parse(y.data)}catch{return}!g||typeof g!="object"||(y.lastEventId&&(i.current=y.lastEventId),s.current?.({type:g.type||$,frame:g,lastEventId:y.lastEventId||null}))};o.onmessage=y=>b(y,"message");for(let y of K5)o.addEventListener(y,$=>b($,y))}function f(){l&&(clearTimeout(l),l=null),o&&(o.close(),o=null),r("paused")}function h(){document.visibilityState==="hidden"?f():o||m()}return m(),document.addEventListener("visibilitychange",h),()=>{document.removeEventListener("visibilitychange",h),l&&clearTimeout(l),o&&o.close()}},[a,e]),{status:n}}var Q5=3e4,V5="credential_stored_gate_resolution_failed",G5="approval_gate_pending_send_blocked",Y5="ironclaw-product-auth",Eh="ironclaw:product-auth:oauth-complete",J5="ironclaw:product-auth:oauth-complete";async function t2(e){let t=new AbortController,a=setTimeout(()=>t.abort(),Q5);try{return await e(t.signal)}finally{clearTimeout(a)}}function X5(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=V5,t.cause=e,t}function W5(){let e=new Error("Resolve the approval request before sending another message.");return e.safeErrorCode=G5,e}function Z5(e){let a=Dt.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function a2(e,t){return!e||!t?.runId||!t?.gateRef?null:`${e}
${t.runId}
${t.gateRef}`}function eD(e){return e?.continuation?.type==="turn_gate_resume"}function tD(e){if(e?.outcome)return e.outcome;let t=String(e?.status||"").toLowerCase();return t==="queued"||t==="running"?"resumed":t==="cancelled"||e?.already_terminal===!0?"cancelled":e?.already_terminal===!1?"resumed":null}function n2(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function aD(e){return e?.type===J5&&e?.status==="completed"}function nD(e,t,a){if(!aD(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function Th(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function rD(e){if(!Sh(e))return null;try{let a=(await Dt.fetchQuery({queryKey:["connectable-channels"],queryFn:Xc}))?.channels||[];return LS(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function r2(e){let t=p.default.useRef(e),a=p.default.useRef(new Map),n=p.default.useRef(1),[r,s]=p.default.useState(0),[i,o]=p.default.useState(Date.now()),[l,c]=p.default.useState(null),d=p.default.useRef(l),m=p.default.useCallback(J=>{let re=typeof J=="function"?J(d.current):J;d.current=re,c(re)},[]);p.default.useEffect(()=>{d.current=l},[l]);let[f,h]=p.default.useState(null),b=p.default.useCallback(()=>a.current.get(e||"__new__")||[],[e]),y=p.default.useCallback(J=>{let re=e||"__new__";J.length>0?a.current.set(re,J):a.current.delete(re)},[e]),{messages:$,hasMore:g,nextCursor:v,isLoading:x,loadError:w,loadHistory:S,seedThreadMessages:k,setMessages:N}=z$(e,{getPendingMessages:b,setPendingMessages:y}),[C,P]=p.default.useState(!1),L=p.default.useRef(C),U=p.default.useCallback(J=>{let re=typeof J=="function"?J(L.current):J;L.current=re,P(re)},[]),[F,T]=p.default.useState(null),K=p.default.useRef(F),[ee,ne]=p.default.useState(null),he=p.default.useCallback(J=>{let re=K.current,ie=typeof J=="function"?J(re):J;Object.is(ie,re)||(K.current=ie,T(ie))},[]),[xt,pt]=p.default.useState(e),Oe=p.default.useRef(zS()),De=p.default.useRef(new Map),at=p.default.useRef({gateKey:null,credentialRef:null,inFlight:!1}),$t=p.default.useRef(!1),Le=p.default.useRef(null);xt!==e&&(pt(e),P(!1),T(null),ne(null),c(null),h(null)),p.default.useEffect(()=>{t.current=e},[e]),p.default.useEffect(()=>()=>{Le.current?.threadId===e&&(Le.current=null)},[e]),p.default.useEffect(()=>{K.current=F},[F]),p.default.useEffect(()=>{L.current=C},[C]),p.default.useEffect(()=>{let J=a2(e,F);ne(re=>re&&re.gateKey!==J?null:re)},[F,e]),p.default.useEffect(()=>{qS(Oe),De.current.clear()},[e]);let Pa=Math.max(0,Math.ceil((r-i)/1e3)),kt=F?.runId&&F?.gateRef?`${F.runId}
${F.gateRef}`:null;p.default.useEffect(()=>{if(!r)return;let J=setInterval(()=>o(Date.now()),250);return()=>clearInterval(J)},[r]),p.default.useEffect(()=>{at.current.gateKey!==kt&&(at.current={gateKey:kt,credentialRef:null,inFlight:!1})},[kt]),p.default.useEffect(()=>{if(!n2(F))return;let J=Date.now(),re=D=>{nD(D,F,J)&&(he(z=>n2(z)?null:z),U(!0))},ie=null;typeof window.BroadcastChannel=="function"&&(ie=new window.BroadcastChannel(Y5),ie.onmessage=D=>re(D.data));let _=D=>{D.key===Eh&&re(Th(D.newValue))};window.addEventListener("storage",_),re(Th(window.localStorage?.getItem?.(Eh)));let E=window.setInterval(()=>{re(Th(window.localStorage?.getItem?.(Eh)))},500);return()=>{window.clearInterval(E),ie&&ie.close(),window.removeEventListener("storage",_)}},[F]);let la=JS({threadId:e,setMessages:N,setIsProcessing:U,setPendingGate:he,setActiveRun:m,activeRunRef:d,locallyResolvedGatesRef:De,toolActivityStateRef:Oe,onRunSettled:(J,{success:re})=>{let ie=Le.current;ie?.runId===J?Le.current=null:J&&ie&&!ie.runId&&(Le.current={...ie,runId:J,settledBeforeResponse:!0}),re&&y([]),S(void 0,{preserveClientOnly:!0,finalReplyTimestampByRun:J&&re?{[J]:new Date().toISOString()}:null})}}),{status:rn}=e2({threadId:e,onEvent:la,enabled:!!e}),ua=p.default.useCallback(async(J,re={})=>{let{threadId:ie,attachments:_=[],displayContent:E}=re,D=_.map(D$),z=_.map(M$),B=typeof E=="string"?E:J;if(F||K.current)throw W5();let O=ie||e,Q=d.current,ue=!!Q&&!!O&&Q.threadId===O,ge=L.current&&!!O&&O===e,vt=!!O&&Le.current?.threadId===O;if($t.current||ge||ue||vt)return null;if(_.length===0){let oe=await rD(J);if(oe)return h(oe),{channel_connect_action:oe}}h(null);let Ce=ie||e;if(!Ce){let oe=await wc();if(Dt.invalidateQueries({queryKey:["threads"]}),Ce=oe?.thread?.thread_id,!Ce)throw new Error("createThread returned no thread_id")}let Ct=Ce,on={id:`pending-${n.current++}`,role:"user",content:B,attachments:z,retryContent:J,retryDisplayContent:B,retryAttachments:_,timestamp:new Date().toISOString(),isOptimistic:!0},ja={id:on.id,role:"user",content:B,attachments:z,retryContent:J,retryDisplayContent:B,retryAttachments:_,timestamp:on.timestamp,isOptimistic:!0};XS(a.current,Ct,on);let Fa=on.id,vr=!e||Ce===e,gr=oe=>{vr&&N(oe)},Zr=oe=>{Ce!==e&&k(Ce,oe)},es=oe=>{vr&&oe()},ts=vr;ts&&(Le.current={threadId:Ce,runId:null,settledBeforeResponse:!1}),$t.current=!0,gr(oe=>[...oe,ja]),Zr(oe=>[...oe,ja]),es(()=>{U(!0),K.current||he(null)});try{let oe=await c$({threadId:Ce,content:J,attachments:D});Z5(Ce)&&Dt.invalidateQueries({queryKey:["threads"]});let as=!1;if(oe?.run_id&&ts){let Lt=Le.current;as=!!(Lt&&Lt.threadId===Ce&&Lt.runId===oe.run_id&&Lt.settledBeforeResponse),as?Le.current=null:Le.current={threadId:Ce,runId:oe.run_id,settledBeforeResponse:!1}}else ts&&(Le.current=null);oe?.run_id&&vr&&!as&&m({runId:oe.run_id,threadId:oe.thread_id||Ce,status:oe.status||null,source:"local"});let xl=ZS(a.current,Ct,Fa,oe?.accepted_message_ref)||Ch(oe?.accepted_message_ref);if(xl){let Lt=ns=>ns.map(An=>An.id===Fa?{...An,timelineMessageId:xl}:An);gr(Lt),Zr(Lt)}if(oe?.outcome==="rejected_busy"){ts&&(Le.current=null);let Lt=ns=>ns.map(An=>An.id===Fa?{...An,isOptimistic:!1,status:"error"}:An);if(gr(Lt),Zr(Lt),oe?.notice){let ns=(Mi=vr)=>{let Tk={id:`system-rejected-${n.current++}`,role:"system",content:oe.notice,timestamp:new Date().toISOString(),isOptimistic:!1},sv=Ak=>[...Ak,Tk];Mi&&N(sv),(!Mi||Ce!==e)&&k(Ce,sv)};if(!t.current||t.current===Ce){let Mi=a2(Ce,K.current);Mi?ne({gateKey:Mi,content:oe.notice}):ns()}else ns(!1)}es(()=>U(!1)),$t.current=!1}else oe?.run_id||(ts&&(Le.current=null),$t.current=!1);return oe}catch(oe){ts&&(Le.current=null),oe.status===429&&s(Date.now()+iD(oe));let as=xl=>xl.map(Lt=>Lt.id===Fa?{...Lt,isOptimistic:!1,status:"error",error:oe.message}:Lt);throw gr(as),Zr(as),es(()=>U(!1)),$t.current=!1,oe&&typeof oe=="object"&&(oe.optimisticMessageId=Fa,oe.optimisticThreadId=Ce),oe}finally{$t.current=!1,WS(a.current,Ct,Fa)}},[e,F,N,k,U,he,m]),Vt=p.default.useCallback(async(J,re={})=>{if(!F)return;let{runId:ie,gateRef:_}=F;if(!ie||!_)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");let E=await Kp({threadId:e,runId:ie,gateRef:_,resolution:J,always:re.always,credentialRef:re.credentialRef}),D=tD(E);if(De.current.set(`${ie}
${_}`,{resolution:J,outcome:D}),sD(J)&&D==="resumed"&&IS(N,F,Oe),he(null),D==="resumed"){U(!0),m({runId:E?.run_id||ie,threadId:E?.thread_id||e,status:E?.status||"queued"});return}U(!1),m(null)},[F,e,N,m]),sn=p.default.useCallback(async J=>{if(!F)throw new Error("auth gate is no longer pending");let{runId:re,gateRef:ie,provider:_}=F;if(!re||!ie||!_)throw new Error("auth gate is missing required credential metadata");let E=F.accountLabel||`${_} credential`,D=`${re}
${ie}`;if(at.current.gateKey!==D&&(at.current={gateKey:D,credentialRef:null,inFlight:!1}),at.current.inFlight)throw new Error("auth token submission already in progress");at.current.inFlight=!0;try{let z=at.current.credentialRef,B=null;if(!z){if(B=await t2(O=>h$({provider:_,accountLabel:E,token:J,threadId:e,runId:re,gateRef:ie,signal:O})),z=B?.credential_ref,!z)throw new Error("manual token submit returned no credential_ref");at.current.credentialRef=z}if(!eD(B))try{await t2(O=>Kp({threadId:e,runId:re,gateRef:ie,resolution:"credential_provided",credentialRef:z,signal:O}))}catch(O){throw X5(O)}at.current={gateKey:null,credentialRef:null,inFlight:!1},he(null),U(!0)}catch(z){throw at.current.gateKey===D&&(at.current.inFlight=!1),z}},[F,e]),ht=p.default.useCallback(async J=>{let re=l?.runId;if(!re||!e)return;he(null),U(!1),m(null),$t.current=!1;let ie=Le.current;(ie?.runId===re||ie?.threadId===e)&&(Le.current=null),await p$({threadId:e,runId:re,reason:J})},[l,e]),ca=p.default.useCallback(()=>{g&&v&&S(v)},[g,v,S]),_a=p.default.useCallback(async(J,re,ie)=>{let _="approved",E=!1;re==="deny"?_="denied":re==="cancel"?_="cancelled":re==="always"&&(_="approved",E=!0),await Vt(_,{always:E})},[Vt]),da=p.default.useCallback(()=>{},[]),Ua=p.default.useCallback(async J=>{if(!J||J.status!=="error")return;let re=typeof J.retryContent=="string"?J.retryContent:typeof J.content=="string"?J.content:"",ie=Array.isArray(J.retryAttachments)?J.retryAttachments:[];if(!re&&ie.length===0)return;let _=D=>D.filter(z=>z.id!==J.id),E=D=>D.some(B=>B.id!==J.id&&B.role==="user"&&B.status==="error"&&B.retryContent===re)||D.some(B=>B.id===J.id)?D:[...D,J];N(_),e&&k(e,_);try{await ua(re,{threadId:e,attachments:ie,displayContent:typeof J.retryDisplayContent=="string"?J.retryDisplayContent:J.content})===null&&(N(E),e&&k(e,E))}catch(D){if(D?.optimisticMessageId){N(_),e&&k(e,_);return}N(E),e&&k(e,E)}},[ua,k,N,e]);return{messages:$,isProcessing:C,pendingGate:F,busyGateNotice:ee,channelConnectAction:f,activeRun:l,sseStatus:rn,historyLoading:x,historyLoadError:w,hasMore:g,cooldownSeconds:Pa,send:ua,resolveGate:Vt,submitAuthToken:sn,cancelRun:ht,loadMore:ca,dismissChannelConnectAction:()=>h(null),suggestions:[],setSuggestions:da,retryMessage:Ua,approve:_a,recoverHistory:da,recoveryNotice:null}}function sD(e){return e==="denied"||e==="cancelled"}function iD(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function s2({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}function oD(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function ad({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let l=o.toString(),c=`/logs${l?`?${l}`:""}`;return i?`/v2${c}`:c}function i2(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(oD),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}var lD=1500;function o2({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let o=R(),{messages:l,isProcessing:c,pendingGate:d,busyGateNotice:m,channelConnectAction:f,suggestions:h,sseStatus:b,historyLoading:y,historyLoadError:$,hasMore:g,cooldownSeconds:v,recoveryNotice:x,activeRun:w,send:S,cancelRun:k,retryMessage:N,approve:C,recoverHistory:P,loadMore:L,setSuggestions:U,submitAuthToken:F,dismissChannelConnectAction:T}=r2(t),K=p.default.useMemo(()=>e.find(ht=>ht.id===t)||null,[e,t]),ee=p.default.useMemo(()=>s2({gatewayStatus:i,activeThread:K}),[i,K]),ne=!!t&&!!d,he=!!t&&c,xt=l.length>0||he||ne||!!f,pt=!y&&!xt&&!$,Oe=ne?"Resolve the approval request before sending another message.":"",De=ne||he&&!ne||v>0,at=p.default.useRef(De);at.current=De;let $t=Oe||(v>0?`Retry in ${v}s`:void 0),Le=t||nl,Pa=!!(t&&w?.runId&&w.threadId===t&&he&&!ne),kt=t&&w?.runId&&w.threadId===t?ad({threadId:t,runId:w.runId},{absolute:!0}):null,la=p.default.useCallback(async(ht,{images:ca=[],attachments:_a=[],displayContent:da}={})=>{if(ne)throw new Error(Oe);if(at.current)return null;let Ua=await S(ht,{images:ca,attachments:_a,displayContent:da,threadId:t}),J=Ua?.thread_id||t;return!t&&J&&a&&a(J,{replace:!0}),Ua},[t,ne,Oe,De,a,S]),rn=p.default.useCallback(async ht=>{De||(U([]),await la(ht))},[De,la,U]),ua=p.default.useCallback(()=>k("user_requested"),[k]);p.default.useEffect(()=>{if(!t)return;if(d){Oc(t,Na.NEEDS_ATTENTION);return}if(c){Oc(t,Na.RUNNING);return}let ht=setTimeout(()=>Xw(t),lD);return()=>clearTimeout(ht)},[t,d,c]);let[Vt,sn]=p.default.useState(!1);return p.default.useEffect(()=>{let ht=ca=>{if(ca.key==="Escape"){sn(!1);return}if(ca.key!=="?")return;let _a=ca.target,da=_a?.tagName;da==="INPUT"||da==="TEXTAREA"||_a?.isContentEditable||(ca.preventDefault(),sn(Ua=>!Ua))};return window.addEventListener("keydown",ht),()=>window.removeEventListener("keydown",ht)},[]),u`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${tS} status=${b} />

        ${c&&!d&&kt&&u`
          <div className="flex justify-end border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-1.5">
            <${Rn}
              to=${kt}
              className="inline-flex h-8 items-center gap-1.5 rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
              title=${o("nav.logs")}
            >
              <${M} name="list" className="h-3.5 w-3.5" />
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

        ${pt&&u`
          <${aS}
            onSuggestion=${rn}
            onSend=${la}
            disabled=${!1}
            sendDisabled=${De}
            initialText=${r}
            resetKey=${s}
            draftKey=${Le}
            context=${ee}
            statusText=${$t}
            canCancel=${Pa}
            onCancel=${ua}
          />
        `}
        ${!pt&&u`
          <${AS}
            messages=${l}
            isLoading=${y}
            hasMore=${g}
            onLoadMore=${L}
            onRetryMessage=${N}
            threadId=${t}
            pending=${he}
          >
            ${x&&u`
              <${DS}
                notice=${x}
                onRecover=${P}
              />
            `}
            ${he&&!ne&&u`<${OS} />`}
            ${f&&u`
              <${W1}
                connectAction=${f}
                onDismiss=${T}
              />
            `}
            ${d&&(d.kind==="auth_required"?d.challengeKind==="oauth_url"?u`
                  <${G1}
                    gate=${d}
                    onCancel=${()=>C(d.requestId,"cancel",d.kind)}
                  />
                `:d.challengeKind==="manual_token"?u`
                  <${Y1}
                    gate=${d}
                    onSubmit=${F}
                    onCancel=${()=>C(d.requestId,"cancel",d.kind)}
                  />
                `:u`
                  <${V1}
                    gate=${d}
                    onCancel=${()=>C(d.requestId,"cancel",d.kind)}
                  />
                `:u`
              <${Q1}
                gate=${d}
                onApprove=${()=>C(d.requestId,"approve",d.kind)}
                onDeny=${()=>C(d.requestId,"deny",d.kind)}
                onAlways=${()=>C(d.requestId,"always",d.kind)}
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

          <${MS}
            suggestions=${h}
            onSelect=${rn}
            disabled=${De}
          />

          <${Qc}
            onSend=${la}
            disabled=${!1}
            sendDisabled=${De}
            initialText=${r}
            resetKey=${s}
            draftKey=${Le}
            context=${ee}
            statusText=${$t}
            canCancel=${Pa}
            onCancel=${ua}
          />
        `}
      </div>
      <${nS}
        open=${Vt}
        onClose=${()=>sn(!1)}
      />
    </div>
  `}function Ah(){let{threadsState:e,gatewayStatus:t}=wa(),{threadId:a}=it(),n=ve(),r=Ae(),s=r.state?.composerDraft||"",i=a||null;p.default.useEffect(()=>{i&&i!==e.activeThreadId?e.setActiveThreadId(i):i||e.setActiveThreadId(null)},[i]);let o=p.default.useCallback((l,c={})=>{if(!l){e.setActiveThreadId(null),n("/chat",c);return}e.setActiveThreadId(l),n(`/chat/${l}`,c)},[e,n]);return u`
    <${o2}
      threads=${e.threads}
      activeThreadId=${i}
      onSelectThread=${o}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function l2(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?ui(e,t):"",model:e?Dc(e,t):""}}function u2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:l}){let[c,d]=p.default.useState(()=>l2(e,a)),[m,f]=p.default.useState(""),[h,b]=p.default.useState([]),[y,$]=p.default.useState(null),[g,v]=p.default.useState(""),x=p.default.useRef(!!e);p.default.useEffect(()=>{n&&(d(l2(e,a)),f(""),b([]),$(null),v(""),x.current=!!e)},[n,e,a]);let w=e?.builtin===!0,S=e&&!e.builtin,k=p.default.useCallback((U,F)=>{d(T=>{let K={...T,[U]:F};return U==="name"&&!x.current&&(K.id=kw(F)),K})},[]),N=p.default.useCallback(()=>!w&&(!c.name.trim()||!c.id.trim())?l("llm.fieldsRequired"):!w&&!Cw(c.id.trim())?l("llm.invalidId"):!S&&!w&&t.includes(c.id.trim())?l("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,w,S,l]),C=p.default.useCallback(async()=>{let U=N();if(U){$({tone:"error",text:U});return}v("save");try{await s({form:c,apiKey:m,provider:e}),r()}catch(F){$({tone:"error",text:F.message})}finally{v("")}},[m,c,r,s,e,N]),P=p.default.useCallback(async()=>{if(!c.model.trim()){$({tone:"error",text:l("llm.modelRequired")});return}v("test");try{let U=await i(sh(e,c,m,a));$({tone:U.ok?"success":"error",text:U.message})}catch(U){$({tone:"error",text:U.message})}finally{v("")}},[m,a,c,i,e,l]),L=p.default.useCallback(async()=>{if((w?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){$({tone:"error",text:l("llm.baseUrlRequired")});return}v("models");try{let F=await o(sh(e,c,m,a));if(!F.ok||!Array.isArray(F.models)||!F.models.length)$({tone:"error",text:F.message||l("llm.modelsFetchFailed")});else{b(F.models);let T=Ew(c.model,F.models);T!==null&&k("model",T),$({tone:"success",text:l("llm.modelsFetched",{count:F.models.length})})}}catch(F){$({tone:"error",text:F.message})}finally{v("")}},[m,a,c,w,o,e,l,k]);return{form:c,apiKey:m,models:h,message:y,busy:g,isBuiltin:w,isEditing:S,setApiKey:f,update:k,submit:C,runTest:P,fetchModels:L,markIdEdited:()=>{x.current=!0}}}function nd({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let l=R(),c=u2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:l});if(!n)return null;let{form:d,apiKey:m,models:f,message:h,busy:b,isBuiltin:y,isEditing:$}=c,g=y?l("llm.configureProvider",{name:e.name||e.id}):l($?"llm.editProvider":"llm.newProvider");return u`
    <${vi} open=${n} onClose=${r} title=${g} size="lg">
      <${gi} className="space-y-4">
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
            <${yh} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${rh.map(v=>u`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
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
            <${A} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${b!==""} onClick=${c.fetchModels}>
              ${l(b==="models"?"llm.fetchingModels":"llm.fetchModels")}
            <//>
          </div>
        </label>

        ${f.length>0&&u`
          <${yh} value=${d.model} onChange=${v=>c.update("model",v.target.value)}>
            ${f.map(v=>u`<option key=${v} value=${v}>${v}</option>`)}
          <//>
        `}

        ${h&&u`
          <div className=${h.tone==="error"?"text-sm text-red-200":"text-sm text-mint"} role="status">
            ${h.text}
          </div>
        `}
      <//>
      <${yi}>
        <${A} type="button" variant="secondary" disabled=${b!==""} onClick=${c.runTest}>
          ${l(b==="test"?"llm.testing":"llm.testConnection")}
        <//>
        <${A} type="button" variant="ghost" disabled=${b!==""} onClick=${r}>${l("common.cancel")}<//>
        <${A} type="button" disabled=${b!==""} onClick=${c.submit}>
          ${l(b==="save"?"common.saving":"common.save")}
        <//>
      <//>
    <//>
  `}function rd({login:e}){let t=R(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return u`
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
  `}function uD(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function sd({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=ci({settings:e,gatewayStatus:t}),[s,i]=p.default.useState(null),[o,l]=p.default.useState(!1),[c,d]=p.default.useState(null),m=p.default.useRef(null),f=p.default.useCallback((g,v)=>{m.current&&window.clearTimeout(m.current),d({tone:g,text:v}),m.current=window.setTimeout(()=>d(null),3500)},[]);p.default.useEffect(()=>()=>{m.current&&window.clearTimeout(m.current)},[]);let h=p.default.useCallback((g=null)=>{i(g),l(!0)},[]),b=p.default.useCallback(async g=>{try{await r.setActiveProvider(g),f("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(h(g),f("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):f("error",v.message)}},[h,r,f,n]),y=p.default.useCallback(async({form:g,apiKey:v,provider:x})=>{if(x?.builtin){await r.saveBuiltinProvider({provider:x,form:g,apiKey:v}),f("success",n("llm.providerConfigured",{name:x.name||x.id}));return}let w=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:x});f("success",n(x?"llm.providerUpdated":"llm.providerAdded",{name:w.name||w.id}))},[r,f,n]),$=p.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),f("success",n("llm.providerDeleted"))}catch(v){f("error",v.message)}},[r,f,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>uD(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:h,closeDialog:()=>l(!1),handleUse:b,handleSave:y,handleDelete:$}}var cD=3e5;function dD(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function mD(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function fD(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=l=>{let c=l.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},cD);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var pD=3e5,hD=9e5,vD=2e3;async function c2(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,vD)),(await Ac().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function id({onSuccess:e}={}){let t=R(),a=W(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),[o,l]=p.default.useState(!1),[c,d]=p.default.useState(""),[m,f]=p.default.useState(null),h=p.default.useCallback(()=>{i(""),d(""),f(null)},[]),b=p.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),y=p.default.useCallback(async v=>{if(h(),dD()){i(t("onboarding.nearaiLocalSso"));return}let x=window.open("about:blank","_blank");if(!x){i(t("onboarding.nearaiFailed"));return}try{x.opener=null}catch{}r(!0);try{let{auth_url:w}=await sw({provider:v,origin:window.location.origin});x.location.href=w;let S=await c2("nearai",pD,x);if(S==="active"){await b();return}x.close(),i(t(S==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{x.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[b,h,t]),$=p.default.useCallback(async()=>{h(),r(!0);try{let v=mD(),x=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!x){i(t("onboarding.nearaiFailed"));return}x.opener=null;let w=await fD(x,v);if(!w){i(t("onboarding.nearaiFailed"));return}await iw({account_id:w.accountId,public_key:w.publicKey,signature:w.signature,message:w.message,recipient:w.recipient,nonce:w.nonce}),await b()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[b,h,t]),g=p.default.useCallback(async()=>{h();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}l(!0);try{let{user_code:x,verification_uri:w}=await ow();f({userCode:x,verificationUri:w}),v&&(v.location.href=w);let S=await c2("openai_codex",hD,v);if(S==="active"){await b();return}v&&v.close(),d(t(S==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{l(!1)}},[b,h,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:m,startNearai:y,startNearaiWallet:$,startCodex:g}}var d2="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",gD="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",yD="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",bD="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",xD={nearai:{color:"#00ec97",path:gD},openai_codex:{color:"#10a37f",path:d2},openai:{color:"#10a37f",path:d2},anthropic:{color:"#d97757",path:yD},ollama:{color:null,path:bD}};function m2({id:e,name:t}){let a=xD[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return u`
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
  `}var $D=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function wD({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=p.default.useState(!1),o=p.default.useRef(null),l=t||a.nearaiBusy;p.default.useEffect(()=>{if(!s)return;let d=f=>{o.current&&!o.current.contains(f.target)&&i(!1)},m=f=>{f.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",m),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",m)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return u`
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
  `}function SD({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let l=s(e.nameKey),c;return e.auth==="nearai"?c=u`<${wD} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=u`
      <${A} type="button" variant="secondary" size="sm" disabled=${r.codexBusy} onClick=${r.startCodex}>
        ${s("onboarding.signIn")}
      <//>
    `:a?c=u`<${A} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>i(t)}>
      ${s("llm.use")}
    <//>`:c=u`<${A} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>o(t)}>
      ${s("onboarding.setUp")}
    <//>`,u`
    <${ae} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-4">
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <${m2} id=${e.id} name=${l} />
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
  `}function f2(){let{isAdmin:e=!1,isChecking:t=!1}=wa();return t?null:e?u`<${ND} />`:u`<${ot} to="/chat" replace />`}function ND(){let e=R(),t=ve(),a=W(),{gatewayStatus:n}=wa(),r=sd({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=$D.map(m=>({entry:m,provider:s.providers.find(f=>f.id===m.id)})).filter(m=>m.provider),o=p.default.useCallback(()=>t("/chat"),[t]),l=id({onSuccess:o}),c=p.default.useCallback(async m=>{let f=m.active_model||m.default_model||"";await sl({provider_id:m.id,model:f}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=p.default.useCallback(async({form:m,apiKey:f,provider:h})=>{await r.handleSave({form:m,apiKey:f,provider:h});let b=h?.id||m.id.trim(),y=m.model?.trim()||h?.default_model||"";await sl({provider_id:b,model:y}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?u`
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
              <${SD}
                key=${m.id}
                entry=${m}
                provider=${f}
                configured=${Kr(f,s.builtinOverrides)}
                isBusy=${s.isBusy}
                login=${l}
                t=${e}
                onUse=${c}
                onSetUp=${r.openDialog}
              />
            `)}
        </div>

        <${rd} login=${l} />

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

      <${nd}
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
  `}function I({children:e,className:t="",...a}){return u`<${ae} className=${t} ...${a}>${e}<//>`}function et({label:e,value:t,tone:a="muted",badgeLabel:n,detail:r,showDivider:s=!0,className:i="",valueClassName:o="text-[1.75rem] md:text-[2rem]"}){return u`
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
          ${r&&u`<div className="mt-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            ${r}
          </div>`}
        </div>
        <${q} tone=${a} label=${n??a} />
      </div>
    </div>
  `}function p2({items:e}){return u`
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
  `;return n?u`<${ae} padding="lg">${r}<//>`:u`<div className="py-8">${r}</div>`}var h2={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function an({result:e,onDismiss:t}){return e?u`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",h2[e.type]||h2.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}var v2="",_D={workspace:"home"};function od(e){return _D[e]||e}function fl(e){return[...e||[]].sort((t,a)=>t.is_dir!==a.is_dir?t.is_dir?-1:1:t.name.localeCompare(a.name,void 0,{sensitivity:"base"}))}function xi(e){return e?e.split("/").filter(Boolean):[]}function ld(e){return e?`/workspace/${xi(e).map(encodeURIComponent).join("/")}`:"/workspace"}function Dh(e){let t=xi(e);return t.pop(),t.join("/")}function g2(e){return/\.mdx?$/i.test(e||"")}function ud({path:e,onNavigate:t}){let a=R(),n=xi(e),r="";return u`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button
        type="button"
        onClick=${()=>t("/workspace")}
        className="text-signal hover:underline"
      >
        ${a("workspace.breadcrumbRoot")}
      </button>
      ${n.map((s,i)=>{r=r?`${r}/${s}`:s;let o=r,l=i===0?od(s):s;return u`
          <span key=${o} className="text-iron-400">/</span>
          <button
            key=${`${o}-button`}
            type="button"
            onClick=${()=>t(ld(o))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${l}
          </button>
        `})}
    </div>
  `}function RD(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function y2({path:e,entries:t,isLoading:a,filter:n,onOpen:r,onNavigate:s}){let i=R();if(a)return u`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;let o=(t||[]).filter(f=>!RD(f.path)),l=String(n||"").trim().toLowerCase(),c=l?o.filter(f=>f.name.toLowerCase().includes(l)):o,d=fl(c),m;return o.length?d.length?m=u`
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
        <${ud} path=${e} onNavigate=${s} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">${m}</div>
    <//>
  `}var cd="/api/webchat/v2/fs",kD=1024*1024,CD=8*1024*1024;function b2(e){let t=String(e||"").split("/").filter(Boolean);return{mount:t.shift()||"",path:t.join("/")}}function ED(e,t){return t?`${e}/${t}`:e}function TD(e){let t=String(e||"").toLowerCase();return t.startsWith("text/")||t==="application/json"||t==="application/javascript"||t==="application/xml"||t.endsWith("+json")||t.endsWith("+xml")}function AD(e){return String(e||"").toLowerCase().startsWith("image/")}function DD(e){let t=String(e||"").toLowerCase();return t.startsWith("audio/")||t.startsWith("video/")||t.startsWith("font/")||t==="application/pdf"||t==="application/zip"||t==="application/gzip"}function MD(e){if(e.subarray(0,Math.min(e.length,8192)).indexOf(0)!==-1)return!0;try{return new TextDecoder("utf-8",{fatal:!0}).decode(e),!1}catch{return!0}}function OD(e,t){let a=new URL(`${cd}/content`,window.location.origin);return a.searchParams.set("mount",e),a.searchParams.set("path",t),a.pathname+a.search}async function LD(){return(await V(`${cd}/mounts`))?.mounts||[]}async function $i(e=""){if(!e)return{entries:(await LD()).map(o=>({name:od(o.mount),path:o.mount,is_dir:!0}))};let{mount:t,path:a}=b2(e),n=new URL(`${cd}/list`,window.location.origin);return n.searchParams.set("mount",t),a&&n.searchParams.set("path",a),{entries:((await V(n.pathname+n.search))?.entries||[]).map(i=>({name:i.name,path:ED(t,i.path),is_dir:i.kind==="directory"}))}}async function x2(e){let{mount:t,path:a}=b2(e);if(!t||!a)return{kind:"directory",path:e};let n=new URL(`${cd}/stat`,window.location.origin);n.searchParams.set("mount",t),n.searchParams.set("path",a);let s=(await V(n.pathname+n.search))?.stat||{},i=s.mime_type||"application/octet-stream",o=Number(s.size_bytes||0),l=OD(t,a),c={path:e,mime:i,size_bytes:o,download_path:l};if(s.kind&&s.kind!=="file")return{...c,kind:"directory"};if(AD(i)){if(o>CD)return{...c,kind:"binary"};let h=await Nc(l);return{...c,kind:"image",image_data_url:h}}if(DD(i)||o>kD)return{...c,kind:"binary"};let d=await Aa(l),m=new Uint8Array(await d.arrayBuffer());if(!TD(i)&&MD(m))return{...c,kind:"binary"};let f=new TextDecoder("utf-8").decode(m);return{...c,kind:"text",content:f}}function $2(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function PD(e,t,a){let n=String(t||"").trim().toLowerCase(),r=(e||[]).filter(s=>!$2(s.path)).filter(s=>!n||s.name.toLowerCase().includes(n)?!0:s.is_dir&&a.has(s.path));return fl(r)}function w2({entry:e,depth:t,selectedPath:a,expandedPaths:n,filter:r,onToggleDirectory:s,onSelectFile:i}){let o=R(),l=n.has(e.path),c=H({queryKey:["workspace-list",e.path],queryFn:()=>$i(e.path),enabled:e.is_dir&&l});if(e.is_dir){let d=PD(c.data?.entries,r,n);return u`
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
                  <${w2}
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
  `}function S2({entries:e,selectedPath:t,expandedPaths:a,filter:n,onToggleDirectory:r,onSelectFile:s,isLoading:i}){let o=R();if(i)return u`<div className="space-y-2 p-3">${[1,2,3,4].map(c=>u`<div key=${c} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let l=fl(e.filter(c=>!$2(c.path)));return l.length?u`
    <div className="space-y-1 p-2">
      ${l.map(c=>u`
        <${w2}
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
  `:u`<div className="px-4 py-8 text-sm text-iron-300">${o("workspace.noFiles")}</div>`}function N2({rootEntries:e,selectedPath:t,expandedPaths:a,filter:n,onFilterChange:r,isLoadingTree:s,onToggleDirectory:i,onSelectFile:o}){let l=R();return u`
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
        <${S2}
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
  `}function _2(e){return xi(e).pop()||"download"}function UD({path:e,file:t}){let a=R();return t.kind==="image"?u`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${t.image_data_url}
          alt=${_2(e)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `:t.kind==="text"?u`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${g2(e)?u`<${sa} content=${t.content} className="max-w-4xl text-base leading-7" />`:u`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
      </div>
    `:u`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${a("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `}function R2({path:e,file:t,isLoading:a,onNavigate:n}){let r=R(),[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(t?.download_path){i(!0);try{let c=await Aa(t.download_path);Vc(c,_2(e))}catch{}finally{i(!1)}}},[t,e]);if(a)return u`
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
    <${I} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${ud} path=${e} onNavigate=${n} />
        <div className="flex items-center gap-2">
          <${q} tone="muted" label=${l} />
          <${A}
            variant="secondary"
            size="sm"
            onClick=${o}
            disabled=${s}
          >${r("workspace.download")}<//>
        </div>
      </div>

      <${UD} path=${e} file=${t} />

      ${Dh(e)&&u`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${r("workspace.parent",{path:Dh(e)})}
        </div>
      `}
    <//>
  `}function k2(e){let t=R(),a=W(),[n,r]=p.default.useState(new Set),[s,i]=p.default.useState(""),[o,l]=p.default.useState(null),c=H({queryKey:["workspace-list",""],queryFn:()=>$i("")}),d=H({queryKey:["workspace-file",e],queryFn:()=>x2(e),enabled:!!e}),m=e===""||d.data?.kind==="directory",f=H({queryKey:["workspace-list",e],queryFn:()=>$i(e),enabled:m});p.default.useEffect(()=>{l(null)},[e]);let h=p.default.useCallback(y=>a.fetchQuery({queryKey:["workspace-list",y],queryFn:()=>$i(y)}),[a]),b=p.default.useCallback(async y=>{let $=new Set(n);if($.has(y)){$.delete(y),r($);return}$.add(y),r($);try{await h(y)}catch(g){l({type:"error",message:g.message||t("workspace.unableOpenDirectory")})}},[n,h,t]);return{rootEntries:c.data?.entries||[],file:d.data||null,selectionIsDirectory:m,currentEntries:f.data?.entries||[],expandedPaths:n,filter:s,setFilter:i,result:o,clearResult:()=>l(null),isLoadingTree:c.isLoading,isLoadingFile:d.isLoading,isLoadingListing:f.isLoading,isFetching:c.isFetching||d.isFetching||f.isFetching,error:c.error||d.error||f.error||null,loadDirectory:h,toggleDirectory:b,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function Mh(){let e=R(),t=ve(),n=it()["*"]||v2,r=k2(n),s=p.default.useCallback(i=>{t(ld(i))},[t]);return u`
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
            <${N2}
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
                  <${y2}
                    path=${n}
                    entries=${r.currentEntries}
                    isLoading=${r.isLoadingListing}
                    filter=${r.filter}
                    onOpen=${s}
                    onNavigate=${t}
                  />
                `:u`
                  <${R2}
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
  `}function C2(e){if(!e)return null;let t=e.metadata&&typeof e.metadata=="object"&&!Array.isArray(e.metadata)?e.metadata:{};return{id:e.project_id,name:e.name,description:e.description,goals:Array.isArray(t.goals)?t.goals:[],icon:e.icon||null,color:e.color||null,state:e.state,role:e.role,metadata:t,created_at:e.created_at,updated_at:e.updated_at,health:e.state==="archived"?"muted":"green"}}async function E2(){let t=((await r$({limit:200}))?.projects||[]).map(C2);return{attention:[],projects:t}}async function T2(e){if(!e)return null;let t=await s$({projectId:e});return C2(t?.project)}function A2(e){return Promise.resolve({missions:[],todo:!0})}function D2(e){return Promise.resolve({threads:[],todo:!0})}function M2(e){return Promise.resolve({widgets:[],todo:!0})}function O2(e){return Promise.resolve(null)}function L2(e){return Promise.resolve(null)}function P2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function U2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function j2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function F2(){let e=W(),t=H({queryKey:["projects-overview"],queryFn:E2,refetchInterval:5e3}),a=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function B2(e){let t=W(),a=!!e,n=H({queryKey:["project-detail",e],queryFn:()=>T2(e),enabled:a,refetchInterval:a?7e3:!1}),r=H({queryKey:["project-missions",e],queryFn:()=>A2(e),enabled:a,refetchInterval:a?5e3:!1}),s=H({queryKey:["project-threads",e],queryFn:()=>D2(e),enabled:a,refetchInterval:a?4e3:!1}),i=H({queryKey:["project-widgets",e],queryFn:()=>M2(e),enabled:a,refetchInterval:a?15e3:!1}),o=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function z2({projectId:e,missionId:t,threadId:a}){let n=W(),[r,s]=p.default.useState(null),i=H({queryKey:["project-mission-detail",t],queryFn:()=>O2(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=H({queryKey:["project-thread-detail",a],queryFn:()=>L2(a),enabled:!!a,refetchInterval:a?4e3:!1}),l=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=G({mutationFn:({targetMissionId:f})=>P2(f),onSuccess:f=>{s({type:"success",message:f?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to fire mission"})}}),d=G({mutationFn:({targetMissionId:f})=>U2(f),onSuccess:()=>{s({type:"success",message:"Mission paused."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to pause mission"})}}),m=G({mutationFn:({targetMissionId:f})=>j2(f),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending}}function dd(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function md(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function q2(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function I2(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function jD(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function H2(e){let t=jD(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function K2(e){let t=e?.projects||[],a=t.reduce((o,l)=>o+Number(l.cost_today_usd||0),0),n=t.reduce((o,l)=>o+Number(l.active_missions||0),0),r=t.reduce((o,l)=>o+Number(l.threads_today||0),0),s=t.reduce((o,l)=>o+Number(l.pending_gates||0),0),i=t.reduce((o,l)=>o+Number(l.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function pl(e,t){return`${e} ${t}${e===1?"":"s"}`}var FD={projects:"muted",attention:"warning",spend:"success"};function Q2({overview:e}){let t=K2(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:md(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return u`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>u`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${q} tone=${FD[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function BD(e){return e?.type==="failure"?"danger":"warning"}function zD(e){return e?.type==="failure"?"failure":"gate"}function V2({items:e,onOpenItem:t}){return e?.length?u`
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
              <${q} tone=${BD(a)} label=${zD(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function qD({project:e,onOpen:t,t:a}){return u`
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
        <${q} tone=${q2(e.health)} label=${e.health||"unknown"} />
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
          <div>${a("projects.card.spendToday",{value:md(e.cost_today_usd||0)})}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${dd(e.last_activity)}</div>
        </div>
        <${A}
          variant="secondary"
          onClick=${n=>{n.stopPropagation(),t(e.id)}}
        >${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function ID({project:e,onOpen:t,t:a}){return u`
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
          <${A}
            variant="secondary"
            onClick=${n=>{n.stopPropagation(),t(e.id)}}
          >${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function G2({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=R(),l=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return!e.length&&t>0?u`
      <${$e}
        title=${o("projects.empty.noMatchTitle")}
        description=${o("projects.empty.noMatchDesc")}
      />
    `:e.length?u`
    <div className="space-y-5">
      ${l&&u`<${ID} project=${l} onOpen=${r} t=${o} />`}

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
            <${A} onClick=${s}>${o(i?"projects.preparingChat":"projects.newProject")}<//>
          </div>
        </div>
      <//>

      ${c.length?u`<div className="grid gap-4 xl:grid-cols-2 2xl:grid-cols-3">
            ${c.map(d=>u`<${qD} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
          </div>`:u`
            <${$e}
              title=${o("projects.scoped.onlyGeneralTitle")}
              description=${o("projects.scoped.onlyGeneralDesc")}
            >
              <${A} onClick=${s}>${o(i?"projects.preparingChat":"projects.startProject")}<//>
            <//>
          `}
    </div>
  `:u`
      <${$e}
        title=${o("projects.empty.noneTitle")}
        description=${o("projects.empty.noneDesc")}
      >
        <${A} onClick=${s}>${o("projects.createFromChat")}<//>
      <//>
    `}function Y2({threads:e,selectedThreadId:t,onSelectThread:a,onNewConversation:n,isStartingConversation:r}){let s=[...e].sort((i,o)=>new Date(o.updated_at||o.created_at)-new Date(i.updated_at||i.created_at));return u`
    <${I} className="p-4 sm:p-5">
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
        ${s.length?s.slice(0,18).map(i=>{let o=H2(i);return u`
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
                    <${q} tone=${I2(i.state)} label=${i.state} />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                    <span>${i.step_count||0} steps</span>
                    <span>${i.total_tokens||0} tokens</span>
                    <span>${dd(i.updated_at||i.created_at)}</span>
                  </div>
                </button>
              `}):u`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                No project threads yet. When an automation runs or scoped chat work happens inside this project, activity will appear here.
              </div>
            `}
      </div>
    <//>
  `}var HD="/workspace";function KD(e){let t=a=>a.kind==="directory"?0:1;return[...e].sort((a,n)=>t(a)-t(n)||a.name.localeCompare(n.name,void 0,{sensitivity:"base"}))}function QD(e){return e?String(e).replace(/^\/workspace\/?/,"").split("/").filter(Boolean):[]}function J2({threadId:e}){let t=R(),[a,n]=p.default.useState(void 0),[r,s]=p.default.useState(null),i=H({queryKey:["project-files",e||"",a||""],queryFn:()=>X0({threadId:e,path:a}),enabled:!!e}),o=p.default.useMemo(()=>KD(i.data?.entries||[]),[i.data]),l=p.default.useCallback(async m=>{if(m.kind==="directory"){s(null),n(m.path);return}try{s(null);let f=await Aa(Sc({threadId:e,path:m.path})),h=URL.createObjectURL(f),b=document.createElement("a");b.href=h,b.download=m.name,document.body.appendChild(b),b.click(),b.remove(),URL.revokeObjectURL(h)}catch(f){s(f?.message||"Unable to download file")}},[e]),c=QD(a),d=u`
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
        ${c.map((m,f)=>{let h=`${HD}/${c.slice(0,f+1).join("/")}`;return u`
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
      <${I} className="p-4 sm:p-5">
        ${d}
        <div className="mt-4 rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
          ${"No files yet \u2014 they appear once a thread has run in this project."}
        </div>
      <//>
    `}function VD(e){return[...e||[]].sort((a,n)=>new Date(n.updated_at||n.created_at)-new Date(a.updated_at||a.created_at))[0]?.id||null}function X2({project:e,threads:t,selectedThreadId:a,onSelectThread:n,onNewConversation:r,isStartingConversation:s}){let i=VD(t);return u`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]">
      <div className="space-y-5">
        <div className="min-w-0">
          <h2 className="text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
          ${e.description?u`<p className="mt-1 text-sm leading-6 text-iron-300">${e.description}</p>`:null}
        </div>

        <${Y2}
          threads=${t}
          selectedThreadId=${a}
          onSelectThread=${n}
          onNewConversation=${r}
          isStartingConversation=${s}
        />
      </div>

      <${J2} threadId=${i} />
    </div>
  `}function hl(){let e=R(),t=ve(),{threadsState:a}=wa(),{projectId:n=null,threadId:r=null}=it(),[s,i]=p.default.useState(""),[o,l]=p.default.useState(null),c=F2(),d=B2(n),m=z2({projectId:n,threadId:r}),f=p.default.useMemo(()=>{let N=s.trim().toLowerCase();return N?c.overview.projects.filter(C=>[C.name,C.description,...C.goals||[]].some(P=>String(P||"").toLowerCase().includes(N))):c.overview.projects},[c.overview.projects,s]),h=p.default.useMemo(()=>c.overview.projects.find(N=>N.id===n)||null,[c.overview.projects,n]),b=p.default.useCallback(()=>{c.invalidate(),d.invalidate()},[c,d]),y=p.default.useCallback(N=>{t(`/projects/${N}`)},[t]),$=p.default.useCallback(N=>{if(N.thread_id){t(`/projects/${N.project_id}/threads/${N.thread_id}`);return}t(`/projects/${N.project_id}`)},[t]),g=p.default.useCallback(async()=>{let N=null;l(null);try{N=await a.createThread()}catch(C){l({type:"error",message:C.message||e("projects.chatAutoFail")})}t("/chat",{state:{composerDraft:e("projects.creationDraft"),threadId:N}})},[t,a]),v=p.default.useCallback(N=>{t(`/projects/${n}/threads/${N}`)},[t,n]),x=p.default.useCallback(async()=>{l(null);try{let N=await a.createThread(n);t("/chat",{state:{threadId:N}}),d.invalidate()}catch(N){l({type:"error",message:N.message||e("projects.chatAutoFail")})}},[t,a,n,d,e]),w=p.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),S=u`
    ${n&&u`<${A} variant="ghost" onClick=${()=>t("/projects")}>${e("projects.allProjects")}<//>`}
  `,k=null;return n?d.isLoading?k=u`
        <div className="space-y-4">
          ${[1,2,3].map(N=>u`<div key=${N} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `:d.error||!d.project&&!h?k=u`
        <${$e}
          title=${e("projects.unavailable")}
          description=${d.error?.message||e("projects.unavailableDesc")}
        >
          <${A} variant="secondary" onClick=${()=>t("/projects")}>${e("projects.returnToProjects")}<//>
        <//>
      `:k=u`
        <${X2}
          project=${d.project||h}
          threads=${d.threads}
          selectedThreadId=${r}
          onSelectThread=${v}
          onNewConversation=${x}
          isStartingConversation=${a.isCreating}
        />
      `:k=c.isLoading?u`
          <div className="space-y-4">
            ${[1,2,3].map(N=>u`<div key=${N} className="v2-skeleton h-40 rounded-[20px]" />`)}
          </div>
        `:u`
          <${G2}
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
            <${Q2} overview=${c.overview} />
            <${V2} items=${c.overview.attention} onOpenItem=${$} />
          `}
          ${k}
        </div>
      </div>
    </div>
  `}function vl(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not scheduled"}function gl(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function W2(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function Z2(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);return r!==0?r:new Date(n.updated_at||0).getTime()-new Date(a.updated_at||0).getTime()})}function fd({label:e,value:t}){return u`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function GD({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=R();return e.status==="Active"?u`
      <${A} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.fireNow")}<//>
      <${A} variant="secondary" onClick=${()=>n(e.id)} disabled=${t}>${s("missions.action.pause")}<//>
    `:e.status==="Paused"?u`
      <${A} onClick=${()=>r(e.id)} disabled=${t}>${s("missions.action.resume")}<//>
      <${A} variant="secondary" onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runOnce")}<//>
    `:u`<${A} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runAgain")}<//>`}function eN({mission:e,isLoading:t,error:a,isBusy:n,onFire:r,onPause:s,onResume:i,onOpenProject:o,onOpenThread:l}){let c=R();return t?u`
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
          <${fd} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${fd} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${fd} label=${c("missions.meta.nextFire")} value=${vl(e.next_fire_at)} />
          <${fd} label=${c("missions.meta.updated")} value=${vl(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${GD}
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
          <${sa} content=${e.goal||c("missions.noGoal")} />
        </div>
      <//>

      ${e.current_focus&&u`
        <${I} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${sa} content=${e.current_focus} />
          </div>
        <//>
      `}

      ${e.success_criteria&&u`
        <${I} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${sa} content=${e.success_criteria} />
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
  `}function YD(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function tN({value:e,onChange:t,children:a,label:n}){return u`
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
  `}function JD({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=R(),s=t===e.id;return u`
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
        <${A}
          variant="ghost"
          onClick=${i=>{i.stopPropagation(),n(e.project.id)}}
        >
          ${e.project.name}
        <//>
      </div>
    </div>
  `}function Oh({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:l,projectOptions:c,onSelectMission:d,onOpenProject:m}){let f=R(),h=YD(f);return u`
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
          onChange=${b=>r(b.target.value)}
          placeholder=${f("missions.searchPlaceholder")}
          className="h-11 min-w-[220px] flex-1 rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/40"
        />
        <${tN} value=${s} onChange=${i} label=${f("missions.filter.status")}>
          ${h.map(b=>u`<option key=${b.value} value=${b.value}>${b.label}<//>`)}
        <//>
        <${tN} value=${o} onChange=${l} label=${f("missions.filter.project")}>
          <option value="all">${f("missions.filter.allProjects")}</option>
          ${c.map(b=>u`<option key=${b.id} value=${b.id}>${b.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(b=>u`
              <${JD}
                key=${b.id}
                mission=${b}
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
  `}function XD(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function aN({summary:e}){let t=R(),a=XD(t);return u`
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
  `}function nN(){return Promise.resolve({projects:[],todo:!0})}function rN({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function sN(e){return Promise.resolve(null)}function iN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function oN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function lN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function uN(e){let t=H({queryKey:["mission-detail",e],queryFn:()=>sN(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function WD(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function cN(){let e=W(),[t,a]=p.default.useState(null),n=H({queryKey:["projects-overview"],queryFn:nN,refetchInterval:7e3}),r=n.data?.projects||[],s=zd({queries:r.map(f=>({queryKey:["missions","project",f.id],queryFn:()=>rN({projectId:f.id}),refetchInterval:5e3,select:h=>h?.missions||[]}))}),i=s.flatMap((f,h)=>{let b=r[h];return(f.data||[]).map(y=>WD(y,b))}),o=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),l=(f,h)=>({mutationFn:({missionId:b})=>f(b),onSuccess:()=>{a({type:"success",message:h}),o()},onError:b=>{a({type:"error",message:b.message||"Unable to update mission"})}}),c=G(l(iN,"Mission fired and a run was queued.")),d=G(l(oN,"Mission paused.")),m=G(l(lN,"Mission resumed."));return{projects:r,missions:i,summary:W2(i),isLoading:n.isLoading||s.some(f=>f.isLoading),isRefreshing:n.isFetching||s.some(f=>f.isFetching),error:n.error||s.find(f=>f.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending,invalidate:o}}function Lh(){let e=R(),t=ve(),{missionId:a=null}=it(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,l]=p.default.useState("all"),c=cN(),d=uN(a),m=p.default.useMemo(()=>{let g=n.trim().toLowerCase();return Z2(c.missions).filter(v=>{let x=!g||[v.name,v.goal,v.project?.name].some(k=>String(k||"").toLowerCase().includes(g)),w=s==="all"||v.status===s,S=o==="all"||v.project?.id===o;return x&&w&&S})},[c.missions,o,n,s]),f=p.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),h=d.mission?{...f,...d.mission,project:f?.project||null}:f,b=p.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),y=p.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),$=a?u`
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <${Oh}
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
          <${eN}
            mission=${h}
            isLoading=${d.isLoading}
            error=${d.error}
            isBusy=${c.isBusy}
            onFire=${g=>y(c.fireMission,g)}
            onPause=${g=>y(c.pauseMission,g)}
            onResume=${g=>y(c.resumeMission,g)}
            onOpenProject=${g=>t(`/projects/${g}`)}
            onOpenThread=${b}
          />
        </div>
      `:u`
        <${Oh}
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

          <${an}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${aN} summary=${c.summary} />

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
  `}var dN=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],ZD=new Set(["pending","in_progress"]),mN=new Set(["failed","interrupted","stuck","cancelled"]);function ur(e){return e?String(e).replace(/_/g," "):"unknown"}function wi(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":mN.has(e)?"danger":"muted":"muted"}function eM(e){return ZD.has(e)}function pd(e){return eM(e?.state)}function fN(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":mN.has(e.state):!1}function Vr(e,t=8){return e?String(e).slice(0,t):"unknown"}function ia(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function pN(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function Ph(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${ia(e.started_at)}`:null].filter(Boolean).join(" / ")}var tM=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function hN(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function aM({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?u`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${t==="tool_use"?a.tool_name||"Tool call":a.tool_name||"Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${hN(t==="tool_use"?a.input:a.output||a.error||a)}</pre>
      </details>
    `:t==="message"?u`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a.role||"assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${a.content||""}</div>
      </div>
    `:u`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t.replace(/_/g," ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${a.message||a.status||hN(a)}</div>
    </div>
  `}function vN({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=R(),[s,i]=p.default.useState("all"),[o,l]=p.default.useState(""),[c,d]=p.default.useState(!0),m=p.default.useRef(null),f=p.default.useMemo(()=>s==="all"?t:t.filter(b=>b.event_type===s),[t,s]);p.default.useEffect(()=>{c&&m.current&&(m.current.scrollTop=m.current.scrollHeight)},[c,f.length]);let h=p.default.useCallback(async(b=!1)=>{let y=o.trim();if(!(!y&&!b))try{await a({content:y||"(done)",done:b}),l("")}catch{}},[o,a]);return u`
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
            onChange=${b=>i(b.target.value)}
            className="v2-select h-10 rounded-md border border-white/10 bg-iron-950/90 px-3 text-sm text-white outline-none focus:border-signal/45"
          >
            ${tM.map(b=>u`<option key=${b.value} value=${b.value}>${b.label}</option>`)}
          </select>
          <label className="flex items-center gap-2 text-sm text-iron-300">
            <input type="checkbox" checked=${c} onChange=${b=>d(b.target.checked)} />
            Auto-scroll
          </label>
        </div>
      </div>

      <div ref=${m} className="mt-5 max-h-[56vh] space-y-3 overflow-y-auto rounded-[18px] border border-white/10 bg-iron-950/78 p-4">
        ${f.length?f.map(b=>u`
              <div key=${b.id||`${b.event_type}-${b.created_at}`}>
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${ia(b.created_at)}</div>
                <${aM} event=${b} />
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
            onInput=${b=>l(b.target.value)}
            onKeyDown=${b=>{b.key==="Enter"&&!b.shiftKey&&(b.preventDefault(),h(!1))}}
            placeholder=${r("job.followupPlaceholder")}
            className="h-11 rounded-md border border-white/10 bg-iron-950/90 px-3 text-sm text-white outline-none focus:border-signal/45"
          />
          <${A} variant="secondary" disabled=${n} onClick=${()=>h(!0)}>${r("common.done")}<//>
          <${A} variant="primary" disabled=${n} onClick=${()=>h(!1)}>${r("common.send")}<//>
        </div>
      `}
    <//>
  `}function gN({job:e,activeTab:t,onTabChange:a,onBack:n,onCancel:r,onRestart:s,isBusy:i,children:o}){return u`
    <div className="space-y-5">
      <${I} className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick=${n} className="text-sm text-signal hover:text-white">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-semibold tracking-tight text-white">${e.title||"Untitled job"}</h2>
              <${q} tone=${wi(e.state)} label=${ur(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${Vr(e.id)}</span>
              <span>created ${ia(e.created_at)}</span>
              ${Ph(e)&&u`<span>${Ph(e)}</span>`}
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
            ${pd(e)&&u`
              <${A} variant="secondary" disabled=${i} onClick=${()=>r(e.id)}>Cancel<//>
            `}
            ${fN(e)&&u`
              <${A} variant="primary" disabled=${i} onClick=${()=>s(e.id)}>Restart<//>
            `}
          </div>
        </div>
      <//>

      <div className="flex flex-wrap gap-2">
        ${dN.map(l=>u`
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
  `}function yN({nodes:e,depth:t=0,selectedPath:a,expandingPath:n,onToggleDirectory:r,onSelectPath:s}){return u`
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
        ${i.isDir&&i.expanded&&i.children?.length?u`<${yN}
              nodes=${i.children}
              depth=${t+1}
              selectedPath=${a}
              expandingPath=${n}
              onToggleDirectory=${r}
              onSelectPath=${s}
            />`:null}
      </div>
    `)}
  `}function bN({canBrowse:e,tree:t,selectedPath:a,selectedFile:n,fileError:r,isLoadingTree:s,isLoadingFile:i,expandingPath:o,treeError:l,onToggleDirectory:c,onSelectPath:d}){return e?u`
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <${I} className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          ${l&&u`<div className="mx-2 mb-3 rounded-md border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${l}</div>`}
          ${s?u`<div className="space-y-2 px-2">${[1,2,3,4].map(m=>u`<div key=${m} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?u`
                  <${yN}
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
    `}function Si({label:e,value:t}){return u`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function xN({job:e}){let t=(e.transitions||[]).map(a=>({title:`${ur(a.from)} -> ${ur(a.to)}`,description:[ia(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return u`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${I} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${q} tone=${wi(e.state)} label=${ur(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${Si} label="Created" value=${ia(e.created_at)} />
          <${Si} label="Started" value=${ia(e.started_at)} />
          <${Si} label="Completed" value=${ia(e.completed_at)} />
          <${Si} label="Duration" value=${pN(e.elapsed_secs)} />
          <${Si} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${Si} label="Mode" value=${e.job_mode||"Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${I} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${e.description?u`<${sa} content=${e.description} className="mt-4 text-sm leading-7 text-iron-200" />`:u`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
        <//>

        ${t.length?u`
              <${I} className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Transitions</div>
                <h3 className="mt-2 text-xl font-semibold text-white">State timeline</h3>
                <div className="mt-3">
                  <${p2} items=${t} />
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
  `}function $N({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:l,isBusy:c,isRefreshing:d}){let m=R(),f=[{value:"all",label:m("jobs.list.filter.all")},{value:"pending",label:m("jobs.list.filter.pending")},{value:"in_progress",label:m("jobs.list.filter.inProgress")},{value:"completed",label:m("jobs.list.filter.completed")},{value:"failed",label:m("jobs.list.filter.failed")},{value:"stuck",label:m("jobs.list.filter.stuck")}];if(!e.length){let h=!!n.trim()||s!=="all";return u`
      <${$e}
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
                  <${q} tone=${wi(h.state)} label=${ur(h.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${Vr(h.id)}</span>
                  <span>${m("jobs.list.created",{value:ia(h.created_at)})}</span>
                  ${h.started_at&&u`<span>${m("jobs.list.started",{value:ia(h.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${pd(h)&&u`
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
  `}var nM=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function wN({summary:e}){return u`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${nM.map(t=>u`
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
  `}function SN(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function NN(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function _N(e){return Promise.resolve(null)}function RN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function kN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function CN(e){return Promise.resolve({events:[],todo:!0})}function EN(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function Uh(e,t=""){return Promise.resolve({entries:[],todo:!0})}function TN(e,t){return Promise.resolve({content:"",todo:!0})}function AN(e){let t=W(),[a,n]=p.default.useState(null),r=H({queryKey:["job-detail",e],queryFn:()=>_N(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=H({queryKey:["job-events",e],queryFn:()=>CN(e),enabled:!!e,refetchInterval:e?2500:!1}),i=G({mutationFn:({content:o,done:l})=>EN(e,{content:o,done:l}),onSuccess:(o,{done:l})=>{n({type:"success",message:l?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return p.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function DN(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function MN(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=MN(a.children,t);if(n)return n}}return null}function hd(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:hd(n.children,t,a)}:n)}function ON(e){let[t,a]=p.default.useState([]),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState(""),c=!!(e?.project_dir&&e?.id),d=H({queryKey:["job-files-root",e?.id],queryFn:()=>Uh(e.id,""),enabled:c}),m=H({queryKey:["job-file",e?.id,n],queryFn:()=>TN(e.id,n),enabled:!!(c&&n)});p.default.useEffect(()=>{a([]),r(""),i(""),l("")},[e?.id]),p.default.useEffect(()=>{d.data?.entries?(a(DN(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let f=p.default.useCallback(async h=>{let b=MN(t,h);if(!(!b||!e?.id)){if(b.expanded){a(y=>hd(y,h,$=>({...$,expanded:!1})));return}if(b.loaded){a(y=>hd(y,h,$=>({...$,expanded:!0})));return}l(h);try{let y=await Uh(e.id,h);a($=>hd($,h,g=>({...g,expanded:!0,loaded:!0,children:DN(y.entries)}))),i("")}catch(y){i(y.message||"Unable to open folder")}finally{l("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:m.data||null,fileError:m.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:m.isLoading||m.isFetching,expandingPath:o,treeError:s,toggleDirectory:f}}function LN(){let e=W(),[t,a]=p.default.useState(null),n=H({queryKey:["jobs-summary"],queryFn:NN,refetchInterval:5e3}),r=H({queryKey:["jobs"],queryFn:SN,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=G({mutationFn:({jobId:l})=>RN(l),onSuccess:(l,{jobId:c})=>{a({type:"success",message:`Job ${Vr(c)} cancelled`}),s()},onError:l=>{a({type:"error",message:l.message||"Unable to cancel job"})}}),o=G({mutationFn:({jobId:l})=>kN(l),onSuccess:l=>{a({type:"success",message:`Restart queued as ${Vr(l?.new_job_id)}`}),s()},onError:l=>{a({type:"error",message:l.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function PN({result:e,onDismiss:t}){let a=R();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return u`
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
  `}function jh(){let e=R(),t=ve(),{jobId:a=null}=it(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,l]=p.default.useState(a?"activity":"overview"),c=LN(),d=AN(a),m=ON(d.job);p.default.useEffect(()=>{l(a?"activity":"overview")},[a]);let f=p.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(x=>{let w=!v||x.title.toLowerCase().includes(v)||x.id.toLowerCase().includes(v),S=s==="all"||x.state===s;return w&&S})},[c.jobs,n,s]),h=p.default.useCallback(v=>t(`/jobs/${v}`),[t]),b=p.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),y=p.default.useCallback(async v=>{try{let x=await c.restartJob({jobId:v});x?.new_job_id&&t(`/jobs/${x.new_job_id}`)}catch{}},[c,t]),$=u`
    ${a&&u`<${A} variant="ghost" onClick=${()=>t("/jobs")}
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
          <${A} variant="secondary" onClick=${()=>t("/jobs")}
            >${e("jobs.returnToJobs")}<//
          >
        <//>
      `;else{let v={overview:u`<${xN} job=${d.job} />`,activity:u`
          <${vN}
            job=${d.job}
            events=${d.events}
            onSendPrompt=${d.sendPrompt}
            isSendingPrompt=${d.isSendingPrompt}
          />
        `,files:u`
          <${bN}
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
        <${gN}
          job=${d.job}
          activeTab=${o}
          onTabChange=${l}
          onBack=${()=>t("/jobs")}
          onCancel=${b}
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
          <${$N}
            jobs=${f}
            totalJobs=${c.jobs.length}
            selectedJobId=${a}
            search=${n}
            onSearchChange=${r}
            stateFilter=${s}
            onStateFilterChange=${i}
            onSelectJob=${h}
            onCancelJob=${b}
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
          <${PN}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${PN}
            result=${d.promptResult}
            onDismiss=${d.clearPromptResult}
          />
          <${wN} summary=${c.summary} />
          ${g}
        </div>
      </div>
    </div>
  `}function cr(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not scheduled"}function vd(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function gd(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function UN(e=[]){return[...e].sort((t,a)=>t.enabled!==a.enabled?t.enabled?-1:1:new Date(a.next_fire_at||a.last_run_at||0).getTime()-new Date(t.next_fire_at||t.last_run_at||0).getTime())}function jN(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function rM(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function FN({runs:e}){return e?.length?u`
    <div className="space-y-3">
      ${e.map(t=>u`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${q} tone=${rM(t.status)} label=${t.status} />
              <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                ${cr(t.started_at)}
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
    `}function dr({label:e,value:t}){return u`
    <div className="rounded-xl border border-iron-700 bg-iron-950/50 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div className="mt-2 min-w-0 break-words text-sm text-iron-100">
        ${t||"\u2014"}
      </div>
    </div>
  `}function BN({title:e,value:t}){return u`
    <div>
      <h3 className="text-sm font-semibold text-iron-100">${e}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >${JSON.stringify(t||{},null,2)}</pre>
    </div>
  `}function zN({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=ve(),l=R();return t?u`
      <div className="space-y-4">
        ${[1,2,3].map(c=>u`<div key=${c} className="v2-skeleton h-32 rounded-xl" />`)}
      </div>
    `:a||!e?u`
      <${$e}
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
              tone=${vd(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${q}
              tone=${gd(e.verification_status)}
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
        <${dr} label="Action" value=${jN(e.action)} />
        <${dr} label="Next fire" value=${cr(e.next_fire_at)} />
        <${dr} label="Last run" value=${cr(e.last_run_at)} />
        <${dr} label="Run count" value=${e.run_count} />
        <${dr} label="Failures" value=${e.consecutive_failures} />
        <${dr} label="Created" value=${cr(e.created_at)} />
        <${dr} label="Routine ID" value=${e.id} />
      </div>

      ${e.conversation_id&&u`
        <div className="mt-5">
          <${A} variant="secondary" onClick=${()=>o(`/chat/${e.conversation_id}`)}>
            Open routine thread
          <//>
        </div>
      `}

      <div className="mt-6 grid gap-6 xl:grid-cols-2">
        <${BN} title=${l("routine.triggerPayload")} value=${e.trigger} />
        <${BN} title=${l("routine.actionPayload")} value=${e.action} />
      </div>

      <div className="mt-6">
        <h3 className="mb-3 text-sm font-semibold text-iron-100">Recent runs</h3>
        <${FN} runs=${e.recent_runs} />
      </div>
    <//>
  `}function qN({routine:e,selectedRoutineId:t,onSelectRoutine:a,onTriggerRoutine:n,onToggleRoutine:r,isBusy:s}){let i=t===e.id;return u`
    <article
      className=${["group flex flex-col gap-4 rounded-[18px] border p-5",i?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick=${()=>a(e.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">${e.name}</h3>
            <${q}
              tone=${vd(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${q}
              tone=${gd(e.verification_status)}
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
  `}var sM=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function Fh({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:l,onToggleRoutine:c,isBusy:d,isRefreshing:m}){let f=R();if(!e.length){let h=!!n.trim()||s!=="all";return u`
      <${$e}
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
            ${sM.map(h=>u`<option key=${h.value} value=${h.value}>${h.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(h=>u`
            <${qN}
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
  `}var iM=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function IN({summary:e}){return u`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${iM.map(t=>u`
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
  `}function HN(e){let[t,a]=p.default.useState(""),[n,r]=p.default.useState("all");return{filteredRoutines:p.default.useMemo(()=>{let i=t.trim().toLowerCase();return UN(e).filter(o=>{let l=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||l.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function KN(){return Promise.resolve({routines:[],todo:!0})}function QN(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function VN(e){return Promise.resolve(null)}function yd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function bd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function GN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function YN(e){let t=W(),[a,n]=p.default.useState(null),r=H({queryKey:["routine-detail",e],queryFn:()=>VN(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:m=>{n({type:"error",message:m.message||"Unable to update routine"})}}),o=G(i(yd,"Routine run queued.")),l=G(i(bd,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:l.mutateAsync,isBusy:o.isPending||l.isPending}}function JN(){let e=W(),[t,a]=p.default.useState(null),n=H({queryKey:["routines-summary"],queryFn:QN,refetchInterval:5e3}),r=H({queryKey:["routines"],queryFn:KN,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,m)=>({mutationFn:({routineId:f})=>d(f),onSuccess:()=>{a({type:"success",message:m}),s()},onError:f=>{a({type:"error",message:f.message||"Unable to update routine"})}}),o=G(i(yd,"Routine run queued.")),l=G(i(bd,"Routine status updated.")),c=G(i(GN,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:l.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||l.isPending||c.isPending,invalidate:s}}function Bh(){let e=ve(),{routineId:t=null}=it(),a=JN(),n=YN(t),r=HN(a.routines),s=p.default.useCallback(async(l,c)=>{try{await l({routineId:c})}catch{}},[]),i=p.default.useCallback(async(l,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:l}),e("/routines")}catch{}},[e,a]),o=t?u`
        <div className="grid gap-5 xl:grid-cols-[minmax(0,0.9fr)_minmax(440px,1.1fr)]">
          <${Fh}
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
          <${zN}
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
        <${Fh}
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

          <${an}
            result=${a.actionResult}
            onDismiss=${a.clearActionResult}
          />
          <${an}
            result=${n.actionResult}
            onDismiss=${n.clearActionResult}
          />
          <${IN} summary=${a.summary} />

          ${a.isLoading?u`
                <div className="space-y-4">
                  ${[1,2,3].map(l=>u`<div key=${l} className="v2-skeleton h-32 rounded-xl" />`)}
                </div>
              `:o}
        </div>
      </div>
    </div>
  `}function oM(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function lM(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function XN({deliveryState:e}){let t=R(),a=e.currentTarget?.target_id||"",[n,r]=p.default.useState(a),[s,i]=p.default.useState(!1),o=p.default.useRef(null);p.default.useEffect(()=>{r(a)},[a]),p.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let l=n!==a,c=e.isLoading||e.isSaving,d=l&&!c,m=!!a&&!c,f=e.finalReplyTargets.length>0,h=e.targets.some(P=>P?.capabilities?.final_replies&&P?.target?.status==="unavailable"),b=f||h,y=P=>(o.current&&clearTimeout(o.current),i(!1),P.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),$=()=>{d&&y(e.saveFinalReplyTarget(n||null))},g=()=>{m&&(r(""),y(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),x=e.currentStatus,w=x==="available"?"success":x==="unavailable"?"warning":"muted",S=t(x==="available"?"automations.delivery.pill.ready":x==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),k=!!e.currentTarget,N=t(k?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),C=lM(t("automations.delivery.footnote"),{command:u`<code
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
        ${k&&u`
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
            ${N}
          </span>
          <div
            className="flex flex-col gap-3"
            role="radiogroup"
            aria-label=${t("automations.delivery.title")}
          >

            <!-- Available external targets -->
            ${e.finalReplyTargets.map(P=>{let L=P?.target?.target_id??"",U=P?.target?.display_name||P?.target?.target_id||"",F=P?.target?.description||"",T=P?.target?.status??"available",K=n===L;return u`
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
                      ${U}
                    </div>
                    ${F&&u`<div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                      ${F}
                    </div>`}
                  </div>
                  <${q}
                    tone=${oM(T)}
                    label=${t(T==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.ready")}
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
        ${b&&u`
          <div
            className="rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-faint)]"
          >
            ${C}
          </div>
        `}

      </div>
    <//>
  `}var uM=["schedule","once"],ZN={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},e_={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},t_={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function oa(e){return typeof e=="function"?e:t=>t}var qh=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:Tn},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:NM},{value:"completed",labelKey:"automations.filter.completed",predicate:_M}];function a_(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>uM.includes(r?.source?.type)).map(r=>bM(r,t,a)).sort(SM)}function n_(e,t){let a=qh.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function r_(e){let t=e.filter(i=>i.state!=="completed"),a=t.filter(i=>Tn(i)).length,n=t.filter(i=>i.has_running_run).length,r=t.filter(i=>i.has_failed_runs).length,s=t.filter(i=>Tn(i)&&zh(i)!=null).sort((i,o)=>(i.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(o.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:t.length,active:a,running:n,failures:r,nextRun:s?.next_run_label||null}}function cM(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=EM(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:l,month:c,dayOfWeek:d,year:m}=s,f=t&&typeof t=="string"?t:null,h=f?` (${f})`:"",b=m==="*"&&l==="*"&&c==="*"&&d==="*";if(b&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=TM(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(mr(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let y=RM(o,i,n);if(!y)return r("automations.schedule.custom");if(b)return r("automations.schedule.everyDayAt",{time:y})+h;let $=AM(d);if(m==="*"&&l==="*"&&c==="*"&&$==="1-5")return r("automations.schedule.weekdaysAt",{time:y})+h;if(m==="*"&&l==="*"&&c==="*"&&mr($,0,7)){let g=kM(Number($)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:y})+h}if(m==="*"&&mr(l,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(l),time:y})+h;if(mr(l,1,31)&&mr(c,1,12)&&d==="*"&&(m==="*"||mr(m,1970,9999))){let g=CM(Number(c),Number(l),m==="*"?null:Number(m),n);return r("automations.schedule.dateAt",{date:g,time:y})+h}return r("automations.schedule.custom")}function Gr(e,t="Unknown",a,n){if(!e)return t;let r=new Date(e);if(Number.isNaN(r.getTime()))return t;let s=n&&typeof n=="string"?{timeZone:n}:{};try{return r.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...s})}catch{return r.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function s_(e,t){let a=ZN[e]?.labelKey||"automations.state.unknown";return oa(t)(a)}function i_(e){return ZN[e]?.tone||"muted"}function dM(e,t){return Tn(e)&&e?.has_running_run?oa(t)("automations.status.running"):Tn(e)&&e?.has_failed_runs?oa(t)("automations.status.needsReview"):s_(e?.state,t)}function mM(e){return Tn(e)&&e?.has_running_run?"info":Tn(e)&&e?.has_failed_runs?"danger":i_(e?.state)}function fM(e,t){let a=e_[e]?.labelKey||"automations.lastStatus.none";return oa(t)(a)}function pM(e){return e_[e]?.tone||"muted"}function hM(e,t){let a=t_[xd(e)]?.labelKey||"automations.runStatus.unknown";return oa(t)(a)}function vM(e){return t_[xd(e)]?.tone||"muted"}function gM(e,t,a,n){if(!e)return oa(a)("automations.schedule.custom");let r=Gr(e,null,n,t);if(!r)return oa(a)("automations.schedule.custom");let s=t&&typeof t=="string"?` (${t})`:"";return oa(a)("automations.schedule.onceAt",{datetime:r})+s}function yM(e,t,a){return e?.type==="once"?gM(e.at,e.timezone,t,a):e?.type==="schedule"?cM(e.cron,e.timezone||"UTC",t,a):oa(t)("automations.schedule.custom")}function bM(e,t,a){let n=oa(t),r=xM(e.recent_runs,t,a),s=r[0]||null,i=r.find(m=>m.status==="running")||null,o=r.find(m=>m.status==="ok"||m.status==="error")||null,l=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null,d={...e,recent_runs:r,has_running_run:r.some(m=>m.status==="running"),has_failed_runs:r.some(m=>m.status==="error")};return{...d,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:yM(e.source,t,a),state_label:s_(e.state,t),state_tone:i_(e.state),primary_status_label:dM(d,t),primary_status_tone:mM(d),next_run_timestamp:Ih(e.next_run_at),next_run_label:Gr(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:Gr(c,n("automations.date.noRuns"),a),last_status_label:fM(l,t),last_status_tone:pM(l),created_label:Gr(e.created_at,n("automations.date.unknown"),a),latest_run:s,current_run:i,success_rate_label:wM(r,t)}}function xM(e,t,a){let n=oa(t);return Array.isArray(e)?e.map(r=>{let s=xd(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=Ih(i);return{...r,status:s,status_label:hM(s,t),status_tone:vM(s),timestamp:o,timestamp_source:i,fired_label:Gr(i,n("automations.date.unscheduled"),a),submitted_label:Gr(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:Gr(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function xd(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function o_(e){let t=Array.isArray(e)?e:[],a={total:t.length,ok:0,error:0,running:0,unknown:0};for(let n of t){let r=xd(n?.status);Object.prototype.hasOwnProperty.call(a,r)?a[r]+=1:a.unknown+=1}return a}function $M(e){let t=o_(e);return[{key:"ok",tone:"text-emerald-300",count:t.ok},{key:"error",tone:"text-red-300",count:t.error},{key:"running",tone:"text-sky-300",count:t.running},{key:"unknown",tone:"text-iron-400",count:t.unknown}].filter(a=>a.count>0)}function l_(e,t){let a=oa(t),n=o_(e),r=$M(e).map(s=>({...s,text:a(`automations.runs.${s.key}`,{count:s.count})}));return{total:n.total,totalText:a("automations.runs.total",{count:n.total}),chips:r}}function wM(e,t){let a=oa(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function SM(e,t){let a=Tn(e),n=Tn(t);return a!==n?a?-1:1:(zh(e)??Number.MAX_SAFE_INTEGER)-(zh(t)??Number.MAX_SAFE_INTEGER)}function Ih(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function Tn(e){return e?.state==="active"||e?.state==="scheduled"}function NM(e){return["paused","disabled","inactive"].includes(e?.state)}function _M(e){return e?.state==="completed"}function zh(e){return e?.next_run_timestamp??Ih(e?.next_run_at)}function Hh(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function RM(e,t,a){return!mr(e,0,23)||!mr(t,0,59)?null:Hh(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function kM(e,t){return Hh(t,{weekday:"long"},new Date(2001,0,7+e))}function CM(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return Hh(n,r,new Date(a??2e3,e-1,t))}function EM(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&WN(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&WN(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function WN(e){return/^0+$/.test(e)}function mr(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function TM(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function AM(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}var DM=8;function Kh(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function $d({runs:e=[]}){let t=R(),a=Array.isArray(e)?e:[],n=a.slice(0,DM);if(!n.length)return u`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;let r=a.length-n.length,s=`+${Math.min(r,999)}`;return u`
    <div
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
    >
      ${n.map(i=>u`
        <span
          key=${Kh(i)}
          title=${`${i.status_label} \xB7 ${i.fired_label}`}
          className=${Y("h-3 w-3 rounded-full border",i.status==="ok"&&"border-emerald-300/50 bg-emerald-400",i.status==="error"&&"border-red-300/50 bg-red-400",i.status==="running"&&"border-sky-300/60 bg-sky-400",i.status==="unknown"&&"border-iron-500 bg-iron-600")}
        />
      `)}
      ${r>0&&u`<span
        className="ml-0.5 font-mono text-[11px] text-iron-400"
        title=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
      >
        ${s}
      </span>`}
    </div>
  `}function wd({runs:e=[],className:t=""}){let a=R(),n=l_(e,a);return n.total?u`
    <div className=${Y("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]",t)}>
      <span className="text-iron-300">${n.totalText}</span>
      ${n.chips.map(r=>u`<span key=${r.key} className=${r.tone}>${r.text}</span>`)}
    </div>
  `:u`<span className=${Y("text-[11px] text-iron-400",t)}>
      ${a("automations.table.noRuns")}
    </span>`}function u_({run:e,onOpenRun:t,onOpenLogs:a}){let n=R(),r=!!e.chat_path,s=ad({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return u`
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
  `}function Sd({label:e,value:t,tone:a}){return u`
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
  `}function c_({automation:e,isMutating:t=!1,onPauseAutomation:a,onResumeAutomation:n,onDeleteAutomation:r}){let s=R(),i=ve();if(!e)return u`
      <${I} className="p-4 sm:p-5">
        <${$e}
          boxed=${!1}
          title=${s("automations.detail.emptyTitle")}
          description=${s("automations.detail.emptyDescription")}
        />
      <//>
    `;let o=e.current_run,l=e.state==="paused",c=e.state==="active"||e.state==="scheduled",m=`${s(l?"missions.action.resume":"missions.action.pause")}: ${e.display_name}`,f=()=>{if(l){n?.(e.automation_id);return}c&&a?.(e.automation_id)},h=`${s("common.delete")}: ${e.display_name}`,b=()=>{window.confirm(h)&&r?.(e.automation_id)};return u`
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
              onClick=${b}
            >
              <${M} name="trash" className="h-4 w-4" />
            <//>
          </div>
        </div>
      </div>

      <div className="space-y-5 p-4 sm:p-5">
        <div className="grid gap-3 sm:grid-cols-2">
          <${Sd} label=${s("automations.detail.schedule")} value=${e.schedule_label} />
          <${Sd}
            label=${s("automations.detail.successRate")}
            value=${e.success_rate_label}
            tone=${e.has_failed_runs?"danger":"success"}
          />
          <${Sd} label=${s("automations.detail.lastCompleted")} value=${e.last_run_label} />
          <${Sd}
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
              <${$d} runs=${e.recent_runs} />
              <${wd} runs=${e.recent_runs} />
            </div>
          </div>

          ${e.recent_runs.length?u`
                <div>
                  ${e.recent_runs.map(y=>u`
                    <${u_}
                      key=${Kh(y)}
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
  `}var MM=["automations.empty.example1","automations.empty.example2","automations.empty.example3"];function OM({promptKey:e}){let t=R(),a=t(e),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>()=>clearTimeout(s.current),[]),u`
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
  `}function d_(){let e=R(),t=ve();return u`
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
            ${MM.map(a=>u`<${OM} key=${a} promptKey=${a} />`)}
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
  `}function m_({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,isMutating:s,selectedAutomationId:i,onSelectAutomation:o,onPauseAutomation:l,onResumeAutomation:c,onDeleteAutomation:d}){let m=R(),f=n_(e,t),h=e.length>0,b=f.find(y=>y.automation_id===i)||f[0]||null;return u`
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
              ${qh.map(y=>u`
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
                      ${f.map(y=>{let $=y.automation_id===b?.automation_id;return u`
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
                                <${$d} runs=${y.recent_runs} />
                                <${wd} runs=${y.recent_runs} />
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

              <${c_}
                automation=${b}
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
            `:u`<${d_} />`}
    </div>
  `}function f_({summary:e,activeFilter:t,onSelectFilter:a}){let n=R(),r=[{key:"scheduled",label:n("automations.summary.scheduled"),value:e?.scheduled??0,tone:"muted",detail:n("automations.summary.scheduledDetail"),filter:"all"},{key:"active",label:n("automations.summary.active"),value:e?.active??0,tone:"signal",detail:n("automations.summary.activeDetail"),filter:"active"},{key:"running",label:n("automations.summary.running"),value:e?.running??0,tone:"info",detail:n("automations.summary.runningDetail"),filter:"running"},{key:"failures",label:n("automations.summary.failures"),value:e?.failures??0,tone:(e?.failures??0)>0?"danger":"success",detail:n("automations.summary.failuresDetail"),filter:(e?.failures??0)>0?"failures":null},{key:"nextRun",label:n("automations.summary.nextRun"),value:e?.nextRun||n("automations.summary.none"),tone:"info",detail:n("automations.summary.nextRunDetail"),valueClassName:"text-lg md:text-xl"}];return u`
    <${I} className="p-4 sm:p-5">
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
              className=${Y(c,"transition-colors hover:border-white/20 hover:bg-white/[0.05]","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",o&&"border-[var(--v2-accent)]/60 bg-[var(--v2-accent-soft)]/30")}
            >
              ${l}
            </button>
          `:u`<div key=${s.key} className=${c}>${l}</div>`})}
      </div>
    <//>
  `}function LM(e){return e==="active"||e==="scheduled"}function PM(e){return Number.isFinite(e)?e:null}function p_(e,t=Date.now()){let a=Array.isArray(e)?e:[],n=null;for(let r of a){if(!r||(r.has_running_run&&(n=n==null?5e3:Math.min(n,5e3)),!LM(r.state)))continue;let s=PM(r.next_run_timestamp);if(s==null)continue;let i=s-t,o=i<=0?2e3:i<3e4?Math.max(1e3,i+1200):null;o!=null&&(n=n==null?o:Math.min(n,o))}return n}var jM=50,FM=25;function h_(e=!1){let{t,lang:a}=$l(),n=W(),r=H({queryKey:["automations",{includeCompleted:e}],queryFn:()=>Z0({limit:jM,runLimit:FM,includeCompleted:e}),refetchInterval:3e4,refetchIntervalInBackground:!1}),s=p.default.useMemo(()=>a_(r.data,t,a),[r.data,t,a]),i=p.default.useMemo(()=>r_(s),[s]),o=p.default.useMemo(()=>p_(s),[s]);p.default.useEffect(()=>{if(o==null)return;let h=setTimeout(()=>{r.refetch()},o);return()=>clearTimeout(h)},[o,r.refetch]);let l=r.data?.scheduler_enabled!==!1,c=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["automations"]})},[n]),d=G({mutationFn:h=>e$({automationId:h}),onSuccess:c}),m=G({mutationFn:h=>t$({automationId:h}),onSuccess:c}),f=G({mutationFn:h=>a$({automationId:h}),onSuccess:c});return{automations:s,summary:i,schedulerEnabled:l,isLoading:r.isLoading,isRefreshing:r.isFetching,isMutating:d.isPending||m.isPending||f.isPending,error:r.error||null,actionError:d.error||m.error||f.error||null,pauseAutomation:d.mutate,resumeAutomation:m.mutate,deleteAutomation:f.mutate,refetch:r.refetch}}var v_=["outbound-delivery","preferences"],g_=["outbound-delivery","targets"];function y_(){let e=W(),t=H({queryKey:v_,queryFn:i$}),a=H({queryKey:g_,queryFn:o$}),n=G({mutationFn:({finalReplyTargetId:i})=>l$({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(v_,i),e.invalidateQueries({queryKey:g_})}}),r=p.default.useMemo(()=>a.data?.targets??[],[a.data]),s=p.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function b_(){let e=R(),[t,a]=p.default.useState("all"),[n,r]=p.default.useState(null),i=h_(t==="completed"),o=y_(),[l,c]=p.default.useState(!1),d=p.default.useRef(null);p.default.useEffect(()=>()=>clearTimeout(d.current),[]);let m=p.default.useCallback(()=>{c(!0),clearTimeout(d.current),d.current=setTimeout(()=>c(!1),1e3),i.refetch()},[i.refetch]),f=i.isRefreshing||l,h=i.error&&!i.isLoading&&i.automations.length===0;return p.default.useEffect(()=>{if(!i.automations.length){r(null);return}i.automations.some(y=>y.automation_id===n)||r(i.automations[0].automation_id)},[i.automations,n]),u`
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
                <${XN} deliveryState=${o} />

                ${i.isLoading?u`
                      <div className="space-y-4">
                        ${[1,2,3].map(b=>u`<div
                              key=${b}
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
  `:null}var S_="/api/webchat/v2/channels/slack/setup";function N_(){return V(S_)}function __(e){let t={installation_id:String(e.installation_id||"").trim(),team_id:String(e.team_id||"").trim(),api_app_id:String(e.api_app_id||"").trim(),user_id:w_(e.user_id),shared_subject_user_id:w_(e.shared_subject_user_id)},a=String(e.bot_token||"").trim(),n=String(e.signing_secret||"").trim();return a&&(t.bot_token=a),n&&(t.signing_secret=n),V(S_,{method:"PUT",body:JSON.stringify(t)})}function Qh(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function w_(e){let t=String(e||"").trim();return t||null}var R_="/api/webchat/v2/channels/slack/allowed",BM="/api/webchat/v2/channels/slack/subjects";function k_(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function C_(){return V(R_)}function E_(){return V(BM)}function T_(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return V(R_,{method:"PUT",body:JSON.stringify(n)})}function A_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var D_=["slack-allowed-channels"];function O_({action:e}){let t=R(),a=W(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState([]),c=qM(e,t),d=H({queryKey:D_,queryFn:C_}),m=H({queryKey:["slack-routable-subjects"],queryFn:E_}),f=m.data?.subjects||[],h=M_(f),b=m.isSuccess||m.isError,y=f.length>0;p.default.useEffect(()=>{d.data&&l(Vh(d.data.channels||[]))},[d.data]);let $=G({mutationFn:({channels:k})=>T_(k),onSuccess:k=>{l(Vh(k.channels||[])),a.invalidateQueries({queryKey:D_}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let k=n.trim();!k||!m.isSuccess||(l(N=>Vh([...N,{channel_id:k,subject_user_id:s}])),r(""))},v=k=>{l(N=>N.filter(C=>C.channel_id!==k))},x=(k,N)=>{l(C=>C.map(P=>P.channel_id===k?{...P,subject_user_id:N}:P))},w=()=>{$.mutate({channels:zM(o)})},S=m.isError&&o.some(k=>!k.subject_user_id);return u`
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
          onChange=${k=>r(k.target.value)}
          onKeyDown=${k=>k.key==="Enter"&&g()}
          placeholder=${c.inputPlaceholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <select
          value=${s}
          onChange=${k=>i(k.target.value)}
          disabled=${!y}
          className="h-9 min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
        >
          ${!y&&u`<option value="">${c.noSubjectsLabel}</option>`}
          ${y&&u`<option value="">${c.autoSubjectLabel}</option>`}
          ${h.map(k=>u`
              <option key=${k.subject_user_id} value=${k.subject_user_id}>
                ${k.display_name}
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
        ${o.map(k=>u`
            <label
              key=${k.channel_id}
              className="flex min-h-10 items-center justify-between gap-3 border-t border-white/[0.05] px-3 first:border-t-0"
            >
              <span className="min-w-0">
                <span className="block truncate font-mono text-xs text-iron-200">
                  ${k.channel_id}
                </span>
              </span>
              <div className="flex shrink-0 items-center gap-2">
                ${y?u`
                    <select
                      value=${k.subject_user_id}
                      onChange=${N=>x(k.channel_id,N.target.value)}
                      className="h-8 rounded-md border border-white/10 bg-white/[0.04] px-2 text-xs text-iron-100 outline-none focus:border-signal/45"
                    >
                      <option value="">${c.autoSubjectLabel}</option>
                      ${M_(f,k).map(N=>u`
                          <option key=${N.subject_user_id} value=${N.subject_user_id}>
                            ${N.display_name}
                          </option>
                        `)}
                    </select>
                  `:u`<span className="max-w-40 truncate text-xs text-iron-500">
                    ${k.subject_user_id?k.subject_display_name||k.subject_user_id:c.autoSubjectLabel}
                  </span>`}
                <input
                  type="checkbox"
                  checked=${!0}
                  aria-label=${c.allowLabel(k.channel_id)}
                  onChange=${()=>v(k.channel_id)}
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
          disabled=${!d.isSuccess||!b||$.isPending||S}
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
  `}function M_(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function Vh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return k_(Array.from(t.keys())).map(a=>t.get(a))}function zM(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function qM(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var Gh=["slack-setup"],Yr={installationId:{body:"Local IronClaw name for this Slack install. Choose one and keep it stable.",example:"Example: local-slack"},teamId:{body:"Slack workspace/team ID from the workspace that installed the app.",example:"Example: T0123456789"},appId:{body:"Slack app Basic Information > App Credentials.",example:"Example: A0123456789"},botUser:{body:"Optional Reborn user. Blank uses the current WebUI operator.",example:"Example: user:operator"},sharedSubject:{body:"Optional default team agent for shared channel turns. Usually blank.",example:"Example: user:slack-shared"},botToken:{body:"Slack app OAuth & Permissions > Bot User OAuth Token.",example:"Example: xoxb-..."},signingSecret:{body:"Slack app Basic Information > App Credentials > Signing Secret.",example:""}};function U_({action:e}){let t=H({queryKey:Gh,queryFn:N_}),a=t.data?.configured===!0;return u`
    <div className="space-y-3">
      <${IM} action=${e} setupQuery=${t} />
      ${a&&u`<${O_} action=${e} />`}
    </div>
  `}function IM({action:e,setupQuery:t}){let a=W(),[n,r]=p.default.useState(HM()),s=p.default.useRef(!1),i=p.default.useRef(!1),o=t.data,l=KM(e);p.default.useEffect(()=>{!o||s.current||i.current||(r(L_(o)),s.current=!0)},[o]);let c=G({mutationFn:__,onSuccess:h=>{i.current=!1,r(L_(h)),s.current=!0,a.setQueryData(Gh,h),a.invalidateQueries({queryKey:Gh}),a.invalidateQueries({queryKey:["slack-allowed-channels"]}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["extensions"]})}}),d=h=>b=>{i.current=!0,r(y=>({...y,[h]:b.target.value}))},m=()=>c.mutate(n),f=n.installation_id.trim()&&n.team_id.trim()&&n.api_app_id.trim()&&(o?.bot_token_configured||n.bot_token.trim())&&(o?.signing_secret_configured||n.signing_secret.trim());return u`
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
        ${yl("Installation ID",n.installation_id,d("installation_id"),"",Yr.installationId)}
        ${yl("Team ID",n.team_id,d("team_id"),"",Yr.teamId)}
        ${yl("App ID",n.api_app_id,d("api_app_id"),"",Yr.appId)}
        ${yl("Bot user",n.user_id,d("user_id"),"default operator",Yr.botUser)}
        ${yl("Shared subject",n.shared_subject_user_id,d("shared_subject_user_id"),"optional",Yr.sharedSubject)}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        ${P_("Bot token",n.bot_token,d("bot_token"),o?.bot_token_configured,Yr.botToken)}
        ${P_("Signing secret",n.signing_secret,d("signing_secret"),o?.signing_secret_configured,Yr.signingSecret)}
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
          ${Qh(t.error,l.errorMessage)}
        </p>`}
        ${c.isError&&u`<p className="text-xs text-red-300">
          ${Qh(c.error,l.errorMessage)}
        </p>`}
        ${c.isSuccess&&u`<p className="text-xs text-emerald-300">${l.successMessage}</p>`}
      </div>
    </div>
  `}function L_(e){return{installation_id:e.installation_id||"",team_id:e.team_id||"",api_app_id:e.api_app_id||"",user_id:e.user_id||"",shared_subject_user_id:e.shared_subject_user_id||"",bot_token:"",signing_secret:""}}function HM(){return{installation_id:"",team_id:"",api_app_id:"",user_id:"",shared_subject_user_id:"",bot_token:"",signing_secret:""}}function yl(e,t,a,n="",r=null){return u`
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-iron-500">${e}</span>
      <input
        type="text"
        value=${t}
        onChange=${a}
        placeholder=${n}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <${j_} help=${r} />
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
      <${j_} help=${r} />
    </label>
  `}function j_({help:e}){return e?u`
    <p className="mt-1.5 min-h-8 text-[11px] leading-4 text-iron-400">
      <span className="block">${e.body}</span>
      ${e.example&&u`<span className="mt-0.5 block font-mono text-iron-300">${e.example}</span>`}
    </p>
  `:null}function KM(e){return{title:"Slack setup",instructions:e?.instructions||"Configure the Slack app before assigning channels.",submitLabel:"Save setup",successMessage:"Slack setup saved.",errorMessage:"Slack setup update failed."}}var Yh={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function Jr(e){return e==="wasm_channel"||e==="channel"}var F_={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},B_={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function z_(e){let t=q_(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||Jr(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function q_(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function Jh(e){let t=q_(e);return t==="active"||t==="ready"}function I_({extension:e,secrets:t=[],fields:a=[]}={}){return Jh(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var H_="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",K_="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",Q_="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",V_="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",G_="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",QM="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function Y_(e){return e.package_ref?.id||""}function VM({actions:e,isBusy:t}){let a=R(),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>{if(!n)return;let i=o=>{s.current&&!s.current.contains(o.target)&&r(!1)};return document.addEventListener("mousedown",i),()=>document.removeEventListener("mousedown",i)},[n]),u`
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
                <${M} name=${i.icon||"settings"} className="h-3.5 w-3.5" />
                ${i.label}
              </button>
            `)}
        </div>
      `}
    </div>
  `}function J_({items:e}){return!e||e.length===0?null:u`
    <div className="mt-3 flex flex-wrap gap-1">
      ${e.map(t=>u`<span key=${t} className=${QM}>${t}</span>`)}
    </div>
  `}function Ni({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=R(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=F_[i]||"muted",l=s(`extensions.state.${i}`)||B_[i]||i,c=s(`extensions.kind.${e.kind}`)||Yh[e.kind]||e.kind,d=e.display_name||Y_(e),m=!!e.package_ref,f=e.tools||[],[h,b]=p.default.useState(!1),$=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],x=[],w=z_(e);w==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):w==="activate"&&v.push({id:"activate",label:s("extensions.activate"),run:()=>t(g)}),m&&(e.needs_setup||e.has_auth)&&w!=="configure"&&x.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)}),m&&Jr(e.kind)&&(i==="setup_required"||i==="failed")&&x.push({id:"setup",label:s("extensions.setup"),icon:"settings",run:()=>a(g)}),m&&Jr(e.kind)&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&x.push({id:"reconfigure",label:s("extensions.reconfigure"),icon:"settings",run:()=>a(g)}),m&&x.push({id:"remove",label:s("common.remove"),icon:"trash",danger:!0,run:()=>n(g)});let S=v[0];return u`
    <div className=${H_}>
      <div className="flex items-start gap-2">
        <${q} tone=${o} label=${l} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${x.length>0&&u`<${VM} actions=${x} isBusy=${r} />`}
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
                onClick=${()=>b(k=>!k)}
                className=${G_}
              >
                <${M} name="layers" className="h-3.5 w-3.5" />
                <span>${f.length===1?s("extensions.oneCapability"):s("extensions.pluralCapabilities",{count:f.length})}</span>
                <${M}
                  name="chevron"
                  className=${["h-3 w-3",h?"rotate-180":""].join(" ")}
                />
              </button>
            `:u`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">${s("extensions.noCapabilities")}</span>`}
        <span className="flex-1"></span>
        ${S&&u`
          <${A} variant="secondary" size="sm" onClick=${S.run} disabled=${r}>
            ${S.label}
          <//>
        `}
      </div>

      ${h&&u`<${J_} items=${f} />`}
    </div>
  `}function Xr({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=R(),s=r(`extensions.kind.${e.kind}`)||Yh[e.kind]||e.kind,i=e.display_name||Y_(e),o=!!(e.package_ref&&t),l=e.keywords||[],[c,d]=p.default.useState(!1);return u`
    <div className=${H_}>
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
            onClick=${()=>t({packageRef:e.package_ref,displayName:i})}
            disabled=${a}
          >
            <${M} name="plus" className="mr-1.5 h-3.5 w-3.5" />
            ${r("extensions.install")}
          <//>
        `}
      </div>

      ${c&&u`<${J_} items=${l} />`}
    </div>
  `}function X_(){return V("/api/webchat/v2/extensions")}function W_(){return V("/api/webchat/v2/extensions/registry")}function Z_(e){return V("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function eR(e){return V(`/api/webchat/v2/extensions/${encodeURIComponent(bl(e))}/activate`,{method:"POST"})}function tR(e){return V(`/api/webchat/v2/extensions/${encodeURIComponent(bl(e))}/remove`,{method:"POST"})}function aR(e){return V(`/api/webchat/v2/extensions/${encodeURIComponent(bl(e))}/setup`)}function nR(e,t,a){return v$(bl(e),{action:"submit",payload:{secrets:t,fields:a}})}function rR(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return V(`/api/webchat/v2/extensions/${encodeURIComponent(bl(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function sR(){return Promise.resolve({requests:[]})}function iR(){return Promise.resolve({success:!1,message:"Pairing requires a v2 pairing endpoint."})}function bl(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var GM=2e3,YM=10*60*1e3;function _i(e){return e?.package_ref?.id||null}function Xh(e){return e?.display_name||_i(e)||""}function oR(e,t,a){return _i(t)||`${e}:${Xh(t)||"unknown"}:${a}`}function JM(e,t){return e.installed!==t.installed?e.installed?-1:1:Xh(e.entry||e.extension).localeCompare(Xh(t.entry||t.extension))}function lR(){let e=R(),t=W(),a=H({queryKey:["gateway-status-extensions"],queryFn:si,staleTime:1e4}),n=H({queryKey:["extensions"],queryFn:X_}),r=H({queryKey:["extension-registry"],queryFn:W_}),s=H({queryKey:["connectable-channels"],queryFn:Xc}),i=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["gateway-status-extensions"]}),t.invalidateQueries({queryKey:["connectable-channels"]})},[t]),[o,l]=p.default.useState(null),c=p.default.useCallback(()=>l(null),[]),d=G({mutationFn:({packageRef:T})=>Z_(T),onSuccess:(T,{displayName:K})=>{T.success?(l({type:"success",message:T.message||T.instructions||e("extensions.installedSuccess",{name:K||e("extensions.defaultName")})}),T.auth_url&&window.open(T.auth_url,"_blank","noopener,noreferrer")):l({type:"error",message:T.message||e("extensions.installFailed")}),i()},onError:T=>{l({type:"error",message:T.message}),i()}}),m=G({mutationFn:({packageRef:T})=>eR(T),onSuccess:(T,{displayName:K})=>{T.success?(l({type:"success",message:T.message||T.instructions||e("extensions.activatedSuccess",{name:K||e("extensions.defaultName")})}),T.auth_url&&window.open(T.auth_url,"_blank","noopener,noreferrer")):T.auth_url?(window.open(T.auth_url,"_blank","noopener,noreferrer"),l({type:"info",message:e("extensions.openingAuth")})):T.awaiting_token?l({type:"info",message:e("extensions.configurationRequired")}):l({type:"error",message:T.message||e("extensions.activationFailed")}),i()},onError:T=>{l({type:"error",message:T.message})}}),f=G({mutationFn:({packageRef:T})=>tR(T),onSuccess:(T,{displayName:K})=>{T.success?l({type:"success",message:e("extensions.removedSuccess",{name:K||e("extensions.defaultName")})}):l({type:"error",message:T.message||e("extensions.removeFailed")}),i()},onError:T=>{l({type:"error",message:T.message})}}),h=a.data||{},b=n.data?.extensions||[],y=r.data?.entries||[],$=s.data?.channels||[],g=new Map(b.map(T=>[_i(T),T]).filter(([T])=>!!T)),v=new Set(y.map(T=>_i(T)).filter(Boolean)),x=[...y.map((T,K)=>{let ee=_i(T),ne=ee&&g.get(ee)||null;return{id:oR("registry",T,K),installed:!!(ne||T.installed),entry:T,extension:ne}}),...b.filter(T=>{let K=_i(T);return!K||!v.has(K)}).map((T,K)=>({id:oR("installed",T,K),installed:!0,entry:null,extension:T}))].sort(JM),w=T=>Jr(T.kind),S=b.filter(w),k=b.filter(T=>T.kind==="mcp_server"),N=b.filter(T=>!w(T)&&T.kind!=="mcp_server"),C=y.filter(T=>w(T)&&!T.installed),P=y.filter(T=>T.kind==="mcp_server"&&!T.installed),L=y.filter(T=>T.kind!=="mcp_server"&&!w(T)&&!T.installed),U=n.isLoading||r.isLoading,F=d.isPending||m.isPending||f.isPending;return{status:h,extensions:b,channels:S,mcpServers:k,tools:N,channelRegistry:C,mcpRegistry:P,toolRegistry:L,registry:y,catalogEntries:x,connectableChannels:$,isLoading:U,isBusy:F,actionResult:o,clearResult:c,install:d.mutate,activate:m.mutate,remove:f.mutate,invalidate:i}}function uR(e){let t=H({queryKey:["extension-setup",e?.id||e],queryFn:()=>aR(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function cR(e,t){let a=W(),n=e?.id||e;return G({mutationFn:({secrets:r,fields:s})=>nR(e,r,s),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function dR(e){let t=W(),a=e?.id||e,n=p.default.useRef(null),r=p.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=p.default.useCallback(()=>{let l=t.getQueryData(["extension-setup",a]);if(l?.secrets?.length>0&&l.secrets.every(f=>f.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(f=>f.package_ref?.id===a),m=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return m==="active"||m==="ready"},[a,t]),o=p.default.useCallback(l=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||l&&l.closed||Date.now()-c>YM)&&(r(),s())},GM)},[r,s,i]);return p.default.useEffect(()=>r,[r]),G({mutationFn:({secret:l,popup:c})=>rR(e,l).then(d=>({res:d,popup:c})),onSuccess:({res:l,popup:c})=>{let d=c;l.authorization_url&&c&&!c.closed?c.location.href=l.authorization_url:l.authorization_url?d=window.open(l.authorization_url,"_blank","noopener,noreferrer"):c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(l,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function mR(e,t={}){let a=H({queryKey:["pairing",e],queryFn:()=>sR(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=W(),r=G({mutationFn:({code:s})=>iR(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function fR(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var XM={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function pR({channel:e,redeemFn:t,i18nKeys:a=XM,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=R(),o=typeof t=="function",l=mR(e,{enabled:!o}),c=W(),[d,m]=p.default.useState(""),f=WM(i,a,r),h=G({mutationFn:({code:S})=>t(e,S),onSuccess:()=>{m("");for(let S of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:S})}}),b=p.default.useCallback(S=>l.approve({code:S}),[l.approve]),y=p.default.useCallback(()=>{let S=d.trim();S&&(o?h.mutate({code:S}):(l.approve({code:S}),m("")))},[o,d,l.approve,h]),$=o?[]:l.requests,g=o?!1:l.isLoading,v=o?h.isPending:l.isApproving,x=o?h.isSuccess?h.data:null:l.result,w=o?h.isError?h.error:null:l.error;return g?u`
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

      ${x?.success&&u`<p className="mb-3 text-xs text-emerald-300">
        ${x.message||f.success}
      </p>`}
      ${x&&!x.success&&u`<p className="mb-3 text-xs text-red-300">
        ${x.message||f.error}
      </p>`}
      ${w&&u`<p className="mb-3 text-xs text-red-300">
        ${fR(w,f.error)}
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
                    onClick=${()=>b(S.code||S.id)}
                    disabled=${v}
                  >
                    ${f.action}
                  <//>
                </div>
              `)}
            </div>
          `:s&&u`<p className="text-xs text-iron-300">${i(a.empty)}</p>`}
    </div>
  `}function WM(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function Nd(e){return e.package_ref?.id||""}function hR(e){return Nd(e)==="slack"}function gR(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function yR(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function ZM(e){let t=e||[],a=[t.find(gR),t.find(yR)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function vR({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>gR(r)?u`<${U_} action=${r.action} />`:yR(r)?u`<${Kc} action=${r.action} />`:null).filter(Boolean);return n.length>0?u`<div className="space-y-3">${n}</div>`:null}function bR({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:l}){let c=R(),d=t||[],m=e.enabled_channels||[],f=ZM(a),h=d.some(hR),b=f.length>0&&!h;return u`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          ${c("channels.builtIn")}
        </h3>
        <${Ri}
          name=${c("channels.webGateway")}
          description=${c("channels.webGatewayDesc")}
          enabled=${!0}
          detail=${"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)}
        />
        <${Ri}
          name=${c("channels.httpWebhook")}
          description=${c("channels.httpWebhookDesc")}
          enabled=${m.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${Ri}
          name=${c("channels.cli")}
          description=${c("channels.cliDesc")}
          enabled=${m.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${Ri}
          name=${c("channels.repl")}
          description=${c("channels.replDesc")}
          enabled=${m.includes("repl")}
          detail="ironclaw run --repl"
        />
        ${b&&u`
          <${Ri}
            name=${c("channels.slack")}
            description=${c("channels.slackDesc")}
            enabled=${!1}
            statusLabel=${c("channels.setup")}
            statusTone="muted"
            detail=${c("channels.slackDetail")}
          >
            <${vR}
              slackConnectActions=${f}
            />
          </${Ri}>
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
                <div key=${Nd(y)} className="flex flex-col gap-3">
                  <${Ni}
                    ext=${y}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${l}
                  />
                  ${hR(y)&&u`<${vR}
                    slackConnectActions=${f}
                  />`}
                  ${(y.onboarding_state==="pairing_required"||y.onboarding_state==="pairing")&&u` <${pR} channel=${Nd(y)} /> `}
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
                  key=${Nd(y)}
                  entry=${y}
                  onInstall=${o}
                  isBusy=${l}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function Ri({name:e,description:t,enabled:a,detail:n,children:r,statusLabel:s=a?"on":"off",statusTone:i=a?"success":"muted"}){return u`
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
  `}function xR({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=R(),s=e?.displayName||e?.packageRef?.id||r("extensions.defaultName"),{secrets:i=[],fields:o=[],onboarding:l,isLoading:c,error:d}=uR(e?.packageRef),[m,f]=p.default.useState({}),[h,b]=p.default.useState({}),y=dR(e?.packageRef),$=cR(e?.packageRef,N=>{N.success!==!1&&(n&&n(N),a())}),g=p.default.useCallback(()=>{let N={};for(let[C,P]of Object.entries(m)){let L=(P||"").trim();L&&(N[C]=L)}$.mutate({secrets:N,fields:h})},[m,h,$]),v=p.default.useCallback(N=>{let C=window.open("about:blank","_blank","width=600,height=600");C&&(C.opener=null),y.mutate({secret:N,popup:C})},[y]),w=i.filter(N=>(N.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,S=Jh(e),k=I_({extension:e,secrets:i,fields:o});return c?u`
      <${_d} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(N=>u`<div
                key=${N}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?u`
      <${_d} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?u`
      <${_d} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")}
        </p>
      <//>
    `:u`
    <${_d} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
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
          ${r("extensions.getCredentials")}
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
                        ${N.provided?r("extensions.authConfigured"):r("extensions.authPopup")}
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
                placeholder=${N.provided?r("extensions.keepSecretPlaceholder"):""}
                value=${m[N.name]||""}
                onChange=${C=>f(P=>({...P,[N.name]:C.target.value}))}
                onKeyDown=${C=>C.key==="Enter"&&g()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
              ${N.auto_generate&&!N.provided&&u`
                <p className="mt-1 text-xs text-iron-700">
                  ${r("extensions.autoGenerated")}
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
                onChange=${C=>b(P=>({...P,[N.name]:C.target.value}))}
                onKeyDown=${C=>C.key==="Enter"&&g()}
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
        <${A} variant="ghost" onClick=${a}>${r("common.cancel")}<//>
        ${k&&u`
        <${A}
          variant="primary"
          onClick=${()=>t?.(e)}
        >
          ${r("extensions.activate")}
        <//>
        `}
        ${w&&u`
        <${A}
          variant=${k?"secondary":"primary"}
          onClick=${g}
          disabled=${$.isPending}
        >
          ${$.isPending?r("common.saving"):r("common.save")}
        <//>
        `}
      </div>
    <//>
  `}function _d({onClose:e,title:t,children:a}){return p.default.useEffect(()=>{let n=r=>{r.key==="Escape"&&e()};return window.addEventListener("keydown",n),()=>window.removeEventListener("keydown",n)},[e]),u`
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
  `}function $R(e){return e.package_ref?.id||""}function wR({mcpServers:e,mcpRegistry:t,onActivate:a,onConfigure:n,onRemove:r,onInstall:s,isBusy:i}){let o=R();return e.length===0&&t.length===0?u`
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
                <${Ni}
                  key=${$R(l)}
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
                  key=${$R(l)}
                  entry=${l}
                  onInstall=${s}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function eO(e){return e?.package_ref?.id||""}function tO(e){return e.entry||e.extension||{}}function SR({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=R(),[o,l]=p.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(y=>{let $=tO(y);return($.display_name||eO($)).toLowerCase().includes(c)||($.description||"").toLowerCase().includes(c)||($.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,m=d.filter(y=>y.installed&&y.extension),f=d.filter(y=>y.installed&&!y.extension&&y.entry),h=m.length+f.length,b=d.filter(y=>!y.installed&&y.entry);return e.length===0?u`
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
                      <${Ni}
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

              ${b.length>0&&u`
                <h3
                  className=${["mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal",h>0?"mt-6":""].join(" ")}
                >
                  ${i("ext.registry.availableTitle")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${b.map(y=>u`
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
  `}function Wh(){let{tab:e="registry"}=it(),[t,a]=p.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:l,connectableChannels:c,isLoading:d,isBusy:m,actionResult:f,clearResult:h,install:b,activate:y,remove:$,invalidate:g}=lR(),v=p.default.useCallback(N=>a(N),[]),x=p.default.useCallback(()=>a(null),[]),w=p.default.useCallback(()=>g(),[g]),S=p.default.useCallback(N=>{N&&(y(N),a(null))},[y]);if(d)return u`
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
    `;if(e==="installed")return u`<${ot} to="/extensions/registry" replace />`;let k={channels:u`<${bR}
      status=${n}
      channels=${r}
      connectableChannels=${c}
      channelRegistry=${i}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${b}
      isBusy=${m}
    />`,mcp:u`<${wR}
      mcpServers=${s}
      mcpRegistry=${o}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${b}
      isBusy=${m}
    />`,registry:u`<${SR}
      catalogEntries=${l}
      onInstall=${b}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      isBusy=${m}
    />`};return k[e]?u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <${$_} result=${f} onDismiss=${h} />
          ${k[e]}
        </div>
      </div>

      ${t&&u`
        <${xR}
          extension=${t}
          onActivate=${S}
          onClose=${x}
          onSaved=${w}
        />
      `}
    </div>
  `:u`<${ot} to="/extensions/registry" replace />`}var NR=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],_R=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],RR=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],Zh=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function kR(e){return String(e||"").trim().toLowerCase()}function CR(e){if(e==null)return"";if(Array.isArray(e))return e.map(CR).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function tt(e,t){let a=kR(e);return a?t.map(CR).join(" ").toLowerCase().includes(a):!0}function ki(e,t,a,n){let r=kR(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(l=>tt(r,[i,l.key,l.labelKey?n(l.labelKey):l.label,l.descKey?n(l.descKey):l.description,t[l.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function aO({visible:e}){let t=R();return e?u`
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      ${t("tools.saved")}
    </span>
  `:null}function nO({checked:e,onChange:t,label:a}){return u`
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
  `}function rO({field:e,value:t,onSave:a,isSaved:n}){let r=R(),[s,i]=p.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",l=e.descKey?r(e.descKey):e.description||"";p.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=p.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let m=parseInt(d,10);isNaN(m)||a(e.key,m)}else if(e.type==="float"){let m=parseFloat(d);isNaN(m)||a(e.key,m)}else a(e.key,d)},[e.key,e.type,a]);return u`
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">${o}</div>
        ${l&&u`<div className="mt-1 text-xs leading-5 text-iron-300">${l}</div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${e.type==="boolean"?u`
              <${nO}
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
        <${aO} visible=${n} />
      </div>
    </div>
  `}function Ci({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=R(),o=t?i(t):e||"";return u`
    <${ae} className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${o}</h3>
      <div>
        ${a.map(l=>u`
              <${rO}
                key=${l.key}
                field=${l}
                value=${n[l.key]}
                onSave=${r}
                isSaved=${s[l.key]}
              />
            `)}
      </div>
    <//>
  `}function Rt({query:e}){let t=R();return u`
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
  `}function ER({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=R();if(n)return u`<${sO} />`;let i=ki(_R,e,r,s);return i.length===0?u`<${Rt} query=${r} />`:u`
    <div className="space-y-5">
      ${i.map(o=>u`
            <${Ci}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function sO(){return u`
    <div className="space-y-5">
      ${[1,2,3].map(e=>u`
            <${ae} key=${e} padding="md">
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
  `}function TR(){let e=H({queryKey:["gateway-status-settings"],queryFn:si,staleTime:1e4}),t=H({queryKey:["extensions"],queryFn:cw}),a=H({queryKey:["extension-registry"],queryFn:dw}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(m=>m.kind==="wasm_channel"||m.kind==="channel"),o=s.filter(m=>(m.kind==="wasm_channel"||m.kind==="channel")&&!m.installed),l=r.filter(m=>m.kind==="mcp_server"),c=s.filter(m=>m.kind==="mcp_server"&&!m.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:l,mcpRegistry:c,extensions:r,isLoading:d}}function iO({name:e,description:t,enabled:a,detail:n}){let r=R();return u`
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
  `}function AR({channel:e,registryEntry:t}){let a=R(),n=t?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},l={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return u`
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
  `}function oO(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function lO({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=oO(e,i).filter(b=>tt(s,[i("channels.builtIn"),b.id,b.name,b.description,b.detail])),l=new Set(t.map(b=>b.name)),c=t.filter(b=>tt(s,[i("channels.messaging"),b.name,b.display_name,b.description,b.onboarding_state])),d=a.filter(b=>!l.has(b.name)).filter(b=>tt(s,[i("channels.messaging"),b.name,b.display_name,b.description])),m=new Set(n.map(b=>b.name)),f=n.filter(b=>tt(s,[i("channels.mcpServers"),b.name,b.display_name,b.description,b.active?i("channels.active"):i("channels.inactive")])),h=r.filter(b=>!m.has(b.name)).filter(b=>tt(s,[i("channels.mcpServers"),b.name,b.display_name,b.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:h}}function DR({searchQuery:e=""}){let t=R(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=TR();if(o)return u`
      <div className="space-y-5">
        <${ae} padding="md">
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
    `;let{builtInChannels:l,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:f}=lO({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return l.length===0&&c.length===0&&d.length===0&&m.length===0&&f.length===0?u`<${Rt} query=${e} />`:u`
    <div className="space-y-5">
      ${l.length>0&&u`
      <${ae} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${l.map(h=>u`
            <${iO}
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
        <${ae} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.messaging")}
          </h3>
          ${c.map(h=>u`
              <${AR}
                key=${h.name}
                channel=${h}
                registryEntry=${r.find(b=>b.name===h.name)}
              />
            `)}
          ${d.map(h=>u`
              <${AR} key=${h.name} registryEntry=${h} />
            `)}
        <//>
      `}
      ${(m.length>0||f.length>0)&&u`
        <${ae} padding="md">
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
  `}function MR({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:l,onNearaiWallet:c,onCodexLogin:d,loginBusy:m}){let f=R(),h=e.id===t,b=Kr(e,n),y=ui(e,n),$=Nw(e,n,t,a),g=Mc(e,n),v=_w(e),x=f(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[w,S]=p.default.useState(h),k=p.default.useCallback(()=>S(xt=>!xt),[]);p.default.useEffect(()=>{S(h)},[h]);let N=b?u`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${il(e.adapter)} · ${$||e.default_model||f("llm.none")}
      </span>`:u`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${x}
      </span>`,C=e.id==="nearai"||e.id==="openai_codex",P=e.api_key_set===!0||e.has_api_key===!0,L=e.builtin?e.id==="nearai"&&v&&!P?f("llm.addApiKey"):f("llm.configure"):f("common.edit"),U=v&&e.builtin?u`
          <${A}
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
          <${A} type="button" variant="secondary" size="sm" disabled=${m} onClick=${c}>
            ${f("onboarding.nearWallet")}
          <//>
          <${A} type="button" variant="secondary" size="sm" disabled=${m} onClick=${()=>l("github")}>
            GitHub
          <//>
          <${A} type="button" variant="secondary" size="sm" disabled=${m} onClick=${()=>l("google")}>
            Google
          <//>
        `:!h&&e.id==="openai_codex"?u`
          <${A} type="button" variant="secondary" size="sm" disabled=${m} onClick=${d}>
            ${f("onboarding.codexSignIn")}
          <//>
        `:null,K=!h&&b&&(!C||e.id==="nearai"&&e.has_api_key===!0)?u`
        <${A}
          type="button"
          variant="primary"
          size="sm"
          disabled=${r}
          onClick=${()=>s(e)}
        >
          ${f("llm.use")}
        <//>
      `:null,ee=b?null:u`
        <${A}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${r}
          onClick=${()=>i(e)}
        >
          ${f(g==="api_key"?"llm.addApiKey":"llm.configure")}
        <//>
      `,ne=h?null:K||(C?F:ee),he=!C&&(e.builtin&&e.id!=="bedrock"||!e.builtin)||e.id==="nearai"&&v;return u`
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
          onClick=${k}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 px-4 py-3 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)] sm:pl-5 sm:pr-3"
        >
          <span
            className=${["h-2 w-2 shrink-0 rounded-full",h?"bg-[var(--v2-positive-text)]":b?"bg-[var(--v2-accent)]":"bg-[var(--v2-warning-text)]"].join(" ")}
          />
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
            <span className="min-w-0 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
              ${e.name||e.id}
            </span>
            <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">${e.id}</span>
            ${h&&u`<${q} tone="positive" label=${f("llm.active")} size="sm" />`}
            ${e.builtin&&!h&&u`<${q} tone="muted" label=${f("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${N}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 py-3 pr-4 sm:pr-5">
          ${ne}
          <button
            type="button"
            onClick=${k}
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
            ${he&&u`
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
  `}var uO=[{key:"active",labelKey:"llm.groupActive",dotClass:"bg-[var(--v2-positive-text)]"},{key:"ready",labelKey:"llm.groupReady",dotClass:"bg-[var(--v2-accent)]"},{key:"setup",labelKey:"llm.groupSetup",dotClass:"bg-[var(--v2-warning-text)]"}];function cO({label:e,count:t,dotClass:a}){return u`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full "+a} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${t}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `}function OR({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=R(),r=sd({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=id(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return u`<${Rt} query=${a} />`;let l=Rw(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return u`
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

      ${r.message&&u`
        <div
          className=${["mb-4 rounded-md border px-3 py-2 text-sm",r.message.tone==="error"?"border-red-400/30 bg-red-500/10 text-red-200":"border-mint/30 bg-mint/10 text-mint"].join(" ")}
          role="status"
        >
          ${r.message.text}
        </div>
      `}

      <${rd} login=${i} />

      ${s.isLoading?u`<div className="text-sm text-[var(--v2-text-muted)]">${n("common.loading")}</div>`:s.error?u`<div className="text-sm text-red-200">${n("error.loadFailed",{what:n("llm.providers"),message:s.error.message})}</div>`:u`
            <div className="space-y-1">
              ${uO.flatMap(c=>{let d=l[c.key];return d.length?[u`
                    <section
                      key=${c.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${c.key}
                      className="mb-3"
                    >
                      <${cO}
                        label=${n(c.labelKey)}
                        count=${d.length}
                        dotClass=${c.dotClass}
                      />
                      <div className="space-y-2">
                      ${d.map(m=>u`
                          <${MR}
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

      <${nd}
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
  `}function LR({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=R(),{activeProviderId:o,selectedModel:l,providers:c,hasActiveProvider:d}=ci({settings:e,gatewayStatus:t});if(r)return u`<${dO} />`;let m=d?o:"",f=c.find(g=>g.id===o),h=d&&(l||f?.default_model||e.selected_model)||"",b=ki(NR,e,s,i),y=tt(s,[i("inference.provider"),i("inference.backend"),m,i("inference.model"),h]),$=tt(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!y&&!$&&b.length===0?u`<${Rt} query=${s} />`:u`
    <div className="space-y-5">
      ${y&&u`
      <${ae} padding="none" className="p-4 sm:p-5">
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
        <${OR}
          settings=${e}
          gatewayStatus=${t}
          searchQuery=${s}
        />
      `}

      ${b.map(g=>u`
            <${Ci}
              key=${g.groupKey}
              groupKey=${g.groupKey}
              fields=${g.fields}
              settings=${e}
              onSave=${a}
              savedKeys=${n}
            />
          `)}
    </div>
  `}function fr({className:e=""}){return u`
    <div
      className=${"rounded animate-pulse bg-[var(--v2-surface-muted)] "+e}
    />
  `}function dO(){return u`
    <div className="space-y-5">
      <${ae} padding="md">
        <${fr} className="mb-4 h-3 w-24" />
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${fr} className="h-3 w-16" />
            <${fr} className="mt-2 h-6 w-28" />
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${fr} className="h-3 w-16" />
            <${fr} className="mt-2 h-6 w-40" />
          </div>
        </div>
      <//>
      ${[1,2].map(e=>u`
            <${ae} key=${e} padding="md">
              <${fr} className="mb-4 h-3 w-20" />
              ${[1,2,3].map(t=>u`
                    <div key=${t} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                      <${fr} className="h-4 w-32" />
                      <${fr} className="h-9 w-36" />
                    </div>
                  `)}
            <//>
          `)}
    </div>
  `}function PR({searchQuery:e=""}){let t=R(),{lang:a,setLang:n}=$l(),r=wl.find(i=>i.code===a)||wl[0],s=wl.filter(i=>tt(e,[i.code,i.name,i.native]));return s.length===0?u`<${Rt} query=${e} />`:u`
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
  `}function UR({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=R();if(n)return u`
      <div className="space-y-5">
        ${[1,2].map(o=>u`
              <${ae} key=${o} padding="md">
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
    `;let i=ki(RR,e,r,s);return i.length===0?u`<${Rt} query=${r} />`:u`
    <div className="space-y-5">
      ${i.map(o=>u`
            <${Ci}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function jR(){let e=R(),[t,a]=p.default.useState(!1),n=p.default.useCallback(()=>a(!0),[]),r=p.default.useCallback(()=>a(!1),[]),s=p.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function FR({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=R(),r=jR({gatewayStatus:t,gatewayStatusQuery:a});return e?u`
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

    <${vi}
      open=${r.confirmOpen}
      onClose=${r.closeConfirm}
      title=${n("restart.title")}
      size="sm"
    >
      <${gi} className="space-y-3">
        <p className="text-sm text-[var(--v2-text)]">
          ${n("restart.description")}
        </p>
        <div className="rounded-xl border border-copper/25 bg-copper/10 px-3 py-2 text-xs text-copper">
          ${n("restart.warning")}
        </div>
      <//>
      <${yi}>
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
  `:null}function BR(){let e=W(),t=H({queryKey:["skills"],queryFn:mw}),a=G({mutationFn:pw,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),n=G({mutationFn:vw,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),r=G({mutationFn:({name:c,content:d})=>hw(c,{content:d}),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),s=G({mutationFn:({name:c,enabled:d})=>gw(c,d),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),i=G({mutationFn:c=>yw(c),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),o=t.data?.skills||[],l=t.data?.auto_activate_learned!==!1;return{skills:o,query:t,autoActivateLearned:l,fetchSkillContent:fw,installSkill:a.mutateAsync,removeSkill:n.mutateAsync,updateSkill:r.mutateAsync,setSkillAutoActivate:s.mutateAsync,setAutoActivateLearned:i.mutateAsync,isInstalling:a.isPending,isRemoving:n.isPending,isUpdating:r.isPending,isSettingAutoActivate:s.isPending,isSettingAutoActivateLearned:i.isPending}}function zR({skill:e,onEdit:t,onRemove:a,onUpdate:n,onSetAutoActivate:r,isRemoving:s,isUpdating:i,isSettingAutoActivate:o}){let l=R(),c=e.name||e.id,d=e.trust||e.trust_level||"installed",m=e.source_kind||"installed",f=!!e.can_edit,h=!!e.can_delete,b=e.auto_activate!==!1,[y,$]=p.default.useState(!1),[g,v]=p.default.useState(""),[x,w]=p.default.useState(""),[S,k]=p.default.useState(!1);p.default.useEffect(()=>{y||(v(""),w(""))},[y]);let N=p.default.useCallback(async()=>{k(!0),w("");try{let P=await t(c);v(P?.content||""),$(!0)}catch(P){w(P.message||l("skills.contentLoadFailed"))}finally{k(!1)}},[c,t,l]),C=p.default.useCallback(async()=>{(await n(c,g))?.success&&$(!1)},[g,c,n]);return u`
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
                  <${Ic}
                    rows=${12}
                    value=${g}
                    className="font-mono text-xs leading-5"
                    onInput=${P=>v(P.currentTarget.value)}
                  />
                </div>
              `:u`<${mO} skill=${e} />`}
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
              onClick=${C}
            >
              <${M} name="check" className="h-4 w-4" />
              ${l(i?"skills.saving":"skills.save")}
            <//>
          `}
          ${f&&!y&&u`
            <${A}
              type="button"
              variant=${b?"secondary":"ghost"}
              size="sm"
              disabled=${o}
              title=${l(b?"skills.autoActivateOnTitle":"skills.autoActivateOffTitle")}
              onClick=${()=>r(c,!b)}
            >
              <${M} name=${b?"check":"close"} className="h-4 w-4" />
              ${l(b?"skills.autoActivateOnLabel":"skills.autoActivateOffLabel")}
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
      ${x&&u`<p className="mt-2 text-xs text-[var(--v2-danger-text)]">${x}</p>`}
    </div>
  `}function mO({skill:e}){let t=R();return u`
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
        ${e.has_requirements&&u`<${ev}>requirements.txt<//>`}
        ${e.has_scripts&&u`<${ev}>scripts/<//>`}
        ${e.install_source_url&&u`<${ev}>${t("skills.imported")}<//>`}
      </div>
    `}
  `}function ev({children:e}){return u`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${e}
    </span>
  `}function qR({onInstall:e,isInstalling:t}){let a=R(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState({name:"",content:""}),[c,d]=p.default.useState(""),[m,f]=p.default.useState(""),h=p.default.useCallback((y,$)=>{l(g=>!g[y]||!$.trim()?g:{...g,[y]:""})},[]),b=p.default.useCallback(async()=>{let y=fO({name:n,content:s}),$=pO(y,a);if($.name||$.content){l($),d(""),f("");return}l({name:"",content:""}),d(""),f("");try{let g=await e(y);if(!g?.success){d(g?.message||a("skills.installFailed"));return}r(""),i(""),f(g.message||a("skills.installedSuccess",{name:y.name}))}catch(g){d(g.message||a("skills.installFailed"))}},[s,n,e,a]);return u`
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
        <${Ic}
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
        <${A} type="button" size="sm" disabled=${t} onClick=${b}>
          <${M} name="upload" className="h-4 w-4" />
          ${a(t?"skills.installing":"skills.install")}
        <//>
      </div>
    <//>
  `}function fO({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function pO(e,t){return{name:e.name?"":t("skills.nameRequired"),content:e.content?"":t("skills.contentRequired")}}function IR({searchQuery:e=""}){let t=R(),{skills:a,query:n,autoActivateLearned:r,fetchSkillContent:s,installSkill:i,removeSkill:o,updateSkill:l,setSkillAutoActivate:c,setAutoActivateLearned:d,isInstalling:m,isRemoving:f,isUpdating:h,isSettingAutoActivate:b,isSettingAutoActivateLearned:y}=BR(),[$,g]=p.default.useState(""),[v,x]=p.default.useState(""),w=p.default.useCallback(async P=>{if(window.confirm(t("skills.confirmDelete",{name:P}))){g(""),x("");try{let L=await o(P);if(!L?.success){g(L?.message||t("skills.removeFailed"));return}x(L.message||t("skills.removed",{name:P}))}catch(L){g(L.message||t("skills.removeFailed"))}}},[o,t]),S=p.default.useCallback(async(P,L)=>{if(!L.trim())return g(t("skills.contentRequired")),x(""),{success:!1,message:t("skills.contentRequired")};g(""),x("");try{let U=await l({name:P,content:L});return U?.success?(x(U.message||t("skills.updated",{name:P})),U):(g(U?.message||t("skills.updateFailed")),U)}catch(U){let F=U.message||t("skills.updateFailed");return g(F),{success:!1,message:F}}},[t,l]),k=p.default.useCallback(async(P,L)=>{g(""),x("");try{let U=await c({name:P,enabled:L});if(!U?.success){g(U?.message||t("skills.updateFailed"));return}x(U.message)}catch(U){g(U.message||t("skills.updateFailed"))}},[c,t]),N=p.default.useCallback(async P=>{g(""),x("");try{let L=await d(P);if(!L?.success){g(L?.message||t("skills.updateFailed"));return}x(L.message)}catch(L){g(L.message||t("skills.updateFailed"))}},[d,t]),C;if(n.isLoading)C=u`
      <${ae} padding="md">
          <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(P=>u`
            <div key=${P} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
              <div>
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              </div>
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
        <//>
    `;else if(n.error)C=u`
      <${ae} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">${t("skills.failedLoad",{message:n.error.message})}</p>
        <//>
    `;else{let P=a.filter(U=>tt(e,[U.name,U.id,U.description,U.keywords,U.trust_level,U.source_kind,U.version])),L=gO(P);a.length===0?C=u`
        <${ae} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:P.length===0?C=u`<${Rt} query=${e} />`:C=u`
        <div id="skills-list">
          ${L.map(U=>u`
              <${vO}
                key=${U.id}
                title=${t(U.labelKey)}
                skills=${U.skills}
                onEdit=${s}
                onRemove=${w}
                onUpdate=${S}
                onSetAutoActivate=${k}
                isRemoving=${f}
                isUpdating=${h}
                isSettingAutoActivate=${b}
              />
            `)}
        </div>
      `}return u`
    <div className="space-y-4">
      <${hO}
        enabled=${r}
        isSaving=${y}
        onToggle=${N}
      />
      <${qR} onInstall=${i} isInstalling=${m} />
      <${yO} error=${$} result=${v} />
      ${C}
    </div>
  `}function hO({enabled:e,isSaving:t,onToggle:a}){let n=R();return u`
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
  `}function vO({title:e,skills:t,onEdit:a,onRemove:n,onUpdate:r,onSetAutoActivate:s,isRemoving:i,isUpdating:o,isSettingAutoActivate:l}){return t.length===0?null:u`
    <${ae} padding="md">
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
  `}function gO(e){let t=[{id:"user",labelKey:"skills.group.user",skills:[]},{id:"system",labelKey:"skills.group.system",skills:[]},{id:"workspace",labelKey:"skills.group.workspace",skills:[]}],a=t[0];for(let n of e){let r=n.source_kind||"";(r==="system"?t[1]:r==="workspace"?t[2]:a).skills.push(n)}return t.filter(n=>n.skills.length>0)}function yO({error:e,result:t}){return!e&&!t?null:u`
    <div
      className=${e?"rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200":"rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200"}
    >
      ${e||t}
    </div>
  `}function Rd(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function HR(){let e=W(),t=H({queryKey:["settings-tools"],queryFn:lw}),a=t.data?.tools||[],[n,r]=p.default.useState({}),s=G({mutationFn:async({name:o,state:l})=>Rd(await uw(o,l),"Save failed"),onSuccess:(o,{name:l,state:c})=>{e.setQueryData(["settings-tools"],d=>{if(!d)return d;let m=o?.tool;return{...d,tools:d.tools.map(f=>f.name===l?{...f,state:c,...m||{}}:f)}}),r(d=>({...d,[l]:!0})),setTimeout(()=>r(d=>({...d,[l]:!1})),2e3)}}),i=p.default.useCallback((o,l)=>s.mutate({name:o,state:l}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}var tv="agent.auto_approve_tools";function KR(e,t){let a=`tools.description.${t.name}`,n=e(a);return n&&n!==a?n:t.description||""}function bO({visible:e}){let t=R();return e?u`
    <span className="font-mono text-[11px] text-[var(--v2-accent-text)]" role="status">
      ${t("tools.saved")}
    </span>
  `:null}function xO({checked:e,disabled:t=!1,label:a,onChange:n}){return u`
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
  `}function av({settings:e,onSave:t,savedKeys:a,isLoading:n}){let r=R(),s=r("settings.field.autoApproveEligibleTools"),i=e?.[tv],o=i==null?!0:i===!0||i==="true";return u`
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
        <${bO} visible=${a?.[tv]} />
        <${xO}
          checked=${o}
          disabled=${n}
          label=${s}
          onChange=${l=>t(tv,l)}
        />
      </div>
    <//>
  `}function $O({tool:e,onPermissionChange:t,isSaved:a}){let n=R(),r=KR(n,e),s=[{value:"default",label:n("tools.followDefault"),tone:"neutral"},{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask_each_time",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],i={default:n("tools.sourceDefault"),global:n("tools.sourceGlobal"),override:n("tools.sourceOverride")},o=e.locked,l=s.find(f=>f.value===e.state)||s[1],c=e.effective_source||"default",d=c==="override"?e.state:"default",m=c==="default"&&e.state===e.default_state;return u`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="flex min-w-0 items-center gap-3">
        ${o&&u`<${M}
          name="lock"
          className="h-3.5 w-3.5 shrink-0 text-[var(--v2-text-faint)]"
        />`}
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
        ${o?u`<${q} tone=${l.tone} label=${l.label} size="sm" />`:u`
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
  `}function QR({settings:e={},onSave:t=()=>{},savedKeys:a={},isLoading:n=!1,searchQuery:r=""}){let s=R(),{tools:i,query:o,setPermission:l,savedTools:c}=HR();if(o.isLoading)return u`
      <div className="space-y-4">
        <${av}
          settings=${e}
          onSave=${t}
          savedKeys=${a}
          isLoading=${n}
        />
        <${ae} padding="md">
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
        <${av}
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
    `;let d=i.filter(m=>{let f=KR(s,m);return tt(r,[m.name,m.description,f,m.state,m.default_state,m.effective_source,m.state==="disabled"?s("tools.disabled"):""])});return u`
    <div className="space-y-4">
      <${av}
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

      <${ae} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${s("tools.permissions")}
        </h3>
        ${d.length===0?u`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${s("tools.noMatch")}
            </p>`:d.map(m=>u`
                  <${$O}
                    key=${m.name}
                    tool=${m}
                    onPermissionChange=${l}
                    isSaved=${c[m.name]}
                  />
                `)}
      <//>
    </div>
  `}function VR(e){return(Number(e)||0).toFixed(2)}function wO(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function GR(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function Wr({label:e,value:t,description:a}){return u`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&u`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function YR({searchQuery:e=""}){let t=R(),{credits:a,query:n,authorize:r}=Pc();if(!tt(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return u`<${Rt} query=${e} />`;let s;if(n.isLoading)s=u`
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
          value=${VR(a.pending_credit)}
        />
        <${Wr}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${VR(a.final_credit)}
        />
        <${Wr}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${wO(a.delayed_credit_delta)}
        />
        <${Wr}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${Wr}
          label=${t("traceCommons.lastSubmission")}
          value=${GR(a.last_submission_at,t)}
        />
        <${Wr}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${GR(a.last_credit_sync_at,t)}
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
  `}function JR(){let e=W(),t=H({queryKey:["admin-users"],queryFn:$w,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=G({mutationFn:ww,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=G({mutationFn:({id:i,payload:o})=>Sw(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function SO({onCreate:e,isCreating:t,error:a}){let n=R(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState("member"),[d,m]=p.default.useState(!1),f=h=>{h.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:l},{onSuccess:()=>{s(""),o(""),m(!1)}})};return d?u`
    <${ae} padding="md">
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
    `}function NO({user:e}){let t=R(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return u`
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
  `}function XR({searchQuery:e=""}){let t=R(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=JR();if(n.isLoading)return u`
      <${ae} padding="md">
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
    `;if(n.error)return u`
      <${ae} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("users.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let l=a.filter(c=>tt(e,[c.id,c.display_name,c.email,c.role,c.status,c.last_active]));return u`
    <div className="space-y-5">
      <${SO}
        onCreate=${s}
        isCreating=${o}
        error=${i}
      />

      <${ae} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("users.title",{count:l.length})}
        </h3>
        ${a.length===0?u`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("users.noUsers")}
            </p>`:l.length===0?u`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("settings.noMatchingSettings",{query:e})}
            </p>`:l.map(c=>u`<${NO} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function WR(){let e=W(),t=H({queryKey:["settings-export"],queryFn:Z$,staleTime:3e4}),a=t.data?.settings||{},[n,r]=p.default.useState({}),[s,i]=p.default.useState(!1),o=G({mutationFn:async({key:m,value:f})=>Rd(await ah(m,f),"Save failed"),onSuccess:(m,{key:f,value:h})=>{e.setQueryData(["settings-export"],b=>{if(!b)return b;let y={...b,settings:{...b.settings}};return h==null?delete y.settings[f]:y.settings[f]=h,y}),r(b=>({...b,[f]:!0})),setTimeout(()=>r(b=>({...b,[f]:!1})),2e3),Zh.has(f)&&i(!0),f==="agent.auto_approve_tools"&&e.invalidateQueries({queryKey:["settings-tools"]})}}),l=p.default.useCallback((m,f)=>o.mutate({key:m,value:f}),[o]),c=G({mutationFn:ew,onSuccess:(m,f)=>{e.invalidateQueries({queryKey:["settings-export"]});let h=Object.keys(f?.settings||{});h.includes("agent.auto_approve_tools")&&e.invalidateQueries({queryKey:["settings-tools"]}),h.some(b=>Zh.has(b))&&i(!0)}}),d=p.default.useCallback(m=>c.mutateAsync(m),[c]);return{settings:a,query:t,save:l,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error||c.error}}function nv(){let e=R(),{tab:t}=it(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=wa(),s=r?"inference":"language",i=t||s,{settings:o,query:l,save:c,savedKeys:d,needsRestart:m,saveError:f}=WR(),[h,b]=p.default.useState("");p.default.useEffect(()=>{b("")},[i]);let y=l.isLoading,$={inference:u`<${LR}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,agent:u`<${ER}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,channels:u`<${DR} searchQuery=${h} />`,networking:u`<${UR}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,tools:u`<${QR}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,skills:u`<${IR} searchQuery=${h} />`,traces:u`<${YR} searchQuery=${h} />`,users:u`<${XR} searchQuery=${h} />`,language:u`<${PR} searchQuery=${h} />`},g=k=>k==="users"||k==="inference",v=k=>Object.prototype.hasOwnProperty.call($,k),x=Object.keys($).filter(k=>r||!g(k)),S=v(s)&&x.includes(s)?s:x[0]||"language";return!v(i)||!r&&g(i)?u`<${ot} to=${`/settings/${S}`} replace />`:u`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${m&&u`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${FR}
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
  `}var rv=Object.freeze({todo:!0});function ZR(){return Promise.resolve({users:[],total:0,...rv})}function ek(e){return Promise.resolve(null)}function tk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function ak(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function nk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function rk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function sk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function ik(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function ok(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...rv})}function lk(e="day",t){return Promise.resolve({entries:[],...rv})}function uk(){return H({queryKey:["admin","usage-summary"],queryFn:ok,refetchInterval:3e4})}function kd(e="day",t){return H({queryKey:["admin","usage",e,t],queryFn:()=>lk(e,t),refetchInterval:3e4})}function Ei(){let e=W(),t=H({queryKey:["admin","users"],queryFn:ZR,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=G({mutationFn:tk,onSuccess:s}),o=G({mutationFn:({id:f,payload:h})=>ak(f,h),onSuccess:s}),l=G({mutationFn:f=>nk(f),onSuccess:s}),c=G({mutationFn:f=>rk(f),onSuccess:s}),d=G({mutationFn:f=>sk(f),onSuccess:s}),m=G({mutationFn:({userId:f,name:h})=>ik(f,h)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(f,h)=>o.mutateAsync({id:f,payload:h}),deleteUser:l.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(f,h)=>m.mutateAsync({userId:f,name:h}),newToken:m.data,clearToken:()=>m.reset()}}function ck(e){return H({queryKey:["admin","user",e],queryFn:()=>ek(e),enabled:!!e,refetchInterval:1e4})}function nn(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function La(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function dk(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function pr(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function Ti(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function Ai(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function Di(e){return e==="admin"?"signal":"muted"}function mk(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function fk(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function pk(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function hk(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function vk(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function _O({users:e,onSelectUser:t}){let a=R(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?u`
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
                <td className="py-3 pr-4"><${q} tone=${Di(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${q} tone=${Ai(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${pr(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:u`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function gk({onSelectUser:e,onNavigateTab:t}){let a=R(),n=uk(),{users:r,query:s}=Ei(),i=n.data||{},o=mk(r),l=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?u`
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
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:dk(i.uptime_seconds)})}</span>
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
        <${_O} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var RO=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function kO({value:e,max:t}){let a=t>0?e/t*100:0;return u`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function yk({onSelectUser:e}){let t=R(),[a,n]=p.default.useState("day"),r=kd(a),s=r.data?.usage||[],i=pk(s),o=hk(s),l=vk(i),c=i.length>0?i[0].cost:0;return r.isLoading?u`
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
            ${RO.map(d=>u`
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
                          ${Ti(d.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${nn(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${nn(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${La(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${kO} value=${d.cost} max=${c} />
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
  `}function hr({label:e,children:t}){return u`
    <div className="flex items-start justify-between gap-4 border-t border-white/[0.06] py-3 first:border-0 first:pt-0">
      <span className="text-xs text-iron-300">${e}</span>
      <span className="text-right text-sm text-iron-100">${t}</span>
    </div>
  `}function bk({userId:e,onBack:t}){let a=R(),n=ck(e),r=kd("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:l,createToken:c,newToken:d,clearToken:m}=Ei(),[f,h]=p.default.useState(null),[b,y]=p.default.useState(!1),$=n.data,g=r.data?.usage||[];if(p.default.useEffect(()=>{$&&f===null&&h($.role)},[$]),n.isLoading)return u`
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
    `;if(!$)return null;let v=async()=>{f&&f!==$.role&&await o($.id,{role:f})},x=async()=>{await l($.id),t()},w=async()=>{let S=window.prompt(a("admin.users.tokenNamePrompt",{name:$.display_name||a("admin.users.userFallback")}));S&&await c($.id,S)};return u`
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
              <${q} tone=${Di($.role)} label=${$.role||"member"} />
              <${q} tone=${Ai($.status)} label=${$.status||"active"} />
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
        <${I} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.profile")}</h3>
          <${hr} label=${a("admin.user.id")}>
            <span className="font-mono text-xs">${$.id}</span>
          <//>
          <${hr} label=${a("admin.user.email")}>${$.email||a("admin.user.notSet")}<//>
          <${hr} label=${a("admin.user.created")}>${pr($.created_at)}<//>
          <${hr} label=${a("admin.user.lastLogin")}>${pr($.last_login_at)}<//>
          ${$.created_by&&u`
            <${hr} label=${a("admin.user.createdBy")}>
              <span className="font-mono text-xs">${Ti($.created_by)}</span>
            <//>
          `}
        <//>

        <${I} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${hr} label=${a("admin.user.jobs")}>${$.job_count??0}<//>
          <${hr} label=${a("admin.user.totalCost")}>${La($.total_cost)}<//>
          <${hr} label=${a("admin.user.lastActive")}>${pr($.last_active_at)}<//>
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
                    ${g.map((S,k)=>u`
                        <tr key=${k} className="border-b border-white/[0.06] last:border-0">
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

      ${b&&u`
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick=${()=>y(!1)}>
          <div className="w-full max-w-md rounded-xl border border-white/10 bg-iron-900 p-6" onClick=${S=>S.stopPropagation()}>
            <h3 className="text-lg font-semibold text-white">${a("admin.users.deleteUserTitle")}</h3>
            <p className="mt-2 text-sm text-iron-300">
              ${a("admin.users.deleteUserDesc",{name:$.display_name})}
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <${A} variant="ghost" onClick=${()=>y(!1)}>${a("admin.users.cancel")}<//>
              <button
                onClick=${x}
                className="v2-button inline-flex h-10 items-center justify-center rounded-md bg-red-500/20 px-4 text-sm font-semibold text-red-200 hover:bg-red-500/30"
              >
                ${a("admin.users.delete")}
              </button>
            </div>
          </div>
        </div>
      `}
    </div>
  `}function CO(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function EO({token:e,onDismiss:t}){let a=R(),[n,r]=p.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return u`
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
  `}function TO({onCreate:e,isCreating:t,error:a}){let n=R(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState("member"),[d,m]=p.default.useState(!1),f=async h=>{h.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:l}),s(""),o(""),m(!1))};return d?u`
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
    `}function AO({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=R();return u`
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
  `}function DO({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=R();return u`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${q} tone=${Di(e.role)} label=${e.role||"member"} />
          <${q} tone=${Ai(e.status)} label=${e.status||"active"} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          ${e.email&&u`<span className="font-mono text-xs text-iron-300">${e.email}</span>`}
          <span className="font-mono text-xs text-iron-700">${Ti(e.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-iron-300 sm:inline">
          ${e.job_count!=null?i("admin.users.jobsCount",{count:e.job_count}):""}
          ${e.total_cost!=null?` \xB7 ${La(e.total_cost)}`:""}
        </span>
        <span className="hidden text-xs text-iron-700 lg:inline">${pr(e.last_active_at)}</span>
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
  `}function xk({selectedUserId:e,onSelectUser:t}){let a=R(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:l,updateUser:c,deleteUser:d,suspendUser:m,activateUser:f,createToken:h,newToken:b,clearToken:y}=Ei(),[$,g]=p.default.useState(""),[v,x]=p.default.useState("all"),[w,S]=p.default.useState(null),k=fk(n,{search:$,filter:v}),N=CO(a),C=L=>{S({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{m(L),S(null)}})},P=async(L,U)=>{let F=window.prompt(a("admin.users.tokenNamePrompt",{name:U||a("admin.users.userFallback")}));F&&await h(L,F)};return r.isLoading?u`
      <${I} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-3 w-24 rounded" />
        ${[1,2,3].map(L=>u`
          <div key=${L} className="flex items-center justify-between border-t border-iron-700 py-3.5 first:border-0">
            <div className="v2-skeleton h-4 w-32 rounded" />
            <div className="v2-skeleton h-6 w-20 rounded-full" />
          </div>
        `)}
      <//>
    `:s?u`
      <${I} className="p-6 sm:p-8">
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
      ${b&&u`
        <${EO}
          token=${b.token||b.plaintext_token}
          onDismiss=${y}
        />
      `}

      <${TO} onCreate=${i} isCreating=${o} error=${l} />

      <${I} className="p-5 sm:p-6">
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${a("admin.users.title",{count:k.length,total:n.length})}
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
              ${N.map(L=>u`
                  <button
                    key=${L.value}
                    onClick=${()=>x(L.value)}
                    className=${["rounded-md px-2.5 py-1.5 text-[11px] font-medium",v===L.value?"border border-signal/35 bg-signal/10 text-iron-100":"border border-transparent text-iron-300 hover:text-iron-100"].join(" ")}
                  >
                    ${L.label}
                  </button>
                `)}
            </div>
          </div>
        </div>

        ${k.length===0?u`<p className="py-4 text-sm text-iron-300">${a("admin.users.noMatch")}</p>`:k.map(L=>u`
                <${DO}
                  key=${L.id}
                  user=${L}
                  onSelect=${t}
                  onSuspend=${C}
                  onActivate=${f}
                  onChangeRole=${(U,F)=>c(U,{role:F})}
                  onCreateToken=${P}
                />
              `)}
      <//>

      ${w&&u`
        <${AO}
          title=${w.title}
          message=${w.message}
          confirmLabel=${w.confirmLabel}
          onConfirm=${w.onConfirm}
          onCancel=${()=>S(null)}
        />
      `}
    </div>
  `}function $k(){let{tab:e="dashboard"}=it(),t=ve(),[a,n]=p.default.useState(null),r=p.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=p.default.useCallback(()=>{n(null)},[]),i={dashboard:u`<${gk}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?u`<${bk} userId=${a} onBack=${s} />`:u`<${xk}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:u`<${yk} onSelectUser=${r} />`};return i[e]?u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:u`<${ot} to="/admin/dashboard" replace />`}var MO=2e3,OO=500,LO=2e3,PO=new Set([403,404]),UO=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function jO(e=globalThis.location,t=null){let a=new URLSearchParams(e?.search||""),n={active:[]};for(let[r,s,i]of UO){let o=a.get(s)?.trim();o?(n[r]=o,n.active.push({key:r,param:s,labelKey:i,value:o})):n[r]=null}return!n.threadId&&t&&(n.threadId=t),n}function wk({isAdmin:e=!1,defaultThreadId:t=null}={}){let a=Ae(),n=a?.search||"",r=p.default.useMemo(()=>jO(a,t),[t,n]),{runId:s,source:i,threadId:o,toolCallId:l,toolName:c,turnId:d}=r,[m,f]=p.default.useState([]),[h,b]=p.default.useState("all"),[y,$]=p.default.useState(""),[g,v]=p.default.useState(!1),[x,w]=p.default.useState(!0),[S,k]=p.default.useState(!0),[N,C]=p.default.useState(null),P=p.default.useRef(new Set),L=p.default.useRef(0),U=!e&&!o;p.default.useEffect(()=>{L.current+=1,f([]),C(null)},[e,s,i,o,l,c,d]);let F=p.default.useCallback(async()=>{if(U){k(!1);return}let ee=++L.current;k(!0);try{let ne={limit:OO,level:h==="all"?null:h,target:y.trim()||null,threadId:o,runId:s,turnId:d,toolCallId:l,toolName:c,source:i},he;try{he=await(e?u$(ne):Ip(ne))}catch(De){if(!e||!PO.has(De?.status))throw De;he=await Ip(ne)}if(ee!==L.current)return;let xt=P.current,Oe=i2(he).entries.filter(De=>!xt.has(De.id));f(Oe),C(null)}catch(ne){if(ee!==L.current)return;C(ne)}finally{ee===L.current&&k(!1)}},[e,h,U,s,i,y,o,l,c,d]);p.default.useEffect(()=>{F()},[F]),p.default.useEffect(()=>{if(g||U)return;let ee=setInterval(F,MO);return()=>clearInterval(ee)},[F,U,g]);let T=p.default.useCallback(()=>{v(ee=>!ee)},[]),K=p.default.useCallback(()=>{let ee=[...P.current,...m.map(ne=>ne.id)].slice(-LO);P.current=new Set(ee),f([])},[m]);return{entries:m,totalCount:m.length,paused:g,togglePause:T,clearEntries:K,levelFilter:h,setLevelFilter:b,targetFilter:y,setTargetFilter:$,autoScroll:x,setAutoScroll:w,serverLevel:null,changeServerLevel:async()=>{},scope:r,needsThreadScope:U,status:U?"needs_scope":N?"error":S?"loading":"ready",isLoading:S,error:N}}var FO=["all","trace","debug","info","warn","error"],BO=["trace","debug","info","warn","error"],Sk={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},zO={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function qO({entry:e}){let t=R(),[a,n]=p.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=Sk[e.level]||Sk.info,i=zO[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(l=>!!l.value);return u`
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
  `}function Nk({value:e,onChange:t,options:a,labelKey:n,t:r}){return u`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>u`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function IO({label:e,value:t,scopeKey:a}){return u`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function _k(){let e=R(),{isAdmin:t=!1,threadsState:a}=wa()||{},{entries:n,totalCount:r,paused:s,togglePause:i,clearEntries:o,levelFilter:l,setLevelFilter:c,targetFilter:d,setTargetFilter:m,autoScroll:f,setAutoScroll:h,serverLevel:b,changeServerLevel:y,scope:$,isLoading:g,error:v,needsThreadScope:x}=wk({isAdmin:t,defaultThreadId:t?null:a?.activeThreadId||null}),w=p.default.useRef(null),S=p.default.useRef(!0);p.default.useEffect(()=>{f&&S.current&&w.current&&(w.current.scrollTop=0)},[n,f]);let k=p.default.useCallback(P=>{S.current=P.currentTarget.scrollTop<=48},[]),N=n.length>0,C=$?.active||[];return u`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${Nk}
          value=${l}
          onChange=${c}
          options=${FO}
          labelKey=${P=>P==="all"?"logs.levelAll":`logs.level.${P}`}
          t=${e}
        />

        <!-- Target filter -->
        <input
          type="text"
          value=${d}
          onInput=${P=>m(P.target.value)}
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
              onChange=${P=>h(P.target.checked)}
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

        ${C.length>0&&u`
          <div
            data-testid="logs-scope-toolbar"
            className="flex w-full flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]"
          >
            <span className="font-medium text-[var(--v2-text-strong)]">${e("logs.scoped")}</span>
            ${C.map(P=>u`<${IO} key=${P.param} scopeKey=${P.param} label=${e(P.labelKey)} value=${P.value} />`)}
            <a
              href="/v2/logs"
              className="ml-auto rounded-[6px] px-2 py-1 text-xs text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
            >
              ${e("logs.clearScope")}
            </a>
          </div>
        `}

        <!-- Server log level -->
        ${b!=null&&u`
          <div className="flex w-full items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]">
            <span>${e("logs.serverLevel")}</span>
            <${Nk}
              value=${b}
              onChange=${y}
              options=${BO}
              labelKey=${P=>`logs.level.${P}`}
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
        onScroll=${k}
        className="min-h-0 flex-1 overflow-y-auto bg-[var(--v2-canvas)]"
      >
        ${v&&N?u`
              <div
                className="sticky top-0 z-10 border-b border-red-500/25 bg-red-950/70 px-4 py-2 text-xs text-red-100 backdrop-blur"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:v.message||v.statusText||"Request failed"})}
              </div>
            `:null}
        ${x?u`
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
              `:N?n.map(P=>u`<${qO} key=${P.id} entry=${P} />`):u`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function kk(){return u`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function HO({auth:e}){let t=ve(),n=Ae().state?.from,r=n?`${n.pathname||Hr}${n.search||""}${n.hash||""}`:Hr,s=`/v2${r==="/"?"":r}`,i=p.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?u`<${kk} />`:e.isAuthenticated?u`<${ot} to=${r} replace />`:u`<${F1}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function KO({auth:e,children:t}){let a=Ae();return e.isChecking?u`<${kk} />`:e.isAuthenticated?t:u`<${ot} to="/login" replace state=${{from:a}} />`}function QO({auth:e}){return u`
    <${KO} auth=${e}>
      <${p1}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        rebornProjectsEnabled=${e.rebornProjectsEnabled}
        onSignOut=${e.signOut}
      />
    <//>
  `}function Rk({auth:e}){return e.isAdmin?u`<${$k} />`:u`<${ot} to=${Hr} replace />`}function Ck(){let e=V$();return u`
    <${Fp} basename="/v2">
      <${Op}>
        <${xe} path="/login" element=${u`<${HO} auth=${e} />`} />
        <${xe} path="/" element=${u`<${QO} auth=${e} />`}>
          <${xe} index element=${u`<${ot} to=${Hr} replace />`} />
          <${xe} path="overview" element=${u`<${ot} to=${Hr} replace />`} />
          <${xe} path="welcome" element=${u`<${f2} />`} />
          <${xe} path="chat" element=${u`<${Ah} />`} />
          <${xe} path="chat/:threadId" element=${u`<${Ah} />`} />
          <${xe} path="workspace" element=${u`<${Mh} />`} />
          <${xe} path="workspace/*" element=${u`<${Mh} />`} />
          <${xe} path="projects" element=${u`<${hl} />`} />
          <${xe} path="projects/:projectId" element=${u`<${hl} />`} />
          <${xe} path="projects/:projectId/missions/:missionId" element=${u`<${hl} />`} />
          <${xe} path="projects/:projectId/threads/:threadId" element=${u`<${hl} />`} />
          <${xe} path="missions" element=${u`<${Lh} />`} />
          <${xe} path="missions/:missionId" element=${u`<${Lh} />`} />
          <${xe} path="jobs" element=${u`<${jh} />`} />
          <${xe} path="jobs/:jobId" element=${u`<${jh} />`} />
          <${xe} path="routines" element=${u`<${Bh} />`} />
          <${xe} path="routines/:routineId" element=${u`<${Bh} />`} />
          <${xe} path="automations" element=${u`<${b_} />`} />
          <${xe} path="extensions" element=${u`<${Wh} />`} />
          <${xe} path="extensions/:tab" element=${u`<${Wh} />`} />
          <${xe} path="logs" element=${u`<${_k} />`} />
          <${xe} path="settings" element=${u`<${nv} />`} />
          <${xe} path="settings/:tab" element=${u`<${nv} />`} />
          <${xe} path="admin" element=${u`<${Rk} auth=${e} />`} />
          <${xe} path="admin/:tab" element=${u`<${Rk} auth=${e} />`} />
        <//>
        <${xe} path="*" element=${u`<${ot} to=${Hr} replace />`} />
      <//>
    <//>
  `}iv("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","approval.viewFullCommand":"View full command","approval.showCommandPreview":"Show preview","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.tabDeclined":"Declined","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitDeclined":"declined","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.serviceUnavailable":"Service unavailable","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveEligibleTools":"Always allow eligible tools","settings.field.autoApproveEligibleToolsDesc":"Applies to tools set to \u201CFollow global\u201D. Per-tool settings override this; hard-floor approval gates still ask.","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.followDefault":"Follow global","tools.sourceDefault":"default permission","tools.sourceGlobal":"global setting","tools.sourceOverride":"per-tool override","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","tools.description.builtin.echo":"Echo a message","tools.description.builtin.time":"Get, parse, format, convert, or diff timestamps","tools.description.builtin.json":"Parse, query, stringify, and validate JSON","tools.description.builtin.http":"Perform an outbound HTTP request through host egress. Redirect responses are returned; the host transport does not follow them. Prefer GitHub extension capabilities for GitHub repository, issue, pull request, release, or workflow data when available.","tools.description.builtin.http.save":"Perform an outbound HTTP request through host egress and save the sanitized response body through scoped filesystem authority. Prefer GitHub extension capabilities for GitHub repository, issue, pull request, release, or workflow data when available.","tools.description.builtin.shell":"Execute shell commands with validation and saved-file references for large local output","tools.description.builtin.spawn_subagent":"Authorize a scoped child subagent run","tools.description.builtin.trace_commons.onboard":"Enroll this IronClaw in Trace Commons using an operator-issued invite link after explicit user consent.","tools.description.builtin.trace_commons.status":"Report Trace Commons enrollment state for the current user.","tools.description.builtin.trace_commons.credits":"Report the current user's Trace Commons credit state, balances, submission counts, and recent explanations.","tools.description.builtin.trace_commons.profile_token":"Mint a short-lived Trace Commons profile-management value for browser or manual profile setup.","tools.description.builtin.trace_commons.profile_set":"Create or update the current user's public Trace Commons community profile after explicit consent.","tools.description.builtin.profile_set":"Record a private local fact about the user's agent context: timezone, locale, or location.","tools.description.builtin.memory_search":"Search Reborn persistent memory documents in the current scope","tools.description.builtin.memory_write":"Write, append, or patch Reborn persistent memory documents in the current scope","tools.description.builtin.memory_read":"Read a Reborn persistent memory document in the current scope","tools.description.builtin.memory_tree":"List Reborn persistent memory documents as a compact tree","tools.description.builtin.read_file":"Read text files and extract text from supported document files through scoped mounts","tools.description.builtin.write_file":"Write content through scoped mounts","tools.description.builtin.list_dir":"List directory contents through scoped mounts","tools.description.builtin.glob":"Find files under a scoped directory with a glob pattern","tools.description.builtin.grep":"Search scoped file contents with grep output modes","tools.description.builtin.apply_patch":"Apply exact or fuzzy search-replace edits through scoped mounts","tools.description.builtin.skill_list":"List Reborn filesystem skills visible to the current local-dev agent","tools.description.builtin.skill_install":"Install a SKILL.md document, URL, ZIP bundle, or GitHub skill repository into the current user's skill root","tools.description.builtin.skill_remove":"Remove a user-installed Reborn filesystem skill","tools.description.builtin.trigger_create":"Create a caller-scoped scheduled trigger, either one-time or recurring","tools.description.builtin.trigger_list":"List scheduled triggers owned by the current caller scope","tools.description.builtin.trigger_remove":"Remove a caller-scoped scheduled trigger","tools.description.builtin.trigger_pause":"Pause a caller-scoped scheduled trigger so it remains retained but does not fire","tools.description.builtin.trigger_resume":"Resume a caller-scoped paused trigger so it may fire on its stored schedule","tools.description.builtin.extension_search":"Search the local Reborn extension catalog by extension, product, provider, or service name","tools.description.builtin.extension_install":"Install a searched Reborn extension into durable local-dev lifecycle state","tools.description.builtin.extension_activate":"Activate an installed Reborn extension for the model-visible local-dev capability surface","tools.description.builtin.extension_remove":"Remove an installed Reborn extension from durable local-dev lifecycle state","tools.description.nearai.web_search":"Search through the NEAR AI MCP server","tools.description.builtin.outbound_delivery_target_set":"Set the current user's final-reply outbound delivery target, such as a Slack DM or Slack channel","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","skills.defaultAutoActivationEnabled":"Default skill auto-activation enabled","skills.defaultAutoActivationDisabled":"Default skill auto-activation disabled","skills.defaultAutoActivationOnDesc":"Skills auto-activate by keyword on matching requests. Turn off to require an explicit /name.","skills.defaultAutoActivationOffDesc":"Skills run only when you type /name. Turn on to let them auto-activate by keyword.","skills.defaultAutoActivationOnButton":"Default: On","skills.defaultAutoActivationOffButton":"Default: Off","skills.autoActivateOnTitle":"Auto-activation on \u2014 runs on matching requests. Click to make it explicit-only (/name).","skills.autoActivateOffTitle":"Explicit-only \u2014 runs only when you type /name. Click to enable auto-activation.","skills.autoActivateOnLabel":"Auto-activate: On","skills.autoActivateOffLabel":"Auto-activate: Off","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Memory, files & attachments","workspace.readOnly":"Read-only","workspace.filterPlaceholder":"Filter by name\u2026","workspace.emptyDir":"This folder is empty.","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.noFiles":"No files here.","workspace.noMatches":"Nothing matches that filter.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a file","workspace.pickFileDesc":"Choose a file from the tree to preview or download it. This viewer is read-only.","workspace.parent":"Parent: {path}","workspace.download":"Download","workspace.binaryPreviewUnavailable":"No inline preview for this file type. Download it to view the contents.","workspace.fileMeta":"{mime} \xB7 {size} bytes","workspace.unableOpenDirectory":"Unable to open directory","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.filter.completed":"Completed","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.schedule.onceAt":"Once on {datetime}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.filterAction":"Show {label}","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.empty.onboardingTitle":"No automations yet","automations.empty.onboardingDescription":"Automations are created by chatting with your agent \u2014 there's no form to fill out. Ask it to do something on a schedule and it will set up a recurring automation for you.","automations.empty.examplesTitle":"Try asking your agent","automations.empty.example1":"Check the nearai/ironclaw repo every 10 minutes and summarize new issues, PRs, and commits.","automations.empty.example2":"Every weekday at 9am, send me a summary of my unread email.","automations.empty.example3":"Remind me to review open pull requests every afternoon at 3pm.","automations.empty.startInChat":"Start in chat","automations.empty.copyPrompt":"Copy prompt","automations.empty.copied":"Copied","automations.refreshing":"Refreshing\u2026","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.runs.total":"Recent runs: {count}","automations.runs.ok":"OK: {count}","automations.runs.error":"Failed: {count}","automations.runs.running":"Running: {count}","automations.runs.unknown":"Unknown: {count}","automations.runs.showingOf":"Showing {shown} of {total} recent runs","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up a project for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open project","projects.openGeneralWorkspace":"Open project","projects.noDescription":"No project description yet. The project is still being shaped by recent activity and thread history.","projects.general.label":"General project","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.activate":"Activate","extensions.setup":"Setup","extensions.install":"Install","extensions.noCapabilities":"No capabilities","extensions.defaultName":"Extension","extensions.installedSuccess":"{name} installed","extensions.activatedSuccess":"{name} activated","extensions.removedSuccess":"{name} removed","extensions.installFailed":"Install failed","extensions.activationFailed":"Activation failed","extensions.removeFailed":"Remove failed","extensions.openingAuth":"Opening authentication...","extensions.configurationRequired":"Configuration required","extensions.getCredentials":"Get credentials","extensions.keepSecretPlaceholder":"\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (leave blank to keep)","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,Ek.createRoot)(document.getElementById("v2-root")).render(u`
  <${ov}>
    <${Bd} client=${Dt}>
      <${Ck} />
    <//>
  <//>
`);
