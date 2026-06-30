import{a as Dn,b as qe,c as Qe,d as p,e as u,f as ov,g as lv,h as $l,i as k,j as wl}from"./chunks/chunk-GE6TJDZP.js";var kv=Dn(Al=>{"use strict";var Qk=Symbol.for("react.transitional.element"),Vk=Symbol.for("react.fragment");function Rv(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:Qk,type:e,key:n,ref:t!==void 0?t:null,props:a}}Al.Fragment=Vk;Al.jsx=Rv;Al.jsxs=Rv});var Fd=Dn((P6,Cv)=>{"use strict";Cv.exports=kv()});var qv=Dn(Ue=>{"use strict";function Qd(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<zl(r,t))e[n]=t,e[a]=r,a=n;else break e}}function Ia(e){return e.length===0?null:e[0]}function ql(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],l=i+1,c=e[l];if(0>zl(o,a))l<r&&0>zl(c,o)?(e[n]=c,e[l]=a,n=l):(e[n]=o,e[i]=a,n=i);else if(l<r&&0>zl(c,a))e[n]=c,e[l]=a,n=l;else break e}}return t}function zl(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}Ue.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(Mv=performance,Ue.unstable_now=function(){return Mv.now()}):(Id=Date,Ov=Id.now(),Ue.unstable_now=function(){return Id.now()-Ov});var Mv,Id,Ov,un=[],Ln=[],Xk=1,ma=null,wt=3,Vd=!1,qi=!1,Ii=!1,Gd=!1,Uv=typeof setTimeout=="function"?setTimeout:null,jv=typeof clearTimeout=="function"?clearTimeout:null,Lv=typeof setImmediate<"u"?setImmediate:null;function Bl(e){for(var t=Ia(Ln);t!==null;){if(t.callback===null)ql(Ln);else if(t.startTime<=e)ql(Ln),t.sortIndex=t.expirationTime,Qd(un,t);else break;t=Ia(Ln)}}function Yd(e){if(Ii=!1,Bl(e),!qi)if(Ia(un)!==null)qi=!0,cs||(cs=!0,us());else{var t=Ia(Ln);t!==null&&Jd(Yd,t.startTime-e)}}var cs=!1,Hi=-1,Fv=5,zv=-1;function Bv(){return Gd?!0:!(Ue.unstable_now()-zv<Fv)}function Hd(){if(Gd=!1,cs){var e=Ue.unstable_now();zv=e;var t=!0;try{e:{qi=!1,Ii&&(Ii=!1,jv(Hi),Hi=-1),Vd=!0;var a=wt;try{t:{for(Bl(e),ma=Ia(un);ma!==null&&!(ma.expirationTime>e&&Bv());){var n=ma.callback;if(typeof n=="function"){ma.callback=null,wt=ma.priorityLevel;var r=n(ma.expirationTime<=e);if(e=Ue.unstable_now(),typeof r=="function"){ma.callback=r,Bl(e),t=!0;break t}ma===Ia(un)&&ql(un),Bl(e)}else ql(un);ma=Ia(un)}if(ma!==null)t=!0;else{var s=Ia(Ln);s!==null&&Jd(Yd,s.startTime-e),t=!1}}break e}finally{ma=null,wt=a,Vd=!1}t=void 0}}finally{t?us():cs=!1}}}var us;typeof Lv=="function"?us=function(){Lv(Hd)}:typeof MessageChannel<"u"?(Kd=new MessageChannel,Pv=Kd.port2,Kd.port1.onmessage=Hd,us=function(){Pv.postMessage(null)}):us=function(){Uv(Hd,0)};var Kd,Pv;function Jd(e,t){Hi=Uv(function(){e(Ue.unstable_now())},t)}Ue.unstable_IdlePriority=5;Ue.unstable_ImmediatePriority=1;Ue.unstable_LowPriority=4;Ue.unstable_NormalPriority=3;Ue.unstable_Profiling=null;Ue.unstable_UserBlockingPriority=2;Ue.unstable_cancelCallback=function(e){e.callback=null};Ue.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):Fv=0<e?Math.floor(1e3/e):5};Ue.unstable_getCurrentPriorityLevel=function(){return wt};Ue.unstable_next=function(e){switch(wt){case 1:case 2:case 3:var t=3;break;default:t=wt}var a=wt;wt=t;try{return e()}finally{wt=a}};Ue.unstable_requestPaint=function(){Gd=!0};Ue.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=wt;wt=e;try{return t()}finally{wt=a}};Ue.unstable_scheduleCallback=function(e,t,a){var n=Ue.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:Xk++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,Qd(Ln,e),Ia(un)===null&&e===Ia(Ln)&&(Ii?(jv(Hi),Hi=-1):Ii=!0,Jd(Yd,a-n))):(e.sortIndex=r,Qd(un,e),qi||Vd||(qi=!0,cs||(cs=!0,us()))),e};Ue.unstable_shouldYield=Bv;Ue.unstable_wrapCallback=function(e){var t=wt;return function(){var a=wt;wt=t;try{return e.apply(this,arguments)}finally{wt=a}}}});var Hv=Dn((yP,Iv)=>{"use strict";Iv.exports=qv()});var Qv=Dn(Tt=>{"use strict";var Wk=Qe();function Kv(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Pn(){}var Et={d:{f:Pn,r:function(){throw Error(Kv(522))},D:Pn,C:Pn,L:Pn,m:Pn,X:Pn,S:Pn,M:Pn},p:0,findDOMNode:null},Zk=Symbol.for("react.portal");function eC(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:Zk,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var Ki=Wk.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function Il(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}Tt.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=Et;Tt.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(Kv(299));return eC(e,t,null,a)};Tt.flushSync=function(e){var t=Ki.T,a=Et.p;try{if(Ki.T=null,Et.p=2,e)return e()}finally{Ki.T=t,Et.p=a,Et.d.f()}};Tt.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,Et.d.C(e,t))};Tt.prefetchDNS=function(e){typeof e=="string"&&Et.d.D(e)};Tt.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=Il(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?Et.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&Et.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};Tt.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=Il(t.as,t.crossOrigin);Et.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&Et.d.M(e)};Tt.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=Il(a,t.crossOrigin);Et.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};Tt.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=Il(t.as,t.crossOrigin);Et.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else Et.d.m(e)};Tt.requestFormReset=function(e){Et.d.r(e)};Tt.unstable_batchedUpdates=function(e,t){return e(t)};Tt.useFormState=function(e,t,a){return Ki.H.useFormState(e,t,a)};Tt.useFormStatus=function(){return Ki.H.useHostTransitionStatus()};Tt.version="19.1.0"});var Yv=Dn((xP,Gv)=>{"use strict";function Vv(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(Vv)}catch(e){console.error(e)}}Vv(),Gv.exports=Qv()});var Xx=Dn(dc=>{"use strict";var st=Hv(),gy=Qe(),tC=Yv();function j(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function yy(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function Mo(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function by(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function Jv(e){if(Mo(e)!==e)throw Error(j(188))}function aC(e){var t=e.alternate;if(!t){if(t=Mo(e),t===null)throw Error(j(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return Jv(r),e;if(s===n)return Jv(r),t;s=s.sibling}throw Error(j(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(j(189))}}if(a.alternate!==n)throw Error(j(190))}if(a.tag!==3)throw Error(j(188));return a.stateNode.current===a?e:t}function xy(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=xy(e),t!==null)return t;e=e.sibling}return null}var Me=Object.assign,nC=Symbol.for("react.element"),Hl=Symbol.for("react.transitional.element"),eo=Symbol.for("react.portal"),gs=Symbol.for("react.fragment"),$y=Symbol.for("react.strict_mode"),Cm=Symbol.for("react.profiler"),rC=Symbol.for("react.provider"),wy=Symbol.for("react.consumer"),pn=Symbol.for("react.context"),Nf=Symbol.for("react.forward_ref"),Em=Symbol.for("react.suspense"),Tm=Symbol.for("react.suspense_list"),_f=Symbol.for("react.memo"),Fn=Symbol.for("react.lazy");Symbol.for("react.scope");var Am=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var sC=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var Xv=Symbol.iterator;function Qi(e){return e===null||typeof e!="object"?null:(e=Xv&&e[Xv]||e["@@iterator"],typeof e=="function"?e:null)}var iC=Symbol.for("react.client.reference");function Dm(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===iC?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case gs:return"Fragment";case Cm:return"Profiler";case $y:return"StrictMode";case Em:return"Suspense";case Tm:return"SuspenseList";case Am:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case eo:return"Portal";case pn:return(e.displayName||"Context")+".Provider";case wy:return(e._context.displayName||"Context")+".Consumer";case Nf:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case _f:return t=e.displayName||null,t!==null?t:Dm(e.type)||"Memo";case Fn:t=e._payload,e=e._init;try{return Dm(e(t))}catch{}}return null}var to=Array.isArray,se=gy.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,be=tC.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,_r={pending:!1,data:null,method:null,action:null},Mm=[],ys=-1;function Ja(e){return{current:e}}function mt(e){0>ys||(e.current=Mm[ys],Mm[ys]=null,ys--)}function Fe(e,t){ys++,Mm[ys]=e.current,e.current=t}var Va=Ja(null),bo=Ja(null),Yn=Ja(null),xu=Ja(null);function $u(e,t){switch(Fe(Yn,t),Fe(bo,e),Fe(Va,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?ny(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=ny(t),e=Fx(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}mt(Va),Fe(Va,e)}function Us(){mt(Va),mt(bo),mt(Yn)}function Om(e){e.memoizedState!==null&&Fe(xu,e);var t=Va.current,a=Fx(t,e.type);t!==a&&(Fe(bo,e),Fe(Va,a))}function wu(e){bo.current===e&&(mt(Va),mt(bo)),xu.current===e&&(mt(xu),Eo._currentValue=_r)}var Lm=Object.prototype.hasOwnProperty,Rf=st.unstable_scheduleCallback,Xd=st.unstable_cancelCallback,oC=st.unstable_shouldYield,lC=st.unstable_requestPaint,Ga=st.unstable_now,uC=st.unstable_getCurrentPriorityLevel,Sy=st.unstable_ImmediatePriority,Ny=st.unstable_UserBlockingPriority,Su=st.unstable_NormalPriority,cC=st.unstable_LowPriority,_y=st.unstable_IdlePriority,dC=st.log,mC=st.unstable_setDisableYieldValue,Oo=null,Wt=null;function Kn(e){if(typeof dC=="function"&&mC(e),Wt&&typeof Wt.setStrictMode=="function")try{Wt.setStrictMode(Oo,e)}catch{}}var Zt=Math.clz32?Math.clz32:hC,fC=Math.log,pC=Math.LN2;function hC(e){return e>>>=0,e===0?32:31-(fC(e)/pC|0)|0}var Kl=256,Ql=4194304;function wr(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Ju(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=wr(n):(i&=o,i!==0?r=wr(i):a||(a=o&~e,a!==0&&(r=wr(a))))):(o=n&~s,o!==0?r=wr(o):i!==0?r=wr(i):a||(a=n&~e,a!==0&&(r=wr(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function Lo(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function vC(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function Ry(){var e=Kl;return Kl<<=1,(Kl&4194048)===0&&(Kl=256),e}function ky(){var e=Ql;return Ql<<=1,(Ql&62914560)===0&&(Ql=4194304),e}function Wd(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function Po(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function gC(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,l=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Zt(a),m=1<<d;o[d]=0,l[d]=-1;var f=c[d];if(f!==null)for(c[d]=null,d=0;d<f.length;d++){var h=f[d];h!==null&&(h.lane&=-536870913)}a&=~m}n!==0&&Cy(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function Cy(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Zt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function Ey(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Zt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function kf(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function Cf(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function Ty(){var e=be.p;return e!==0?e:(e=window.event,e===void 0?32:Yx(e.type))}function yC(e,t){var a=be.p;try{return be.p=e,t()}finally{be.p=a}}var ir=Math.random().toString(36).slice(2),St="__reactFiber$"+ir,qt="__reactProps$"+ir,Gs="__reactContainer$"+ir,Pm="__reactEvents$"+ir,bC="__reactListeners$"+ir,xC="__reactHandles$"+ir,Wv="__reactResources$"+ir,Uo="__reactMarker$"+ir;function Ef(e){delete e[St],delete e[qt],delete e[Pm],delete e[bC],delete e[xC]}function bs(e){var t=e[St];if(t)return t;for(var a=e.parentNode;a;){if(t=a[Gs]||a[St]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=iy(e);e!==null;){if(a=e[St])return a;e=iy(e)}return t}e=a,a=e.parentNode}return null}function Ys(e){if(e=e[St]||e[Gs]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function ao(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(j(33))}function Es(e){var t=e[Wv];return t||(t=e[Wv]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function ct(e){e[Uo]=!0}var Ay=new Set,Dy={};function Pr(e,t){js(e,t),js(e+"Capture",t)}function js(e,t){for(Dy[e]=t,e=0;e<t.length;e++)Ay.add(t[e])}var $C=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),Zv={},eg={};function wC(e){return Lm.call(eg,e)?!0:Lm.call(Zv,e)?!1:$C.test(e)?eg[e]=!0:(Zv[e]=!0,!1)}function ou(e,t,a){if(wC(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function Vl(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function cn(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var Zd,tg;function ps(e){if(Zd===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);Zd=t&&t[1]||"",tg=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+Zd+e+tg}var em=!1;function tm(e,t){if(!e||em)return"";em=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var m=function(){throw Error()};if(Object.defineProperty(m.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(m,[])}catch(h){var f=h}Reflect.construct(e,[],m)}else{try{m.call()}catch(h){f=h}e.call(m.prototype)}}else{try{throw Error()}catch(h){f=h}(m=e())&&typeof m.catch=="function"&&m.catch(function(){})}}catch(h){if(h&&f&&typeof h.stack=="string")return[h.stack,f.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var l=i.split(`
`),c=o.split(`
`);for(r=n=0;n<l.length&&!l[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===l.length||r===c.length)for(n=l.length-1,r=c.length-1;1<=n&&0<=r&&l[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(l[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||l[n]!==c[r]){var d=`
`+l[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{em=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?ps(a):""}function SC(e){switch(e.tag){case 26:case 27:case 5:return ps(e.type);case 16:return ps("Lazy");case 13:return ps("Suspense");case 19:return ps("SuspenseList");case 0:case 15:return tm(e.type,!1);case 11:return tm(e.type.render,!1);case 1:return tm(e.type,!0);case 31:return ps("Activity");default:return""}}function ag(e){try{var t="";do t+=SC(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function pa(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function My(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function NC(e){var t=My(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function Nu(e){e._valueTracker||(e._valueTracker=NC(e))}function Oy(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=My(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function _u(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var _C=/[\n"\\]/g;function ga(e){return e.replace(_C,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function Um(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+pa(t)):e.value!==""+pa(t)&&(e.value=""+pa(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?jm(e,i,pa(t)):a!=null?jm(e,i,pa(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+pa(o):e.removeAttribute("name")}function Ly(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+pa(a):"",t=t!=null?""+pa(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function jm(e,t,a){t==="number"&&_u(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function Ts(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+pa(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function Py(e,t,a){if(t!=null&&(t=""+pa(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+pa(a):""}function Uy(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(j(92));if(to(n)){if(1<n.length)throw Error(j(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=pa(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function Fs(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var RC=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function ng(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||RC.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function jy(e,t,a){if(t!=null&&typeof t!="object")throw Error(j(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&ng(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&ng(e,s,t[s])}function Tf(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var kC=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),CC=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function lu(e){return CC.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var Fm=null;function Af(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var xs=null,As=null;function rg(e){var t=Ys(e);if(t&&(e=t.stateNode)){var a=e[qt]||null;e:switch(e=t.stateNode,t.type){case"input":if(Um(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+ga(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[qt]||null;if(!r)throw Error(j(90));Um(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&Oy(n)}break e;case"textarea":Py(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&Ts(e,!!a.multiple,t,!1)}}}var am=!1;function Fy(e,t,a){if(am)return e(t,a);am=!0;try{var n=e(t);return n}finally{if(am=!1,(xs!==null||As!==null)&&(ic(),xs&&(t=xs,e=As,As=xs=null,rg(t),e)))for(t=0;t<e.length;t++)rg(e[t])}}function xo(e,t){var a=e.stateNode;if(a===null)return null;var n=a[qt]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(j(231,t,typeof a));return a}var $n=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),zm=!1;if($n)try{ds={},Object.defineProperty(ds,"passive",{get:function(){zm=!0}}),window.addEventListener("test",ds,ds),window.removeEventListener("test",ds,ds)}catch{zm=!1}var ds,Qn=null,Df=null,uu=null;function zy(){if(uu)return uu;var e,t=Df,a=t.length,n,r="value"in Qn?Qn.value:Qn.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return uu=r.slice(e,1<n?1-n:void 0)}function cu(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function Gl(){return!0}function sg(){return!1}function It(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?Gl:sg,this.isPropagationStopped=sg,this}return Me(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=Gl)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=Gl)},persist:function(){},isPersistent:Gl}),t}var Ur={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Xu=It(Ur),jo=Me({},Ur,{view:0,detail:0}),EC=It(jo),nm,rm,Vi,Wu=Me({},jo,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:Mf,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==Vi&&(Vi&&e.type==="mousemove"?(nm=e.screenX-Vi.screenX,rm=e.screenY-Vi.screenY):rm=nm=0,Vi=e),nm)},movementY:function(e){return"movementY"in e?e.movementY:rm}}),ig=It(Wu),TC=Me({},Wu,{dataTransfer:0}),AC=It(TC),DC=Me({},jo,{relatedTarget:0}),sm=It(DC),MC=Me({},Ur,{animationName:0,elapsedTime:0,pseudoElement:0}),OC=It(MC),LC=Me({},Ur,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),PC=It(LC),UC=Me({},Ur,{data:0}),og=It(UC),jC={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},FC={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},zC={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function BC(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=zC[e])?!!t[e]:!1}function Mf(){return BC}var qC=Me({},jo,{key:function(e){if(e.key){var t=jC[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=cu(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?FC[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:Mf,charCode:function(e){return e.type==="keypress"?cu(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?cu(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),IC=It(qC),HC=Me({},Wu,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),lg=It(HC),KC=Me({},jo,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:Mf}),QC=It(KC),VC=Me({},Ur,{propertyName:0,elapsedTime:0,pseudoElement:0}),GC=It(VC),YC=Me({},Wu,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),JC=It(YC),XC=Me({},Ur,{newState:0,oldState:0}),WC=It(XC),ZC=[9,13,27,32],Of=$n&&"CompositionEvent"in window,ro=null;$n&&"documentMode"in document&&(ro=document.documentMode);var eE=$n&&"TextEvent"in window&&!ro,By=$n&&(!Of||ro&&8<ro&&11>=ro),ug=" ",cg=!1;function qy(e,t){switch(e){case"keyup":return ZC.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function Iy(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var $s=!1;function tE(e,t){switch(e){case"compositionend":return Iy(t);case"keypress":return t.which!==32?null:(cg=!0,ug);case"textInput":return e=t.data,e===ug&&cg?null:e;default:return null}}function aE(e,t){if($s)return e==="compositionend"||!Of&&qy(e,t)?(e=zy(),uu=Df=Qn=null,$s=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return By&&t.locale!=="ko"?null:t.data;default:return null}}var nE={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function dg(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!nE[e.type]:t==="textarea"}function Hy(e,t,a,n){xs?As?As.push(n):As=[n]:xs=n,t=Iu(t,"onChange"),0<t.length&&(a=new Xu("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var so=null,$o=null;function rE(e){Px(e,0)}function Zu(e){var t=ao(e);if(Oy(t))return e}function mg(e,t){if(e==="change")return t}var Ky=!1;$n&&($n?(Jl="oninput"in document,Jl||(im=document.createElement("div"),im.setAttribute("oninput","return;"),Jl=typeof im.oninput=="function"),Yl=Jl):Yl=!1,Ky=Yl&&(!document.documentMode||9<document.documentMode));var Yl,Jl,im;function fg(){so&&(so.detachEvent("onpropertychange",Qy),$o=so=null)}function Qy(e){if(e.propertyName==="value"&&Zu($o)){var t=[];Hy(t,$o,e,Af(e)),Fy(rE,t)}}function sE(e,t,a){e==="focusin"?(fg(),so=t,$o=a,so.attachEvent("onpropertychange",Qy)):e==="focusout"&&fg()}function iE(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return Zu($o)}function oE(e,t){if(e==="click")return Zu(t)}function lE(e,t){if(e==="input"||e==="change")return Zu(t)}function uE(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var aa=typeof Object.is=="function"?Object.is:uE;function wo(e,t){if(aa(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!Lm.call(t,r)||!aa(e[r],t[r]))return!1}return!0}function pg(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function hg(e,t){var a=pg(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=pg(a)}}function Vy(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?Vy(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function Gy(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=_u(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=_u(e.document)}return t}function Lf(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var cE=$n&&"documentMode"in document&&11>=document.documentMode,ws=null,Bm=null,io=null,qm=!1;function vg(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;qm||ws==null||ws!==_u(n)||(n=ws,"selectionStart"in n&&Lf(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),io&&wo(io,n)||(io=n,n=Iu(Bm,"onSelect"),0<n.length&&(t=new Xu("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=ws)))}function $r(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var Ss={animationend:$r("Animation","AnimationEnd"),animationiteration:$r("Animation","AnimationIteration"),animationstart:$r("Animation","AnimationStart"),transitionrun:$r("Transition","TransitionRun"),transitionstart:$r("Transition","TransitionStart"),transitioncancel:$r("Transition","TransitionCancel"),transitionend:$r("Transition","TransitionEnd")},om={},Yy={};$n&&(Yy=document.createElement("div").style,"AnimationEvent"in window||(delete Ss.animationend.animation,delete Ss.animationiteration.animation,delete Ss.animationstart.animation),"TransitionEvent"in window||delete Ss.transitionend.transition);function jr(e){if(om[e])return om[e];if(!Ss[e])return e;var t=Ss[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in Yy)return om[e]=t[a];return e}var Jy=jr("animationend"),Xy=jr("animationiteration"),Wy=jr("animationstart"),dE=jr("transitionrun"),mE=jr("transitionstart"),fE=jr("transitioncancel"),Zy=jr("transitionend"),eb=new Map,Im="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");Im.push("scrollEnd");function Ea(e,t){eb.set(e,t),Pr(t,[e])}var gg=new WeakMap;function ya(e,t){if(typeof e=="object"&&e!==null){var a=gg.get(e);return a!==void 0?a:(t={value:e,source:t,stack:ag(t)},gg.set(e,t),t)}return{value:e,source:t,stack:ag(t)}}var fa=[],Ns=0,Pf=0;function ec(){for(var e=Ns,t=Pf=Ns=0;t<e;){var a=fa[t];fa[t++]=null;var n=fa[t];fa[t++]=null;var r=fa[t];fa[t++]=null;var s=fa[t];if(fa[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&tb(a,r,s)}}function tc(e,t,a,n){fa[Ns++]=e,fa[Ns++]=t,fa[Ns++]=a,fa[Ns++]=n,Pf|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function Uf(e,t,a,n){return tc(e,t,a,n),Ru(e)}function Js(e,t){return tc(e,null,null,t),Ru(e)}function tb(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Zt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function Ru(e){if(50<go)throw go=0,df=null,Error(j(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var _s={};function pE(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Xt(e,t,a,n){return new pE(e,t,a,n)}function jf(e){return e=e.prototype,!(!e||!e.isReactComponent)}function bn(e,t){var a=e.alternate;return a===null?(a=Xt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function ab(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function du(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")jf(e)&&(i=1);else if(typeof e=="string")i=p3(e,a,Va.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case Am:return e=Xt(31,a,t,r),e.elementType=Am,e.lanes=s,e;case gs:return Rr(a.children,r,s,t);case $y:i=8,r|=24;break;case Cm:return e=Xt(12,a,t,r|2),e.elementType=Cm,e.lanes=s,e;case Em:return e=Xt(13,a,t,r),e.elementType=Em,e.lanes=s,e;case Tm:return e=Xt(19,a,t,r),e.elementType=Tm,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case rC:case pn:i=10;break e;case wy:i=9;break e;case Nf:i=11;break e;case _f:i=14;break e;case Fn:i=16,n=null;break e}i=29,a=Error(j(130,e===null?"null":typeof e,"")),n=null}return t=Xt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function Rr(e,t,a,n){return e=Xt(7,e,n,t),e.lanes=a,e}function lm(e,t,a){return e=Xt(6,e,null,t),e.lanes=a,e}function um(e,t,a){return t=Xt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var Rs=[],ks=0,ku=null,Cu=0,ha=[],va=0,kr=null,hn=1,vn="";function Sr(e,t){Rs[ks++]=Cu,Rs[ks++]=ku,ku=e,Cu=t}function nb(e,t,a){ha[va++]=hn,ha[va++]=vn,ha[va++]=kr,kr=e;var n=hn;e=vn;var r=32-Zt(n)-1;n&=~(1<<r),a+=1;var s=32-Zt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,hn=1<<32-Zt(t)+r|a<<r|n,vn=s+e}else hn=1<<s|a<<r|n,vn=e}function Ff(e){e.return!==null&&(Sr(e,1),nb(e,1,0))}function zf(e){for(;e===ku;)ku=Rs[--ks],Rs[ks]=null,Cu=Rs[--ks],Rs[ks]=null;for(;e===kr;)kr=ha[--va],ha[va]=null,vn=ha[--va],ha[va]=null,hn=ha[--va],ha[va]=null}var At=null,Ie=null,ye=!1,Cr=null,Ka=!1,Hm=Error(j(519));function Dr(e){var t=Error(j(418,""));throw So(ya(t,e)),Hm}function yg(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[St]=e,t[qt]=n,a){case"dialog":de("cancel",t),de("close",t);break;case"iframe":case"object":case"embed":de("load",t);break;case"video":case"audio":for(a=0;a<Ro.length;a++)de(Ro[a],t);break;case"source":de("error",t);break;case"img":case"image":case"link":de("error",t),de("load",t);break;case"details":de("toggle",t);break;case"input":de("invalid",t),Ly(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),Nu(t);break;case"select":de("invalid",t);break;case"textarea":de("invalid",t),Uy(t,n.value,n.defaultValue,n.children),Nu(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||jx(t.textContent,a)?(n.popover!=null&&(de("beforetoggle",t),de("toggle",t)),n.onScroll!=null&&de("scroll",t),n.onScrollEnd!=null&&de("scrollend",t),n.onClick!=null&&(t.onclick=uc),t=!0):t=!1,t||Dr(e)}function bg(e){for(At=e.return;At;)switch(At.tag){case 5:case 13:Ka=!1;return;case 27:case 3:Ka=!0;return;default:At=At.return}}function Gi(e){if(e!==At)return!1;if(!ye)return bg(e),ye=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||gf(e.type,e.memoizedProps)),a=!a),a&&Ie&&Dr(e),bg(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(j(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){Ie=Ca(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}Ie=null}}else t===27?(t=Ie,or(e.type)?(e=xf,xf=null,Ie=e):Ie=t):Ie=At?Ca(e.stateNode.nextSibling):null;return!0}function Fo(){Ie=At=null,ye=!1}function xg(){var e=Cr;return e!==null&&(Bt===null?Bt=e:Bt.push.apply(Bt,e),Cr=null),e}function So(e){Cr===null?Cr=[e]:Cr.push(e)}var Km=Ja(null),Fr=null,gn=null;function Bn(e,t,a){Fe(Km,t._currentValue),t._currentValue=a}function xn(e){e._currentValue=Km.current,mt(Km)}function Qm(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function Vm(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var l=0;l<t.length;l++)if(o.context===t[l]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),Qm(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(j(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),Qm(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function zo(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(j(387));if(i=i.memoizedProps,i!==null){var o=r.type;aa(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===xu.current){if(i=r.alternate,i===null)throw Error(j(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(Eo):e=[Eo])}r=r.return}e!==null&&Vm(t,e,a,n),t.flags|=262144}function Eu(e){for(e=e.firstContext;e!==null;){if(!aa(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function Mr(e){Fr=e,gn=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function Nt(e){return rb(Fr,e)}function Xl(e,t){return Fr===null&&Mr(e),rb(e,t)}function rb(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},gn===null){if(e===null)throw Error(j(308));gn=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else gn=gn.next=t;return a}var hE=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},vE=st.unstable_scheduleCallback,gE=st.unstable_NormalPriority,nt={$$typeof:pn,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function Bf(){return{controller:new hE,data:new Map,refCount:0}}function Bo(e){e.refCount--,e.refCount===0&&vE(gE,function(){e.controller.abort()})}var oo=null,Gm=0,zs=0,Ds=null;function yE(e,t){if(oo===null){var a=oo=[];Gm=0,zs=cp(),Ds={status:"pending",value:void 0,then:function(n){a.push(n)}}}return Gm++,t.then($g,$g),t}function $g(){if(--Gm===0&&oo!==null){Ds!==null&&(Ds.status="fulfilled");var e=oo;oo=null,zs=0,Ds=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function bE(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var wg=se.S;se.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&yE(e,t),wg!==null&&wg(e,t)};var Er=Ja(null);function qf(){var e=Er.current;return e!==null?e:Ee.pooledCache}function mu(e,t){t===null?Fe(Er,Er.current):Fe(Er,t.pool)}function sb(){var e=qf();return e===null?null:{parent:nt._currentValue,pool:e}}var qo=Error(j(460)),ib=Error(j(474)),ac=Error(j(542)),Ym={then:function(){}};function Sg(e){return e=e.status,e==="fulfilled"||e==="rejected"}function Wl(){}function ob(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(Wl,Wl),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,_g(e),e;default:if(typeof t.status=="string")t.then(Wl,Wl);else{if(e=Ee,e!==null&&100<e.shellSuspendCounter)throw Error(j(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,_g(e),e}throw lo=t,qo}}var lo=null;function Ng(){if(lo===null)throw Error(j(459));var e=lo;return lo=null,e}function _g(e){if(e===qo||e===ac)throw Error(j(483))}var zn=!1;function If(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function Jm(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function Jn(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function Xn(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,(Se&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=Ru(e),tb(e,null,a),t}return tc(e,n,t,a),Ru(e)}function uo(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,Ey(e,a)}}function cm(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var Xm=!1;function co(){if(Xm){var e=Ds;if(e!==null)throw e}}function mo(e,t,a,n){Xm=!1;var r=e.updateQueue;zn=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var l=o,c=l.next;l.next=null,i===null?s=c:i.next=c,i=l;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=l))}if(s!==null){var m=r.baseState;i=0,d=c=l=null,o=s;do{var f=o.lane&-536870913,h=f!==o.lane;if(h?(he&f)===f:(n&f)===f){f!==0&&f===zs&&(Xm=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var x=e,y=o;f=t;var $=a;switch(y.tag){case 1:if(x=y.payload,typeof x=="function"){m=x.call($,m,f);break e}m=x;break e;case 3:x.flags=x.flags&-65537|128;case 0:if(x=y.payload,f=typeof x=="function"?x.call($,m,f):x,f==null)break e;m=Me({},m,f);break e;case 2:zn=!0}}f=o.callback,f!==null&&(e.flags|=64,h&&(e.flags|=8192),h=r.callbacks,h===null?r.callbacks=[f]:h.push(f))}else h={lane:f,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=h,l=m):d=d.next=h,i|=f;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;h=o,o=h.next,h.next=null,r.lastBaseUpdate=h,r.shared.pending=null}}while(!0);d===null&&(l=m),r.baseState=l,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),sr|=i,e.lanes=i,e.memoizedState=m}}function lb(e,t){if(typeof e!="function")throw Error(j(191,e));e.call(t)}function ub(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)lb(a[e],t)}var Bs=Ja(null),Tu=Ja(0);function Rg(e,t){e=Nn,Fe(Tu,e),Fe(Bs,t),Nn=e|t.baseLanes}function Wm(){Fe(Tu,Nn),Fe(Bs,Bs.current)}function Hf(){Nn=Tu.current,mt(Bs),mt(Tu)}var nr=0,ue=null,_e=null,Je=null,Au=!1,Ms=!1,Or=!1,Du=0,No=0,Os=null,xE=0;function Ve(){throw Error(j(321))}function Kf(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!aa(e[a],t[a]))return!1;return!0}function Qf(e,t,a,n,r,s){return nr=s,ue=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,se.H=e===null||e.memoizedState===null?zb:Bb,Or=!1,s=a(n,r),Or=!1,Ms&&(s=db(t,a,n,r)),cb(e),s}function cb(e){se.H=Mu;var t=_e!==null&&_e.next!==null;if(nr=0,Je=_e=ue=null,Au=!1,No=0,Os=null,t)throw Error(j(300));e===null||dt||(e=e.dependencies,e!==null&&Eu(e)&&(dt=!0))}function db(e,t,a,n){ue=e;var r=0;do{if(Ms&&(Os=null),No=0,Ms=!1,25<=r)throw Error(j(301));if(r+=1,Je=_e=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}se.H=kE,s=t(a,n)}while(Ms);return s}function $E(){var e=se.H,t=e.useState()[0];return t=typeof t.then=="function"?Io(t):t,e=e.useState()[0],(_e!==null?_e.memoizedState:null)!==e&&(ue.flags|=1024),t}function Vf(){var e=Du!==0;return Du=0,e}function Gf(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function Yf(e){if(Au){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}Au=!1}nr=0,Je=_e=ue=null,Ms=!1,No=Du=0,Os=null}function Ft(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return Je===null?ue.memoizedState=Je=e:Je=Je.next=e,Je}function Xe(){if(_e===null){var e=ue.alternate;e=e!==null?e.memoizedState:null}else e=_e.next;var t=Je===null?ue.memoizedState:Je.next;if(t!==null)Je=t,_e=e;else{if(e===null)throw ue.alternate===null?Error(j(467)):Error(j(310));_e=e,e={memoizedState:_e.memoizedState,baseState:_e.baseState,baseQueue:_e.baseQueue,queue:_e.queue,next:null},Je===null?ue.memoizedState=Je=e:Je=Je.next=e}return Je}function Jf(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function Io(e){var t=No;return No+=1,Os===null&&(Os=[]),e=ob(Os,e,t),t=ue,(Je===null?t.memoizedState:Je.next)===null&&(t=t.alternate,se.H=t===null||t.memoizedState===null?zb:Bb),e}function nc(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return Io(e);if(e.$$typeof===pn)return Nt(e)}throw Error(j(438,String(e)))}function Xf(e){var t=null,a=ue.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=ue.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=Jf(),ue.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=sC;return t.index++,a}function wn(e,t){return typeof t=="function"?t(e):t}function fu(e){var t=Xe();return Wf(t,_e,e)}function Wf(e,t,a){var n=e.queue;if(n===null)throw Error(j(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,l=null,c=t,d=!1;do{var m=c.lane&-536870913;if(m!==c.lane?(he&m)===m:(nr&m)===m){var f=c.revertLane;if(f===0)l!==null&&(l=l.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),m===zs&&(d=!0);else if((nr&f)===f){c=c.next,f===zs&&(d=!0);continue}else m={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},l===null?(o=l=m,i=s):l=l.next=m,ue.lanes|=f,sr|=f;m=c.action,Or&&a(s,m),s=c.hasEagerState?c.eagerState:a(s,m)}else f={lane:m,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},l===null?(o=l=f,i=s):l=l.next=f,ue.lanes|=m,sr|=m;c=c.next}while(c!==null&&c!==t);if(l===null?i=s:l.next=o,!aa(s,e.memoizedState)&&(dt=!0,d&&(a=Ds,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=l,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function dm(e){var t=Xe(),a=t.queue;if(a===null)throw Error(j(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);aa(s,t.memoizedState)||(dt=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function mb(e,t,a){var n=ue,r=Xe(),s=ye;if(s){if(a===void 0)throw Error(j(407));a=a()}else a=t();var i=!aa((_e||r).memoizedState,a);i&&(r.memoizedState=a,dt=!0),r=r.queue;var o=hb.bind(null,n,r,e);if(Ho(2048,8,o,[e]),r.getSnapshot!==t||i||Je!==null&&Je.memoizedState.tag&1){if(n.flags|=2048,qs(9,rc(),pb.bind(null,n,r,a,t),null),Ee===null)throw Error(j(349));s||(nr&124)!==0||fb(n,t,a)}return a}function fb(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=ue.updateQueue,t===null?(t=Jf(),ue.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function pb(e,t,a,n){t.value=a,t.getSnapshot=n,vb(t)&&gb(e)}function hb(e,t,a){return a(function(){vb(t)&&gb(e)})}function vb(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!aa(e,a)}catch{return!0}}function gb(e){var t=Js(e,2);t!==null&&ta(t,e,2)}function Zm(e){var t=Ft();if(typeof e=="function"){var a=e;if(e=a(),Or){Kn(!0);try{a()}finally{Kn(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:wn,lastRenderedState:e},t}function yb(e,t,a,n){return e.baseState=a,Wf(e,_e,typeof n=="function"?n:wn)}function wE(e,t,a,n,r){if(sc(e))throw Error(j(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};se.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,bb(t,s)):(s.next=a.next,t.pending=a.next=s)}}function bb(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=se.T,i={};se.T=i;try{var o=a(r,n),l=se.S;l!==null&&l(i,o),kg(e,t,o)}catch(c){ef(e,t,c)}finally{se.T=s}}else try{s=a(r,n),kg(e,t,s)}catch(c){ef(e,t,c)}}function kg(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){Cg(e,t,n)},function(n){return ef(e,t,n)}):Cg(e,t,a)}function Cg(e,t,a){t.status="fulfilled",t.value=a,xb(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,bb(e,a)))}function ef(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,xb(t),t=t.next;while(t!==n)}e.action=null}function xb(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function $b(e,t){return t}function Eg(e,t){if(ye){var a=Ee.formState;if(a!==null){e:{var n=ue;if(ye){if(Ie){t:{for(var r=Ie,s=Ka;r.nodeType!==8;){if(!s){r=null;break t}if(r=Ca(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){Ie=Ca(r.nextSibling),n=r.data==="F!";break e}}Dr(n)}n=!1}n&&(t=a[0])}}return a=Ft(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:$b,lastRenderedState:t},a.queue=n,a=Ub.bind(null,ue,n),n.dispatch=a,n=Zm(!1),s=ap.bind(null,ue,!1,n.queue),n=Ft(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=wE.bind(null,ue,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function Tg(e){var t=Xe();return wb(t,_e,e)}function wb(e,t,a){if(t=Wf(e,t,$b)[0],e=fu(wn)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=Io(t)}catch(i){throw i===qo?ac:i}else n=t;t=Xe();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(ue.flags|=2048,qs(9,rc(),SE.bind(null,r,a),null)),[n,s,e]}function SE(e,t){e.action=t}function Ag(e){var t=Xe(),a=_e;if(a!==null)return wb(t,a,e);Xe(),t=t.memoizedState,a=Xe();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function qs(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=ue.updateQueue,t===null&&(t=Jf(),ue.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function rc(){return{destroy:void 0,resource:void 0}}function Sb(){return Xe().memoizedState}function pu(e,t,a,n){var r=Ft();n=n===void 0?null:n,ue.flags|=e,r.memoizedState=qs(1|t,rc(),a,n)}function Ho(e,t,a,n){var r=Xe();n=n===void 0?null:n;var s=r.memoizedState.inst;_e!==null&&n!==null&&Kf(n,_e.memoizedState.deps)?r.memoizedState=qs(t,s,a,n):(ue.flags|=e,r.memoizedState=qs(1|t,s,a,n))}function Dg(e,t){pu(8390656,8,e,t)}function Nb(e,t){Ho(2048,8,e,t)}function _b(e,t){return Ho(4,2,e,t)}function Rb(e,t){return Ho(4,4,e,t)}function kb(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function Cb(e,t,a){a=a!=null?a.concat([e]):null,Ho(4,4,kb.bind(null,t,e),a)}function Zf(){}function Eb(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&Kf(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function Tb(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&Kf(t,n[1]))return n[0];if(n=e(),Or){Kn(!0);try{e()}finally{Kn(!1)}}return a.memoizedState=[n,t],n}function ep(e,t,a){return a===void 0||(nr&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=xx(),ue.lanes|=e,sr|=e,a)}function Ab(e,t,a,n){return aa(a,t)?a:Bs.current!==null?(e=ep(e,a,n),aa(e,t)||(dt=!0),e):(nr&42)===0?(dt=!0,e.memoizedState=a):(e=xx(),ue.lanes|=e,sr|=e,t)}function Db(e,t,a,n,r){var s=be.p;be.p=s!==0&&8>s?s:8;var i=se.T,o={};se.T=o,ap(e,!1,t,a);try{var l=r(),c=se.S;if(c!==null&&c(o,l),l!==null&&typeof l=="object"&&typeof l.then=="function"){var d=bE(l,n);fo(e,t,d,ea(e))}else fo(e,t,n,ea(e))}catch(m){fo(e,t,{then:function(){},status:"rejected",reason:m},ea())}finally{be.p=s,se.T=i}}function NE(){}function tf(e,t,a,n){if(e.tag!==5)throw Error(j(476));var r=Mb(e).queue;Db(e,r,t,_r,a===null?NE:function(){return Ob(e),a(n)})}function Mb(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:_r,baseState:_r,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:wn,lastRenderedState:_r},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:wn,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function Ob(e){var t=Mb(e).next.queue;fo(e,t,{},ea())}function tp(){return Nt(Eo)}function Lb(){return Xe().memoizedState}function Pb(){return Xe().memoizedState}function _E(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=ea();e=Jn(a);var n=Xn(t,e,a);n!==null&&(ta(n,t,a),uo(n,t,a)),t={cache:Bf()},e.payload=t;return}t=t.return}}function RE(e,t,a){var n=ea();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},sc(e)?jb(t,a):(a=Uf(e,t,a,n),a!==null&&(ta(a,e,n),Fb(a,t,n)))}function Ub(e,t,a){var n=ea();fo(e,t,a,n)}function fo(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(sc(e))jb(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,aa(o,i))return tc(e,t,r,0),Ee===null&&ec(),!1}catch{}finally{}if(a=Uf(e,t,r,n),a!==null)return ta(a,e,n),Fb(a,t,n),!0}return!1}function ap(e,t,a,n){if(n={lane:2,revertLane:cp(),action:n,hasEagerState:!1,eagerState:null,next:null},sc(e)){if(t)throw Error(j(479))}else t=Uf(e,a,n,2),t!==null&&ta(t,e,2)}function sc(e){var t=e.alternate;return e===ue||t!==null&&t===ue}function jb(e,t){Ms=Au=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function Fb(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,Ey(e,a)}}var Mu={readContext:Nt,use:nc,useCallback:Ve,useContext:Ve,useEffect:Ve,useImperativeHandle:Ve,useLayoutEffect:Ve,useInsertionEffect:Ve,useMemo:Ve,useReducer:Ve,useRef:Ve,useState:Ve,useDebugValue:Ve,useDeferredValue:Ve,useTransition:Ve,useSyncExternalStore:Ve,useId:Ve,useHostTransitionStatus:Ve,useFormState:Ve,useActionState:Ve,useOptimistic:Ve,useMemoCache:Ve,useCacheRefresh:Ve},zb={readContext:Nt,use:nc,useCallback:function(e,t){return Ft().memoizedState=[e,t===void 0?null:t],e},useContext:Nt,useEffect:Dg,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,pu(4194308,4,kb.bind(null,t,e),a)},useLayoutEffect:function(e,t){return pu(4194308,4,e,t)},useInsertionEffect:function(e,t){pu(4,2,e,t)},useMemo:function(e,t){var a=Ft();t=t===void 0?null:t;var n=e();if(Or){Kn(!0);try{e()}finally{Kn(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=Ft();if(a!==void 0){var r=a(t);if(Or){Kn(!0);try{a(t)}finally{Kn(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=RE.bind(null,ue,e),[n.memoizedState,e]},useRef:function(e){var t=Ft();return e={current:e},t.memoizedState=e},useState:function(e){e=Zm(e);var t=e.queue,a=Ub.bind(null,ue,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:Zf,useDeferredValue:function(e,t){var a=Ft();return ep(a,e,t)},useTransition:function(){var e=Zm(!1);return e=Db.bind(null,ue,e.queue,!0,!1),Ft().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=ue,r=Ft();if(ye){if(a===void 0)throw Error(j(407));a=a()}else{if(a=t(),Ee===null)throw Error(j(349));(he&124)!==0||fb(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,Dg(hb.bind(null,n,s,e),[e]),n.flags|=2048,qs(9,rc(),pb.bind(null,n,s,a,t),null),a},useId:function(){var e=Ft(),t=Ee.identifierPrefix;if(ye){var a=vn,n=hn;a=(n&~(1<<32-Zt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=Du++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=xE++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:tp,useFormState:Eg,useActionState:Eg,useOptimistic:function(e){var t=Ft();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=ap.bind(null,ue,!0,a),a.dispatch=t,[e,t]},useMemoCache:Xf,useCacheRefresh:function(){return Ft().memoizedState=_E.bind(null,ue)}},Bb={readContext:Nt,use:nc,useCallback:Eb,useContext:Nt,useEffect:Nb,useImperativeHandle:Cb,useInsertionEffect:_b,useLayoutEffect:Rb,useMemo:Tb,useReducer:fu,useRef:Sb,useState:function(){return fu(wn)},useDebugValue:Zf,useDeferredValue:function(e,t){var a=Xe();return Ab(a,_e.memoizedState,e,t)},useTransition:function(){var e=fu(wn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:Io(e),t]},useSyncExternalStore:mb,useId:Lb,useHostTransitionStatus:tp,useFormState:Tg,useActionState:Tg,useOptimistic:function(e,t){var a=Xe();return yb(a,_e,e,t)},useMemoCache:Xf,useCacheRefresh:Pb},kE={readContext:Nt,use:nc,useCallback:Eb,useContext:Nt,useEffect:Nb,useImperativeHandle:Cb,useInsertionEffect:_b,useLayoutEffect:Rb,useMemo:Tb,useReducer:dm,useRef:Sb,useState:function(){return dm(wn)},useDebugValue:Zf,useDeferredValue:function(e,t){var a=Xe();return _e===null?ep(a,e,t):Ab(a,_e.memoizedState,e,t)},useTransition:function(){var e=dm(wn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:Io(e),t]},useSyncExternalStore:mb,useId:Lb,useHostTransitionStatus:tp,useFormState:Ag,useActionState:Ag,useOptimistic:function(e,t){var a=Xe();return _e!==null?yb(a,_e,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:Xf,useCacheRefresh:Pb},Ls=null,_o=0;function Zl(e){var t=_o;return _o+=1,Ls===null&&(Ls=[]),ob(Ls,e,t)}function Yi(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function eu(e,t){throw t.$$typeof===nC?Error(j(525)):(e=Object.prototype.toString.call(t),Error(j(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function Mg(e){var t=e._init;return t(e._payload)}function qb(e){function t(g,v){if(e){var b=g.deletions;b===null?(g.deletions=[v],g.flags|=16):b.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=bn(g,v),g.index=0,g.sibling=null,g}function s(g,v,b){return g.index=b,e?(b=g.alternate,b!==null?(b=b.index,b<v?(g.flags|=67108866,v):b):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,b,w){return v===null||v.tag!==6?(v=lm(b,g.mode,w),v.return=g,v):(v=r(v,b),v.return=g,v)}function l(g,v,b,w){var S=b.type;return S===gs?d(g,v,b.props.children,w,b.key):v!==null&&(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Fn&&Mg(S)===v.type)?(v=r(v,b.props),Yi(v,b),v.return=g,v):(v=du(b.type,b.key,b.props,null,g.mode,w),Yi(v,b),v.return=g,v)}function c(g,v,b,w){return v===null||v.tag!==4||v.stateNode.containerInfo!==b.containerInfo||v.stateNode.implementation!==b.implementation?(v=um(b,g.mode,w),v.return=g,v):(v=r(v,b.children||[]),v.return=g,v)}function d(g,v,b,w,S){return v===null||v.tag!==7?(v=Rr(b,g.mode,w,S),v.return=g,v):(v=r(v,b),v.return=g,v)}function m(g,v,b){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=lm(""+v,g.mode,b),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case Hl:return b=du(v.type,v.key,v.props,null,g.mode,b),Yi(b,v),b.return=g,b;case eo:return v=um(v,g.mode,b),v.return=g,v;case Fn:var w=v._init;return v=w(v._payload),m(g,v,b)}if(to(v)||Qi(v))return v=Rr(v,g.mode,b,null),v.return=g,v;if(typeof v.then=="function")return m(g,Zl(v),b);if(v.$$typeof===pn)return m(g,Xl(g,v),b);eu(g,v)}return null}function f(g,v,b,w){var S=v!==null?v.key:null;if(typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint")return S!==null?null:o(g,v,""+b,w);if(typeof b=="object"&&b!==null){switch(b.$$typeof){case Hl:return b.key===S?l(g,v,b,w):null;case eo:return b.key===S?c(g,v,b,w):null;case Fn:return S=b._init,b=S(b._payload),f(g,v,b,w)}if(to(b)||Qi(b))return S!==null?null:d(g,v,b,w,null);if(typeof b.then=="function")return f(g,v,Zl(b),w);if(b.$$typeof===pn)return f(g,v,Xl(g,b),w);eu(g,b)}return null}function h(g,v,b,w,S){if(typeof w=="string"&&w!==""||typeof w=="number"||typeof w=="bigint")return g=g.get(b)||null,o(v,g,""+w,S);if(typeof w=="object"&&w!==null){switch(w.$$typeof){case Hl:return g=g.get(w.key===null?b:w.key)||null,l(v,g,w,S);case eo:return g=g.get(w.key===null?b:w.key)||null,c(v,g,w,S);case Fn:var C=w._init;return w=C(w._payload),h(g,v,b,w,S)}if(to(w)||Qi(w))return g=g.get(b)||null,d(v,g,w,S,null);if(typeof w.then=="function")return h(g,v,b,Zl(w),S);if(w.$$typeof===pn)return h(g,v,b,Xl(v,w),S);eu(v,w)}return null}function x(g,v,b,w){for(var S=null,C=null,R=v,_=v=0,M=null;R!==null&&_<b.length;_++){R.index>_?(M=R,R=null):M=R.sibling;var L=f(g,R,b[_],w);if(L===null){R===null&&(R=M);break}e&&R&&L.alternate===null&&t(g,R),v=s(L,v,_),C===null?S=L:C.sibling=L,C=L,R=M}if(_===b.length)return a(g,R),ye&&Sr(g,_),S;if(R===null){for(;_<b.length;_++)R=m(g,b[_],w),R!==null&&(v=s(R,v,_),C===null?S=R:C.sibling=R,C=R);return ye&&Sr(g,_),S}for(R=n(R);_<b.length;_++)M=h(R,g,_,b[_],w),M!==null&&(e&&M.alternate!==null&&R.delete(M.key===null?_:M.key),v=s(M,v,_),C===null?S=M:C.sibling=M,C=M);return e&&R.forEach(function(U){return t(g,U)}),ye&&Sr(g,_),S}function y(g,v,b,w){if(b==null)throw Error(j(151));for(var S=null,C=null,R=v,_=v=0,M=null,L=b.next();R!==null&&!L.done;_++,L=b.next()){R.index>_?(M=R,R=null):M=R.sibling;var U=f(g,R,L.value,w);if(U===null){R===null&&(R=M);break}e&&R&&U.alternate===null&&t(g,R),v=s(U,v,_),C===null?S=U:C.sibling=U,C=U,R=M}if(L.done)return a(g,R),ye&&Sr(g,_),S;if(R===null){for(;!L.done;_++,L=b.next())L=m(g,L.value,w),L!==null&&(v=s(L,v,_),C===null?S=L:C.sibling=L,C=L);return ye&&Sr(g,_),S}for(R=n(R);!L.done;_++,L=b.next())L=h(R,g,_,L.value,w),L!==null&&(e&&L.alternate!==null&&R.delete(L.key===null?_:L.key),v=s(L,v,_),C===null?S=L:C.sibling=L,C=L);return e&&R.forEach(function(F){return t(g,F)}),ye&&Sr(g,_),S}function $(g,v,b,w){if(typeof b=="object"&&b!==null&&b.type===gs&&b.key===null&&(b=b.props.children),typeof b=="object"&&b!==null){switch(b.$$typeof){case Hl:e:{for(var S=b.key;v!==null;){if(v.key===S){if(S=b.type,S===gs){if(v.tag===7){a(g,v.sibling),w=r(v,b.props.children),w.return=g,g=w;break e}}else if(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Fn&&Mg(S)===v.type){a(g,v.sibling),w=r(v,b.props),Yi(w,b),w.return=g,g=w;break e}a(g,v);break}else t(g,v);v=v.sibling}b.type===gs?(w=Rr(b.props.children,g.mode,w,b.key),w.return=g,g=w):(w=du(b.type,b.key,b.props,null,g.mode,w),Yi(w,b),w.return=g,g=w)}return i(g);case eo:e:{for(S=b.key;v!==null;){if(v.key===S)if(v.tag===4&&v.stateNode.containerInfo===b.containerInfo&&v.stateNode.implementation===b.implementation){a(g,v.sibling),w=r(v,b.children||[]),w.return=g,g=w;break e}else{a(g,v);break}else t(g,v);v=v.sibling}w=um(b,g.mode,w),w.return=g,g=w}return i(g);case Fn:return S=b._init,b=S(b._payload),$(g,v,b,w)}if(to(b))return x(g,v,b,w);if(Qi(b)){if(S=Qi(b),typeof S!="function")throw Error(j(150));return b=S.call(b),y(g,v,b,w)}if(typeof b.then=="function")return $(g,v,Zl(b),w);if(b.$$typeof===pn)return $(g,v,Xl(g,b),w);eu(g,b)}return typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint"?(b=""+b,v!==null&&v.tag===6?(a(g,v.sibling),w=r(v,b),w.return=g,g=w):(a(g,v),w=lm(b,g.mode,w),w.return=g,g=w),i(g)):a(g,v)}return function(g,v,b,w){try{_o=0;var S=$(g,v,b,w);return Ls=null,S}catch(R){if(R===qo||R===ac)throw R;var C=Xt(29,R,null,g.mode);return C.lanes=w,C.return=g,C}finally{}}}var Is=qb(!0),Ib=qb(!1),xa=Ja(null),Ya=null;function qn(e){var t=e.alternate;Fe(rt,rt.current&1),Fe(xa,e),Ya===null&&(t===null||Bs.current!==null||t.memoizedState!==null)&&(Ya=e)}function Hb(e){if(e.tag===22){if(Fe(rt,rt.current),Fe(xa,e),Ya===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(Ya=e)}}else In(e)}function In(){Fe(rt,rt.current),Fe(xa,xa.current)}function yn(e){mt(xa),Ya===e&&(Ya=null),mt(rt)}var rt=Ja(0);function Ou(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||bf(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function mm(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:Me({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var af={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=ea(),r=Jn(n);r.payload=t,a!=null&&(r.callback=a),t=Xn(e,r,n),t!==null&&(ta(t,e,n),uo(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=ea(),r=Jn(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=Xn(e,r,n),t!==null&&(ta(t,e,n),uo(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=ea(),n=Jn(a);n.tag=2,t!=null&&(n.callback=t),t=Xn(e,n,a),t!==null&&(ta(t,e,a),uo(t,e,a))}};function Og(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!wo(a,n)||!wo(r,s):!0}function Lg(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&af.enqueueReplaceState(t,t.state,null)}function Lr(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=Me({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var Lu=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function Kb(e){Lu(e)}function Qb(e){console.error(e)}function Vb(e){Lu(e)}function Pu(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function Pg(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function nf(e,t,a){return a=Jn(a),a.tag=3,a.payload={element:null},a.callback=function(){Pu(e,t)},a}function Gb(e){return e=Jn(e),e.tag=3,e}function Yb(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){Pg(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){Pg(t,a,n),typeof r!="function"&&(Wn===null?Wn=new Set([this]):Wn.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function CE(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&zo(t,a,r,!0),a=xa.current,a!==null){switch(a.tag){case 13:return Ya===null?mf():a.alternate===null&&He===0&&(He=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===Ym?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),Sm(e,n,r)),!1;case 22:return a.flags|=65536,n===Ym?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),Sm(e,n,r)),!1}throw Error(j(435,a.tag))}return Sm(e,n,r),mf(),!1}if(ye)return t=xa.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==Hm&&(e=Error(j(422),{cause:n}),So(ya(e,a)))):(n!==Hm&&(t=Error(j(423),{cause:n}),So(ya(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=ya(n,a),r=nf(e.stateNode,n,r),cm(e,r),He!==4&&(He=2)),!1;var s=Error(j(520),{cause:n});if(s=ya(s,a),vo===null?vo=[s]:vo.push(s),He!==4&&(He=2),t===null)return!0;n=ya(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=nf(a.stateNode,n,e),cm(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(Wn===null||!Wn.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=Gb(r),Yb(r,e,a,n),cm(a,r),!1}a=a.return}while(a!==null);return!1}var Jb=Error(j(461)),dt=!1;function yt(e,t,a,n){t.child=e===null?Ib(t,null,a,n):Is(t,e.child,a,n)}function Ug(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return Mr(t),n=Qf(e,t,a,i,s,r),o=Vf(),e!==null&&!dt?(Gf(e,t,r),Sn(e,t,r)):(ye&&o&&Ff(t),t.flags|=1,yt(e,t,n,r),t.child)}function jg(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!jf(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,Xb(e,t,s,n,r)):(e=du(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!np(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:wo,a(i,n)&&e.ref===t.ref)return Sn(e,t,r)}return t.flags|=1,e=bn(s,n),e.ref=t.ref,e.return=t,t.child=e}function Xb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if(wo(s,n)&&e.ref===t.ref)if(dt=!1,t.pendingProps=n=s,np(e,r))(e.flags&131072)!==0&&(dt=!0);else return t.lanes=e.lanes,Sn(e,t,r)}return rf(e,t,a,n,r)}function Wb(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return Fg(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&mu(t,s!==null?s.cachePool:null),s!==null?Rg(t,s):Wm(),Hb(t);else return t.lanes=t.childLanes=536870912,Fg(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(mu(t,s.cachePool),Rg(t,s),In(t),t.memoizedState=null):(e!==null&&mu(t,null),Wm(),In(t));return yt(e,t,r,a),t.child}function Fg(e,t,a,n){var r=qf();return r=r===null?null:{parent:nt._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&mu(t,null),Wm(),Hb(t),e!==null&&zo(e,t,n,!0),null}function hu(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(j(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function rf(e,t,a,n,r){return Mr(t),a=Qf(e,t,a,n,void 0,r),n=Vf(),e!==null&&!dt?(Gf(e,t,r),Sn(e,t,r)):(ye&&n&&Ff(t),t.flags|=1,yt(e,t,a,r),t.child)}function zg(e,t,a,n,r,s){return Mr(t),t.updateQueue=null,a=db(t,n,a,r),cb(e),n=Vf(),e!==null&&!dt?(Gf(e,t,s),Sn(e,t,s)):(ye&&n&&Ff(t),t.flags|=1,yt(e,t,a,s),t.child)}function Bg(e,t,a,n,r){if(Mr(t),t.stateNode===null){var s=_s,i=a.contextType;typeof i=="object"&&i!==null&&(s=Nt(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=af,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},If(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?Nt(i):_s,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(mm(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&af.enqueueReplaceState(s,s.state,null),mo(t,n,s,r),co(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,l=Lr(a,o);s.props=l;var c=s.context,d=a.contextType;i=_s,typeof d=="object"&&d!==null&&(i=Nt(d));var m=a.getDerivedStateFromProps;d=typeof m=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&Lg(t,s,n,i),zn=!1;var f=t.memoizedState;s.state=f,mo(t,n,s,r),co(),c=t.memoizedState,o||f!==c||zn?(typeof m=="function"&&(mm(t,a,m,n),c=t.memoizedState),(l=zn||Og(t,a,l,n,f,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=l):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,Jm(e,t),i=t.memoizedProps,d=Lr(a,i),s.props=d,m=t.pendingProps,f=s.context,c=a.contextType,l=_s,typeof c=="object"&&c!==null&&(l=Nt(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==m||f!==l)&&Lg(t,s,n,l),zn=!1,f=t.memoizedState,s.state=f,mo(t,n,s,r),co();var h=t.memoizedState;i!==m||f!==h||zn||e!==null&&e.dependencies!==null&&Eu(e.dependencies)?(typeof o=="function"&&(mm(t,a,o,n),h=t.memoizedState),(d=zn||Og(t,a,d,n,f,h,l)||e!==null&&e.dependencies!==null&&Eu(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,h,l),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,h,l)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=h),s.props=n,s.state=h,s.context=l,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,hu(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=Is(t,e.child,null,r),t.child=Is(t,null,a,r)):yt(e,t,a,r),t.memoizedState=s.state,e=t.child):e=Sn(e,t,r),e}function qg(e,t,a,n){return Fo(),t.flags|=256,yt(e,t,a,n),t.child}var fm={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function pm(e){return{baseLanes:e,cachePool:sb()}}function hm(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=ba),e}function Zb(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(rt.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(ye){if(r?qn(t):In(t),ye){var o=Ie,l;if(l=o){e:{for(l=o,o=Ka;l.nodeType!==8;){if(!o){o=null;break e}if(l=Ca(l.nextSibling),l===null){o=null;break e}}o=l}o!==null?(t.memoizedState={dehydrated:o,treeContext:kr!==null?{id:hn,overflow:vn}:null,retryLane:536870912,hydrationErrors:null},l=Xt(18,null,null,0),l.stateNode=o,l.return=t,t.child=l,At=t,Ie=null,l=!0):l=!1}l||Dr(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return bf(o)?t.lanes=32:t.lanes=536870912,null;yn(t)}return o=n.children,n=n.fallback,r?(In(t),r=t.mode,o=Uu({mode:"hidden",children:o},r),n=Rr(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=pm(a),r.childLanes=hm(e,i,a),t.memoizedState=fm,n):(qn(t),sf(t,o))}if(l=e.memoizedState,l!==null&&(o=l.dehydrated,o!==null)){if(s)t.flags&256?(qn(t),t.flags&=-257,t=vm(e,t,a)):t.memoizedState!==null?(In(t),t.child=e.child,t.flags|=128,t=null):(In(t),r=n.fallback,o=t.mode,n=Uu({mode:"visible",children:n.children},o),r=Rr(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,Is(t,e.child,null,a),n=t.child,n.memoizedState=pm(a),n.childLanes=hm(e,i,a),t.memoizedState=fm,t=r);else if(qn(t),bf(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(j(419)),n.stack="",n.digest=i,So({value:n,source:null,stack:null}),t=vm(e,t,a)}else if(dt||zo(e,t,a,!1),i=(a&e.childLanes)!==0,dt||i){if(i=Ee,i!==null&&(n=a&-a,n=(n&42)!==0?1:kf(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==l.retryLane))throw l.retryLane=n,Js(e,n),ta(i,e,n),Jb;o.data==="$?"||mf(),t=vm(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=l.treeContext,Ie=Ca(o.nextSibling),At=t,ye=!0,Cr=null,Ka=!1,e!==null&&(ha[va++]=hn,ha[va++]=vn,ha[va++]=kr,hn=e.id,vn=e.overflow,kr=t),t=sf(t,n.children),t.flags|=4096);return t}return r?(In(t),r=n.fallback,o=t.mode,l=e.child,c=l.sibling,n=bn(l,{mode:"hidden",children:n.children}),n.subtreeFlags=l.subtreeFlags&65011712,c!==null?r=bn(c,r):(r=Rr(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=pm(a):(l=o.cachePool,l!==null?(c=nt._currentValue,l=l.parent!==c?{parent:c,pool:c}:l):l=sb(),o={baseLanes:o.baseLanes|a,cachePool:l}),r.memoizedState=o,r.childLanes=hm(e,i,a),t.memoizedState=fm,n):(qn(t),a=e.child,e=a.sibling,a=bn(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function sf(e,t){return t=Uu({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function Uu(e,t){return e=Xt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function vm(e,t,a){return Is(t,e.child,null,a),e=sf(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function Ig(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),Qm(e.return,t,a)}function gm(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function ex(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(yt(e,t,n.children,a),n=rt.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&Ig(e,a,t);else if(e.tag===19)Ig(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Fe(rt,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&Ou(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),gm(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&Ou(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}gm(t,!0,a,null,s);break;case"together":gm(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function Sn(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),sr|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(zo(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(j(153));if(t.child!==null){for(e=t.child,a=bn(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=bn(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function np(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&Eu(e)))}function EE(e,t,a){switch(t.tag){case 3:$u(t,t.stateNode.containerInfo),Bn(t,nt,e.memoizedState.cache),Fo();break;case 27:case 5:Om(t);break;case 4:$u(t,t.stateNode.containerInfo);break;case 10:Bn(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(qn(t),t.flags|=128,null):(a&t.child.childLanes)!==0?Zb(e,t,a):(qn(t),e=Sn(e,t,a),e!==null?e.sibling:null);qn(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(zo(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return ex(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Fe(rt,rt.current),n)break;return null;case 22:case 23:return t.lanes=0,Wb(e,t,a);case 24:Bn(t,nt,e.memoizedState.cache)}return Sn(e,t,a)}function tx(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)dt=!0;else{if(!np(e,a)&&(t.flags&128)===0)return dt=!1,EE(e,t,a);dt=(e.flags&131072)!==0}else dt=!1,ye&&(t.flags&1048576)!==0&&nb(t,Cu,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")jf(n)?(e=Lr(n,e),t.tag=1,t=Bg(null,t,n,e,a)):(t.tag=0,t=rf(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===Nf){t.tag=11,t=Ug(null,t,n,e,a);break e}else if(r===_f){t.tag=14,t=jg(null,t,n,e,a);break e}}throw t=Dm(n)||n,Error(j(306,t,""))}}return t;case 0:return rf(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Lr(n,t.pendingProps),Bg(e,t,n,r,a);case 3:e:{if($u(t,t.stateNode.containerInfo),e===null)throw Error(j(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,Jm(e,t),mo(t,n,null,a);var i=t.memoizedState;if(n=i.cache,Bn(t,nt,n),n!==s.cache&&Vm(t,[nt],a,!0),co(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=qg(e,t,n,a);break e}else if(n!==r){r=ya(Error(j(424)),t),So(r),t=qg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(Ie=Ca(e.firstChild),At=t,ye=!0,Cr=null,Ka=!0,a=Ib(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if(Fo(),n===r){t=Sn(e,t,a);break e}yt(e,t,n,a)}t=t.child}return t;case 26:return hu(e,t),e===null?(a=ly(t.type,null,t.pendingProps,null))?t.memoizedState=a:ye||(a=t.type,e=t.pendingProps,n=Hu(Yn.current).createElement(a),n[St]=t,n[qt]=e,xt(n,a,e),ct(n),t.stateNode=n):t.memoizedState=ly(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return Om(t),e===null&&ye&&(n=t.stateNode=Bx(t.type,t.pendingProps,Yn.current),At=t,Ka=!0,r=Ie,or(t.type)?(xf=r,Ie=Ca(n.firstChild)):Ie=r),yt(e,t,t.pendingProps.children,a),hu(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&ye&&((r=n=Ie)&&(n=t3(n,t.type,t.pendingProps,Ka),n!==null?(t.stateNode=n,At=t,Ie=Ca(n.firstChild),Ka=!1,r=!0):r=!1),r||Dr(t)),Om(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,gf(r,s)?n=null:i!==null&&gf(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=Qf(e,t,$E,null,null,a),Eo._currentValue=r),hu(e,t),yt(e,t,n,a),t.child;case 6:return e===null&&ye&&((e=a=Ie)&&(a=a3(a,t.pendingProps,Ka),a!==null?(t.stateNode=a,At=t,Ie=null,e=!0):e=!1),e||Dr(t)),null;case 13:return Zb(e,t,a);case 4:return $u(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=Is(t,null,n,a):yt(e,t,n,a),t.child;case 11:return Ug(e,t,t.type,t.pendingProps,a);case 7:return yt(e,t,t.pendingProps,a),t.child;case 8:return yt(e,t,t.pendingProps.children,a),t.child;case 12:return yt(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,Bn(t,t.type,n.value),yt(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,Mr(t),r=Nt(r),n=n(r),t.flags|=1,yt(e,t,n,a),t.child;case 14:return jg(e,t,t.type,t.pendingProps,a);case 15:return Xb(e,t,t.type,t.pendingProps,a);case 19:return ex(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=Uu(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=bn(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return Wb(e,t,a);case 24:return Mr(t),n=Nt(nt),e===null?(r=qf(),r===null&&(r=Ee,s=Bf(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},If(t),Bn(t,nt,r)):((e.lanes&a)!==0&&(Jm(e,t),mo(t,null,null,a),co()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),Bn(t,nt,n)):(n=s.cache,Bn(t,nt,n),n!==r.cache&&Vm(t,[nt],a,!0))),yt(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(j(156,t.tag))}function dn(e){e.flags|=4}function Hg(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!Hx(t)){if(t=xa.current,t!==null&&((he&4194048)===he?Ya!==null:(he&62914560)!==he&&(he&536870912)===0||t!==Ya))throw lo=Ym,ib;e.flags|=8192}}function tu(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?ky():536870912,e.lanes|=t,Hs|=t)}function Ji(e,t){if(!ye)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function Be(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function TE(e,t,a){var n=t.pendingProps;switch(zf(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return Be(t),null;case 1:return Be(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),xn(nt),Us(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Gi(t)?dn(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,xg())),Be(t),null;case 26:return a=t.memoizedState,e===null?(dn(t),a!==null?(Be(t),Hg(t,a)):(Be(t),t.flags&=-16777217)):a?a!==e.memoizedState?(dn(t),Be(t),Hg(t,a)):(Be(t),t.flags&=-16777217):(e.memoizedProps!==n&&dn(t),Be(t),t.flags&=-16777217),null;case 27:wu(t),a=Yn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&dn(t);else{if(!n){if(t.stateNode===null)throw Error(j(166));return Be(t),null}e=Va.current,Gi(t)?yg(t,e):(e=Bx(r,n,a),t.stateNode=e,dn(t))}return Be(t),null;case 5:if(wu(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&dn(t);else{if(!n){if(t.stateNode===null)throw Error(j(166));return Be(t),null}if(e=Va.current,Gi(t))yg(t,e);else{switch(r=Hu(Yn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[St]=t,e[qt]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(xt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&dn(t)}}return Be(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&dn(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(j(166));if(e=Yn.current,Gi(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=At,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[St]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||jx(e.nodeValue,a)),e||Dr(t)}else e=Hu(e).createTextNode(n),e[St]=t,t.stateNode=e}return Be(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Gi(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(j(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(j(317));r[St]=t}else Fo(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;Be(t),r=!1}else r=xg(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(yn(t),t):(yn(t),null)}if(yn(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),tu(t,t.updateQueue),Be(t),null;case 4:return Us(),e===null&&dp(t.stateNode.containerInfo),Be(t),null;case 10:return xn(t.type),Be(t),null;case 19:if(mt(rt),r=t.memoizedState,r===null)return Be(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)Ji(r,!1);else{if(He!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=Ou(e),s!==null){for(t.flags|=128,Ji(r,!1),e=s.updateQueue,t.updateQueue=e,tu(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)ab(a,e),a=a.sibling;return Fe(rt,rt.current&1|2),t.child}e=e.sibling}r.tail!==null&&Ga()>Fu&&(t.flags|=128,n=!0,Ji(r,!1),t.lanes=4194304)}else{if(!n)if(e=Ou(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,tu(t,e),Ji(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!ye)return Be(t),null}else 2*Ga()-r.renderingStartTime>Fu&&a!==536870912&&(t.flags|=128,n=!0,Ji(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=Ga(),t.sibling=null,e=rt.current,Fe(rt,n?e&1|2:e&1),t):(Be(t),null);case 22:case 23:return yn(t),Hf(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(Be(t),t.subtreeFlags&6&&(t.flags|=8192)):Be(t),a=t.updateQueue,a!==null&&tu(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&mt(Er),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),xn(nt),Be(t),null;case 25:return null;case 30:return null}throw Error(j(156,t.tag))}function AE(e,t){switch(zf(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return xn(nt),Us(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return wu(t),null;case 13:if(yn(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(j(340));Fo()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return mt(rt),null;case 4:return Us(),null;case 10:return xn(t.type),null;case 22:case 23:return yn(t),Hf(),e!==null&&mt(Er),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return xn(nt),null;case 25:return null;default:return null}}function ax(e,t){switch(zf(t),t.tag){case 3:xn(nt),Us();break;case 26:case 27:case 5:wu(t);break;case 4:Us();break;case 13:yn(t);break;case 19:mt(rt);break;case 10:xn(t.type);break;case 22:case 23:yn(t),Hf(),e!==null&&mt(Er);break;case 24:xn(nt)}}function Ko(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){Re(t,t.return,o)}}function rr(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var l=a,c=o;try{c()}catch(d){Re(r,l,d)}}}n=n.next}while(n!==s)}}catch(d){Re(t,t.return,d)}}function nx(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{ub(t,a)}catch(n){Re(e,e.return,n)}}}function rx(e,t,a){a.props=Lr(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){Re(e,t,n)}}function po(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){Re(e,t,r)}}function Qa(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){Re(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){Re(e,t,r)}else a.current=null}function sx(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){Re(e,e.return,r)}}function ym(e,t,a){try{var n=e.stateNode;JE(n,e.type,a,t),n[qt]=t}catch(r){Re(e,e.return,r)}}function ix(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&or(e.type)||e.tag===4}function bm(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||ix(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&or(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function of(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=uc));else if(n!==4&&(n===27&&or(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(of(e,t,a),e=e.sibling;e!==null;)of(e,t,a),e=e.sibling}function ju(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&or(e.type)&&(a=e.stateNode),e=e.child,e!==null))for(ju(e,t,a),e=e.sibling;e!==null;)ju(e,t,a),e=e.sibling}function ox(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);xt(t,n,a),t[St]=e,t[qt]=a}catch(s){Re(e,e.return,s)}}var fn=!1,Ge=!1,xm=!1,Kg=typeof WeakSet=="function"?WeakSet:Set,ut=null;function DE(e,t){if(e=e.containerInfo,hf=Gu,e=Gy(e),Lf(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,l=-1,c=0,d=0,m=e,f=null;t:for(;;){for(var h;m!==a||r!==0&&m.nodeType!==3||(o=i+r),m!==s||n!==0&&m.nodeType!==3||(l=i+n),m.nodeType===3&&(i+=m.nodeValue.length),(h=m.firstChild)!==null;)f=m,m=h;for(;;){if(m===e)break t;if(f===a&&++c===r&&(o=i),f===s&&++d===n&&(l=i),(h=m.nextSibling)!==null)break;m=f,f=m.parentNode}m=h}a=o===-1||l===-1?null:{start:o,end:l}}else a=null}a=a||{start:0,end:0}}else a=null;for(vf={focusedElem:e,selectionRange:a},Gu=!1,ut=t;ut!==null;)if(t=ut,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,ut=e;else for(;ut!==null;){switch(t=ut,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var x=Lr(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(x,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(y){Re(a,a.return,y)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)yf(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":yf(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(j(163))}if(e=t.sibling,e!==null){e.return=t.return,ut=e;break}ut=t.return}}function lx(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:Un(e,a),n&4&&Ko(5,a);break;case 1:if(Un(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){Re(a,a.return,i)}else{var r=Lr(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){Re(a,a.return,i)}}n&64&&nx(a),n&512&&po(a,a.return);break;case 3:if(Un(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{ub(e,t)}catch(i){Re(a,a.return,i)}}break;case 27:t===null&&n&4&&ox(a);case 26:case 5:Un(e,a),t===null&&n&4&&sx(a),n&512&&po(a,a.return);break;case 12:Un(e,a);break;case 13:Un(e,a),n&4&&dx(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=BE.bind(null,a),n3(e,a))));break;case 22:if(n=a.memoizedState!==null||fn,!n){t=t!==null&&t.memoizedState!==null||Ge,r=fn;var s=Ge;fn=n,(Ge=t)&&!s?jn(e,a,(a.subtreeFlags&8772)!==0):Un(e,a),fn=r,Ge=s}break;case 30:break;default:Un(e,a)}}function ux(e){var t=e.alternate;t!==null&&(e.alternate=null,ux(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&Ef(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var je=null,zt=!1;function mn(e,t,a){for(a=a.child;a!==null;)cx(e,t,a),a=a.sibling}function cx(e,t,a){if(Wt&&typeof Wt.onCommitFiberUnmount=="function")try{Wt.onCommitFiberUnmount(Oo,a)}catch{}switch(a.tag){case 26:Ge||Qa(a,t),mn(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:Ge||Qa(a,t);var n=je,r=zt;or(a.type)&&(je=a.stateNode,zt=!1),mn(e,t,a),yo(a.stateNode),je=n,zt=r;break;case 5:Ge||Qa(a,t);case 6:if(n=je,r=zt,je=null,mn(e,t,a),je=n,zt=r,je!==null)if(zt)try{(je.nodeType===9?je.body:je.nodeName==="HTML"?je.ownerDocument.body:je).removeChild(a.stateNode)}catch(s){Re(a,t,s)}else try{je.removeChild(a.stateNode)}catch(s){Re(a,t,s)}break;case 18:je!==null&&(zt?(e=je,sy(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),Do(e)):sy(je,a.stateNode));break;case 4:n=je,r=zt,je=a.stateNode.containerInfo,zt=!0,mn(e,t,a),je=n,zt=r;break;case 0:case 11:case 14:case 15:Ge||rr(2,a,t),Ge||rr(4,a,t),mn(e,t,a);break;case 1:Ge||(Qa(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&rx(a,t,n)),mn(e,t,a);break;case 21:mn(e,t,a);break;case 22:Ge=(n=Ge)||a.memoizedState!==null,mn(e,t,a),Ge=n;break;default:mn(e,t,a)}}function dx(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{Do(e)}catch(a){Re(t,t.return,a)}}function ME(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new Kg),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new Kg),t;default:throw Error(j(435,e.tag))}}function $m(e,t){var a=ME(e);t.forEach(function(n){var r=qE.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function Gt(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(or(o.type)){je=o.stateNode,zt=!1;break e}break;case 5:je=o.stateNode,zt=!1;break e;case 3:case 4:je=o.stateNode.containerInfo,zt=!0;break e}o=o.return}if(je===null)throw Error(j(160));cx(s,i,r),je=null,zt=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)mx(t,e),t=t.sibling}var ka=null;function mx(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:Gt(t,e),Yt(e),n&4&&(rr(3,e,e.return),Ko(3,e),rr(5,e,e.return));break;case 1:Gt(t,e),Yt(e),n&512&&(Ge||a===null||Qa(a,a.return)),n&64&&fn&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=ka;if(Gt(t,e),Yt(e),n&512&&(Ge||a===null||Qa(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[Uo]||s[St]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),xt(s,n,a),s[St]=e,ct(s),n=s;break e;case"link":var i=cy("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),xt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=cy("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),xt(s,n,a),r.head.appendChild(s);break;default:throw Error(j(468,n))}s[St]=e,ct(s),n=s}e.stateNode=n}else dy(r,e.type,e.stateNode);else e.stateNode=uy(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?dy(r,e.type,e.stateNode):uy(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&ym(e,e.memoizedProps,a.memoizedProps)}break;case 27:Gt(t,e),Yt(e),n&512&&(Ge||a===null||Qa(a,a.return)),a!==null&&n&4&&ym(e,e.memoizedProps,a.memoizedProps);break;case 5:if(Gt(t,e),Yt(e),n&512&&(Ge||a===null||Qa(a,a.return)),e.flags&32){r=e.stateNode;try{Fs(r,"")}catch(h){Re(e,e.return,h)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,ym(e,r,a!==null?a.memoizedProps:r)),n&1024&&(xm=!0);break;case 6:if(Gt(t,e),Yt(e),n&4){if(e.stateNode===null)throw Error(j(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(h){Re(e,e.return,h)}}break;case 3:if(yu=null,r=ka,ka=Ku(t.containerInfo),Gt(t,e),ka=r,Yt(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{Do(t.containerInfo)}catch(h){Re(e,e.return,h)}xm&&(xm=!1,fx(e));break;case 4:n=ka,ka=Ku(e.stateNode.containerInfo),Gt(t,e),Yt(e),ka=n;break;case 12:Gt(t,e),Yt(e);break;case 13:Gt(t,e),Yt(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(lp=Ga()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,$m(e,n)));break;case 22:r=e.memoizedState!==null;var l=a!==null&&a.memoizedState!==null,c=fn,d=Ge;if(fn=c||r,Ge=d||l,Gt(t,e),Ge=d,fn=c,Yt(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||l||fn||Ge||Nr(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){l=a=t;try{if(s=l.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=l.stateNode;var m=l.memoizedProps.style,f=m!=null&&m.hasOwnProperty("display")?m.display:null;o.style.display=f==null||typeof f=="boolean"?"":(""+f).trim()}}catch(h){Re(l,l.return,h)}}}else if(t.tag===6){if(a===null){l=t;try{l.stateNode.nodeValue=r?"":l.memoizedProps}catch(h){Re(l,l.return,h)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,$m(e,a))));break;case 19:Gt(t,e),Yt(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,$m(e,n)));break;case 30:break;case 21:break;default:Gt(t,e),Yt(e)}}function Yt(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(ix(n)){a=n;break}n=n.return}if(a==null)throw Error(j(160));switch(a.tag){case 27:var r=a.stateNode,s=bm(e);ju(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(Fs(i,""),a.flags&=-33);var o=bm(e);ju(e,o,i);break;case 3:case 4:var l=a.stateNode.containerInfo,c=bm(e);of(e,c,l);break;default:throw Error(j(161))}}catch(d){Re(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function fx(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;fx(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function Un(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)lx(e,t.alternate,t),t=t.sibling}function Nr(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:rr(4,t,t.return),Nr(t);break;case 1:Qa(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&rx(t,t.return,a),Nr(t);break;case 27:yo(t.stateNode);case 26:case 5:Qa(t,t.return),Nr(t);break;case 22:t.memoizedState===null&&Nr(t);break;case 30:Nr(t);break;default:Nr(t)}e=e.sibling}}function jn(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:jn(r,s,a),Ko(4,s);break;case 1:if(jn(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){Re(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var l=r.shared.hiddenCallbacks;if(l!==null)for(r.shared.hiddenCallbacks=null,r=0;r<l.length;r++)lb(l[r],o)}catch(c){Re(n,n.return,c)}}a&&i&64&&nx(s),po(s,s.return);break;case 27:ox(s);case 26:case 5:jn(r,s,a),a&&n===null&&i&4&&sx(s),po(s,s.return);break;case 12:jn(r,s,a);break;case 13:jn(r,s,a),a&&i&4&&dx(r,s);break;case 22:s.memoizedState===null&&jn(r,s,a),po(s,s.return);break;case 30:break;default:jn(r,s,a)}t=t.sibling}}function rp(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&Bo(a))}function sp(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&Bo(e))}function Ha(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)px(e,t,a,n),t=t.sibling}function px(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:Ha(e,t,a,n),r&2048&&Ko(9,t);break;case 1:Ha(e,t,a,n);break;case 3:Ha(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&Bo(e)));break;case 12:if(r&2048){Ha(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(l){Re(t,t.return,l)}}else Ha(e,t,a,n);break;case 13:Ha(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?Ha(e,t,a,n):ho(e,t):s._visibility&2?Ha(e,t,a,n):(s._visibility|=2,hs(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&rp(i,t);break;case 24:Ha(e,t,a,n),r&2048&&sp(t.alternate,t);break;default:Ha(e,t,a,n)}}function hs(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,l=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:hs(s,i,o,l,r),Ko(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?hs(s,i,o,l,r):ho(s,i):(d._visibility|=2,hs(s,i,o,l,r)),r&&c&2048&&rp(i.alternate,i);break;case 24:hs(s,i,o,l,r),r&&c&2048&&sp(i.alternate,i);break;default:hs(s,i,o,l,r)}t=t.sibling}}function ho(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:ho(a,n),r&2048&&rp(n.alternate,n);break;case 24:ho(a,n),r&2048&&sp(n.alternate,n);break;default:ho(a,n)}t=t.sibling}}var no=8192;function ms(e){if(e.subtreeFlags&no)for(e=e.child;e!==null;)hx(e),e=e.sibling}function hx(e){switch(e.tag){case 26:ms(e),e.flags&no&&e.memoizedState!==null&&v3(ka,e.memoizedState,e.memoizedProps);break;case 5:ms(e);break;case 3:case 4:var t=ka;ka=Ku(e.stateNode.containerInfo),ms(e),ka=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=no,no=16777216,ms(e),no=t):ms(e));break;default:ms(e)}}function vx(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function Xi(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ut=n,yx(n,e)}vx(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)gx(e),e=e.sibling}function gx(e){switch(e.tag){case 0:case 11:case 15:Xi(e),e.flags&2048&&rr(9,e,e.return);break;case 3:Xi(e);break;case 12:Xi(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,vu(e)):Xi(e);break;default:Xi(e)}}function vu(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ut=n,yx(n,e)}vx(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:rr(8,t,t.return),vu(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,vu(t));break;default:vu(t)}e=e.sibling}}function yx(e,t){for(;ut!==null;){var a=ut;switch(a.tag){case 0:case 11:case 15:rr(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:Bo(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,ut=n;else e:for(a=e;ut!==null;){n=ut;var r=n.sibling,s=n.return;if(ux(n),n===a){ut=null;break e}if(r!==null){r.return=s,ut=r;break e}ut=s}}}var OE={getCacheForType:function(e){var t=Nt(nt),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},LE=typeof WeakMap=="function"?WeakMap:Map,Se=0,Ee=null,me=null,he=0,we=0,Jt=null,Vn=!1,Xs=!1,ip=!1,Nn=0,He=0,sr=0,Tr=0,op=0,ba=0,Hs=0,vo=null,Bt=null,lf=!1,lp=0,Fu=1/0,zu=null,Wn=null,bt=0,Zn=null,Ks=null,Ps=0,uf=0,cf=null,bx=null,go=0,df=null;function ea(){if((Se&2)!==0&&he!==0)return he&-he;if(se.T!==null){var e=zs;return e!==0?e:cp()}return Ty()}function xx(){ba===0&&(ba=(he&536870912)===0||ye?Ry():536870912);var e=xa.current;return e!==null&&(e.flags|=32),ba}function ta(e,t,a){(e===Ee&&(we===2||we===9)||e.cancelPendingCommit!==null)&&(Qs(e,0),Gn(e,he,ba,!1)),Po(e,a),((Se&2)===0||e!==Ee)&&(e===Ee&&((Se&2)===0&&(Tr|=a),He===4&&Gn(e,he,ba,!1)),Xa(e))}function $x(e,t,a){if((Se&6)!==0)throw Error(j(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||Lo(e,t),r=n?jE(e,t):wm(e,t,!0),s=n;do{if(r===0){Xs&&!n&&Gn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!PE(a)){r=wm(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=vo;var l=o.current.memoizedState.isDehydrated;if(l&&(Qs(o,i).flags|=256),i=wm(o,i,!1),i!==2){if(ip&&!l){o.errorRecoveryDisabledLanes|=s,Tr|=s,r=4;break e}s=Bt,Bt=r,s!==null&&(Bt===null?Bt=s:Bt.push.apply(Bt,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){Qs(e,0),Gn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(j(345));case 4:if((t&4194048)!==t)break;case 6:Gn(n,t,ba,!Vn);break e;case 2:Bt=null;break;case 3:case 5:break;default:throw Error(j(329))}if((t&62914560)===t&&(r=lp+300-Ga(),10<r)){if(Gn(n,t,ba,!Vn),Ju(n,0,!0)!==0)break e;n.timeoutHandle=zx(Qg.bind(null,n,a,Bt,zu,lf,t,ba,Tr,Hs,Vn,s,2,-0,0),r);break e}Qg(n,a,Bt,zu,lf,t,ba,Tr,Hs,Vn,s,0,-0,0)}}break}while(!0);Xa(e)}function Qg(e,t,a,n,r,s,i,o,l,c,d,m,f,h){if(e.timeoutHandle=-1,m=t.subtreeFlags,(m&8192||(m&16785408)===16785408)&&(Co={stylesheets:null,count:0,unsuspend:h3},hx(t),m=g3(),m!==null)){e.cancelPendingCommit=m(Gg.bind(null,e,t,s,a,n,r,i,o,l,d,1,f,h)),Gn(e,s,i,!c);return}Gg(e,t,s,a,n,r,i,o,l)}function PE(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!aa(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function Gn(e,t,a,n){t&=~op,t&=~Tr,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Zt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&Cy(e,a,t)}function ic(){return(Se&6)===0?(Qo(0,!1),!1):!0}function up(){if(me!==null){if(we===0)var e=me.return;else e=me,gn=Fr=null,Yf(e),Ls=null,_o=0,e=me;for(;e!==null;)ax(e.alternate,e),e=e.return;me=null}}function Qs(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,WE(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),up(),Ee=e,me=a=bn(e.current,null),he=t,we=0,Jt=null,Vn=!1,Xs=Lo(e,t),ip=!1,Hs=ba=op=Tr=sr=He=0,Bt=vo=null,lf=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Zt(n),s=1<<r;t|=e[r],n&=~s}return Nn=t,ec(),a}function wx(e,t){ue=null,se.H=Mu,t===qo||t===ac?(t=Ng(),we=3):t===ib?(t=Ng(),we=4):we=t===Jb?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Jt=t,me===null&&(He=1,Pu(e,ya(t,e.current)))}function Sx(){var e=se.H;return se.H=Mu,e===null?Mu:e}function Nx(){var e=se.A;return se.A=OE,e}function mf(){He=4,Vn||(he&4194048)!==he&&xa.current!==null||(Xs=!0),(sr&134217727)===0&&(Tr&134217727)===0||Ee===null||Gn(Ee,he,ba,!1)}function wm(e,t,a){var n=Se;Se|=2;var r=Sx(),s=Nx();(Ee!==e||he!==t)&&(zu=null,Qs(e,t)),t=!1;var i=He;e:do try{if(we!==0&&me!==null){var o=me,l=Jt;switch(we){case 8:up(),i=6;break e;case 3:case 2:case 9:case 6:xa.current===null&&(t=!0);var c=we;if(we=0,Jt=null,Cs(e,o,l,c),a&&Xs){i=0;break e}break;default:c=we,we=0,Jt=null,Cs(e,o,l,c)}}UE(),i=He;break}catch(d){wx(e,d)}while(!0);return t&&e.shellSuspendCounter++,gn=Fr=null,Se=n,se.H=r,se.A=s,me===null&&(Ee=null,he=0,ec()),i}function UE(){for(;me!==null;)_x(me)}function jE(e,t){var a=Se;Se|=2;var n=Sx(),r=Nx();Ee!==e||he!==t?(zu=null,Fu=Ga()+500,Qs(e,t)):Xs=Lo(e,t);e:do try{if(we!==0&&me!==null){t=me;var s=Jt;t:switch(we){case 1:we=0,Jt=null,Cs(e,t,s,1);break;case 2:case 9:if(Sg(s)){we=0,Jt=null,Vg(t);break}t=function(){we!==2&&we!==9||Ee!==e||(we=7),Xa(e)},s.then(t,t);break e;case 3:we=7;break e;case 4:we=5;break e;case 7:Sg(s)?(we=0,Jt=null,Vg(t)):(we=0,Jt=null,Cs(e,t,s,7));break;case 5:var i=null;switch(me.tag){case 26:i=me.memoizedState;case 5:case 27:var o=me;if(!i||Hx(i)){we=0,Jt=null;var l=o.sibling;if(l!==null)me=l;else{var c=o.return;c!==null?(me=c,oc(c)):me=null}break t}}we=0,Jt=null,Cs(e,t,s,5);break;case 6:we=0,Jt=null,Cs(e,t,s,6);break;case 8:up(),He=6;break e;default:throw Error(j(462))}}FE();break}catch(d){wx(e,d)}while(!0);return gn=Fr=null,se.H=n,se.A=r,Se=a,me!==null?0:(Ee=null,he=0,ec(),He)}function FE(){for(;me!==null&&!oC();)_x(me)}function _x(e){var t=tx(e.alternate,e,Nn);e.memoizedProps=e.pendingProps,t===null?oc(e):me=t}function Vg(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=zg(a,t,t.pendingProps,t.type,void 0,he);break;case 11:t=zg(a,t,t.pendingProps,t.type.render,t.ref,he);break;case 5:Yf(t);default:ax(a,t),t=me=ab(t,Nn),t=tx(a,t,Nn)}e.memoizedProps=e.pendingProps,t===null?oc(e):me=t}function Cs(e,t,a,n){gn=Fr=null,Yf(t),Ls=null,_o=0;var r=t.return;try{if(CE(e,r,t,a,he)){He=1,Pu(e,ya(a,e.current)),me=null;return}}catch(s){if(r!==null)throw me=r,s;He=1,Pu(e,ya(a,e.current)),me=null;return}t.flags&32768?(ye||n===1?e=!0:Xs||(he&536870912)!==0?e=!1:(Vn=e=!0,(n===2||n===9||n===3||n===6)&&(n=xa.current,n!==null&&n.tag===13&&(n.flags|=16384))),Rx(t,e)):oc(t)}function oc(e){var t=e;do{if((t.flags&32768)!==0){Rx(t,Vn);return}e=t.return;var a=TE(t.alternate,t,Nn);if(a!==null){me=a;return}if(t=t.sibling,t!==null){me=t;return}me=t=e}while(t!==null);He===0&&(He=5)}function Rx(e,t){do{var a=AE(e.alternate,e);if(a!==null){a.flags&=32767,me=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){me=e;return}me=e=a}while(e!==null);He=6,me=null}function Gg(e,t,a,n,r,s,i,o,l){e.cancelPendingCommit=null;do lc();while(bt!==0);if((Se&6)!==0)throw Error(j(327));if(t!==null){if(t===e.current)throw Error(j(177));if(s=t.lanes|t.childLanes,s|=Pf,gC(e,a,s,i,o,l),e===Ee&&(me=Ee=null,he=0),Ks=t,Zn=e,Ps=a,uf=s,cf=r,bx=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,IE(Su,function(){return Ax(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=se.T,se.T=null,r=be.p,be.p=2,i=Se,Se|=4;try{DE(e,t,a)}finally{Se=i,be.p=r,se.T=n}}bt=1,kx(),Cx(),Ex()}}function kx(){if(bt===1){bt=0;var e=Zn,t=Ks,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=se.T,se.T=null;var n=be.p;be.p=2;var r=Se;Se|=4;try{mx(t,e);var s=vf,i=Gy(e.containerInfo),o=s.focusedElem,l=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&Vy(o.ownerDocument.documentElement,o)){if(l!==null&&Lf(o)){var c=l.start,d=l.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var m=o.ownerDocument||document,f=m&&m.defaultView||window;if(f.getSelection){var h=f.getSelection(),x=o.textContent.length,y=Math.min(l.start,x),$=l.end===void 0?y:Math.min(l.end,x);!h.extend&&y>$&&(i=$,$=y,y=i);var g=hg(o,y),v=hg(o,$);if(g&&v&&(h.rangeCount!==1||h.anchorNode!==g.node||h.anchorOffset!==g.offset||h.focusNode!==v.node||h.focusOffset!==v.offset)){var b=m.createRange();b.setStart(g.node,g.offset),h.removeAllRanges(),y>$?(h.addRange(b),h.extend(v.node,v.offset)):(b.setEnd(v.node,v.offset),h.addRange(b))}}}}for(m=[],h=o;h=h.parentNode;)h.nodeType===1&&m.push({element:h,left:h.scrollLeft,top:h.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<m.length;o++){var w=m[o];w.element.scrollLeft=w.left,w.element.scrollTop=w.top}}Gu=!!hf,vf=hf=null}finally{Se=r,be.p=n,se.T=a}}e.current=t,bt=2}}function Cx(){if(bt===2){bt=0;var e=Zn,t=Ks,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=se.T,se.T=null;var n=be.p;be.p=2;var r=Se;Se|=4;try{lx(e,t.alternate,t)}finally{Se=r,be.p=n,se.T=a}}bt=3}}function Ex(){if(bt===4||bt===3){bt=0,lC();var e=Zn,t=Ks,a=Ps,n=bx;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?bt=5:(bt=0,Ks=Zn=null,Tx(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(Wn=null),Cf(a),t=t.stateNode,Wt&&typeof Wt.onCommitFiberRoot=="function")try{Wt.onCommitFiberRoot(Oo,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=se.T,r=be.p,be.p=2,se.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{se.T=t,be.p=r}}(Ps&3)!==0&&lc(),Xa(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===df?go++:(go=0,df=e):go=0,Qo(0,!1)}}function Tx(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,Bo(t)))}function lc(e){return kx(),Cx(),Ex(),Ax(e)}function Ax(){if(bt!==5)return!1;var e=Zn,t=uf;uf=0;var a=Cf(Ps),n=se.T,r=be.p;try{be.p=32>a?32:a,se.T=null,a=cf,cf=null;var s=Zn,i=Ps;if(bt=0,Ks=Zn=null,Ps=0,(Se&6)!==0)throw Error(j(331));var o=Se;if(Se|=4,gx(s.current),px(s,s.current,i,a),Se=o,Qo(0,!1),Wt&&typeof Wt.onPostCommitFiberRoot=="function")try{Wt.onPostCommitFiberRoot(Oo,s)}catch{}return!0}finally{be.p=r,se.T=n,Tx(e,t)}}function Yg(e,t,a){t=ya(a,t),t=nf(e.stateNode,t,2),e=Xn(e,t,2),e!==null&&(Po(e,2),Xa(e))}function Re(e,t,a){if(e.tag===3)Yg(e,e,a);else for(;t!==null;){if(t.tag===3){Yg(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(Wn===null||!Wn.has(n))){e=ya(a,e),a=Gb(2),n=Xn(t,a,2),n!==null&&(Yb(a,n,t,e),Po(n,2),Xa(n));break}}t=t.return}}function Sm(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new LE;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(ip=!0,r.add(a),e=zE.bind(null,e,t,a),t.then(e,e))}function zE(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,Ee===e&&(he&a)===a&&(He===4||He===3&&(he&62914560)===he&&300>Ga()-lp?(Se&2)===0&&Qs(e,0):op|=a,Hs===he&&(Hs=0)),Xa(e)}function Dx(e,t){t===0&&(t=ky()),e=Js(e,t),e!==null&&(Po(e,t),Xa(e))}function BE(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),Dx(e,a)}function qE(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(j(314))}n!==null&&n.delete(t),Dx(e,a)}function IE(e,t){return Rf(e,t)}var Bu=null,vs=null,ff=!1,qu=!1,Nm=!1,Ar=0;function Xa(e){e!==vs&&e.next===null&&(vs===null?Bu=vs=e:vs=vs.next=e),qu=!0,ff||(ff=!0,KE())}function Qo(e,t){if(!Nm&&qu){Nm=!0;do for(var a=!1,n=Bu;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Zt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,Jg(n,s))}else s=he,s=Ju(n,n===Ee?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||Lo(n,s)||(a=!0,Jg(n,s));n=n.next}while(a);Nm=!1}}function HE(){Mx()}function Mx(){qu=ff=!1;var e=0;Ar!==0&&(XE()&&(e=Ar),Ar=0);for(var t=Ga(),a=null,n=Bu;n!==null;){var r=n.next,s=Ox(n,t);s===0?(n.next=null,a===null?Bu=r:a.next=r,r===null&&(vs=a)):(a=n,(e!==0||(s&3)!==0)&&(qu=!0)),n=r}Qo(e,!1)}function Ox(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Zt(s),o=1<<i,l=r[i];l===-1?((o&a)===0||(o&n)!==0)&&(r[i]=vC(o,t)):l<=t&&(e.expiredLanes|=o),s&=~o}if(t=Ee,a=he,a=Ju(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&(we===2||we===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&Xd(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||Lo(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&Xd(n),Cf(a)){case 2:case 8:a=Ny;break;case 32:a=Su;break;case 268435456:a=_y;break;default:a=Su}return n=Lx.bind(null,e),a=Rf(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&Xd(n),e.callbackPriority=2,e.callbackNode=null,2}function Lx(e,t){if(bt!==0&&bt!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(lc(!0)&&e.callbackNode!==a)return null;var n=he;return n=Ju(e,e===Ee?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:($x(e,n,t),Ox(e,Ga()),e.callbackNode!=null&&e.callbackNode===a?Lx.bind(null,e):null)}function Jg(e,t){if(lc())return null;$x(e,t,!0)}function KE(){ZE(function(){(Se&6)!==0?Rf(Sy,HE):Mx()})}function cp(){return Ar===0&&(Ar=Ry()),Ar}function Xg(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:lu(""+e)}function Wg(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function QE(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=Xg((r[qt]||null).action),i=n.submitter;i&&(t=(t=i[qt]||null)?Xg(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Xu("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(Ar!==0){var l=i?Wg(r,i):new FormData(r);tf(a,{pending:!0,data:l,method:r.method,action:s},null,l)}}else typeof s=="function"&&(o.preventDefault(),l=i?Wg(r,i):new FormData(r),tf(a,{pending:!0,data:l,method:r.method,action:s},s,l))},currentTarget:r}]})}}for(au=0;au<Im.length;au++)nu=Im[au],Zg=nu.toLowerCase(),ey=nu[0].toUpperCase()+nu.slice(1),Ea(Zg,"on"+ey);var nu,Zg,ey,au;Ea(Jy,"onAnimationEnd");Ea(Xy,"onAnimationIteration");Ea(Wy,"onAnimationStart");Ea("dblclick","onDoubleClick");Ea("focusin","onFocus");Ea("focusout","onBlur");Ea(dE,"onTransitionRun");Ea(mE,"onTransitionStart");Ea(fE,"onTransitionCancel");Ea(Zy,"onTransitionEnd");js("onMouseEnter",["mouseout","mouseover"]);js("onMouseLeave",["mouseout","mouseover"]);js("onPointerEnter",["pointerout","pointerover"]);js("onPointerLeave",["pointerout","pointerover"]);Pr("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Pr("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Pr("onBeforeInput",["compositionend","keypress","textInput","paste"]);Pr("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Pr("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Pr("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var Ro="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),VE=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(Ro));function Px(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],l=o.instance,c=o.currentTarget;if(o=o.listener,l!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Lu(d)}r.currentTarget=null,s=l}else for(i=0;i<n.length;i++){if(o=n[i],l=o.instance,c=o.currentTarget,o=o.listener,l!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Lu(d)}r.currentTarget=null,s=l}}}}function de(e,t){var a=t[Pm];a===void 0&&(a=t[Pm]=new Set);var n=e+"__bubble";a.has(n)||(Ux(t,e,2,!1),a.add(n))}function _m(e,t,a){var n=0;t&&(n|=4),Ux(a,e,n,t)}var ru="_reactListening"+Math.random().toString(36).slice(2);function dp(e){if(!e[ru]){e[ru]=!0,Ay.forEach(function(a){a!=="selectionchange"&&(VE.has(a)||_m(a,!1,e),_m(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[ru]||(t[ru]=!0,_m("selectionchange",!1,t))}}function Ux(e,t,a,n){switch(Yx(t)){case 2:var r=x3;break;case 8:r=$3;break;default:r=hp}a=r.bind(null,t,a,e),r=void 0,!zm||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function Rm(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var l=i.tag;if((l===3||l===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=bs(o),i===null)return;if(l=i.tag,l===5||l===6||l===26||l===27){n=s=i;continue e}o=o.parentNode}}n=n.return}Fy(function(){var c=s,d=Af(a),m=[];e:{var f=eb.get(e);if(f!==void 0){var h=Xu,x=e;switch(e){case"keypress":if(cu(a)===0)break e;case"keydown":case"keyup":h=IC;break;case"focusin":x="focus",h=sm;break;case"focusout":x="blur",h=sm;break;case"beforeblur":case"afterblur":h=sm;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":h=ig;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":h=AC;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":h=QC;break;case Jy:case Xy:case Wy:h=OC;break;case Zy:h=GC;break;case"scroll":case"scrollend":h=EC;break;case"wheel":h=JC;break;case"copy":case"cut":case"paste":h=PC;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":h=lg;break;case"toggle":case"beforetoggle":h=WC}var y=(t&4)!==0,$=!y&&(e==="scroll"||e==="scrollend"),g=y?f!==null?f+"Capture":null:f;y=[];for(var v=c,b;v!==null;){var w=v;if(b=w.stateNode,w=w.tag,w!==5&&w!==26&&w!==27||b===null||g===null||(w=xo(v,g),w!=null&&y.push(ko(v,w,b))),$)break;v=v.return}0<y.length&&(f=new h(f,x,null,a,d),m.push({event:f,listeners:y}))}}if((t&7)===0){e:{if(f=e==="mouseover"||e==="pointerover",h=e==="mouseout"||e==="pointerout",f&&a!==Fm&&(x=a.relatedTarget||a.fromElement)&&(bs(x)||x[Gs]))break e;if((h||f)&&(f=d.window===d?d:(f=d.ownerDocument)?f.defaultView||f.parentWindow:window,h?(x=a.relatedTarget||a.toElement,h=c,x=x?bs(x):null,x!==null&&($=Mo(x),y=x.tag,x!==$||y!==5&&y!==27&&y!==6)&&(x=null)):(h=null,x=c),h!==x)){if(y=ig,w="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(y=lg,w="onPointerLeave",g="onPointerEnter",v="pointer"),$=h==null?f:ao(h),b=x==null?f:ao(x),f=new y(w,v+"leave",h,a,d),f.target=$,f.relatedTarget=b,w=null,bs(d)===c&&(y=new y(g,v+"enter",x,a,d),y.target=b,y.relatedTarget=$,w=y),$=w,h&&x)t:{for(y=h,g=x,v=0,b=y;b;b=fs(b))v++;for(b=0,w=g;w;w=fs(w))b++;for(;0<v-b;)y=fs(y),v--;for(;0<b-v;)g=fs(g),b--;for(;v--;){if(y===g||g!==null&&y===g.alternate)break t;y=fs(y),g=fs(g)}y=null}else y=null;h!==null&&ty(m,f,h,y,!1),x!==null&&$!==null&&ty(m,$,x,y,!0)}}e:{if(f=c?ao(c):window,h=f.nodeName&&f.nodeName.toLowerCase(),h==="select"||h==="input"&&f.type==="file")var S=mg;else if(dg(f))if(Ky)S=lE;else{S=iE;var C=sE}else h=f.nodeName,!h||h.toLowerCase()!=="input"||f.type!=="checkbox"&&f.type!=="radio"?c&&Tf(c.elementType)&&(S=mg):S=oE;if(S&&(S=S(e,c))){Hy(m,S,a,d);break e}C&&C(e,f,c),e==="focusout"&&c&&f.type==="number"&&c.memoizedProps.value!=null&&jm(f,"number",f.value)}switch(C=c?ao(c):window,e){case"focusin":(dg(C)||C.contentEditable==="true")&&(ws=C,Bm=c,io=null);break;case"focusout":io=Bm=ws=null;break;case"mousedown":qm=!0;break;case"contextmenu":case"mouseup":case"dragend":qm=!1,vg(m,a,d);break;case"selectionchange":if(cE)break;case"keydown":case"keyup":vg(m,a,d)}var R;if(Of)e:{switch(e){case"compositionstart":var _="onCompositionStart";break e;case"compositionend":_="onCompositionEnd";break e;case"compositionupdate":_="onCompositionUpdate";break e}_=void 0}else $s?qy(e,a)&&(_="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(_="onCompositionStart");_&&(By&&a.locale!=="ko"&&($s||_!=="onCompositionStart"?_==="onCompositionEnd"&&$s&&(R=zy()):(Qn=d,Df="value"in Qn?Qn.value:Qn.textContent,$s=!0)),C=Iu(c,_),0<C.length&&(_=new og(_,e,null,a,d),m.push({event:_,listeners:C}),R?_.data=R:(R=Iy(a),R!==null&&(_.data=R)))),(R=eE?tE(e,a):aE(e,a))&&(_=Iu(c,"onBeforeInput"),0<_.length&&(C=new og("onBeforeInput","beforeinput",null,a,d),m.push({event:C,listeners:_}),C.data=R)),QE(m,e,c,a,d)}Px(m,t)})}function ko(e,t,a){return{instance:e,listener:t,currentTarget:a}}function Iu(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=xo(e,a),r!=null&&n.unshift(ko(e,r,s)),r=xo(e,t),r!=null&&n.push(ko(e,r,s))),e.tag===3)return n;e=e.return}return[]}function fs(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function ty(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,l=o.alternate,c=o.stateNode;if(o=o.tag,l!==null&&l===n)break;o!==5&&o!==26&&o!==27||c===null||(l=c,r?(c=xo(a,s),c!=null&&i.unshift(ko(a,c,l))):r||(c=xo(a,s),c!=null&&i.push(ko(a,c,l)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var GE=/\r\n?/g,YE=/\u0000|\uFFFD/g;function ay(e){return(typeof e=="string"?e:""+e).replace(GE,`
`).replace(YE,"")}function jx(e,t){return t=ay(t),ay(e)===t}function uc(){}function Ne(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||Fs(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&Fs(e,""+n);break;case"className":Vl(e,"class",n);break;case"tabIndex":Vl(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":Vl(e,a,n);break;case"style":jy(e,n,s);break;case"data":if(t!=="object"){Vl(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=lu(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&Ne(e,t,"name",r.name,r,null),Ne(e,t,"formEncType",r.formEncType,r,null),Ne(e,t,"formMethod",r.formMethod,r,null),Ne(e,t,"formTarget",r.formTarget,r,null)):(Ne(e,t,"encType",r.encType,r,null),Ne(e,t,"method",r.method,r,null),Ne(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=lu(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=uc);break;case"onScroll":n!=null&&de("scroll",e);break;case"onScrollEnd":n!=null&&de("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(j(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(j(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=lu(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":de("beforetoggle",e),de("toggle",e),ou(e,"popover",n);break;case"xlinkActuate":cn(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":cn(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":cn(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":cn(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":cn(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":cn(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":cn(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":cn(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":cn(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":ou(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=kC.get(a)||a,ou(e,a,n))}}function pf(e,t,a,n,r,s){switch(a){case"style":jy(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(j(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(j(60));e.innerHTML=a}}break;case"children":typeof n=="string"?Fs(e,n):(typeof n=="number"||typeof n=="bigint")&&Fs(e,""+n);break;case"onScroll":n!=null&&de("scroll",e);break;case"onScrollEnd":n!=null&&de("scrollend",e);break;case"onClick":n!=null&&(e.onclick=uc);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!Dy.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[qt]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):ou(e,a,n)}}}function xt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":de("error",e),de("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(j(137,t));default:Ne(e,t,s,i,a,null)}}r&&Ne(e,t,"srcSet",a.srcSet,a,null),n&&Ne(e,t,"src",a.src,a,null);return;case"input":de("invalid",e);var o=s=i=r=null,l=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":l=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(j(137,t));break;default:Ne(e,t,n,d,a,null)}}Ly(e,s,o,l,c,i,r,!1),Nu(e);return;case"select":de("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:Ne(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?Ts(e,!!n,t,!1):a!=null&&Ts(e,!!n,a,!0);return;case"textarea":de("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(j(91));break;default:Ne(e,t,i,o,a,null)}Uy(e,n,r,s),Nu(e);return;case"option":for(l in a)if(a.hasOwnProperty(l)&&(n=a[l],n!=null))switch(l){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:Ne(e,t,l,n,a,null)}return;case"dialog":de("beforetoggle",e),de("toggle",e),de("cancel",e),de("close",e);break;case"iframe":case"object":de("load",e);break;case"video":case"audio":for(n=0;n<Ro.length;n++)de(Ro[n],e);break;case"image":de("error",e),de("load",e);break;case"details":de("toggle",e);break;case"embed":case"source":case"link":de("error",e),de("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(j(137,t));default:Ne(e,t,c,n,a,null)}return;default:if(Tf(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&pf(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&Ne(e,t,o,n,a,null))}function JE(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,l=null,c=null,d=null;for(h in a){var m=a[h];if(a.hasOwnProperty(h)&&m!=null)switch(h){case"checked":break;case"value":break;case"defaultValue":l=m;default:n.hasOwnProperty(h)||Ne(e,t,h,null,n,m)}}for(var f in n){var h=n[f];if(m=a[f],n.hasOwnProperty(f)&&(h!=null||m!=null))switch(f){case"type":s=h;break;case"name":r=h;break;case"checked":c=h;break;case"defaultChecked":d=h;break;case"value":i=h;break;case"defaultValue":o=h;break;case"children":case"dangerouslySetInnerHTML":if(h!=null)throw Error(j(137,t));break;default:h!==m&&Ne(e,t,f,h,n,m)}}Um(e,i,o,l,c,d,s,r);return;case"select":h=i=o=f=null;for(s in a)if(l=a[s],a.hasOwnProperty(s)&&l!=null)switch(s){case"value":break;case"multiple":h=l;default:n.hasOwnProperty(s)||Ne(e,t,s,null,n,l)}for(r in n)if(s=n[r],l=a[r],n.hasOwnProperty(r)&&(s!=null||l!=null))switch(r){case"value":f=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==l&&Ne(e,t,r,s,n,l)}t=o,a=i,n=h,f!=null?Ts(e,!!a,f,!1):!!n!=!!a&&(t!=null?Ts(e,!!a,t,!0):Ts(e,!!a,a?[]:"",!1));return;case"textarea":h=f=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:Ne(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":f=r;break;case"defaultValue":h=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(j(91));break;default:r!==s&&Ne(e,t,i,r,n,s)}Py(e,f,h);return;case"option":for(var x in a)if(f=a[x],a.hasOwnProperty(x)&&f!=null&&!n.hasOwnProperty(x))switch(x){case"selected":e.selected=!1;break;default:Ne(e,t,x,null,n,f)}for(l in n)if(f=n[l],h=a[l],n.hasOwnProperty(l)&&f!==h&&(f!=null||h!=null))switch(l){case"selected":e.selected=f&&typeof f!="function"&&typeof f!="symbol";break;default:Ne(e,t,l,f,n,h)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var y in a)f=a[y],a.hasOwnProperty(y)&&f!=null&&!n.hasOwnProperty(y)&&Ne(e,t,y,null,n,f);for(c in n)if(f=n[c],h=a[c],n.hasOwnProperty(c)&&f!==h&&(f!=null||h!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(f!=null)throw Error(j(137,t));break;default:Ne(e,t,c,f,n,h)}return;default:if(Tf(t)){for(var $ in a)f=a[$],a.hasOwnProperty($)&&f!==void 0&&!n.hasOwnProperty($)&&pf(e,t,$,void 0,n,f);for(d in n)f=n[d],h=a[d],!n.hasOwnProperty(d)||f===h||f===void 0&&h===void 0||pf(e,t,d,f,n,h);return}}for(var g in a)f=a[g],a.hasOwnProperty(g)&&f!=null&&!n.hasOwnProperty(g)&&Ne(e,t,g,null,n,f);for(m in n)f=n[m],h=a[m],!n.hasOwnProperty(m)||f===h||f==null&&h==null||Ne(e,t,m,f,n,h)}var hf=null,vf=null;function Hu(e){return e.nodeType===9?e:e.ownerDocument}function ny(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function Fx(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function gf(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var km=null;function XE(){var e=window.event;return e&&e.type==="popstate"?e===km?!1:(km=e,!0):(km=null,!1)}var zx=typeof setTimeout=="function"?setTimeout:void 0,WE=typeof clearTimeout=="function"?clearTimeout:void 0,ry=typeof Promise=="function"?Promise:void 0,ZE=typeof queueMicrotask=="function"?queueMicrotask:typeof ry<"u"?function(e){return ry.resolve(null).then(e).catch(e3)}:zx;function e3(e){setTimeout(function(){throw e})}function or(e){return e==="head"}function sy(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&yo(i.documentElement),a&2&&yo(i.body),a&4)for(a=i.head,yo(a),i=a.firstChild;i;){var o=i.nextSibling,l=i.nodeName;i[Uo]||l==="SCRIPT"||l==="STYLE"||l==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),Do(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);Do(t)}function yf(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":yf(a),Ef(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function t3(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[Uo])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=Ca(e.nextSibling),e===null)break}return null}function a3(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=Ca(e.nextSibling),e===null))return null;return e}function bf(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function n3(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function Ca(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var xf=null;function iy(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function Bx(e,t,a){switch(t=Hu(a),e){case"html":if(e=t.documentElement,!e)throw Error(j(452));return e;case"head":if(e=t.head,!e)throw Error(j(453));return e;case"body":if(e=t.body,!e)throw Error(j(454));return e;default:throw Error(j(451))}}function yo(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);Ef(e)}var $a=new Map,oy=new Set;function Ku(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var _n=be.d;be.d={f:r3,r:s3,D:i3,C:o3,L:l3,m:u3,X:d3,S:c3,M:m3};function r3(){var e=_n.f(),t=ic();return e||t}function s3(e){var t=Ys(e);t!==null&&t.tag===5&&t.type==="form"?Ob(t):_n.r(e)}var Ws=typeof document>"u"?null:document;function qx(e,t,a){var n=Ws;if(n&&typeof t=="string"&&t){var r=ga(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),oy.has(r)||(oy.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),xt(t,"link",e),ct(t),n.head.appendChild(t)))}}function i3(e){_n.D(e),qx("dns-prefetch",e,null)}function o3(e,t){_n.C(e,t),qx("preconnect",e,t)}function l3(e,t,a){_n.L(e,t,a);var n=Ws;if(n&&e&&t){var r='link[rel="preload"][as="'+ga(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+ga(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+ga(a.imageSizes)+'"]')):r+='[href="'+ga(e)+'"]';var s=r;switch(t){case"style":s=Vs(e);break;case"script":s=Zs(e)}$a.has(s)||(e=Me({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),$a.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(Vo(s))||t==="script"&&n.querySelector(Go(s))||(t=n.createElement("link"),xt(t,"link",e),ct(t),n.head.appendChild(t)))}}function u3(e,t){_n.m(e,t);var a=Ws;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+ga(n)+'"][href="'+ga(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=Zs(e)}if(!$a.has(s)&&(e=Me({rel:"modulepreload",href:e},t),$a.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(Go(s)))return}n=a.createElement("link"),xt(n,"link",e),ct(n),a.head.appendChild(n)}}}function c3(e,t,a){_n.S(e,t,a);var n=Ws;if(n&&e){var r=Es(n).hoistableStyles,s=Vs(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(Vo(s)))o.loading=5;else{e=Me({rel:"stylesheet",href:e,"data-precedence":t},a),(a=$a.get(s))&&mp(e,a);var l=i=n.createElement("link");ct(l),xt(l,"link",e),l._p=new Promise(function(c,d){l.onload=c,l.onerror=d}),l.addEventListener("load",function(){o.loading|=1}),l.addEventListener("error",function(){o.loading|=2}),o.loading|=4,gu(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function d3(e,t){_n.X(e,t);var a=Ws;if(a&&e){var n=Es(a).hoistableScripts,r=Zs(e),s=n.get(r);s||(s=a.querySelector(Go(r)),s||(e=Me({src:e,async:!0},t),(t=$a.get(r))&&fp(e,t),s=a.createElement("script"),ct(s),xt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function m3(e,t){_n.M(e,t);var a=Ws;if(a&&e){var n=Es(a).hoistableScripts,r=Zs(e),s=n.get(r);s||(s=a.querySelector(Go(r)),s||(e=Me({src:e,async:!0,type:"module"},t),(t=$a.get(r))&&fp(e,t),s=a.createElement("script"),ct(s),xt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function ly(e,t,a,n){var r=(r=Yn.current)?Ku(r):null;if(!r)throw Error(j(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=Vs(a.href),a=Es(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=Vs(a.href);var s=Es(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(Vo(e)))&&!s._p&&(i.instance=s,i.state.loading=5),$a.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},$a.set(e,a),s||f3(r,e,a,i.state))),t&&n===null)throw Error(j(528,""));return i}if(t&&n!==null)throw Error(j(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=Zs(a),a=Es(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(j(444,e))}}function Vs(e){return'href="'+ga(e)+'"'}function Vo(e){return'link[rel="stylesheet"]['+e+"]"}function Ix(e){return Me({},e,{"data-precedence":e.precedence,precedence:null})}function f3(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),xt(t,"link",a),ct(t),e.head.appendChild(t))}function Zs(e){return'[src="'+ga(e)+'"]'}function Go(e){return"script[async]"+e}function uy(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+ga(a.href)+'"]');if(n)return t.instance=n,ct(n),n;var r=Me({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),ct(n),xt(n,"style",r),gu(n,a.precedence,e),t.instance=n;case"stylesheet":r=Vs(a.href);var s=e.querySelector(Vo(r));if(s)return t.state.loading|=4,t.instance=s,ct(s),s;n=Ix(a),(r=$a.get(r))&&mp(n,r),s=(e.ownerDocument||e).createElement("link"),ct(s);var i=s;return i._p=new Promise(function(o,l){i.onload=o,i.onerror=l}),xt(s,"link",n),t.state.loading|=4,gu(s,a.precedence,e),t.instance=s;case"script":return s=Zs(a.src),(r=e.querySelector(Go(s)))?(t.instance=r,ct(r),r):(n=a,(r=$a.get(s))&&(n=Me({},a),fp(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),ct(r),xt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(j(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,gu(n,a.precedence,e));return t.instance}function gu(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function mp(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function fp(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var yu=null;function cy(e,t,a){if(yu===null){var n=new Map,r=yu=new Map;r.set(a,n)}else r=yu,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[Uo]||s[St]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function dy(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function p3(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function Hx(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var Co=null;function h3(){}function v3(e,t,a){if(Co===null)throw Error(j(475));var n=Co;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=Vs(a.href),s=e.querySelector(Vo(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=Qu.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,ct(s);return}s=e.ownerDocument||e,a=Ix(a),(r=$a.get(r))&&mp(a,r),s=s.createElement("link"),ct(s);var i=s;i._p=new Promise(function(o,l){i.onload=o,i.onerror=l}),xt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=Qu.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function g3(){if(Co===null)throw Error(j(475));var e=Co;return e.stylesheets&&e.count===0&&$f(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&$f(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function Qu(){if(this.count--,this.count===0){if(this.stylesheets)$f(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var Vu=null;function $f(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,Vu=new Map,t.forEach(y3,e),Vu=null,Qu.call(e))}function y3(e,t){if(!(t.state.loading&4)){var a=Vu.get(e);if(a)var n=a.get(null);else{a=new Map,Vu.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=Qu.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var Eo={$$typeof:pn,Provider:null,Consumer:null,_currentValue:_r,_currentValue2:_r,_threadCount:0};function b3(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=Wd(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=Wd(0),this.hiddenUpdates=Wd(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function Kx(e,t,a,n,r,s,i,o,l,c,d,m){return e=new b3(e,t,a,i,o,l,c,m),t=1,s===!0&&(t|=24),s=Xt(3,null,null,t),e.current=s,s.stateNode=e,t=Bf(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},If(s),e}function Qx(e){return e?(e=_s,e):_s}function Vx(e,t,a,n,r,s){r=Qx(r),n.context===null?n.context=r:n.pendingContext=r,n=Jn(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=Xn(e,n,t),a!==null&&(ta(a,e,t),uo(a,e,t))}function my(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function pp(e,t){my(e,t),(e=e.alternate)&&my(e,t)}function Gx(e){if(e.tag===13){var t=Js(e,67108864);t!==null&&ta(t,e,67108864),pp(e,67108864)}}var Gu=!0;function x3(e,t,a,n){var r=se.T;se.T=null;var s=be.p;try{be.p=2,hp(e,t,a,n)}finally{be.p=s,se.T=r}}function $3(e,t,a,n){var r=se.T;se.T=null;var s=be.p;try{be.p=8,hp(e,t,a,n)}finally{be.p=s,se.T=r}}function hp(e,t,a,n){if(Gu){var r=wf(n);if(r===null)Rm(e,t,n,Yu,a),fy(e,n);else if(S3(r,e,t,a,n))n.stopPropagation();else if(fy(e,n),t&4&&-1<w3.indexOf(e)){for(;r!==null;){var s=Ys(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=wr(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var l=1<<31-Zt(i);o.entanglements[1]|=l,i&=~l}Xa(s),(Se&6)===0&&(Fu=Ga()+500,Qo(0,!1))}}break;case 13:o=Js(s,2),o!==null&&ta(o,s,2),ic(),pp(s,2)}if(s=wf(n),s===null&&Rm(e,t,n,Yu,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else Rm(e,t,n,null,a)}}function wf(e){return e=Af(e),vp(e)}var Yu=null;function vp(e){if(Yu=null,e=bs(e),e!==null){var t=Mo(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=by(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return Yu=e,null}function Yx(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(uC()){case Sy:return 2;case Ny:return 8;case Su:case cC:return 32;case _y:return 268435456;default:return 32}default:return 32}}var Sf=!1,er=null,tr=null,ar=null,To=new Map,Ao=new Map,Hn=[],w3="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function fy(e,t){switch(e){case"focusin":case"focusout":er=null;break;case"dragenter":case"dragleave":tr=null;break;case"mouseover":case"mouseout":ar=null;break;case"pointerover":case"pointerout":To.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":Ao.delete(t.pointerId)}}function Wi(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Ys(t),t!==null&&Gx(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function S3(e,t,a,n,r){switch(t){case"focusin":return er=Wi(er,e,t,a,n,r),!0;case"dragenter":return tr=Wi(tr,e,t,a,n,r),!0;case"mouseover":return ar=Wi(ar,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return To.set(s,Wi(To.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,Ao.set(s,Wi(Ao.get(s)||null,e,t,a,n,r)),!0}return!1}function Jx(e){var t=bs(e.target);if(t!==null){var a=Mo(t);if(a!==null){if(t=a.tag,t===13){if(t=by(a),t!==null){e.blockedOn=t,yC(e.priority,function(){if(a.tag===13){var n=ea();n=kf(n);var r=Js(a,n);r!==null&&ta(r,a,n),pp(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function bu(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=wf(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);Fm=n,a.target.dispatchEvent(n),Fm=null}else return t=Ys(a),t!==null&&Gx(t),e.blockedOn=a,!1;t.shift()}return!0}function py(e,t,a){bu(e)&&a.delete(t)}function N3(){Sf=!1,er!==null&&bu(er)&&(er=null),tr!==null&&bu(tr)&&(tr=null),ar!==null&&bu(ar)&&(ar=null),To.forEach(py),Ao.forEach(py)}function su(e,t){e.blockedOn===t&&(e.blockedOn=null,Sf||(Sf=!0,st.unstable_scheduleCallback(st.unstable_NormalPriority,N3)))}var iu=null;function hy(e){iu!==e&&(iu=e,st.unstable_scheduleCallback(st.unstable_NormalPriority,function(){iu===e&&(iu=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(vp(n||a)===null)continue;break}var s=Ys(a);s!==null&&(e.splice(t,3),t-=3,tf(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function Do(e){function t(l){return su(l,e)}er!==null&&su(er,e),tr!==null&&su(tr,e),ar!==null&&su(ar,e),To.forEach(t),Ao.forEach(t);for(var a=0;a<Hn.length;a++){var n=Hn[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<Hn.length&&(a=Hn[0],a.blockedOn===null);)Jx(a),a.blockedOn===null&&Hn.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[qt]||null;if(typeof s=="function")i||hy(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[qt]||null)o=i.formAction;else if(vp(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),hy(a)}}}function gp(e){this._internalRoot=e}cc.prototype.render=gp.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(j(409));var a=t.current,n=ea();Vx(a,n,e,t,null,null)};cc.prototype.unmount=gp.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;Vx(e.current,2,null,e,null,null),ic(),t[Gs]=null}};function cc(e){this._internalRoot=e}cc.prototype.unstable_scheduleHydration=function(e){if(e){var t=Ty();e={blockedOn:null,target:e,priority:t};for(var a=0;a<Hn.length&&t!==0&&t<Hn[a].priority;a++);Hn.splice(a,0,e),a===0&&Jx(e)}};var vy=gy.version;if(vy!=="19.1.0")throw Error(j(527,vy,"19.1.0"));be.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(j(188)):(e=Object.keys(e).join(","),Error(j(268,e)));return e=aC(t),e=e!==null?xy(e):null,e=e===null?null:e.stateNode,e};var _3={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:se,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(Zi=__REACT_DEVTOOLS_GLOBAL_HOOK__,!Zi.isDisabled&&Zi.supportsFiber))try{Oo=Zi.inject(_3),Wt=Zi}catch{}var Zi;dc.createRoot=function(e,t){if(!yy(e))throw Error(j(299));var a=!1,n="",r=Kb,s=Qb,i=Vb,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=Kx(e,1,!1,null,null,a,n,r,s,i,o,null),e[Gs]=t.current,dp(e),new gp(t)};dc.hydrateRoot=function(e,t,a){if(!yy(e))throw Error(j(299));var n=!1,r="",s=Kb,i=Qb,o=Vb,l=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(l=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=Kx(e,1,!0,t,a??null,n,r,s,i,o,l,c),t.context=Qx(null),a=t.current,n=ea(),n=kf(n),r=Jn(n),r.callback=null,Xn(a,r,n),a=n,t.current.lanes=a,Po(t,a),Xa(t),e[Gs]=t.current,dp(e),new cc(t)};dc.version="19.1.0"});var e0=Dn((wP,Zx)=>{"use strict";function Wx(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(Wx)}catch(e){console.error(e)}}Wx(),Zx.exports=Xx()});var Pt=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var Lk={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},Pk=class{#t=Lk;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},za=new Pk;function uv(e){setTimeout(e,0)}var Ut=typeof window>"u"||"Deno"in globalThis;function Pe(){}function mv(e,t){return typeof e=="function"?e(t):e}function Oi(e){return typeof e=="number"&&e>=0&&e!==1/0}function Sl(e,t){return Math.max(e+(t||0)-Date.now(),0)}function Ra(e,t){return typeof e=="function"?e(t):e}function jt(e,t){return typeof e=="function"?e(t):e}function Nl(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==Li(i,t.options))return!1}else if(!br(t.queryKey,i))return!1}if(a!=="all"){let l=t.isActive();if(a==="active"&&!l||a==="inactive"&&l)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function _l(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(Ba(t.options.mutationKey)!==Ba(s))return!1}else if(!br(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function Li(e,t){return(t?.queryKeyHashFn||Ba)(e)}function Ba(e){return JSON.stringify(e,(t,a)=>Ed(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function br(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>br(e[a],t[a])):!1}var Uk=Object.prototype.hasOwnProperty;function Pi(e,t){if(e===t)return e;let a=cv(e)&&cv(t);if(!a&&!(Ed(e)&&Ed(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},l=0;for(let c=0;c<i;c++){let d=a?c:s[c],m=e[d],f=t[d];if(m===f){o[d]=m,(a?c<r:Uk.call(e,d))&&l++;continue}if(m===null||f===null||typeof m!="object"||typeof f!="object"){o[d]=f;continue}let h=Pi(m,f);o[d]=h,h===m&&l++}return r===i&&l===r?e:o}function Mn(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function cv(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function Ed(e){if(!dv(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!dv(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function dv(e){return Object.prototype.toString.call(e)==="[object Object]"}function fv(e){return new Promise(t=>{za.setTimeout(t,e)})}function Ui(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?Pi(e,t):t}function pv(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function hv(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var rs=Symbol();function Rl(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===rs?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function ji(e,t){return typeof e=="function"?e(...t):!!e}var jk=class extends Pt{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!Ut&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},ss=new jk;function Fi(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var vv=uv;function Fk(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=vv,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(l=>{a(l)})})})};return{batch:o=>{let l;t++;try{l=o()}finally{t--,t||i()}return l},batchCalls:o=>(...l)=>{s(()=>{o(...l)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var pe=Fk();var zk=class extends Pt{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!Ut&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},is=new zk;function Bk(e){return Math.min(1e3*2**e,3e4)}function Td(e){return(e??"online")==="online"?is.isOnline():!0}var kl=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function Cl(e){let t=!1,a=0,n,r=Fi(),s=()=>r.status!=="pending",i=y=>{if(!s()){let $=new kl(y);f($),e.onCancel?.($)}},o=()=>{t=!0},l=()=>{t=!1},c=()=>ss.isFocused()&&(e.networkMode==="always"||is.isOnline())&&e.canRun(),d=()=>Td(e.networkMode)&&e.canRun(),m=y=>{s()||(n?.(),r.resolve(y))},f=y=>{s()||(n?.(),r.reject(y))},h=()=>new Promise(y=>{n=$=>{(s()||c())&&y($)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),x=()=>{if(s())return;let y,$=a===0?e.initialPromise:void 0;try{y=$??e.fn()}catch(g){y=Promise.reject(g)}Promise.resolve(y).then(m).catch(g=>{if(s())return;let v=e.retry??(Ut?0:3),b=e.retryDelay??Bk,w=typeof b=="function"?b(a,g):b,S=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!S){f(g);return}a++,e.onFail?.(a,g),fv(w).then(()=>c()?void 0:h()).then(()=>{t?f(g):x()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:l,canStart:d,start:()=>(d()?x():h().then(x),r)}}var El=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),Oi(this.gcTime)&&(this.#t=za.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(Ut?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(za.clearTimeout(this.#t),this.#t=void 0)}};var yv=class extends El{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=gv(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=gv(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=Ui(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(Pe).catch(Pe):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>jt(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===rs||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>Ra(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!Sl(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(l=>l.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=Rl(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=Cl({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof kl&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,l)=>{this.#i({type:"failed",failureCount:o,error:l})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof kl){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,...Ad(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),pe.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function Ad(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:Td(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function gv(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var xr=class extends Pt{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=Fi(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),bv(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return Dd(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return Dd(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof jt(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!Mn(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&xv(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||jt(this.options.enabled,this.#e)!==jt(t.enabled,this.#e)||Ra(this.options.staleTime,this.#e)!==Ra(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||jt(this.options.enabled,this.#e)!==jt(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return Ik(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(Pe)),t}#v(){this.#x();let e=Ra(this.options.staleTime,this.#e);if(Ut||this.#n.isStale||!Oi(e))return;let a=Sl(this.#n.dataUpdatedAt,e)+1;this.#u=za.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(Ut||jt(this.options.enabled,this.#e)===!1||!Oi(this.#l)||this.#l===0)&&(this.#c=za.setInterval(()=>{(this.options.refetchIntervalInBackground||ss.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(za.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(za.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,l=e!==a?e.state:this.#a,{state:c}=e,d={...c},m=!1,f;if(t._optimisticResults){let _=this.hasListeners(),M=!_&&bv(e,t),L=_&&xv(e,a,t,n);(M||L)&&(d={...d,...Ad(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:h,errorUpdatedAt:x,status:y}=d;f=d.data;let $=!1;if(t.placeholderData!==void 0&&f===void 0&&y==="pending"){let _;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(_=r.data,$=!0):_=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,_!==void 0&&(y="success",f=Ui(r?.data,_,t),m=!0)}if(t.select&&f!==void 0&&!$)if(r&&f===s?.data&&t.select===this.#f)f=this.#d;else try{this.#f=t.select,f=t.select(f),f=Ui(r?.data,f,t),this.#d=f,this.#i=null}catch(_){this.#i=_}this.#i&&(h=this.#i,f=this.#d,x=Date.now(),y="error");let g=d.fetchStatus==="fetching",v=y==="pending",b=y==="error",w=v&&g,S=f!==void 0,R={status:y,fetchStatus:d.fetchStatus,isPending:v,isSuccess:y==="success",isError:b,isInitialLoading:w,isLoading:w,data:f,dataUpdatedAt:d.dataUpdatedAt,error:h,errorUpdatedAt:x,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>l.dataUpdateCount||d.errorUpdateCount>l.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:b&&!S,isPaused:d.fetchStatus==="paused",isPlaceholderData:m,isRefetchError:b&&S,isStale:Md(e,t),refetch:this.refetch,promise:this.#o,isEnabled:jt(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let _=U=>{R.status==="error"?U.reject(R.error):R.data!==void 0&&U.resolve(R.data)},M=()=>{let U=this.#o=R.promise=Fi();_(U)},L=this.#o;switch(L.status){case"pending":e.queryHash===a.queryHash&&_(L);break;case"fulfilled":(R.status==="error"||R.data!==L.value)&&M();break;case"rejected":(R.status!=="error"||R.error!==L.reason)&&M();break}}return R}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),Mn(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){pe.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function qk(e,t){return jt(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function bv(e,t){return qk(e,t)||e.state.data!==void 0&&Dd(e,t,t.refetchOnMount)}function Dd(e,t,a){if(jt(t.enabled,e)!==!1&&Ra(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&Md(e,t)}return!1}function xv(e,t,a,n){return(e!==t||jt(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&Md(e,a)}function Md(e,t){return jt(t.enabled,e)!==!1&&e.isStaleByTime(Ra(t.staleTime,e))}function Ik(e,t){return!Mn(e.getCurrentResult(),t)}function Od(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},l=0,c=async()=>{let d=!1,m=x=>{Object.defineProperty(x,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},f=Rl(t.options,t.fetchOptions),h=async(x,y,$)=>{if(d)return Promise.reject();if(y==null&&x.pages.length)return Promise.resolve(x);let v=(()=>{let C={client:t.client,queryKey:t.queryKey,pageParam:y,direction:$?"backward":"forward",meta:t.options.meta};return m(C),C})(),b=await f(v),{maxPages:w}=t.options,S=$?hv:pv;return{pages:S(x.pages,b,w),pageParams:S(x.pageParams,y,w)}};if(r&&s.length){let x=r==="backward",y=x?Hk:$v,$={pages:s,pageParams:i},g=y(n,$);o=await h($,g,x)}else{let x=e??s.length;do{let y=l===0?i[0]??n.initialPageParam:$v(n,o);if(l>0&&y==null)break;o=await h(o,y),l++}while(l<x)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function $v(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function Hk(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var wv=class extends El{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||Ld(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=Cl({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),pe.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function Ld(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var Sv=class extends Pt{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new wv({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=Tl(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=Tl(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=Tl(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=Tl(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){pe.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>_l(t,a))}findAll(e={}){return this.getAll().filter(t=>_l(e,t))}notify(e){pe.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return pe.batch(()=>Promise.all(e.map(t=>t.continue().catch(Pe))))}};function Tl(e){return e.options.scope?.id}var Pd=class extends Pt{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),Mn(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&Ba(t.mutationKey)!==Ba(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??Ld();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){pe.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function Nv(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function Kk(e,t,a){let n=e.slice(0);return n[t]=a,n}var Ud=class extends Pt{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,pe.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,m)=>d!==a[m]),l=i||o,c=l?!0:s.some((d,m)=>{let f=this.#e[m];return!f||!Mn(d,f)});!l&&!c||(l&&(this.#r=r),this.#e=s,this.hasListeners()&&(l&&(Nv(a,r).forEach(d=>{d.destroy()}),Nv(r,a).forEach(d=>{d.subscribe(m=>{this.#c(d,m)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=Pi(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new xr(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=Kk(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&pe.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var _v=class extends Pt{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??Li(n,t),s=this.get(r);return s||(s=new yv({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){pe.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>Nl(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>Nl(e,a)):t}notify(e){pe.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){pe.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){pe.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var jd=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new _v,this.#e=e.mutationCache||new Sv,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=ss.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=is.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime(Ra(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=mv(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return pe.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;pe.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return pe.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=pe.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(Pe).catch(Pe)}invalidateQueries(e,t={}){return pe.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=pe.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(Pe)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(Pe)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime(Ra(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(Pe).catch(Pe)}fetchInfiniteQuery(e){return e.behavior=Od(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(Pe).catch(Pe)}ensureInfiniteQueryData(e){return e.behavior=Od(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return is.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(Ba(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{br(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(Ba(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{br(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=Li(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===rs&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var qa=qe(Qe(),1);var os=qe(Qe(),1),Ev=qe(Fd(),1),zd=os.createContext(void 0),Z=e=>{let t=os.useContext(zd);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},Bd=({client:e,children:t})=>(os.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,Ev.jsx)(zd.Provider,{value:e,children:t}));var Dl=qe(Qe(),1),Tv=Dl.createContext(!1),Ml=()=>Dl.useContext(Tv),j6=Tv.Provider;var zi=qe(Qe(),1),Gk=qe(Fd(),1);function Yk(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var Jk=zi.createContext(Yk()),Ol=()=>zi.useContext(Jk);var Av=qe(Qe(),1);var Ll=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},Pl=e=>{Av.useEffect(()=>{e.clearReset()},[e])},Ul=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||ji(a,[e.error,n]));var jl=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},Fl=(e,t)=>e.isLoading&&e.isFetching&&!t,Bi=(e,t)=>e?.suspense&&t.isPending,ls=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function qd({queries:e,...t},a){let n=Z(a),r=Ml(),s=Ol(),i=qa.useMemo(()=>e.map(y=>{let $=n.defaultQueryOptions(y);return $._optimisticResults=r?"isRestoring":"optimistic",$}),[e,n,r]);i.forEach(y=>{jl(y),Ll(y,s)}),Pl(s);let[o]=qa.useState(()=>new Ud(n,i,t)),[l,c,d]=o.getOptimisticResult(i,t.combine),m=!r&&t.subscribed!==!1;qa.useSyncExternalStore(qa.useCallback(y=>m?o.subscribe(pe.batchCalls(y)):Pe,[o,m]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),qa.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let h=l.some((y,$)=>Bi(i[$],y))?l.flatMap((y,$)=>{let g=i[$];if(g){let v=new xr(n,g);if(Bi(g,y))return ls(g,v,s);Fl(y,r)&&ls(g,v,s)}return[]}):[];if(h.length>0)throw Promise.all(h);let x=l.find((y,$)=>{let g=i[$];return g&&Ul({result:y,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(x?.error)throw x.error;return c(d())}var On=qe(Qe(),1);function Dv(e,t,a){let n=Ml(),r=Ol(),s=Z(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",jl(i),Ll(i,r),Pl(r);let o=!s.getQueryCache().get(i.queryHash),[l]=On.useState(()=>new t(s,i)),c=l.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(On.useSyncExternalStore(On.useCallback(m=>{let f=d?l.subscribe(pe.batchCalls(m)):Pe;return l.updateResult(),f},[l,d]),()=>l.getCurrentResult(),()=>l.getCurrentResult()),On.useEffect(()=>{l.setOptions(i)},[i,l]),Bi(i,c))throw ls(i,l,r);if(Ul({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!Ut&&Fl(c,n)&&(o?ls(i,l,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(Pe).finally(()=>{l.updateResult()}),i.notifyOnChangeProps?c:l.trackResult(c)}function K(e,t){return Dv(e,xr,t)}var ln=qe(Qe(),1);function Y(e,t){let a=Z(t),[n]=ln.useState(()=>new Pd(a,e));ln.useEffect(()=>{n.setOptions(e)},[n,e]);let r=ln.useSyncExternalStore(ln.useCallback(i=>n.subscribe(pe.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=ln.useCallback((i,o)=>{n.mutate(i,o).catch(Pe)},[n]);if(r.error&&ji(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var Dk=qe(e0());var Ht=qe(Qe(),1),W=qe(Qe(),1),ke=qe(Qe(),1),Pp=qe(Qe(),1),O0=qe(Qe(),1),fe=qe(Qe(),1),FT=qe(Qe(),1),zT=qe(Qe(),1),BT=qe(Qe(),1),te=qe(Qe(),1),Q0=qe(Qe(),1);var t0="popstate";function a0(e){return typeof e=="object"&&e!=null&&"pathname"in e&&"search"in e&&"hash"in e&&"state"in e&&"key"in e}function u0(e={}){function t(n,r){let s=r.state?.masked,{pathname:i,search:o,hash:l}=s||n.location;return $p("",{pathname:i,search:o,hash:l},r.state&&r.state.usr||null,r.state&&r.state.key||"default",s?{pathname:n.location.pathname,search:n.location.search,hash:n.location.hash}:void 0)}function a(n,r){return typeof r=="string"?r:ei(r)}return k3(t,a,null,e)}function Te(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function na(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function R3(){return Math.random().toString(36).substring(2,10)}function n0(e,t){return{usr:e.state,key:e.key,idx:t,masked:e.mask?{pathname:e.pathname,search:e.search,hash:e.hash}:void 0}}function $p(e,t,a=null,n,r){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?zr(t):t,state:a,key:t&&t.key||n||R3(),mask:r}}function ei({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function zr(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function k3(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",l=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function m(){o="POP";let $=d(),g=$==null?null:$-c;c=$,l&&l({action:o,location:y.location,delta:g})}function f($,g){o="PUSH";let v=a0($)?$:$p(y.location,$,g);a&&a(v,$),c=d()+1;let b=n0(v,c),w=y.createHref(v.mask||v);try{i.pushState(b,"",w)}catch(S){if(S instanceof DOMException&&S.name==="DataCloneError")throw S;r.location.assign(w)}s&&l&&l({action:o,location:y.location,delta:1})}function h($,g){o="REPLACE";let v=a0($)?$:$p(y.location,$,g);a&&a(v,$),c=d();let b=n0(v,c),w=y.createHref(v.mask||v);i.replaceState(b,"",w),s&&l&&l({action:o,location:y.location,delta:0})}function x($){return C3($)}let y={get action(){return o},get location(){return e(r,i)},listen($){if(l)throw new Error("A history only accepts one active listener");return r.addEventListener(t0,m),l=$,()=>{r.removeEventListener(t0,m),l=null}},createHref($){return t(r,$)},createURL:x,encodeLocation($){let g=x($);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:f,replace:h,go($){return i.go($)}};return y}function C3(e,t=!1){let a="http://localhost";typeof window<"u"&&(a=window.location.origin!=="null"?window.location.origin:window.location.href),Te(a,"No window.location.(origin|href) available to create URL");let n=typeof e=="string"?e:ei(e);return n=n.replace(/ $/,"%20"),!t&&n.startsWith("//")&&(n=a+n),new URL(n,a)}var E3;E3=new WeakMap;function _p(e,t,a="/"){return T3(e,t,a,!1)}function T3(e,t,a,n,r){let s=typeof t=="string"?zr(t):t,i=Wa(s.pathname||"/",a);if(i==null)return null;let o=r??D3(e),l=null,c=H3(i);for(let d=0;l==null&&d<o.length;++d)l=q3(o[d],c,n);return l}function A3(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function D3(e){let t=c0(e);return M3(t),t}function c0(e,t=[],a=[],n="",r=!1){let s=(i,o,l=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&l)return;Te(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let m=Ta([n,d.relativePath]),f=a.concat(d);i.children&&i.children.length>0&&(Te(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${m}".`),c0(i.children,t,f,m,l)),!(i.path==null&&!i.index)&&t.push({path:m,score:z3(m,i.index),routesMeta:f})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let l of d0(i.path))s(i,o,!0,l)}),t}function d0(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=d0(n.join("/")),o=[];return o.push(...i.map(l=>l===""?s:[s,l].join("/"))),r&&o.push(...i),o.map(l=>e.startsWith("/")&&l===""?"/":l)}function M3(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:B3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var O3=/^:[\w-]+$/,L3=3,P3=2,U3=1,j3=10,F3=-2,r0=e=>e==="*";function z3(e,t){let a=e.split("/"),n=a.length;return a.some(r0)&&(n+=F3),t&&(n+=P3),a.filter(r=>!r0(r)).reduce((r,s)=>r+(O3.test(s)?L3:s===""?U3:j3),n)}function B3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function q3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let l=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",m=Jo({path:l.relativePath,caseSensitive:l.caseSensitive,end:c},d),f=l.route;if(!m&&c&&a&&!n[n.length-1].route.index&&(m=Jo({path:l.relativePath,caseSensitive:l.caseSensitive,end:!1},d)),!m)return null;Object.assign(r,m.params),i.push({params:r,pathname:Ta([s,m.pathname]),pathnameBase:V3(Ta([s,m.pathnameBase])),route:f}),m.pathnameBase!=="/"&&(s=Ta([s,m.pathnameBase]))}return i}function Jo(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=I3(e.path,e.caseSensitive,e.end),r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:m},f)=>{if(d==="*"){let x=o[f]||"";i=s.slice(0,s.length-x.length).replace(/(.)\/+$/,"$1")}let h=o[f];return m&&!h?c[d]=void 0:c[d]=(h||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function I3(e,t=!1,a=!0){na(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,l,c,d)=>{if(n.push({paramName:o,isOptional:l!=null}),l){let m=d.charAt(c+i.length);return m&&m!=="/"?"/([^\\/]*)":"(?:/([^\\/]*))?"}return"/([^\\/]+)"}).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function H3(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return na(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function Wa(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}var K3=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i;function m0(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?zr(e):e,s;return a?(a=f0(a),a.startsWith("/")?s=s0(a.substring(1),"/"):s=s0(a,t)):s=t,{pathname:s,search:G3(n),hash:Y3(r)}}function s0(e,t){let a=vc(t).split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function yp(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function Q3(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function Rp(e){let t=Q3(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function yc(e,t,a,n=!1){let r;typeof e=="string"?r=zr(e):(r={...e},Te(!r.pathname||!r.pathname.includes("?"),yp("?","pathname","search",r)),Te(!r.pathname||!r.pathname.includes("#"),yp("#","pathname","hash",r)),Te(!r.search||!r.search.includes("#"),yp("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let m=t.length-1;if(!n&&i.startsWith("..")){let f=i.split("/");for(;f[0]==="..";)f.shift(),m-=1;r.pathname=f.join("/")}o=m>=0?t[m]:"/"}let l=m0(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!l.pathname.endsWith("/")&&(c||d)&&(l.pathname+="/"),l}var f0=e=>e.replace(/\/\/+/g,"/"),Ta=e=>f0(e.join("/")),vc=e=>e.replace(/\/+$/,""),V3=e=>vc(e).replace(/^\/*/,"/"),G3=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,Y3=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;var p0=class{constructor(e,t,a,n=!1){this.status=e,this.statusText=t||"",this.internal=n,a instanceof Error?(this.data=a.toString(),this.error=a):this.data=a}};function h0(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}function J3(e){let t=e.map(a=>a.route.path).filter(Boolean);return Ta(t)||"/"}var v0=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";function g0(e,t){let a=e;if(typeof a!="string"||!K3.test(a))return{absoluteURL:void 0,isExternal:!1,to:a};let n=a,r=!1;if(v0)try{let s=new URL(window.location.href),i=a.startsWith("//")?new URL(s.protocol+a):new URL(a),o=Wa(i.pathname,t);i.origin===s.origin&&o!=null?a=o+i.search+i.hash:r=!0}catch{na(!1,`<Link to="${a}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}return{absoluteURL:n,isExternal:r,to:a}}var SP=Symbol("Uninstrumented");var NP=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");var y0=["POST","PUT","PATCH","DELETE"],_P=new Set(y0),X3=["GET",...y0],RP=new Set(X3);var kP=Symbol("ResetLoaderData"),W3,Z3,eT,tT;W3=new WeakMap;Z3=new WeakMap;eT=new WeakMap;tT=new WeakMap;var Br=Ht.createContext(null);Br.displayName="DataRouter";var ti=Ht.createContext(null);ti.displayName="DataRouterState";var b0=Ht.createContext(!1);function aT(){return Ht.useContext(b0)}var kp=Ht.createContext({isTransitioning:!1});kp.displayName="ViewTransition";var x0=Ht.createContext(new Map);x0.displayName="Fetchers";var nT=Ht.createContext(null);nT.displayName="Await";var _t=Ht.createContext(null);_t.displayName="Navigation";var ai=Ht.createContext(null);ai.displayName="Location";var ra=Ht.createContext({outlet:null,matches:[],isDataRoute:!1});ra.displayName="Route";var Cp=Ht.createContext(null);Cp.displayName="RouteError";var wp=!0,$0="REACT_ROUTER_ERROR",rT="REDIRECT",sT="ROUTE_ERROR_RESPONSE";function iT(e){if(e.startsWith(`${$0}:${rT}:{`))try{let t=JSON.parse(e.slice(28));if(typeof t=="object"&&t&&typeof t.status=="number"&&typeof t.statusText=="string"&&typeof t.location=="string"&&typeof t.reloadDocument=="boolean"&&typeof t.replace=="boolean")return t}catch{}}function oT(e){if(e.startsWith(`${$0}:${sT}:{`))try{let t=JSON.parse(e.slice(40));if(typeof t=="object"&&t&&typeof t.status=="number"&&typeof t.statusText=="string")return new p0(t.status,t.statusText,t.data)}catch{}}function w0(e,{relative:t}={}){Te(qr(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=W.useContext(_t),{hash:r,pathname:s,search:i}=ni(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:Ta([a,s])),n.createHref({pathname:o,search:i,hash:r})}function qr(){return W.useContext(ai)!=null}function Ae(){return Te(qr(),"useLocation() may be used only in the context of a <Router> component."),W.useContext(ai).location}var S0="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function N0(e){W.useContext(_t).static||W.useLayoutEffect(e)}function ve(){let{isDataRoute:e}=W.useContext(ra);return e?gT():lT()}function lT(){Te(qr(),"useNavigate() may be used only in the context of a <Router> component.");let e=W.useContext(Br),{basename:t,navigator:a}=W.useContext(_t),{matches:n}=W.useContext(ra),{pathname:r}=Ae(),s=JSON.stringify(Rp(n)),i=W.useRef(!1);return N0(()=>{i.current=!0}),W.useCallback((l,c={})=>{if(na(i.current,S0),!i.current)return;if(typeof l=="number"){a.go(l);return}let d=yc(l,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:Ta([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var _0=W.createContext(null);function wa(){return W.useContext(_0)}function R0(e){let t=W.useContext(ra).outlet;return W.useMemo(()=>t&&W.createElement(_0.Provider,{value:e},t),[t,e])}function it(){let{matches:e}=W.useContext(ra);return e[e.length-1]?.params??{}}function ni(e,{relative:t}={}){let{matches:a}=W.useContext(ra),{pathname:n}=Ae(),r=JSON.stringify(Rp(a));return W.useMemo(()=>yc(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function k0(e,t){return C0(e,t)}function C0(e,t,a){Te(qr(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:n}=W.useContext(_t),{matches:r}=W.useContext(ra),s=r[r.length-1],i=s?s.params:{},o=s?s.pathname:"/",l=s?s.pathnameBase:"/",c=s&&s.route;if(wp){let $=c&&c.path||"";D0(o,!c||$.endsWith("*")||$.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${o}" (under <Route path="${$}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${$}"> to <Route path="${$==="/"?"*":`${$}/*`}">.`)}let d=Ae(),m;if(t){let $=typeof t=="string"?zr(t):t;Te(l==="/"||$.pathname?.startsWith(l),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${l}" but pathname "${$.pathname}" was given in the \`location\` prop.`),m=$}else m=d;let f=m.pathname||"/",h=f;if(l!=="/"){let $=l.replace(/^\//,"").split("/");h="/"+f.replace(/^\//,"").split("/").slice($.length).join("/")}let x=a&&a.state.matches.length?a.state.matches.map($=>Object.assign($,{route:a.manifest[$.route.id]||$.route})):_p(e,{pathname:h});wp&&(na(c||x!=null,`No routes matched location "${m.pathname}${m.search}${m.hash}" `),na(x==null||x[x.length-1].route.element!==void 0||x[x.length-1].route.Component!==void 0||x[x.length-1].route.lazy!==void 0,`Matched leaf route at location "${m.pathname}${m.search}${m.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let y=fT(x&&x.map($=>Object.assign({},$,{params:Object.assign({},i,$.params),pathname:Ta([l,n.encodeLocation?n.encodeLocation($.pathname.replace(/%/g,"%25").replace(/\?/g,"%3F").replace(/#/g,"%23")).pathname:$.pathname]),pathnameBase:$.pathnameBase==="/"?l:Ta([l,n.encodeLocation?n.encodeLocation($.pathnameBase.replace(/%/g,"%25").replace(/\?/g,"%3F").replace(/#/g,"%23")).pathname:$.pathnameBase])})),r,a);return t&&y?W.createElement(ai.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",mask:void 0,...m},navigationType:"POP"}},y):y}function uT(){let e=A0(),t=h0(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return wp&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=W.createElement(W.Fragment,null,W.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),W.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",W.createElement("code",{style:s},"ErrorBoundary")," or"," ",W.createElement("code",{style:s},"errorElement")," prop on your route."))),W.createElement(W.Fragment,null,W.createElement("h2",null,"Unexpected Application Error!"),W.createElement("h3",{style:{fontStyle:"italic"}},t),a?W.createElement("pre",{style:r},a):null,i)}var cT=W.createElement(uT,null),E0=class extends W.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.onError?this.props.onError(e,t):console.error("React Router caught the following error during render",e)}render(){let e=this.state.error;if(this.context&&typeof e=="object"&&e&&"digest"in e&&typeof e.digest=="string"){let a=oT(e.digest);a&&(e=a)}let t=e!==void 0?W.createElement(ra.Provider,{value:this.props.routeContext},W.createElement(Cp.Provider,{value:e,children:this.props.component})):this.props.children;return this.context?W.createElement(dT,{error:e},t):t}};E0.contextType=b0;var bp=new WeakMap;function dT({children:e,error:t}){let{basename:a}=W.useContext(_t);if(typeof t=="object"&&t&&"digest"in t&&typeof t.digest=="string"){let n=iT(t.digest);if(n){let r=bp.get(t);if(r)throw r;let s=g0(n.location,a);if(v0&&!bp.get(t))if(s.isExternal||n.reloadDocument)window.location.href=s.absoluteURL||s.to;else{let i=Promise.resolve().then(()=>window.__reactRouterDataRouter.navigate(s.to,{replace:n.replace}));throw bp.set(t,i),i}return W.createElement("meta",{httpEquiv:"refresh",content:`0;url=${s.absoluteURL||s.to}`})}}return e}function mT({routeContext:e,match:t,children:a}){let n=W.useContext(Br);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),W.createElement(ra.Provider,{value:e},a)}function fT(e,t=[],a){let n=a?.state;if(e==null){if(!n)return null;if(n.errors)e=n.matches;else if(t.length===0&&!n.initialized&&n.matches.length>0)e=n.matches;else return null}let r=e,s=n?.errors;if(s!=null){let d=r.findIndex(m=>m.route.id&&s?.[m.route.id]!==void 0);Te(d>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(s).join(",")}`),r=r.slice(0,Math.min(r.length,d+1))}let i=!1,o=-1;if(a&&n){i=n.renderFallback;for(let d=0;d<r.length;d++){let m=r[d];if((m.route.HydrateFallback||m.route.hydrateFallbackElement)&&(o=d),m.route.id){let{loaderData:f,errors:h}=n,x=m.route.loader&&!f.hasOwnProperty(m.route.id)&&(!h||h[m.route.id]===void 0);if(m.route.lazy||x){a.isStatic&&(i=!0),o>=0?r=r.slice(0,o+1):r=[r[0]];break}}}}let l=a?.onError,c=n&&l?(d,m)=>{l(d,{location:n.location,params:n.matches?.[0]?.params??{},pattern:J3(n.matches),errorInfo:m})}:void 0;return r.reduceRight((d,m,f)=>{let h,x=!1,y=null,$=null;n&&(h=s&&m.route.id?s[m.route.id]:void 0,y=m.route.errorElement||cT,i&&(o<0&&f===0?(D0("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),x=!0,$=null):o===f&&(x=!0,$=m.route.hydrateFallbackElement||null)));let g=t.concat(r.slice(0,f+1)),v=()=>{let b;return h?b=y:x?b=$:m.route.Component?b=W.createElement(m.route.Component,null):m.route.element?b=m.route.element:b=d,W.createElement(mT,{match:m,routeContext:{outlet:d,matches:g,isDataRoute:n!=null},children:b})};return n&&(m.route.ErrorBoundary||m.route.errorElement||f===0)?W.createElement(E0,{location:n.location,revalidation:n.revalidation,component:y,error:h,children:v(),routeContext:{outlet:null,matches:g,isDataRoute:!0},onError:c}):v()},null)}function Ep(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function pT(e){let t=W.useContext(Br);return Te(t,Ep(e)),t}function Tp(e){let t=W.useContext(ti);return Te(t,Ep(e)),t}function hT(e){let t=W.useContext(ra);return Te(t,Ep(e)),t}function Ap(e){let t=hT(e),a=t.matches[t.matches.length-1];return Te(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function vT(){return Ap("useRouteId")}function T0(){let e=Tp("useNavigation");return W.useMemo(()=>{let{matches:t,historyAction:a,...n}=e.navigation;return n},[e.navigation])}function Dp(){let{matches:e,loaderData:t}=Tp("useMatches");return W.useMemo(()=>e.map(a=>A3(a,t)),[e,t])}function A0(){let e=W.useContext(Cp),t=Tp("useRouteError"),a=Ap("useRouteError");return e!==void 0?e:t.errors?.[a]}function gT(){let{router:e}=pT("useNavigate"),t=Ap("useNavigate"),a=W.useRef(!1);return N0(()=>{a.current=!0}),W.useCallback(async(r,s={})=>{na(a.current,S0),a.current&&(typeof r=="number"?await e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var i0={};function D0(e,t,a){!t&&!i0[e]&&(i0[e]=!0,na(!1,a))}var yT="useOptimistic",CP=ke[yT];var EP=ke.memo(bT);function bT({routes:e,manifest:t,future:a,state:n,isStatic:r,onError:s}){return C0(e,void 0,{manifest:t,state:n,isStatic:r,onError:s,future:a})}function ot({to:e,replace:t,state:a,relative:n}){Te(qr(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=ke.useContext(_t);na(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=ke.useContext(ra),{pathname:i}=Ae(),o=ve(),l=yc(e,Rp(s),i,n==="path"),c=JSON.stringify(l);return ke.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function Mp(e){return R0(e.context)}function xe(e){Te(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function Op({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1,useTransitions:i}){Te(!qr(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let o=e.replace(/^\/*/,"/"),l=ke.useMemo(()=>({basename:o,navigator:r,static:s,useTransitions:i,future:{}}),[o,r,s,i]);typeof a=="string"&&(a=zr(a));let{pathname:c="/",search:d="",hash:m="",state:f=null,key:h="default",mask:x}=a,y=ke.useMemo(()=>{let $=Wa(c,o);return $==null?null:{location:{pathname:$,search:d,hash:m,state:f,key:h,mask:x},navigationType:n}},[o,c,d,m,f,h,n,x]);return na(y!=null,`<Router basename="${o}"> is not able to match the URL "${c}${d}${m}" because it does not start with the basename, so the <Router> won't render anything.`),y==null?null:ke.createElement(_t.Provider,{value:l},ke.createElement(ai.Provider,{children:t,value:y}))}function Lp({children:e,location:t}){return k0(gc(e),t)}function gc(e,t=[]){let a=[];return ke.Children.forEach(e,(n,r)=>{if(!ke.isValidElement(n))return;let s=[...t,r];if(n.type===ke.Fragment){a.push.apply(a,gc(n.props.children,s));return}Te(n.type===xe,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),Te(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,middleware:n.props.middleware,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=gc(n.props.children,s)),a.push(i)}),a}var pc="get",hc="application/x-www-form-urlencoded";function bc(e){return typeof HTMLElement<"u"&&e instanceof HTMLElement}function xT(e){return bc(e)&&e.tagName.toLowerCase()==="button"}function $T(e){return bc(e)&&e.tagName.toLowerCase()==="form"}function wT(e){return bc(e)&&e.tagName.toLowerCase()==="input"}function ST(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function NT(e,t){return e.button===0&&(!t||t==="_self")&&!ST(e)}var mc=null;function _T(){if(mc===null)try{new FormData(document.createElement("form"),0),mc=!1}catch{mc=!0}return mc}var RT=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function xp(e){return e!=null&&!RT.has(e)?(na(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${hc}"`),null):e}function kT(e,t){let a,n,r,s,i;if($T(e)){let o=e.getAttribute("action");n=o?Wa(o,t):null,a=e.getAttribute("method")||pc,r=xp(e.getAttribute("enctype"))||hc,s=new FormData(e)}else if(xT(e)||wT(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let l=e.getAttribute("formaction")||o.getAttribute("action");if(n=l?Wa(l,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||pc,r=xp(e.getAttribute("formenctype"))||xp(o.getAttribute("enctype"))||hc,s=new FormData(o,e),!_T()){let{name:c,type:d,value:m}=e;if(d==="image"){let f=c?`${c}.`:"";s.append(`${f}x`,"0"),s.append(`${f}y`,"0")}else c&&s.append(c,m)}}else{if(bc(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=pc,n=null,r=hc,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var TP=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");var CT={"&":"\\u0026",">":"\\u003e","<":"\\u003c","\u2028":"\\u2028","\u2029":"\\u2029"},ET=/[&><\u2028\u2029]/g;function o0(e){return e.replace(ET,t=>CT[t])}function Up(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var TT=Symbol("SingleFetchRedirect");function M0(e,t,a,n){let r=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return a?r.pathname.endsWith("/")?r.pathname=`${r.pathname}_.${n}`:r.pathname=`${r.pathname}.${n}`:r.pathname==="/"?r.pathname=`_root.${n}`:t&&Wa(r.pathname,t)==="/"?r.pathname=`${vc(t)}/_root.${n}`:r.pathname=`${vc(r.pathname)}.${n}`,r}async function AT(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function DT(e){return e!=null&&typeof e.page=="string"}function MT(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function OT(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await AT(s,a);return i.links?i.links():[]}return[]}));return jT(n.flat(1).filter(MT).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function l0(e,t,a,n,r,s){let i=(l,c)=>a[c]?l.route.id!==a[c].route.id:!0,o=(l,c)=>a[c].pathname!==l.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==l.params["*"];return s==="assets"?t.filter((l,c)=>i(l,c)||o(l,c)):s==="data"?t.filter((l,c)=>{let d=n.routes[l.route.id];if(!d||!d.hasLoader)return!1;if(i(l,c)||o(l,c))return!0;if(l.route.shouldRevalidate){let m=l.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:l.params,defaultShouldRevalidate:!0});if(typeof m=="boolean")return m}return!0}):[]}function LT(e,t,{includeHydrateFallback:a}={}){return PT(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function PT(e){return[...new Set(e)]}function UT(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function jT(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!DT(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(UT(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function jp(){let e=fe.useContext(Br);return Up(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function qT(){let e=fe.useContext(ti);return Up(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var Xo=fe.createContext(void 0);Xo.displayName="FrameworkContext";function Fp(){let e=fe.useContext(Xo);return Up(e,"You must render this element inside a <HydratedRouter> element"),e}function IT(e,t){let a=fe.useContext(Xo),[n,r]=fe.useState(!1),[s,i]=fe.useState(!1),{onFocus:o,onBlur:l,onMouseEnter:c,onMouseLeave:d,onTouchStart:m}=t,f=fe.useRef(null);fe.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let y=g=>{g.forEach(v=>{i(v.isIntersecting)})},$=new IntersectionObserver(y,{threshold:.5});return f.current&&$.observe(f.current),()=>{$.disconnect()}}},[e]),fe.useEffect(()=>{if(n){let y=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(y)}}},[n]);let h=()=>{r(!0)},x=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,f,{}]:[s,f,{onFocus:Yo(o,h),onBlur:Yo(l,x),onMouseEnter:Yo(c,h),onMouseLeave:Yo(d,x),onTouchStart:Yo(m,h)}]:[!1,f,{}]}function Yo(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function L0({page:e,...t}){let a=aT(),{router:n}=jp(),r=fe.useMemo(()=>_p(n.routes,e,n.basename),[n.routes,e,n.basename]);return r?a?fe.createElement(KT,{page:e,matches:r,...t}):fe.createElement(QT,{page:e,matches:r,...t}):null}function HT(e){let{manifest:t,routeModules:a}=Fp(),[n,r]=fe.useState([]);return fe.useEffect(()=>{let s=!1;return OT(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function KT({page:e,matches:t,...a}){let n=Ae(),{future:r}=Fp(),{basename:s}=jp(),i=fe.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let o=M0(e,s,r.unstable_trailingSlashAwareDataRequests,"rsc"),l=!1,c=[];for(let d of t)typeof d.route.shouldRevalidate=="function"?l=!0:c.push(d.route.id);return l&&c.length>0&&o.searchParams.set("_routes",c.join(",")),[o.pathname+o.search]},[s,r.unstable_trailingSlashAwareDataRequests,e,n,t]);return fe.createElement(fe.Fragment,null,i.map(o=>fe.createElement("link",{key:o,rel:"prefetch",as:"fetch",href:o,...a})))}function QT({page:e,matches:t,...a}){let n=Ae(),{future:r,manifest:s,routeModules:i}=Fp(),{basename:o}=jp(),{loaderData:l,matches:c}=qT(),d=fe.useMemo(()=>l0(e,t,c,s,n,"data"),[e,t,c,s,n]),m=fe.useMemo(()=>l0(e,t,c,s,n,"assets"),[e,t,c,s,n]),f=fe.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let y=new Set,$=!1;if(t.forEach(v=>{let b=s.routes[v.route.id];!b||!b.hasLoader||(!d.some(w=>w.route.id===v.route.id)&&v.route.id in l&&i[v.route.id]?.shouldRevalidate||b.hasClientLoader?$=!0:y.add(v.route.id))}),y.size===0)return[];let g=M0(e,o,r.unstable_trailingSlashAwareDataRequests,"data");return $&&y.size>0&&g.searchParams.set("_routes",t.filter(v=>y.has(v.route.id)).map(v=>v.route.id).join(",")),[g.pathname+g.search]},[o,r.unstable_trailingSlashAwareDataRequests,l,n,s,d,t,e,i]),h=fe.useMemo(()=>LT(m,s),[m,s]),x=HT(m);return fe.createElement(fe.Fragment,null,f.map(y=>fe.createElement("link",{key:y,rel:"prefetch",as:"fetch",href:y,...a})),h.map(y=>fe.createElement("link",{key:y,rel:"modulepreload",href:y,...a})),x.map(({key:y,link:$})=>fe.createElement("link",{key:y,nonce:a.nonce,...$,crossOrigin:$.crossOrigin??a.crossOrigin})))}function VT(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var GT=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{GT&&(window.__reactRouterVersion="7.15.1")}catch{}function zp({basename:e,children:t,useTransitions:a,window:n}){let r=te.useRef();r.current==null&&(r.current=u0({window:n,v5Compat:!0}));let s=r.current,[i,o]=te.useState({action:s.action,location:s.location}),l=te.useCallback(c=>{a===!1?o(c):te.startTransition(()=>o(c))},[a]);return te.useLayoutEffect(()=>s.listen(l),[s,l]),te.createElement(Op,{basename:e,children:t,location:i.location,navigationType:i.action,navigator:s,useTransitions:a})}function P0({basename:e,children:t,history:a,useTransitions:n}){let[r,s]=te.useState({action:a.action,location:a.location}),i=te.useCallback(o=>{n===!1?s(o):te.startTransition(()=>s(o))},[n]);return te.useLayoutEffect(()=>a.listen(i),[a,i]),te.createElement(Op,{basename:e,children:t,location:r.location,navigationType:r.action,navigator:a,useTransitions:n})}P0.displayName="unstable_HistoryRouter";var U0=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i,Rn=te.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,mask:o,state:l,target:c,to:d,preventScrollReset:m,viewTransition:f,defaultShouldRevalidate:h,...x},y){let{basename:$,navigator:g,useTransitions:v}=te.useContext(_t),b=typeof d=="string"&&U0.test(d),w=g0(d,$);d=w.to;let S=w0(d,{relative:r}),C=Ae(),R=null;if(o){let G=yc(o,[],C.mask?C.mask.pathname:"/",!0);$!=="/"&&(G.pathname=G.pathname==="/"?$:Ta([$,G.pathname])),R=g.createHref(G)}let[_,M,L]=IT(n,x),U=B0(d,{replace:i,mask:o,state:l,target:c,preventScrollReset:m,relative:r,viewTransition:f,defaultShouldRevalidate:h,useTransitions:v});function F(G){t&&t(G),G.defaultPrevented||U(G)}let z=!(w.isExternal||s),P=te.createElement("a",{...x,...L,href:(z?R:void 0)||w.absoluteURL||S,onClick:z?F:t,ref:VT(y,M),target:c,"data-discover":!b&&a==="render"?"true":void 0});return _&&!b?te.createElement(te.Fragment,null,P,te.createElement(L0,{page:S})):P});Rn.displayName="Link";var Za=te.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:l,...c},d){let m=ni(i,{relative:c.relative}),f=Ae(),h=te.useContext(ti),{navigator:x,basename:y}=te.useContext(_t),$=h!=null&&K0(m)&&o===!0,g=x.encodeLocation?x.encodeLocation(m).pathname:m.pathname,v=f.pathname,b=h&&h.navigation&&h.navigation.location?h.navigation.location.pathname:null;a||(v=v.toLowerCase(),b=b?b.toLowerCase():null,g=g.toLowerCase()),b&&y&&(b=Wa(b,y)||b);let w=g!=="/"&&g.endsWith("/")?g.length-1:g.length,S=v===g||!r&&v.startsWith(g)&&v.charAt(w)==="/",C=b!=null&&(b===g||!r&&b.startsWith(g)&&b.charAt(g.length)==="/"),R={isActive:S,isPending:C,isTransitioning:$},_=S?t:void 0,M;typeof n=="function"?M=n(R):M=[n,S?"active":null,C?"pending":null,$?"transitioning":null].filter(Boolean).join(" ");let L=typeof s=="function"?s(R):s;return te.createElement(Rn,{...c,"aria-current":_,className:M,ref:d,style:L,to:i,viewTransition:o},typeof l=="function"?l(R):l)});Za.displayName="NavLink";var j0=te.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=pc,action:o,onSubmit:l,relative:c,preventScrollReset:d,viewTransition:m,defaultShouldRevalidate:f,...h},x)=>{let{useTransitions:y}=te.useContext(_t),$=q0(),g=I0(o,{relative:c}),v=i.toLowerCase()==="get"?"get":"post",b=typeof o=="string"&&U0.test(o);return te.createElement("form",{ref:x,method:v,action:g,onSubmit:n?l:S=>{if(l&&l(S),S.defaultPrevented)return;S.preventDefault();let C=S.nativeEvent.submitter,R=C?.getAttribute("formmethod")||i,_=()=>$(C||S.currentTarget,{fetcherKey:t,method:R,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:m,defaultShouldRevalidate:f});y&&a!==!1?te.startTransition(()=>_()):_()},...h,"data-discover":!b&&e==="render"?"true":void 0})});j0.displayName="Form";function F0({getKey:e,storageKey:t,...a}){let n=te.useContext(Xo),{basename:r}=te.useContext(_t),s=Ae(),i=Dp();H0({getKey:e,storageKey:t});let o=te.useMemo(()=>{if(!n||!e)return null;let c=Np(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let l=((c,d)=>{if(!window.history.state||!window.history.state.key){let m=Math.random().toString(32).slice(2);window.history.replaceState({key:m},"")}try{let f=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof f=="number"&&window.scrollTo(0,f)}catch(m){console.error(m),sessionStorage.removeItem(c)}}).toString();return te.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${l})(${o0(JSON.stringify(t||Sp))}, ${o0(JSON.stringify(o))})`}})}F0.displayName="ScrollRestoration";function z0(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function Bp(e){let t=te.useContext(Br);return Te(t,z0(e)),t}function YT(e){let t=te.useContext(ti);return Te(t,z0(e)),t}function B0(e,{target:t,replace:a,mask:n,state:r,preventScrollReset:s,relative:i,viewTransition:o,defaultShouldRevalidate:l,useTransitions:c}={}){let d=ve(),m=Ae(),f=ni(e,{relative:i});return te.useCallback(h=>{if(NT(h,t)){h.preventDefault();let x=a!==void 0?a:ei(m)===ei(f),y=()=>d(e,{replace:x,mask:n,state:r,preventScrollReset:s,relative:i,viewTransition:o,defaultShouldRevalidate:l});c?te.startTransition(()=>y()):y()}},[m,d,f,a,n,r,t,e,s,i,o,l,c])}var JT=0,XT=()=>`__${String(++JT)}__`;function q0(){let{router:e}=Bp("useSubmit"),{basename:t}=te.useContext(_t),a=vT(),n=e.fetch,r=e.navigate;return te.useCallback(async(s,i={})=>{let{action:o,method:l,encType:c,formData:d,body:m}=kT(s,t);if(i.navigate===!1){let f=i.fetcherKey||XT();await n(f,a,i.action||o,{defaultShouldRevalidate:i.defaultShouldRevalidate,preventScrollReset:i.preventScrollReset,formData:d,body:m,formMethod:i.method||l,formEncType:i.encType||c,flushSync:i.flushSync})}else await r(i.action||o,{defaultShouldRevalidate:i.defaultShouldRevalidate,preventScrollReset:i.preventScrollReset,formData:d,body:m,formMethod:i.method||l,formEncType:i.encType||c,replace:i.replace,state:i.state,fromRouteId:a,flushSync:i.flushSync,viewTransition:i.viewTransition})},[n,r,t,a])}function I0(e,{relative:t}={}){let{basename:a}=te.useContext(_t),n=te.useContext(ra);Te(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...ni(e||".",{relative:t})},i=Ae();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),l=o.getAll("index");if(l.some(d=>d==="")){o.delete("index"),l.filter(m=>m).forEach(m=>o.append("index",m));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:Ta([a,s.pathname])),ei(s)}var Sp="react-router-scroll-positions",fc={};function Np(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:Wa(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function H0({getKey:e,storageKey:t}={}){let{router:a}=Bp("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=YT("useScrollRestoration"),{basename:s}=te.useContext(_t),i=Ae(),o=Dp(),l=T0();te.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),WT(te.useCallback(()=>{if(l.state==="idle"){let c=Np(i,o,s,e);fc[c]=window.scrollY}try{sessionStorage.setItem(t||Sp,JSON.stringify(fc))}catch(c){na(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[l.state,e,s,i,o,t])),typeof document<"u"&&(te.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||Sp);c&&(fc=JSON.parse(c))}catch{}},[t]),te.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(fc,()=>window.scrollY,e?(d,m)=>Np(d,m,s,e):void 0);return()=>c&&c()},[a,s,e]),te.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{na(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function WT(e,t){let{capture:a}=t||{};te.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function K0(e,{relative:t}={}){let a=te.useContext(kp);Te(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=Bp("useViewTransitionState"),r=ni(e,{relative:t});if(!a.isTransitioning)return!1;let s=Wa(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=Wa(a.nextLocation.pathname,n)||a.nextLocation.pathname;return Jo(r.pathname,i)!=null||Jo(r.pathname,s)!=null}var Dt=new jd({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var qp="ironclaw_token",Ke="/api/webchat/v2",Ir=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function Sa(){return sessionStorage.getItem(qp)||""}function ri(e){e?sessionStorage.setItem(qp,e):sessionStorage.removeItem(qp)}function xc(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function G0(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function V0(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function Y0({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=V0(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=V0(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function V(e,t={}){let a=Sa(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await G0(r);throw new Ir(Y0({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function $c(){return V(`${Ke}/session`)}function wc({clientActionId:e,requestedThreadId:t,projectId:a}={}){let n={client_action_id:e||xc()};return t&&(n.requested_thread_id=t),a&&(n.project_id=a),V(`${Ke}/threads`,{method:"POST",body:JSON.stringify(n)})}function J0({limit:e,cursor:t,projectId:a}={}){let n=new URL(`${Ke}/threads`,window.location.origin);return e!=null&&n.searchParams.set("limit",String(e)),t&&n.searchParams.set("cursor",t),a&&n.searchParams.set("project_id",a),V(n.pathname+n.search)}function X0({threadId:e}={}){return e?V(`${Ke}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function Ip(e){return`${Ke}/threads/${encodeURIComponent(e)}/files`}function W0({threadId:e,path:t}={}){if(!e)return Promise.reject(new Error("threadId is required"));let a=new URL(Ip(e),window.location.origin);return t&&a.searchParams.set("path",t),V(a.pathname+a.search)}function Z0({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${Ip(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),V(a.pathname+a.search)}function Sc({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${Ip(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function e$({limit:e,runLimit:t,includeCompleted:a}={}){let n=new URLSearchParams;e!=null&&n.set("limit",String(e)),t!=null&&n.set("run_limit",String(t)),a===!0&&n.set("include_completed","true");let r=n.toString();return V(`${Ke}/automations${r?`?${r}`:""}`)}function t$({automationId:e}={}){return e?V(`${Ke}/automations/${encodeURIComponent(e)}/pause`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function a$({automationId:e}={}){return e?V(`${Ke}/automations/${encodeURIComponent(e)}/resume`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function n$({automationId:e}={}){return e?V(`${Ke}/automations/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("automationId is required"))}var r$=`${Ke}/projects`;function ZT(e){return`${r$}/${encodeURIComponent(e)}`}function s$({limit:e}={}){let t=new URL(r$,window.location.origin);return e!=null&&t.searchParams.set("limit",String(e)),V(t.pathname+t.search)}function i$({projectId:e}={}){return e?V(ZT(e)):Promise.reject(new Error("projectId is required"))}function o$(){return V(`${Ke}/outbound/preferences`)}function l$(){return V(`${Ke}/outbound/targets`)}function u$({finalReplyTargetId:e}={}){return V(`${Ke}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function Hp({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:l,source:c,tail:d,follow:m}={}){let f=new URL(`${Ke}/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),l&&f.searchParams.set("tool_name",l),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),V(f.pathname+f.search)}function c$({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:l,source:c,tail:d,follow:m}={}){let f=new URL(`${Ke}/operator/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),l&&f.searchParams.set("tool_name",l),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),V(f.pathname+f.search)}function d$({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||xc(),content:t};return a.length>0&&(r.attachments=a),V(`${Ke}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function m$({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${Ke}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),V(n.pathname+n.search)}function f$({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${Ke}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function Aa(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new Ir("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=Sa(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await G0(r);throw new Ir(Y0({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function Kp(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function Nc(e){return Kp(await Aa(e))}function p$({threadId:e,afterCursor:t}={}){let a=new URL(`${Ke}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=Sa();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function h$({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||xc()};return a&&(r.reason=a),V(`${Ke}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function Qp({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let l={client_action_id:i||xc(),resolution:n};return r!=null&&(l.always=r),s&&(l.credential_ref=s),V(`${Ke}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(l)})}function v$({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return V("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function g$(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),V(`${Ke}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function si(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function y$(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function b$(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new Ir("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new Ir("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function x$(){let e=Sa();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var _c="anon",$$=_c;function w$(e){$$=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:_c}function ft(){return $$}var S$="ironclaw:v2-thread-pins:",Vp=new Set,kn=new Set,Gp=null;function Yp(){return`${S$}${ft()}`}function eA(){try{let e=window.localStorage.getItem(Yp());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function tA(){try{kn.size===0?window.localStorage.removeItem(Yp()):window.localStorage.setItem(Yp(),JSON.stringify([...kn]))}catch{}}function N$(){let e=ft();if(e!==Gp){kn.clear();for(let t of eA())kn.add(t);Gp=e}}function _$(){return new Set(kn)}function R$(){let e=_$();for(let t of Vp)try{t(e)}catch{}}function k$(e){e&&(N$(),kn.has(e)?kn.delete(e):kn.add(e),tA(),R$())}function C$(){return N$(),_$()}function E$(e){return Vp.add(e),()=>{Vp.delete(e)}}function T$(){kn.clear(),Gp=ft();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(S$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}R$()}var aA=0,Hr={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function Jp(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function A$(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":nA(t)?"text":"download"}function nA(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function Wo(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function rA(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function sA(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function iA(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function D$(e,{limits:t,existing:a=[],t:n}){let r=t||Hr,s=[],i=[],o=a.length,l=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!rA(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:Wo(r.maxFileBytes)}));continue}if(l+c.size>r.maxTotalBytes){let y=n("chat.attachmentTotalTooLarge",{max:Wo(r.maxTotalBytes)});i.includes(y)||i.push(y);continue}let d;try{d=await sA(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:m,base64:f}=iA(d,c.type),h=m||"application/octet-stream",x=Jp(h);s.push({id:`staged-${aA++}`,filename:c.name||"attachment",mimeType:h,kind:x,sizeBytes:c.size,sizeLabel:Wo(c.size),dataBase64:f,previewUrl:x==="image"?d:null}),o+=1,l+=c.size}return{staged:s,errors:i}}function M$(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function O$(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}var Rc="__ironclaw_attachments_only_v1__";function oA(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||Jp(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?f$({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?Wo(n.size_bytes):"",preview_url:null,fetch_url:s}})}function P$(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let m=dA(s);if(!m)continue;let f=`tool-${m.invocationId}`;if(n.has(f))continue;n.add(f),r.push({id:f,role:"tool_activity",...m,timestamp:L$(s)||m.updatedAt||null,sequence:s.sequence,activityOrder:m.activityOrder,activityOrderSource:m.activityOrderSource,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=cA(s),l=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy"),c=oA(s,a),d=o==="user"&&c?.length>0&&s.content===Rc?"":s.content||"";r.push({id:i,role:o,content:d,attachments:c,timestamp:L$(s),kind:s.kind,status:l?"error":s.status,...l&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:uA(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=lA(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function lA(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function uA(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function cA(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function L$(e){return e.received_at||e.created_at||null}function dA(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:Xp(t)}var mA="gate_declined";function Xp(e){let t=e.status==="failed"||e.status==="killed",a=e.error_kind||null,n=F$(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:el(e.title||e.capability_id)||"tool",toolStatus:j$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(U$(a)||e.output_summary||e.output_preview||e.result_ref)||null,toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null,activityOrder:n,activityOrderSource:Number.isFinite(n)?"projection":null}}function Wp(e){let t=F$(e.activity_order),a=e.error_kind||null;return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:el(e.capability_id)||"tool",toolStatus:j$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:null,toolError:U$(a),toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null,activityOrder:t,activityOrderSource:Number.isFinite(t)?"projection":null}}function U$(e){return e||null}function Zo(e){return e==="success"||e==="error"||e==="declined"}function el(e){let t=typeof e=="string"?e.trim():"";if(!t)return"";let a=t.split(".");return a[a.length-1]||t}function j$(e,t=null){if(t===mA)return"declined";switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}function F$(e){let t=Number(e);return Number.isFinite(t)?t:null}var fA=50,Da=new Map,pA=30;function tl(e,t){for(Da.delete(e),Da.set(e,t);Da.size>pA;){let a=Da.keys().next().value;Da.delete(a)}}function ii(e){return`${ft()}:${e}`}function B$(){Da.clear()}function q$(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?Da.get(ii(e)):null,[s,i]=p.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),[o,l]=p.default.useState(e);if(o!==e){let h=e?Da.get(ii(e)):null;l(e),i({messages:h?.messages||[],nextCursor:h?.nextCursor||null,isLoading:!!e&&!h,loadError:null})}let c=p.default.useRef(new Set),d=p.default.useRef(e);d.current=e;let m=p.default.useCallback(async(h,x={})=>{let{preserveClientOnly:y=!1,finalReplyTimestampByRun:$=null}=x;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(c.current.has(e))return;c.current.add(e);let g=ft(),v=ii(e);i(b=>({...b,isLoading:!0}));try{let b=await m$({threadId:e,limit:fA,cursor:h});if(ft()!==g)return;let w=h?[]:a?.()||[],S=P$(b.messages||[],w,e),C=b.next_cursor||null;if(h||n?.([]),!h){let R=Da.get(v)?.messages||[],_=z$(S,R,{preserveClientOnly:y,finalReplyTimestampByRun:$});tl(v,{messages:_,nextCursor:C})}i(R=>{if(d.current!==e)return R;let _;return h?_=hA(S,R.messages):_=z$(S,R.messages,{preserveClientOnly:y,finalReplyTimestampByRun:$}),tl(v,{messages:_,nextCursor:C}),{messages:_,nextCursor:C,isLoading:!1,loadError:null}})}catch(b){if(console.error("Failed to load timeline:",b),ft()!==g)return;i(w=>d.current===e?{...w,isLoading:!1,loadError:"Failed to load conversation history."}:w)}finally{c.current.delete(e)}},[e,a,n]);p.default.useEffect(()=>{let h=e?Da.get(ii(e)):null;i({messages:h?.messages||[],nextCursor:h?.nextCursor||null,isLoading:!!e&&!h,loadError:null}),e&&m()},[e,m]);let f=p.default.useCallback((h,x)=>{if(!h)return;let y=ii(h),$=b=>typeof x=="function"?x(b||[]):x;if(d.current===h){i(b=>{let w=$(b.messages||[]);return tl(y,{messages:w,nextCursor:b.nextCursor||null}),{...b,messages:w}});return}let g=Da.get(y)||{messages:[],nextCursor:null},v=$(g.messages||[]);tl(y,{messages:v,nextCursor:g.nextCursor||null})},[]);return{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:m,seedThreadMessages:f,setMessages:h=>i(x=>{let y=typeof h=="function"?h(x.messages):h;return e&&tl(ii(e),{messages:y,nextCursor:x.nextCursor}),{...x,messages:y}})}}function hA(e,t){let a=new Set(t.map(n=>n?.id).filter(Boolean));return[...e.filter(n=>!a.has(n?.id)),...t]}function z$(e,t,a={}){let{preserveClientOnly:n=!1,finalReplyTimestampByRun:r=null}=a,s=gA(e,t,{finalReplyTimestampByRun:r}),i=new Set(s.map(l=>l?.id).filter(Boolean)),o=t.filter(l=>!l||typeof l.id!="string"||i.has(l.id)?!1:I$(l)?!0:typeof l.timelineMessageId=="string"&&i.has(`msg-${l.timelineMessageId}`)?!1:vA(l)?!0:n&&l.id.startsWith("err-"));return o.length>0?yA(s,o,t):s}function vA(e){return e?.isOptimistic===!0&&typeof e.id=="string"&&e.id.startsWith("pending-")&&(e.role==="user"||e.role==="assistant")}function gA(e,t,a={}){let{finalReplyTimestampByRun:n=null}=a,r=new Map,s=new Map;for(let i of t||[])!i||!i.timestamp||(typeof i.id=="string"&&r.set(i.id,i),typeof i.timelineMessageId=="string"&&r.set(`msg-${i.timelineMessageId}`,i),Zp(i)&&typeof i.turnRunId=="string"&&s.set(i.turnRunId,i));return r.size===0&&s.size===0&&!n?e:e.map(i=>{if(!i||i.timestamp||typeof i.id!="string")return i;let o=typeof i.turnRunId=="string"?i.turnRunId:null,l=r.get(i.id)||(Zp(i)&&o?s.get(o):null),c=Zp(i)&&o?n?.[o]:null,d=l?.timestamp||c;return d?{...i,timestamp:d}:i})}function Zp(e){return e?.role==="assistant"&&e?.isFinalReply===!0}function I$(e){return e?.role==="tool_activity"||e?.role==="thinking"}function yA(e,t,a){let n=new Map;for(let[l,c]of e.entries())typeof c?.id=="string"&&n.set(c.id,l);let r=a.map(l=>bA(l,n)),s=new Map,i=[];for(let l of t){if(!I$(l)){i.push(l);continue}let c=a.indexOf(l),d=null;for(let m=c-1;m>=0;m-=1)if(r[m]!==null){d=r[m];break}if(d!==null){let m=s.get(d)||[];m.push(l),s.set(d,m)}else i.push(l)}let o=[];for(let[l,c]of e.entries()){o.push(c);let d=s.get(l);d&&o.push(...d)}return o.push(...i),o}function bA(e,t){if(!e)return null;if(typeof e.id=="string"&&t.has(e.id))return t.get(e.id);if(typeof e.timelineMessageId=="string"){let a=`msg-${e.timelineMessageId}`;if(t.has(a))return t.get(a)}return null}var nl="__new__",H$="ironclaw:v2-draft:";function oi(e){return`${H$}${ft()}:${e||nl}`}function eh(e){try{return window.localStorage.getItem(oi(e))||""}catch{return""}}function th(e,t){try{t?window.localStorage.setItem(oi(e),t):window.localStorage.removeItem(oi(e))}catch{}}function K$(e){th(e,"")}var al=new Map;function ah(e){return al.get(oi(e))||[]}function kc(e,t){let a=oi(e);t&&t.length>0?al.set(a,t):al.delete(a)}function Q$(e){al.delete(oi(e))}function V$(){al.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(H$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function xA(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function $A(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function wA(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=xA(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?$A(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),Sa()?"":(ri(n),n)}function SA(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var NA={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function _A(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),NA[t]||"Could not complete sign-in. Please try again."):""}function G$(){let[e,t]=p.default.useState(()=>wA()||Sa()),[a,n]=p.default.useState(()=>_A()),[r]=p.default.useState(()=>SA()),[s,i]=p.default.useState(null),[o,l]=p.default.useState(()=>!!(r&&!Sa())),[c,d]=p.default.useState(()=>!!Sa());p.default.useEffect(()=>{if(!r||Sa()){l(!1);return}let x=!1;return b$(r).then(y=>{x||(ri(y),d(!0),t(y),i(null),n(""),l(!1),Dt.clear())}).catch(()=>{x||(n("Could not complete sign-in. Please try again."),l(!1))}),()=>{x=!0}},[r]),p.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let x=!1;return d(!0),$c().then(y=>{x||(i(y),d(!1))}).catch(y=>{x||(i(null),d(!1),(y?.status===401||y?.status===403)&&(ri(""),t(""),n("Your session expired. Please sign in again."),Dt.clear()))}),()=>{x=!0}},[e,o]),w$(s);let m=p.default.useRef(null);p.default.useEffect(()=>{let x=ft();m.current&&m.current!==_c&&m.current!==x&&(B$(),V$(),T$()),m.current=x},[s]);let f=p.default.useCallback(x=>{ri(x),d(!!x),t(x),i(null),n(""),Dt.clear()},[]),h=p.default.useCallback(()=>{x$().catch(()=>{}),ri(""),d(!1),t(""),i(null),n(""),Dt.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,rebornProjectsEnabled:!!s?.features?.reborn_projects,signIn:f,signOut:h}}var Kr="/chat",rl=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace"},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"logs",path:"/logs",labelKey:"nav.logs",hidden:!0},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var RA=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"tools",labelKey:"settings.tools",icon:"tool"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],kA=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],CA=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],Cc={settings:RA,extensions:kA,admin:CA};var Y$="ironclaw:v2-theme";function EA(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(Y$);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function Ec(){let[e,t]=p.default.useState(EA);p.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(Y$,e)}catch{}},[e]);let a=p.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function J$(e){return K({enabled:!!e,queryKey:["gateway-status",e],queryFn:si,refetchInterval:3e4})}var TA="/api/webchat/v2/operator/config",Tc="/api/webchat/v2/settings/tools",li="agent.auto_approve_tools",X$="tool.",AA=new Set(["always_allow","ask_each_time","disabled"]),DA=new Set(["default","always_allow","ask_each_time","disabled"]);function W$(e){return e==="ask"?"ask_each_time":AA.has(e)?e:"ask_each_time"}function MA(e){return e==="ask"?"ask_each_time":DA.has(e)?e:"default"}function OA(e){return["default","global","override"].includes(e)?e:"default"}function Z$(e){if(!e?.key?.startsWith(X$))return null;let t=e.value||{};return{name:t.name||e.key.slice(X$.length),description:t.description||"",state:W$(t.state),default_state:W$(t.default_state),locked:!!(t.locked||e.mutable===!1),effective_source:OA(t.effective_source||e.source)}}function LA(e){let t={};for(let a of e.entries||[])a?.key===li&&(t[li]=!!a.value);return t}async function ew(){let e=await V(Tc);return{settings:LA(e),diagnostics:e.diagnostics||[],precedence:e.precedence||[]}}async function nh(e,t){if(e===li){let n=await V(Tc,{method:"POST",body:JSON.stringify({enabled:!!t})});return{success:!0,entry:n.entry,value:n.entry?.value}}let a=await V(`${TA}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({value:t})});return{success:!0,entry:a.entry,value:a.entry?.value}}async function tw(e){let t=e?.settings||{},a=[];return Object.prototype.hasOwnProperty.call(t,li)&&a.push(await nh(li,!!t[li])),{success:!0,imported:a.length,results:a}}function Ac(){return V("/api/webchat/v2/llm/providers")}function aw(e){return V("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function nw(e){return V(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function sl(e){return V("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function rw(e){return V("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function sw(e){return V("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function iw(e){return V("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function ow(e){return V("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function lw(){return V("/api/webchat/v2/llm/codex/login",{method:"POST"})}async function uw(){let e=await V(Tc);return{tools:(e.entries||[]).map(Z$).filter(Boolean),diagnostics:e.diagnostics||[]}}async function cw(e,t){let a=MA(t),n=await V(`${Tc}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({state:a})});return{success:!0,tool:Z$(n.entry),entry:n.entry}}function dw(){return V("/api/webchat/v2/extensions")}function mw(){return V("/api/webchat/v2/extensions/registry")}function fw(){return V("/api/webchat/v2/skills")}function pw(e){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function hw(e){return V("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function vw(e,t){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function gw(e){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function yw(e,t){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}/auto-activate`,{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:t})})}function bw(e){return V("/api/webchat/v2/skills/auto-activate-learned",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function xw(){return V("/api/webchat/v2/traces/credit")}function $w(e){return V(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function ww(){return Promise.resolve({users:[],todo:!0})}function Sw(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function Nw(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var rh="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",sh=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function il(e){return sh.find(t=>t.value===e)?.label||e}function ui(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function _w(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function Dc(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function Rw(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function Qr(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===rh||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?ui(e,t).trim().length>0:!0:!1}function PA(e,t,a){return e.id===a?"active":Qr(e,t)?"ready":"setup"}function kw(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=PA(r,t,a);n[s]&&n[s].push(r)}return n}function Mc(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===rh||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!ui(e,t).trim()?"base_url":"ok"}function ih(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===rh&&(i.api_key=void 0),i}function Cw(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function Ew(e){return/^[a-z0-9_-]+$/.test(e)}function Tw(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var UA=Object.freeze({});function ci({settings:e,gatewayStatus:t,enabled:a=!0}){let n=Z(),r=K({queryKey:["llm-providers"],queryFn:Ac,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=UA,l=(s.providers||[]).map(w=>({...w,name:w.description,has_api_key:w.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,m=d||"nearai",f=s.active?.model||t?.llm_model||"",h=l.filter(w=>w.builtin),x=l.filter(w=>!w.builtin),y=[...l].sort((w,S)=>w.id===d?-1:S.id===d?1:(w.name||w.id).localeCompare(S.name||S.id)),$=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=Y({mutationFn:async w=>{if(!Qr(w,o)){let C=Mc(w,o);throw new Error(C==="base_url"?"base_url":"api_key")}let S=Dc(w,o);if(!S)throw new Error("model");return await sl({provider_id:w.id,model:S}),w},onSuccess:$}),v=Y({mutationFn:async({provider:w,form:S,apiKey:C,editingProvider:R})=>{let _=!!w?.builtin,L={id:(_?w.id:S.id.trim()).trim(),name:_?w.name||w.id:S.name.trim(),adapter:_?w.adapter:S.adapter,base_url:S.baseUrl.trim()||w?.base_url||"",default_model:S.model.trim()||void 0};return C.trim()&&(L.api_key=C.trim()),(R||w)?.id===m&&L.default_model&&(L.set_active=!0,L.model=L.default_model),await aw(L),L},onSuccess:$}),b=Y({mutationFn:async w=>(await nw(w.id),w),onSuccess:$});return{providers:y,builtinProviders:h,customProviders:x,builtinOverrides:o,activeProviderId:d,selectedModel:f,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:w=>g.mutateAsync(w),saveCustomProvider:w=>v.mutateAsync(w),saveBuiltinProvider:w=>v.mutateAsync(w),deleteCustomProvider:w=>b.mutateAsync(w),testConnection:rw,listModels:sw,isBusy:g.isPending||v.isPending||b.isPending}}function Aw({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}var Dw="ironclaw:v2-sidebar-open";function Mw(){return typeof window>"u"?null:window}function Ow(){try{return Mw()?.localStorage||null}catch{return null}}function Lw(e=Ow()){try{return e?.getItem(Dw)!=="false"}catch{return!0}}function Pw(e,t=Ow()){try{t?.setItem(Dw,e?"true":"false")}catch{}}function Uw(e=Mw()){try{return e?.matchMedia?.("(min-width: 768px)").matches===!0}catch{return!1}}function jw(e,t){return t?{...e,desktopOpen:!e.desktopOpen}:{...e,mobileOpen:!e.mobileOpen}}function Fw(e,t){return t?e.desktopOpen:e.mobileOpen}function zw({onNewChat:e}={}){let t=ve(),[a,n]=p.default.useState(()=>({mobileOpen:!1,desktopOpen:Lw()})),[r,s]=p.default.useState(()=>Uw());p.default.useEffect(()=>{let d=window.matchMedia("(min-width: 768px)"),m=()=>s(d.matches);return m(),d.addEventListener?.("change",m),()=>d.removeEventListener?.("change",m)},[]),p.default.useEffect(()=>{Pw(a.desktopOpen)},[a.desktopOpen]);let i=p.default.useCallback(()=>{n(d=>({...d,mobileOpen:!1}))},[]),o=p.default.useCallback(()=>{n(d=>jw(d,r))},[r]),l=p.default.useCallback(async()=>{let d=await e?.(),m=typeof d=="string"&&d.length>0?d:null;t(m?`/chat/${m}`:"/chat"),i()},[t,i,e]),c=p.default.useCallback(d=>{t(`/chat/${d}`),i()},[t,i]);return{mobileOpen:a.mobileOpen,desktopOpen:a.desktopOpen,currentOpen:Fw(a,r),close:i,toggle:o,newChat:l,selectThread:c}}var oh=new Set,jA=0;function di(e,t={}){let a={id:++jA,message:e,tone:t.tone||"info",duration:t.duration??2600};return oh.forEach(n=>n(a)),a.id}function Bw(e){return oh.add(e),()=>oh.delete(e)}function FA(e){return e?.status===409&&e?.payload?.kind==="busy"}function qw(e,t){return FA(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function Iw(){let e=K({queryKey:["threads"],queryFn:()=>J0({})}),[t,a]=p.default.useState(null),[n,r]=p.default.useState(!1),s=p.default.useRef(new Map),i=p.default.useCallback(async c=>{let d=c||"__global__",m=s.current.get(d);if(m)return m;r(!0);let f=(async()=>{try{let h=await wc(c?{projectId:c}:void 0);Dt.invalidateQueries({queryKey:["threads"]});let x=h?.thread?.thread_id;return x&&a(x),x}finally{s.current.delete(d),r(s.current.size>0)}})();return s.current.set(d,f),f},[]),o=p.default.useCallback(async c=>{await X0({threadId:c}),t===c&&a(null),Dt.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:p.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var Hw={attach:u`<path
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
      ${Hw[e]||Hw.spark}
    </svg>
  `}function J(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=J(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function Kw(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function zA(e){return Kw(e).trim().charAt(0).toUpperCase()||"I"}function BA(){let[e,t]=p.default.useState(!1),a=p.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function Qw({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=k(),s=BA(),i=Kw(a),o=a?.email||a?.role||r("common.gatewaySession");return u`
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
  `}var Vw={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",logs:"list",settings:"settings",admin:"shield"},qA=rl.filter(e=>e.id!=="chat"&&!e.hidden);function IA({route:e,label:t,onNavigate:a}){return u`
    <${Za}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>J("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${D} name=${Vw[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function HA({route:e,label:t,subRoutes:a,onNavigate:n}){let r=k(),s=Ae(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return u`
    <div className="flex flex-col">
      <${Za}
        to=${o}
        onClick=${n}
        className=${()=>J("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${D}
          name=${Vw[e.id]||"bolt"}
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
  `}function Gw({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=k(),s=p.default.useMemo(()=>qA.filter(i=>a||i.id!=="admin"),[a]);return u`
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
        ${s.map(i=>{let o=(Cc[i.id]||[]).filter(l=>a||!(i.id==="settings"&&["users","inference"].includes(l.id)));return o.length>0?u`
              <${HA}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:u`
            <${IA}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var Na=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),ol=new Set([Na.NEEDS_ATTENTION,Na.FAILED]),lh="ironclaw:v2-thread-attention",uh=new Set,mi=new Map;function KA(){try{let e=window.localStorage.getItem(lh);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&ol.has(a[1])):[]}catch{return[]}}function Yw(){let e=[];for(let[t,a]of mi)ol.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(lh):window.localStorage.setItem(lh,JSON.stringify(e))}catch{}}for(let[e,t]of KA())mi.set(e,t);function Xw(){return new Map(mi)}function Jw(){let e=Xw();for(let t of uh)try{t(e)}catch{}}function Oc(e,t){if(!e)return;let a=mi.get(e);if(t==null){if(!mi.delete(e))return;ol.has(a)&&Yw(),Jw();return}a!==t&&(mi.set(e,t),(ol.has(t)||ol.has(a))&&Yw(),Jw())}function Ww(e){Oc(e,null)}function QA(){return Xw()}function VA(e){return uh.add(e),()=>{uh.delete(e)}}function Zw(){let[e,t]=p.default.useState(QA);return p.default.useEffect(()=>VA(t),[]),e}function Lc(e){return e.updated_at||e.created_at||null}function ch(e,t){let a=Lc(e)||"",n=Lc(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function e1(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function t1(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function GA(){let[e,t]=p.default.useState(C$);return p.default.useEffect(()=>E$(t),[]),e}var YA=Object.freeze({[Na.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-transparent"},[Na.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[Na.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function JA(e){return e&&YA[e]||null}function XA(e){let t=String(e?.state||"").toLowerCase();return t==="processing"||t==="running"?Na.RUNNING:t==="needs_attention"||t==="awaitingapproval"||t==="awaiting_approval"?Na.NEEDS_ATTENTION:t==="failed"||t==="interrupted"?Na.FAILED:null}function WA({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=k(),o=Lc(e),l=e1(o),c=t1(o),d=p.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(h=>{window.alert(h?.message||"Unable to delete chat")})},[s,e.id]),m=p.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),k$(e.id)},[e.id]);return u`
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
  `}function a1({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:u`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>u`
          <${WA}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${JA(n.has(o.id)?n.get(o.id):XA(o))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function n1({threads:e,activeThreadId:t,rebornProjectsEnabled:a=!1,onSelect:n,onDelete:r,onNavigate:s}){let[i,o]=p.default.useState(!1),[l,c]=p.default.useState(""),d=Zw(),m=GA(),f=k(),{pinned:h,recent:x,totalMatches:y}=p.default.useMemo(()=>{let $=l.trim().toLowerCase(),g=$?e.filter(w=>(w.title||w.id||"").toLowerCase().includes($)):e,v=[],b=[];for(let w of g)m.has(w.id)?v.push(w):b.push(w);return v.sort(ch),b.sort(ch),{pinned:v,recent:b,totalMatches:v.length+b.length}},[e,l,m]);return u`
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

          <${a1}
            label=${f("common.pinned")}
            items=${h}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${m}
            onSelect=${n}
            onDelete=${r}
          />
          <${a1}
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
  `}function Pc(){let e=Z(),t=K({queryKey:["trace-credits"],queryFn:xw,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=Y({mutationFn:$w,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function ZA(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function r1(){let e=k(),{credits:t}=Pc();if(!t||!t.enrolled)return null;let a=ZA(t.final_credit),n=t.submissions_accepted||0,r=t.submissions_submitted||0,s=t.manual_review_hold_count||0;return u`
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
  `}function s1({id:e,threadsState:t,theme:a,toggleTheme:n,profile:r,isAdmin:s,rebornProjectsEnabled:i=!1,onSignOut:o,onClose:l,onNewChat:c,onSelectThread:d,onDeleteThread:m}){return u`
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

      <${Gw}
        onNewChat=${c}
        isCreating=${t.isCreating}
        isAdmin=${s}
        onNavigate=${l}
      />

      <${r1} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${n1}
          threads=${t.threads}
          activeThreadId=${t.activeThreadId}
          rebornProjectsEnabled=${i}
          onSelect=${d}
          onDelete=${m}
          onNavigate=${l}
        />
      </div>

      <${Qw}
        theme=${a}
        toggleTheme=${n}
        profile=${r}
        onSignOut=${o}
      />
    </aside>
  `}var e4="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",t4="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",i1="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",o1={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},l1={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function T({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=o1[n]??o1.md,l=r?"w-full":"";if(a==="primary")return u`
      <${s}
        style=${{background:e4,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${J(i1,o,l,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:t4}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=l1[a]??l1.outline;return u`
    <${s}
      className=${J(i1,o,l,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function u1(){let e=p.default.useMemo(()=>a4(window.location),[]),[t,a]=p.default.useState(null),[n,r]=p.default.useState(null),[s,i]=p.default.useState(!1),[o,l]=p.default.useState(""),[c,d]=p.default.useState(!1);p.default.useEffect(()=>{if(!e)return;let h=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:h.signal}).then(x=>{if(!x.ok)throw new Error(String(x.status));return x.json()}).then(a).catch(()=>{h.signal.aborted||a(null)}),()=>h.abort()},[e]);let m=p.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),l("");try{let h=await fetch(`${e.base}/attestation/report`);if(!h.ok)throw new Error(String(h.status));let x=await h.json();return r(x),x}catch(h){return l(h.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),f=p.default.useCallback(async()=>{let h=n||await m();return!h||!navigator.clipboard?!1:(await navigator.clipboard.writeText(JSON.stringify({...h,instance_attestation:t},null,2)),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[m,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:m,copyReport:f}}function a4(e){let t=e.hostname;if(!t||t==="localhost"||n4(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function n4(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}var r4=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function c1(){let e=k(),t=u1(),[a,n]=p.default.useState(!1),r=p.default.useCallback(()=>{n(o=>{let l=!o;return l&&t.loadReport(),l})},[t]),s=p.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=s4({teeInfo:t.teeInfo,report:t.report,t:e});return u`
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
  `}function s4({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return r4.map(([r,s])=>({label:a(s),value:i4(n[r])||a("common.unknown")}))}function i4(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var o4="https://docs.ironclaw.com";function d1({threadsState:e,onToggleSidebar:t,sidebarOpen:a=!0}){let n=k(),r=Ae(),s=p.default.useMemo(()=>{for(let o of rl){let l=Cc[o.id];if(!l)continue;let c=o.path+"/";if(r.pathname.startsWith(c)){let d=r.pathname.slice(c.length).split("/")[0],m=l.find(f=>f.id===d);if(m)return{parent:n(o.labelKey),current:n(m.labelKey)}}}return null},[r.pathname,n]),i=p.default.useMemo(()=>{if(s)return null;if(r.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(c=>c.id===e.activeThreadId)?.title||n("nav.chat");let o=rl.find(l=>r.pathname.startsWith(l.path));return o?n(o.labelKey):""},[r.pathname,e.activeThreadId,e.threads,n,s]);return u`
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
        <${c1} />
        <${Za}
          to="/logs"
          className=${({isActive:o})=>J("inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",o&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
          title=${n("nav.logs")}
        >
          ${n("nav.logs")}
        <//>
        <a
          href=${o4}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${n("nav.docs")}
        >
          ${n("nav.docs")}
        </a>
      </div>
    </header>
  `}function m1({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=ve(),i=k(),[o,l]=p.default.useState(""),[c,d]=p.default.useState(0),m=p.default.useRef(null),f=p.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(b=>({id:`thread-${b.id}`,label:b.title||`Thread ${b.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${b.id}`)}));return[...g,...v]},[a,s,n,r]),h=p.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?f.filter(v=>v.label.toLowerCase().includes(g)):f},[f,o]);p.default.useEffect(()=>{if(!e)return;l(""),d(0);let g=window.requestAnimationFrame(()=>m.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),p.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,h.length-1)))},[h.length]);let x=p.default.useCallback(g=>{g&&(t(),g.run())},[t]),y=p.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,h.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),x(h[c])):g.key==="Escape"&&(g.preventDefault(),t())},[h,c,x,t]);if(!e)return null;let $=null;return u`
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
  `}var f1={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},l4={info:"bolt",success:"check",error:"close"};function p1(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>Bw(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:u`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${e.map(a=>u`
          <div
            key=${a.id}
            role="status"
            className=${["pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",f1[a.tone]||f1.info].join(" ")}
          >
            <${D} name=${l4[a.tone]||"bolt"} className="h-4 w-4 shrink-0" />
            <span>${a.message}</span>
          </div>
        `)}
    </div>
  `}function h1({token:e,profile:t,isChecking:a=!1,isAdmin:n,rebornProjectsEnabled:r=!1,onSignOut:s}){let i=k(),{theme:o,toggleTheme:l}=Ec(),c=J$(e),d=Iw(),m=zw({onNewChat:()=>d.setActiveThreadId(null)}),f=c.data,h=Ae(),x=ve(),y=ci({settings:{},gatewayStatus:f,enabled:n}),$=n&&Aw({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),g=h.pathname==="/welcome"||h.pathname.startsWith("/settings"),[v,b]=p.default.useState(!1);p.default.useEffect(()=>{let S=C=>{(C.metaKey||C.ctrlKey)&&C.key.toLowerCase()==="k"&&(C.preventDefault(),b(R=>!R))};return window.addEventListener("keydown",S),()=>window.removeEventListener("keydown",S)},[]);let w=p.default.useCallback(async S=>{let C=d.activeThreadId===S;try{await d.deleteThread(S),C&&x("/chat",{replace:!0})}catch(R){console.error("Failed to delete thread:",R),di(qw(R,i),{tone:"error"})}},[x,d,i]);return $&&!g?u`<${ot} to="/welcome" replace />`:u`
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
        <${s1}
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
        <${d1}
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
          <${Mp}
            context=${{gatewayStatus:f,gatewayStatusQuery:c,currentUser:t,isChecking:a,isAdmin:n,threadsState:d}}
          />
        </main>
      </div>
      <${m1}
        open=${v}
        onClose=${()=>b(!1)}
        threadsState=${d}
        onNewChat=${m.newChat}
        onToggleTheme=${l}
      />
      <${p1} />
    </div>
  `}var Kt=qe(Qe(),1),ml=e=>e.type==="checkbox",Vr=e=>e instanceof Date,Mt=e=>e==null,C1=e=>typeof e=="object",Ye=e=>!Mt(e)&&!Array.isArray(e)&&C1(e)&&!Vr(e),u4=e=>Ye(e)&&e.target?ml(e.target)?e.target.checked:e.target.value:e,c4=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,d4=(e,t)=>e.has(c4(t)),m4=e=>{let t=e.constructor&&e.constructor.prototype;return Ye(t)&&t.hasOwnProperty("isPrototypeOf")},fh=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function pt(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(fh&&(e instanceof Blob||n))&&(a||Ye(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!m4(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=pt(e[r]));else return e;return t}var Bc=e=>/^\w*$/.test(e),Ze=e=>e===void 0,ph=e=>Array.isArray(e)?e.filter(Boolean):[],hh=e=>ph(e.replace(/["|']|\]/g,"").split(/\.|\[/)),ee=(e,t,a)=>{if(!t||!Ye(e))return a;let n=(Bc(t)?[t]:hh(t)).reduce((r,s)=>Mt(r)?r:r[s],e);return Ze(n)||n===e?Ze(e[t])?a:e[t]:n},en=e=>typeof e=="boolean",ze=(e,t,a)=>{let n=-1,r=Bc(t)?[t]:hh(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],l=a;if(n!==i){let c=e[o];l=Ye(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=l,e=e[o]}},v1={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},Ma={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},Cn={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},f4=Kt.default.createContext(null);f4.displayName="HookFormContext";var p4=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==Ma.all&&(t._proxyFormState[i]=!n||Ma.all),a&&(a[i]=!0),e[i]}});return r},h4=typeof window<"u"?Kt.default.useLayoutEffect:Kt.default.useEffect;var tn=e=>typeof e=="string",v4=(e,t,a,n,r)=>tn(e)?(n&&t.watch.add(e),ee(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),ee(a,s))):(n&&(t.watchAll=!0),a),mh=e=>Mt(e)||!C1(e);function lr(e,t,a=new WeakSet){if(mh(e)||mh(t))return e===t;if(Vr(e)&&Vr(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(Vr(i)&&Vr(o)||Ye(i)&&Ye(o)||Array.isArray(i)&&Array.isArray(o)?!lr(i,o,a):i!==o)return!1}}return!0}var g4=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},cl=e=>Array.isArray(e)?e:[e],g1=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},Qt=e=>Ye(e)&&!Object.keys(e).length,vh=e=>e.type==="file",Oa=e=>typeof e=="function",jc=e=>{if(!fh)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},E1=e=>e.type==="select-multiple",gh=e=>e.type==="radio",y4=e=>gh(e)||ml(e),dh=e=>jc(e)&&e.isConnected;function b4(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=Ze(e)?n++:e[t[n++]];return e}function x4(e){for(let t in e)if(e.hasOwnProperty(t)&&!Ze(e[t]))return!1;return!0}function We(e,t){let a=Array.isArray(t)?t:Bc(t)?[t]:hh(t),n=a.length===1?e:b4(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Ye(n)&&Qt(n)||Array.isArray(n)&&x4(n))&&We(e,a.slice(0,-1)),e}var T1=e=>{for(let t in e)if(Oa(e[t]))return!0;return!1};function Fc(e,t={}){let a=Array.isArray(e);if(Ye(e)||a)for(let n in e)Array.isArray(e[n])||Ye(e[n])&&!T1(e[n])?(t[n]=Array.isArray(e[n])?[]:{},Fc(e[n],t[n])):Mt(e[n])||(t[n]=!0);return t}function A1(e,t,a){let n=Array.isArray(e);if(Ye(e)||n)for(let r in e)Array.isArray(e[r])||Ye(e[r])&&!T1(e[r])?Ze(t)||mh(a[r])?a[r]=Array.isArray(e[r])?Fc(e[r],[]):{...Fc(e[r])}:A1(e[r],Mt(t)?{}:t[r],a[r]):a[r]=!lr(e[r],t[r]);return a}var ll=(e,t)=>A1(e,t,Fc(t)),y1={value:!1,isValid:!1},b1={value:!0,isValid:!0},D1=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!Ze(e[0].attributes.value)?Ze(e[0].value)||e[0].value===""?b1:{value:e[0].value,isValid:!0}:b1:y1}return y1},M1=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>Ze(e)?e:t?e===""?NaN:e&&+e:a&&tn(e)?new Date(e):n?n(e):e,x1={isValid:!1,value:null},O1=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,x1):x1;function $1(e){let t=e.ref;return vh(t)?t.files:gh(t)?O1(e.refs).value:E1(t)?[...t.selectedOptions].map(({value:a})=>a):ml(t)?D1(e.refs).value:M1(Ze(t.value)?e.ref.value:t.value,e)}var $4=(e,t,a,n)=>{let r={};for(let s of e){let i=ee(t,s);i&&ze(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},zc=e=>e instanceof RegExp,ul=e=>Ze(e)?e:zc(e)?e.source:Ye(e)?zc(e.value)?e.value.source:e.value:e,w1=e=>({isOnSubmit:!e||e===Ma.onSubmit,isOnBlur:e===Ma.onBlur,isOnChange:e===Ma.onChange,isOnAll:e===Ma.all,isOnTouch:e===Ma.onTouched}),S1="AsyncFunction",w4=e=>!!e&&!!e.validate&&!!(Oa(e.validate)&&e.validate.constructor.name===S1||Ye(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===S1)),S4=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),N1=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),dl=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=ee(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(dl(o,t))break}else if(Ye(o)&&dl(o,t))break}}};function _1(e,t,a){let n=ee(e,a);if(n||Bc(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=ee(t,s),o=ee(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var N4=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return Qt(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||Ma.all))},_4=(e,t,a)=>!e||!t||e===t||cl(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),R4=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,k4=(e,t)=>!ph(ee(e,t)).length&&We(e,t),C4=(e,t,a)=>{let n=cl(ee(e,a));return ze(n,"root",t[a]),ze(e,a,n),e},Uc=e=>tn(e);function R1(e,t,a="validate"){if(Uc(e)||Array.isArray(e)&&e.every(Uc)||en(e)&&!e)return{type:a,message:Uc(e)?e:"",ref:t}}var fi=e=>Ye(e)&&!zc(e)?e:{value:e,message:""},k1=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:l,maxLength:c,minLength:d,min:m,max:f,pattern:h,validate:x,name:y,valueAsNumber:$,mount:g}=e._f,v=ee(a,y);if(!g||t.has(y))return{};let b=o?o[0]:i,w=F=>{r&&b.reportValidity&&(b.setCustomValidity(en(F)?"":F||""),b.reportValidity())},S={},C=gh(i),R=ml(i),_=C||R,M=($||vh(i))&&Ze(i.value)&&Ze(v)||jc(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,L=g4.bind(null,y,n,S),U=(F,z,P,G=Cn.maxLength,ae=Cn.minLength)=>{let le=F?z:P;S[y]={type:F?G:ae,message:le,ref:i,...L(F?G:ae,le)}};if(s?!Array.isArray(v)||!v.length:l&&(!_&&(M||Mt(v))||en(v)&&!v||R&&!D1(o).isValid||C&&!O1(o).isValid)){let{value:F,message:z}=Uc(l)?{value:!!l,message:l}:fi(l);if(F&&(S[y]={type:Cn.required,message:z,ref:b,...L(Cn.required,z)},!n))return w(z),S}if(!M&&(!Mt(m)||!Mt(f))){let F,z,P=fi(f),G=fi(m);if(!Mt(v)&&!isNaN(v)){let ae=i.valueAsNumber||v&&+v;Mt(P.value)||(F=ae>P.value),Mt(G.value)||(z=ae<G.value)}else{let ae=i.valueAsDate||new Date(v),le=Oe=>new Date(new Date().toDateString()+" "+Oe),lt=i.type=="time",ht=i.type=="week";tn(P.value)&&v&&(F=lt?le(v)>le(P.value):ht?v>P.value:ae>new Date(P.value)),tn(G.value)&&v&&(z=lt?le(v)<le(G.value):ht?v<G.value:ae<new Date(G.value))}if((F||z)&&(U(!!F,P.message,G.message,Cn.max,Cn.min),!n))return w(S[y].message),S}if((c||d)&&!M&&(tn(v)||s&&Array.isArray(v))){let F=fi(c),z=fi(d),P=!Mt(F.value)&&v.length>+F.value,G=!Mt(z.value)&&v.length<+z.value;if((P||G)&&(U(P,F.message,z.message),!n))return w(S[y].message),S}if(h&&!M&&tn(v)){let{value:F,message:z}=fi(h);if(zc(F)&&!v.match(F)&&(S[y]={type:Cn.pattern,message:z,ref:i,...L(Cn.pattern,z)},!n))return w(z),S}if(x){if(Oa(x)){let F=await x(v,a),z=R1(F,b);if(z&&(S[y]={...z,...L(Cn.validate,z.message)},!n))return w(z.message),S}else if(Ye(x)){let F={};for(let z in x){if(!Qt(F)&&!n)break;let P=R1(await x[z](v,a),b,z);P&&(F={...P,...L(z,P.message)},w(P.message),n&&(S[y]=F))}if(!Qt(F)&&(S[y]={ref:b,...F},!n))return S}}return w(!0),S},E4={mode:Ma.onSubmit,reValidateMode:Ma.onChange,shouldFocusError:!0};function T4(e={}){let t={...E4,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:Oa(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Ye(t.defaultValues)||Ye(t.values)?pt(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:pt(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},l,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},m={...d},f={array:g1(),state:g1()},h=t.criteriaMode===Ma.all,x=N=>E=>{clearTimeout(c),c=setTimeout(N,E)},y=async N=>{if(!t.disabled&&(d.isValid||m.isValid||N)){let E=t.resolver?Qt((await R()).errors):await M(n,!0);E!==a.isValid&&f.state.next({isValid:E})}},$=(N,E)=>{!t.disabled&&(d.isValidating||d.validatingFields||m.isValidating||m.validatingFields)&&((N||Array.from(o.mount)).forEach(A=>{A&&(E?ze(a.validatingFields,A,E):We(a.validatingFields,A))}),f.state.next({validatingFields:a.validatingFields,isValidating:!Qt(a.validatingFields)}))},g=(N,E=[],A,q,B=!0,O=!0)=>{if(q&&A&&!t.disabled){if(i.action=!0,O&&Array.isArray(ee(n,N))){let Q=A(ee(n,N),q.argA,q.argB);B&&ze(n,N,Q)}if(O&&Array.isArray(ee(a.errors,N))){let Q=A(ee(a.errors,N),q.argA,q.argB);B&&ze(a.errors,N,Q),k4(a.errors,N)}if((d.touchedFields||m.touchedFields)&&O&&Array.isArray(ee(a.touchedFields,N))){let Q=A(ee(a.touchedFields,N),q.argA,q.argB);B&&ze(a.touchedFields,N,Q)}(d.dirtyFields||m.dirtyFields)&&(a.dirtyFields=ll(r,s)),f.state.next({name:N,isDirty:U(N,E),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else ze(s,N,E)},v=(N,E)=>{ze(a.errors,N,E),f.state.next({errors:a.errors})},b=N=>{a.errors=N,f.state.next({errors:a.errors,isValid:!1})},w=(N,E,A,q)=>{let B=ee(n,N);if(B){let O=ee(s,N,Ze(A)?ee(r,N):A);Ze(O)||q&&q.defaultChecked||E?ze(s,N,E?O:$1(B._f)):P(N,O),i.mount&&y()}},S=(N,E,A,q,B)=>{let O=!1,Q=!1,ce={name:N};if(!t.disabled){if(!A||q){(d.isDirty||m.isDirty)&&(Q=a.isDirty,a.isDirty=ce.isDirty=U(),O=Q!==ce.isDirty);let ge=lr(ee(r,N),E);Q=!!ee(a.dirtyFields,N),ge?We(a.dirtyFields,N):ze(a.dirtyFields,N,!0),ce.dirtyFields=a.dirtyFields,O=O||(d.dirtyFields||m.dirtyFields)&&Q!==!ge}if(A){let ge=ee(a.touchedFields,N);ge||(ze(a.touchedFields,N,A),ce.touchedFields=a.touchedFields,O=O||(d.touchedFields||m.touchedFields)&&ge!==A)}O&&B&&f.state.next(ce)}return O?ce:{}},C=(N,E,A,q)=>{let B=ee(a.errors,N),O=(d.isValid||m.isValid)&&en(E)&&a.isValid!==E;if(t.delayError&&A?(l=x(()=>v(N,A)),l(t.delayError)):(clearTimeout(c),l=null,A?ze(a.errors,N,A):We(a.errors,N)),(A?!lr(B,A):B)||!Qt(q)||O){let Q={...q,...O&&en(E)?{isValid:E}:{},errors:a.errors,name:N};a={...a,...Q},f.state.next(Q)}},R=async N=>{$(N,!0);let E=await t.resolver(s,t.context,$4(N||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return $(N),E},_=async N=>{let{errors:E}=await R(N);if(N)for(let A of N){let q=ee(E,A);q?ze(a.errors,A,q):We(a.errors,A)}else a.errors=E;return E},M=async(N,E,A={valid:!0})=>{for(let q in N){let B=N[q];if(B){let{_f:O,...Q}=B;if(O){let ce=o.array.has(O.name),ge=B._f&&w4(B._f);ge&&d.validatingFields&&$([q],!0);let gt=await k1(B,o.disabled,s,h,t.shouldUseNativeValidation&&!E,ce);if(ge&&d.validatingFields&&$([q]),gt[O.name]&&(A.valid=!1,E))break;!E&&(ee(gt,O.name)?ce?C4(a.errors,gt,O.name):ze(a.errors,O.name,gt[O.name]):We(a.errors,O.name))}!Qt(Q)&&await M(Q,E,A)}}return A.valid},L=()=>{for(let N of o.unMount){let E=ee(n,N);E&&(E._f.refs?E._f.refs.every(A=>!dh(A)):!dh(E._f.ref))&&la(N)}o.unMount=new Set},U=(N,E)=>!t.disabled&&(N&&E&&ze(s,N,E),!lr(Oe(),r)),F=(N,E,A)=>v4(N,o,{...i.mount?s:Ze(E)?r:tn(N)?{[N]:E}:E},A,E),z=N=>ph(ee(i.mount?s:r,N,t.shouldUnregister?ee(r,N,[]):[])),P=(N,E,A={})=>{let q=ee(n,N),B=E;if(q){let O=q._f;O&&(!O.disabled&&ze(s,N,M1(E,O)),B=jc(O.ref)&&Mt(E)?"":E,E1(O.ref)?[...O.ref.options].forEach(Q=>Q.selected=B.includes(Q.value)):O.refs?ml(O.ref)?O.refs.forEach(Q=>{(!Q.defaultChecked||!Q.disabled)&&(Array.isArray(B)?Q.checked=!!B.find(ce=>ce===Q.value):Q.checked=B===Q.value||!!B)}):O.refs.forEach(Q=>Q.checked=Q.value===B):vh(O.ref)?O.ref.value="":(O.ref.value=B,O.ref.type||f.state.next({name:N,values:pt(s)})))}(A.shouldDirty||A.shouldTouch)&&S(N,B,A.shouldTouch,A.shouldDirty,!0),A.shouldValidate&&ht(N)},G=(N,E,A)=>{for(let q in E){if(!E.hasOwnProperty(q))return;let B=E[q],O=N+"."+q,Q=ee(n,O);(o.array.has(N)||Ye(B)||Q&&!Q._f)&&!Vr(B)?G(O,B,A):P(O,B,A)}},ae=(N,E,A={})=>{let q=ee(n,N),B=o.array.has(N),O=pt(E);ze(s,N,O),B?(f.array.next({name:N,values:pt(s)}),(d.isDirty||d.dirtyFields||m.isDirty||m.dirtyFields)&&A.shouldDirty&&f.state.next({name:N,dirtyFields:ll(r,s),isDirty:U(N,O)})):q&&!q._f&&!Mt(O)?G(N,O,A):P(N,O,A),N1(N,o)&&f.state.next({...a,name:N}),f.state.next({name:i.mount?N:void 0,values:pt(s)})},le=async N=>{i.mount=!0;let E=N.target,A=E.name,q=!0,B=ee(n,A),O=ge=>{q=Number.isNaN(ge)||Vr(ge)&&isNaN(ge.getTime())||lr(ge,ee(s,A,ge))},Q=w1(t.mode),ce=w1(t.reValidateMode);if(B){let ge,gt,Ce=E.type?$1(B._f):u4(N),Ct=N.type===v1.BLUR||N.type===v1.FOCUS_OUT,on=!S4(B._f)&&!t.resolver&&!ee(a.errors,A)&&!B._f.deps||R4(Ct,ee(a.touchedFields,A),a.isSubmitted,ce,Q),ja=N1(A,o,Ct);ze(s,A,Ce),Ct?(!E||!E.readOnly)&&(B._f.onBlur&&B._f.onBlur(N),l&&l(0)):B._f.onChange&&B._f.onChange(N);let Fa=S(A,Ce,Ct),gr=!Qt(Fa)||ja;if(!Ct&&f.state.next({name:A,type:N.type,values:pt(s)}),on)return(d.isValid||m.isValid)&&(t.mode==="onBlur"?Ct&&y():Ct||y()),gr&&f.state.next({name:A,...ja?{}:Fa});if(!Ct&&ja&&f.state.next({...a}),t.resolver){let{errors:yr}=await R([A]);if(O(Ce),q){let Zr=_1(a.errors,n,A),es=_1(yr,n,Zr.name||A);ge=es.error,A=es.name,gt=Qt(yr)}}else $([A],!0),ge=(await k1(B,o.disabled,s,h,t.shouldUseNativeValidation))[A],$([A]),O(Ce),q&&(ge?gt=!1:(d.isValid||m.isValid)&&(gt=await M(n,!0)));q&&(B._f.deps&&ht(B._f.deps),C(A,gt,ge,Fa))}},lt=(N,E)=>{if(ee(a.errors,E)&&N.focus)return N.focus(),1},ht=async(N,E={})=>{let A,q,B=cl(N);if(t.resolver){let O=await _(Ze(N)?N:B);A=Qt(O),q=N?!B.some(Q=>ee(O,Q)):A}else N?(q=(await Promise.all(B.map(async O=>{let Q=ee(n,O);return await M(Q&&Q._f?{[O]:Q}:Q)}))).every(Boolean),!(!q&&!a.isValid)&&y()):q=A=await M(n);return f.state.next({...!tn(N)||(d.isValid||m.isValid)&&A!==a.isValid?{}:{name:N},...t.resolver||!N?{isValid:A}:{},errors:a.errors}),E.shouldFocus&&!q&&dl(n,lt,N?B:o.mount),q},Oe=N=>{let E={...i.mount?s:r};return Ze(N)?E:tn(N)?ee(E,N):N.map(A=>ee(E,A))},De=(N,E)=>({invalid:!!ee((E||a).errors,N),isDirty:!!ee((E||a).dirtyFields,N),error:ee((E||a).errors,N),isValidating:!!ee(a.validatingFields,N),isTouched:!!ee((E||a).touchedFields,N)}),at=N=>{N&&cl(N).forEach(E=>We(a.errors,E)),f.state.next({errors:N?a.errors:{}})},$t=(N,E,A)=>{let q=(ee(n,N,{_f:{}})._f||{}).ref,B=ee(a.errors,N)||{},{ref:O,message:Q,type:ce,...ge}=B;ze(a.errors,N,{...ge,...E,ref:q}),f.state.next({name:N,errors:a.errors,isValid:!1}),A&&A.shouldFocus&&q&&q.focus&&q.focus()},Le=(N,E)=>Oa(N)?f.state.subscribe({next:A=>"values"in A&&N(F(void 0,E),A)}):F(N,E,!0),Pa=N=>f.state.subscribe({next:E=>{_4(N.name,E.name,N.exact)&&N4(E,N.formState||d,X,N.reRenderRoot)&&N.callback({values:{...s},...a,...E,defaultValues:r})}}).unsubscribe,kt=N=>(i.mount=!0,m={...m,...N.formState},Pa({...N,formState:m})),la=(N,E={})=>{for(let A of N?cl(N):o.mount)o.mount.delete(A),o.array.delete(A),E.keepValue||(We(n,A),We(s,A)),!E.keepError&&We(a.errors,A),!E.keepDirty&&We(a.dirtyFields,A),!E.keepTouched&&We(a.touchedFields,A),!E.keepIsValidating&&We(a.validatingFields,A),!t.shouldUnregister&&!E.keepDefaultValue&&We(r,A);f.state.next({values:pt(s)}),f.state.next({...a,...E.keepDirty?{isDirty:U()}:{}}),!E.keepIsValid&&y()},rn=({disabled:N,name:E})=>{(en(N)&&i.mount||N||o.disabled.has(E))&&(N?o.disabled.add(E):o.disabled.delete(E))},ua=(N,E={})=>{let A=ee(n,N),q=en(E.disabled)||en(t.disabled);return ze(n,N,{...A||{},_f:{...A&&A._f?A._f:{ref:{name:N}},name:N,mount:!0,...E}}),o.mount.add(N),A?rn({disabled:en(E.disabled)?E.disabled:t.disabled,name:N}):w(N,!0,E.value),{...q?{disabled:E.disabled||t.disabled}:{},...t.progressive?{required:!!E.required,min:ul(E.min),max:ul(E.max),minLength:ul(E.minLength),maxLength:ul(E.maxLength),pattern:ul(E.pattern)}:{},name:N,onChange:le,onBlur:le,ref:B=>{if(B){ua(N,E),A=ee(n,N);let O=Ze(B.value)&&B.querySelectorAll&&B.querySelectorAll("input,select,textarea")[0]||B,Q=y4(O),ce=A._f.refs||[];if(Q?ce.find(ge=>ge===O):O===A._f.ref)return;ze(n,N,{_f:{...A._f,...Q?{refs:[...ce.filter(dh),O,...Array.isArray(ee(r,N))?[{}]:[]],ref:{type:O.type,name:N}}:{ref:O}}}),w(N,!1,void 0,O)}else A=ee(n,N,{}),A._f&&(A._f.mount=!1),(t.shouldUnregister||E.shouldUnregister)&&!(d4(o.array,N)&&i.action)&&o.unMount.add(N)}}},Vt=()=>t.shouldFocusError&&dl(n,lt,o.mount),sn=N=>{en(N)&&(f.state.next({disabled:N}),dl(n,(E,A)=>{let q=ee(n,A);q&&(E.disabled=q._f.disabled||N,Array.isArray(q._f.refs)&&q._f.refs.forEach(B=>{B.disabled=q._f.disabled||N}))},0,!1))},vt=(N,E)=>async A=>{let q;A&&(A.preventDefault&&A.preventDefault(),A.persist&&A.persist());let B=pt(s);if(f.state.next({isSubmitting:!0}),t.resolver){let{errors:O,values:Q}=await R();a.errors=O,B=pt(Q)}else await M(n);if(o.disabled.size)for(let O of o.disabled)We(B,O);if(We(a.errors,"root"),Qt(a.errors)){f.state.next({errors:{}});try{await N(B,A)}catch(O){q=O}}else E&&await E({...a.errors},A),Vt(),setTimeout(Vt);if(f.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:Qt(a.errors)&&!q,submitCount:a.submitCount+1,errors:a.errors}),q)throw q},ca=(N,E={})=>{ee(n,N)&&(Ze(E.defaultValue)?ae(N,pt(ee(r,N))):(ae(N,E.defaultValue),ze(r,N,pt(E.defaultValue))),E.keepTouched||We(a.touchedFields,N),E.keepDirty||(We(a.dirtyFields,N),a.isDirty=E.defaultValue?U(N,pt(ee(r,N))):U()),E.keepError||(We(a.errors,N),d.isValid&&y()),f.state.next({...a}))},_a=(N,E={})=>{let A=N?pt(N):r,q=pt(A),B=Qt(N),O=B?r:q;if(E.keepDefaultValues||(r=A),!E.keepValues){if(E.keepDirtyValues){let Q=new Set([...o.mount,...Object.keys(ll(r,s))]);for(let ce of Array.from(Q))ee(a.dirtyFields,ce)?ze(O,ce,ee(s,ce)):ae(ce,ee(O,ce))}else{if(fh&&Ze(N))for(let Q of o.mount){let ce=ee(n,Q);if(ce&&ce._f){let ge=Array.isArray(ce._f.refs)?ce._f.refs[0]:ce._f.ref;if(jc(ge)){let gt=ge.closest("form");if(gt){gt.reset();break}}}}if(E.keepFieldsRef)for(let Q of o.mount)ae(Q,ee(O,Q));else n={}}s=t.shouldUnregister?E.keepDefaultValues?pt(r):{}:pt(O),f.array.next({values:{...O}}),f.state.next({values:{...O}})}o={mount:E.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!E.keepIsValid||!!E.keepDirtyValues,i.watch=!!t.shouldUnregister,f.state.next({submitCount:E.keepSubmitCount?a.submitCount:0,isDirty:B?!1:E.keepDirty?a.isDirty:!!(E.keepDefaultValues&&!lr(N,r)),isSubmitted:E.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:B?{}:E.keepDirtyValues?E.keepDefaultValues&&s?ll(r,s):a.dirtyFields:E.keepDefaultValues&&N?ll(r,N):E.keepDirty?a.dirtyFields:{},touchedFields:E.keepTouched?a.touchedFields:{},errors:E.keepErrors?a.errors:{},isSubmitSuccessful:E.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},da=(N,E)=>_a(Oa(N)?N(s):N,E),Ua=(N,E={})=>{let A=ee(n,N),q=A&&A._f;if(q){let B=q.refs?q.refs[0]:q.ref;B.focus&&(B.focus(),E.shouldSelect&&Oa(B.select)&&B.select())}},X=N=>{a={...a,...N}},ie={control:{register:ua,unregister:la,getFieldState:De,handleSubmit:vt,setError:$t,_subscribe:Pa,_runSchema:R,_focusError:Vt,_getWatch:F,_getDirty:U,_setValid:y,_setFieldArray:g,_setDisabledField:rn,_setErrors:b,_getFieldArray:z,_reset:_a,_resetDefaultValues:()=>Oa(t.defaultValues)&&t.defaultValues().then(N=>{da(N,t.resetOptions),f.state.next({isLoading:!1})}),_removeUnmounted:L,_disableForm:sn,_subjects:f,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(N){i=N},get _defaultValues(){return r},get _names(){return o},set _names(N){o=N},get _formState(){return a},get _options(){return t},set _options(N){t={...t,...N}}},subscribe:kt,trigger:ht,register:ua,handleSubmit:vt,watch:Le,setValue:ae,getValues:Oe,reset:da,resetField:ca,clearErrors:at,unregister:la,setError:$t,setFocus:Ua,getFieldState:De};return{...ie,formControl:ie}}function L1(e={}){let t=Kt.default.useRef(void 0),a=Kt.default.useRef(void 0),[n,r]=Kt.default.useState({isDirty:!1,isValidating:!1,isLoading:Oa(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:Oa(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!Oa(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=T4(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,h4(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),Kt.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),Kt.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),Kt.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),Kt.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),Kt.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),Kt.default.useEffect(()=>{e.values&&!lr(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),Kt.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=p4(n,s),t.current}var P1={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},U1={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},A4={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function ne({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return u`
    <${s}
      className=${J(P1[a]??P1.default,U1[n]??U1.md,A4[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var yh="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",qc={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function Ot({className:e="",size:t="md",error:a=!1,...n}){return u`
    <input
      className=${J(yh,qc[t]??qc.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Ic({className:e="",error:t=!1,rows:a=4,...n}){return u`
    <textarea
      rows=${a}
      className=${J(yh,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function bh({children:e,className:t="",size:a="md",error:n=!1,...r}){return u`
    <div className="relative w-full">
      <select
        className=${J(yh,qc[a]??qc.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
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
  `}function D4({children:e,className:t="",required:a=!1,...n}){return u`
    <label
      className=${J("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&u`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function En({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return u`
    <div className=${J("flex flex-col gap-2",s)}>
      ${e&&u`<${D4} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&u`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&u`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var M4={google:"Google",github:"GitHub",apple:"Apple"};function O4(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function j1({providers:e,redirectAfter:t}){let a=k();return e.length?u`
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
              href=${O4(n,t)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <${D} name="shield" className="h-4 w-4" />
              ${a("login.oauthProvider",{provider:M4[n]||n})}
            <//>
          `)}
      </div>
    </div>
  `:null}var L4=["google","github","apple"];function F1(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>{let a=!1;return y$().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(L4.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function z1({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=k(),{theme:s,toggleTheme:i}=Ec(),o=F1(),{formState:{errors:l,isSubmitting:c},handleSubmit:d,register:m}=L1({defaultValues:{token:e||""}});return u`
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

        <${j1}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var B1={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},q1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function I({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return u`
    <span
      className=${J("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",q1[n]??q1.md,B1[e]??B1.muted,r)}
    >
      ${a&&u`<span
          className=${J("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var P4=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,I1=/(bash|shell|exec|run|command|terminal|spawn|process)/,H1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function K1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return P4.test(n)?{tone:"danger",key:"tool.riskWrite"}:I1.test(n)?{tone:"warning",key:"tool.riskExec"}:H1.test(n)?{tone:"info",key:"tool.riskNetwork"}:I1.test(r)?{tone:"warning",key:"tool.riskExec"}:H1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}var Hc=480;function U4(e,t){return t&&t.length>0?t.some(a=>typeof a?.value=="string"&&a.value.length>Hc):typeof e=="string"&&e.length>Hc}function Q1(e,t){return typeof e!="string"||t||e.length<=Hc?e:`${e.slice(0,Hc).trimEnd()}
...`}function V1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=k(),{toolName:s,description:i,parameters:o,allowAlways:l,approvalDetails:c=[]}=e,[d,m]=p.default.useState(!1),[f,h]=p.default.useState(!1),[x,y]=p.default.useState(!1),$=p.default.useRef(!1),g=p.default.useRef(e);g.current=e,p.default.useEffect(()=>{h(!1),$.current=!1,y(!1)},[e]);let v=p.default.useMemo(()=>K1(s,i,o),[s,i,o]),b=s||r("approval.thisTool"),w=U4(o,c),S=f?"max-h-72":"max-h-36",C=p.default.useCallback(async _=>{if($.current)return;let M=g.current;$.current=!0,y(!0);try{await _?.()}finally{g.current===M&&($.current=!1,y(!1))}},[]),R=p.default.useCallback(()=>{C(d&&l?n:t)},[d,l,n,t,C]);return u`
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
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${Q1(_.value,f)}</dd>
                  </div>
                `)}
            </dl>
          `:o&&u`<pre className=${`mb-2 ${S} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${Q1(o,f)}</pre>`}

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
  `}function G1({gate:e,onCancel:t}){let a=k();return u`
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
  `}function Y1({gate:e,onCancel:t}){let a=k(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),o=p.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]);p.default.useEffect(()=>{i("")},[e.authorizationUrl,e.gateRef,e.runId]);let l=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),c=p.default.useCallback(()=>{if(!o){i(a("authGate.serviceUnavailable"));return}i(""),window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0)},[e.authorizationUrl,o]),d=n?a("authGate.reopenAuthorization",{provider:l}):a("authGate.openAuthorization",{provider:l});return u`
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
  `}function J1({gate:e,onSubmit:t,onCancel:a}){let n=k(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState(!1),d=p.default.useCallback(async m=>{m.preventDefault();let f=r.trim();if(!f){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(f),s("")}catch(h){o(h?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return u`
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
  `}var j4="/api/webchat/v2/extensions/pairing/redeem";function X1({channel:e,action:t}){let a=k(),n=Z(),[r,s]=p.default.useState(""),i=z4(t,a),o=Y({mutationFn:({code:c})=>F4(e,c),onSuccess:()=>{s(""),n.invalidateQueries({queryKey:["extensions"]}),n.invalidateQueries({queryKey:["connectable-channels"]}),n.invalidateQueries({queryKey:["pairing",e]})}}),l=()=>{if(o.isPending)return;let c=r.trim().toUpperCase();c&&o.mutate({code:c})};return u`
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
        ${B4(o.error,i.errorMessage)}
      </p>`}
    </div>
  `}function F4(e,t){return V(j4,{method:"POST",body:JSON.stringify({channel:e,code:t})}).then(a=>({...a,success:!0}))}function z4(e,t){return{title:e?.title||t("pairing.title"),instructions:e?.instructions||t("pairing.instructions"),placeholder:e?.input_placeholder||e?.code_placeholder||t("pairing.placeholder"),submitLabel:e?.submit_label||t("pairing.approve"),successMessage:e?.success_message||t("pairing.success"),errorMessage:e?.error_message||t("pairing.error")}}function B4(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var q4="/api/webchat/v2/extensions/pairing/redeem";function W1(e){return V(q4,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function Kc({action:e}){let t=k(),a=Z(),n=Y({mutationFn:({code:l})=>W1(l),onSuccess:()=>{s(""),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=p.default.useState(""),i=I4(e,t),o=()=>{if(n.isPending)return;let l=r.trim().toUpperCase();l&&n.mutate({code:l})};return u`
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
        ${H4(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function I4(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function H4(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function K4(e,t){return e?.channel==="slack"&&e.strategy===t}function Z1({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return u`
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

      ${K4(e,"inbound_proof_code")?u`<${Kc} action=${e.action} />`:e.strategy==="inbound_proof_code"?u`
              <${X1}
                channel=${a}
                action=${e.action}
              />
            `:u`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function Q4(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):Hr.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:Hr.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:Hr.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:Hr.maxTotalBytes}:Hr}function eS(){let e=Sa(),t=K({enabled:!!e,queryKey:["session"],queryFn:$c,staleTime:5*6e4});return Q4(t.data)}function Qc({onSend:e,onCancel:t,disabled:a,sendDisabled:n=a,canCancel:r=!1,initialText:s="",resetKey:i="",draftKey:o=nl,variant:l="dock",context:c={},statusText:d=""}){let m=k(),f=ft(),h=l==="hero",x=eS(),[y,$]=p.default.useState(()=>eh(o)),[g,v]=p.default.useState(()=>ah(o)),[b,w]=p.default.useState(""),[S,C]=p.default.useState(!1),[R,_]=p.default.useState(!1),[M,L]=p.default.useState(!1),U=p.default.useRef(null),F=p.default.useRef(null),z=p.default.useRef(!1),P=a||n||S,G=p.default.useRef(a||n);G.current=a||n,z.current=P;let ae=p.default.useRef([]),le=p.default.useRef(Promise.resolve()),lt=p.default.useRef({draftKey:o,storageScope:f});lt.current={draftKey:o,storageScope:f},p.default.useEffect(()=>{ae.current=g},[g]);let ht=p.default.useRef(null),Oe=p.default.useRef(null),De=p.default.useCallback(()=>{Oe.current&&(window.clearTimeout(Oe.current),Oe.current=null);let O=ht.current;ht.current=null,O&&O.scope===ft()&&th(O.key,O.text)},[]),at=p.default.useCallback(()=>{Oe.current&&(window.clearTimeout(Oe.current),Oe.current=null),ht.current=null},[]),$t=p.default.useCallback(()=>{let O=U.current;O&&(O.style.height="auto",O.style.height=`${Math.min(O.scrollHeight,200)}px`)},[]);p.default.useEffect(()=>{$t()},[y,$t]),p.default.useEffect(()=>($(eh(o)),()=>De()),[o,f,De]);let Le=p.default.useRef(o),Pa=p.default.useRef(f);p.default.useEffect(()=>{if(Le.current!==o||Pa.current!==f){Le.current=o,Pa.current=f,v(ah(o)),w("");return}kc(o,g)},[o,f,g]),p.default.useEffect(()=>{s&&($(s),window.requestAnimationFrame(()=>{U.current&&(U.current.focus(),U.current.setSelectionRange(s.length,s.length))}))},[s,i]);let kt=p.default.useCallback(O=>{if(a||!O||O.length===0)return;let Q=o,ce=f;le.current=le.current.then(async()=>{let ge=o,gt=f,{staged:Ce,errors:Ct}=await D$(O,{limits:x,existing:ae.current,t:m}),on=lt.current;if(!(on.draftKey!==ge||on.storageScope!==gt||ft()!==gt)){if(Ce.length>0){let ja=[...ae.current,...Ce];ae.current=ja,kc(ge,ja),v(ja)}w(Ct.length>0?Ct.join(" "):"")}}).catch(()=>{w(m("chat.attachmentStagingFailed"))})},[a,o,x,f,m]),la=p.default.useCallback(O=>{let Q=ae.current.filter(ce=>ce.id!==O);ae.current=Q,kc(o,Q),v(Q),w("")},[o]),rn=p.default.useCallback(()=>{a||F.current?.click()},[a]),ua=p.default.useCallback(O=>{let Q=Array.from(O.target.files||[]);kt(Q),O.target.value=""},[kt]),Vt=p.default.useCallback(async()=>{let O=y.trim(),Q=g.length>0,ce=O||(Q?Rc:"");if(!(!ce||z.current)){z.current=!0,C(!0);try{if(await e(ce,{attachments:g,displayContent:O})===null)return;$(""),v([]),ae.current=[],w(""),at(),K$(o),Q$(o),U.current&&(U.current.style.height="auto")}catch{}finally{z.current=G.current,C(!1)}}},[y,g,e,o,at,a,n]),sn=p.default.useCallback(O=>{let Q=O.target.value;$(Q),ht.current={key:o,text:Q,scope:ft()},Oe.current&&window.clearTimeout(Oe.current),Oe.current=window.setTimeout(De,300)},[o,De]),vt=p.default.useCallback(async()=>{if(!(!r||R||!t)){_(!0);try{await t()}finally{_(!1)}}},[r,R,t]),ca=p.default.useCallback(O=>{if(O.key==="Enter"&&!O.shiftKey){if(O.preventDefault(),U.current?.dataset?.sendDisabled==="true"||z.current)return;Vt()}},[Vt]),_a=p.default.useCallback(O=>{let Q=Array.from(O.clipboardData?.files||[]);Q.length>0&&(O.preventDefault(),kt(Q))},[kt]),da=p.default.useCallback(O=>{O.preventDefault(),L(!1);let Q=Array.from(O.dataTransfer?.files||[]);Q.length>0&&kt(Q)},[kt]),Ua=p.default.useCallback(O=>{O.preventDefault(),!a&&L(!0)},[a]),X=p.default.useCallback(O=>{O.currentTarget.contains(O.relatedTarget)||L(!1)},[]),re=y.trim()||g.length>0,ie=a||n,N=m(h?"chat.heroPlaceholder":"chat.followUpPlaceholder"),E=x.accept.length>0?x.accept.join(","):void 0,A=h?"w-full":"px-4 py-3 sm:px-5 lg:px-8",q=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",h?"min-h-[120px]":"",a?"opacity-70":""].join(" "),B=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",h?"min-h-[72px]":"min-h-[40px]"].join(" ");return u`
    <div className=${A}>
      <div
        className=${q}
        onDrop=${da}
        onDragOver=${Ua}
        onDragLeave=${X}
      >
        ${M&&u`
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
  `}var tS={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function aS({status:e}){let t=k();if(e==="idle"||e==="connecting"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return u`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",tS[e]||tS.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function nS({onSuggestion:e,onSend:t,disabled:a,sendDisabled:n,initialText:r,resetKey:s,draftKey:i,context:o,statusText:l,canCancel:c,onCancel:d}){let m=k(),f=[{icon:"tool",title:m("chat.suggestion1"),detail:m("chat.suggestion1Desc")},{icon:"shield",title:m("chat.suggestion2"),detail:m("chat.suggestion2Desc")},{icon:"plug",title:m("chat.suggestion3"),detail:m("chat.suggestion3Desc")}];return u`
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
  `}var V4=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function rS({open:e,onClose:t}){let a=k();return e?u`
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
          ${V4.map((n,r)=>u`
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
  `:null}function iS(e){let t=0,a=0,n=0,r=0,s=0;for(let o of e){if(o.role==="thinking"&&(t+=1),o.role==="tool_activity"){let l=sS([o]);a+=l.tools,n+=l.failed,r+=l.declined,s+=l.running}if(G4(o)){let l=sS(o.toolCalls);a+=l.tools,n+=l.failed,r+=l.declined,s+=l.running}}let i=[];return t&&i.push(`${t} reasoning`),a&&i.push(`${a} ${a===1?"tool":"tools"}`),n&&i.push(`${n} failed`),r&&i.push(`${r} declined`),!n&&!r&&s&&i.push("running"),{hasError:n>0,hasDeclined:r>0,label:`Activity${i.length?` - ${i.join(", ")}`:""}`}}function sS(e){let t=0,a=0,n=0;for(let r of e)r.toolStatus==="error"&&(t+=1),r.toolStatus==="declined"&&(a+=1),r.toolStatus==="running"&&(n+=1);return{tools:e.length,failed:t,declined:a,running:n}}function G4(e){return e.toolCalls&&e.toolCalls.length>0}var oS=!1;function Y4(){oS||!window.DOMPurify||(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),oS=!0)}function lS(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}Y4();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var xh=360;function J4(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let l=s("Copy");if(l.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),l.textContent="Copied",di("Code copied",{tone:"success"}),setTimeout(()=>l.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(l),n.appendChild(r),t.scrollHeight>xh){t.style.maxHeight=`${xh}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${xh}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function X4({content:e,className:t=""}){let a=p.default.useRef(null),n=p.default.useMemo(()=>lS(e),[e]);return p.default.useEffect(()=>{J4(a.current)},[n]),u`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var sa=p.default.memo(X4);var uS={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",declined:"bg-iron-400",error:"bg-[var(--v2-danger-text)]"},W4={success:"ok",declined:"declined",error:"err",running:"run"},Z4=2;function hi({activity:e}){return e.toolCalls&&e.toolCalls.length>0?u`<${t5} tools=${e.toolCalls} />`:u`<${a5} activity=${e} />`}function e5(e,t){let a=0,n=0,r=0,s=0;for(let l of t){let c=String(l.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function t5({tools:e}){let t=k(),a=e.some(o=>o.toolStatus==="error"),n=e.some(o=>o.toolStatus==="error"||o.toolStatus==="declined"),[r,s]=p.default.useState(n);if(p.default.useEffect(()=>{n&&s(!0)},[n]),e.length<=Z4)return u`
      <div className="flex flex-col gap-3">
        ${e.map((o,l)=>u`<${hi}
            key=${o.id||o.callId||`${o.toolName}-${l}`}
            activity=${o}
          />`)}
      </div>
    `;let i=e5(t,e);return u`
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
  `}function a5({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:l}=e,[c,d]=p.default.useState(n==="error"||n==="declined");p.default.useEffect(()=>{(n==="error"||n==="declined")&&d(!0)},[n]);let m=uS[n]||uS.running,f=i!=null,h=p.default.useId(),x=u`
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
        >${W4[n]||"run"}</span
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
        ${c&&u`<${n5}
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
  `}function n5({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=k(),l=p.default.useMemo(()=>{let f=[];return r&&f.push({id:s==="declined"?"declined":"error",label:o(s==="declined"?"tool.tabDeclined":"tool.tabError")}),t&&f.push({id:"details",label:o("tool.tabDetails")}),a&&f.push({id:"params",label:o("tool.tabParameters")}),n&&f.push({id:"result",label:o("tool.tabResult")}),f},[o,r,t,a,n,s]),[c,d]=p.default.useState(null),m=c&&l.some(f=>f.id===c)?c:l[0]?.id;return p.default.useEffect(()=>{r&&d(s==="declined"?"declined":"error")},[r,s]),l.length===0?u`
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
        ${m==="result"&&u`<${r5} text=${n} />`}
        ${(m==="error"||m==="declined")&&u`<pre
          className=${["overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono",m==="declined"?"text-iron-300":"text-[var(--v2-danger-text)]"].join(" ")}
        >${r}</pre>`}
      </div>
    </div>
  `}function r5({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return u`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(s5)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return u`
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
                  >${i5(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?u`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:u`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function s5(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function i5(e){return e==null?"":String(e)}function cS({activity:e}){let t=iS(e),a=u5(e),[n,r]=p.default.useState(a);return p.default.useEffect(()=>{a&&r(!0)},[a]),u`
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
            <${o5}
              key=${s.id||`${s.role||"activity"}-${i}`}
              item=${s}
            />
          `)}
        </div>
      `}
    </div>
  `}function o5({item:e}){if(e.role==="thinking")return u`<${l5} content=${e.content} />`;if(e.role==="tool_activity"||$h(e)){let t=$h(e)?{id:e.id,toolCalls:e.toolCalls}:e;return u`<${hi} activity=${t} />`}return null}function l5({content:e}){return e?u`
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
  `:null}function $h(e){return e?.toolCalls&&e.toolCalls.length>0}function u5(e){return(e||[]).some(t=>t?.role==="thinking"||t?.toolStatus==="running"||t?.toolStatus==="error"||t?.toolStatus==="declined"?!0:$h(t)?t.toolCalls.some(a=>a?.toolStatus==="running"||a?.toolStatus==="error"||a?.toolStatus==="declined"):!1)}function Vc(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function c5({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=p.default.useState(()=>t&&e.preview_url||null);return p.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return Nc(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?u`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:u`<${D} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var dS="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",mS="px-3 py-2";function Gc({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await Aa(e.fetch_url);Vc(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),l=u`
    <${c5} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?u`<div
      className=${`${dS} ${mS} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${l}
    </div>`:u`<div className=${`${dS} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${mS} text-left transition-colors hover:bg-iron-900/80`}
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
  </div>`}var fS={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function vi({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return p.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),p.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?u`
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
        className=${J("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",fS[n]??fS.md,r)}
      >
        ${a?u`<${wh} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function wh({children:e,onClose:t,className:a=""}){return u`
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
  `}function gi({children:e,className:t=""}){return u`
    <div className=${J("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function yi({children:e,className:t=""}){return u`
    <div
      className=${J("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var pS=1e5;function Yc({attachment:e,onClose:t}){let a=!!e,[n,r]=p.default.useState("loading"),[s,i]=p.default.useState({}),o=e?A$(e.mime_type):"download";if(p.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return Aa(e.fetch_url).then(async m=>{d=URL.createObjectURL(m);let f={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")f.dataUrl=await Kp(m);else if(o==="pdf")f.frameUrl=d;else if(o==="text"){let h=await m.text();f.truncated=h.length>pS,f.text=f.truncated?h.slice(0,pS):h}if(c){URL.revokeObjectURL(d);return}i(f),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let l=e.filename||"attachment";return u`
    <${vi} open=${a} onClose=${t} size="xl">
      <${wh} onClose=${t}>
        <span className="block truncate">${l}</span>
      <//>
      <${gi} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&u`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&u`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&u`<${d5} mode=${o} view=${s} filename=${l} />`}
      <//>
      <${yi}>
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
  `}function d5({mode:e,view:t,filename:a}){switch(e){case"image":return u`<img
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
      </div>`}}var m5=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function f5(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function hS(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of f5(e).matchAll(m5)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function vS(e){return e.split("/").filter(Boolean).pop()||e}function gS(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function p5({threadId:e,path:t,onPreview:a}){let[n,r]=p.default.useState({mime_type:"",size_label:""});p.default.useEffect(()=>{let i=!0;return Z0({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:gS(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:vS(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:Sc({threadId:e,path:t})};return u`<${Gc}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function yS({threadId:e,content:t}){let a=p.default.useMemo(()=>hS(t),[t]),[n,r]=p.default.useState(null);return!e||a.length===0?null:u`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>u`<${p5}
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
  `}var bS={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function h5(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function v5({content:e}){let[t,a]=p.default.useState(!1);return e?u`
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
  `:null}function g5({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:l,status:c,error:d,toolCalls:m,timestamp:f}=e,h=n==="user",[x,y]=p.default.useState(!1),[$,g]=p.default.useState(null),v=p.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),y(!0),di("Copied to clipboard",{tone:"success"}),setTimeout(()=>y(!1),1400)}catch{}},[r]);if(n==="tool_activity"||m&&m.length>0){let L=m&&m.length>0?{id:e.id,toolCalls:m}:e;return u`<${hi} activity=${L} />`}if(n==="thinking")return u`<${v5} content=${r} />`;if(n==="image")return u`
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
    `;let b=h5(f),w=n==="user"||n==="assistant"&&!l,S=n==="system"||n==="error",C=h?"max-w-[85%]":S?"mx-auto max-w-[85%]":"w-full max-w-[85%]",R=h?"":"w-full min-w-0 max-w-full",_=c==="error"&&t,M=w||_||b;return u`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",h?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col",C].join(" ")}>
        <div
          className=${["text-base leading-7",R,bS[n]||bS.assistant,l?"opacity-70":""].join(" ")}
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

          ${n==="assistant"&&u`<${yS}
            threadId=${a}
            content=${typeof r=="string"?r:""}
          />`}
        </div>
      </div>

      ${M&&u`
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
  `}var xS=p.default.memo(g5);function RS(e){let t=y5(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(kS(r)){let s=$S(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){wS(a,s),SS(a,r),n+=s.length;continue}}if(Sh(r)){let s=$S(t,n);wS(a,s),n+=s.length-1;continue}SS(a,r)}return a}function y5(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=Jc(i);o&&kS(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!Sh(i))continue;let o=Jc(i),l=o?t.get(o):void 0;if(l===void 0||l>=s)continue;let c=a.get(l)||[];c.push(i),a.set(l,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function $S(e,t){let a=t,n=Jc(e[t]);for(;a<e.length&&Sh(e[a])&&b5(n,e[a]);)a+=1;return e.slice(t,a)}function b5(e,t){let a=Jc(t);return!e||!a||a===e}function wS(e,t){if(t.length===0)return;let a=x5(t);e.push({type:"activity-run",id:`activity-run-${a[0].id}`,activity:a})}function SS(e,t){e.push({type:"message",id:t.id,message:t})}function kS(e){return e.role==="assistant"&&!CS(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function Sh(e){return e.role==="thinking"||e.role==="tool_activity"||CS(e)}function CS(e){return e?.toolCalls&&e.toolCalls.length>0}function Jc(e){return e?.turnRunId||null}function x5(e){return[...e].sort((t,a)=>t?.role!=="tool_activity"||a?.role!=="tool_activity"?0:$5(t,a))}function $5(e,t){if(Number.isFinite(e.activityOrder)&&Number.isFinite(t.activityOrder)){let n=e.activityOrder-t.activityOrder;if(n!==0)return n}let a=NS(_S(e.updatedAt||e.timestamp),_S(t.updatedAt||t.timestamp));return a!==0?a:NS(e.sequence,t.sequence)}function NS(e,t){let a=Number.isFinite(e)?e:null,n=Number.isFinite(t)?t:null;return a===null&&n===null?0:a===null?1:n===null?-1:a-n}function _S(e){if(!e)return null;let t=Date.parse(e);return Number.isFinite(t)?t:null}var w5=100,S5=100;function N5(e){return e?e.scrollHeight-e.scrollTop-e.clientHeight:Number.POSITIVE_INFINITY}function ES(e,t=w5){return N5(e)<=t}function TS(e){e&&(e.scrollTop=Math.max(0,e.scrollHeight-e.clientHeight))}function AS(e){return e?.id?`${e.role||""}:${e.id}`:null}function _5(e,t){let a=AS(t);return!!(a&&t?.role==="user"&&a!==e)}function R5(){return typeof window>"u"||!window.getSelection?"":String(window.getSelection()?.toString?.()||"")}function DS({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let l=k(),c=p.default.useRef(null),d=p.default.useRef(null),m=p.default.useRef(!0),f=p.default.useRef(null),h=p.default.useRef(null),x=p.default.useRef(null),y=p.default.useRef(0),$=p.default.useRef(!1),[g,v]=p.default.useState(!0),b=p.default.useCallback(()=>{h.current!==null&&(window.cancelAnimationFrame(h.current),h.current=null)},[]),w=p.default.useCallback((z=!1)=>{c.current&&(z&&(m.current=!0,$.current=!1),m.current&&(b(),h.current=window.requestAnimationFrame(()=>{h.current=null;let G=c.current;!G||!z&&!m.current||(TS(G),y.current=G.scrollTop,$.current=!1,v(!0))})))},[b]),S=p.default.useCallback(()=>{x.current!==null&&(window.cancelAnimationFrame(x.current),x.current=null)},[]);p.default.useLayoutEffect(()=>{let z=e.length>0?e[e.length-1]:null,P=AS(z),G=_5(f.current,z);return f.current=P,w(G),b},[e,i,w,b]),p.default.useLayoutEffect(()=>{let z=d.current;if(!z||typeof ResizeObserver!="function")return;let P=new ResizeObserver(()=>{w()});return P.observe(z),()=>{P.disconnect(),b()}},[w,b]);let C=p.default.useCallback(()=>{x.current=null;let z=c.current;if(!z)return;let P=ES(z);y.current=z.scrollTop,P?(m.current=!0,$.current=!1,v(!0)):$.current?(m.current=!1,v(!1)):(m.current=!0,v(!0),w()),a&&z.scrollTop<S5&&n&&!t&&n()},[a,n,t,w]),R=p.default.useCallback(()=>{$.current=!0},[]),_=p.default.useCallback(z=>{let P=c.current;if(!P||typeof z?.clientX!="number")return;let G=P.offsetWidth-P.clientWidth;if(G<=0)return;let ae=P.getBoundingClientRect().right;z.clientX>=ae-G-2&&($.current=!0)},[]),M=p.default.useCallback(()=>{let z=c.current;if(!z)return;let P=ES(z),G=z.scrollTop<y.current;y.current=z.scrollTop,!P&&G&&($.current=!0),P?(m.current=!0,$.current=!1):$.current?(m.current=!1,b()):m.current=!0,x.current===null&&(x.current=window.requestAnimationFrame(C))},[b,C]),L=p.default.useCallback(()=>{let z=c.current;z&&(TS(z),y.current=z.scrollTop,m.current=!0,$.current=!1,v(!0))},[]),U=p.default.useCallback(z=>{let P=R5();!P||!z.clipboardData||(z.preventDefault(),z.clipboardData.clearData(),z.clipboardData.setData("text/plain",P))},[]);p.default.useEffect(()=>S,[S]);let F=p.default.useMemo(()=>RS(e),[e]);return u`
    <div className="relative flex min-h-0 min-w-0 flex-1">
    <div
      ref=${c}
      onScroll=${M}
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
        ${F.map(z=>z.type==="activity-run"?u`<${cS} key=${z.id} activity=${z.activity} />`:u`<${xS}
                key=${z.id}
                message=${z.message}
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
  `}function MS({notice:e,onRecover:t}){return u`
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
  `}function OS({suggestions:e,onSelect:t,disabled:a=!1}){return!e||e.length===0?null:u`
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
  `}function LS(){return u`
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
  `}function Xc(){return V("/api/webchat/v2/channels/connectable")}function PS(e,t){if(!Nh(e))return null;let a=Wc(e),n=T5(a),r=null;for(let s of t||[]){if(!E5(s))continue;let i=A5(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function Nh(e){let t=Wc(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function k5(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function C5(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>US(Wc(n))):a}function E5(e){return e?.strategy!=="admin_managed_channels"}function T5(e){return jS(e,"slack")&&US(e)}function US(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function Wc(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function A5(e,t,a={}){return(a.commandAliasesOnly?C5(t,{channelManagementOnly:!0}):k5(t)).reduce((r,s)=>{let i=Wc(s);return jS(e,i)?Math.max(r,i.length):r},0)}function jS(e,t){return t?` ${e} `.includes(` ${t} `):!1}function FS(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n={kind:"gate",gateKind:"approval",runId:t.turn_run_id,gateRef:t.gate_ref,invocationId:t.invocation_id||null,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return D5(n,a,t.body)}return e==="auth_required"?{kind:"auth_required",gateKind:"auth",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,invocationId:t.invocation_id||null,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function zS(e){if(!e?.run_id||!e.gate_ref)return null;let t=e.gate_kind||"generic",a={gateKind:t,runId:e.run_id,gateRef:e.gate_ref,invocationId:e.invocation_id||null,headline:e.headline,body:e.body||"",allowAlways:e.allow_always===!0};if(t==="auth"){let n=e.auth_context||{};return{...a,kind:"auth_required",challengeKind:n.challenge_kind||"other",provider:n.provider||null,accountLabel:n.account_label||"",authorizationUrl:n.authorization_url||null,expiresAt:n.expires_at||null}}return{...a,kind:"gate"}}function D5(e,t,a){if(!t)return e;let n=M5(t);return{...e,toolName:t.tool_name||null,description:t.reason||a,actionLabel:t.action?.label||null,destination:t.destination||null,approvalScope:t.scope||null,approvalDetails:n,parameters:n.length?n.map(r=>`${r.label}: ${r.value}`).join(`
`):null}}function M5(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function BS({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function qS(){return{terminalByInvocation:new Map}}function IS(e){e?.current?.terminalByInvocation?.clear()}function Rh(e,t,a){let n=KS(t,{toolStatus:"running"});n&&bi(e,n,a)}function HS(e,t,a,n="gate_declined"){let r=KS(t,{toolStatus:"declined",toolError:n,toolErrorKind:"gate_declined"});r&&bi(e,r,a)}function bi(e,t,a){if(!t)return;let n=F5(t);n=j5(n,a),e(r=>{let s=QS(n),i=L5(r,n,s);if(i>=0){let l=[...r];return l[i]=P5(l[i],n),_h(l[i],a),l}let o={id:s,role:"tool_activity",...n};return _h(o,a),[...r,o]})}function KS(e,t={}){let a=e?.kind==="gate",n=e?.kind==="auth_required",r=n&&t.toolStatus==="declined";if(!e?.runId||!e?.gateRef||!e.invocationId&&!r||!a&&!n)return null;let s=e.invocationId||O5(e),i=e.toolName||e.headline||e.gateKind||"gate";return{invocationId:s,callId:s,capabilityId:e.toolName||e.gateKind||null,toolName:el(i)||i,toolStatus:t.toolStatus||"running",toolDetail:null,toolParameters:null,toolResultPreview:null,toolError:t.toolError||null,toolErrorKind:t.toolErrorKind||null,toolDurationMs:null,updatedAt:t.updatedAt||new Date().toISOString(),resultRef:null,truncated:!1,outputBytes:null,outputKind:null,turnRunId:e.runId,gateRef:e.gateRef,gateActivity:!0}}function O5(e){return`gate:${e.runId}:${e.kind}:${e.gateRef}`}function QS(e){return`tool-${e.invocationId}`}function L5(e,t,a){let n=e.findIndex(s=>s?.id===a);if(n>=0)return n;let r=t.gateRef||null;if(r){let s=e.findIndex(i=>i?.role==="tool_activity"&&i.turnRunId===t.turnRunId&&i.gateRef===r);if(s>=0)return s}return-1}function P5(e,t){let a=Zo(e.toolStatus),n=Zo(t.toolStatus),r=a&&!n,s=t.gateActivity&&!e.gateActivity,i={...e,...t,id:e.id,role:"tool_activity",invocationId:e.gateActivity&&!t.gateActivity?t.invocationId:e.invocationId||t.invocationId,callId:e.gateActivity&&!t.gateActivity?t.callId:e.callId||t.callId,toolName:s?e.toolName:t.toolName||e.toolName,toolStatus:r?e.toolStatus:t.toolStatus,toolError:t.toolError||e.toolError,toolErrorKind:t.toolErrorKind||e.toolErrorKind||null,updatedAt:r?e.updatedAt||t.updatedAt:t.updatedAt||e.updatedAt,turnRunId:t.turnRunId||e.turnRunId||null,gateRef:t.gateRef||e.gateRef||null,gateActivity:e.gateActivity&&t.gateActivity,capabilityId:s?e.capabilityId||t.capabilityId||null:t.capabilityId||e.capabilityId||null,activityOrder:U5(e,t),activityOrderSource:t.activityOrderSource||e.activityOrderSource||null};return e.gateActivity&&!t.gateActivity&&(i.id=QS(t),i.gateActivity=!1),i}function U5(e,t){return Number.isFinite(t.activityOrder)?t.activityOrder:e.activityOrder}function j5(e,t){if(!e?.invocationId)return e;if(Zo(e.toolStatus))return _h(e,t),e;let a=t?.current?.terminalByInvocation?.get(e.invocationId);return a?Number.isFinite(e.activityOrder)?{...a,activityOrder:e.activityOrder,activityOrderSource:e.activityOrderSource||a.activityOrderSource||null}:a:e}function _h(e,t){!e?.invocationId||!Zo(e.toolStatus)||t?.current?.terminalByInvocation?.set(e.invocationId,e)}function F5(e){let t=el(e.toolName||e.capabilityId);return{...e,toolName:t||e.toolName||"tool"}}function XS({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o,onRunSettled:l}){let c=p.default.useRef(new Set),d=p.default.useRef(null),m=p.default.useRef(null);return p.default.useCallback(f=>{let{type:h,frame:x}=f||{};if(!(!h||!x))switch(h){case"accepted":{let y=x.ack||{};y.run_id&&(d.current=y.run_id),r?.({runId:y.run_id||null,threadId:y.thread_id||e,status:y.status||null}),a(!0);return}case"running":case"capability_progress":{let y=x.progress||{};y.turn_run_id&&(d.current=y.turn_run_id,r?.($=>$&&$.runId===y.turn_run_id?{...$,status:"running"}:{runId:y.turn_run_id,threadId:e,status:"running"}),z5(n,y.turn_run_id,m)),a(!0);return}case"capability_activity":{let y=x.activity;if(!y||!y.invocation_id)return;bi(t,Wp(y),o);return}case"capability_display_preview":{let y=x.preview;if(!y||!y.invocation_id)return;let $=Xp(y);bi(t,$,o);return}case"gate":case"auth_required":{let y=FS(h,x.prompt);y&&(Rh(t,y,o),n(y),r?.({runId:y.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let y=x.reply||{};t($=>[...$,{id:`reply-${y.turn_run_id||Date.now()}`,role:"assistant",content:y.text||"",timestamp:y.generated_at||new Date().toISOString(),turnRunId:y.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let y=x.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),td(c,l,y,!1);return}case"failed":{let y=x.run_state||{},$=y.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Ch(t,{runId:$,status:y.status||"failed",failureCategory:H5(y),failureSummary:null}),td(c,l,$,!1);return}case"projection_snapshot":case"projection_update":{let y=x.state?.items||[];q5({items:y,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:l,settledRunsRef:c,latestRunIdRef:d,promptRunIdRef:m,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i,o,l])}function td(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var VS=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),GS=new Set(["completed","succeeded"]),Zc=new Set(["blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]),ed=new Set(["awaiting_gate","blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]);function YS(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function z5(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function B5(e,t,a,n,r,s){let i=t?.runId||null;if(!i||r?.has(i))return!0;let o=a?.get(i);if(o)return!ed.has(o);let l=e?.current,c=l?.runId||n?.current||null;if(c&&i!==c)return!0;let d=s?.current===c;return c&&i===c&&!d&&l?.status&&!ed.has(l.status)?!0:!l?.runId||!l.status?!1:!ed.has(l.status)}function q5({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:l,promptRunIdRef:c,activeRunRef:d,locallyResolvedGatesRef:m,toolActivityStateRef:f}){let h=new Map,x=new Set,y=d?.current||null,$=y?.runId||l?.current||null;for(let v of e){let b=v.run_status;b?.run_id&&b.status&&(h.set(b.run_id,b.status),$&&$!==b.run_id&&y?.status&&!VS.has(y.status)&&Zc.has(b.status)&&x.add(b.run_id))}let g=l?.current??null;for(let v of e){if(v.run_status){let{run_id:b,status:w,failure_category:S,failure_summary:C}=v.run_status,R=VS.has(w),_=d?.current?.source==="local"?d.current.runId:null,M=!!(b&&_&&_!==b),L=g??l?.current??null,U=!!(R&&b&&L&&L!==b),F=b&&Zc.has(w)?JS(m,b):null;if(b&&x.has(b)||M)continue;if(U){JS(m,d?.current?.runId)?.outcome==="resumed"&&(I5({runId:b,activePromptRunId:d?.current?.runId,success:GS.has(w),status:w,failureCategory:S,failureSummary:C,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:l,promptRunIdRef:c,locallyResolvedGatesRef:m}),g=null);continue}if(F){YS(r,b,c),F.outcome==="resumed"?(n(!0),s?.(z=>z&&z.runId===b?{...z,status:z.status==="awaiting_gate"?"queued":z.status||"queued"}:{runId:b,threadId:t,status:"queued"}),g=b,l&&(l.current=b)):(n(!1),d?.current?.runId===b&&s?.(null),g=null,l?.current===b&&(l.current=null));continue}b&&(g=b,!R&&l&&(l.current=b),s?.(z=>z&&z.runId===b?{...z,status:w}:{runId:b,threadId:t,status:w})),b&&Zc.has(w)?c&&(c.current=b):b&&c?.current===b&&(c.current=null),R?(n(!1),r(null),s?.(null),kh(m,b),g=null,l&&(l.current=null),b&&c?.current===b&&(c.current=null),td(o,i,b,GS.has(w)),(w==="failed"||w==="recovery_required")&&Ch(a,{runId:b,status:w,failureCategory:S,failureSummary:C})):Zc.has(w)||(YS(r,b,c),kh(m,b),n(!0))}if(v.text){let b=`text-${v.text.id}`;a(w=>{let S=v.text.id?`msg-${v.text.id}`:null,C=w.findIndex(_=>_.id===b||S&&_.id===S),R={...C>=0?w[C]:{},id:b,role:"assistant",content:v.text.body||"",timestamp:w[C]?.timestamp||new Date().toISOString(),isFinalReply:!0};if(C>=0){let _=[...w];return _[C]=R,_}return[...w,R]}),n(!1)}if(v.thinking){let b=`thinking-${v.thinking.id}`;a(w=>{let S=w.findIndex(R=>R.id===b),C={id:b,role:"thinking",content:v.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:v.thinking.run_id||null};if(S>=0){let R=[...w];return R[S]=C,R}return[...w,C]})}if(v.capability_activity){let b=v.capability_activity;b.invocation_id&&bi(a,Wp(b),f)}if(v.gate){let b=zS(v.gate),w=b?.runId||null;w&&!B5(d,b,h,l,x,c)&&!Q5(m,w,b.gateRef)&&(Rh(a,b,f),r(S=>S||b),s?.(S=>S&&S.runId===w?{...S,status:ed.has(S.status)?S.status:"awaiting_gate"}:{runId:w,threadId:t,status:"awaiting_gate"}),c&&(c.current=w),n(!1))}if(v.skill_activation){let{id:b,skill_names:w=[],feedback:S=[]}=v.skill_activation;if(w.length||S.length){let C=`skill-${b||w.join("-")||"activation"}`,R=[w.length?`Skill activated: ${w.join(", ")}`:"",...S].filter(Boolean).join(`
`);a(_=>_.some(M=>M.id===C)?_:[..._,{id:C,role:"system",content:R,timestamp:new Date().toISOString()}])}}}l&&g&&(l.current=g)}function I5({runId:e,activePromptRunId:t,success:a,status:n,failureCategory:r,failureSummary:s,setMessages:i,setIsProcessing:o,setPendingGate:l,setActiveRun:c,onRunSettled:d,settledRunsRef:m,latestRunIdRef:f,promptRunIdRef:h,locallyResolvedGatesRef:x}){o(!1),l(null),c?.(null),kh(x,t),f&&(f.current=null),h?.current===t&&(h.current=null),td(m,d,e,a),(n==="failed"||n==="recovery_required")&&Ch(i,{runId:e,status:n,failureCategory:r,failureSummary:s})}function H5(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function Ch(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`;e(i=>{let o=i.findIndex(c=>c.id===s),l=BS({status:a,failureCategory:n,failureSummary:r});if(o>=0){if(!!!(r||n)||i[o].content===l)return i;let d=[...i];return d[o]={...d[o],content:l},d}return[...i,{id:s,role:"error",content:l,timestamp:new Date().toISOString()}]})}function JS(e,t){if(!t)return null;let a=e?.current;if(!a)return null;for(let[n,r]of a.entries())if(n.startsWith(`${t}
`))return K5(r);return null}function K5(e){return e&&typeof e=="object"?{resolution:e.resolution||null,outcome:e.outcome||null}:{resolution:e||null,outcome:null}}function kh(e,t){if(!t)return;let a=e?.current;if(a)for(let n of Array.from(a.keys()))n.startsWith(`${t}
`)&&a.delete(n)}function Q5(e,t,a){return!t||!a?!1:!!e?.current?.has(`${t}
${a}`)}function WS(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function ZS(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function e2(e,t,a,n){let r=Eh(n);return r?(V5(e,t,a,{timelineMessageId:r}),r):null}function V5(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function Eh(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var G5=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function t2({threadId:e,onEvent:t,enabled:a}){let[n,r]=p.default.useState("idle"),s=p.default.useRef(t);s.current=t;let i=p.default.useRef(null);return p.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,l=null,c=0,d=3e4;function m(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=p$({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let y=Math.min(1e3*2**c,d);l=setTimeout(m,y)};let x=(y,$)=>{let g=null;try{g=JSON.parse(y.data)}catch{return}!g||typeof g!="object"||(y.lastEventId&&(i.current=y.lastEventId),s.current?.({type:g.type||$,frame:g,lastEventId:y.lastEventId||null}))};o.onmessage=y=>x(y,"message");for(let y of G5)o.addEventListener(y,$=>x($,y))}function f(){l&&(clearTimeout(l),l=null),o&&(o.close(),o=null),r("paused")}function h(){document.visibilityState==="hidden"?f():o||m()}return m(),document.addEventListener("visibilitychange",h),()=>{document.removeEventListener("visibilitychange",h),l&&clearTimeout(l),o&&o.close()}},[a,e]),{status:n}}var Y5=3e4,J5="credential_stored_gate_resolution_failed",X5="approval_gate_pending_send_blocked",W5="ironclaw-product-auth",Th="ironclaw:product-auth:oauth-complete",Z5="ironclaw:product-auth:oauth-complete";async function a2(e){let t=new AbortController,a=setTimeout(()=>t.abort(),Y5);try{return await e(t.signal)}finally{clearTimeout(a)}}function eD(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=J5,t.cause=e,t}function tD(){let e=new Error("Resolve the approval request before sending another message.");return e.safeErrorCode=X5,e}function aD(e){let a=Dt.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function n2(e,t){return!e||!t?.runId||!t?.gateRef?null:`${e}
${t.runId}
${t.gateRef}`}function nD(e){return e?.continuation?.type==="turn_gate_resume"}function rD(e){if(e?.outcome)return e.outcome;let t=String(e?.status||"").toLowerCase();return t==="queued"||t==="running"?"resumed":t==="cancelled"||e?.already_terminal===!0?"cancelled":e?.already_terminal===!1?"resumed":null}function r2(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function sD(e){return e?.type===Z5&&e?.status==="completed"}function iD(e,t,a){if(!sD(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function Ah(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function oD(e){if(!Nh(e))return null;try{let a=(await Dt.fetchQuery({queryKey:["connectable-channels"],queryFn:Xc}))?.channels||[];return PS(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function s2(e){let t=p.default.useRef(e),a=p.default.useRef(new Map),n=p.default.useRef(1),[r,s]=p.default.useState(0),[i,o]=p.default.useState(Date.now()),[l,c]=p.default.useState(null),d=p.default.useRef(l),m=p.default.useCallback(X=>{let re=typeof X=="function"?X(d.current):X;d.current=re,c(re)},[]);p.default.useEffect(()=>{d.current=l},[l]);let[f,h]=p.default.useState(null),x=p.default.useCallback(()=>a.current.get(e||"__new__")||[],[e]),y=p.default.useCallback(X=>{let re=e||"__new__";X.length>0?a.current.set(re,X):a.current.delete(re)},[e]),{messages:$,hasMore:g,nextCursor:v,isLoading:b,loadError:w,loadHistory:S,seedThreadMessages:C,setMessages:R}=q$(e,{getPendingMessages:x,setPendingMessages:y}),[_,M]=p.default.useState(!1),L=p.default.useRef(_),U=p.default.useCallback(X=>{let re=typeof X=="function"?X(L.current):X;L.current=re,M(re)},[]),[F,z]=p.default.useState(null),P=p.default.useRef(F),[G,ae]=p.default.useState(null),le=p.default.useCallback(X=>{let re=P.current,ie=typeof X=="function"?X(re):X;Object.is(ie,re)||(P.current=ie,z(ie))},[]),[lt,ht]=p.default.useState(e),Oe=p.default.useRef(qS()),De=p.default.useRef(new Map),at=p.default.useRef({gateKey:null,credentialRef:null,inFlight:!1}),$t=p.default.useRef(!1),Le=p.default.useRef(null);lt!==e&&(ht(e),M(!1),z(null),ae(null),c(null),h(null)),p.default.useEffect(()=>{t.current=e},[e]),p.default.useEffect(()=>()=>{Le.current?.threadId===e&&(Le.current=null)},[e]),p.default.useEffect(()=>{P.current=F},[F]),p.default.useEffect(()=>{L.current=_},[_]),p.default.useEffect(()=>{let X=n2(e,F);ae(re=>re&&re.gateKey!==X?null:re)},[F,e]),p.default.useEffect(()=>{IS(Oe),De.current.clear()},[e]);let Pa=Math.max(0,Math.ceil((r-i)/1e3)),kt=F?.runId&&F?.gateRef?`${F.runId}
${F.gateRef}`:null;p.default.useEffect(()=>{if(!r)return;let X=setInterval(()=>o(Date.now()),250);return()=>clearInterval(X)},[r]),p.default.useEffect(()=>{at.current.gateKey!==kt&&(at.current={gateKey:kt,credentialRef:null,inFlight:!1})},[kt]),p.default.useEffect(()=>{if(!r2(F))return;let X=Date.now(),re=A=>{iD(A,F,X)&&(le(q=>r2(q)?null:q),U(!0))},ie=null;typeof window.BroadcastChannel=="function"&&(ie=new window.BroadcastChannel(W5),ie.onmessage=A=>re(A.data));let N=A=>{A.key===Th&&re(Ah(A.newValue))};window.addEventListener("storage",N),re(Ah(window.localStorage?.getItem?.(Th)));let E=window.setInterval(()=>{re(Ah(window.localStorage?.getItem?.(Th)))},500);return()=>{window.clearInterval(E),ie&&ie.close(),window.removeEventListener("storage",N)}},[F]);let la=XS({threadId:e,setMessages:R,setIsProcessing:U,setPendingGate:le,setActiveRun:m,activeRunRef:d,locallyResolvedGatesRef:De,toolActivityStateRef:Oe,onRunSettled:(X,{success:re})=>{let ie=Le.current;ie?.runId===X?Le.current=null:X&&ie&&!ie.runId&&(Le.current={...ie,runId:X,settledBeforeResponse:!0}),re&&y([]),S(void 0,{preserveClientOnly:!0,finalReplyTimestampByRun:X&&re?{[X]:new Date().toISOString()}:null})}}),{status:rn}=t2({threadId:e,onEvent:la,enabled:!!e}),ua=p.default.useCallback(async(X,re={})=>{let{threadId:ie,attachments:N=[],displayContent:E}=re,A=N.map(M$),q=N.map(O$),B=typeof E=="string"?E:X;if(F||P.current)throw tD();let O=ie||e,Q=d.current,ce=!!Q&&!!O&&Q.threadId===O,ge=L.current&&!!O&&O===e,gt=!!O&&Le.current?.threadId===O;if($t.current||ge||ce||gt)return null;if(N.length===0){let oe=await oD(X);if(oe)return h(oe),{channel_connect_action:oe}}h(null);let Ce=ie||e;if(!Ce){let oe=await wc();if(Dt.invalidateQueries({queryKey:["threads"]}),Ce=oe?.thread?.thread_id,!Ce)throw new Error("createThread returned no thread_id")}let Ct=Ce,on={id:`pending-${n.current++}`,role:"user",content:B,attachments:q,retryContent:X,retryDisplayContent:B,retryAttachments:N,timestamp:new Date().toISOString(),isOptimistic:!0},ja={id:on.id,role:"user",content:B,attachments:q,retryContent:X,retryDisplayContent:B,retryAttachments:N,timestamp:on.timestamp,isOptimistic:!0};WS(a.current,Ct,on);let Fa=on.id,gr=!e||Ce===e,yr=oe=>{gr&&R(oe)},Zr=oe=>{Ce!==e&&C(Ce,oe)},es=oe=>{gr&&oe()},ts=gr;ts&&(Le.current={threadId:Ce,runId:null,settledBeforeResponse:!1}),$t.current=!0,yr(oe=>[...oe,ja]),Zr(oe=>[...oe,ja]),es(()=>{U(!0),P.current||le(null)});try{let oe=await d$({threadId:Ce,content:X,attachments:A});aD(Ce)&&Dt.invalidateQueries({queryKey:["threads"]});let as=!1;if(oe?.run_id&&ts){let Lt=Le.current;as=!!(Lt&&Lt.threadId===Ce&&Lt.runId===oe.run_id&&Lt.settledBeforeResponse),as?Le.current=null:Le.current={threadId:Ce,runId:oe.run_id,settledBeforeResponse:!1}}else ts&&(Le.current=null);oe?.run_id&&gr&&!as&&m({runId:oe.run_id,threadId:oe.thread_id||Ce,status:oe.status||null,source:"local"});let xl=e2(a.current,Ct,Fa,oe?.accepted_message_ref)||Eh(oe?.accepted_message_ref);if(xl){let Lt=ns=>ns.map(An=>An.id===Fa?{...An,timelineMessageId:xl}:An);yr(Lt),Zr(Lt)}if(oe?.outcome==="rejected_busy"){ts&&(Le.current=null);let Lt=ns=>ns.map(An=>An.id===Fa?{...An,isOptimistic:!1,status:"error"}:An);if(yr(Lt),Zr(Lt),oe?.notice){let ns=(Mi=gr)=>{let Mk={id:`system-rejected-${n.current++}`,role:"system",content:oe.notice,timestamp:new Date().toISOString(),isOptimistic:!1},iv=Ok=>[...Ok,Mk];Mi&&R(iv),(!Mi||Ce!==e)&&C(Ce,iv)};if(!t.current||t.current===Ce){let Mi=n2(Ce,P.current);Mi?ae({gateKey:Mi,content:oe.notice}):ns()}else ns(!1)}es(()=>U(!1)),$t.current=!1}else oe?.run_id||(ts&&(Le.current=null),$t.current=!1);return oe}catch(oe){ts&&(Le.current=null),oe.status===429&&s(Date.now()+uD(oe));let as=xl=>xl.map(Lt=>Lt.id===Fa?{...Lt,isOptimistic:!1,status:"error",error:oe.message}:Lt);throw yr(as),Zr(as),es(()=>U(!1)),$t.current=!1,oe&&typeof oe=="object"&&(oe.optimisticMessageId=Fa,oe.optimisticThreadId=Ce),oe}finally{$t.current=!1,ZS(a.current,Ct,Fa)}},[e,F,R,C,U,le,m]),Vt=p.default.useCallback(async(X,re={})=>{if(!F)return;let{runId:ie,gateRef:N}=F;if(!ie||!N)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");let E=await Qp({threadId:e,runId:ie,gateRef:N,resolution:X,always:re.always,credentialRef:re.credentialRef}),A=rD(E);if(De.current.set(`${ie}
${N}`,{resolution:X,outcome:A}),lD(X)&&A==="resumed"&&HS(R,F,Oe),le(null),A==="resumed"){U(!0),m({runId:E?.run_id||ie,threadId:E?.thread_id||e,status:E?.status||"queued"});return}U(!1),m(null)},[F,e,R,m]),sn=p.default.useCallback(async X=>{if(!F)throw new Error("auth gate is no longer pending");let{runId:re,gateRef:ie,provider:N}=F;if(!re||!ie||!N)throw new Error("auth gate is missing required credential metadata");let E=F.accountLabel||`${N} credential`,A=`${re}
${ie}`;if(at.current.gateKey!==A&&(at.current={gateKey:A,credentialRef:null,inFlight:!1}),at.current.inFlight)throw new Error("auth token submission already in progress");at.current.inFlight=!0;try{let q=at.current.credentialRef,B=null;if(!q){if(B=await a2(O=>v$({provider:N,accountLabel:E,token:X,threadId:e,runId:re,gateRef:ie,signal:O})),q=B?.credential_ref,!q)throw new Error("manual token submit returned no credential_ref");at.current.credentialRef=q}if(!nD(B))try{await a2(O=>Qp({threadId:e,runId:re,gateRef:ie,resolution:"credential_provided",credentialRef:q,signal:O}))}catch(O){throw eD(O)}at.current={gateKey:null,credentialRef:null,inFlight:!1},le(null),U(!0)}catch(q){throw at.current.gateKey===A&&(at.current.inFlight=!1),q}},[F,e]),vt=p.default.useCallback(async X=>{let re=l?.runId;if(!re||!e)return;le(null),U(!1),m(null),$t.current=!1;let ie=Le.current;(ie?.runId===re||ie?.threadId===e)&&(Le.current=null),await h$({threadId:e,runId:re,reason:X})},[l,e]),ca=p.default.useCallback(()=>{g&&v&&S(v)},[g,v,S]),_a=p.default.useCallback(async(X,re,ie)=>{let N="approved",E=!1;re==="deny"?N="denied":re==="cancel"?N="cancelled":re==="always"&&(N="approved",E=!0),await Vt(N,{always:E})},[Vt]),da=p.default.useCallback(()=>{},[]),Ua=p.default.useCallback(async X=>{if(!X||X.status!=="error")return;let re=typeof X.retryContent=="string"?X.retryContent:typeof X.content=="string"?X.content:"",ie=Array.isArray(X.retryAttachments)?X.retryAttachments:[];if(!re&&ie.length===0)return;let N=A=>A.filter(q=>q.id!==X.id),E=A=>A.some(B=>B.id!==X.id&&B.role==="user"&&B.status==="error"&&B.retryContent===re)||A.some(B=>B.id===X.id)?A:[...A,X];R(N),e&&C(e,N);try{await ua(re,{threadId:e,attachments:ie,displayContent:typeof X.retryDisplayContent=="string"?X.retryDisplayContent:X.content})===null&&(R(E),e&&C(e,E))}catch(A){if(A?.optimisticMessageId){R(N),e&&C(e,N);return}R(E),e&&C(e,E)}},[ua,C,R,e]);return{messages:$,isProcessing:_,pendingGate:F,busyGateNotice:G,channelConnectAction:f,activeRun:l,sseStatus:rn,historyLoading:b,historyLoadError:w,hasMore:g,cooldownSeconds:Pa,send:ua,resolveGate:Vt,submitAuthToken:sn,cancelRun:vt,loadMore:ca,dismissChannelConnectAction:()=>h(null),suggestions:[],setSuggestions:da,retryMessage:Ua,approve:_a,recoverHistory:da,recoveryNotice:null}}function lD(e){return e==="denied"||e==="cancelled"}function uD(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function i2({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}function cD(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function ad({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let l=o.toString(),c=`/logs${l?`?${l}`:""}`;return i?`/v2${c}`:c}function o2(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(cD),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}var dD=1500;function l2({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let o=k(),{messages:l,isProcessing:c,pendingGate:d,busyGateNotice:m,channelConnectAction:f,suggestions:h,sseStatus:x,historyLoading:y,historyLoadError:$,hasMore:g,cooldownSeconds:v,recoveryNotice:b,activeRun:w,send:S,cancelRun:C,retryMessage:R,approve:_,recoverHistory:M,loadMore:L,setSuggestions:U,submitAuthToken:F,dismissChannelConnectAction:z}=s2(t),P=p.default.useMemo(()=>e.find(vt=>vt.id===t)||null,[e,t]),G=p.default.useMemo(()=>i2({gatewayStatus:i,activeThread:P}),[i,P]),ae=!!t&&!!d,le=!!t&&c,lt=l.length>0||le||ae||!!f,ht=!y&&!lt&&!$,Oe=ae?"Resolve the approval request before sending another message.":"",De=ae||le&&!ae||v>0,at=p.default.useRef(De);at.current=De;let $t=Oe||(v>0?`Retry in ${v}s`:void 0),Le=t||nl,Pa=!!(t&&w?.runId&&w.threadId===t&&le&&!ae),kt=t&&w?.runId&&w.threadId===t?ad({threadId:t,runId:w.runId},{absolute:!0}):null,la=p.default.useCallback(async(vt,{images:ca=[],attachments:_a=[],displayContent:da}={})=>{if(ae)throw new Error(Oe);if(at.current)return null;let Ua=await S(vt,{images:ca,attachments:_a,displayContent:da,threadId:t}),X=Ua?.thread_id||t;return!t&&X&&a&&a(X,{replace:!0}),Ua},[t,ae,Oe,De,a,S]),rn=p.default.useCallback(async vt=>{De||(U([]),await la(vt))},[De,la,U]),ua=p.default.useCallback(()=>C("user_requested"),[C]);p.default.useEffect(()=>{if(!t)return;if(d){Oc(t,Na.NEEDS_ATTENTION);return}if(c){Oc(t,Na.RUNNING);return}let vt=setTimeout(()=>Ww(t),dD);return()=>clearTimeout(vt)},[t,d,c]);let[Vt,sn]=p.default.useState(!1);return p.default.useEffect(()=>{let vt=ca=>{if(ca.key==="Escape"){sn(!1);return}if(ca.key!=="?")return;let _a=ca.target,da=_a?.tagName;da==="INPUT"||da==="TEXTAREA"||_a?.isContentEditable||(ca.preventDefault(),sn(Ua=>!Ua))};return window.addEventListener("keydown",vt),()=>window.removeEventListener("keydown",vt)},[]),u`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${aS} status=${x} />

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
          <${nS}
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
          <${DS}
            messages=${l}
            isLoading=${y}
            hasMore=${g}
            onLoadMore=${L}
            onRetryMessage=${R}
            threadId=${t}
            pending=${le}
          >
            ${b&&u`
              <${MS}
                notice=${b}
                onRecover=${M}
              />
            `}
            ${le&&!ae&&u`<${LS} />`}
            ${f&&u`
              <${Z1}
                connectAction=${f}
                onDismiss=${z}
              />
            `}
            ${d&&(d.kind==="auth_required"?d.challengeKind==="oauth_url"?u`
                  <${Y1}
                    gate=${d}
                    onCancel=${()=>_(d.requestId,"cancel",d.kind)}
                  />
                `:d.challengeKind==="manual_token"?u`
                  <${J1}
                    gate=${d}
                    onSubmit=${F}
                    onCancel=${()=>_(d.requestId,"cancel",d.kind)}
                  />
                `:u`
                  <${G1}
                    gate=${d}
                    onCancel=${()=>_(d.requestId,"cancel",d.kind)}
                  />
                `:u`
              <${V1}
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

          <${OS}
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
            context=${G}
            statusText=${$t}
            canCancel=${Pa}
            onCancel=${ua}
          />
        `}
      </div>
      <${rS}
        open=${Vt}
        onClose=${()=>sn(!1)}
      />
    </div>
  `}function Dh(){let{threadsState:e,gatewayStatus:t}=wa(),{threadId:a}=it(),n=ve(),r=Ae(),s=r.state?.composerDraft||"",i=a||null;p.default.useEffect(()=>{i&&i!==e.activeThreadId?e.setActiveThreadId(i):i||e.setActiveThreadId(null)},[i]);let o=p.default.useCallback((l,c={})=>{if(!l){e.setActiveThreadId(null),n("/chat",c);return}e.setActiveThreadId(l),n(`/chat/${l}`,c)},[e,n]);return u`
    <${l2}
      threads=${e.threads}
      activeThreadId=${i}
      onSelectThread=${o}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function u2(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?ui(e,t):"",model:e?Dc(e,t):""}}function c2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:l}){let[c,d]=p.default.useState(()=>u2(e,a)),[m,f]=p.default.useState(""),[h,x]=p.default.useState([]),[y,$]=p.default.useState(null),[g,v]=p.default.useState(""),b=p.default.useRef(!!e);p.default.useEffect(()=>{n&&(d(u2(e,a)),f(""),x([]),$(null),v(""),b.current=!!e)},[n,e,a]);let w=e?.builtin===!0,S=e&&!e.builtin,C=p.default.useCallback((U,F)=>{d(z=>{let P={...z,[U]:F};return U==="name"&&!b.current&&(P.id=Cw(F)),P})},[]),R=p.default.useCallback(()=>!w&&(!c.name.trim()||!c.id.trim())?l("llm.fieldsRequired"):!w&&!Ew(c.id.trim())?l("llm.invalidId"):!S&&!w&&t.includes(c.id.trim())?l("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,w,S,l]),_=p.default.useCallback(async()=>{let U=R();if(U){$({tone:"error",text:U});return}v("save");try{await s({form:c,apiKey:m,provider:e}),r()}catch(F){$({tone:"error",text:F.message})}finally{v("")}},[m,c,r,s,e,R]),M=p.default.useCallback(async()=>{if(!c.model.trim()){$({tone:"error",text:l("llm.modelRequired")});return}v("test");try{let U=await i(ih(e,c,m,a));$({tone:U.ok?"success":"error",text:U.message})}catch(U){$({tone:"error",text:U.message})}finally{v("")}},[m,a,c,i,e,l]),L=p.default.useCallback(async()=>{if((w?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){$({tone:"error",text:l("llm.baseUrlRequired")});return}v("models");try{let F=await o(ih(e,c,m,a));if(!F.ok||!Array.isArray(F.models)||!F.models.length)$({tone:"error",text:F.message||l("llm.modelsFetchFailed")});else{x(F.models);let z=Tw(c.model,F.models);z!==null&&C("model",z),$({tone:"success",text:l("llm.modelsFetched",{count:F.models.length})})}}catch(F){$({tone:"error",text:F.message})}finally{v("")}},[m,a,c,w,o,e,l,C]);return{form:c,apiKey:m,models:h,message:y,busy:g,isBuiltin:w,isEditing:S,setApiKey:f,update:C,submit:_,runTest:M,fetchModels:L,markIdEdited:()=>{b.current=!0}}}function nd({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let l=k(),c=c2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:l});if(!n)return null;let{form:d,apiKey:m,models:f,message:h,busy:x,isBuiltin:y,isEditing:$}=c,g=y?l("llm.configureProvider",{name:e.name||e.id}):l($?"llm.editProvider":"llm.newProvider");return u`
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
            <${bh} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${sh.map(v=>u`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
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
            <${T} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${x!==""} onClick=${c.fetchModels}>
              ${l(x==="models"?"llm.fetchingModels":"llm.fetchModels")}
            <//>
          </div>
        </label>

        ${f.length>0&&u`
          <${bh} value=${d.model} onChange=${v=>c.update("model",v.target.value)}>
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
        <${T} type="button" variant="secondary" disabled=${x!==""} onClick=${c.runTest}>
          ${l(x==="test"?"llm.testing":"llm.testConnection")}
        <//>
        <${T} type="button" variant="ghost" disabled=${x!==""} onClick=${r}>${l("common.cancel")}<//>
        <${T} type="button" disabled=${x!==""} onClick=${c.submit}>
          ${l(x==="save"?"common.saving":"common.save")}
        <//>
      <//>
    <//>
  `}function rd({login:e}){let t=k(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return u`
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
  `}function mD(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function sd({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=ci({settings:e,gatewayStatus:t}),[s,i]=p.default.useState(null),[o,l]=p.default.useState(!1),[c,d]=p.default.useState(null),m=p.default.useRef(null),f=p.default.useCallback((g,v)=>{m.current&&window.clearTimeout(m.current),d({tone:g,text:v}),m.current=window.setTimeout(()=>d(null),3500)},[]);p.default.useEffect(()=>()=>{m.current&&window.clearTimeout(m.current)},[]);let h=p.default.useCallback((g=null)=>{i(g),l(!0)},[]),x=p.default.useCallback(async g=>{try{await r.setActiveProvider(g),f("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(h(g),f("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):f("error",v.message)}},[h,r,f,n]),y=p.default.useCallback(async({form:g,apiKey:v,provider:b})=>{if(b?.builtin){await r.saveBuiltinProvider({provider:b,form:g,apiKey:v}),f("success",n("llm.providerConfigured",{name:b.name||b.id}));return}let w=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:b});f("success",n(b?"llm.providerUpdated":"llm.providerAdded",{name:w.name||w.id}))},[r,f,n]),$=p.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),f("success",n("llm.providerDeleted"))}catch(v){f("error",v.message)}},[r,f,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>mD(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:h,closeDialog:()=>l(!1),handleUse:x,handleSave:y,handleDelete:$}}var fD=3e5;function pD(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function hD(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function vD(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=l=>{let c=l.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},fD);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var gD=3e5,yD=9e5,bD=2e3;async function d2(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,bD)),(await Ac().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function id({onSuccess:e}={}){let t=k(),a=Z(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),[o,l]=p.default.useState(!1),[c,d]=p.default.useState(""),[m,f]=p.default.useState(null),h=p.default.useCallback(()=>{i(""),d(""),f(null)},[]),x=p.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),y=p.default.useCallback(async v=>{if(h(),pD()){i(t("onboarding.nearaiLocalSso"));return}let b=window.open("about:blank","_blank");if(!b){i(t("onboarding.nearaiFailed"));return}try{b.opener=null}catch{}r(!0);try{let{auth_url:w}=await iw({provider:v,origin:window.location.origin});b.location.href=w;let S=await d2("nearai",gD,b);if(S==="active"){await x();return}b.close(),i(t(S==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{b.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,h,t]),$=p.default.useCallback(async()=>{h(),r(!0);try{let v=hD(),b=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!b){i(t("onboarding.nearaiFailed"));return}b.opener=null;let w=await vD(b,v);if(!w){i(t("onboarding.nearaiFailed"));return}await ow({account_id:w.accountId,public_key:w.publicKey,signature:w.signature,message:w.message,recipient:w.recipient,nonce:w.nonce}),await x()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,h,t]),g=p.default.useCallback(async()=>{h();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}l(!0);try{let{user_code:b,verification_uri:w}=await lw();f({userCode:b,verificationUri:w}),v&&(v.location.href=w);let S=await d2("openai_codex",yD,v);if(S==="active"){await x();return}v&&v.close(),d(t(S==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{l(!1)}},[x,h,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:m,startNearai:y,startNearaiWallet:$,startCodex:g}}var m2="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",xD="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",$D="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",wD="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",SD={nearai:{color:"#00ec97",path:xD},openai_codex:{color:"#10a37f",path:m2},openai:{color:"#10a37f",path:m2},anthropic:{color:"#d97757",path:$D},ollama:{color:null,path:wD}};function f2({id:e,name:t}){let a=SD[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return u`
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
  `}var ND=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function _D({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=p.default.useState(!1),o=p.default.useRef(null),l=t||a.nearaiBusy;p.default.useEffect(()=>{if(!s)return;let d=f=>{o.current&&!o.current.contains(f.target)&&i(!1)},m=f=>{f.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",m),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",m)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return u`
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
  `}function RD({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let l=s(e.nameKey),c;return e.auth==="nearai"?c=u`<${_D} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=u`
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
        <${f2} id=${e.id} name=${l} />
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
  `}function p2(){let{isAdmin:e=!1,isChecking:t=!1}=wa();return t?null:e?u`<${kD} />`:u`<${ot} to="/chat" replace />`}function kD(){let e=k(),t=ve(),a=Z(),{gatewayStatus:n}=wa(),r=sd({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=ND.map(m=>({entry:m,provider:s.providers.find(f=>f.id===m.id)})).filter(m=>m.provider),o=p.default.useCallback(()=>t("/chat"),[t]),l=id({onSuccess:o}),c=p.default.useCallback(async m=>{let f=m.active_model||m.default_model||"";await sl({provider_id:m.id,model:f}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=p.default.useCallback(async({form:m,apiKey:f,provider:h})=>{await r.handleSave({form:m,apiKey:f,provider:h});let x=h?.id||m.id.trim(),y=m.model?.trim()||h?.default_model||"";await sl({provider_id:x,model:y}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?u`
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
              <${RD}
                key=${m.id}
                entry=${m}
                provider=${f}
                configured=${Qr(f,s.builtinOverrides)}
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
  `}function h2({items:e}){return u`
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
  `;return n?u`<${ne} padding="lg">${r}<//>`:u`<div className="py-8">${r}</div>`}var v2={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function an({result:e,onDismiss:t}){return e?u`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",v2[e.type]||v2.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}var g2="",CD={workspace:"home"};function od(e){return CD[e]||e}function fl(e){return[...e||[]].sort((t,a)=>t.is_dir!==a.is_dir?t.is_dir?-1:1:t.name.localeCompare(a.name,void 0,{sensitivity:"base"}))}function xi(e){return e?e.split("/").filter(Boolean):[]}function ld(e){return e?`/workspace/${xi(e).map(encodeURIComponent).join("/")}`:"/workspace"}function Mh(e){let t=xi(e);return t.pop(),t.join("/")}function y2(e){return/\.mdx?$/i.test(e||"")}function ud({path:e,onNavigate:t}){let a=k(),n=xi(e),r="";return u`
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
  `}function ED(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function b2({path:e,entries:t,isLoading:a,filter:n,onOpen:r,onNavigate:s}){let i=k();if(a)return u`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;let o=(t||[]).filter(f=>!ED(f.path)),l=String(n||"").trim().toLowerCase(),c=l?o.filter(f=>f.name.toLowerCase().includes(l)):o,d=fl(c),m;return o.length?d.length?m=u`
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
        <${ud} path=${e} onNavigate=${s} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">${m}</div>
    <//>
  `}var cd="/api/webchat/v2/fs",TD=1024*1024,AD=8*1024*1024;function x2(e){let t=String(e||"").split("/").filter(Boolean);return{mount:t.shift()||"",path:t.join("/")}}function DD(e,t){return t?`${e}/${t}`:e}function MD(e){let t=String(e||"").toLowerCase();return t.startsWith("text/")||t==="application/json"||t==="application/javascript"||t==="application/xml"||t.endsWith("+json")||t.endsWith("+xml")}function OD(e){return String(e||"").toLowerCase().startsWith("image/")}function LD(e){let t=String(e||"").toLowerCase();return t.startsWith("audio/")||t.startsWith("video/")||t.startsWith("font/")||t==="application/pdf"||t==="application/zip"||t==="application/gzip"}function PD(e){if(e.subarray(0,Math.min(e.length,8192)).indexOf(0)!==-1)return!0;try{return new TextDecoder("utf-8",{fatal:!0}).decode(e),!1}catch{return!0}}function UD(e,t){let a=new URL(`${cd}/content`,window.location.origin);return a.searchParams.set("mount",e),a.searchParams.set("path",t),a.pathname+a.search}async function jD(){return(await V(`${cd}/mounts`))?.mounts||[]}async function $i(e=""){if(!e)return{entries:(await jD()).map(o=>({name:od(o.mount),path:o.mount,is_dir:!0}))};let{mount:t,path:a}=x2(e),n=new URL(`${cd}/list`,window.location.origin);return n.searchParams.set("mount",t),a&&n.searchParams.set("path",a),{entries:((await V(n.pathname+n.search))?.entries||[]).map(i=>({name:i.name,path:DD(t,i.path),is_dir:i.kind==="directory"}))}}async function $2(e){let{mount:t,path:a}=x2(e);if(!t||!a)return{kind:"directory",path:e};let n=new URL(`${cd}/stat`,window.location.origin);n.searchParams.set("mount",t),n.searchParams.set("path",a);let s=(await V(n.pathname+n.search))?.stat||{},i=s.mime_type||"application/octet-stream",o=Number(s.size_bytes||0),l=UD(t,a),c={path:e,mime:i,size_bytes:o,download_path:l};if(s.kind&&s.kind!=="file")return{...c,kind:"directory"};if(OD(i)){if(o>AD)return{...c,kind:"binary"};let h=await Nc(l);return{...c,kind:"image",image_data_url:h}}if(LD(i)||o>TD)return{...c,kind:"binary"};let d=await Aa(l),m=new Uint8Array(await d.arrayBuffer());if(!MD(i)&&PD(m))return{...c,kind:"binary"};let f=new TextDecoder("utf-8").decode(m);return{...c,kind:"text",content:f}}function w2(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function FD(e,t,a){let n=String(t||"").trim().toLowerCase(),r=(e||[]).filter(s=>!w2(s.path)).filter(s=>!n||s.name.toLowerCase().includes(n)?!0:s.is_dir&&a.has(s.path));return fl(r)}function S2({entry:e,depth:t,selectedPath:a,expandedPaths:n,filter:r,onToggleDirectory:s,onSelectFile:i}){let o=k(),l=n.has(e.path),c=K({queryKey:["workspace-list",e.path],queryFn:()=>$i(e.path),enabled:e.is_dir&&l});if(e.is_dir){let d=FD(c.data?.entries,r,n);return u`
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
                  <${S2}
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
  `}function N2({entries:e,selectedPath:t,expandedPaths:a,filter:n,onToggleDirectory:r,onSelectFile:s,isLoading:i}){let o=k();if(i)return u`<div className="space-y-2 p-3">${[1,2,3,4].map(c=>u`<div key=${c} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let l=fl(e.filter(c=>!w2(c.path)));return l.length?u`
    <div className="space-y-1 p-2">
      ${l.map(c=>u`
        <${S2}
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
  `:u`<div className="px-4 py-8 text-sm text-iron-300">${o("workspace.noFiles")}</div>`}function _2({rootEntries:e,selectedPath:t,expandedPaths:a,filter:n,onFilterChange:r,isLoadingTree:s,onToggleDirectory:i,onSelectFile:o}){let l=k();return u`
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
        <${N2}
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
  `}function R2(e){return xi(e).pop()||"download"}function zD({path:e,file:t}){let a=k();return t.kind==="image"?u`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${t.image_data_url}
          alt=${R2(e)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `:t.kind==="text"?u`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${y2(e)?u`<${sa} content=${t.content} className="max-w-4xl text-base leading-7" />`:u`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
      </div>
    `:u`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${a("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `}function k2({path:e,file:t,isLoading:a,onNavigate:n}){let r=k(),[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(t?.download_path){i(!0);try{let c=await Aa(t.download_path);Vc(c,R2(e))}catch{}finally{i(!1)}}},[t,e]);if(a)return u`
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
        <${ud} path=${e} onNavigate=${n} />
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

      ${Mh(e)&&u`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${r("workspace.parent",{path:Mh(e)})}
        </div>
      `}
    <//>
  `}function C2(e){let t=k(),a=Z(),[n,r]=p.default.useState(new Set),[s,i]=p.default.useState(""),[o,l]=p.default.useState(null),c=K({queryKey:["workspace-list",""],queryFn:()=>$i("")}),d=K({queryKey:["workspace-file",e],queryFn:()=>$2(e),enabled:!!e}),m=e===""||d.data?.kind==="directory",f=K({queryKey:["workspace-list",e],queryFn:()=>$i(e),enabled:m});p.default.useEffect(()=>{l(null)},[e]);let h=p.default.useCallback(y=>a.fetchQuery({queryKey:["workspace-list",y],queryFn:()=>$i(y)}),[a]),x=p.default.useCallback(async y=>{let $=new Set(n);if($.has(y)){$.delete(y),r($);return}$.add(y),r($);try{await h(y)}catch(g){l({type:"error",message:g.message||t("workspace.unableOpenDirectory")})}},[n,h,t]);return{rootEntries:c.data?.entries||[],file:d.data||null,selectionIsDirectory:m,currentEntries:f.data?.entries||[],expandedPaths:n,filter:s,setFilter:i,result:o,clearResult:()=>l(null),isLoadingTree:c.isLoading,isLoadingFile:d.isLoading,isLoadingListing:f.isLoading,isFetching:c.isFetching||d.isFetching||f.isFetching,error:c.error||d.error||f.error||null,loadDirectory:h,toggleDirectory:x,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function Oh(){let e=k(),t=ve(),n=it()["*"]||g2,r=C2(n),s=p.default.useCallback(i=>{t(ld(i))},[t]);return u`
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
            <${_2}
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
                  <${b2}
                    path=${n}
                    entries=${r.currentEntries}
                    isLoading=${r.isLoadingListing}
                    filter=${r.filter}
                    onOpen=${s}
                    onNavigate=${t}
                  />
                `:u`
                  <${k2}
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
  `}function E2(e){if(!e)return null;let t=e.metadata&&typeof e.metadata=="object"&&!Array.isArray(e.metadata)?e.metadata:{};return{id:e.project_id,name:e.name,description:e.description,goals:Array.isArray(t.goals)?t.goals:[],icon:e.icon||null,color:e.color||null,state:e.state,role:e.role,metadata:t,created_at:e.created_at,updated_at:e.updated_at,health:e.state==="archived"?"muted":"green"}}async function T2(){let t=((await s$({limit:200}))?.projects||[]).map(E2);return{attention:[],projects:t}}async function A2(e){if(!e)return null;let t=await i$({projectId:e});return E2(t?.project)}function D2(e){return Promise.resolve({missions:[],todo:!0})}function M2(e){return Promise.resolve({threads:[],todo:!0})}function O2(e){return Promise.resolve({widgets:[],todo:!0})}function L2(e){return Promise.resolve(null)}function P2(e){return Promise.resolve(null)}function U2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function j2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function F2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function z2(){let e=Z(),t=K({queryKey:["projects-overview"],queryFn:T2,refetchInterval:5e3}),a=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function B2(e){let t=Z(),a=!!e,n=K({queryKey:["project-detail",e],queryFn:()=>A2(e),enabled:a,refetchInterval:a?7e3:!1}),r=K({queryKey:["project-missions",e],queryFn:()=>D2(e),enabled:a,refetchInterval:a?5e3:!1}),s=K({queryKey:["project-threads",e],queryFn:()=>M2(e),enabled:a,refetchInterval:a?4e3:!1}),i=K({queryKey:["project-widgets",e],queryFn:()=>O2(e),enabled:a,refetchInterval:a?15e3:!1}),o=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function q2({projectId:e,missionId:t,threadId:a}){let n=Z(),[r,s]=p.default.useState(null),i=K({queryKey:["project-mission-detail",t],queryFn:()=>L2(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=K({queryKey:["project-thread-detail",a],queryFn:()=>P2(a),enabled:!!a,refetchInterval:a?4e3:!1}),l=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=Y({mutationFn:({targetMissionId:f})=>U2(f),onSuccess:f=>{s({type:"success",message:f?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to fire mission"})}}),d=Y({mutationFn:({targetMissionId:f})=>j2(f),onSuccess:()=>{s({type:"success",message:"Mission paused."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to pause mission"})}}),m=Y({mutationFn:({targetMissionId:f})=>F2(f),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending}}function dd(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function md(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function I2(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function H2(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function BD(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function K2(e){let t=BD(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function Q2(e){let t=e?.projects||[],a=t.reduce((o,l)=>o+Number(l.cost_today_usd||0),0),n=t.reduce((o,l)=>o+Number(l.active_missions||0),0),r=t.reduce((o,l)=>o+Number(l.threads_today||0),0),s=t.reduce((o,l)=>o+Number(l.pending_gates||0),0),i=t.reduce((o,l)=>o+Number(l.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function pl(e,t){return`${e} ${t}${e===1?"":"s"}`}var qD={projects:"muted",attention:"warning",spend:"success"};function V2({overview:e}){let t=Q2(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:md(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return u`
    <${H} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>u`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${I} tone=${qD[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function ID(e){return e?.type==="failure"?"danger":"warning"}function HD(e){return e?.type==="failure"?"failure":"gate"}function G2({items:e,onOpenItem:t}){return e?.length?u`
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
              <${I} tone=${ID(a)} label=${HD(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function KD({project:e,onOpen:t,t:a}){return u`
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
        <${I} tone=${I2(e.health)} label=${e.health||"unknown"} />
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
        <${T}
          variant="secondary"
          onClick=${n=>{n.stopPropagation(),t(e.id)}}
        >${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function QD({project:e,onOpen:t,t:a}){return u`
    <${H}
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
          <${T}
            variant="secondary"
            onClick=${n=>{n.stopPropagation(),t(e.id)}}
          >${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function Y2({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=k(),l=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return!e.length&&t>0?u`
      <${$e}
        title=${o("projects.empty.noMatchTitle")}
        description=${o("projects.empty.noMatchDesc")}
      />
    `:e.length?u`
    <div className="space-y-5">
      ${l&&u`<${QD} project=${l} onOpen=${r} t=${o} />`}

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
            ${c.map(d=>u`<${KD} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
          </div>`:u`
            <${$e}
              title=${o("projects.scoped.onlyGeneralTitle")}
              description=${o("projects.scoped.onlyGeneralDesc")}
            >
              <${T} onClick=${s}>${o(i?"projects.preparingChat":"projects.startProject")}<//>
            <//>
          `}
    </div>
  `:u`
      <${$e}
        title=${o("projects.empty.noneTitle")}
        description=${o("projects.empty.noneDesc")}
      >
        <${T} onClick=${s}>${o("projects.createFromChat")}<//>
      <//>
    `}function J2({threads:e,selectedThreadId:t,onSelectThread:a,onNewConversation:n,isStartingConversation:r}){let s=[...e].sort((i,o)=>new Date(o.updated_at||o.created_at)-new Date(i.updated_at||i.created_at));return u`
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
        ${s.length?s.slice(0,18).map(i=>{let o=K2(i);return u`
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
                    <${I} tone=${H2(i.state)} label=${i.state} />
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
  `}var VD="/workspace";function GD(e){let t=a=>a.kind==="directory"?0:1;return[...e].sort((a,n)=>t(a)-t(n)||a.name.localeCompare(n.name,void 0,{sensitivity:"base"}))}function YD(e){return e?String(e).replace(/^\/workspace\/?/,"").split("/").filter(Boolean):[]}function X2({threadId:e}){let t=k(),[a,n]=p.default.useState(void 0),[r,s]=p.default.useState(null),i=K({queryKey:["project-files",e||"",a||""],queryFn:()=>W0({threadId:e,path:a}),enabled:!!e}),o=p.default.useMemo(()=>GD(i.data?.entries||[]),[i.data]),l=p.default.useCallback(async m=>{if(m.kind==="directory"){s(null),n(m.path);return}try{s(null);let f=await Aa(Sc({threadId:e,path:m.path})),h=URL.createObjectURL(f),x=document.createElement("a");x.href=h,x.download=m.name,document.body.appendChild(x),x.click(),x.remove(),URL.revokeObjectURL(h)}catch(f){s(f?.message||"Unable to download file")}},[e]),c=YD(a),d=u`
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
        ${c.map((m,f)=>{let h=`${VD}/${c.slice(0,f+1).join("/")}`;return u`
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
    `}function JD(e){return[...e||[]].sort((a,n)=>new Date(n.updated_at||n.created_at)-new Date(a.updated_at||a.created_at))[0]?.id||null}function W2({project:e,threads:t,selectedThreadId:a,onSelectThread:n,onNewConversation:r,isStartingConversation:s}){let i=JD(t);return u`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]">
      <div className="space-y-5">
        <div className="min-w-0">
          <h2 className="text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
          ${e.description?u`<p className="mt-1 text-sm leading-6 text-iron-300">${e.description}</p>`:null}
        </div>

        <${J2}
          threads=${t}
          selectedThreadId=${a}
          onSelectThread=${n}
          onNewConversation=${r}
          isStartingConversation=${s}
        />
      </div>

      <${X2} threadId=${i} />
    </div>
  `}function hl(){let e=k(),t=ve(),{threadsState:a}=wa(),{projectId:n=null,threadId:r=null}=it(),[s,i]=p.default.useState(""),[o,l]=p.default.useState(null),c=z2(),d=B2(n),m=q2({projectId:n,threadId:r}),f=p.default.useMemo(()=>{let R=s.trim().toLowerCase();return R?c.overview.projects.filter(_=>[_.name,_.description,..._.goals||[]].some(M=>String(M||"").toLowerCase().includes(R))):c.overview.projects},[c.overview.projects,s]),h=p.default.useMemo(()=>c.overview.projects.find(R=>R.id===n)||null,[c.overview.projects,n]),x=p.default.useCallback(()=>{c.invalidate(),d.invalidate()},[c,d]),y=p.default.useCallback(R=>{t(`/projects/${R}`)},[t]),$=p.default.useCallback(R=>{if(R.thread_id){t(`/projects/${R.project_id}/threads/${R.thread_id}`);return}t(`/projects/${R.project_id}`)},[t]),g=p.default.useCallback(async()=>{let R=null;l(null);try{R=await a.createThread()}catch(_){l({type:"error",message:_.message||e("projects.chatAutoFail")})}t("/chat",{state:{composerDraft:e("projects.creationDraft"),threadId:R}})},[t,a]),v=p.default.useCallback(R=>{t(`/projects/${n}/threads/${R}`)},[t,n]),b=p.default.useCallback(async()=>{l(null);try{let R=await a.createThread(n);t("/chat",{state:{threadId:R}}),d.invalidate()}catch(R){l({type:"error",message:R.message||e("projects.chatAutoFail")})}},[t,a,n,d,e]),w=p.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),S=u`
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
        <${W2}
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
          <${Y2}
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
            <${V2} overview=${c.overview} />
            <${G2} items=${c.overview.attention} onOpenItem=${$} />
          `}
          ${C}
        </div>
      </div>
    </div>
  `}function vl(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not scheduled"}function gl(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function Z2(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function eN(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);return r!==0?r:new Date(n.updated_at||0).getTime()-new Date(a.updated_at||0).getTime()})}function fd({label:e,value:t}){return u`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function XD({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=k();return e.status==="Active"?u`
      <${T} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.fireNow")}<//>
      <${T} variant="secondary" onClick=${()=>n(e.id)} disabled=${t}>${s("missions.action.pause")}<//>
    `:e.status==="Paused"?u`
      <${T} onClick=${()=>r(e.id)} disabled=${t}>${s("missions.action.resume")}<//>
      <${T} variant="secondary" onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runOnce")}<//>
    `:u`<${T} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runAgain")}<//>`}function tN({mission:e,isLoading:t,error:a,isBusy:n,onFire:r,onPause:s,onResume:i,onOpenProject:o,onOpenThread:l}){let c=k();return t?u`
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
          <${I} tone=${gl(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${fd} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${fd} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${fd} label=${c("missions.meta.nextFire")} value=${vl(e.next_fire_at)} />
          <${fd} label=${c("missions.meta.updated")} value=${vl(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${XD}
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
                  <${I} tone=${gl(d.state==="Running"?"Active":d.state==="Failed"?"Failed":"Completed")} label=${d.state} />
                </div>
              </button>
            `)}
          </div>
        <//>
      `:null}
    </div>
  `}function WD(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function aN({value:e,onChange:t,children:a,label:n}){return u`
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
  `}function ZD({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=k(),s=t===e.id;return u`
    <div
      className=${["w-full rounded-xl border p-4 text-left",s?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80"].join(" ")}
    >
      <button type="button" onClick=${()=>a(e.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${e.name}</div>
              <${I} tone=${gl(e.status)} label=${e.status} />
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
        <${T}
          variant="ghost"
          onClick=${i=>{i.stopPropagation(),n(e.project.id)}}
        >
          ${e.project.name}
        <//>
      </div>
    </div>
  `}function Lh({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:l,projectOptions:c,onSelectMission:d,onOpenProject:m}){let f=k(),h=WD(f);return u`
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
        <${aN} value=${s} onChange=${i} label=${f("missions.filter.status")}>
          ${h.map(x=>u`<option key=${x.value} value=${x.value}>${x.label}<//>`)}
        <//>
        <${aN} value=${o} onChange=${l} label=${f("missions.filter.project")}>
          <option value="all">${f("missions.filter.allProjects")}</option>
          ${c.map(x=>u`<option key=${x.id} value=${x.id}>${x.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(x=>u`
              <${ZD}
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
  `}function eM(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function nN({summary:e}){let t=k(),a=eM(t);return u`
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
  `}function rN(){return Promise.resolve({projects:[],todo:!0})}function sN({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function iN(e){return Promise.resolve(null)}function oN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function lN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function uN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function cN(e){let t=K({queryKey:["mission-detail",e],queryFn:()=>iN(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function tM(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function dN(){let e=Z(),[t,a]=p.default.useState(null),n=K({queryKey:["projects-overview"],queryFn:rN,refetchInterval:7e3}),r=n.data?.projects||[],s=qd({queries:r.map(f=>({queryKey:["missions","project",f.id],queryFn:()=>sN({projectId:f.id}),refetchInterval:5e3,select:h=>h?.missions||[]}))}),i=s.flatMap((f,h)=>{let x=r[h];return(f.data||[]).map(y=>tM(y,x))}),o=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),l=(f,h)=>({mutationFn:({missionId:x})=>f(x),onSuccess:()=>{a({type:"success",message:h}),o()},onError:x=>{a({type:"error",message:x.message||"Unable to update mission"})}}),c=Y(l(oN,"Mission fired and a run was queued.")),d=Y(l(lN,"Mission paused.")),m=Y(l(uN,"Mission resumed."));return{projects:r,missions:i,summary:Z2(i),isLoading:n.isLoading||s.some(f=>f.isLoading),isRefreshing:n.isFetching||s.some(f=>f.isFetching),error:n.error||s.find(f=>f.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending,invalidate:o}}function Ph(){let e=k(),t=ve(),{missionId:a=null}=it(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,l]=p.default.useState("all"),c=dN(),d=cN(a),m=p.default.useMemo(()=>{let g=n.trim().toLowerCase();return eN(c.missions).filter(v=>{let b=!g||[v.name,v.goal,v.project?.name].some(C=>String(C||"").toLowerCase().includes(g)),w=s==="all"||v.status===s,S=o==="all"||v.project?.id===o;return b&&w&&S})},[c.missions,o,n,s]),f=p.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),h=d.mission?{...f,...d.mission,project:f?.project||null}:f,x=p.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),y=p.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),$=a?u`
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <${Lh}
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
          <${tN}
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
        <${Lh}
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
          <${nN} summary=${c.summary} />

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
  `}var mN=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],aM=new Set(["pending","in_progress"]),fN=new Set(["failed","interrupted","stuck","cancelled"]);function ur(e){return e?String(e).replace(/_/g," "):"unknown"}function wi(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":fN.has(e)?"danger":"muted":"muted"}function nM(e){return aM.has(e)}function pd(e){return nM(e?.state)}function pN(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":fN.has(e.state):!1}function Gr(e,t=8){return e?String(e).slice(0,t):"unknown"}function ia(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function hN(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function Uh(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${ia(e.started_at)}`:null].filter(Boolean).join(" / ")}var rM=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function vN(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function sM({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?u`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${t==="tool_use"?a.tool_name||"Tool call":a.tool_name||"Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${vN(t==="tool_use"?a.input:a.output||a.error||a)}</pre>
      </details>
    `:t==="message"?u`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a.role||"assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${a.content||""}</div>
      </div>
    `:u`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t.replace(/_/g," ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${a.message||a.status||vN(a)}</div>
    </div>
  `}function gN({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=k(),[s,i]=p.default.useState("all"),[o,l]=p.default.useState(""),[c,d]=p.default.useState(!0),m=p.default.useRef(null),f=p.default.useMemo(()=>s==="all"?t:t.filter(x=>x.event_type===s),[t,s]);p.default.useEffect(()=>{c&&m.current&&(m.current.scrollTop=m.current.scrollHeight)},[c,f.length]);let h=p.default.useCallback(async(x=!1)=>{let y=o.trim();if(!(!y&&!x))try{await a({content:y||"(done)",done:x}),l("")}catch{}},[o,a]);return u`
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
            ${rM.map(x=>u`<option key=${x.value} value=${x.value}>${x.label}</option>`)}
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
                <${sM} event=${x} />
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
  `}function yN({job:e,activeTab:t,onTabChange:a,onBack:n,onCancel:r,onRestart:s,isBusy:i,children:o}){return u`
    <div className="space-y-5">
      <${H} className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick=${n} className="text-sm text-signal hover:text-white">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-semibold tracking-tight text-white">${e.title||"Untitled job"}</h2>
              <${I} tone=${wi(e.state)} label=${ur(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${Gr(e.id)}</span>
              <span>created ${ia(e.created_at)}</span>
              ${Uh(e)&&u`<span>${Uh(e)}</span>`}
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
              <${T} variant="secondary" disabled=${i} onClick=${()=>r(e.id)}>Cancel<//>
            `}
            ${pN(e)&&u`
              <${T} variant="primary" disabled=${i} onClick=${()=>s(e.id)}>Restart<//>
            `}
          </div>
        </div>
      <//>

      <div className="flex flex-wrap gap-2">
        ${mN.map(l=>u`
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
  `}function bN({nodes:e,depth:t=0,selectedPath:a,expandingPath:n,onToggleDirectory:r,onSelectPath:s}){return u`
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
        ${i.isDir&&i.expanded&&i.children?.length?u`<${bN}
              nodes=${i.children}
              depth=${t+1}
              selectedPath=${a}
              expandingPath=${n}
              onToggleDirectory=${r}
              onSelectPath=${s}
            />`:null}
      </div>
    `)}
  `}function xN({canBrowse:e,tree:t,selectedPath:a,selectedFile:n,fileError:r,isLoadingTree:s,isLoadingFile:i,expandingPath:o,treeError:l,onToggleDirectory:c,onSelectPath:d}){return e?u`
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <${H} className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          ${l&&u`<div className="mx-2 mb-3 rounded-md border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${l}</div>`}
          ${s?u`<div className="space-y-2 px-2">${[1,2,3,4].map(m=>u`<div key=${m} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?u`
                  <${bN}
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
    `}function Si({label:e,value:t}){return u`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function $N({job:e}){let t=(e.transitions||[]).map(a=>({title:`${ur(a.from)} -> ${ur(a.to)}`,description:[ia(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return u`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${H} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${I} tone=${wi(e.state)} label=${ur(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${Si} label="Created" value=${ia(e.created_at)} />
          <${Si} label="Started" value=${ia(e.started_at)} />
          <${Si} label="Completed" value=${ia(e.completed_at)} />
          <${Si} label="Duration" value=${hN(e.elapsed_secs)} />
          <${Si} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${Si} label="Mode" value=${e.job_mode||"Default worker"} />
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
                  <${h2} items=${t} />
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
  `}function wN({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:l,isBusy:c,isRefreshing:d}){let m=k(),f=[{value:"all",label:m("jobs.list.filter.all")},{value:"pending",label:m("jobs.list.filter.pending")},{value:"in_progress",label:m("jobs.list.filter.inProgress")},{value:"completed",label:m("jobs.list.filter.completed")},{value:"failed",label:m("jobs.list.filter.failed")},{value:"stuck",label:m("jobs.list.filter.stuck")}];if(!e.length){let h=!!n.trim()||s!=="all";return u`
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
                  <${I} tone=${wi(h.state)} label=${ur(h.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${Gr(h.id)}</span>
                  <span>${m("jobs.list.created",{value:ia(h.created_at)})}</span>
                  ${h.started_at&&u`<span>${m("jobs.list.started",{value:ia(h.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${pd(h)&&u`
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
  `}var iM=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function SN({summary:e}){return u`
    <${H} className="p-4 sm:p-5">
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
  `}function NN(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function _N(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function RN(e){return Promise.resolve(null)}function kN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function CN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function EN(e){return Promise.resolve({events:[],todo:!0})}function TN(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function jh(e,t=""){return Promise.resolve({entries:[],todo:!0})}function AN(e,t){return Promise.resolve({content:"",todo:!0})}function DN(e){let t=Z(),[a,n]=p.default.useState(null),r=K({queryKey:["job-detail",e],queryFn:()=>RN(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=K({queryKey:["job-events",e],queryFn:()=>EN(e),enabled:!!e,refetchInterval:e?2500:!1}),i=Y({mutationFn:({content:o,done:l})=>TN(e,{content:o,done:l}),onSuccess:(o,{done:l})=>{n({type:"success",message:l?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return p.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function MN(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function ON(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=ON(a.children,t);if(n)return n}}return null}function hd(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:hd(n.children,t,a)}:n)}function LN(e){let[t,a]=p.default.useState([]),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState(""),c=!!(e?.project_dir&&e?.id),d=K({queryKey:["job-files-root",e?.id],queryFn:()=>jh(e.id,""),enabled:c}),m=K({queryKey:["job-file",e?.id,n],queryFn:()=>AN(e.id,n),enabled:!!(c&&n)});p.default.useEffect(()=>{a([]),r(""),i(""),l("")},[e?.id]),p.default.useEffect(()=>{d.data?.entries?(a(MN(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let f=p.default.useCallback(async h=>{let x=ON(t,h);if(!(!x||!e?.id)){if(x.expanded){a(y=>hd(y,h,$=>({...$,expanded:!1})));return}if(x.loaded){a(y=>hd(y,h,$=>({...$,expanded:!0})));return}l(h);try{let y=await jh(e.id,h);a($=>hd($,h,g=>({...g,expanded:!0,loaded:!0,children:MN(y.entries)}))),i("")}catch(y){i(y.message||"Unable to open folder")}finally{l("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:m.data||null,fileError:m.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:m.isLoading||m.isFetching,expandingPath:o,treeError:s,toggleDirectory:f}}function PN(){let e=Z(),[t,a]=p.default.useState(null),n=K({queryKey:["jobs-summary"],queryFn:_N,refetchInterval:5e3}),r=K({queryKey:["jobs"],queryFn:NN,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=Y({mutationFn:({jobId:l})=>kN(l),onSuccess:(l,{jobId:c})=>{a({type:"success",message:`Job ${Gr(c)} cancelled`}),s()},onError:l=>{a({type:"error",message:l.message||"Unable to cancel job"})}}),o=Y({mutationFn:({jobId:l})=>CN(l),onSuccess:l=>{a({type:"success",message:`Restart queued as ${Gr(l?.new_job_id)}`}),s()},onError:l=>{a({type:"error",message:l.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function UN({result:e,onDismiss:t}){let a=k();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return u`
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
  `}function Fh(){let e=k(),t=ve(),{jobId:a=null}=it(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,l]=p.default.useState(a?"activity":"overview"),c=PN(),d=DN(a),m=LN(d.job);p.default.useEffect(()=>{l(a?"activity":"overview")},[a]);let f=p.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(b=>{let w=!v||b.title.toLowerCase().includes(v)||b.id.toLowerCase().includes(v),S=s==="all"||b.state===s;return w&&S})},[c.jobs,n,s]),h=p.default.useCallback(v=>t(`/jobs/${v}`),[t]),x=p.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),y=p.default.useCallback(async v=>{try{let b=await c.restartJob({jobId:v});b?.new_job_id&&t(`/jobs/${b.new_job_id}`)}catch{}},[c,t]),$=u`
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
      `;else{let v={overview:u`<${$N} job=${d.job} />`,activity:u`
          <${gN}
            job=${d.job}
            events=${d.events}
            onSendPrompt=${d.sendPrompt}
            isSendingPrompt=${d.isSendingPrompt}
          />
        `,files:u`
          <${xN}
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
        <${yN}
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
          <${wN}
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
          <${UN}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${UN}
            result=${d.promptResult}
            onDismiss=${d.clearPromptResult}
          />
          <${SN} summary=${c.summary} />
          ${g}
        </div>
      </div>
    </div>
  `}function cr(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not scheduled"}function vd(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function gd(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function jN(e=[]){return[...e].sort((t,a)=>t.enabled!==a.enabled?t.enabled?-1:1:new Date(a.next_fire_at||a.last_run_at||0).getTime()-new Date(t.next_fire_at||t.last_run_at||0).getTime())}function FN(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function oM(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function zN({runs:e}){return e?.length?u`
    <div className="space-y-3">
      ${e.map(t=>u`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${I} tone=${oM(t.status)} label=${t.status} />
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
  `}function qN({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=ve(),l=k();return t?u`
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
              tone=${vd(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${I}
              tone=${gd(e.verification_status)}
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
        <${dr} label="Trigger" value=${e.trigger_summary||e.trigger_type} />
        <${dr} label="Action" value=${FN(e.action)} />
        <${dr} label="Next fire" value=${cr(e.next_fire_at)} />
        <${dr} label="Last run" value=${cr(e.last_run_at)} />
        <${dr} label="Run count" value=${e.run_count} />
        <${dr} label="Failures" value=${e.consecutive_failures} />
        <${dr} label="Created" value=${cr(e.created_at)} />
        <${dr} label="Routine ID" value=${e.id} />
      </div>

      ${e.conversation_id&&u`
        <div className="mt-5">
          <${T} variant="secondary" onClick=${()=>o(`/chat/${e.conversation_id}`)}>
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
        <${zN} runs=${e.recent_runs} />
      </div>
    <//>
  `}function IN({routine:e,selectedRoutineId:t,onSelectRoutine:a,onTriggerRoutine:n,onToggleRoutine:r,isBusy:s}){let i=t===e.id;return u`
    <article
      className=${["group flex flex-col gap-4 rounded-[18px] border p-5",i?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick=${()=>a(e.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">${e.name}</h3>
            <${I}
              tone=${vd(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${I}
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
  `}var lM=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function zh({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:l,onToggleRoutine:c,isBusy:d,isRefreshing:m}){let f=k();if(!e.length){let h=!!n.trim()||s!=="all";return u`
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
            ${lM.map(h=>u`<option key=${h.value} value=${h.value}>${h.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(h=>u`
            <${IN}
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
  `}var uM=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function HN({summary:e}){return u`
    <${H} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${uM.map(t=>u`
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
  `}function KN(e){let[t,a]=p.default.useState(""),[n,r]=p.default.useState("all");return{filteredRoutines:p.default.useMemo(()=>{let i=t.trim().toLowerCase();return jN(e).filter(o=>{let l=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||l.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function QN(){return Promise.resolve({routines:[],todo:!0})}function VN(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function GN(e){return Promise.resolve(null)}function yd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function bd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function YN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function JN(e){let t=Z(),[a,n]=p.default.useState(null),r=K({queryKey:["routine-detail",e],queryFn:()=>GN(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:m=>{n({type:"error",message:m.message||"Unable to update routine"})}}),o=Y(i(yd,"Routine run queued.")),l=Y(i(bd,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:l.mutateAsync,isBusy:o.isPending||l.isPending}}function XN(){let e=Z(),[t,a]=p.default.useState(null),n=K({queryKey:["routines-summary"],queryFn:VN,refetchInterval:5e3}),r=K({queryKey:["routines"],queryFn:QN,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,m)=>({mutationFn:({routineId:f})=>d(f),onSuccess:()=>{a({type:"success",message:m}),s()},onError:f=>{a({type:"error",message:f.message||"Unable to update routine"})}}),o=Y(i(yd,"Routine run queued.")),l=Y(i(bd,"Routine status updated.")),c=Y(i(YN,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:l.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||l.isPending||c.isPending,invalidate:s}}function Bh(){let e=ve(),{routineId:t=null}=it(),a=XN(),n=JN(t),r=KN(a.routines),s=p.default.useCallback(async(l,c)=>{try{await l({routineId:c})}catch{}},[]),i=p.default.useCallback(async(l,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:l}),e("/routines")}catch{}},[e,a]),o=t?u`
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
          <${qN}
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
          <${HN} summary=${a.summary} />

          ${a.isLoading?u`
                <div className="space-y-4">
                  ${[1,2,3].map(l=>u`<div key=${l} className="v2-skeleton h-32 rounded-xl" />`)}
                </div>
              `:o}
        </div>
      </div>
    </div>
  `}function cM(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function dM(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function WN({deliveryState:e}){let t=k(),a=e.currentTarget?.target_id||"",[n,r]=p.default.useState(a),[s,i]=p.default.useState(!1),o=p.default.useRef(null);p.default.useEffect(()=>{r(a)},[a]),p.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let l=n!==a,c=e.isLoading||e.isSaving,d=l&&!c,m=!!a&&!c,f=e.finalReplyTargets.length>0,h=e.targets.some(M=>M?.capabilities?.final_replies&&M?.target?.status==="unavailable"),x=f||h,y=M=>(o.current&&clearTimeout(o.current),i(!1),M.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),$=()=>{d&&y(e.saveFinalReplyTarget(n||null))},g=()=>{m&&(r(""),y(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),b=e.currentStatus,w=b==="available"?"success":b==="unavailable"?"warning":"muted",S=t(b==="available"?"automations.delivery.pill.ready":b==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),C=!!e.currentTarget,R=t(C?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),_=dM(t("automations.delivery.footnote"),{command:u`<code
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
            ${e.finalReplyTargets.map(M=>{let L=M?.target?.target_id??"",U=M?.target?.display_name||M?.target?.target_id||"",F=M?.target?.description||"",z=M?.target?.status??"available",P=n===L;return u`
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
                    tone=${cM(z)}
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
  `}var mM=["schedule","once"],e_={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},t_={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},a_={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function oa(e){return typeof e=="function"?e:t=>t}var Ih=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:Tn},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:kM},{value:"completed",labelKey:"automations.filter.completed",predicate:CM}];function n_(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>mM.includes(r?.source?.type)).map(r=>wM(r,t,a)).sort(RM)}function r_(e,t){let a=Ih.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function s_(e){let t=e.filter(i=>i.state!=="completed"),a=t.filter(i=>Tn(i)).length,n=t.filter(i=>i.has_running_run).length,r=t.filter(i=>i.has_failed_runs).length,s=t.filter(i=>Tn(i)&&qh(i)!=null).sort((i,o)=>(i.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(o.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:t.length,active:a,running:n,failures:r,nextRun:s?.next_run_label||null}}function fM(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=DM(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:l,month:c,dayOfWeek:d,year:m}=s,f=t&&typeof t=="string"?t:null,h=f?` (${f})`:"",x=m==="*"&&l==="*"&&c==="*"&&d==="*";if(x&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=MM(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(mr(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let y=EM(o,i,n);if(!y)return r("automations.schedule.custom");if(x)return r("automations.schedule.everyDayAt",{time:y})+h;let $=OM(d);if(m==="*"&&l==="*"&&c==="*"&&$==="1-5")return r("automations.schedule.weekdaysAt",{time:y})+h;if(m==="*"&&l==="*"&&c==="*"&&mr($,0,7)){let g=TM(Number($)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:y})+h}if(m==="*"&&mr(l,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(l),time:y})+h;if(mr(l,1,31)&&mr(c,1,12)&&d==="*"&&(m==="*"||mr(m,1970,9999))){let g=AM(Number(c),Number(l),m==="*"?null:Number(m),n);return r("automations.schedule.dateAt",{date:g,time:y})+h}return r("automations.schedule.custom")}function Yr(e,t="Unknown",a,n){if(!e)return t;let r=new Date(e);if(Number.isNaN(r.getTime()))return t;let s=n&&typeof n=="string"?{timeZone:n}:{};try{return r.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...s})}catch{return r.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function i_(e,t){let a=e_[e]?.labelKey||"automations.state.unknown";return oa(t)(a)}function o_(e){return e_[e]?.tone||"muted"}function pM(e,t){return Tn(e)&&e?.has_running_run?oa(t)("automations.status.running"):Tn(e)&&e?.has_failed_runs?oa(t)("automations.status.needsReview"):i_(e?.state,t)}function hM(e){return Tn(e)&&e?.has_running_run?"info":Tn(e)&&e?.has_failed_runs?"danger":o_(e?.state)}function vM(e,t){let a=t_[e]?.labelKey||"automations.lastStatus.none";return oa(t)(a)}function gM(e){return t_[e]?.tone||"muted"}function yM(e,t){let a=a_[xd(e)]?.labelKey||"automations.runStatus.unknown";return oa(t)(a)}function bM(e){return a_[xd(e)]?.tone||"muted"}function xM(e,t,a,n){if(!e)return oa(a)("automations.schedule.custom");let r=Yr(e,null,n,t);if(!r)return oa(a)("automations.schedule.custom");let s=t&&typeof t=="string"?` (${t})`:"";return oa(a)("automations.schedule.onceAt",{datetime:r})+s}function $M(e,t,a){return e?.type==="once"?xM(e.at,e.timezone,t,a):e?.type==="schedule"?fM(e.cron,e.timezone||"UTC",t,a):oa(t)("automations.schedule.custom")}function wM(e,t,a){let n=oa(t),r=SM(e.recent_runs,t,a),s=r[0]||null,i=r.find(m=>m.status==="running")||null,o=r.find(m=>m.status==="ok"||m.status==="error")||null,l=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null,d={...e,recent_runs:r,has_running_run:r.some(m=>m.status==="running"),has_failed_runs:r.some(m=>m.status==="error")};return{...d,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:$M(e.source,t,a),state_label:i_(e.state,t),state_tone:o_(e.state),primary_status_label:pM(d,t),primary_status_tone:hM(d),next_run_timestamp:Hh(e.next_run_at),next_run_label:Yr(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:Yr(c,n("automations.date.noRuns"),a),last_status_label:vM(l,t),last_status_tone:gM(l),created_label:Yr(e.created_at,n("automations.date.unknown"),a),latest_run:s,current_run:i,success_rate_label:_M(r,t)}}function SM(e,t,a){let n=oa(t);return Array.isArray(e)?e.map(r=>{let s=xd(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=Hh(i);return{...r,status:s,status_label:yM(s,t),status_tone:bM(s),timestamp:o,timestamp_source:i,fired_label:Yr(i,n("automations.date.unscheduled"),a),submitted_label:Yr(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:Yr(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function xd(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function l_(e){let t=Array.isArray(e)?e:[],a={total:t.length,ok:0,error:0,running:0,unknown:0};for(let n of t){let r=xd(n?.status);Object.prototype.hasOwnProperty.call(a,r)?a[r]+=1:a.unknown+=1}return a}function NM(e){let t=l_(e);return[{key:"ok",tone:"text-emerald-300",count:t.ok},{key:"error",tone:"text-red-300",count:t.error},{key:"running",tone:"text-sky-300",count:t.running},{key:"unknown",tone:"text-iron-400",count:t.unknown}].filter(a=>a.count>0)}function u_(e,t){let a=oa(t),n=l_(e),r=NM(e).map(s=>({...s,text:a(`automations.runs.${s.key}`,{count:s.count})}));return{total:n.total,totalText:a("automations.runs.total",{count:n.total}),chips:r}}function _M(e,t){let a=oa(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function RM(e,t){let a=Tn(e),n=Tn(t);return a!==n?a?-1:1:(qh(e)??Number.MAX_SAFE_INTEGER)-(qh(t)??Number.MAX_SAFE_INTEGER)}function Hh(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function Tn(e){return e?.state==="active"||e?.state==="scheduled"}function kM(e){return["paused","disabled","inactive"].includes(e?.state)}function CM(e){return e?.state==="completed"}function qh(e){return e?.next_run_timestamp??Hh(e?.next_run_at)}function Kh(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function EM(e,t,a){return!mr(e,0,23)||!mr(t,0,59)?null:Kh(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function TM(e,t){return Kh(t,{weekday:"long"},new Date(2001,0,7+e))}function AM(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return Kh(n,r,new Date(a??2e3,e-1,t))}function DM(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&ZN(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&ZN(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function ZN(e){return/^0+$/.test(e)}function mr(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function MM(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function OM(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}var LM=8;function Qh(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function $d({runs:e=[]}){let t=k(),a=Array.isArray(e)?e:[],n=a.slice(0,LM);if(!n.length)return u`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;let r=a.length-n.length,s=`+${Math.min(r,999)}`;return u`
    <div
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
    >
      ${n.map(i=>u`
        <span
          key=${Qh(i)}
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
  `}function wd({runs:e=[],className:t=""}){let a=k(),n=u_(e,a);return n.total?u`
    <div className=${J("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]",t)}>
      <span className="text-iron-300">${n.totalText}</span>
      ${n.chips.map(r=>u`<span key=${r.key} className=${r.tone}>${r.text}</span>`)}
    </div>
  `:u`<span className=${J("text-[11px] text-iron-400",t)}>
      ${a("automations.table.noRuns")}
    </span>`}function c_({run:e,onOpenRun:t,onOpenLogs:a}){let n=k(),r=!!e.chat_path,s=ad({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return u`
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
  `}function Sd({label:e,value:t,tone:a}){return u`
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
  `}function d_({automation:e,isMutating:t=!1,onPauseAutomation:a,onResumeAutomation:n,onDeleteAutomation:r}){let s=k(),i=ve();if(!e)return u`
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
                    <${c_}
                      key=${Qh(y)}
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
  `}var PM=["automations.empty.example1","automations.empty.example2","automations.empty.example3"];function UM({promptKey:e}){let t=k(),a=t(e),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>()=>clearTimeout(s.current),[]),u`
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
  `}function m_(){let e=k(),t=ve();return u`
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
            ${PM.map(a=>u`<${UM} key=${a} promptKey=${a} />`)}
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
  `}function f_({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,isMutating:s,selectedAutomationId:i,onSelectAutomation:o,onPauseAutomation:l,onResumeAutomation:c,onDeleteAutomation:d}){let m=k(),f=r_(e,t),h=e.length>0,x=f.find(y=>y.automation_id===i)||f[0]||null;return u`
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
              ${Ih.map(y=>u`
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
                                <${$d} runs=${y.recent_runs} />
                                <${wd} runs=${y.recent_runs} />
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

              <${d_}
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
            `:u`<${m_} />`}
    </div>
  `}function p_({summary:e,activeFilter:t,onSelectFilter:a}){let n=k(),r=[{key:"scheduled",label:n("automations.summary.scheduled"),value:e?.scheduled??0,tone:"muted",detail:n("automations.summary.scheduledDetail"),filter:"all"},{key:"active",label:n("automations.summary.active"),value:e?.active??0,tone:"signal",detail:n("automations.summary.activeDetail"),filter:"active"},{key:"running",label:n("automations.summary.running"),value:e?.running??0,tone:"info",detail:n("automations.summary.runningDetail"),filter:"running"},{key:"failures",label:n("automations.summary.failures"),value:e?.failures??0,tone:(e?.failures??0)>0?"danger":"success",detail:n("automations.summary.failuresDetail"),filter:(e?.failures??0)>0?"failures":null},{key:"nextRun",label:n("automations.summary.nextRun"),value:e?.nextRun||n("automations.summary.none"),tone:"info",detail:n("automations.summary.nextRunDetail"),valueClassName:"text-lg md:text-xl"}];return u`
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
  `}function jM(e){return e==="active"||e==="scheduled"}function FM(e){return Number.isFinite(e)?e:null}function h_(e,t=Date.now()){let a=Array.isArray(e)?e:[],n=null;for(let r of a){if(!r||(r.has_running_run&&(n=n==null?5e3:Math.min(n,5e3)),!jM(r.state)))continue;let s=FM(r.next_run_timestamp);if(s==null)continue;let i=s-t,o=i<=0?2e3:i<3e4?Math.max(1e3,i+1200):null;o!=null&&(n=n==null?o:Math.min(n,o))}return n}var BM=50,qM=25;function v_(e=!1){let{t,lang:a}=$l(),n=Z(),r=K({queryKey:["automations",{includeCompleted:e}],queryFn:()=>e$({limit:BM,runLimit:qM,includeCompleted:e}),refetchInterval:3e4,refetchIntervalInBackground:!1}),s=p.default.useMemo(()=>n_(r.data,t,a),[r.data,t,a]),i=p.default.useMemo(()=>s_(s),[s]),o=p.default.useMemo(()=>h_(s),[s]);p.default.useEffect(()=>{if(o==null)return;let h=setTimeout(()=>{r.refetch()},o);return()=>clearTimeout(h)},[o,r.refetch]);let l=r.data?.scheduler_enabled!==!1,c=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["automations"]})},[n]),d=Y({mutationFn:h=>t$({automationId:h}),onSuccess:c}),m=Y({mutationFn:h=>a$({automationId:h}),onSuccess:c}),f=Y({mutationFn:h=>n$({automationId:h}),onSuccess:c});return{automations:s,summary:i,schedulerEnabled:l,isLoading:r.isLoading,isRefreshing:r.isFetching,isMutating:d.isPending||m.isPending||f.isPending,error:r.error||null,actionError:d.error||m.error||f.error||null,pauseAutomation:d.mutate,resumeAutomation:m.mutate,deleteAutomation:f.mutate,refetch:r.refetch}}var g_=["outbound-delivery","preferences"],y_=["outbound-delivery","targets"];function b_(){let e=Z(),t=K({queryKey:g_,queryFn:o$}),a=K({queryKey:y_,queryFn:l$}),n=Y({mutationFn:({finalReplyTargetId:i})=>u$({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(g_,i),e.invalidateQueries({queryKey:y_})}}),r=p.default.useMemo(()=>a.data?.targets??[],[a.data]),s=p.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function x_(){let e=k(),[t,a]=p.default.useState("all"),[n,r]=p.default.useState(null),i=v_(t==="completed"),o=b_(),[l,c]=p.default.useState(!1),d=p.default.useRef(null);p.default.useEffect(()=>()=>clearTimeout(d.current),[]);let m=p.default.useCallback(()=>{c(!0),clearTimeout(d.current),d.current=setTimeout(()=>c(!1),1e3),i.refetch()},[i.refetch]),f=i.isRefreshing||l,h=i.error&&!i.isLoading&&i.automations.length===0;return p.default.useEffect(()=>{if(!i.automations.length){r(null);return}i.automations.some(y=>y.automation_id===n)||r(i.automations[0].automation_id)},[i.automations,n]),u`
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
                <${p_}
                  summary=${i.summary}
                  activeFilter=${t}
                  onSelectFilter=${a}
                />
                <${WN} deliveryState=${o} />

                ${i.isLoading?u`
                      <div className="space-y-4">
                        ${[1,2,3].map(x=>u`<div
                              key=${x}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`)}
                      </div>
                    `:u`
                      <${f_}
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
  `}var $_={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function w_({result:e,onDismiss:t}){return p.default.useEffect(()=>{if(!e)return;let a=setTimeout(t,4e3);return()=>clearTimeout(a)},[e,t]),e?u`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",$_[e.type]||$_.info].join(" ")}>
      <${D}
        name=${e.type==="success"?"check":e.type==="error"?"close":"bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">
        <${D} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `:null}var N_="/api/webchat/v2/channels/slack/setup";function __(){return V(N_)}function R_(e){let t={installation_id:String(e.installation_id||"").trim(),team_id:String(e.team_id||"").trim(),api_app_id:String(e.api_app_id||"").trim(),user_id:S_(e.user_id),shared_subject_user_id:S_(e.shared_subject_user_id)},a=String(e.bot_token||"").trim(),n=String(e.signing_secret||"").trim();return a&&(t.bot_token=a),n&&(t.signing_secret=n),V(N_,{method:"PUT",body:JSON.stringify(t)})}function Vh(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function S_(e){let t=String(e||"").trim();return t||null}var k_="/api/webchat/v2/channels/slack/allowed",IM="/api/webchat/v2/channels/slack/subjects";function C_(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function E_(){return V(k_)}function T_(){return V(IM)}function A_(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return V(k_,{method:"PUT",body:JSON.stringify(n)})}function D_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var M_=["slack-allowed-channels"];function L_({action:e}){let t=k(),a=Z(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState([]),c=KM(e,t),d=K({queryKey:M_,queryFn:E_}),m=K({queryKey:["slack-routable-subjects"],queryFn:T_}),f=m.data?.subjects||[],h=O_(f),x=m.isSuccess||m.isError,y=f.length>0;p.default.useEffect(()=>{d.data&&l(Gh(d.data.channels||[]))},[d.data]);let $=Y({mutationFn:({channels:C})=>A_(C),onSuccess:C=>{l(Gh(C.channels||[])),a.invalidateQueries({queryKey:M_}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let C=n.trim();!C||!m.isSuccess||(l(R=>Gh([...R,{channel_id:C,subject_user_id:s}])),r(""))},v=C=>{l(R=>R.filter(_=>_.channel_id!==C))},b=(C,R)=>{l(_=>_.map(M=>M.channel_id===C?{...M,subject_user_id:R}:M))},w=()=>{$.mutate({channels:HM(o)})},S=m.isError&&o.some(C=>!C.subject_user_id);return u`
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
                      ${O_(f,C).map(R=>u`
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
          ${D_($.error||d.error||m.error,c.errorMessage)}
        </p>`}
      </div>
    </div>
  `}function O_(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function Gh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return C_(Array.from(t.keys())).map(a=>t.get(a))}function HM(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function KM(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var Yh=["slack-setup"],Jr={installationId:{body:"Local IronClaw name for this Slack install. Choose one and keep it stable.",example:"Example: local-slack"},teamId:{body:"Slack workspace/team ID from the workspace that installed the app.",example:"Example: T0123456789"},appId:{body:"Slack app Basic Information > App Credentials.",example:"Example: A0123456789"},botUser:{body:"Optional Reborn user. Blank uses the current WebUI operator.",example:"Example: user:operator"},sharedSubject:{body:"Optional default team agent for shared channel turns. Usually blank.",example:"Example: user:slack-shared"},botToken:{body:"Slack app OAuth & Permissions > Bot User OAuth Token.",example:"Example: xoxb-..."},signingSecret:{body:"Slack app Basic Information > App Credentials > Signing Secret.",example:""}};function j_({action:e}){let t=K({queryKey:Yh,queryFn:__}),a=t.data?.configured===!0;return u`
    <div className="space-y-3">
      <${QM} action=${e} setupQuery=${t} />
      ${a&&u`<${L_} action=${e} />`}
    </div>
  `}function QM({action:e,setupQuery:t}){let a=Z(),[n,r]=p.default.useState(VM()),s=p.default.useRef(!1),i=p.default.useRef(!1),o=t.data,l=GM(e);p.default.useEffect(()=>{!o||s.current||i.current||(r(P_(o)),s.current=!0)},[o]);let c=Y({mutationFn:R_,onSuccess:h=>{i.current=!1,r(P_(h)),s.current=!0,a.setQueryData(Yh,h),a.invalidateQueries({queryKey:Yh}),a.invalidateQueries({queryKey:["slack-allowed-channels"]}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["extensions"]})}}),d=h=>x=>{i.current=!0,r(y=>({...y,[h]:x.target.value}))},m=()=>c.mutate(n),f=n.installation_id.trim()&&n.team_id.trim()&&n.api_app_id.trim()&&(o?.bot_token_configured||n.bot_token.trim())&&(o?.signing_secret_configured||n.signing_secret.trim());return u`
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
        ${yl("Installation ID",n.installation_id,d("installation_id"),"",Jr.installationId)}
        ${yl("Team ID",n.team_id,d("team_id"),"",Jr.teamId)}
        ${yl("App ID",n.api_app_id,d("api_app_id"),"",Jr.appId)}
        ${yl("Bot user",n.user_id,d("user_id"),"default operator",Jr.botUser)}
        ${yl("Shared subject",n.shared_subject_user_id,d("shared_subject_user_id"),"optional",Jr.sharedSubject)}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        ${U_("Bot token",n.bot_token,d("bot_token"),o?.bot_token_configured,Jr.botToken)}
        ${U_("Signing secret",n.signing_secret,d("signing_secret"),o?.signing_secret_configured,Jr.signingSecret)}
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
          ${Vh(t.error,l.errorMessage)}
        </p>`}
        ${c.isError&&u`<p className="text-xs text-red-300">
          ${Vh(c.error,l.errorMessage)}
        </p>`}
        ${c.isSuccess&&u`<p className="text-xs text-emerald-300">${l.successMessage}</p>`}
      </div>
    </div>
  `}function P_(e){return{installation_id:e.installation_id||"",team_id:e.team_id||"",api_app_id:e.api_app_id||"",user_id:e.user_id||"",shared_subject_user_id:e.shared_subject_user_id||"",bot_token:"",signing_secret:""}}function VM(){return{installation_id:"",team_id:"",api_app_id:"",user_id:"",shared_subject_user_id:"",bot_token:"",signing_secret:""}}function yl(e,t,a,n="",r=null){return u`
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
  `}function U_(e,t,a,n,r=null){return u`
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
  `:null}function GM(e){return{title:"Slack setup",instructions:e?.instructions||"Configure the Slack app before assigning channels.",submitLabel:"Save setup",successMessage:"Slack setup saved.",errorMessage:"Slack setup update failed."}}var Jh={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function fr(e){return e==="wasm_channel"||e==="channel"}var z_={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},B_={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function q_(e){let t=I_(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||fr(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function I_(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function Xh(e){let t=I_(e);return t==="active"||t==="ready"}function H_({extension:e,secrets:t=[],fields:a=[]}={}){return Xh(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var K_="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",Q_="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",V_="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",G_="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",Y_="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",YM="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function J_(e){return e.package_ref?.id||""}function JM({actions:e,isBusy:t}){let a=k(),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>{if(!n)return;let i=o=>{s.current&&!s.current.contains(o.target)&&r(!1)};return document.addEventListener("mousedown",i),()=>document.removeEventListener("mousedown",i)},[n]),u`
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
      ${e.map(t=>u`<span key=${t} className=${YM}>${t}</span>`)}
    </div>
  `}function Ni({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=k(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=z_[i]||"muted",l=s(`extensions.state.${i}`)||B_[i]||i,c=s(`extensions.kind.${e.kind}`)||Jh[e.kind]||e.kind,d=e.display_name||J_(e),m=!!e.package_ref,f=e.tools||[],[h,x]=p.default.useState(!1),$=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],b=[],w=q_(e);w==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):w==="activate"&&v.push({id:"activate",label:s("extensions.activate"),run:()=>t(g)}),m&&(e.needs_setup||e.has_auth)&&w!=="configure"&&b.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)});let S=b.some(R=>R.id==="configure");m&&w!=="configure"&&fr(e.kind)&&(i==="setup_required"||i==="failed")&&b.push({id:"setup",label:s("extensions.setup"),icon:"settings",run:()=>a(g)}),m&&fr(e.kind)&&!S&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&b.push({id:"reconfigure",label:s("extensions.reconfigure"),icon:"settings",run:()=>a(g)}),m&&b.push({id:"remove",label:s("common.remove"),icon:"trash",danger:!0,run:()=>n(g)});let C=v[0];return u`
    <div className=${K_}>
      <div className="flex items-start gap-2">
        <${I} tone=${o} label=${l} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${b.length>0&&u`<${JM} actions=${b} isBusy=${r} />`}
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
  `}function Xr({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=k(),s=r(`extensions.kind.${e.kind}`)||Jh[e.kind]||e.kind,i=e.display_name||J_(e),o=!!(e.package_ref&&t),l=!!(e.needs_setup||e.has_auth||fr(e.kind)),c=e.keywords||[],[d,m]=p.default.useState(!1);return u`
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
  `}var XM="/api/webchat/v2/extensions/pairing/redeem";function W_(e,t){return V(XM,{method:"POST",body:JSON.stringify({channel:e,code:t})}).then(a=>({success:!0,provider:a.provider,provider_user_id:a.provider_user_id}))}function Z_(){return V("/api/webchat/v2/extensions")}function eR(){return V("/api/webchat/v2/extensions/registry")}function tR(e){return V("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function aR(e){return V(`/api/webchat/v2/extensions/${encodeURIComponent(bl(e))}/activate`,{method:"POST"})}function nR(e){return V(`/api/webchat/v2/extensions/${encodeURIComponent(bl(e))}/remove`,{method:"POST"})}function rR(e){return V(`/api/webchat/v2/extensions/${encodeURIComponent(bl(e))}/setup`)}function sR(e,t,a){return g$(bl(e),{action:"submit",payload:{secrets:t,fields:a}})}function iR(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return V(`/api/webchat/v2/extensions/${encodeURIComponent(bl(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function oR(){return Promise.resolve({requests:[]})}function lR(e,t){return W_(e,t)}function bl(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var WM=2e3,ZM=10*60*1e3;function cR(e){try{return new URL(e).protocol==="https:"}catch{return!1}}function Nd(e,t=null){return cR(e)?t&&!t.closed?(t.location.href=e,{ok:!0,popup:t}):{ok:!0,popup:window.open(e,"_blank","noopener,noreferrer")}:{ok:!1,popup:null}}function _i(e){return e?.package_ref?.id||null}function Wh(e){return e?.display_name||_i(e)||""}function uR(e,t,a){return _i(t)||`${e}:${Wh(t)||"unknown"}:${a}`}function eO(e,t){return e.installed!==t.installed?e.installed?-1:1:Wh(e.entry||e.extension).localeCompare(Wh(t.entry||t.extension))}function dR(){let e=k(),t=Z(),a=K({queryKey:["gateway-status-extensions"],queryFn:si,staleTime:1e4}),n=K({queryKey:["extensions"],queryFn:Z_,refetchOnMount:"always"}),r=K({queryKey:["extension-registry"],queryFn:eR,refetchOnMount:"always"}),s=K({queryKey:["connectable-channels"],queryFn:Xc,refetchOnMount:"always"}),i=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["gateway-status-extensions"]}),t.invalidateQueries({queryKey:["connectable-channels"]})},[t]),[o,l]=p.default.useState(null),c=p.default.useCallback(()=>l(null),[]),d=Y({mutationFn:({packageRef:P})=>tR(P),onSuccess:(P,{displayName:G,configureAfterInstall:ae,onNeedsSetup:le,packageRef:lt})=>{P.success?(l({type:"success",message:P.message||P.instructions||e("extensions.installedSuccess",{name:G||e("extensions.defaultName")})}),P.auth_url&&!Nd(P.auth_url).ok?l({type:"error",message:"Authentication URL must use HTTPS."}):!P.auth_url&&ae&&typeof le=="function"&&le({packageRef:lt,displayName:G,active:!1,activationStatus:"setup_required",onboardingState:"setup_required"})):l({type:"error",message:P.message||e("extensions.installFailed")}),i()},onError:P=>{l({type:"error",message:P.message}),i()}}),m=Y({mutationFn:({packageRef:P})=>aR(P),onSuccess:(P,{displayName:G})=>{P.success?(l({type:"success",message:P.message||P.instructions||e("extensions.activatedSuccess",{name:G||e("extensions.defaultName")})}),P.auth_url&&!Nd(P.auth_url).ok&&l({type:"error",message:"Authentication URL must use HTTPS."})):P.auth_url?Nd(P.auth_url).ok?l({type:"info",message:e("extensions.openingAuth")}):l({type:"error",message:"Authentication URL must use HTTPS."}):P.awaiting_token?l({type:"info",message:e("extensions.configurationRequired")}):l({type:"error",message:P.message||e("extensions.activationFailed")}),i()},onError:P=>{l({type:"error",message:P.message})}}),f=Y({mutationFn:({packageRef:P})=>nR(P),onSuccess:(P,{displayName:G})=>{P.success?l({type:"success",message:e("extensions.removedSuccess",{name:G||e("extensions.defaultName")})}):l({type:"error",message:P.message||e("extensions.removeFailed")}),i()},onError:P=>{l({type:"error",message:P.message})}}),h=a.data||{},x=n.data?.extensions||[],y=r.data?.entries||[],$=s.data?.channels||[],g=new Map(x.map(P=>[_i(P),P]).filter(([P])=>!!P)),v=new Set(y.map(P=>_i(P)).filter(Boolean)),b=[...y.map((P,G)=>{let ae=_i(P),le=ae&&g.get(ae)||null;return{id:uR("registry",P,G),installed:!!(le||P.installed),entry:P,extension:le}}),...x.filter(P=>{let G=_i(P);return!G||!v.has(G)}).map((P,G)=>({id:uR("installed",P,G),installed:!0,entry:null,extension:P}))].sort(eO),w=P=>fr(P.kind),S=x.filter(w),C=x.filter(P=>P.kind==="mcp_server"),R=x.filter(P=>!w(P)&&P.kind!=="mcp_server"),_=y.filter(P=>w(P)&&!P.installed),M=y.filter(P=>P.kind==="mcp_server"&&!P.installed),L=y.filter(P=>P.kind!=="mcp_server"&&!w(P)&&!P.installed),U=n.isLoading||r.isLoading,F=d.isPending||m.isPending||f.isPending,z=p.default.useCallback(P=>{let G=P?.displayName||P?.packageRef?.id||"this extension";window.confirm(`Remove ${G}?`)&&f.mutate(P)},[f]);return{status:h,extensions:x,channels:S,mcpServers:C,tools:R,channelRegistry:_,mcpRegistry:M,toolRegistry:L,registry:y,catalogEntries:b,connectableChannels:$,isLoading:U,isBusy:F,actionResult:o,clearResult:c,install:d.mutate,activate:m.mutate,remove:z,invalidate:i}}function mR(e){let t=K({queryKey:["extension-setup",e?.id||e],queryFn:()=>rR(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function fR(e,t){let a=Z(),n=e?.id||e;return Y({mutationFn:({secrets:r,fields:s})=>sR(e,r,s).then(i=>{if(i.success===!1)throw new Error(i.message||"Setup failed");return i}),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function pR(e){let t=Z(),a=e?.id||e,n=p.default.useRef(null),r=p.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=p.default.useCallback(()=>{let l=t.getQueryData(["extension-setup",a]);if(l?.secrets?.length>0&&l.secrets.every(f=>f.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(f=>f.package_ref?.id===a),m=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return m==="active"||m==="ready"},[a,t]),o=p.default.useCallback(l=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||l&&l.closed||Date.now()-c>ZM)&&(r(),s())},WM)},[r,s,i]);return p.default.useEffect(()=>r,[r]),Y({mutationFn:({secret:l,popup:c})=>iR(e,l).then(d=>{if(d.success===!1)throw new Error(d.message||"OAuth setup failed");if(d.authorization_url&&!cR(d.authorization_url))throw new Error("Authorization URL must use HTTPS.");return{res:d,popup:c}}),onSuccess:({res:l,popup:c})=>{let d=c;l.authorization_url?d=Nd(l.authorization_url,c).popup:c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(l,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function hR(e,t={}){let a=K({queryKey:["pairing",e],queryFn:()=>oR(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=Z(),r=Y({mutationFn:({code:s})=>lR(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function vR(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var tO={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function gR({channel:e,redeemFn:t,i18nKeys:a=tO,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=k(),o=typeof t=="function",l=hR(e,{enabled:!o}),c=Z(),[d,m]=p.default.useState(""),f=aO(i,a,r),h=Y({mutationFn:({code:S})=>t(e,S),onSuccess:()=>{m("");for(let S of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:S})}}),x=p.default.useCallback(S=>l.approve({code:S}),[l.approve]),y=p.default.useCallback(()=>{let S=d.trim().toUpperCase();S&&(o?h.mutate({code:S}):l.approve({code:S}))},[o,d,l.approve,h]),$=o?[]:l.requests,g=o?!1:l.isLoading,v=o?h.isPending:l.isApproving,b=o?h.isSuccess?h.data:null:l.result,w=o?h.isError?h.error:null:l.error;return p.default.useEffect(()=>{b?.success&&m("")},[b?.success]),g?u`
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
  `}function aO(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function _d(e){return e.package_ref?.id||""}function yR(e){return _d(e)==="slack"}function xR(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function $R(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function nO(e){let t=e||[],a=[t.find(xR),t.find($R)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function bR({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>xR(r)?u`<${j_} action=${r.action} />`:$R(r)?u`<${Kc} action=${r.action} />`:null).filter(Boolean);return n.length>0?u`<div className="space-y-3">${n}</div>`:null}function wR({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:l}){let c=k(),d=t||[],m=e.enabled_channels||[],f=nO(a),h=d.some(yR),x=f.length>0&&!h;return u`
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
        ${x&&u`
          <${Ri}
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
                <div key=${_d(y)} className="flex flex-col gap-3">
                  <${Ni}
                    ext=${y}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${l}
                  />
                  ${yR(y)&&u`<${bR}
                    slackConnectActions=${f}
                  />`}
                  ${(y.onboarding_state==="pairing_required"||y.onboarding_state==="pairing")&&u` <${gR} channel=${_d(y)} /> `}
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
                  key=${_d(y)}
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
  `}function SR({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=k(),s=e?.displayName||e?.packageRef?.id||r("extensions.defaultName"),{secrets:i=[],fields:o=[],onboarding:l,isLoading:c,error:d}=mR(e?.packageRef),[m,f]=p.default.useState({}),[h,x]=p.default.useState({}),y=pR(e?.packageRef),$=fR(e?.packageRef,_=>{_.success!==!1&&(n&&n(_),a())}),g=p.default.useCallback(()=>{let _={};for(let[M,L]of Object.entries(m)){let U=(L||"").trim();U&&(_[M]=U)}$.mutate({secrets:_,fields:h})},[m,h,$]),v=p.default.useCallback(_=>{let M=window.open("about:blank","_blank","width=600,height=600");M&&(M.opener=null),y.mutate({secret:_,popup:M})},[y]),w=i.filter(_=>(_.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,S=Xh(e),C=H_({extension:e,secrets:i,fields:o}),R=rO(l?.setup_url);return c?u`
      <${Rd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(_=>u`<div
                key=${_}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?u`
      <${Rd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?u`
      <${Rd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")}
        </p>
      <//>
    `:u`
    <${Rd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
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
                onChange=${M=>f(L=>({...L,[_.name]:M.target.value}))}
                onKeyDown=${M=>M.key==="Enter"&&g()}
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
                onChange=${M=>x(L=>({...L,[_.name]:M.target.value}))}
                onKeyDown=${M=>M.key==="Enter"&&g()}
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
  `}function rO(e){if(!e)return null;try{let t=new URL(String(e));return t.protocol==="https:"?t.href:null}catch{return null}}function Rd({onClose:e,title:t,children:a}){let n=p.default.useId();return p.default.useEffect(()=>{let r=s=>{s.key==="Escape"&&e()};return window.addEventListener("keydown",r),()=>window.removeEventListener("keydown",r)},[e]),u`
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
                <${Ni}
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
  `}function sO(e){return e?.package_ref?.id||""}function iO(e){return e.entry||e.extension||{}}function RR({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=k(),[o,l]=p.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(y=>{let $=iO(y);return($.display_name||sO($)).toLowerCase().includes(c)||($.description||"").toLowerCase().includes(c)||($.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,m=d.filter(y=>y.installed&&y.extension),f=d.filter(y=>y.installed&&!y.extension&&y.entry),h=m.length+f.length,x=d.filter(y=>!y.installed&&y.entry);return e.length===0?u`
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
  `}function Zh(){let{tab:e="registry"}=it(),[t,a]=p.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:l,connectableChannels:c,isLoading:d,isBusy:m,actionResult:f,clearResult:h,install:x,activate:y,remove:$,invalidate:g}=dR(),v=p.default.useCallback(_=>a(_),[]),b=p.default.useCallback(_=>x({..._,onNeedsSetup:v}),[v,x]),w=p.default.useCallback(()=>a(null),[]),S=p.default.useCallback(()=>g(),[g]),C=p.default.useCallback(_=>{_&&(y(_),a(null))},[y]);if(d)return u`
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
          <${w_} result=${f} onDismiss=${h} />
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
  `:u`<${ot} to="/extensions/registry" replace />`}var kR=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],CR=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],ER=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],ev=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function TR(e){return String(e||"").trim().toLowerCase()}function AR(e){if(e==null)return"";if(Array.isArray(e))return e.map(AR).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function tt(e,t){let a=TR(e);return a?t.map(AR).join(" ").toLowerCase().includes(a):!0}function ki(e,t,a,n){let r=TR(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(l=>tt(r,[i,l.key,l.labelKey?n(l.labelKey):l.label,l.descKey?n(l.descKey):l.description,t[l.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function oO({visible:e}){let t=k();return e?u`
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      ${t("tools.saved")}
    </span>
  `:null}function lO({checked:e,onChange:t,label:a}){return u`
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
  `}function uO({field:e,value:t,onSave:a,isSaved:n}){let r=k(),[s,i]=p.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",l=e.descKey?r(e.descKey):e.description||"";p.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=p.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let m=parseInt(d,10);isNaN(m)||a(e.key,m)}else if(e.type==="float"){let m=parseFloat(d);isNaN(m)||a(e.key,m)}else a(e.key,d)},[e.key,e.type,a]);return u`
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">${o}</div>
        ${l&&u`<div className="mt-1 text-xs leading-5 text-iron-300">${l}</div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${e.type==="boolean"?u`
              <${lO}
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
        <${oO} visible=${n} />
      </div>
    </div>
  `}function Ci({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=k(),o=t?i(t):e||"";return u`
    <${ne} className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${o}</h3>
      <div>
        ${a.map(l=>u`
              <${uO}
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
  `}function DR({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=k();if(n)return u`<${cO} />`;let i=ki(CR,e,r,s);return i.length===0?u`<${Rt} query=${r} />`:u`
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
  `}function cO(){return u`
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
  `}function MR(){let e=K({queryKey:["gateway-status-settings"],queryFn:si,staleTime:1e4}),t=K({queryKey:["extensions"],queryFn:dw}),a=K({queryKey:["extension-registry"],queryFn:mw}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(m=>m.kind==="wasm_channel"||m.kind==="channel"),o=s.filter(m=>(m.kind==="wasm_channel"||m.kind==="channel")&&!m.installed),l=r.filter(m=>m.kind==="mcp_server"),c=s.filter(m=>m.kind==="mcp_server"&&!m.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:l,mcpRegistry:c,extensions:r,isLoading:d}}function dO({name:e,description:t,enabled:a,detail:n}){let r=k();return u`
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
  `}function OR({channel:e,registryEntry:t}){let a=k(),n=t?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},l={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return u`
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
  `}function mO(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function fO({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=mO(e,i).filter(x=>tt(s,[i("channels.builtIn"),x.id,x.name,x.description,x.detail])),l=new Set(t.map(x=>x.name)),c=t.filter(x=>tt(s,[i("channels.messaging"),x.name,x.display_name,x.description,x.onboarding_state])),d=a.filter(x=>!l.has(x.name)).filter(x=>tt(s,[i("channels.messaging"),x.name,x.display_name,x.description])),m=new Set(n.map(x=>x.name)),f=n.filter(x=>tt(s,[i("channels.mcpServers"),x.name,x.display_name,x.description,x.active?i("channels.active"):i("channels.inactive")])),h=r.filter(x=>!m.has(x.name)).filter(x=>tt(s,[i("channels.mcpServers"),x.name,x.display_name,x.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:h}}function LR({searchQuery:e=""}){let t=k(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=MR();if(o)return u`
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
    `;let{builtInChannels:l,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:f}=fO({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return l.length===0&&c.length===0&&d.length===0&&m.length===0&&f.length===0?u`<${Rt} query=${e} />`:u`
    <div className="space-y-5">
      ${l.length>0&&u`
      <${ne} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${l.map(h=>u`
            <${dO}
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
  `}function PR({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:l,onNearaiWallet:c,onCodexLogin:d,loginBusy:m}){let f=k(),h=e.id===t,x=Qr(e,n),y=ui(e,n),$=_w(e,n,t,a),g=Mc(e,n),v=Rw(e),b=f(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[w,S]=p.default.useState(h),C=p.default.useCallback(()=>S(lt=>!lt),[]);p.default.useEffect(()=>{S(h)},[h]);let R=x?u`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${il(e.adapter)} · ${$||e.default_model||f("llm.none")}
      </span>`:u`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${b}
      </span>`,_=e.id==="nearai"||e.id==="openai_codex",M=e.api_key_set===!0||e.has_api_key===!0,L=e.builtin?e.id==="nearai"&&v&&!M?f("llm.addApiKey"):f("llm.configure"):f("common.edit"),U=v&&e.builtin?u`
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
  `}var pO=[{key:"active",labelKey:"llm.groupActive",dotClass:"bg-[var(--v2-positive-text)]"},{key:"ready",labelKey:"llm.groupReady",dotClass:"bg-[var(--v2-accent)]"},{key:"setup",labelKey:"llm.groupSetup",dotClass:"bg-[var(--v2-warning-text)]"}];function hO({label:e,count:t,dotClass:a}){return u`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full "+a} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${t}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `}function UR({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=k(),r=sd({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=id(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return u`<${Rt} query=${a} />`;let l=kw(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return u`
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

      <${rd} login=${i} />

      ${s.isLoading?u`<div className="text-sm text-[var(--v2-text-muted)]">${n("common.loading")}</div>`:s.error?u`<div className="text-sm text-red-200">${n("error.loadFailed",{what:n("llm.providers"),message:s.error.message})}</div>`:u`
            <div className="space-y-1">
              ${pO.flatMap(c=>{let d=l[c.key];return d.length?[u`
                    <section
                      key=${c.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${c.key}
                      className="mb-3"
                    >
                      <${hO}
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
  `}function jR({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=k(),{activeProviderId:o,selectedModel:l,providers:c,hasActiveProvider:d}=ci({settings:e,gatewayStatus:t});if(r)return u`<${vO} />`;let m=d?o:"",f=c.find(g=>g.id===o),h=d&&(l||f?.default_model||e.selected_model)||"",x=ki(kR,e,s,i),y=tt(s,[i("inference.provider"),i("inference.backend"),m,i("inference.model"),h]),$=tt(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!y&&!$&&x.length===0?u`<${Rt} query=${s} />`:u`
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
  `}function pr({className:e=""}){return u`
    <div
      className=${"rounded animate-pulse bg-[var(--v2-surface-muted)] "+e}
    />
  `}function vO(){return u`
    <div className="space-y-5">
      <${ne} padding="md">
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
      ${[1,2].map(e=>u`
            <${ne} key=${e} padding="md">
              <${pr} className="mb-4 h-3 w-20" />
              ${[1,2,3].map(t=>u`
                    <div key=${t} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                      <${pr} className="h-4 w-32" />
                      <${pr} className="h-9 w-36" />
                    </div>
                  `)}
            <//>
          `)}
    </div>
  `}function FR({searchQuery:e=""}){let t=k(),{lang:a,setLang:n}=$l(),r=wl.find(i=>i.code===a)||wl[0],s=wl.filter(i=>tt(e,[i.code,i.name,i.native]));return s.length===0?u`<${Rt} query=${e} />`:u`
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
  `}function zR({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=k();if(n)return u`
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
    `;let i=ki(ER,e,r,s);return i.length===0?u`<${Rt} query=${r} />`:u`
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
  `}function BR(){let e=k(),[t,a]=p.default.useState(!1),n=p.default.useCallback(()=>a(!0),[]),r=p.default.useCallback(()=>a(!1),[]),s=p.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function qR({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=k(),r=BR({gatewayStatus:t,gatewayStatusQuery:a});return e?u`
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
  `:null}function IR(){let e=Z(),t=K({queryKey:["skills"],queryFn:fw}),a=Y({mutationFn:hw,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),n=Y({mutationFn:gw,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),r=Y({mutationFn:({name:c,content:d})=>vw(c,{content:d}),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),s=Y({mutationFn:({name:c,enabled:d})=>yw(c,d),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),i=Y({mutationFn:c=>bw(c),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),o=t.data?.skills||[],l=t.data?.auto_activate_learned!==!1;return{skills:o,query:t,autoActivateLearned:l,fetchSkillContent:pw,installSkill:a.mutateAsync,removeSkill:n.mutateAsync,updateSkill:r.mutateAsync,setSkillAutoActivate:s.mutateAsync,setAutoActivateLearned:i.mutateAsync,isInstalling:a.isPending,isRemoving:n.isPending,isUpdating:r.isPending,isSettingAutoActivate:s.isPending,isSettingAutoActivateLearned:i.isPending}}function HR({skill:e,onEdit:t,onRemove:a,onUpdate:n,onSetAutoActivate:r,isRemoving:s,isUpdating:i,isSettingAutoActivate:o}){let l=k(),c=e.name||e.id,d=e.trust||e.trust_level||"installed",m=e.source_kind||"installed",f=!!e.can_edit,h=!!e.can_delete,x=e.auto_activate!==!1,[y,$]=p.default.useState(!1),[g,v]=p.default.useState(""),[b,w]=p.default.useState(""),[S,C]=p.default.useState(!1);p.default.useEffect(()=>{y||(v(""),w(""))},[y]);let R=p.default.useCallback(async()=>{C(!0),w("");try{let M=await t(c);v(M?.content||""),$(!0)}catch(M){w(M.message||l("skills.contentLoadFailed"))}finally{C(!1)}},[c,t,l]),_=p.default.useCallback(async()=>{(await n(c,g))?.success&&$(!1)},[g,c,n]);return u`
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
                  <${Ic}
                    rows=${12}
                    value=${g}
                    className="font-mono text-xs leading-5"
                    onInput=${M=>v(M.currentTarget.value)}
                  />
                </div>
              `:u`<${gO} skill=${e} />`}
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
  `}function gO({skill:e}){let t=k();return u`
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
        ${e.has_requirements&&u`<${tv}>requirements.txt<//>`}
        ${e.has_scripts&&u`<${tv}>scripts/<//>`}
        ${e.install_source_url&&u`<${tv}>${t("skills.imported")}<//>`}
      </div>
    `}
  `}function tv({children:e}){return u`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${e}
    </span>
  `}function KR({onInstall:e,isInstalling:t}){let a=k(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,l]=p.default.useState({name:"",content:""}),[c,d]=p.default.useState(""),[m,f]=p.default.useState(""),h=p.default.useCallback((y,$)=>{l(g=>!g[y]||!$.trim()?g:{...g,[y]:""})},[]),x=p.default.useCallback(async()=>{let y=yO({name:n,content:s}),$=bO(y,a);if($.name||$.content){l($),d(""),f("");return}l({name:"",content:""}),d(""),f("");try{let g=await e(y);if(!g?.success){d(g?.message||a("skills.installFailed"));return}r(""),i(""),f(g.message||a("skills.installedSuccess",{name:y.name}))}catch(g){d(g.message||a("skills.installFailed"))}},[s,n,e,a]);return u`
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
        <${T} type="button" size="sm" disabled=${t} onClick=${x}>
          <${D} name="upload" className="h-4 w-4" />
          ${a(t?"skills.installing":"skills.install")}
        <//>
      </div>
    <//>
  `}function yO({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function bO(e,t){return{name:e.name?"":t("skills.nameRequired"),content:e.content?"":t("skills.contentRequired")}}function QR({searchQuery:e=""}){let t=k(),{skills:a,query:n,autoActivateLearned:r,fetchSkillContent:s,installSkill:i,removeSkill:o,updateSkill:l,setSkillAutoActivate:c,setAutoActivateLearned:d,isInstalling:m,isRemoving:f,isUpdating:h,isSettingAutoActivate:x,isSettingAutoActivateLearned:y}=IR(),[$,g]=p.default.useState(""),[v,b]=p.default.useState(""),w=p.default.useCallback(async M=>{if(window.confirm(t("skills.confirmDelete",{name:M}))){g(""),b("");try{let L=await o(M);if(!L?.success){g(L?.message||t("skills.removeFailed"));return}b(L.message||t("skills.removed",{name:M}))}catch(L){g(L.message||t("skills.removeFailed"))}}},[o,t]),S=p.default.useCallback(async(M,L)=>{if(!L.trim())return g(t("skills.contentRequired")),b(""),{success:!1,message:t("skills.contentRequired")};g(""),b("");try{let U=await l({name:M,content:L});return U?.success?(b(U.message||t("skills.updated",{name:M})),U):(g(U?.message||t("skills.updateFailed")),U)}catch(U){let F=U.message||t("skills.updateFailed");return g(F),{success:!1,message:F}}},[t,l]),C=p.default.useCallback(async(M,L)=>{g(""),b("");try{let U=await c({name:M,enabled:L});if(!U?.success){g(U?.message||t("skills.updateFailed"));return}b(U.message)}catch(U){g(U.message||t("skills.updateFailed"))}},[c,t]),R=p.default.useCallback(async M=>{g(""),b("");try{let L=await d(M);if(!L?.success){g(L?.message||t("skills.updateFailed"));return}b(L.message)}catch(L){g(L.message||t("skills.updateFailed"))}},[d,t]),_;if(n.isLoading)_=u`
      <${ne} padding="md">
          <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(M=>u`
            <div key=${M} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
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
    `;else{let M=a.filter(U=>tt(e,[U.name,U.id,U.description,U.keywords,U.trust_level,U.source_kind,U.version])),L=wO(M);a.length===0?_=u`
        <${ne} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:M.length===0?_=u`<${Rt} query=${e} />`:_=u`
        <div id="skills-list">
          ${L.map(U=>u`
              <${$O}
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
      <${xO}
        enabled=${r}
        isSaving=${y}
        onToggle=${R}
      />
      <${KR} onInstall=${i} isInstalling=${m} />
      <${SO} error=${$} result=${v} />
      ${_}
    </div>
  `}function xO({enabled:e,isSaving:t,onToggle:a}){let n=k();return u`
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
  `}function $O({title:e,skills:t,onEdit:a,onRemove:n,onUpdate:r,onSetAutoActivate:s,isRemoving:i,isUpdating:o,isSettingAutoActivate:l}){return t.length===0?null:u`
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
  `}function wO(e){let t=[{id:"user",labelKey:"skills.group.user",skills:[]},{id:"system",labelKey:"skills.group.system",skills:[]},{id:"workspace",labelKey:"skills.group.workspace",skills:[]}],a=t[0];for(let n of e){let r=n.source_kind||"";(r==="system"?t[1]:r==="workspace"?t[2]:a).skills.push(n)}return t.filter(n=>n.skills.length>0)}function SO({error:e,result:t}){return!e&&!t?null:u`
    <div
      className=${e?"rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200":"rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200"}
    >
      ${e||t}
    </div>
  `}function kd(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function VR(){let e=Z(),t=K({queryKey:["settings-tools"],queryFn:uw}),a=t.data?.tools||[],[n,r]=p.default.useState({}),s=Y({mutationFn:async({name:o,state:l})=>kd(await cw(o,l),"Save failed"),onSuccess:(o,{name:l,state:c})=>{e.setQueryData(["settings-tools"],d=>{if(!d)return d;let m=o?.tool;return{...d,tools:d.tools.map(f=>f.name===l?{...f,state:c,...m||{}}:f)}}),r(d=>({...d,[l]:!0})),setTimeout(()=>r(d=>({...d,[l]:!1})),2e3)}}),i=p.default.useCallback((o,l)=>s.mutate({name:o,state:l}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}var av="agent.auto_approve_tools";function GR(e,t){let a=`tools.description.${t.name}`,n=e(a);return n&&n!==a?n:t.description||""}function NO({visible:e}){let t=k();return e?u`
    <span className="font-mono text-[11px] text-[var(--v2-accent-text)]" role="status">
      ${t("tools.saved")}
    </span>
  `:null}function _O({checked:e,disabled:t=!1,label:a,onChange:n}){return u`
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
  `}function nv({settings:e,onSave:t,savedKeys:a,isLoading:n}){let r=k(),s=r("settings.field.autoApproveEligibleTools"),i=e?.[av],o=i==null?!0:i===!0||i==="true";return u`
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
        <${NO} visible=${a?.[av]} />
        <${_O}
          checked=${o}
          disabled=${n}
          label=${s}
          onChange=${l=>t(av,l)}
        />
      </div>
    <//>
  `}function RO({tool:e,onPermissionChange:t,isSaved:a}){let n=k(),r=GR(n,e),s=[{value:"default",label:n("tools.followDefault"),tone:"neutral"},{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask_each_time",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],i={default:n("tools.sourceDefault"),global:n("tools.sourceGlobal"),override:n("tools.sourceOverride")},o=e.locked,l=s.find(f=>f.value===e.state)||s[1],c=e.effective_source||"default",d=c==="override"?e.state:"default",m=c==="default"&&e.state===e.default_state;return u`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="flex min-w-0 items-center gap-3">
        ${o&&u`<${D}
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
  `}function YR({settings:e={},onSave:t=()=>{},savedKeys:a={},isLoading:n=!1,searchQuery:r=""}){let s=k(),{tools:i,query:o,setPermission:l,savedTools:c}=VR();if(o.isLoading)return u`
      <div className="space-y-4">
        <${nv}
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
        <${nv}
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
    `;let d=i.filter(m=>{let f=GR(s,m);return tt(r,[m.name,m.description,f,m.state,m.default_state,m.effective_source,m.state==="disabled"?s("tools.disabled"):""])});return u`
    <div className="space-y-4">
      <${nv}
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
                  <${RO}
                    key=${m.name}
                    tool=${m}
                    onPermissionChange=${l}
                    isSaved=${c[m.name]}
                  />
                `)}
      <//>
    </div>
  `}function JR(e){return(Number(e)||0).toFixed(2)}function kO(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function XR(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function Wr({label:e,value:t,description:a}){return u`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&u`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function WR({searchQuery:e=""}){let t=k(),{credits:a,query:n,authorize:r}=Pc();if(!tt(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return u`<${Rt} query=${e} />`;let s;if(n.isLoading)s=u`
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
          value=${JR(a.pending_credit)}
        />
        <${Wr}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${JR(a.final_credit)}
        />
        <${Wr}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${kO(a.delayed_credit_delta)}
        />
        <${Wr}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${Wr}
          label=${t("traceCommons.lastSubmission")}
          value=${XR(a.last_submission_at,t)}
        />
        <${Wr}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${XR(a.last_credit_sync_at,t)}
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
  `}function ZR(){let e=Z(),t=K({queryKey:["admin-users"],queryFn:ww,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=Y({mutationFn:Sw,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=Y({mutationFn:({id:i,payload:o})=>Nw(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function CO({onCreate:e,isCreating:t,error:a}){let n=k(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState("member"),[d,m]=p.default.useState(!1),f=h=>{h.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:l},{onSuccess:()=>{s(""),o(""),m(!1)}})};return d?u`
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
    `}function EO({user:e}){let t=k(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return u`
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
  `}function ek({searchQuery:e=""}){let t=k(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=ZR();if(n.isLoading)return u`
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
      <${CO}
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
            </p>`:l.map(c=>u`<${EO} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function tk(){let e=Z(),t=K({queryKey:["settings-export"],queryFn:ew,staleTime:3e4}),a=t.data?.settings||{},[n,r]=p.default.useState({}),[s,i]=p.default.useState(!1),o=Y({mutationFn:async({key:m,value:f})=>kd(await nh(m,f),"Save failed"),onSuccess:(m,{key:f,value:h})=>{e.setQueryData(["settings-export"],x=>{if(!x)return x;let y={...x,settings:{...x.settings}};return h==null?delete y.settings[f]:y.settings[f]=h,y}),r(x=>({...x,[f]:!0})),setTimeout(()=>r(x=>({...x,[f]:!1})),2e3),ev.has(f)&&i(!0),f==="agent.auto_approve_tools"&&e.invalidateQueries({queryKey:["settings-tools"]})}}),l=p.default.useCallback((m,f)=>o.mutate({key:m,value:f}),[o]),c=Y({mutationFn:tw,onSuccess:(m,f)=>{e.invalidateQueries({queryKey:["settings-export"]});let h=Object.keys(f?.settings||{});h.includes("agent.auto_approve_tools")&&e.invalidateQueries({queryKey:["settings-tools"]}),h.some(x=>ev.has(x))&&i(!0)}}),d=p.default.useCallback(m=>c.mutateAsync(m),[c]);return{settings:a,query:t,save:l,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error||c.error}}function rv(){let e=k(),{tab:t}=it(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=wa(),s=r?"inference":"language",i=t||s,{settings:o,query:l,save:c,savedKeys:d,needsRestart:m,saveError:f}=tk(),[h,x]=p.default.useState("");p.default.useEffect(()=>{x("")},[i]);let y=l.isLoading,$={inference:u`<${jR}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,agent:u`<${DR}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,channels:u`<${LR} searchQuery=${h} />`,networking:u`<${zR}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,tools:u`<${YR}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,skills:u`<${QR} searchQuery=${h} />`,traces:u`<${WR} searchQuery=${h} />`,users:u`<${ek} searchQuery=${h} />`,language:u`<${FR} searchQuery=${h} />`},g=C=>C==="users"||C==="inference",v=C=>Object.prototype.hasOwnProperty.call($,C),b=Object.keys($).filter(C=>r||!g(C)),S=v(s)&&b.includes(s)?s:b[0]||"language";return!v(i)||!r&&g(i)?u`<${ot} to=${`/settings/${S}`} replace />`:u`
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
  `}var sv=Object.freeze({todo:!0});function ak(){return Promise.resolve({users:[],total:0,...sv})}function nk(e){return Promise.resolve(null)}function rk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function sk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function ik(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function ok(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function lk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function uk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function ck(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...sv})}function dk(e="day",t){return Promise.resolve({entries:[],...sv})}function mk(){return K({queryKey:["admin","usage-summary"],queryFn:ck,refetchInterval:3e4})}function Cd(e="day",t){return K({queryKey:["admin","usage",e,t],queryFn:()=>dk(e,t),refetchInterval:3e4})}function Ei(){let e=Z(),t=K({queryKey:["admin","users"],queryFn:ak,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=Y({mutationFn:rk,onSuccess:s}),o=Y({mutationFn:({id:f,payload:h})=>sk(f,h),onSuccess:s}),l=Y({mutationFn:f=>ik(f),onSuccess:s}),c=Y({mutationFn:f=>ok(f),onSuccess:s}),d=Y({mutationFn:f=>lk(f),onSuccess:s}),m=Y({mutationFn:({userId:f,name:h})=>uk(f,h)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(f,h)=>o.mutateAsync({id:f,payload:h}),deleteUser:l.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(f,h)=>m.mutateAsync({userId:f,name:h}),newToken:m.data,clearToken:()=>m.reset()}}function fk(e){return K({queryKey:["admin","user",e],queryFn:()=>nk(e),enabled:!!e,refetchInterval:1e4})}function nn(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function La(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function pk(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function hr(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function Ti(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function Ai(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function Di(e){return e==="admin"?"signal":"muted"}function hk(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function vk(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function gk(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function yk(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function bk(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function TO({users:e,onSelectUser:t}){let a=k(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?u`
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
                <td className="py-3 pr-4"><${I} tone=${Di(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${I} tone=${Ai(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${hr(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:u`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function xk({onSelectUser:e,onNavigateTab:t}){let a=k(),n=mk(),{users:r,query:s}=Ei(),i=n.data||{},o=hk(r),l=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?u`
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
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:pk(i.uptime_seconds)})}</span>
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
        <${TO} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var AO=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function DO({value:e,max:t}){let a=t>0?e/t*100:0;return u`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function $k({onSelectUser:e}){let t=k(),[a,n]=p.default.useState("day"),r=Cd(a),s=r.data?.usage||[],i=gk(s),o=yk(s),l=bk(i),c=i.length>0?i[0].cost:0;return r.isLoading?u`
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
            ${AO.map(d=>u`
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
                          ${Ti(d.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${nn(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${nn(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${La(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${DO} value=${d.cost} max=${c} />
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
  `}function vr({label:e,children:t}){return u`
    <div className="flex items-start justify-between gap-4 border-t border-white/[0.06] py-3 first:border-0 first:pt-0">
      <span className="text-xs text-iron-300">${e}</span>
      <span className="text-right text-sm text-iron-100">${t}</span>
    </div>
  `}function wk({userId:e,onBack:t}){let a=k(),n=fk(e),r=Cd("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:l,createToken:c,newToken:d,clearToken:m}=Ei(),[f,h]=p.default.useState(null),[x,y]=p.default.useState(!1),$=n.data,g=r.data?.usage||[];if(p.default.useEffect(()=>{$&&f===null&&h($.role)},[$]),n.isLoading)return u`
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
              <${I} tone=${Di($.role)} label=${$.role||"member"} />
              <${I} tone=${Ai($.status)} label=${$.status||"active"} />
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
          <${vr} label=${a("admin.user.id")}>
            <span className="font-mono text-xs">${$.id}</span>
          <//>
          <${vr} label=${a("admin.user.email")}>${$.email||a("admin.user.notSet")}<//>
          <${vr} label=${a("admin.user.created")}>${hr($.created_at)}<//>
          <${vr} label=${a("admin.user.lastLogin")}>${hr($.last_login_at)}<//>
          ${$.created_by&&u`
            <${vr} label=${a("admin.user.createdBy")}>
              <span className="font-mono text-xs">${Ti($.created_by)}</span>
            <//>
          `}
        <//>

        <${H} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${vr} label=${a("admin.user.jobs")}>${$.job_count??0}<//>
          <${vr} label=${a("admin.user.totalCost")}>${La($.total_cost)}<//>
          <${vr} label=${a("admin.user.lastActive")}>${hr($.last_active_at)}<//>
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
  `}function MO(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function OO({token:e,onDismiss:t}){let a=k(),[n,r]=p.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return u`
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
  `}function LO({onCreate:e,isCreating:t,error:a}){let n=k(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[l,c]=p.default.useState("member"),[d,m]=p.default.useState(!1),f=async h=>{h.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:l}),s(""),o(""),m(!1))};return d?u`
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
    `}function PO({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=k();return u`
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
  `}function UO({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=k();return u`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${I} tone=${Di(e.role)} label=${e.role||"member"} />
          <${I} tone=${Ai(e.status)} label=${e.status||"active"} />
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
        <span className="hidden text-xs text-iron-700 lg:inline">${hr(e.last_active_at)}</span>
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
  `}function Sk({selectedUserId:e,onSelectUser:t}){let a=k(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:l,updateUser:c,deleteUser:d,suspendUser:m,activateUser:f,createToken:h,newToken:x,clearToken:y}=Ei(),[$,g]=p.default.useState(""),[v,b]=p.default.useState("all"),[w,S]=p.default.useState(null),C=vk(n,{search:$,filter:v}),R=MO(a),_=L=>{S({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{m(L),S(null)}})},M=async(L,U)=>{let F=window.prompt(a("admin.users.tokenNamePrompt",{name:U||a("admin.users.userFallback")}));F&&await h(L,F)};return r.isLoading?u`
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
        <${OO}
          token=${x.token||x.plaintext_token}
          onDismiss=${y}
        />
      `}

      <${LO} onCreate=${i} isCreating=${o} error=${l} />

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
                <${UO}
                  key=${L.id}
                  user=${L}
                  onSelect=${t}
                  onSuspend=${_}
                  onActivate=${f}
                  onChangeRole=${(U,F)=>c(U,{role:F})}
                  onCreateToken=${M}
                />
              `)}
      <//>

      ${w&&u`
        <${PO}
          title=${w.title}
          message=${w.message}
          confirmLabel=${w.confirmLabel}
          onConfirm=${w.onConfirm}
          onCancel=${()=>S(null)}
        />
      `}
    </div>
  `}function Nk(){let{tab:e="dashboard"}=it(),t=ve(),[a,n]=p.default.useState(null),r=p.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=p.default.useCallback(()=>{n(null)},[]),i={dashboard:u`<${xk}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?u`<${wk} userId=${a} onBack=${s} />`:u`<${Sk}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:u`<${$k} onSelectUser=${r} />`};return i[e]?u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:u`<${ot} to="/admin/dashboard" replace />`}var jO=2e3,FO=500,zO=2e3,BO=new Set([403,404]),qO=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function IO(e=globalThis.location,t=null){let a=new URLSearchParams(e?.search||""),n={active:[]};for(let[r,s,i]of qO){let o=a.get(s)?.trim();o?(n[r]=o,n.active.push({key:r,param:s,labelKey:i,value:o})):n[r]=null}return!n.threadId&&t&&(n.threadId=t),n}function _k({isAdmin:e=!1,defaultThreadId:t=null}={}){let a=Ae(),n=a?.search||"",r=p.default.useMemo(()=>IO(a,t),[t,n]),{runId:s,source:i,threadId:o,toolCallId:l,toolName:c,turnId:d}=r,[m,f]=p.default.useState([]),[h,x]=p.default.useState("all"),[y,$]=p.default.useState(""),[g,v]=p.default.useState(!1),[b,w]=p.default.useState(!0),[S,C]=p.default.useState(!0),[R,_]=p.default.useState(null),M=p.default.useRef(new Set),L=p.default.useRef(0),U=!e&&!o;p.default.useEffect(()=>{L.current+=1,f([]),_(null)},[e,s,i,o,l,c,d]);let F=p.default.useCallback(async()=>{if(U){C(!1);return}let G=++L.current;C(!0);try{let ae={limit:FO,level:h==="all"?null:h,target:y.trim()||null,threadId:o,runId:s,turnId:d,toolCallId:l,toolName:c,source:i},le;try{le=await(e?c$(ae):Hp(ae))}catch(De){if(!e||!BO.has(De?.status))throw De;le=await Hp(ae)}if(G!==L.current)return;let lt=M.current,Oe=o2(le).entries.filter(De=>!lt.has(De.id));f(Oe),_(null)}catch(ae){if(G!==L.current)return;_(ae)}finally{G===L.current&&C(!1)}},[e,h,U,s,i,y,o,l,c,d]);p.default.useEffect(()=>{F()},[F]),p.default.useEffect(()=>{if(g||U)return;let G=setInterval(F,jO);return()=>clearInterval(G)},[F,U,g]);let z=p.default.useCallback(()=>{v(G=>!G)},[]),P=p.default.useCallback(()=>{let G=[...M.current,...m.map(ae=>ae.id)].slice(-zO);M.current=new Set(G),f([])},[m]);return{entries:m,totalCount:m.length,paused:g,togglePause:z,clearEntries:P,levelFilter:h,setLevelFilter:x,targetFilter:y,setTargetFilter:$,autoScroll:b,setAutoScroll:w,serverLevel:null,changeServerLevel:async()=>{},scope:r,needsThreadScope:U,status:U?"needs_scope":R?"error":S?"loading":"ready",isLoading:S,error:R}}var HO=["all","trace","debug","info","warn","error"],KO=["trace","debug","info","warn","error"],Rk={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},QO={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function VO({entry:e}){let t=k(),[a,n]=p.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=Rk[e.level]||Rk.info,i=QO[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(l=>!!l.value);return u`
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
  `}function kk({value:e,onChange:t,options:a,labelKey:n,t:r}){return u`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>u`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function GO({label:e,value:t,scopeKey:a}){return u`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function Ck(){let e=k(),{isAdmin:t=!1,threadsState:a}=wa()||{},{entries:n,totalCount:r,paused:s,togglePause:i,clearEntries:o,levelFilter:l,setLevelFilter:c,targetFilter:d,setTargetFilter:m,autoScroll:f,setAutoScroll:h,serverLevel:x,changeServerLevel:y,scope:$,isLoading:g,error:v,needsThreadScope:b}=_k({isAdmin:t,defaultThreadId:t?null:a?.activeThreadId||null}),w=p.default.useRef(null),S=p.default.useRef(!0);p.default.useEffect(()=>{f&&S.current&&w.current&&(w.current.scrollTop=0)},[n,f]);let C=p.default.useCallback(M=>{S.current=M.currentTarget.scrollTop<=48},[]),R=n.length>0,_=$?.active||[];return u`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${kk}
          value=${l}
          onChange=${c}
          options=${HO}
          labelKey=${M=>M==="all"?"logs.levelAll":`logs.level.${M}`}
          t=${e}
        />

        <!-- Target filter -->
        <input
          type="text"
          value=${d}
          onInput=${M=>m(M.target.value)}
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
              onChange=${M=>h(M.target.checked)}
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
            ${_.map(M=>u`<${GO} key=${M.param} scopeKey=${M.param} label=${e(M.labelKey)} value=${M.value} />`)}
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
            <${kk}
              value=${x}
              onChange=${y}
              options=${KO}
              labelKey=${M=>`logs.level.${M}`}
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
              `:R?n.map(M=>u`<${VO} key=${M.id} entry=${M} />`):u`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function Tk(){return u`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function YO({auth:e}){let t=ve(),n=Ae().state?.from,r=n?`${n.pathname||Kr}${n.search||""}${n.hash||""}`:Kr,s=`/v2${r==="/"?"":r}`,i=p.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?u`<${Tk} />`:e.isAuthenticated?u`<${ot} to=${r} replace />`:u`<${z1}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function JO({auth:e,children:t}){let a=Ae();return e.isChecking?u`<${Tk} />`:e.isAuthenticated?t:u`<${ot} to="/login" replace state=${{from:a}} />`}function XO({auth:e}){return u`
    <${JO} auth=${e}>
      <${h1}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        rebornProjectsEnabled=${e.rebornProjectsEnabled}
        onSignOut=${e.signOut}
      />
    <//>
  `}function Ek({auth:e}){return e.isAdmin?u`<${Nk} />`:u`<${ot} to=${Kr} replace />`}function Ak(){let e=G$();return u`
    <${zp} basename="/v2">
      <${Lp}>
        <${xe} path="/login" element=${u`<${YO} auth=${e} />`} />
        <${xe} path="/" element=${u`<${XO} auth=${e} />`}>
          <${xe} index element=${u`<${ot} to=${Kr} replace />`} />
          <${xe} path="overview" element=${u`<${ot} to=${Kr} replace />`} />
          <${xe} path="welcome" element=${u`<${p2} />`} />
          <${xe} path="chat" element=${u`<${Dh} />`} />
          <${xe} path="chat/:threadId" element=${u`<${Dh} />`} />
          <${xe} path="workspace" element=${u`<${Oh} />`} />
          <${xe} path="workspace/*" element=${u`<${Oh} />`} />
          <${xe} path="projects" element=${u`<${hl} />`} />
          <${xe} path="projects/:projectId" element=${u`<${hl} />`} />
          <${xe} path="projects/:projectId/missions/:missionId" element=${u`<${hl} />`} />
          <${xe} path="projects/:projectId/threads/:threadId" element=${u`<${hl} />`} />
          <${xe} path="missions" element=${u`<${Ph} />`} />
          <${xe} path="missions/:missionId" element=${u`<${Ph} />`} />
          <${xe} path="jobs" element=${u`<${Fh} />`} />
          <${xe} path="jobs/:jobId" element=${u`<${Fh} />`} />
          <${xe} path="routines" element=${u`<${Bh} />`} />
          <${xe} path="routines/:routineId" element=${u`<${Bh} />`} />
          <${xe} path="automations" element=${u`<${x_} />`} />
          <${xe} path="extensions" element=${u`<${Zh} />`} />
          <${xe} path="extensions/:tab" element=${u`<${Zh} />`} />
          <${xe} path="logs" element=${u`<${Ck} />`} />
          <${xe} path="settings" element=${u`<${rv} />`} />
          <${xe} path="settings/:tab" element=${u`<${rv} />`} />
          <${xe} path="admin" element=${u`<${Ek} auth=${e} />`} />
          <${xe} path="admin/:tab" element=${u`<${Ek} auth=${e} />`} />
        <//>
        <${xe} path="*" element=${u`<${ot} to=${Kr} replace />`} />
      <//>
    <//>
  `}ov("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","approval.viewFullCommand":"View full command","approval.showCommandPreview":"Show preview","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.tabDeclined":"Declined","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitDeclined":"declined","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.serviceUnavailable":"Service unavailable","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveEligibleTools":"Always allow eligible tools","settings.field.autoApproveEligibleToolsDesc":"Applies to tools set to \u201CFollow global\u201D. Per-tool settings override this; hard-floor approval gates still ask.","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.followDefault":"Follow global","tools.sourceDefault":"default permission","tools.sourceGlobal":"global setting","tools.sourceOverride":"per-tool override","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","tools.description.builtin.echo":"Echo a message","tools.description.builtin.time":"Get, parse, format, convert, or diff timestamps","tools.description.builtin.json":"Parse, query, stringify, and validate JSON","tools.description.builtin.http":"Perform an outbound HTTP request through host egress. Redirect responses are returned; the host transport does not follow them. Prefer GitHub extension capabilities for GitHub repository, issue, pull request, release, or workflow data when available.","tools.description.builtin.http.save":"Perform an outbound HTTP request through host egress and save the sanitized response body through scoped filesystem authority. Prefer GitHub extension capabilities for GitHub repository, issue, pull request, release, or workflow data when available.","tools.description.builtin.shell":"Execute shell commands with validation and saved-file references for large local output","tools.description.builtin.spawn_subagent":"Authorize a scoped child subagent run","tools.description.builtin.trace_commons.onboard":"Enroll this IronClaw in Trace Commons using an operator-issued invite link after explicit user consent.","tools.description.builtin.trace_commons.status":"Report Trace Commons enrollment state for the current user.","tools.description.builtin.trace_commons.credits":"Report the current user's Trace Commons credit state, balances, submission counts, and recent explanations.","tools.description.builtin.trace_commons.profile_token":"Mint a short-lived Trace Commons profile-management value for browser or manual profile setup.","tools.description.builtin.trace_commons.profile_set":"Create or update the current user's public Trace Commons community profile after explicit consent.","tools.description.builtin.profile_set":"Record a private local fact about the user's agent context: timezone, locale, or location.","tools.description.builtin.memory_search":"Search Reborn persistent memory documents in the current scope","tools.description.builtin.memory_write":"Write, append, or patch Reborn persistent memory documents in the current scope","tools.description.builtin.memory_read":"Read a Reborn persistent memory document in the current scope","tools.description.builtin.memory_tree":"List Reborn persistent memory documents as a compact tree","tools.description.builtin.read_file":"Read text files and extract text from supported document files through scoped mounts","tools.description.builtin.write_file":"Write content through scoped mounts","tools.description.builtin.list_dir":"List directory contents through scoped mounts","tools.description.builtin.glob":"Find files under a scoped directory with a glob pattern","tools.description.builtin.grep":"Search scoped file contents with grep output modes","tools.description.builtin.apply_patch":"Apply exact or fuzzy search-replace edits through scoped mounts","tools.description.builtin.skill_list":"List Reborn filesystem skills visible to the current local-dev agent","tools.description.builtin.skill_install":"Install a SKILL.md document, URL, ZIP bundle, or GitHub skill repository into the current user's skill root","tools.description.builtin.skill_remove":"Remove a user-installed Reborn filesystem skill","tools.description.builtin.trigger_create":"Create a caller-scoped scheduled trigger, either one-time or recurring","tools.description.builtin.trigger_list":"List scheduled triggers owned by the current caller scope","tools.description.builtin.trigger_remove":"Remove a caller-scoped scheduled trigger","tools.description.builtin.trigger_pause":"Pause a caller-scoped scheduled trigger so it remains retained but does not fire","tools.description.builtin.trigger_resume":"Resume a caller-scoped paused trigger so it may fire on its stored schedule","tools.description.builtin.extension_search":"Search the local Reborn extension catalog by extension, product, provider, or service name","tools.description.builtin.extension_install":"Install a searched Reborn extension into durable local-dev lifecycle state","tools.description.builtin.extension_activate":"Activate an installed Reborn extension for the model-visible local-dev capability surface","tools.description.builtin.extension_remove":"Remove an installed Reborn extension from durable local-dev lifecycle state","tools.description.nearai.web_search":"Search through the NEAR AI MCP server","tools.description.builtin.outbound_delivery_target_set":"Set the current user's final-reply outbound delivery target, such as a Slack DM or Slack channel","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","skills.defaultAutoActivationEnabled":"Default skill auto-activation enabled","skills.defaultAutoActivationDisabled":"Default skill auto-activation disabled","skills.defaultAutoActivationOnDesc":"Skills auto-activate by keyword on matching requests. Turn off to require an explicit /name.","skills.defaultAutoActivationOffDesc":"Skills run only when you type /name. Turn on to let them auto-activate by keyword.","skills.defaultAutoActivationOnButton":"Default: On","skills.defaultAutoActivationOffButton":"Default: Off","skills.autoActivateOnTitle":"Auto-activation on \u2014 runs on matching requests. Click to make it explicit-only (/name).","skills.autoActivateOffTitle":"Explicit-only \u2014 runs only when you type /name. Click to enable auto-activation.","skills.autoActivateOnLabel":"Auto-activate: On","skills.autoActivateOffLabel":"Auto-activate: Off","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Memory, files & attachments","workspace.readOnly":"Read-only","workspace.filterPlaceholder":"Filter by name\u2026","workspace.emptyDir":"This folder is empty.","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.noFiles":"No files here.","workspace.noMatches":"Nothing matches that filter.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a file","workspace.pickFileDesc":"Choose a file from the tree to preview or download it. This viewer is read-only.","workspace.parent":"Parent: {path}","workspace.download":"Download","workspace.binaryPreviewUnavailable":"No inline preview for this file type. Download it to view the contents.","workspace.fileMeta":"{mime} \xB7 {size} bytes","workspace.unableOpenDirectory":"Unable to open directory","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.filter.completed":"Completed","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.schedule.onceAt":"Once on {datetime}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.filterAction":"Show {label}","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.empty.onboardingTitle":"No automations yet","automations.empty.onboardingDescription":"Automations are created by chatting with your agent \u2014 there's no form to fill out. Ask it to do something on a schedule and it will set up a recurring automation for you.","automations.empty.examplesTitle":"Try asking your agent","automations.empty.example1":"Check the nearai/ironclaw repo every 10 minutes and summarize new issues, PRs, and commits.","automations.empty.example2":"Every weekday at 9am, send me a summary of my unread email.","automations.empty.example3":"Remind me to review open pull requests every afternoon at 3pm.","automations.empty.startInChat":"Start in chat","automations.empty.copyPrompt":"Copy prompt","automations.empty.copied":"Copied","automations.refreshing":"Refreshing\u2026","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.runs.total":"Recent runs: {count}","automations.runs.ok":"OK: {count}","automations.runs.error":"Failed: {count}","automations.runs.running":"Running: {count}","automations.runs.unknown":"Unknown: {count}","automations.runs.showingOf":"Showing {shown} of {total} recent runs","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up a project for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open project","projects.openGeneralWorkspace":"Open project","projects.noDescription":"No project description yet. The project is still being shaped by recent activity and thread history.","projects.general.label":"General project","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.activate":"Activate","extensions.setup":"Setup","extensions.install":"Install","extensions.noCapabilities":"No capabilities","extensions.defaultName":"Extension","extensions.installedSuccess":"{name} installed","extensions.activatedSuccess":"{name} activated","extensions.removedSuccess":"{name} removed","extensions.installFailed":"Install failed","extensions.activationFailed":"Activation failed","extensions.removeFailed":"Remove failed","extensions.openingAuth":"Opening authentication...","extensions.configurationRequired":"Configuration required","extensions.getCredentials":"Get credentials","extensions.keepSecretPlaceholder":"\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (leave blank to keep)","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,Dk.createRoot)(document.getElementById("v2-root")).render(u`
  <${lv}>
    <${Bd} client=${Dt}>
      <${Ak} />
    <//>
  <//>
`);
