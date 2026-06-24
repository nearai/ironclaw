import{a as Nn,b as Ie,c as Ve,d as h,e as l,f as Mh,g as Oh,h as nl,i as k,j as rl}from"./chunks/chunk-UQPKMZ52.js";var Wh=Nn(fl=>{"use strict";var Wk=Symbol.for("react.transitional.element"),eR=Symbol.for("react.fragment");function Zh(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:Wk,type:e,key:n,ref:t!==void 0?t:null,props:a}}fl.Fragment=eR;fl.jsx=Zh;fl.jsxs=Zh});var hd=Nn((aL,ev)=>{"use strict";ev.exports=Wh()});var pv=Nn(Le=>{"use strict";function wd(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<wl(r,t))e[n]=t,e[a]=r,a=n;else break e}}function La(e){return e.length===0?null:e[0]}function Nl(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],u=i+1,c=e[u];if(0>wl(o,a))u<r&&0>wl(c,o)?(e[n]=c,e[u]=a,n=u):(e[n]=o,e[i]=a,n=i);else if(u<r&&0>wl(c,a))e[n]=c,e[u]=a,n=u;else break e}}return t}function wl(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}Le.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(sv=performance,Le.unstable_now=function(){return sv.now()}):(bd=Date,iv=bd.now(),Le.unstable_now=function(){return bd.now()-iv});var sv,bd,iv,Za=[],Rn=[],rR=1,ua=null,St=3,Sd=!1,Ri=!1,Ci=!1,Nd=!1,uv=typeof setTimeout=="function"?setTimeout:null,cv=typeof clearTimeout=="function"?clearTimeout:null,ov=typeof setImmediate<"u"?setImmediate:null;function Sl(e){for(var t=La(Rn);t!==null;){if(t.callback===null)Nl(Rn);else if(t.startTime<=e)Nl(Rn),t.sortIndex=t.expirationTime,wd(Za,t);else break;t=La(Rn)}}function _d(e){if(Ci=!1,Sl(e),!Ri)if(La(Za)!==null)Ri=!0,Xr||(Xr=!0,Jr());else{var t=La(Rn);t!==null&&kd(_d,t.startTime-e)}}var Xr=!1,Ei=-1,dv=5,mv=-1;function fv(){return Nd?!0:!(Le.unstable_now()-mv<dv)}function xd(){if(Nd=!1,Xr){var e=Le.unstable_now();mv=e;var t=!0;try{e:{Ri=!1,Ci&&(Ci=!1,cv(Ei),Ei=-1),Sd=!0;var a=St;try{t:{for(Sl(e),ua=La(Za);ua!==null&&!(ua.expirationTime>e&&fv());){var n=ua.callback;if(typeof n=="function"){ua.callback=null,St=ua.priorityLevel;var r=n(ua.expirationTime<=e);if(e=Le.unstable_now(),typeof r=="function"){ua.callback=r,Sl(e),t=!0;break t}ua===La(Za)&&Nl(Za),Sl(e)}else Nl(Za);ua=La(Za)}if(ua!==null)t=!0;else{var s=La(Rn);s!==null&&kd(_d,s.startTime-e),t=!1}}break e}finally{ua=null,St=a,Sd=!1}t=void 0}}finally{t?Jr():Xr=!1}}}var Jr;typeof ov=="function"?Jr=function(){ov(xd)}:typeof MessageChannel<"u"?($d=new MessageChannel,lv=$d.port2,$d.port1.onmessage=xd,Jr=function(){lv.postMessage(null)}):Jr=function(){uv(xd,0)};var $d,lv;function kd(e,t){Ei=uv(function(){e(Le.unstable_now())},t)}Le.unstable_IdlePriority=5;Le.unstable_ImmediatePriority=1;Le.unstable_LowPriority=4;Le.unstable_NormalPriority=3;Le.unstable_Profiling=null;Le.unstable_UserBlockingPriority=2;Le.unstable_cancelCallback=function(e){e.callback=null};Le.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):dv=0<e?Math.floor(1e3/e):5};Le.unstable_getCurrentPriorityLevel=function(){return St};Le.unstable_next=function(e){switch(St){case 1:case 2:case 3:var t=3;break;default:t=St}var a=St;St=t;try{return e()}finally{St=a}};Le.unstable_requestPaint=function(){Nd=!0};Le.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=St;St=e;try{return t()}finally{St=a}};Le.unstable_scheduleCallback=function(e,t,a){var n=Le.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:rR++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,wd(Rn,e),La(Za)===null&&e===La(Rn)&&(Ci?(cv(Ei),Ei=-1):Ci=!0,kd(_d,a-n))):(e.sortIndex=r,wd(Za,e),Ri||Sd||(Ri=!0,Xr||(Xr=!0,Jr()))),e};Le.unstable_shouldYield=fv;Le.unstable_wrapCallback=function(e){var t=St;return function(){var a=St;St=t;try{return e.apply(this,arguments)}finally{St=a}}}});var vv=Nn((jL,hv)=>{"use strict";hv.exports=pv()});var yv=Nn(Et=>{"use strict";var sR=Ve();function gv(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Cn(){}var Ct={d:{f:Cn,r:function(){throw Error(gv(522))},D:Cn,C:Cn,L:Cn,m:Cn,X:Cn,S:Cn,M:Cn},p:0,findDOMNode:null},iR=Symbol.for("react.portal");function oR(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:iR,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var Ai=sR.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function _l(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}Et.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=Ct;Et.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(gv(299));return oR(e,t,null,a)};Et.flushSync=function(e){var t=Ai.T,a=Ct.p;try{if(Ai.T=null,Ct.p=2,e)return e()}finally{Ai.T=t,Ct.p=a,Ct.d.f()}};Et.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,Ct.d.C(e,t))};Et.prefetchDNS=function(e){typeof e=="string"&&Ct.d.D(e)};Et.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=_l(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?Ct.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&Ct.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};Et.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=_l(t.as,t.crossOrigin);Ct.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&Ct.d.M(e)};Et.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=_l(a,t.crossOrigin);Ct.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};Et.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=_l(t.as,t.crossOrigin);Ct.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else Ct.d.m(e)};Et.requestFormReset=function(e){Ct.d.r(e)};Et.unstable_batchedUpdates=function(e,t){return e(t)};Et.useFormState=function(e,t,a){return Ai.H.useFormState(e,t,a)};Et.useFormStatus=function(){return Ai.H.useHostTransitionStatus()};Et.version="19.1.0"});var $v=Nn((qL,xv)=>{"use strict";function bv(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(bv)}catch(e){console.error(e)}}bv(),xv.exports=yv()});var S0=Nn(Vu=>{"use strict";var lt=vv(),Ig=Ve(),lR=$v();function L(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Hg(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function yo(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function Kg(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function wv(e){if(yo(e)!==e)throw Error(L(188))}function uR(e){var t=e.alternate;if(!t){if(t=yo(e),t===null)throw Error(L(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return wv(r),e;if(s===n)return wv(r),t;s=s.sibling}throw Error(L(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(L(189))}}if(a.alternate!==n)throw Error(L(190))}if(a.tag!==3)throw Error(L(188));return a.stateNode.current===a?e:t}function Qg(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=Qg(e),t!==null)return t;e=e.sibling}return null}var Me=Object.assign,cR=Symbol.for("react.element"),kl=Symbol.for("react.transitional.element"),Fi=Symbol.for("react.portal"),rs=Symbol.for("react.fragment"),Vg=Symbol.for("react.strict_mode"),rm=Symbol.for("react.profiler"),dR=Symbol.for("react.provider"),Gg=Symbol.for("react.consumer"),nn=Symbol.for("react.context"),Wm=Symbol.for("react.forward_ref"),sm=Symbol.for("react.suspense"),im=Symbol.for("react.suspense_list"),ef=Symbol.for("react.memo"),Tn=Symbol.for("react.lazy");Symbol.for("react.scope");var om=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var mR=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var Sv=Symbol.iterator;function Ti(e){return e===null||typeof e!="object"?null:(e=Sv&&e[Sv]||e["@@iterator"],typeof e=="function"?e:null)}var fR=Symbol.for("react.client.reference");function lm(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===fR?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case rs:return"Fragment";case rm:return"Profiler";case Vg:return"StrictMode";case sm:return"Suspense";case im:return"SuspenseList";case om:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case Fi:return"Portal";case nn:return(e.displayName||"Context")+".Provider";case Gg:return(e._context.displayName||"Context")+".Consumer";case Wm:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case ef:return t=e.displayName||null,t!==null?t:lm(e.type)||"Memo";case Tn:t=e._payload,e=e._init;try{return lm(e(t))}catch{}}return null}var qi=Array.isArray,re=Ig.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,he=lR.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,pr={pending:!1,data:null,method:null,action:null},um=[],ss=-1;function Ba(e){return{current:e}}function pt(e){0>ss||(e.current=um[ss],um[ss]=null,ss--)}function Ue(e,t){ss++,um[ss]=e.current,e.current=t}var Fa=Ba(null),no=Ba(null),zn=Ba(null),au=Ba(null);function nu(e,t){switch(Ue(zn,t),Ue(no,e),Ue(Fa,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?Eg(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=Eg(t),e=d0(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}pt(Fa),Ue(Fa,e)}function Ns(){pt(Fa),pt(no),pt(zn)}function cm(e){e.memoizedState!==null&&Ue(au,e);var t=Fa.current,a=d0(t,e.type);t!==a&&(Ue(no,e),Ue(Fa,a))}function ru(e){no.current===e&&(pt(Fa),pt(no)),au.current===e&&(pt(au),po._currentValue=pr)}var dm=Object.prototype.hasOwnProperty,tf=lt.unstable_scheduleCallback,Rd=lt.unstable_cancelCallback,pR=lt.unstable_shouldYield,hR=lt.unstable_requestPaint,qa=lt.unstable_now,vR=lt.unstable_getCurrentPriorityLevel,Yg=lt.unstable_ImmediatePriority,Jg=lt.unstable_UserBlockingPriority,su=lt.unstable_NormalPriority,gR=lt.unstable_LowPriority,Xg=lt.unstable_IdlePriority,yR=lt.log,bR=lt.unstable_setDisableYieldValue,bo=null,Jt=null;function Un(e){if(typeof yR=="function"&&bR(e),Jt&&typeof Jt.setStrictMode=="function")try{Jt.setStrictMode(bo,e)}catch{}}var Xt=Math.clz32?Math.clz32:wR,xR=Math.log,$R=Math.LN2;function wR(e){return e>>>=0,e===0?32:31-(xR(e)/$R|0)|0}var Rl=256,Cl=4194304;function dr(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Du(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=dr(n):(i&=o,i!==0?r=dr(i):a||(a=o&~e,a!==0&&(r=dr(a))))):(o=n&~s,o!==0?r=dr(o):i!==0?r=dr(i):a||(a=n&~e,a!==0&&(r=dr(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function xo(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function SR(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function Zg(){var e=Rl;return Rl<<=1,(Rl&4194048)===0&&(Rl=256),e}function Wg(){var e=Cl;return Cl<<=1,(Cl&62914560)===0&&(Cl=4194304),e}function Cd(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function $o(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function NR(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,u=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Xt(a),f=1<<d;o[d]=0,u[d]=-1;var m=c[d];if(m!==null)for(c[d]=null,d=0;d<m.length;d++){var p=m[d];p!==null&&(p.lane&=-536870913)}a&=~f}n!==0&&ey(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function ey(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Xt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function ty(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Xt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function af(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function nf(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function ay(){var e=he.p;return e!==0?e:(e=window.event,e===void 0?32:$0(e.type))}function _R(e,t){var a=he.p;try{return he.p=e,t()}finally{he.p=a}}var Zn=Math.random().toString(36).slice(2),Nt="__reactFiber$"+Zn,zt="__reactProps$"+Zn,Ls="__reactContainer$"+Zn,mm="__reactEvents$"+Zn,kR="__reactListeners$"+Zn,RR="__reactHandles$"+Zn,Nv="__reactResources$"+Zn,wo="__reactMarker$"+Zn;function rf(e){delete e[Nt],delete e[zt],delete e[mm],delete e[kR],delete e[RR]}function is(e){var t=e[Nt];if(t)return t;for(var a=e.parentNode;a;){if(t=a[Ls]||a[Nt]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=Dg(e);e!==null;){if(a=e[Nt])return a;e=Dg(e)}return t}e=a,a=e.parentNode}return null}function Ps(e){if(e=e[Nt]||e[Ls]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function zi(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(L(33))}function vs(e){var t=e[Nv];return t||(t=e[Nv]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function mt(e){e[wo]=!0}var ny=new Set,ry={};function _r(e,t){_s(e,t),_s(e+"Capture",t)}function _s(e,t){for(ry[e]=t,e=0;e<t.length;e++)ny.add(t[e])}var CR=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),_v={},kv={};function ER(e){return dm.call(kv,e)?!0:dm.call(_v,e)?!1:CR.test(e)?kv[e]=!0:(_v[e]=!0,!1)}function Il(e,t,a){if(ER(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function El(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function Wa(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var Ed,Rv;function ts(e){if(Ed===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);Ed=t&&t[1]||"",Rv=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+Ed+e+Rv}var Ad=!1;function Td(e,t){if(!e||Ad)return"";Ad=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var f=function(){throw Error()};if(Object.defineProperty(f.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(f,[])}catch(p){var m=p}Reflect.construct(e,[],f)}else{try{f.call()}catch(p){m=p}e.call(f.prototype)}}else{try{throw Error()}catch(p){m=p}(f=e())&&typeof f.catch=="function"&&f.catch(function(){})}}catch(p){if(p&&m&&typeof p.stack=="string")return[p.stack,m.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var u=i.split(`
`),c=o.split(`
`);for(r=n=0;n<u.length&&!u[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===u.length||r===c.length)for(n=u.length-1,r=c.length-1;1<=n&&0<=r&&u[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(u[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||u[n]!==c[r]){var d=`
`+u[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{Ad=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?ts(a):""}function AR(e){switch(e.tag){case 26:case 27:case 5:return ts(e.type);case 16:return ts("Lazy");case 13:return ts("Suspense");case 19:return ts("SuspenseList");case 0:case 15:return Td(e.type,!1);case 11:return Td(e.type.render,!1);case 1:return Td(e.type,!0);case 31:return ts("Activity");default:return""}}function Cv(e){try{var t="";do t+=AR(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function da(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function sy(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function TR(e){var t=sy(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function iu(e){e._valueTracker||(e._valueTracker=TR(e))}function iy(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=sy(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function ou(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var DR=/[\n"\\]/g;function pa(e){return e.replace(DR,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function fm(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+da(t)):e.value!==""+da(t)&&(e.value=""+da(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?pm(e,i,da(t)):a!=null?pm(e,i,da(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+da(o):e.removeAttribute("name")}function oy(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+da(a):"",t=t!=null?""+da(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function pm(e,t,a){t==="number"&&ou(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function gs(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+da(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function ly(e,t,a){if(t!=null&&(t=""+da(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+da(a):""}function uy(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(L(92));if(qi(n)){if(1<n.length)throw Error(L(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=da(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function ks(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var MR=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function Ev(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||MR.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function cy(e,t,a){if(t!=null&&typeof t!="object")throw Error(L(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&Ev(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&Ev(e,s,t[s])}function sf(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var OR=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),LR=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function Hl(e){return LR.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var hm=null;function of(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var os=null,ys=null;function Av(e){var t=Ps(e);if(t&&(e=t.stateNode)){var a=e[zt]||null;e:switch(e=t.stateNode,t.type){case"input":if(fm(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+pa(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[zt]||null;if(!r)throw Error(L(90));fm(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&iy(n)}break e;case"textarea":ly(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&gs(e,!!a.multiple,t,!1)}}}var Dd=!1;function dy(e,t,a){if(Dd)return e(t,a);Dd=!0;try{var n=e(t);return n}finally{if(Dd=!1,(os!==null||ys!==null)&&(Bu(),os&&(t=os,e=ys,ys=os=null,Av(t),e)))for(t=0;t<e.length;t++)Av(e[t])}}function ro(e,t){var a=e.stateNode;if(a===null)return null;var n=a[zt]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(L(231,t,typeof a));return a}var dn=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),vm=!1;if(dn)try{Zr={},Object.defineProperty(Zr,"passive",{get:function(){vm=!0}}),window.addEventListener("test",Zr,Zr),window.removeEventListener("test",Zr,Zr)}catch{vm=!1}var Zr,jn=null,lf=null,Kl=null;function my(){if(Kl)return Kl;var e,t=lf,a=t.length,n,r="value"in jn?jn.value:jn.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return Kl=r.slice(e,1<n?1-n:void 0)}function Ql(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function Al(){return!0}function Tv(){return!1}function Bt(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?Al:Tv,this.isPropagationStopped=Tv,this}return Me(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=Al)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=Al)},persist:function(){},isPersistent:Al}),t}var kr={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Mu=Bt(kr),So=Me({},kr,{view:0,detail:0}),PR=Bt(So),Md,Od,Di,Ou=Me({},So,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:uf,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==Di&&(Di&&e.type==="mousemove"?(Md=e.screenX-Di.screenX,Od=e.screenY-Di.screenY):Od=Md=0,Di=e),Md)},movementY:function(e){return"movementY"in e?e.movementY:Od}}),Dv=Bt(Ou),UR=Me({},Ou,{dataTransfer:0}),jR=Bt(UR),FR=Me({},So,{relatedTarget:0}),Ld=Bt(FR),qR=Me({},kr,{animationName:0,elapsedTime:0,pseudoElement:0}),zR=Bt(qR),BR=Me({},kr,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),IR=Bt(BR),HR=Me({},kr,{data:0}),Mv=Bt(HR),KR={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},QR={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},VR={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function GR(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=VR[e])?!!t[e]:!1}function uf(){return GR}var YR=Me({},So,{key:function(e){if(e.key){var t=KR[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=Ql(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?QR[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:uf,charCode:function(e){return e.type==="keypress"?Ql(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?Ql(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),JR=Bt(YR),XR=Me({},Ou,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),Ov=Bt(XR),ZR=Me({},So,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:uf}),WR=Bt(ZR),eC=Me({},kr,{propertyName:0,elapsedTime:0,pseudoElement:0}),tC=Bt(eC),aC=Me({},Ou,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),nC=Bt(aC),rC=Me({},kr,{newState:0,oldState:0}),sC=Bt(rC),iC=[9,13,27,32],cf=dn&&"CompositionEvent"in window,Ii=null;dn&&"documentMode"in document&&(Ii=document.documentMode);var oC=dn&&"TextEvent"in window&&!Ii,fy=dn&&(!cf||Ii&&8<Ii&&11>=Ii),Lv=" ",Pv=!1;function py(e,t){switch(e){case"keyup":return iC.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function hy(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var ls=!1;function lC(e,t){switch(e){case"compositionend":return hy(t);case"keypress":return t.which!==32?null:(Pv=!0,Lv);case"textInput":return e=t.data,e===Lv&&Pv?null:e;default:return null}}function uC(e,t){if(ls)return e==="compositionend"||!cf&&py(e,t)?(e=my(),Kl=lf=jn=null,ls=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return fy&&t.locale!=="ko"?null:t.data;default:return null}}var cC={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function Uv(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!cC[e.type]:t==="textarea"}function vy(e,t,a,n){os?ys?ys.push(n):ys=[n]:os=n,t=_u(t,"onChange"),0<t.length&&(a=new Mu("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var Hi=null,so=null;function dC(e){l0(e,0)}function Lu(e){var t=zi(e);if(iy(t))return e}function jv(e,t){if(e==="change")return t}var gy=!1;dn&&(dn?(Dl="oninput"in document,Dl||(Pd=document.createElement("div"),Pd.setAttribute("oninput","return;"),Dl=typeof Pd.oninput=="function"),Tl=Dl):Tl=!1,gy=Tl&&(!document.documentMode||9<document.documentMode));var Tl,Dl,Pd;function Fv(){Hi&&(Hi.detachEvent("onpropertychange",yy),so=Hi=null)}function yy(e){if(e.propertyName==="value"&&Lu(so)){var t=[];vy(t,so,e,of(e)),dy(dC,t)}}function mC(e,t,a){e==="focusin"?(Fv(),Hi=t,so=a,Hi.attachEvent("onpropertychange",yy)):e==="focusout"&&Fv()}function fC(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return Lu(so)}function pC(e,t){if(e==="click")return Lu(t)}function hC(e,t){if(e==="input"||e==="change")return Lu(t)}function vC(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var ea=typeof Object.is=="function"?Object.is:vC;function io(e,t){if(ea(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!dm.call(t,r)||!ea(e[r],t[r]))return!1}return!0}function qv(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function zv(e,t){var a=qv(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=qv(a)}}function by(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?by(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function xy(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=ou(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=ou(e.document)}return t}function df(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var gC=dn&&"documentMode"in document&&11>=document.documentMode,us=null,gm=null,Ki=null,ym=!1;function Bv(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;ym||us==null||us!==ou(n)||(n=us,"selectionStart"in n&&df(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),Ki&&io(Ki,n)||(Ki=n,n=_u(gm,"onSelect"),0<n.length&&(t=new Mu("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=us)))}function cr(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var cs={animationend:cr("Animation","AnimationEnd"),animationiteration:cr("Animation","AnimationIteration"),animationstart:cr("Animation","AnimationStart"),transitionrun:cr("Transition","TransitionRun"),transitionstart:cr("Transition","TransitionStart"),transitioncancel:cr("Transition","TransitionCancel"),transitionend:cr("Transition","TransitionEnd")},Ud={},$y={};dn&&($y=document.createElement("div").style,"AnimationEvent"in window||(delete cs.animationend.animation,delete cs.animationiteration.animation,delete cs.animationstart.animation),"TransitionEvent"in window||delete cs.transitionend.transition);function Rr(e){if(Ud[e])return Ud[e];if(!cs[e])return e;var t=cs[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in $y)return Ud[e]=t[a];return e}var wy=Rr("animationend"),Sy=Rr("animationiteration"),Ny=Rr("animationstart"),yC=Rr("transitionrun"),bC=Rr("transitionstart"),xC=Rr("transitioncancel"),_y=Rr("transitionend"),ky=new Map,bm="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");bm.push("scrollEnd");function ka(e,t){ky.set(e,t),_r(t,[e])}var Iv=new WeakMap;function ha(e,t){if(typeof e=="object"&&e!==null){var a=Iv.get(e);return a!==void 0?a:(t={value:e,source:t,stack:Cv(t)},Iv.set(e,t),t)}return{value:e,source:t,stack:Cv(t)}}var ca=[],ds=0,mf=0;function Pu(){for(var e=ds,t=mf=ds=0;t<e;){var a=ca[t];ca[t++]=null;var n=ca[t];ca[t++]=null;var r=ca[t];ca[t++]=null;var s=ca[t];if(ca[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&Ry(a,r,s)}}function Uu(e,t,a,n){ca[ds++]=e,ca[ds++]=t,ca[ds++]=a,ca[ds++]=n,mf|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function ff(e,t,a,n){return Uu(e,t,a,n),lu(e)}function Us(e,t){return Uu(e,null,null,t),lu(e)}function Ry(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Xt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function lu(e){if(50<to)throw to=0,qm=null,Error(L(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var ms={};function $C(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Yt(e,t,a,n){return new $C(e,t,a,n)}function pf(e){return e=e.prototype,!(!e||!e.isReactComponent)}function un(e,t){var a=e.alternate;return a===null?(a=Yt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function Cy(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function Vl(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")pf(e)&&(i=1);else if(typeof e=="string")i=$3(e,a,Fa.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case om:return e=Yt(31,a,t,r),e.elementType=om,e.lanes=s,e;case rs:return hr(a.children,r,s,t);case Vg:i=8,r|=24;break;case rm:return e=Yt(12,a,t,r|2),e.elementType=rm,e.lanes=s,e;case sm:return e=Yt(13,a,t,r),e.elementType=sm,e.lanes=s,e;case im:return e=Yt(19,a,t,r),e.elementType=im,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case dR:case nn:i=10;break e;case Gg:i=9;break e;case Wm:i=11;break e;case ef:i=14;break e;case Tn:i=16,n=null;break e}i=29,a=Error(L(130,e===null?"null":typeof e,"")),n=null}return t=Yt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function hr(e,t,a,n){return e=Yt(7,e,n,t),e.lanes=a,e}function jd(e,t,a){return e=Yt(6,e,null,t),e.lanes=a,e}function Fd(e,t,a){return t=Yt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var fs=[],ps=0,uu=null,cu=0,ma=[],fa=0,vr=null,rn=1,sn="";function mr(e,t){fs[ps++]=cu,fs[ps++]=uu,uu=e,cu=t}function Ey(e,t,a){ma[fa++]=rn,ma[fa++]=sn,ma[fa++]=vr,vr=e;var n=rn;e=sn;var r=32-Xt(n)-1;n&=~(1<<r),a+=1;var s=32-Xt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,rn=1<<32-Xt(t)+r|a<<r|n,sn=s+e}else rn=1<<s|a<<r|n,sn=e}function hf(e){e.return!==null&&(mr(e,1),Ey(e,1,0))}function vf(e){for(;e===uu;)uu=fs[--ps],fs[ps]=null,cu=fs[--ps],fs[ps]=null;for(;e===vr;)vr=ma[--fa],ma[fa]=null,sn=ma[--fa],ma[fa]=null,rn=ma[--fa],ma[fa]=null}var At=null,He=null,pe=!1,gr=null,Ua=!1,xm=Error(L(519));function $r(e){var t=Error(L(418,""));throw oo(ha(t,e)),xm}function Hv(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[Nt]=e,t[zt]=n,a){case"dialog":le("cancel",t),le("close",t);break;case"iframe":case"object":case"embed":le("load",t);break;case"video":case"audio":for(a=0;a<co.length;a++)le(co[a],t);break;case"source":le("error",t);break;case"img":case"image":case"link":le("error",t),le("load",t);break;case"details":le("toggle",t);break;case"input":le("invalid",t),oy(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),iu(t);break;case"select":le("invalid",t);break;case"textarea":le("invalid",t),uy(t,n.value,n.defaultValue,n.children),iu(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||c0(t.textContent,a)?(n.popover!=null&&(le("beforetoggle",t),le("toggle",t)),n.onScroll!=null&&le("scroll",t),n.onScrollEnd!=null&&le("scrollend",t),n.onClick!=null&&(t.onclick=Ku),t=!0):t=!1,t||$r(e)}function Kv(e){for(At=e.return;At;)switch(At.tag){case 5:case 13:Ua=!1;return;case 27:case 3:Ua=!0;return;default:At=At.return}}function Mi(e){if(e!==At)return!1;if(!pe)return Kv(e),pe=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||Qm(e.type,e.memoizedProps)),a=!a),a&&He&&$r(e),Kv(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(L(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){He=_a(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}He=null}}else t===27?(t=He,Wn(e.type)?(e=Ym,Ym=null,He=e):He=t):He=At?_a(e.stateNode.nextSibling):null;return!0}function No(){He=At=null,pe=!1}function Qv(){var e=gr;return e!==null&&(qt===null?qt=e:qt.push.apply(qt,e),gr=null),e}function oo(e){gr===null?gr=[e]:gr.push(e)}var $m=Ba(null),Cr=null,on=null;function Mn(e,t,a){Ue($m,t._currentValue),t._currentValue=a}function cn(e){e._currentValue=$m.current,pt($m)}function wm(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function Sm(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var u=0;u<t.length;u++)if(o.context===t[u]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),wm(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(L(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),wm(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function _o(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(L(387));if(i=i.memoizedProps,i!==null){var o=r.type;ea(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===au.current){if(i=r.alternate,i===null)throw Error(L(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(po):e=[po])}r=r.return}e!==null&&Sm(t,e,a,n),t.flags|=262144}function du(e){for(e=e.firstContext;e!==null;){if(!ea(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function wr(e){Cr=e,on=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function _t(e){return Ay(Cr,e)}function Ml(e,t){return Cr===null&&wr(e),Ay(e,t)}function Ay(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},on===null){if(e===null)throw Error(L(308));on=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else on=on.next=t;return a}var wC=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},SC=lt.unstable_scheduleCallback,NC=lt.unstable_NormalPriority,it={$$typeof:nn,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function gf(){return{controller:new wC,data:new Map,refCount:0}}function ko(e){e.refCount--,e.refCount===0&&SC(NC,function(){e.controller.abort()})}var Qi=null,Nm=0,Rs=0,bs=null;function _C(e,t){if(Qi===null){var a=Qi=[];Nm=0,Rs=qf(),bs={status:"pending",value:void 0,then:function(n){a.push(n)}}}return Nm++,t.then(Vv,Vv),t}function Vv(){if(--Nm===0&&Qi!==null){bs!==null&&(bs.status="fulfilled");var e=Qi;Qi=null,Rs=0,bs=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function kC(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var Gv=re.S;re.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&_C(e,t),Gv!==null&&Gv(e,t)};var yr=Ba(null);function yf(){var e=yr.current;return e!==null?e:Ae.pooledCache}function Gl(e,t){t===null?Ue(yr,yr.current):Ue(yr,t.pool)}function Ty(){var e=yf();return e===null?null:{parent:it._currentValue,pool:e}}var Ro=Error(L(460)),Dy=Error(L(474)),ju=Error(L(542)),_m={then:function(){}};function Yv(e){return e=e.status,e==="fulfilled"||e==="rejected"}function Ol(){}function My(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(Ol,Ol),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,Xv(e),e;default:if(typeof t.status=="string")t.then(Ol,Ol);else{if(e=Ae,e!==null&&100<e.shellSuspendCounter)throw Error(L(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,Xv(e),e}throw Vi=t,Ro}}var Vi=null;function Jv(){if(Vi===null)throw Error(L(459));var e=Vi;return Vi=null,e}function Xv(e){if(e===Ro||e===ju)throw Error(L(483))}var Dn=!1;function bf(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function km(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function Bn(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function In(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,(we&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=lu(e),Ry(e,null,a),t}return Uu(e,n,t,a),lu(e)}function Gi(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,ty(e,a)}}function qd(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var Rm=!1;function Yi(){if(Rm){var e=bs;if(e!==null)throw e}}function Ji(e,t,a,n){Rm=!1;var r=e.updateQueue;Dn=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var u=o,c=u.next;u.next=null,i===null?s=c:i.next=c,i=u;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=u))}if(s!==null){var f=r.baseState;i=0,d=c=u=null,o=s;do{var m=o.lane&-536870913,p=m!==o.lane;if(p?(de&m)===m:(n&m)===m){m!==0&&m===Rs&&(Rm=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var b=e,y=o;m=t;var $=a;switch(y.tag){case 1:if(b=y.payload,typeof b=="function"){f=b.call($,f,m);break e}f=b;break e;case 3:b.flags=b.flags&-65537|128;case 0:if(b=y.payload,m=typeof b=="function"?b.call($,f,m):b,m==null)break e;f=Me({},f,m);break e;case 2:Dn=!0}}m=o.callback,m!==null&&(e.flags|=64,p&&(e.flags|=8192),p=r.callbacks,p===null?r.callbacks=[m]:p.push(m))}else p={lane:m,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=p,u=f):d=d.next=p,i|=m;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;p=o,o=p.next,p.next=null,r.lastBaseUpdate=p,r.shared.pending=null}}while(!0);d===null&&(u=f),r.baseState=u,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),Xn|=i,e.lanes=i,e.memoizedState=f}}function Oy(e,t){if(typeof e!="function")throw Error(L(191,e));e.call(t)}function Ly(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)Oy(a[e],t)}var Cs=Ba(null),mu=Ba(0);function Zv(e,t){e=pn,Ue(mu,e),Ue(Cs,t),pn=e|t.baseLanes}function Cm(){Ue(mu,pn),Ue(Cs,Cs.current)}function xf(){pn=mu.current,pt(Cs),pt(mu)}var Yn=0,ie=null,_e=null,We=null,fu=!1,xs=!1,Sr=!1,pu=0,lo=0,$s=null,RC=0;function Ge(){throw Error(L(321))}function $f(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!ea(e[a],t[a]))return!1;return!0}function wf(e,t,a,n,r,s){return Yn=s,ie=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,re.H=e===null||e.memoizedState===null?mb:fb,Sr=!1,s=a(n,r),Sr=!1,xs&&(s=Uy(t,a,n,r)),Py(e),s}function Py(e){re.H=hu;var t=_e!==null&&_e.next!==null;if(Yn=0,We=_e=ie=null,fu=!1,lo=0,$s=null,t)throw Error(L(300));e===null||ft||(e=e.dependencies,e!==null&&du(e)&&(ft=!0))}function Uy(e,t,a,n){ie=e;var r=0;do{if(xs&&($s=null),lo=0,xs=!1,25<=r)throw Error(L(301));if(r+=1,We=_e=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}re.H=OC,s=t(a,n)}while(xs);return s}function CC(){var e=re.H,t=e.useState()[0];return t=typeof t.then=="function"?Co(t):t,e=e.useState()[0],(_e!==null?_e.memoizedState:null)!==e&&(ie.flags|=1024),t}function Sf(){var e=pu!==0;return pu=0,e}function Nf(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function _f(e){if(fu){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}fu=!1}Yn=0,We=_e=ie=null,xs=!1,lo=pu=0,$s=null}function jt(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return We===null?ie.memoizedState=We=e:We=We.next=e,We}function et(){if(_e===null){var e=ie.alternate;e=e!==null?e.memoizedState:null}else e=_e.next;var t=We===null?ie.memoizedState:We.next;if(t!==null)We=t,_e=e;else{if(e===null)throw ie.alternate===null?Error(L(467)):Error(L(310));_e=e,e={memoizedState:_e.memoizedState,baseState:_e.baseState,baseQueue:_e.baseQueue,queue:_e.queue,next:null},We===null?ie.memoizedState=We=e:We=We.next=e}return We}function kf(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function Co(e){var t=lo;return lo+=1,$s===null&&($s=[]),e=My($s,e,t),t=ie,(We===null?t.memoizedState:We.next)===null&&(t=t.alternate,re.H=t===null||t.memoizedState===null?mb:fb),e}function Fu(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return Co(e);if(e.$$typeof===nn)return _t(e)}throw Error(L(438,String(e)))}function Rf(e){var t=null,a=ie.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=ie.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=kf(),ie.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=mR;return t.index++,a}function mn(e,t){return typeof t=="function"?t(e):t}function Yl(e){var t=et();return Cf(t,_e,e)}function Cf(e,t,a){var n=e.queue;if(n===null)throw Error(L(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,u=null,c=t,d=!1;do{var f=c.lane&-536870913;if(f!==c.lane?(de&f)===f:(Yn&f)===f){var m=c.revertLane;if(m===0)u!==null&&(u=u.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),f===Rs&&(d=!0);else if((Yn&m)===m){c=c.next,m===Rs&&(d=!0);continue}else f={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=f,i=s):u=u.next=f,ie.lanes|=m,Xn|=m;f=c.action,Sr&&a(s,f),s=c.hasEagerState?c.eagerState:a(s,f)}else m={lane:f,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=m,i=s):u=u.next=m,ie.lanes|=f,Xn|=f;c=c.next}while(c!==null&&c!==t);if(u===null?i=s:u.next=o,!ea(s,e.memoizedState)&&(ft=!0,d&&(a=bs,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=u,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function zd(e){var t=et(),a=t.queue;if(a===null)throw Error(L(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);ea(s,t.memoizedState)||(ft=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function jy(e,t,a){var n=ie,r=et(),s=pe;if(s){if(a===void 0)throw Error(L(407));a=a()}else a=t();var i=!ea((_e||r).memoizedState,a);i&&(r.memoizedState=a,ft=!0),r=r.queue;var o=zy.bind(null,n,r,e);if(Eo(2048,8,o,[e]),r.getSnapshot!==t||i||We!==null&&We.memoizedState.tag&1){if(n.flags|=2048,Es(9,qu(),qy.bind(null,n,r,a,t),null),Ae===null)throw Error(L(349));s||(Yn&124)!==0||Fy(n,t,a)}return a}function Fy(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=ie.updateQueue,t===null?(t=kf(),ie.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function qy(e,t,a,n){t.value=a,t.getSnapshot=n,By(t)&&Iy(e)}function zy(e,t,a){return a(function(){By(t)&&Iy(e)})}function By(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!ea(e,a)}catch{return!0}}function Iy(e){var t=Us(e,2);t!==null&&Wt(t,e,2)}function Em(e){var t=jt();if(typeof e=="function"){var a=e;if(e=a(),Sr){Un(!0);try{a()}finally{Un(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:mn,lastRenderedState:e},t}function Hy(e,t,a,n){return e.baseState=a,Cf(e,_e,typeof n=="function"?n:mn)}function EC(e,t,a,n,r){if(zu(e))throw Error(L(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};re.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,Ky(t,s)):(s.next=a.next,t.pending=a.next=s)}}function Ky(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=re.T,i={};re.T=i;try{var o=a(r,n),u=re.S;u!==null&&u(i,o),Wv(e,t,o)}catch(c){Am(e,t,c)}finally{re.T=s}}else try{s=a(r,n),Wv(e,t,s)}catch(c){Am(e,t,c)}}function Wv(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){eg(e,t,n)},function(n){return Am(e,t,n)}):eg(e,t,a)}function eg(e,t,a){t.status="fulfilled",t.value=a,Qy(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,Ky(e,a)))}function Am(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,Qy(t),t=t.next;while(t!==n)}e.action=null}function Qy(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function Vy(e,t){return t}function tg(e,t){if(pe){var a=Ae.formState;if(a!==null){e:{var n=ie;if(pe){if(He){t:{for(var r=He,s=Ua;r.nodeType!==8;){if(!s){r=null;break t}if(r=_a(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){He=_a(r.nextSibling),n=r.data==="F!";break e}}$r(n)}n=!1}n&&(t=a[0])}}return a=jt(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:Vy,lastRenderedState:t},a.queue=n,a=ub.bind(null,ie,n),n.dispatch=a,n=Em(!1),s=Df.bind(null,ie,!1,n.queue),n=jt(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=EC.bind(null,ie,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function ag(e){var t=et();return Gy(t,_e,e)}function Gy(e,t,a){if(t=Cf(e,t,Vy)[0],e=Yl(mn)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=Co(t)}catch(i){throw i===Ro?ju:i}else n=t;t=et();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(ie.flags|=2048,Es(9,qu(),AC.bind(null,r,a),null)),[n,s,e]}function AC(e,t){e.action=t}function ng(e){var t=et(),a=_e;if(a!==null)return Gy(t,a,e);et(),t=t.memoizedState,a=et();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function Es(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=ie.updateQueue,t===null&&(t=kf(),ie.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function qu(){return{destroy:void 0,resource:void 0}}function Yy(){return et().memoizedState}function Jl(e,t,a,n){var r=jt();n=n===void 0?null:n,ie.flags|=e,r.memoizedState=Es(1|t,qu(),a,n)}function Eo(e,t,a,n){var r=et();n=n===void 0?null:n;var s=r.memoizedState.inst;_e!==null&&n!==null&&$f(n,_e.memoizedState.deps)?r.memoizedState=Es(t,s,a,n):(ie.flags|=e,r.memoizedState=Es(1|t,s,a,n))}function rg(e,t){Jl(8390656,8,e,t)}function Jy(e,t){Eo(2048,8,e,t)}function Xy(e,t){return Eo(4,2,e,t)}function Zy(e,t){return Eo(4,4,e,t)}function Wy(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function eb(e,t,a){a=a!=null?a.concat([e]):null,Eo(4,4,Wy.bind(null,t,e),a)}function Ef(){}function tb(e,t){var a=et();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&$f(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function ab(e,t){var a=et();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&$f(t,n[1]))return n[0];if(n=e(),Sr){Un(!0);try{e()}finally{Un(!1)}}return a.memoizedState=[n,t],n}function Af(e,t,a){return a===void 0||(Yn&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=Qb(),ie.lanes|=e,Xn|=e,a)}function nb(e,t,a,n){return ea(a,t)?a:Cs.current!==null?(e=Af(e,a,n),ea(e,t)||(ft=!0),e):(Yn&42)===0?(ft=!0,e.memoizedState=a):(e=Qb(),ie.lanes|=e,Xn|=e,t)}function rb(e,t,a,n,r){var s=he.p;he.p=s!==0&&8>s?s:8;var i=re.T,o={};re.T=o,Df(e,!1,t,a);try{var u=r(),c=re.S;if(c!==null&&c(o,u),u!==null&&typeof u=="object"&&typeof u.then=="function"){var d=kC(u,n);Xi(e,t,d,Zt(e))}else Xi(e,t,n,Zt(e))}catch(f){Xi(e,t,{then:function(){},status:"rejected",reason:f},Zt())}finally{he.p=s,re.T=i}}function TC(){}function Tm(e,t,a,n){if(e.tag!==5)throw Error(L(476));var r=sb(e).queue;rb(e,r,t,pr,a===null?TC:function(){return ib(e),a(n)})}function sb(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:pr,baseState:pr,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:mn,lastRenderedState:pr},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:mn,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function ib(e){var t=sb(e).next.queue;Xi(e,t,{},Zt())}function Tf(){return _t(po)}function ob(){return et().memoizedState}function lb(){return et().memoizedState}function DC(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=Zt();e=Bn(a);var n=In(t,e,a);n!==null&&(Wt(n,t,a),Gi(n,t,a)),t={cache:gf()},e.payload=t;return}t=t.return}}function MC(e,t,a){var n=Zt();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},zu(e)?cb(t,a):(a=ff(e,t,a,n),a!==null&&(Wt(a,e,n),db(a,t,n)))}function ub(e,t,a){var n=Zt();Xi(e,t,a,n)}function Xi(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(zu(e))cb(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,ea(o,i))return Uu(e,t,r,0),Ae===null&&Pu(),!1}catch{}finally{}if(a=ff(e,t,r,n),a!==null)return Wt(a,e,n),db(a,t,n),!0}return!1}function Df(e,t,a,n){if(n={lane:2,revertLane:qf(),action:n,hasEagerState:!1,eagerState:null,next:null},zu(e)){if(t)throw Error(L(479))}else t=ff(e,a,n,2),t!==null&&Wt(t,e,2)}function zu(e){var t=e.alternate;return e===ie||t!==null&&t===ie}function cb(e,t){xs=fu=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function db(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,ty(e,a)}}var hu={readContext:_t,use:Fu,useCallback:Ge,useContext:Ge,useEffect:Ge,useImperativeHandle:Ge,useLayoutEffect:Ge,useInsertionEffect:Ge,useMemo:Ge,useReducer:Ge,useRef:Ge,useState:Ge,useDebugValue:Ge,useDeferredValue:Ge,useTransition:Ge,useSyncExternalStore:Ge,useId:Ge,useHostTransitionStatus:Ge,useFormState:Ge,useActionState:Ge,useOptimistic:Ge,useMemoCache:Ge,useCacheRefresh:Ge},mb={readContext:_t,use:Fu,useCallback:function(e,t){return jt().memoizedState=[e,t===void 0?null:t],e},useContext:_t,useEffect:rg,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,Jl(4194308,4,Wy.bind(null,t,e),a)},useLayoutEffect:function(e,t){return Jl(4194308,4,e,t)},useInsertionEffect:function(e,t){Jl(4,2,e,t)},useMemo:function(e,t){var a=jt();t=t===void 0?null:t;var n=e();if(Sr){Un(!0);try{e()}finally{Un(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=jt();if(a!==void 0){var r=a(t);if(Sr){Un(!0);try{a(t)}finally{Un(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=MC.bind(null,ie,e),[n.memoizedState,e]},useRef:function(e){var t=jt();return e={current:e},t.memoizedState=e},useState:function(e){e=Em(e);var t=e.queue,a=ub.bind(null,ie,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:Ef,useDeferredValue:function(e,t){var a=jt();return Af(a,e,t)},useTransition:function(){var e=Em(!1);return e=rb.bind(null,ie,e.queue,!0,!1),jt().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=ie,r=jt();if(pe){if(a===void 0)throw Error(L(407));a=a()}else{if(a=t(),Ae===null)throw Error(L(349));(de&124)!==0||Fy(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,rg(zy.bind(null,n,s,e),[e]),n.flags|=2048,Es(9,qu(),qy.bind(null,n,s,a,t),null),a},useId:function(){var e=jt(),t=Ae.identifierPrefix;if(pe){var a=sn,n=rn;a=(n&~(1<<32-Xt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=pu++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=RC++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:Tf,useFormState:tg,useActionState:tg,useOptimistic:function(e){var t=jt();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=Df.bind(null,ie,!0,a),a.dispatch=t,[e,t]},useMemoCache:Rf,useCacheRefresh:function(){return jt().memoizedState=DC.bind(null,ie)}},fb={readContext:_t,use:Fu,useCallback:tb,useContext:_t,useEffect:Jy,useImperativeHandle:eb,useInsertionEffect:Xy,useLayoutEffect:Zy,useMemo:ab,useReducer:Yl,useRef:Yy,useState:function(){return Yl(mn)},useDebugValue:Ef,useDeferredValue:function(e,t){var a=et();return nb(a,_e.memoizedState,e,t)},useTransition:function(){var e=Yl(mn)[0],t=et().memoizedState;return[typeof e=="boolean"?e:Co(e),t]},useSyncExternalStore:jy,useId:ob,useHostTransitionStatus:Tf,useFormState:ag,useActionState:ag,useOptimistic:function(e,t){var a=et();return Hy(a,_e,e,t)},useMemoCache:Rf,useCacheRefresh:lb},OC={readContext:_t,use:Fu,useCallback:tb,useContext:_t,useEffect:Jy,useImperativeHandle:eb,useInsertionEffect:Xy,useLayoutEffect:Zy,useMemo:ab,useReducer:zd,useRef:Yy,useState:function(){return zd(mn)},useDebugValue:Ef,useDeferredValue:function(e,t){var a=et();return _e===null?Af(a,e,t):nb(a,_e.memoizedState,e,t)},useTransition:function(){var e=zd(mn)[0],t=et().memoizedState;return[typeof e=="boolean"?e:Co(e),t]},useSyncExternalStore:jy,useId:ob,useHostTransitionStatus:Tf,useFormState:ng,useActionState:ng,useOptimistic:function(e,t){var a=et();return _e!==null?Hy(a,_e,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:Rf,useCacheRefresh:lb},ws=null,uo=0;function Ll(e){var t=uo;return uo+=1,ws===null&&(ws=[]),My(ws,e,t)}function Oi(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function Pl(e,t){throw t.$$typeof===cR?Error(L(525)):(e=Object.prototype.toString.call(t),Error(L(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function sg(e){var t=e._init;return t(e._payload)}function pb(e){function t(g,v){if(e){var x=g.deletions;x===null?(g.deletions=[v],g.flags|=16):x.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=un(g,v),g.index=0,g.sibling=null,g}function s(g,v,x){return g.index=x,e?(x=g.alternate,x!==null?(x=x.index,x<v?(g.flags|=67108866,v):x):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,x,w){return v===null||v.tag!==6?(v=jd(x,g.mode,w),v.return=g,v):(v=r(v,x),v.return=g,v)}function u(g,v,x,w){var S=x.type;return S===rs?d(g,v,x.props.children,w,x.key):v!==null&&(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Tn&&sg(S)===v.type)?(v=r(v,x.props),Oi(v,x),v.return=g,v):(v=Vl(x.type,x.key,x.props,null,g.mode,w),Oi(v,x),v.return=g,v)}function c(g,v,x,w){return v===null||v.tag!==4||v.stateNode.containerInfo!==x.containerInfo||v.stateNode.implementation!==x.implementation?(v=Fd(x,g.mode,w),v.return=g,v):(v=r(v,x.children||[]),v.return=g,v)}function d(g,v,x,w,S){return v===null||v.tag!==7?(v=hr(x,g.mode,w,S),v.return=g,v):(v=r(v,x),v.return=g,v)}function f(g,v,x){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=jd(""+v,g.mode,x),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case kl:return x=Vl(v.type,v.key,v.props,null,g.mode,x),Oi(x,v),x.return=g,x;case Fi:return v=Fd(v,g.mode,x),v.return=g,v;case Tn:var w=v._init;return v=w(v._payload),f(g,v,x)}if(qi(v)||Ti(v))return v=hr(v,g.mode,x,null),v.return=g,v;if(typeof v.then=="function")return f(g,Ll(v),x);if(v.$$typeof===nn)return f(g,Ml(g,v),x);Pl(g,v)}return null}function m(g,v,x,w){var S=v!==null?v.key:null;if(typeof x=="string"&&x!==""||typeof x=="number"||typeof x=="bigint")return S!==null?null:o(g,v,""+x,w);if(typeof x=="object"&&x!==null){switch(x.$$typeof){case kl:return x.key===S?u(g,v,x,w):null;case Fi:return x.key===S?c(g,v,x,w):null;case Tn:return S=x._init,x=S(x._payload),m(g,v,x,w)}if(qi(x)||Ti(x))return S!==null?null:d(g,v,x,w,null);if(typeof x.then=="function")return m(g,v,Ll(x),w);if(x.$$typeof===nn)return m(g,v,Ml(g,x),w);Pl(g,x)}return null}function p(g,v,x,w,S){if(typeof w=="string"&&w!==""||typeof w=="number"||typeof w=="bigint")return g=g.get(x)||null,o(v,g,""+w,S);if(typeof w=="object"&&w!==null){switch(w.$$typeof){case kl:return g=g.get(w.key===null?x:w.key)||null,u(v,g,w,S);case Fi:return g=g.get(w.key===null?x:w.key)||null,c(v,g,w,S);case Tn:var R=w._init;return w=R(w._payload),p(g,v,x,w,S)}if(qi(w)||Ti(w))return g=g.get(x)||null,d(v,g,w,S,null);if(typeof w.then=="function")return p(g,v,x,Ll(w),S);if(w.$$typeof===nn)return p(g,v,x,Ml(v,w),S);Pl(v,w)}return null}function b(g,v,x,w){for(var S=null,R=null,N=v,A=v=0,M=null;N!==null&&A<x.length;A++){N.index>A?(M=N,N=null):M=N.sibling;var P=m(g,N,x[A],w);if(P===null){N===null&&(N=M);break}e&&N&&P.alternate===null&&t(g,N),v=s(P,v,A),R===null?S=P:R.sibling=P,R=P,N=M}if(A===x.length)return a(g,N),pe&&mr(g,A),S;if(N===null){for(;A<x.length;A++)N=f(g,x[A],w),N!==null&&(v=s(N,v,A),R===null?S=N:R.sibling=N,R=N);return pe&&mr(g,A),S}for(N=n(N);A<x.length;A++)M=p(N,g,A,x[A],w),M!==null&&(e&&M.alternate!==null&&N.delete(M.key===null?A:M.key),v=s(M,v,A),R===null?S=M:R.sibling=M,R=M);return e&&N.forEach(function(G){return t(g,G)}),pe&&mr(g,A),S}function y(g,v,x,w){if(x==null)throw Error(L(151));for(var S=null,R=null,N=v,A=v=0,M=null,P=x.next();N!==null&&!P.done;A++,P=x.next()){N.index>A?(M=N,N=null):M=N.sibling;var G=m(g,N,P.value,w);if(G===null){N===null&&(N=M);break}e&&N&&G.alternate===null&&t(g,N),v=s(G,v,A),R===null?S=G:R.sibling=G,R=G,N=M}if(P.done)return a(g,N),pe&&mr(g,A),S;if(N===null){for(;!P.done;A++,P=x.next())P=f(g,P.value,w),P!==null&&(v=s(P,v,A),R===null?S=P:R.sibling=P,R=P);return pe&&mr(g,A),S}for(N=n(N);!P.done;A++,P=x.next())P=p(N,g,A,P.value,w),P!==null&&(e&&P.alternate!==null&&N.delete(P.key===null?A:P.key),v=s(P,v,A),R===null?S=P:R.sibling=P,R=P);return e&&N.forEach(function(C){return t(g,C)}),pe&&mr(g,A),S}function $(g,v,x,w){if(typeof x=="object"&&x!==null&&x.type===rs&&x.key===null&&(x=x.props.children),typeof x=="object"&&x!==null){switch(x.$$typeof){case kl:e:{for(var S=x.key;v!==null;){if(v.key===S){if(S=x.type,S===rs){if(v.tag===7){a(g,v.sibling),w=r(v,x.props.children),w.return=g,g=w;break e}}else if(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Tn&&sg(S)===v.type){a(g,v.sibling),w=r(v,x.props),Oi(w,x),w.return=g,g=w;break e}a(g,v);break}else t(g,v);v=v.sibling}x.type===rs?(w=hr(x.props.children,g.mode,w,x.key),w.return=g,g=w):(w=Vl(x.type,x.key,x.props,null,g.mode,w),Oi(w,x),w.return=g,g=w)}return i(g);case Fi:e:{for(S=x.key;v!==null;){if(v.key===S)if(v.tag===4&&v.stateNode.containerInfo===x.containerInfo&&v.stateNode.implementation===x.implementation){a(g,v.sibling),w=r(v,x.children||[]),w.return=g,g=w;break e}else{a(g,v);break}else t(g,v);v=v.sibling}w=Fd(x,g.mode,w),w.return=g,g=w}return i(g);case Tn:return S=x._init,x=S(x._payload),$(g,v,x,w)}if(qi(x))return b(g,v,x,w);if(Ti(x)){if(S=Ti(x),typeof S!="function")throw Error(L(150));return x=S.call(x),y(g,v,x,w)}if(typeof x.then=="function")return $(g,v,Ll(x),w);if(x.$$typeof===nn)return $(g,v,Ml(g,x),w);Pl(g,x)}return typeof x=="string"&&x!==""||typeof x=="number"||typeof x=="bigint"?(x=""+x,v!==null&&v.tag===6?(a(g,v.sibling),w=r(v,x),w.return=g,g=w):(a(g,v),w=jd(x,g.mode,w),w.return=g,g=w),i(g)):a(g,v)}return function(g,v,x,w){try{uo=0;var S=$(g,v,x,w);return ws=null,S}catch(N){if(N===Ro||N===ju)throw N;var R=Yt(29,N,null,g.mode);return R.lanes=w,R.return=g,R}finally{}}}var As=pb(!0),hb=pb(!1),ga=Ba(null),za=null;function On(e){var t=e.alternate;Ue(ot,ot.current&1),Ue(ga,e),za===null&&(t===null||Cs.current!==null||t.memoizedState!==null)&&(za=e)}function vb(e){if(e.tag===22){if(Ue(ot,ot.current),Ue(ga,e),za===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(za=e)}}else Ln(e)}function Ln(){Ue(ot,ot.current),Ue(ga,ga.current)}function ln(e){pt(ga),za===e&&(za=null),pt(ot)}var ot=Ba(0);function vu(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||Gm(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function Bd(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:Me({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var Dm={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=Zt(),r=Bn(n);r.payload=t,a!=null&&(r.callback=a),t=In(e,r,n),t!==null&&(Wt(t,e,n),Gi(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=Zt(),r=Bn(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=In(e,r,n),t!==null&&(Wt(t,e,n),Gi(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=Zt(),n=Bn(a);n.tag=2,t!=null&&(n.callback=t),t=In(e,n,a),t!==null&&(Wt(t,e,a),Gi(t,e,a))}};function ig(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!io(a,n)||!io(r,s):!0}function og(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&Dm.enqueueReplaceState(t,t.state,null)}function Nr(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=Me({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var gu=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function gb(e){gu(e)}function yb(e){console.error(e)}function bb(e){gu(e)}function yu(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function lg(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function Mm(e,t,a){return a=Bn(a),a.tag=3,a.payload={element:null},a.callback=function(){yu(e,t)},a}function xb(e){return e=Bn(e),e.tag=3,e}function $b(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){lg(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){lg(t,a,n),typeof r!="function"&&(Hn===null?Hn=new Set([this]):Hn.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function LC(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&_o(t,a,r,!0),a=ga.current,a!==null){switch(a.tag){case 13:return za===null?zm():a.alternate===null&&Ke===0&&(Ke=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===_m?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),Wd(e,n,r)),!1;case 22:return a.flags|=65536,n===_m?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),Wd(e,n,r)),!1}throw Error(L(435,a.tag))}return Wd(e,n,r),zm(),!1}if(pe)return t=ga.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==xm&&(e=Error(L(422),{cause:n}),oo(ha(e,a)))):(n!==xm&&(t=Error(L(423),{cause:n}),oo(ha(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=ha(n,a),r=Mm(e.stateNode,n,r),qd(e,r),Ke!==4&&(Ke=2)),!1;var s=Error(L(520),{cause:n});if(s=ha(s,a),eo===null?eo=[s]:eo.push(s),Ke!==4&&(Ke=2),t===null)return!0;n=ha(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=Mm(a.stateNode,n,e),qd(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(Hn===null||!Hn.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=xb(r),$b(r,e,a,n),qd(a,r),!1}a=a.return}while(a!==null);return!1}var wb=Error(L(461)),ft=!1;function yt(e,t,a,n){t.child=e===null?hb(t,null,a,n):As(t,e.child,a,n)}function ug(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return wr(t),n=wf(e,t,a,i,s,r),o=Sf(),e!==null&&!ft?(Nf(e,t,r),fn(e,t,r)):(pe&&o&&hf(t),t.flags|=1,yt(e,t,n,r),t.child)}function cg(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!pf(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,Sb(e,t,s,n,r)):(e=Vl(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!Mf(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:io,a(i,n)&&e.ref===t.ref)return fn(e,t,r)}return t.flags|=1,e=un(s,n),e.ref=t.ref,e.return=t,t.child=e}function Sb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if(io(s,n)&&e.ref===t.ref)if(ft=!1,t.pendingProps=n=s,Mf(e,r))(e.flags&131072)!==0&&(ft=!0);else return t.lanes=e.lanes,fn(e,t,r)}return Om(e,t,a,n,r)}function Nb(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return dg(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&Gl(t,s!==null?s.cachePool:null),s!==null?Zv(t,s):Cm(),vb(t);else return t.lanes=t.childLanes=536870912,dg(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(Gl(t,s.cachePool),Zv(t,s),Ln(t),t.memoizedState=null):(e!==null&&Gl(t,null),Cm(),Ln(t));return yt(e,t,r,a),t.child}function dg(e,t,a,n){var r=yf();return r=r===null?null:{parent:it._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&Gl(t,null),Cm(),vb(t),e!==null&&_o(e,t,n,!0),null}function Xl(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(L(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function Om(e,t,a,n,r){return wr(t),a=wf(e,t,a,n,void 0,r),n=Sf(),e!==null&&!ft?(Nf(e,t,r),fn(e,t,r)):(pe&&n&&hf(t),t.flags|=1,yt(e,t,a,r),t.child)}function mg(e,t,a,n,r,s){return wr(t),t.updateQueue=null,a=Uy(t,n,a,r),Py(e),n=Sf(),e!==null&&!ft?(Nf(e,t,s),fn(e,t,s)):(pe&&n&&hf(t),t.flags|=1,yt(e,t,a,s),t.child)}function fg(e,t,a,n,r){if(wr(t),t.stateNode===null){var s=ms,i=a.contextType;typeof i=="object"&&i!==null&&(s=_t(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=Dm,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},bf(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?_t(i):ms,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(Bd(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&Dm.enqueueReplaceState(s,s.state,null),Ji(t,n,s,r),Yi(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,u=Nr(a,o);s.props=u;var c=s.context,d=a.contextType;i=ms,typeof d=="object"&&d!==null&&(i=_t(d));var f=a.getDerivedStateFromProps;d=typeof f=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&og(t,s,n,i),Dn=!1;var m=t.memoizedState;s.state=m,Ji(t,n,s,r),Yi(),c=t.memoizedState,o||m!==c||Dn?(typeof f=="function"&&(Bd(t,a,f,n),c=t.memoizedState),(u=Dn||ig(t,a,u,n,m,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=u):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,km(e,t),i=t.memoizedProps,d=Nr(a,i),s.props=d,f=t.pendingProps,m=s.context,c=a.contextType,u=ms,typeof c=="object"&&c!==null&&(u=_t(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==f||m!==u)&&og(t,s,n,u),Dn=!1,m=t.memoizedState,s.state=m,Ji(t,n,s,r),Yi();var p=t.memoizedState;i!==f||m!==p||Dn||e!==null&&e.dependencies!==null&&du(e.dependencies)?(typeof o=="function"&&(Bd(t,a,o,n),p=t.memoizedState),(d=Dn||ig(t,a,d,n,m,p,u)||e!==null&&e.dependencies!==null&&du(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,p,u),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,p,u)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=p),s.props=n,s.state=p,s.context=u,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,Xl(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=As(t,e.child,null,r),t.child=As(t,null,a,r)):yt(e,t,a,r),t.memoizedState=s.state,e=t.child):e=fn(e,t,r),e}function pg(e,t,a,n){return No(),t.flags|=256,yt(e,t,a,n),t.child}var Id={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function Hd(e){return{baseLanes:e,cachePool:Ty()}}function Kd(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=va),e}function _b(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(ot.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(pe){if(r?On(t):Ln(t),pe){var o=He,u;if(u=o){e:{for(u=o,o=Ua;u.nodeType!==8;){if(!o){o=null;break e}if(u=_a(u.nextSibling),u===null){o=null;break e}}o=u}o!==null?(t.memoizedState={dehydrated:o,treeContext:vr!==null?{id:rn,overflow:sn}:null,retryLane:536870912,hydrationErrors:null},u=Yt(18,null,null,0),u.stateNode=o,u.return=t,t.child=u,At=t,He=null,u=!0):u=!1}u||$r(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return Gm(o)?t.lanes=32:t.lanes=536870912,null;ln(t)}return o=n.children,n=n.fallback,r?(Ln(t),r=t.mode,o=bu({mode:"hidden",children:o},r),n=hr(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=Hd(a),r.childLanes=Kd(e,i,a),t.memoizedState=Id,n):(On(t),Lm(t,o))}if(u=e.memoizedState,u!==null&&(o=u.dehydrated,o!==null)){if(s)t.flags&256?(On(t),t.flags&=-257,t=Qd(e,t,a)):t.memoizedState!==null?(Ln(t),t.child=e.child,t.flags|=128,t=null):(Ln(t),r=n.fallback,o=t.mode,n=bu({mode:"visible",children:n.children},o),r=hr(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,As(t,e.child,null,a),n=t.child,n.memoizedState=Hd(a),n.childLanes=Kd(e,i,a),t.memoizedState=Id,t=r);else if(On(t),Gm(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(L(419)),n.stack="",n.digest=i,oo({value:n,source:null,stack:null}),t=Qd(e,t,a)}else if(ft||_o(e,t,a,!1),i=(a&e.childLanes)!==0,ft||i){if(i=Ae,i!==null&&(n=a&-a,n=(n&42)!==0?1:af(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==u.retryLane))throw u.retryLane=n,Us(e,n),Wt(i,e,n),wb;o.data==="$?"||zm(),t=Qd(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=u.treeContext,He=_a(o.nextSibling),At=t,pe=!0,gr=null,Ua=!1,e!==null&&(ma[fa++]=rn,ma[fa++]=sn,ma[fa++]=vr,rn=e.id,sn=e.overflow,vr=t),t=Lm(t,n.children),t.flags|=4096);return t}return r?(Ln(t),r=n.fallback,o=t.mode,u=e.child,c=u.sibling,n=un(u,{mode:"hidden",children:n.children}),n.subtreeFlags=u.subtreeFlags&65011712,c!==null?r=un(c,r):(r=hr(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=Hd(a):(u=o.cachePool,u!==null?(c=it._currentValue,u=u.parent!==c?{parent:c,pool:c}:u):u=Ty(),o={baseLanes:o.baseLanes|a,cachePool:u}),r.memoizedState=o,r.childLanes=Kd(e,i,a),t.memoizedState=Id,n):(On(t),a=e.child,e=a.sibling,a=un(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function Lm(e,t){return t=bu({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function bu(e,t){return e=Yt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function Qd(e,t,a){return As(t,e.child,null,a),e=Lm(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function hg(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),wm(e.return,t,a)}function Vd(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function kb(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(yt(e,t,n.children,a),n=ot.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&hg(e,a,t);else if(e.tag===19)hg(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Ue(ot,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&vu(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),Vd(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&vu(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}Vd(t,!0,a,null,s);break;case"together":Vd(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function fn(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),Xn|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(_o(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(L(153));if(t.child!==null){for(e=t.child,a=un(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=un(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function Mf(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&du(e)))}function PC(e,t,a){switch(t.tag){case 3:nu(t,t.stateNode.containerInfo),Mn(t,it,e.memoizedState.cache),No();break;case 27:case 5:cm(t);break;case 4:nu(t,t.stateNode.containerInfo);break;case 10:Mn(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(On(t),t.flags|=128,null):(a&t.child.childLanes)!==0?_b(e,t,a):(On(t),e=fn(e,t,a),e!==null?e.sibling:null);On(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(_o(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return kb(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Ue(ot,ot.current),n)break;return null;case 22:case 23:return t.lanes=0,Nb(e,t,a);case 24:Mn(t,it,e.memoizedState.cache)}return fn(e,t,a)}function Rb(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)ft=!0;else{if(!Mf(e,a)&&(t.flags&128)===0)return ft=!1,PC(e,t,a);ft=(e.flags&131072)!==0}else ft=!1,pe&&(t.flags&1048576)!==0&&Ey(t,cu,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")pf(n)?(e=Nr(n,e),t.tag=1,t=fg(null,t,n,e,a)):(t.tag=0,t=Om(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===Wm){t.tag=11,t=ug(null,t,n,e,a);break e}else if(r===ef){t.tag=14,t=cg(null,t,n,e,a);break e}}throw t=lm(n)||n,Error(L(306,t,""))}}return t;case 0:return Om(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Nr(n,t.pendingProps),fg(e,t,n,r,a);case 3:e:{if(nu(t,t.stateNode.containerInfo),e===null)throw Error(L(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,km(e,t),Ji(t,n,null,a);var i=t.memoizedState;if(n=i.cache,Mn(t,it,n),n!==s.cache&&Sm(t,[it],a,!0),Yi(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=pg(e,t,n,a);break e}else if(n!==r){r=ha(Error(L(424)),t),oo(r),t=pg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(He=_a(e.firstChild),At=t,pe=!0,gr=null,Ua=!0,a=hb(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if(No(),n===r){t=fn(e,t,a);break e}yt(e,t,n,a)}t=t.child}return t;case 26:return Xl(e,t),e===null?(a=Og(t.type,null,t.pendingProps,null))?t.memoizedState=a:pe||(a=t.type,e=t.pendingProps,n=ku(zn.current).createElement(a),n[Nt]=t,n[zt]=e,xt(n,a,e),mt(n),t.stateNode=n):t.memoizedState=Og(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return cm(t),e===null&&pe&&(n=t.stateNode=f0(t.type,t.pendingProps,zn.current),At=t,Ua=!0,r=He,Wn(t.type)?(Ym=r,He=_a(n.firstChild)):He=r),yt(e,t,t.pendingProps.children,a),Xl(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&pe&&((r=n=He)&&(n=l3(n,t.type,t.pendingProps,Ua),n!==null?(t.stateNode=n,At=t,He=_a(n.firstChild),Ua=!1,r=!0):r=!1),r||$r(t)),cm(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,Qm(r,s)?n=null:i!==null&&Qm(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=wf(e,t,CC,null,null,a),po._currentValue=r),Xl(e,t),yt(e,t,n,a),t.child;case 6:return e===null&&pe&&((e=a=He)&&(a=u3(a,t.pendingProps,Ua),a!==null?(t.stateNode=a,At=t,He=null,e=!0):e=!1),e||$r(t)),null;case 13:return _b(e,t,a);case 4:return nu(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=As(t,null,n,a):yt(e,t,n,a),t.child;case 11:return ug(e,t,t.type,t.pendingProps,a);case 7:return yt(e,t,t.pendingProps,a),t.child;case 8:return yt(e,t,t.pendingProps.children,a),t.child;case 12:return yt(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,Mn(t,t.type,n.value),yt(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,wr(t),r=_t(r),n=n(r),t.flags|=1,yt(e,t,n,a),t.child;case 14:return cg(e,t,t.type,t.pendingProps,a);case 15:return Sb(e,t,t.type,t.pendingProps,a);case 19:return kb(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=bu(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=un(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return Nb(e,t,a);case 24:return wr(t),n=_t(it),e===null?(r=yf(),r===null&&(r=Ae,s=gf(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},bf(t),Mn(t,it,r)):((e.lanes&a)!==0&&(km(e,t),Ji(t,null,null,a),Yi()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),Mn(t,it,n)):(n=s.cache,Mn(t,it,n),n!==r.cache&&Sm(t,[it],a,!0))),yt(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(L(156,t.tag))}function en(e){e.flags|=4}function vg(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!v0(t)){if(t=ga.current,t!==null&&((de&4194048)===de?za!==null:(de&62914560)!==de&&(de&536870912)===0||t!==za))throw Vi=_m,Dy;e.flags|=8192}}function Ul(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?Wg():536870912,e.lanes|=t,Ts|=t)}function Li(e,t){if(!pe)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function ze(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function UC(e,t,a){var n=t.pendingProps;switch(vf(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return ze(t),null;case 1:return ze(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),cn(it),Ns(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Mi(t)?en(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,Qv())),ze(t),null;case 26:return a=t.memoizedState,e===null?(en(t),a!==null?(ze(t),vg(t,a)):(ze(t),t.flags&=-16777217)):a?a!==e.memoizedState?(en(t),ze(t),vg(t,a)):(ze(t),t.flags&=-16777217):(e.memoizedProps!==n&&en(t),ze(t),t.flags&=-16777217),null;case 27:ru(t),a=zn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&en(t);else{if(!n){if(t.stateNode===null)throw Error(L(166));return ze(t),null}e=Fa.current,Mi(t)?Hv(t,e):(e=f0(r,n,a),t.stateNode=e,en(t))}return ze(t),null;case 5:if(ru(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&en(t);else{if(!n){if(t.stateNode===null)throw Error(L(166));return ze(t),null}if(e=Fa.current,Mi(t))Hv(t,e);else{switch(r=ku(zn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[Nt]=t,e[zt]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(xt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&en(t)}}return ze(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&en(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(L(166));if(e=zn.current,Mi(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=At,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[Nt]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||c0(e.nodeValue,a)),e||$r(t)}else e=ku(e).createTextNode(n),e[Nt]=t,t.stateNode=e}return ze(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Mi(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(L(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(L(317));r[Nt]=t}else No(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;ze(t),r=!1}else r=Qv(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(ln(t),t):(ln(t),null)}if(ln(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),Ul(t,t.updateQueue),ze(t),null;case 4:return Ns(),e===null&&zf(t.stateNode.containerInfo),ze(t),null;case 10:return cn(t.type),ze(t),null;case 19:if(pt(ot),r=t.memoizedState,r===null)return ze(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)Li(r,!1);else{if(Ke!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=vu(e),s!==null){for(t.flags|=128,Li(r,!1),e=s.updateQueue,t.updateQueue=e,Ul(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)Cy(a,e),a=a.sibling;return Ue(ot,ot.current&1|2),t.child}e=e.sibling}r.tail!==null&&qa()>$u&&(t.flags|=128,n=!0,Li(r,!1),t.lanes=4194304)}else{if(!n)if(e=vu(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,Ul(t,e),Li(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!pe)return ze(t),null}else 2*qa()-r.renderingStartTime>$u&&a!==536870912&&(t.flags|=128,n=!0,Li(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=qa(),t.sibling=null,e=ot.current,Ue(ot,n?e&1|2:e&1),t):(ze(t),null);case 22:case 23:return ln(t),xf(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(ze(t),t.subtreeFlags&6&&(t.flags|=8192)):ze(t),a=t.updateQueue,a!==null&&Ul(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&pt(yr),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),cn(it),ze(t),null;case 25:return null;case 30:return null}throw Error(L(156,t.tag))}function jC(e,t){switch(vf(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return cn(it),Ns(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return ru(t),null;case 13:if(ln(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(L(340));No()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return pt(ot),null;case 4:return Ns(),null;case 10:return cn(t.type),null;case 22:case 23:return ln(t),xf(),e!==null&&pt(yr),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return cn(it),null;case 25:return null;default:return null}}function Cb(e,t){switch(vf(t),t.tag){case 3:cn(it),Ns();break;case 26:case 27:case 5:ru(t);break;case 4:Ns();break;case 13:ln(t);break;case 19:pt(ot);break;case 10:cn(t.type);break;case 22:case 23:ln(t),xf(),e!==null&&pt(yr);break;case 24:cn(it)}}function Ao(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){ke(t,t.return,o)}}function Jn(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var u=a,c=o;try{c()}catch(d){ke(r,u,d)}}}n=n.next}while(n!==s)}}catch(d){ke(t,t.return,d)}}function Eb(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{Ly(t,a)}catch(n){ke(e,e.return,n)}}}function Ab(e,t,a){a.props=Nr(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){ke(e,t,n)}}function Zi(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){ke(e,t,r)}}function ja(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){ke(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){ke(e,t,r)}else a.current=null}function Tb(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){ke(e,e.return,r)}}function Gd(e,t,a){try{var n=e.stateNode;n3(n,e.type,a,t),n[zt]=t}catch(r){ke(e,e.return,r)}}function Db(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&Wn(e.type)||e.tag===4}function Yd(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||Db(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&Wn(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function Pm(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=Ku));else if(n!==4&&(n===27&&Wn(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(Pm(e,t,a),e=e.sibling;e!==null;)Pm(e,t,a),e=e.sibling}function xu(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&Wn(e.type)&&(a=e.stateNode),e=e.child,e!==null))for(xu(e,t,a),e=e.sibling;e!==null;)xu(e,t,a),e=e.sibling}function Mb(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);xt(t,n,a),t[Nt]=e,t[zt]=a}catch(s){ke(e,e.return,s)}}var an=!1,Ye=!1,Jd=!1,gg=typeof WeakSet=="function"?WeakSet:Set,dt=null;function FC(e,t){if(e=e.containerInfo,Hm=Au,e=xy(e),df(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,u=-1,c=0,d=0,f=e,m=null;t:for(;;){for(var p;f!==a||r!==0&&f.nodeType!==3||(o=i+r),f!==s||n!==0&&f.nodeType!==3||(u=i+n),f.nodeType===3&&(i+=f.nodeValue.length),(p=f.firstChild)!==null;)m=f,f=p;for(;;){if(f===e)break t;if(m===a&&++c===r&&(o=i),m===s&&++d===n&&(u=i),(p=f.nextSibling)!==null)break;f=m,m=f.parentNode}f=p}a=o===-1||u===-1?null:{start:o,end:u}}else a=null}a=a||{start:0,end:0}}else a=null;for(Km={focusedElem:e,selectionRange:a},Au=!1,dt=t;dt!==null;)if(t=dt,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,dt=e;else for(;dt!==null;){switch(t=dt,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var b=Nr(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(b,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(y){ke(a,a.return,y)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)Vm(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":Vm(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(L(163))}if(e=t.sibling,e!==null){e.return=t.return,dt=e;break}dt=t.return}}function Ob(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:En(e,a),n&4&&Ao(5,a);break;case 1:if(En(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){ke(a,a.return,i)}else{var r=Nr(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){ke(a,a.return,i)}}n&64&&Eb(a),n&512&&Zi(a,a.return);break;case 3:if(En(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{Ly(e,t)}catch(i){ke(a,a.return,i)}}break;case 27:t===null&&n&4&&Mb(a);case 26:case 5:En(e,a),t===null&&n&4&&Tb(a),n&512&&Zi(a,a.return);break;case 12:En(e,a);break;case 13:En(e,a),n&4&&Ub(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=GC.bind(null,a),c3(e,a))));break;case 22:if(n=a.memoizedState!==null||an,!n){t=t!==null&&t.memoizedState!==null||Ye,r=an;var s=Ye;an=n,(Ye=t)&&!s?An(e,a,(a.subtreeFlags&8772)!==0):En(e,a),an=r,Ye=s}break;case 30:break;default:En(e,a)}}function Lb(e){var t=e.alternate;t!==null&&(e.alternate=null,Lb(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&rf(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var Pe=null,Ft=!1;function tn(e,t,a){for(a=a.child;a!==null;)Pb(e,t,a),a=a.sibling}function Pb(e,t,a){if(Jt&&typeof Jt.onCommitFiberUnmount=="function")try{Jt.onCommitFiberUnmount(bo,a)}catch{}switch(a.tag){case 26:Ye||ja(a,t),tn(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:Ye||ja(a,t);var n=Pe,r=Ft;Wn(a.type)&&(Pe=a.stateNode,Ft=!1),tn(e,t,a),ao(a.stateNode),Pe=n,Ft=r;break;case 5:Ye||ja(a,t);case 6:if(n=Pe,r=Ft,Pe=null,tn(e,t,a),Pe=n,Ft=r,Pe!==null)if(Ft)try{(Pe.nodeType===9?Pe.body:Pe.nodeName==="HTML"?Pe.ownerDocument.body:Pe).removeChild(a.stateNode)}catch(s){ke(a,t,s)}else try{Pe.removeChild(a.stateNode)}catch(s){ke(a,t,s)}break;case 18:Pe!==null&&(Ft?(e=Pe,Tg(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),go(e)):Tg(Pe,a.stateNode));break;case 4:n=Pe,r=Ft,Pe=a.stateNode.containerInfo,Ft=!0,tn(e,t,a),Pe=n,Ft=r;break;case 0:case 11:case 14:case 15:Ye||Jn(2,a,t),Ye||Jn(4,a,t),tn(e,t,a);break;case 1:Ye||(ja(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&Ab(a,t,n)),tn(e,t,a);break;case 21:tn(e,t,a);break;case 22:Ye=(n=Ye)||a.memoizedState!==null,tn(e,t,a),Ye=n;break;default:tn(e,t,a)}}function Ub(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{go(e)}catch(a){ke(t,t.return,a)}}function qC(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new gg),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new gg),t;default:throw Error(L(435,e.tag))}}function Xd(e,t){var a=qC(e);t.forEach(function(n){var r=YC.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function Qt(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(Wn(o.type)){Pe=o.stateNode,Ft=!1;break e}break;case 5:Pe=o.stateNode,Ft=!1;break e;case 3:case 4:Pe=o.stateNode.containerInfo,Ft=!0;break e}o=o.return}if(Pe===null)throw Error(L(160));Pb(s,i,r),Pe=null,Ft=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)jb(t,e),t=t.sibling}var Na=null;function jb(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:Qt(t,e),Vt(e),n&4&&(Jn(3,e,e.return),Ao(3,e),Jn(5,e,e.return));break;case 1:Qt(t,e),Vt(e),n&512&&(Ye||a===null||ja(a,a.return)),n&64&&an&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=Na;if(Qt(t,e),Vt(e),n&512&&(Ye||a===null||ja(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[wo]||s[Nt]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),xt(s,n,a),s[Nt]=e,mt(s),n=s;break e;case"link":var i=Pg("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),xt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=Pg("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),xt(s,n,a),r.head.appendChild(s);break;default:throw Error(L(468,n))}s[Nt]=e,mt(s),n=s}e.stateNode=n}else Ug(r,e.type,e.stateNode);else e.stateNode=Lg(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?Ug(r,e.type,e.stateNode):Lg(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&Gd(e,e.memoizedProps,a.memoizedProps)}break;case 27:Qt(t,e),Vt(e),n&512&&(Ye||a===null||ja(a,a.return)),a!==null&&n&4&&Gd(e,e.memoizedProps,a.memoizedProps);break;case 5:if(Qt(t,e),Vt(e),n&512&&(Ye||a===null||ja(a,a.return)),e.flags&32){r=e.stateNode;try{ks(r,"")}catch(p){ke(e,e.return,p)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,Gd(e,r,a!==null?a.memoizedProps:r)),n&1024&&(Jd=!0);break;case 6:if(Qt(t,e),Vt(e),n&4){if(e.stateNode===null)throw Error(L(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(p){ke(e,e.return,p)}}break;case 3:if(eu=null,r=Na,Na=Ru(t.containerInfo),Qt(t,e),Na=r,Vt(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{go(t.containerInfo)}catch(p){ke(e,e.return,p)}Jd&&(Jd=!1,Fb(e));break;case 4:n=Na,Na=Ru(e.stateNode.containerInfo),Qt(t,e),Vt(e),Na=n;break;case 12:Qt(t,e),Vt(e);break;case 13:Qt(t,e),Vt(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(jf=qa()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,Xd(e,n)));break;case 22:r=e.memoizedState!==null;var u=a!==null&&a.memoizedState!==null,c=an,d=Ye;if(an=c||r,Ye=d||u,Qt(t,e),Ye=d,an=c,Vt(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||u||an||Ye||fr(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){u=a=t;try{if(s=u.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=u.stateNode;var f=u.memoizedProps.style,m=f!=null&&f.hasOwnProperty("display")?f.display:null;o.style.display=m==null||typeof m=="boolean"?"":(""+m).trim()}}catch(p){ke(u,u.return,p)}}}else if(t.tag===6){if(a===null){u=t;try{u.stateNode.nodeValue=r?"":u.memoizedProps}catch(p){ke(u,u.return,p)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,Xd(e,a))));break;case 19:Qt(t,e),Vt(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,Xd(e,n)));break;case 30:break;case 21:break;default:Qt(t,e),Vt(e)}}function Vt(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(Db(n)){a=n;break}n=n.return}if(a==null)throw Error(L(160));switch(a.tag){case 27:var r=a.stateNode,s=Yd(e);xu(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(ks(i,""),a.flags&=-33);var o=Yd(e);xu(e,o,i);break;case 3:case 4:var u=a.stateNode.containerInfo,c=Yd(e);Pm(e,c,u);break;default:throw Error(L(161))}}catch(d){ke(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function Fb(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;Fb(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function En(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)Ob(e,t.alternate,t),t=t.sibling}function fr(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:Jn(4,t,t.return),fr(t);break;case 1:ja(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&Ab(t,t.return,a),fr(t);break;case 27:ao(t.stateNode);case 26:case 5:ja(t,t.return),fr(t);break;case 22:t.memoizedState===null&&fr(t);break;case 30:fr(t);break;default:fr(t)}e=e.sibling}}function An(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:An(r,s,a),Ao(4,s);break;case 1:if(An(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){ke(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var u=r.shared.hiddenCallbacks;if(u!==null)for(r.shared.hiddenCallbacks=null,r=0;r<u.length;r++)Oy(u[r],o)}catch(c){ke(n,n.return,c)}}a&&i&64&&Eb(s),Zi(s,s.return);break;case 27:Mb(s);case 26:case 5:An(r,s,a),a&&n===null&&i&4&&Tb(s),Zi(s,s.return);break;case 12:An(r,s,a);break;case 13:An(r,s,a),a&&i&4&&Ub(r,s);break;case 22:s.memoizedState===null&&An(r,s,a),Zi(s,s.return);break;case 30:break;default:An(r,s,a)}t=t.sibling}}function Of(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&ko(a))}function Lf(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&ko(e))}function Pa(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)qb(e,t,a,n),t=t.sibling}function qb(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:Pa(e,t,a,n),r&2048&&Ao(9,t);break;case 1:Pa(e,t,a,n);break;case 3:Pa(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&ko(e)));break;case 12:if(r&2048){Pa(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(u){ke(t,t.return,u)}}else Pa(e,t,a,n);break;case 13:Pa(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?Pa(e,t,a,n):Wi(e,t):s._visibility&2?Pa(e,t,a,n):(s._visibility|=2,as(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&Of(i,t);break;case 24:Pa(e,t,a,n),r&2048&&Lf(t.alternate,t);break;default:Pa(e,t,a,n)}}function as(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,u=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:as(s,i,o,u,r),Ao(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?as(s,i,o,u,r):Wi(s,i):(d._visibility|=2,as(s,i,o,u,r)),r&&c&2048&&Of(i.alternate,i);break;case 24:as(s,i,o,u,r),r&&c&2048&&Lf(i.alternate,i);break;default:as(s,i,o,u,r)}t=t.sibling}}function Wi(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:Wi(a,n),r&2048&&Of(n.alternate,n);break;case 24:Wi(a,n),r&2048&&Lf(n.alternate,n);break;default:Wi(a,n)}t=t.sibling}}var Bi=8192;function Wr(e){if(e.subtreeFlags&Bi)for(e=e.child;e!==null;)zb(e),e=e.sibling}function zb(e){switch(e.tag){case 26:Wr(e),e.flags&Bi&&e.memoizedState!==null&&S3(Na,e.memoizedState,e.memoizedProps);break;case 5:Wr(e);break;case 3:case 4:var t=Na;Na=Ru(e.stateNode.containerInfo),Wr(e),Na=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=Bi,Bi=16777216,Wr(e),Bi=t):Wr(e));break;default:Wr(e)}}function Bb(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function Pi(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];dt=n,Hb(n,e)}Bb(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)Ib(e),e=e.sibling}function Ib(e){switch(e.tag){case 0:case 11:case 15:Pi(e),e.flags&2048&&Jn(9,e,e.return);break;case 3:Pi(e);break;case 12:Pi(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,Zl(e)):Pi(e);break;default:Pi(e)}}function Zl(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];dt=n,Hb(n,e)}Bb(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:Jn(8,t,t.return),Zl(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,Zl(t));break;default:Zl(t)}e=e.sibling}}function Hb(e,t){for(;dt!==null;){var a=dt;switch(a.tag){case 0:case 11:case 15:Jn(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:ko(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,dt=n;else e:for(a=e;dt!==null;){n=dt;var r=n.sibling,s=n.return;if(Lb(n),n===a){dt=null;break e}if(r!==null){r.return=s,dt=r;break e}dt=s}}}var zC={getCacheForType:function(e){var t=_t(it),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},BC=typeof WeakMap=="function"?WeakMap:Map,we=0,Ae=null,ue=null,de=0,$e=0,Gt=null,Fn=!1,js=!1,Pf=!1,pn=0,Ke=0,Xn=0,br=0,Uf=0,va=0,Ts=0,eo=null,qt=null,Um=!1,jf=0,$u=1/0,wu=null,Hn=null,bt=0,Kn=null,Ds=null,Ss=0,jm=0,Fm=null,Kb=null,to=0,qm=null;function Zt(){if((we&2)!==0&&de!==0)return de&-de;if(re.T!==null){var e=Rs;return e!==0?e:qf()}return ay()}function Qb(){va===0&&(va=(de&536870912)===0||pe?Zg():536870912);var e=ga.current;return e!==null&&(e.flags|=32),va}function Wt(e,t,a){(e===Ae&&($e===2||$e===9)||e.cancelPendingCommit!==null)&&(Ms(e,0),qn(e,de,va,!1)),$o(e,a),((we&2)===0||e!==Ae)&&(e===Ae&&((we&2)===0&&(br|=a),Ke===4&&qn(e,de,va,!1)),Ia(e))}function Vb(e,t,a){if((we&6)!==0)throw Error(L(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||xo(e,t),r=n?KC(e,t):Zd(e,t,!0),s=n;do{if(r===0){js&&!n&&qn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!IC(a)){r=Zd(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=eo;var u=o.current.memoizedState.isDehydrated;if(u&&(Ms(o,i).flags|=256),i=Zd(o,i,!1),i!==2){if(Pf&&!u){o.errorRecoveryDisabledLanes|=s,br|=s,r=4;break e}s=qt,qt=r,s!==null&&(qt===null?qt=s:qt.push.apply(qt,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){Ms(e,0),qn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(L(345));case 4:if((t&4194048)!==t)break;case 6:qn(n,t,va,!Fn);break e;case 2:qt=null;break;case 3:case 5:break;default:throw Error(L(329))}if((t&62914560)===t&&(r=jf+300-qa(),10<r)){if(qn(n,t,va,!Fn),Du(n,0,!0)!==0)break e;n.timeoutHandle=m0(yg.bind(null,n,a,qt,wu,Um,t,va,br,Ts,Fn,s,2,-0,0),r);break e}yg(n,a,qt,wu,Um,t,va,br,Ts,Fn,s,0,-0,0)}}break}while(!0);Ia(e)}function yg(e,t,a,n,r,s,i,o,u,c,d,f,m,p){if(e.timeoutHandle=-1,f=t.subtreeFlags,(f&8192||(f&16785408)===16785408)&&(fo={stylesheets:null,count:0,unsuspend:w3},zb(t),f=N3(),f!==null)){e.cancelPendingCommit=f(xg.bind(null,e,t,s,a,n,r,i,o,u,d,1,m,p)),qn(e,s,i,!c);return}xg(e,t,s,a,n,r,i,o,u)}function IC(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!ea(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function qn(e,t,a,n){t&=~Uf,t&=~br,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Xt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&ey(e,a,t)}function Bu(){return(we&6)===0?(To(0,!1),!1):!0}function Ff(){if(ue!==null){if($e===0)var e=ue.return;else e=ue,on=Cr=null,_f(e),ws=null,uo=0,e=ue;for(;e!==null;)Cb(e.alternate,e),e=e.return;ue=null}}function Ms(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,s3(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),Ff(),Ae=e,ue=a=un(e.current,null),de=t,$e=0,Gt=null,Fn=!1,js=xo(e,t),Pf=!1,Ts=va=Uf=br=Xn=Ke=0,qt=eo=null,Um=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Xt(n),s=1<<r;t|=e[r],n&=~s}return pn=t,Pu(),a}function Gb(e,t){ie=null,re.H=hu,t===Ro||t===ju?(t=Jv(),$e=3):t===Dy?(t=Jv(),$e=4):$e=t===wb?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Gt=t,ue===null&&(Ke=1,yu(e,ha(t,e.current)))}function Yb(){var e=re.H;return re.H=hu,e===null?hu:e}function Jb(){var e=re.A;return re.A=zC,e}function zm(){Ke=4,Fn||(de&4194048)!==de&&ga.current!==null||(js=!0),(Xn&134217727)===0&&(br&134217727)===0||Ae===null||qn(Ae,de,va,!1)}function Zd(e,t,a){var n=we;we|=2;var r=Yb(),s=Jb();(Ae!==e||de!==t)&&(wu=null,Ms(e,t)),t=!1;var i=Ke;e:do try{if($e!==0&&ue!==null){var o=ue,u=Gt;switch($e){case 8:Ff(),i=6;break e;case 3:case 2:case 9:case 6:ga.current===null&&(t=!0);var c=$e;if($e=0,Gt=null,hs(e,o,u,c),a&&js){i=0;break e}break;default:c=$e,$e=0,Gt=null,hs(e,o,u,c)}}HC(),i=Ke;break}catch(d){Gb(e,d)}while(!0);return t&&e.shellSuspendCounter++,on=Cr=null,we=n,re.H=r,re.A=s,ue===null&&(Ae=null,de=0,Pu()),i}function HC(){for(;ue!==null;)Xb(ue)}function KC(e,t){var a=we;we|=2;var n=Yb(),r=Jb();Ae!==e||de!==t?(wu=null,$u=qa()+500,Ms(e,t)):js=xo(e,t);e:do try{if($e!==0&&ue!==null){t=ue;var s=Gt;t:switch($e){case 1:$e=0,Gt=null,hs(e,t,s,1);break;case 2:case 9:if(Yv(s)){$e=0,Gt=null,bg(t);break}t=function(){$e!==2&&$e!==9||Ae!==e||($e=7),Ia(e)},s.then(t,t);break e;case 3:$e=7;break e;case 4:$e=5;break e;case 7:Yv(s)?($e=0,Gt=null,bg(t)):($e=0,Gt=null,hs(e,t,s,7));break;case 5:var i=null;switch(ue.tag){case 26:i=ue.memoizedState;case 5:case 27:var o=ue;if(!i||v0(i)){$e=0,Gt=null;var u=o.sibling;if(u!==null)ue=u;else{var c=o.return;c!==null?(ue=c,Iu(c)):ue=null}break t}}$e=0,Gt=null,hs(e,t,s,5);break;case 6:$e=0,Gt=null,hs(e,t,s,6);break;case 8:Ff(),Ke=6;break e;default:throw Error(L(462))}}QC();break}catch(d){Gb(e,d)}while(!0);return on=Cr=null,re.H=n,re.A=r,we=a,ue!==null?0:(Ae=null,de=0,Pu(),Ke)}function QC(){for(;ue!==null&&!pR();)Xb(ue)}function Xb(e){var t=Rb(e.alternate,e,pn);e.memoizedProps=e.pendingProps,t===null?Iu(e):ue=t}function bg(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=mg(a,t,t.pendingProps,t.type,void 0,de);break;case 11:t=mg(a,t,t.pendingProps,t.type.render,t.ref,de);break;case 5:_f(t);default:Cb(a,t),t=ue=Cy(t,pn),t=Rb(a,t,pn)}e.memoizedProps=e.pendingProps,t===null?Iu(e):ue=t}function hs(e,t,a,n){on=Cr=null,_f(t),ws=null,uo=0;var r=t.return;try{if(LC(e,r,t,a,de)){Ke=1,yu(e,ha(a,e.current)),ue=null;return}}catch(s){if(r!==null)throw ue=r,s;Ke=1,yu(e,ha(a,e.current)),ue=null;return}t.flags&32768?(pe||n===1?e=!0:js||(de&536870912)!==0?e=!1:(Fn=e=!0,(n===2||n===9||n===3||n===6)&&(n=ga.current,n!==null&&n.tag===13&&(n.flags|=16384))),Zb(t,e)):Iu(t)}function Iu(e){var t=e;do{if((t.flags&32768)!==0){Zb(t,Fn);return}e=t.return;var a=UC(t.alternate,t,pn);if(a!==null){ue=a;return}if(t=t.sibling,t!==null){ue=t;return}ue=t=e}while(t!==null);Ke===0&&(Ke=5)}function Zb(e,t){do{var a=jC(e.alternate,e);if(a!==null){a.flags&=32767,ue=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){ue=e;return}ue=e=a}while(e!==null);Ke=6,ue=null}function xg(e,t,a,n,r,s,i,o,u){e.cancelPendingCommit=null;do Hu();while(bt!==0);if((we&6)!==0)throw Error(L(327));if(t!==null){if(t===e.current)throw Error(L(177));if(s=t.lanes|t.childLanes,s|=mf,NR(e,a,s,i,o,u),e===Ae&&(ue=Ae=null,de=0),Ds=t,Kn=e,Ss=a,jm=s,Fm=r,Kb=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,JC(su,function(){return n0(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=re.T,re.T=null,r=he.p,he.p=2,i=we,we|=4;try{FC(e,t,a)}finally{we=i,he.p=r,re.T=n}}bt=1,Wb(),e0(),t0()}}function Wb(){if(bt===1){bt=0;var e=Kn,t=Ds,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=re.T,re.T=null;var n=he.p;he.p=2;var r=we;we|=4;try{jb(t,e);var s=Km,i=xy(e.containerInfo),o=s.focusedElem,u=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&by(o.ownerDocument.documentElement,o)){if(u!==null&&df(o)){var c=u.start,d=u.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var f=o.ownerDocument||document,m=f&&f.defaultView||window;if(m.getSelection){var p=m.getSelection(),b=o.textContent.length,y=Math.min(u.start,b),$=u.end===void 0?y:Math.min(u.end,b);!p.extend&&y>$&&(i=$,$=y,y=i);var g=zv(o,y),v=zv(o,$);if(g&&v&&(p.rangeCount!==1||p.anchorNode!==g.node||p.anchorOffset!==g.offset||p.focusNode!==v.node||p.focusOffset!==v.offset)){var x=f.createRange();x.setStart(g.node,g.offset),p.removeAllRanges(),y>$?(p.addRange(x),p.extend(v.node,v.offset)):(x.setEnd(v.node,v.offset),p.addRange(x))}}}}for(f=[],p=o;p=p.parentNode;)p.nodeType===1&&f.push({element:p,left:p.scrollLeft,top:p.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<f.length;o++){var w=f[o];w.element.scrollLeft=w.left,w.element.scrollTop=w.top}}Au=!!Hm,Km=Hm=null}finally{we=r,he.p=n,re.T=a}}e.current=t,bt=2}}function e0(){if(bt===2){bt=0;var e=Kn,t=Ds,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=re.T,re.T=null;var n=he.p;he.p=2;var r=we;we|=4;try{Ob(e,t.alternate,t)}finally{we=r,he.p=n,re.T=a}}bt=3}}function t0(){if(bt===4||bt===3){bt=0,hR();var e=Kn,t=Ds,a=Ss,n=Kb;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?bt=5:(bt=0,Ds=Kn=null,a0(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(Hn=null),nf(a),t=t.stateNode,Jt&&typeof Jt.onCommitFiberRoot=="function")try{Jt.onCommitFiberRoot(bo,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=re.T,r=he.p,he.p=2,re.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{re.T=t,he.p=r}}(Ss&3)!==0&&Hu(),Ia(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===qm?to++:(to=0,qm=e):to=0,To(0,!1)}}function a0(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,ko(t)))}function Hu(e){return Wb(),e0(),t0(),n0(e)}function n0(){if(bt!==5)return!1;var e=Kn,t=jm;jm=0;var a=nf(Ss),n=re.T,r=he.p;try{he.p=32>a?32:a,re.T=null,a=Fm,Fm=null;var s=Kn,i=Ss;if(bt=0,Ds=Kn=null,Ss=0,(we&6)!==0)throw Error(L(331));var o=we;if(we|=4,Ib(s.current),qb(s,s.current,i,a),we=o,To(0,!1),Jt&&typeof Jt.onPostCommitFiberRoot=="function")try{Jt.onPostCommitFiberRoot(bo,s)}catch{}return!0}finally{he.p=r,re.T=n,a0(e,t)}}function $g(e,t,a){t=ha(a,t),t=Mm(e.stateNode,t,2),e=In(e,t,2),e!==null&&($o(e,2),Ia(e))}function ke(e,t,a){if(e.tag===3)$g(e,e,a);else for(;t!==null;){if(t.tag===3){$g(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(Hn===null||!Hn.has(n))){e=ha(a,e),a=xb(2),n=In(t,a,2),n!==null&&($b(a,n,t,e),$o(n,2),Ia(n));break}}t=t.return}}function Wd(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new BC;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(Pf=!0,r.add(a),e=VC.bind(null,e,t,a),t.then(e,e))}function VC(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,Ae===e&&(de&a)===a&&(Ke===4||Ke===3&&(de&62914560)===de&&300>qa()-jf?(we&2)===0&&Ms(e,0):Uf|=a,Ts===de&&(Ts=0)),Ia(e)}function r0(e,t){t===0&&(t=Wg()),e=Us(e,t),e!==null&&($o(e,t),Ia(e))}function GC(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),r0(e,a)}function YC(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(L(314))}n!==null&&n.delete(t),r0(e,a)}function JC(e,t){return tf(e,t)}var Su=null,ns=null,Bm=!1,Nu=!1,em=!1,xr=0;function Ia(e){e!==ns&&e.next===null&&(ns===null?Su=ns=e:ns=ns.next=e),Nu=!0,Bm||(Bm=!0,ZC())}function To(e,t){if(!em&&Nu){em=!0;do for(var a=!1,n=Su;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Xt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,wg(n,s))}else s=de,s=Du(n,n===Ae?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||xo(n,s)||(a=!0,wg(n,s));n=n.next}while(a);em=!1}}function XC(){s0()}function s0(){Nu=Bm=!1;var e=0;xr!==0&&(r3()&&(e=xr),xr=0);for(var t=qa(),a=null,n=Su;n!==null;){var r=n.next,s=i0(n,t);s===0?(n.next=null,a===null?Su=r:a.next=r,r===null&&(ns=a)):(a=n,(e!==0||(s&3)!==0)&&(Nu=!0)),n=r}To(e,!1)}function i0(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Xt(s),o=1<<i,u=r[i];u===-1?((o&a)===0||(o&n)!==0)&&(r[i]=SR(o,t)):u<=t&&(e.expiredLanes|=o),s&=~o}if(t=Ae,a=de,a=Du(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&($e===2||$e===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&Rd(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||xo(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&Rd(n),nf(a)){case 2:case 8:a=Jg;break;case 32:a=su;break;case 268435456:a=Xg;break;default:a=su}return n=o0.bind(null,e),a=tf(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&Rd(n),e.callbackPriority=2,e.callbackNode=null,2}function o0(e,t){if(bt!==0&&bt!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(Hu(!0)&&e.callbackNode!==a)return null;var n=de;return n=Du(e,e===Ae?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:(Vb(e,n,t),i0(e,qa()),e.callbackNode!=null&&e.callbackNode===a?o0.bind(null,e):null)}function wg(e,t){if(Hu())return null;Vb(e,t,!0)}function ZC(){i3(function(){(we&6)!==0?tf(Yg,XC):s0()})}function qf(){return xr===0&&(xr=Zg()),xr}function Sg(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:Hl(""+e)}function Ng(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function WC(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=Sg((r[zt]||null).action),i=n.submitter;i&&(t=(t=i[zt]||null)?Sg(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Mu("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(xr!==0){var u=i?Ng(r,i):new FormData(r);Tm(a,{pending:!0,data:u,method:r.method,action:s},null,u)}}else typeof s=="function"&&(o.preventDefault(),u=i?Ng(r,i):new FormData(r),Tm(a,{pending:!0,data:u,method:r.method,action:s},s,u))},currentTarget:r}]})}}for(jl=0;jl<bm.length;jl++)Fl=bm[jl],_g=Fl.toLowerCase(),kg=Fl[0].toUpperCase()+Fl.slice(1),ka(_g,"on"+kg);var Fl,_g,kg,jl;ka(wy,"onAnimationEnd");ka(Sy,"onAnimationIteration");ka(Ny,"onAnimationStart");ka("dblclick","onDoubleClick");ka("focusin","onFocus");ka("focusout","onBlur");ka(yC,"onTransitionRun");ka(bC,"onTransitionStart");ka(xC,"onTransitionCancel");ka(_y,"onTransitionEnd");_s("onMouseEnter",["mouseout","mouseover"]);_s("onMouseLeave",["mouseout","mouseover"]);_s("onPointerEnter",["pointerout","pointerover"]);_s("onPointerLeave",["pointerout","pointerover"]);_r("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));_r("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));_r("onBeforeInput",["compositionend","keypress","textInput","paste"]);_r("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));_r("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));_r("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var co="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),e3=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(co));function l0(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],u=o.instance,c=o.currentTarget;if(o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){gu(d)}r.currentTarget=null,s=u}else for(i=0;i<n.length;i++){if(o=n[i],u=o.instance,c=o.currentTarget,o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){gu(d)}r.currentTarget=null,s=u}}}}function le(e,t){var a=t[mm];a===void 0&&(a=t[mm]=new Set);var n=e+"__bubble";a.has(n)||(u0(t,e,2,!1),a.add(n))}function tm(e,t,a){var n=0;t&&(n|=4),u0(a,e,n,t)}var ql="_reactListening"+Math.random().toString(36).slice(2);function zf(e){if(!e[ql]){e[ql]=!0,ny.forEach(function(a){a!=="selectionchange"&&(e3.has(a)||tm(a,!1,e),tm(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[ql]||(t[ql]=!0,tm("selectionchange",!1,t))}}function u0(e,t,a,n){switch($0(t)){case 2:var r=R3;break;case 8:r=C3;break;default:r=Kf}a=r.bind(null,t,a,e),r=void 0,!vm||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function am(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var u=i.tag;if((u===3||u===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=is(o),i===null)return;if(u=i.tag,u===5||u===6||u===26||u===27){n=s=i;continue e}o=o.parentNode}}n=n.return}dy(function(){var c=s,d=of(a),f=[];e:{var m=ky.get(e);if(m!==void 0){var p=Mu,b=e;switch(e){case"keypress":if(Ql(a)===0)break e;case"keydown":case"keyup":p=JR;break;case"focusin":b="focus",p=Ld;break;case"focusout":b="blur",p=Ld;break;case"beforeblur":case"afterblur":p=Ld;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":p=Dv;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":p=jR;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":p=WR;break;case wy:case Sy:case Ny:p=zR;break;case _y:p=tC;break;case"scroll":case"scrollend":p=PR;break;case"wheel":p=nC;break;case"copy":case"cut":case"paste":p=IR;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":p=Ov;break;case"toggle":case"beforetoggle":p=sC}var y=(t&4)!==0,$=!y&&(e==="scroll"||e==="scrollend"),g=y?m!==null?m+"Capture":null:m;y=[];for(var v=c,x;v!==null;){var w=v;if(x=w.stateNode,w=w.tag,w!==5&&w!==26&&w!==27||x===null||g===null||(w=ro(v,g),w!=null&&y.push(mo(v,w,x))),$)break;v=v.return}0<y.length&&(m=new p(m,b,null,a,d),f.push({event:m,listeners:y}))}}if((t&7)===0){e:{if(m=e==="mouseover"||e==="pointerover",p=e==="mouseout"||e==="pointerout",m&&a!==hm&&(b=a.relatedTarget||a.fromElement)&&(is(b)||b[Ls]))break e;if((p||m)&&(m=d.window===d?d:(m=d.ownerDocument)?m.defaultView||m.parentWindow:window,p?(b=a.relatedTarget||a.toElement,p=c,b=b?is(b):null,b!==null&&($=yo(b),y=b.tag,b!==$||y!==5&&y!==27&&y!==6)&&(b=null)):(p=null,b=c),p!==b)){if(y=Dv,w="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(y=Ov,w="onPointerLeave",g="onPointerEnter",v="pointer"),$=p==null?m:zi(p),x=b==null?m:zi(b),m=new y(w,v+"leave",p,a,d),m.target=$,m.relatedTarget=x,w=null,is(d)===c&&(y=new y(g,v+"enter",b,a,d),y.target=x,y.relatedTarget=$,w=y),$=w,p&&b)t:{for(y=p,g=b,v=0,x=y;x;x=es(x))v++;for(x=0,w=g;w;w=es(w))x++;for(;0<v-x;)y=es(y),v--;for(;0<x-v;)g=es(g),x--;for(;v--;){if(y===g||g!==null&&y===g.alternate)break t;y=es(y),g=es(g)}y=null}else y=null;p!==null&&Rg(f,m,p,y,!1),b!==null&&$!==null&&Rg(f,$,b,y,!0)}}e:{if(m=c?zi(c):window,p=m.nodeName&&m.nodeName.toLowerCase(),p==="select"||p==="input"&&m.type==="file")var S=jv;else if(Uv(m))if(gy)S=hC;else{S=fC;var R=mC}else p=m.nodeName,!p||p.toLowerCase()!=="input"||m.type!=="checkbox"&&m.type!=="radio"?c&&sf(c.elementType)&&(S=jv):S=pC;if(S&&(S=S(e,c))){vy(f,S,a,d);break e}R&&R(e,m,c),e==="focusout"&&c&&m.type==="number"&&c.memoizedProps.value!=null&&pm(m,"number",m.value)}switch(R=c?zi(c):window,e){case"focusin":(Uv(R)||R.contentEditable==="true")&&(us=R,gm=c,Ki=null);break;case"focusout":Ki=gm=us=null;break;case"mousedown":ym=!0;break;case"contextmenu":case"mouseup":case"dragend":ym=!1,Bv(f,a,d);break;case"selectionchange":if(gC)break;case"keydown":case"keyup":Bv(f,a,d)}var N;if(cf)e:{switch(e){case"compositionstart":var A="onCompositionStart";break e;case"compositionend":A="onCompositionEnd";break e;case"compositionupdate":A="onCompositionUpdate";break e}A=void 0}else ls?py(e,a)&&(A="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(A="onCompositionStart");A&&(fy&&a.locale!=="ko"&&(ls||A!=="onCompositionStart"?A==="onCompositionEnd"&&ls&&(N=my()):(jn=d,lf="value"in jn?jn.value:jn.textContent,ls=!0)),R=_u(c,A),0<R.length&&(A=new Mv(A,e,null,a,d),f.push({event:A,listeners:R}),N?A.data=N:(N=hy(a),N!==null&&(A.data=N)))),(N=oC?lC(e,a):uC(e,a))&&(A=_u(c,"onBeforeInput"),0<A.length&&(R=new Mv("onBeforeInput","beforeinput",null,a,d),f.push({event:R,listeners:A}),R.data=N)),WC(f,e,c,a,d)}l0(f,t)})}function mo(e,t,a){return{instance:e,listener:t,currentTarget:a}}function _u(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=ro(e,a),r!=null&&n.unshift(mo(e,r,s)),r=ro(e,t),r!=null&&n.push(mo(e,r,s))),e.tag===3)return n;e=e.return}return[]}function es(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function Rg(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,u=o.alternate,c=o.stateNode;if(o=o.tag,u!==null&&u===n)break;o!==5&&o!==26&&o!==27||c===null||(u=c,r?(c=ro(a,s),c!=null&&i.unshift(mo(a,c,u))):r||(c=ro(a,s),c!=null&&i.push(mo(a,c,u)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var t3=/\r\n?/g,a3=/\u0000|\uFFFD/g;function Cg(e){return(typeof e=="string"?e:""+e).replace(t3,`
`).replace(a3,"")}function c0(e,t){return t=Cg(t),Cg(e)===t}function Ku(){}function Ne(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||ks(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&ks(e,""+n);break;case"className":El(e,"class",n);break;case"tabIndex":El(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":El(e,a,n);break;case"style":cy(e,n,s);break;case"data":if(t!=="object"){El(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=Hl(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&Ne(e,t,"name",r.name,r,null),Ne(e,t,"formEncType",r.formEncType,r,null),Ne(e,t,"formMethod",r.formMethod,r,null),Ne(e,t,"formTarget",r.formTarget,r,null)):(Ne(e,t,"encType",r.encType,r,null),Ne(e,t,"method",r.method,r,null),Ne(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=Hl(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=Ku);break;case"onScroll":n!=null&&le("scroll",e);break;case"onScrollEnd":n!=null&&le("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(L(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(L(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=Hl(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":le("beforetoggle",e),le("toggle",e),Il(e,"popover",n);break;case"xlinkActuate":Wa(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":Wa(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":Wa(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":Wa(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":Wa(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":Wa(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":Wa(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":Wa(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":Wa(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":Il(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=OR.get(a)||a,Il(e,a,n))}}function Im(e,t,a,n,r,s){switch(a){case"style":cy(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(L(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(L(60));e.innerHTML=a}}break;case"children":typeof n=="string"?ks(e,n):(typeof n=="number"||typeof n=="bigint")&&ks(e,""+n);break;case"onScroll":n!=null&&le("scroll",e);break;case"onScrollEnd":n!=null&&le("scrollend",e);break;case"onClick":n!=null&&(e.onclick=Ku);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!ry.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[zt]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):Il(e,a,n)}}}function xt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":le("error",e),le("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(L(137,t));default:Ne(e,t,s,i,a,null)}}r&&Ne(e,t,"srcSet",a.srcSet,a,null),n&&Ne(e,t,"src",a.src,a,null);return;case"input":le("invalid",e);var o=s=i=r=null,u=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":u=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(L(137,t));break;default:Ne(e,t,n,d,a,null)}}oy(e,s,o,u,c,i,r,!1),iu(e);return;case"select":le("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:Ne(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?gs(e,!!n,t,!1):a!=null&&gs(e,!!n,a,!0);return;case"textarea":le("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(L(91));break;default:Ne(e,t,i,o,a,null)}uy(e,n,r,s),iu(e);return;case"option":for(u in a)if(a.hasOwnProperty(u)&&(n=a[u],n!=null))switch(u){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:Ne(e,t,u,n,a,null)}return;case"dialog":le("beforetoggle",e),le("toggle",e),le("cancel",e),le("close",e);break;case"iframe":case"object":le("load",e);break;case"video":case"audio":for(n=0;n<co.length;n++)le(co[n],e);break;case"image":le("error",e),le("load",e);break;case"details":le("toggle",e);break;case"embed":case"source":case"link":le("error",e),le("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(L(137,t));default:Ne(e,t,c,n,a,null)}return;default:if(sf(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&Im(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&Ne(e,t,o,n,a,null))}function n3(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,u=null,c=null,d=null;for(p in a){var f=a[p];if(a.hasOwnProperty(p)&&f!=null)switch(p){case"checked":break;case"value":break;case"defaultValue":u=f;default:n.hasOwnProperty(p)||Ne(e,t,p,null,n,f)}}for(var m in n){var p=n[m];if(f=a[m],n.hasOwnProperty(m)&&(p!=null||f!=null))switch(m){case"type":s=p;break;case"name":r=p;break;case"checked":c=p;break;case"defaultChecked":d=p;break;case"value":i=p;break;case"defaultValue":o=p;break;case"children":case"dangerouslySetInnerHTML":if(p!=null)throw Error(L(137,t));break;default:p!==f&&Ne(e,t,m,p,n,f)}}fm(e,i,o,u,c,d,s,r);return;case"select":p=i=o=m=null;for(s in a)if(u=a[s],a.hasOwnProperty(s)&&u!=null)switch(s){case"value":break;case"multiple":p=u;default:n.hasOwnProperty(s)||Ne(e,t,s,null,n,u)}for(r in n)if(s=n[r],u=a[r],n.hasOwnProperty(r)&&(s!=null||u!=null))switch(r){case"value":m=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==u&&Ne(e,t,r,s,n,u)}t=o,a=i,n=p,m!=null?gs(e,!!a,m,!1):!!n!=!!a&&(t!=null?gs(e,!!a,t,!0):gs(e,!!a,a?[]:"",!1));return;case"textarea":p=m=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:Ne(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":m=r;break;case"defaultValue":p=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(L(91));break;default:r!==s&&Ne(e,t,i,r,n,s)}ly(e,m,p);return;case"option":for(var b in a)if(m=a[b],a.hasOwnProperty(b)&&m!=null&&!n.hasOwnProperty(b))switch(b){case"selected":e.selected=!1;break;default:Ne(e,t,b,null,n,m)}for(u in n)if(m=n[u],p=a[u],n.hasOwnProperty(u)&&m!==p&&(m!=null||p!=null))switch(u){case"selected":e.selected=m&&typeof m!="function"&&typeof m!="symbol";break;default:Ne(e,t,u,m,n,p)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var y in a)m=a[y],a.hasOwnProperty(y)&&m!=null&&!n.hasOwnProperty(y)&&Ne(e,t,y,null,n,m);for(c in n)if(m=n[c],p=a[c],n.hasOwnProperty(c)&&m!==p&&(m!=null||p!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(m!=null)throw Error(L(137,t));break;default:Ne(e,t,c,m,n,p)}return;default:if(sf(t)){for(var $ in a)m=a[$],a.hasOwnProperty($)&&m!==void 0&&!n.hasOwnProperty($)&&Im(e,t,$,void 0,n,m);for(d in n)m=n[d],p=a[d],!n.hasOwnProperty(d)||m===p||m===void 0&&p===void 0||Im(e,t,d,m,n,p);return}}for(var g in a)m=a[g],a.hasOwnProperty(g)&&m!=null&&!n.hasOwnProperty(g)&&Ne(e,t,g,null,n,m);for(f in n)m=n[f],p=a[f],!n.hasOwnProperty(f)||m===p||m==null&&p==null||Ne(e,t,f,m,n,p)}var Hm=null,Km=null;function ku(e){return e.nodeType===9?e:e.ownerDocument}function Eg(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function d0(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function Qm(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var nm=null;function r3(){var e=window.event;return e&&e.type==="popstate"?e===nm?!1:(nm=e,!0):(nm=null,!1)}var m0=typeof setTimeout=="function"?setTimeout:void 0,s3=typeof clearTimeout=="function"?clearTimeout:void 0,Ag=typeof Promise=="function"?Promise:void 0,i3=typeof queueMicrotask=="function"?queueMicrotask:typeof Ag<"u"?function(e){return Ag.resolve(null).then(e).catch(o3)}:m0;function o3(e){setTimeout(function(){throw e})}function Wn(e){return e==="head"}function Tg(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&ao(i.documentElement),a&2&&ao(i.body),a&4)for(a=i.head,ao(a),i=a.firstChild;i;){var o=i.nextSibling,u=i.nodeName;i[wo]||u==="SCRIPT"||u==="STYLE"||u==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),go(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);go(t)}function Vm(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":Vm(a),rf(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function l3(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[wo])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=_a(e.nextSibling),e===null)break}return null}function u3(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=_a(e.nextSibling),e===null))return null;return e}function Gm(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function c3(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function _a(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var Ym=null;function Dg(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function f0(e,t,a){switch(t=ku(a),e){case"html":if(e=t.documentElement,!e)throw Error(L(452));return e;case"head":if(e=t.head,!e)throw Error(L(453));return e;case"body":if(e=t.body,!e)throw Error(L(454));return e;default:throw Error(L(451))}}function ao(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);rf(e)}var ya=new Map,Mg=new Set;function Ru(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var hn=he.d;he.d={f:d3,r:m3,D:f3,C:p3,L:h3,m:v3,X:y3,S:g3,M:b3};function d3(){var e=hn.f(),t=Bu();return e||t}function m3(e){var t=Ps(e);t!==null&&t.tag===5&&t.type==="form"?ib(t):hn.r(e)}var Fs=typeof document>"u"?null:document;function p0(e,t,a){var n=Fs;if(n&&typeof t=="string"&&t){var r=pa(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),Mg.has(r)||(Mg.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),xt(t,"link",e),mt(t),n.head.appendChild(t)))}}function f3(e){hn.D(e),p0("dns-prefetch",e,null)}function p3(e,t){hn.C(e,t),p0("preconnect",e,t)}function h3(e,t,a){hn.L(e,t,a);var n=Fs;if(n&&e&&t){var r='link[rel="preload"][as="'+pa(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+pa(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+pa(a.imageSizes)+'"]')):r+='[href="'+pa(e)+'"]';var s=r;switch(t){case"style":s=Os(e);break;case"script":s=qs(e)}ya.has(s)||(e=Me({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),ya.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(Do(s))||t==="script"&&n.querySelector(Mo(s))||(t=n.createElement("link"),xt(t,"link",e),mt(t),n.head.appendChild(t)))}}function v3(e,t){hn.m(e,t);var a=Fs;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+pa(n)+'"][href="'+pa(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=qs(e)}if(!ya.has(s)&&(e=Me({rel:"modulepreload",href:e},t),ya.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(Mo(s)))return}n=a.createElement("link"),xt(n,"link",e),mt(n),a.head.appendChild(n)}}}function g3(e,t,a){hn.S(e,t,a);var n=Fs;if(n&&e){var r=vs(n).hoistableStyles,s=Os(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(Do(s)))o.loading=5;else{e=Me({rel:"stylesheet",href:e,"data-precedence":t},a),(a=ya.get(s))&&Bf(e,a);var u=i=n.createElement("link");mt(u),xt(u,"link",e),u._p=new Promise(function(c,d){u.onload=c,u.onerror=d}),u.addEventListener("load",function(){o.loading|=1}),u.addEventListener("error",function(){o.loading|=2}),o.loading|=4,Wl(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function y3(e,t){hn.X(e,t);var a=Fs;if(a&&e){var n=vs(a).hoistableScripts,r=qs(e),s=n.get(r);s||(s=a.querySelector(Mo(r)),s||(e=Me({src:e,async:!0},t),(t=ya.get(r))&&If(e,t),s=a.createElement("script"),mt(s),xt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function b3(e,t){hn.M(e,t);var a=Fs;if(a&&e){var n=vs(a).hoistableScripts,r=qs(e),s=n.get(r);s||(s=a.querySelector(Mo(r)),s||(e=Me({src:e,async:!0,type:"module"},t),(t=ya.get(r))&&If(e,t),s=a.createElement("script"),mt(s),xt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function Og(e,t,a,n){var r=(r=zn.current)?Ru(r):null;if(!r)throw Error(L(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=Os(a.href),a=vs(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=Os(a.href);var s=vs(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(Do(e)))&&!s._p&&(i.instance=s,i.state.loading=5),ya.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},ya.set(e,a),s||x3(r,e,a,i.state))),t&&n===null)throw Error(L(528,""));return i}if(t&&n!==null)throw Error(L(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=qs(a),a=vs(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(L(444,e))}}function Os(e){return'href="'+pa(e)+'"'}function Do(e){return'link[rel="stylesheet"]['+e+"]"}function h0(e){return Me({},e,{"data-precedence":e.precedence,precedence:null})}function x3(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),xt(t,"link",a),mt(t),e.head.appendChild(t))}function qs(e){return'[src="'+pa(e)+'"]'}function Mo(e){return"script[async]"+e}function Lg(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+pa(a.href)+'"]');if(n)return t.instance=n,mt(n),n;var r=Me({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),mt(n),xt(n,"style",r),Wl(n,a.precedence,e),t.instance=n;case"stylesheet":r=Os(a.href);var s=e.querySelector(Do(r));if(s)return t.state.loading|=4,t.instance=s,mt(s),s;n=h0(a),(r=ya.get(r))&&Bf(n,r),s=(e.ownerDocument||e).createElement("link"),mt(s);var i=s;return i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),xt(s,"link",n),t.state.loading|=4,Wl(s,a.precedence,e),t.instance=s;case"script":return s=qs(a.src),(r=e.querySelector(Mo(s)))?(t.instance=r,mt(r),r):(n=a,(r=ya.get(s))&&(n=Me({},a),If(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),mt(r),xt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(L(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,Wl(n,a.precedence,e));return t.instance}function Wl(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function Bf(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function If(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var eu=null;function Pg(e,t,a){if(eu===null){var n=new Map,r=eu=new Map;r.set(a,n)}else r=eu,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[wo]||s[Nt]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function Ug(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function $3(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function v0(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var fo=null;function w3(){}function S3(e,t,a){if(fo===null)throw Error(L(475));var n=fo;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=Os(a.href),s=e.querySelector(Do(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=Cu.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,mt(s);return}s=e.ownerDocument||e,a=h0(a),(r=ya.get(r))&&Bf(a,r),s=s.createElement("link"),mt(s);var i=s;i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),xt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=Cu.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function N3(){if(fo===null)throw Error(L(475));var e=fo;return e.stylesheets&&e.count===0&&Jm(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&Jm(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function Cu(){if(this.count--,this.count===0){if(this.stylesheets)Jm(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var Eu=null;function Jm(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,Eu=new Map,t.forEach(_3,e),Eu=null,Cu.call(e))}function _3(e,t){if(!(t.state.loading&4)){var a=Eu.get(e);if(a)var n=a.get(null);else{a=new Map,Eu.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=Cu.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var po={$$typeof:nn,Provider:null,Consumer:null,_currentValue:pr,_currentValue2:pr,_threadCount:0};function k3(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=Cd(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=Cd(0),this.hiddenUpdates=Cd(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function g0(e,t,a,n,r,s,i,o,u,c,d,f){return e=new k3(e,t,a,i,o,u,c,f),t=1,s===!0&&(t|=24),s=Yt(3,null,null,t),e.current=s,s.stateNode=e,t=gf(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},bf(s),e}function y0(e){return e?(e=ms,e):ms}function b0(e,t,a,n,r,s){r=y0(r),n.context===null?n.context=r:n.pendingContext=r,n=Bn(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=In(e,n,t),a!==null&&(Wt(a,e,t),Gi(a,e,t))}function jg(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function Hf(e,t){jg(e,t),(e=e.alternate)&&jg(e,t)}function x0(e){if(e.tag===13){var t=Us(e,67108864);t!==null&&Wt(t,e,67108864),Hf(e,67108864)}}var Au=!0;function R3(e,t,a,n){var r=re.T;re.T=null;var s=he.p;try{he.p=2,Kf(e,t,a,n)}finally{he.p=s,re.T=r}}function C3(e,t,a,n){var r=re.T;re.T=null;var s=he.p;try{he.p=8,Kf(e,t,a,n)}finally{he.p=s,re.T=r}}function Kf(e,t,a,n){if(Au){var r=Xm(n);if(r===null)am(e,t,n,Tu,a),Fg(e,n);else if(A3(r,e,t,a,n))n.stopPropagation();else if(Fg(e,n),t&4&&-1<E3.indexOf(e)){for(;r!==null;){var s=Ps(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=dr(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var u=1<<31-Xt(i);o.entanglements[1]|=u,i&=~u}Ia(s),(we&6)===0&&($u=qa()+500,To(0,!1))}}break;case 13:o=Us(s,2),o!==null&&Wt(o,s,2),Bu(),Hf(s,2)}if(s=Xm(n),s===null&&am(e,t,n,Tu,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else am(e,t,n,null,a)}}function Xm(e){return e=of(e),Qf(e)}var Tu=null;function Qf(e){if(Tu=null,e=is(e),e!==null){var t=yo(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=Kg(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return Tu=e,null}function $0(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(vR()){case Yg:return 2;case Jg:return 8;case su:case gR:return 32;case Xg:return 268435456;default:return 32}default:return 32}}var Zm=!1,Qn=null,Vn=null,Gn=null,ho=new Map,vo=new Map,Pn=[],E3="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function Fg(e,t){switch(e){case"focusin":case"focusout":Qn=null;break;case"dragenter":case"dragleave":Vn=null;break;case"mouseover":case"mouseout":Gn=null;break;case"pointerover":case"pointerout":ho.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":vo.delete(t.pointerId)}}function Ui(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Ps(t),t!==null&&x0(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function A3(e,t,a,n,r){switch(t){case"focusin":return Qn=Ui(Qn,e,t,a,n,r),!0;case"dragenter":return Vn=Ui(Vn,e,t,a,n,r),!0;case"mouseover":return Gn=Ui(Gn,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return ho.set(s,Ui(ho.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,vo.set(s,Ui(vo.get(s)||null,e,t,a,n,r)),!0}return!1}function w0(e){var t=is(e.target);if(t!==null){var a=yo(t);if(a!==null){if(t=a.tag,t===13){if(t=Kg(a),t!==null){e.blockedOn=t,_R(e.priority,function(){if(a.tag===13){var n=Zt();n=af(n);var r=Us(a,n);r!==null&&Wt(r,a,n),Hf(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function tu(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=Xm(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);hm=n,a.target.dispatchEvent(n),hm=null}else return t=Ps(a),t!==null&&x0(t),e.blockedOn=a,!1;t.shift()}return!0}function qg(e,t,a){tu(e)&&a.delete(t)}function T3(){Zm=!1,Qn!==null&&tu(Qn)&&(Qn=null),Vn!==null&&tu(Vn)&&(Vn=null),Gn!==null&&tu(Gn)&&(Gn=null),ho.forEach(qg),vo.forEach(qg)}function zl(e,t){e.blockedOn===t&&(e.blockedOn=null,Zm||(Zm=!0,lt.unstable_scheduleCallback(lt.unstable_NormalPriority,T3)))}var Bl=null;function zg(e){Bl!==e&&(Bl=e,lt.unstable_scheduleCallback(lt.unstable_NormalPriority,function(){Bl===e&&(Bl=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(Qf(n||a)===null)continue;break}var s=Ps(a);s!==null&&(e.splice(t,3),t-=3,Tm(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function go(e){function t(u){return zl(u,e)}Qn!==null&&zl(Qn,e),Vn!==null&&zl(Vn,e),Gn!==null&&zl(Gn,e),ho.forEach(t),vo.forEach(t);for(var a=0;a<Pn.length;a++){var n=Pn[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<Pn.length&&(a=Pn[0],a.blockedOn===null);)w0(a),a.blockedOn===null&&Pn.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[zt]||null;if(typeof s=="function")i||zg(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[zt]||null)o=i.formAction;else if(Qf(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),zg(a)}}}function Vf(e){this._internalRoot=e}Qu.prototype.render=Vf.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(L(409));var a=t.current,n=Zt();b0(a,n,e,t,null,null)};Qu.prototype.unmount=Vf.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;b0(e.current,2,null,e,null,null),Bu(),t[Ls]=null}};function Qu(e){this._internalRoot=e}Qu.prototype.unstable_scheduleHydration=function(e){if(e){var t=ay();e={blockedOn:null,target:e,priority:t};for(var a=0;a<Pn.length&&t!==0&&t<Pn[a].priority;a++);Pn.splice(a,0,e),a===0&&w0(e)}};var Bg=Ig.version;if(Bg!=="19.1.0")throw Error(L(527,Bg,"19.1.0"));he.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(L(188)):(e=Object.keys(e).join(","),Error(L(268,e)));return e=uR(t),e=e!==null?Qg(e):null,e=e===null?null:e.stateNode,e};var D3={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:re,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(ji=__REACT_DEVTOOLS_GLOBAL_HOOK__,!ji.isDisabled&&ji.supportsFiber))try{bo=ji.inject(D3),Jt=ji}catch{}var ji;Vu.createRoot=function(e,t){if(!Hg(e))throw Error(L(299));var a=!1,n="",r=gb,s=yb,i=bb,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=g0(e,1,!1,null,null,a,n,r,s,i,o,null),e[Ls]=t.current,zf(e),new Vf(t)};Vu.hydrateRoot=function(e,t,a){if(!Hg(e))throw Error(L(299));var n=!1,r="",s=gb,i=yb,o=bb,u=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(u=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=g0(e,1,!0,t,a??null,n,r,s,i,o,u,c),t.context=y0(null),a=t.current,n=Zt(),n=af(n),r=Bn(n),r.callback=null,In(a,r,n),a=n,t.current.lanes=a,$o(t,a),Ia(t),e[Ls]=t.current,zf(e),new Qu(t)};Vu.version="19.1.0"});var k0=Nn((BL,_0)=>{"use strict";function N0(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(N0)}catch(e){console.error(e)}}N0(),_0.exports=S0()});var Lt=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var Bk={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},Ik=class{#t=Bk;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},Da=new Ik;function Lh(e){setTimeout(e,0)}var Pt=typeof window>"u"||"Deno"in globalThis;function Oe(){}function jh(e,t){return typeof e=="function"?e(t):e}function bi(e){return typeof e=="number"&&e>=0&&e!==1/0}function sl(e,t){return Math.max(e+(t||0)-Date.now(),0)}function Sa(e,t){return typeof e=="function"?e(t):e}function Ut(e,t){return typeof e=="function"?e(t):e}function il(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==xi(i,t.options))return!1}else if(!lr(t.queryKey,i))return!1}if(a!=="all"){let u=t.isActive();if(a==="active"&&!u||a==="inactive"&&u)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function ol(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(Ma(t.options.mutationKey)!==Ma(s))return!1}else if(!lr(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function xi(e,t){return(t?.queryKeyHashFn||Ma)(e)}function Ma(e){return JSON.stringify(e,(t,a)=>sd(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function lr(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>lr(e[a],t[a])):!1}var Hk=Object.prototype.hasOwnProperty;function $i(e,t){if(e===t)return e;let a=Ph(e)&&Ph(t);if(!a&&!(sd(e)&&sd(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},u=0;for(let c=0;c<i;c++){let d=a?c:s[c],f=e[d],m=t[d];if(f===m){o[d]=f,(a?c<r:Hk.call(e,d))&&u++;continue}if(f===null||m===null||typeof f!="object"||typeof m!="object"){o[d]=m;continue}let p=$i(f,m);o[d]=p,p===f&&u++}return r===i&&u===r?e:o}function _n(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function Ph(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function sd(e){if(!Uh(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!Uh(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function Uh(e){return Object.prototype.toString.call(e)==="[object Object]"}function Fh(e){return new Promise(t=>{Da.setTimeout(t,e)})}function wi(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?$i(e,t):t}function qh(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function zh(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var Kr=Symbol();function ll(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===Kr?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function Si(e,t){return typeof e=="function"?e(...t):!!e}var Kk=class extends Lt{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!Pt&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},Qr=new Kk;function Ni(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var Bh=Lh;function Qk(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=Bh,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(u=>{a(u)})})})};return{batch:o=>{let u;t++;try{u=o()}finally{t--,t||i()}return u},batchCalls:o=>(...u)=>{s(()=>{o(...u)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var ce=Qk();var Vk=class extends Lt{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!Pt&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},Vr=new Vk;function Gk(e){return Math.min(1e3*2**e,3e4)}function id(e){return(e??"online")==="online"?Vr.isOnline():!0}var ul=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function cl(e){let t=!1,a=0,n,r=Ni(),s=()=>r.status!=="pending",i=y=>{if(!s()){let $=new ul(y);m($),e.onCancel?.($)}},o=()=>{t=!0},u=()=>{t=!1},c=()=>Qr.isFocused()&&(e.networkMode==="always"||Vr.isOnline())&&e.canRun(),d=()=>id(e.networkMode)&&e.canRun(),f=y=>{s()||(n?.(),r.resolve(y))},m=y=>{s()||(n?.(),r.reject(y))},p=()=>new Promise(y=>{n=$=>{(s()||c())&&y($)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),b=()=>{if(s())return;let y,$=a===0?e.initialPromise:void 0;try{y=$??e.fn()}catch(g){y=Promise.reject(g)}Promise.resolve(y).then(f).catch(g=>{if(s())return;let v=e.retry??(Pt?0:3),x=e.retryDelay??Gk,w=typeof x=="function"?x(a,g):x,S=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!S){m(g);return}a++,e.onFail?.(a,g),Fh(w).then(()=>c()?void 0:p()).then(()=>{t?m(g):b()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:u,canStart:d,start:()=>(d()?b():p().then(b),r)}}var dl=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),bi(this.gcTime)&&(this.#t=Da.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(Pt?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(Da.clearTimeout(this.#t),this.#t=void 0)}};var Hh=class extends dl{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=Ih(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=Ih(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=wi(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(Oe).catch(Oe):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>Ut(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===Kr||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>Sa(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!sl(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(u=>u.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=ll(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=cl({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof ul&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,u)=>{this.#i({type:"failed",failureCount:o,error:u})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof ul){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,...od(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),ce.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function od(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:id(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function Ih(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var ur=class extends Lt{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=Ni(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),Kh(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return ld(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return ld(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof Ut(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!_n(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&Qh(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||Ut(this.options.enabled,this.#e)!==Ut(t.enabled,this.#e)||Sa(this.options.staleTime,this.#e)!==Sa(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||Ut(this.options.enabled,this.#e)!==Ut(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return Jk(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(Oe)),t}#v(){this.#x();let e=Sa(this.options.staleTime,this.#e);if(Pt||this.#n.isStale||!bi(e))return;let a=sl(this.#n.dataUpdatedAt,e)+1;this.#u=Da.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(Pt||Ut(this.options.enabled,this.#e)===!1||!bi(this.#l)||this.#l===0)&&(this.#c=Da.setInterval(()=>{(this.options.refetchIntervalInBackground||Qr.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(Da.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(Da.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,u=e!==a?e.state:this.#a,{state:c}=e,d={...c},f=!1,m;if(t._optimisticResults){let A=this.hasListeners(),M=!A&&Kh(e,t),P=A&&Qh(e,a,t,n);(M||P)&&(d={...d,...od(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:p,errorUpdatedAt:b,status:y}=d;m=d.data;let $=!1;if(t.placeholderData!==void 0&&m===void 0&&y==="pending"){let A;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(A=r.data,$=!0):A=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,A!==void 0&&(y="success",m=wi(r?.data,A,t),f=!0)}if(t.select&&m!==void 0&&!$)if(r&&m===s?.data&&t.select===this.#f)m=this.#d;else try{this.#f=t.select,m=t.select(m),m=wi(r?.data,m,t),this.#d=m,this.#i=null}catch(A){this.#i=A}this.#i&&(p=this.#i,m=this.#d,b=Date.now(),y="error");let g=d.fetchStatus==="fetching",v=y==="pending",x=y==="error",w=v&&g,S=m!==void 0,N={status:y,fetchStatus:d.fetchStatus,isPending:v,isSuccess:y==="success",isError:x,isInitialLoading:w,isLoading:w,data:m,dataUpdatedAt:d.dataUpdatedAt,error:p,errorUpdatedAt:b,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>u.dataUpdateCount||d.errorUpdateCount>u.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:x&&!S,isPaused:d.fetchStatus==="paused",isPlaceholderData:f,isRefetchError:x&&S,isStale:ud(e,t),refetch:this.refetch,promise:this.#o,isEnabled:Ut(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let A=G=>{N.status==="error"?G.reject(N.error):N.data!==void 0&&G.resolve(N.data)},M=()=>{let G=this.#o=N.promise=Ni();A(G)},P=this.#o;switch(P.status){case"pending":e.queryHash===a.queryHash&&A(P);break;case"fulfilled":(N.status==="error"||N.data!==P.value)&&M();break;case"rejected":(N.status!=="error"||N.error!==P.reason)&&M();break}}return N}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),_n(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){ce.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function Yk(e,t){return Ut(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function Kh(e,t){return Yk(e,t)||e.state.data!==void 0&&ld(e,t,t.refetchOnMount)}function ld(e,t,a){if(Ut(t.enabled,e)!==!1&&Sa(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&ud(e,t)}return!1}function Qh(e,t,a,n){return(e!==t||Ut(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&ud(e,a)}function ud(e,t){return Ut(t.enabled,e)!==!1&&e.isStaleByTime(Sa(t.staleTime,e))}function Jk(e,t){return!_n(e.getCurrentResult(),t)}function cd(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},u=0,c=async()=>{let d=!1,f=b=>{Object.defineProperty(b,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},m=ll(t.options,t.fetchOptions),p=async(b,y,$)=>{if(d)return Promise.reject();if(y==null&&b.pages.length)return Promise.resolve(b);let v=(()=>{let R={client:t.client,queryKey:t.queryKey,pageParam:y,direction:$?"backward":"forward",meta:t.options.meta};return f(R),R})(),x=await m(v),{maxPages:w}=t.options,S=$?zh:qh;return{pages:S(b.pages,x,w),pageParams:S(b.pageParams,y,w)}};if(r&&s.length){let b=r==="backward",y=b?Xk:Vh,$={pages:s,pageParams:i},g=y(n,$);o=await p($,g,b)}else{let b=e??s.length;do{let y=u===0?i[0]??n.initialPageParam:Vh(n,o);if(u>0&&y==null)break;o=await p(o,y),u++}while(u<b)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function Vh(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function Xk(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var Gh=class extends dl{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||dd(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=cl({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),ce.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function dd(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var Yh=class extends Lt{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new Gh({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=ml(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=ml(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=ml(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=ml(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){ce.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>ol(t,a))}findAll(e={}){return this.getAll().filter(t=>ol(e,t))}notify(e){ce.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return ce.batch(()=>Promise.all(e.map(t=>t.continue().catch(Oe))))}};function ml(e){return e.options.scope?.id}var md=class extends Lt{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),_n(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&Ma(t.mutationKey)!==Ma(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??dd();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){ce.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function Jh(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function Zk(e,t,a){let n=e.slice(0);return n[t]=a,n}var fd=class extends Lt{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,ce.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,f)=>d!==a[f]),u=i||o,c=u?!0:s.some((d,f)=>{let m=this.#e[f];return!m||!_n(d,m)});!u&&!c||(u&&(this.#r=r),this.#e=s,this.hasListeners()&&(u&&(Jh(a,r).forEach(d=>{d.destroy()}),Jh(r,a).forEach(d=>{d.subscribe(f=>{this.#c(d,f)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=$i(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new ur(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=Zk(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&ce.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var Xh=class extends Lt{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??xi(n,t),s=this.get(r);return s||(s=new Hh({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){ce.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>il(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>il(e,a)):t}notify(e){ce.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){ce.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){ce.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var pd=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new Xh,this.#e=e.mutationCache||new Yh,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=Qr.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=Vr.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime(Sa(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=jh(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return ce.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;ce.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return ce.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=ce.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(Oe).catch(Oe)}invalidateQueries(e,t={}){return ce.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=ce.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(Oe)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(Oe)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime(Sa(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(Oe).catch(Oe)}fetchInfiniteQuery(e){return e.behavior=cd(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(Oe).catch(Oe)}ensureInfiniteQueryData(e){return e.behavior=cd(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return Vr.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(Ma(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{lr(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(Ma(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{lr(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=xi(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===Kr&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var Oa=Ie(Ve(),1);var Gr=Ie(Ve(),1),tv=Ie(hd(),1),vd=Gr.createContext(void 0),Z=e=>{let t=Gr.useContext(vd);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},gd=({client:e,children:t})=>(Gr.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,tv.jsx)(vd.Provider,{value:e,children:t}));var pl=Ie(Ve(),1),av=pl.createContext(!1),hl=()=>pl.useContext(av),rL=av.Provider;var _i=Ie(Ve(),1),tR=Ie(hd(),1);function aR(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var nR=_i.createContext(aR()),vl=()=>_i.useContext(nR);var nv=Ie(Ve(),1);var gl=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},yl=e=>{nv.useEffect(()=>{e.clearReset()},[e])},bl=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||Si(a,[e.error,n]));var xl=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},$l=(e,t)=>e.isLoading&&e.isFetching&&!t,ki=(e,t)=>e?.suspense&&t.isPending,Yr=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function yd({queries:e,...t},a){let n=Z(a),r=hl(),s=vl(),i=Oa.useMemo(()=>e.map(y=>{let $=n.defaultQueryOptions(y);return $._optimisticResults=r?"isRestoring":"optimistic",$}),[e,n,r]);i.forEach(y=>{xl(y),gl(y,s)}),yl(s);let[o]=Oa.useState(()=>new fd(n,i,t)),[u,c,d]=o.getOptimisticResult(i,t.combine),f=!r&&t.subscribed!==!1;Oa.useSyncExternalStore(Oa.useCallback(y=>f?o.subscribe(ce.batchCalls(y)):Oe,[o,f]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),Oa.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let p=u.some((y,$)=>ki(i[$],y))?u.flatMap((y,$)=>{let g=i[$];if(g){let v=new ur(n,g);if(ki(g,y))return Yr(g,v,s);$l(y,r)&&Yr(g,v,s)}return[]}):[];if(p.length>0)throw Promise.all(p);let b=u.find((y,$)=>{let g=i[$];return g&&bl({result:y,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(b?.error)throw b.error;return c(d())}var kn=Ie(Ve(),1);function rv(e,t,a){let n=hl(),r=vl(),s=Z(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",xl(i),gl(i,r),yl(r);let o=!s.getQueryCache().get(i.queryHash),[u]=kn.useState(()=>new t(s,i)),c=u.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(kn.useSyncExternalStore(kn.useCallback(f=>{let m=d?u.subscribe(ce.batchCalls(f)):Oe;return u.updateResult(),m},[u,d]),()=>u.getCurrentResult(),()=>u.getCurrentResult()),kn.useEffect(()=>{u.setOptions(i)},[i,u]),ki(i,c))throw Yr(i,u,r);if(bl({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!Pt&&$l(c,n)&&(o?Yr(i,u,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(Oe).finally(()=>{u.updateResult()}),i.notifyOnChangeProps?c:u.trackResult(c)}function q(e,t){return rv(e,ur,t)}var Xa=Ie(Ve(),1);function B(e,t){let a=Z(t),[n]=Xa.useState(()=>new md(a,e));Xa.useEffect(()=>{n.setOptions(e)},[n,e]);let r=Xa.useSyncExternalStore(Xa.useCallback(i=>n.subscribe(ce.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=Xa.useCallback((i,o)=>{n.mutate(i,o).catch(Oe)},[n]);if(r.error&&Si(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var jk=Ie(k0());var aa=Ie(Ve(),1),W=Ie(Ve(),1),De=Ie(Ve(),1),mp=Ie(Ve(),1),Y0=Ie(Ve(),1),ve=Ie(Ve(),1),ME=Ie(Ve(),1),OE=Ie(Ve(),1),LE=Ie(Ve(),1),te=Ie(Ve(),1),cx=Ie(Ve(),1);var R0="popstate";function D0(e={}){function t(n,r){let{pathname:s,search:i,hash:o}=n.location;return Jf("",{pathname:s,search:i,hash:o},r.state&&r.state.usr||null,r.state&&r.state.key||"default")}function a(n,r){return typeof r=="string"?r:zs(r)}return O3(t,a,null,e)}function Te(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function ta(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function M3(){return Math.random().toString(36).substring(2,10)}function C0(e,t){return{usr:e.state,key:e.key,idx:t}}function Jf(e,t,a=null,n){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?Er(t):t,state:a,key:t&&t.key||n||M3()}}function zs({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function Er(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function O3(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",u=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function f(){o="POP";let $=d(),g=$==null?null:$-c;c=$,u&&u({action:o,location:y.location,delta:g})}function m($,g){o="PUSH";let v=Jf(y.location,$,g);a&&a(v,$),c=d()+1;let x=C0(v,c),w=y.createHref(v);try{i.pushState(x,"",w)}catch(S){if(S instanceof DOMException&&S.name==="DataCloneError")throw S;r.location.assign(w)}s&&u&&u({action:o,location:y.location,delta:1})}function p($,g){o="REPLACE";let v=Jf(y.location,$,g);a&&a(v,$),c=d();let x=C0(v,c),w=y.createHref(v);i.replaceState(x,"",w),s&&u&&u({action:o,location:y.location,delta:0})}function b($){return L3($)}let y={get action(){return o},get location(){return e(r,i)},listen($){if(u)throw new Error("A history only accepts one active listener");return r.addEventListener(R0,f),u=$,()=>{r.removeEventListener(R0,f),u=null}},createHref($){return t(r,$)},createURL:b,encodeLocation($){let g=b($);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:m,replace:p,go($){return i.go($)}};return y}function L3(e,t=!1){let a="http://localhost";typeof window<"u"&&(a=window.location.origin!=="null"?window.location.origin:window.location.href),Te(a,"No window.location.(origin|href) available to create URL");let n=typeof e=="string"?e:zs(e);return n=n.replace(/ $/,"%20"),!t&&n.startsWith("//")&&(n=a+n),new URL(n,a)}var P3;P3=new WeakMap;function ep(e,t,a="/"){return U3(e,t,a,!1)}function U3(e,t,a,n){let r=typeof t=="string"?Er(t):t,s=Ha(r.pathname||"/",a);if(s==null)return null;let i=M0(e);F3(i);let o=null;for(let u=0;o==null&&u<i.length;++u){let c=J3(s);o=G3(i[u],c,n)}return o}function j3(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function M0(e,t=[],a=[],n="",r=!1){let s=(i,o,u=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&u)return;Te(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let f=vn([n,d.relativePath]),m=a.concat(d);i.children&&i.children.length>0&&(Te(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${f}".`),M0(i.children,t,m,f,u)),!(i.path==null&&!i.index)&&t.push({path:f,score:Q3(f,i.index),routesMeta:m})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let u of O0(i.path))s(i,o,!0,u)}),t}function O0(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=O0(n.join("/")),o=[];return o.push(...i.map(u=>u===""?s:[s,u].join("/"))),r&&o.push(...i),o.map(u=>e.startsWith("/")&&u===""?"/":u)}function F3(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:V3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var q3=/^:[\w-]+$/,z3=3,B3=2,I3=1,H3=10,K3=-2,E0=e=>e==="*";function Q3(e,t){let a=e.split("/"),n=a.length;return a.some(E0)&&(n+=K3),t&&(n+=B3),a.filter(r=>!E0(r)).reduce((r,s)=>r+(q3.test(s)?z3:s===""?I3:H3),n)}function V3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function G3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let u=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",f=Lo({path:u.relativePath,caseSensitive:u.caseSensitive,end:c},d),m=u.route;if(!f&&c&&a&&!n[n.length-1].route.index&&(f=Lo({path:u.relativePath,caseSensitive:u.caseSensitive,end:!1},d)),!f)return null;Object.assign(r,f.params),i.push({params:r,pathname:vn([s,f.pathname]),pathnameBase:W3(vn([s,f.pathnameBase])),route:m}),f.pathnameBase!=="/"&&(s=vn([s,f.pathnameBase]))}return i}function Lo(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=Y3(e.path,e.caseSensitive,e.end),r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:f},m)=>{if(d==="*"){let b=o[m]||"";i=s.slice(0,s.length-b.length).replace(/(.)\/+$/,"$1")}let p=o[m];return f&&!p?c[d]=void 0:c[d]=(p||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function Y3(e,t=!1,a=!0){ta(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,u)=>(n.push({paramName:o,isOptional:u!=null}),u?"/?([^\\/]+)?":"/([^\\/]+)")).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function J3(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return ta(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function Ha(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}function L0(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?Er(e):e;return{pathname:a?a.startsWith("/")?a:X3(a,t):t,search:eE(n),hash:tE(r)}}function X3(e,t){let a=t.replace(/\/+$/,"").split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function Gf(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function Z3(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function tp(e){let t=Z3(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function ap(e,t,a,n=!1){let r;typeof e=="string"?r=Er(e):(r={...e},Te(!r.pathname||!r.pathname.includes("?"),Gf("?","pathname","search",r)),Te(!r.pathname||!r.pathname.includes("#"),Gf("#","pathname","hash",r)),Te(!r.search||!r.search.includes("#"),Gf("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let f=t.length-1;if(!n&&i.startsWith("..")){let m=i.split("/");for(;m[0]==="..";)m.shift(),f-=1;r.pathname=m.join("/")}o=f>=0?t[f]:"/"}let u=L0(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!u.pathname.endsWith("/")&&(c||d)&&(u.pathname+="/"),u}var vn=e=>e.join("/").replace(/\/\/+/g,"/"),W3=e=>e.replace(/\/+$/,"").replace(/^\/*/,"/"),eE=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,tE=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;function P0(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}var U0=["POST","PUT","PATCH","DELETE"],IL=new Set(U0),aE=["GET",...U0],HL=new Set(aE);var KL=Symbol("ResetLoaderData");var Ar=aa.createContext(null);Ar.displayName="DataRouter";var Bs=aa.createContext(null);Bs.displayName="DataRouterState";var QL=aa.createContext(!1);var np=aa.createContext({isTransitioning:!1});np.displayName="ViewTransition";var j0=aa.createContext(new Map);j0.displayName="Fetchers";var nE=aa.createContext(null);nE.displayName="Await";var It=aa.createContext(null);It.displayName="Navigation";var Is=aa.createContext(null);Is.displayName="Location";var na=aa.createContext({outlet:null,matches:[],isDataRoute:!1});na.displayName="Route";var rp=aa.createContext(null);rp.displayName="RouteError";var Xf=!0;function F0(e,{relative:t}={}){Te(Tr(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=W.useContext(It),{hash:r,pathname:s,search:i}=Hs(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:vn([a,s])),n.createHref({pathname:o,search:i,hash:r})}function Tr(){return W.useContext(Is)!=null}function je(){return Te(Tr(),"useLocation() may be used only in the context of a <Router> component."),W.useContext(Is).location}var q0="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function z0(e){W.useContext(It).static||W.useLayoutEffect(e)}function me(){let{isDataRoute:e}=W.useContext(na);return e?fE():rE()}function rE(){Te(Tr(),"useNavigate() may be used only in the context of a <Router> component.");let e=W.useContext(Ar),{basename:t,navigator:a}=W.useContext(It),{matches:n}=W.useContext(na),{pathname:r}=je(),s=JSON.stringify(tp(n)),i=W.useRef(!1);return z0(()=>{i.current=!0}),W.useCallback((u,c={})=>{if(ta(i.current,q0),!i.current)return;if(typeof u=="number"){a.go(u);return}let d=ap(u,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:vn([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var B0=W.createContext(null);function Ka(){return W.useContext(B0)}function I0(e){let t=W.useContext(na).outlet;return t&&W.createElement(B0.Provider,{value:e},t)}function ut(){let{matches:e}=W.useContext(na),t=e[e.length-1];return t?t.params:{}}function Hs(e,{relative:t}={}){let{matches:a}=W.useContext(na),{pathname:n}=je(),r=JSON.stringify(tp(a));return W.useMemo(()=>ap(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function H0(e,t){return K0(e,t)}function K0(e,t,a,n,r){Te(Tr(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:s}=W.useContext(It),{matches:i}=W.useContext(na),o=i[i.length-1],u=o?o.params:{},c=o?o.pathname:"/",d=o?o.pathnameBase:"/",f=o&&o.route;if(Xf){let v=f&&f.path||"";G0(c,!f||v.endsWith("*")||v.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${c}" (under <Route path="${v}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${v}"> to <Route path="${v==="/"?"*":`${v}/*`}">.`)}let m=je(),p;if(t){let v=typeof t=="string"?Er(t):t;Te(d==="/"||v.pathname?.startsWith(d),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${d}" but pathname "${v.pathname}" was given in the \`location\` prop.`),p=v}else p=m;let b=p.pathname||"/",y=b;if(d!=="/"){let v=d.replace(/^\//,"").split("/");y="/"+b.replace(/^\//,"").split("/").slice(v.length).join("/")}let $=ep(e,{pathname:y});Xf&&(ta(f||$!=null,`No routes matched location "${p.pathname}${p.search}${p.hash}" `),ta($==null||$[$.length-1].route.element!==void 0||$[$.length-1].route.Component!==void 0||$[$.length-1].route.lazy!==void 0,`Matched leaf route at location "${p.pathname}${p.search}${p.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let g=uE($&&$.map(v=>Object.assign({},v,{params:Object.assign({},u,v.params),pathname:vn([d,s.encodeLocation?s.encodeLocation(v.pathname).pathname:v.pathname]),pathnameBase:v.pathnameBase==="/"?d:vn([d,s.encodeLocation?s.encodeLocation(v.pathnameBase).pathname:v.pathnameBase])})),i,a,n,r);return t&&g?W.createElement(Is.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",...p},navigationType:"POP"}},g):g}function sE(){let e=V0(),t=P0(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return Xf&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=W.createElement(W.Fragment,null,W.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),W.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",W.createElement("code",{style:s},"ErrorBoundary")," or"," ",W.createElement("code",{style:s},"errorElement")," prop on your route."))),W.createElement(W.Fragment,null,W.createElement("h2",null,"Unexpected Application Error!"),W.createElement("h3",{style:{fontStyle:"italic"}},t),a?W.createElement("pre",{style:r},a):null,i)}var iE=W.createElement(sE,null),oE=class extends W.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.unstable_onError?this.props.unstable_onError(e,t):console.error("React Router caught the following error during render",e)}render(){return this.state.error!==void 0?W.createElement(na.Provider,{value:this.props.routeContext},W.createElement(rp.Provider,{value:this.state.error,children:this.props.component})):this.props.children}};function lE({routeContext:e,match:t,children:a}){let n=W.useContext(Ar);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),W.createElement(na.Provider,{value:e},a)}function uE(e,t=[],a=null,n=null,r=null){if(e==null){if(!a)return null;if(a.errors)e=a.matches;else if(t.length===0&&!a.initialized&&a.matches.length>0)e=a.matches;else return null}let s=e,i=a?.errors;if(i!=null){let c=s.findIndex(d=>d.route.id&&i?.[d.route.id]!==void 0);Te(c>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(i).join(",")}`),s=s.slice(0,Math.min(s.length,c+1))}let o=!1,u=-1;if(a)for(let c=0;c<s.length;c++){let d=s[c];if((d.route.HydrateFallback||d.route.hydrateFallbackElement)&&(u=c),d.route.id){let{loaderData:f,errors:m}=a,p=d.route.loader&&!f.hasOwnProperty(d.route.id)&&(!m||m[d.route.id]===void 0);if(d.route.lazy||p){o=!0,u>=0?s=s.slice(0,u+1):s=[s[0]];break}}}return s.reduceRight((c,d,f)=>{let m,p=!1,b=null,y=null;a&&(m=i&&d.route.id?i[d.route.id]:void 0,b=d.route.errorElement||iE,o&&(u<0&&f===0?(G0("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),p=!0,y=null):u===f&&(p=!0,y=d.route.hydrateFallbackElement||null)));let $=t.concat(s.slice(0,f+1)),g=()=>{let v;return m?v=b:p?v=y:d.route.Component?v=W.createElement(d.route.Component,null):d.route.element?v=d.route.element:v=c,W.createElement(lE,{match:d,routeContext:{outlet:c,matches:$,isDataRoute:a!=null},children:v})};return a&&(d.route.ErrorBoundary||d.route.errorElement||f===0)?W.createElement(oE,{location:a.location,revalidation:a.revalidation,component:b,error:m,children:g(),routeContext:{outlet:null,matches:$,isDataRoute:!0},unstable_onError:n}):g()},null)}function sp(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function cE(e){let t=W.useContext(Ar);return Te(t,sp(e)),t}function ip(e){let t=W.useContext(Bs);return Te(t,sp(e)),t}function dE(e){let t=W.useContext(na);return Te(t,sp(e)),t}function op(e){let t=dE(e),a=t.matches[t.matches.length-1];return Te(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function mE(){return op("useRouteId")}function Q0(){return ip("useNavigation").navigation}function lp(){let{matches:e,loaderData:t}=ip("useMatches");return W.useMemo(()=>e.map(a=>j3(a,t)),[e,t])}function V0(){let e=W.useContext(rp),t=ip("useRouteError"),a=op("useRouteError");return e!==void 0?e:t.errors?.[a]}function fE(){let{router:e}=cE("useNavigate"),t=op("useNavigate"),a=W.useRef(!1);return z0(()=>{a.current=!0}),W.useCallback(async(r,s={})=>{ta(a.current,q0),a.current&&(typeof r=="number"?e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var A0={};function G0(e,t,a){!t&&!A0[e]&&(A0[e]=!0,ta(!1,a))}var VL=De.memo(pE);function pE({routes:e,future:t,state:a,unstable_onError:n}){return K0(e,void 0,a,n,t)}function ct({to:e,replace:t,state:a,relative:n}){Te(Tr(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=De.useContext(It);ta(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=De.useContext(na),{pathname:i}=je(),o=me(),u=ap(e,tp(s),i,n==="path"),c=JSON.stringify(u);return De.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function up(e){return I0(e.context)}function ge(e){Te(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function cp({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1}){Te(!Tr(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let i=e.replace(/^\/*/,"/"),o=De.useMemo(()=>({basename:i,navigator:r,static:s,future:{}}),[i,r,s]);typeof a=="string"&&(a=Er(a));let{pathname:u="/",search:c="",hash:d="",state:f=null,key:m="default"}=a,p=De.useMemo(()=>{let b=Ha(u,i);return b==null?null:{location:{pathname:b,search:c,hash:d,state:f,key:m},navigationType:n}},[i,u,c,d,f,m,n]);return ta(p!=null,`<Router basename="${i}"> is not able to match the URL "${u}${c}${d}" because it does not start with the basename, so the <Router> won't render anything.`),p==null?null:De.createElement(It.Provider,{value:o},De.createElement(Is.Provider,{children:t,value:p}))}function dp({children:e,location:t}){return H0(Zu(e),t)}function Zu(e,t=[]){let a=[];return De.Children.forEach(e,(n,r)=>{if(!De.isValidElement(n))return;let s=[...t,r];if(n.type===De.Fragment){a.push.apply(a,Zu(n.props.children,s));return}Te(n.type===ge,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),Te(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=Zu(n.props.children,s)),a.push(i)}),a}var Ju="get",Xu="application/x-www-form-urlencoded";function Wu(e){return e!=null&&typeof e.tagName=="string"}function hE(e){return Wu(e)&&e.tagName.toLowerCase()==="button"}function vE(e){return Wu(e)&&e.tagName.toLowerCase()==="form"}function gE(e){return Wu(e)&&e.tagName.toLowerCase()==="input"}function yE(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function bE(e,t){return e.button===0&&(!t||t==="_self")&&!yE(e)}var Gu=null;function xE(){if(Gu===null)try{new FormData(document.createElement("form"),0),Gu=!1}catch{Gu=!0}return Gu}var $E=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function Yf(e){return e!=null&&!$E.has(e)?(ta(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${Xu}"`),null):e}function wE(e,t){let a,n,r,s,i;if(vE(e)){let o=e.getAttribute("action");n=o?Ha(o,t):null,a=e.getAttribute("method")||Ju,r=Yf(e.getAttribute("enctype"))||Xu,s=new FormData(e)}else if(hE(e)||gE(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let u=e.getAttribute("formaction")||o.getAttribute("action");if(n=u?Ha(u,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||Ju,r=Yf(e.getAttribute("formenctype"))||Yf(o.getAttribute("enctype"))||Xu,s=new FormData(o,e),!xE()){let{name:c,type:d,value:f}=e;if(d==="image"){let m=c?`${c}.`:"";s.append(`${m}x`,"0"),s.append(`${m}y`,"0")}else c&&s.append(c,f)}}else{if(Wu(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=Ju,n=null,r=Xu,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var GL=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");function fp(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var SE=Symbol("SingleFetchRedirect");function NE(e,t,a){let n=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return n.pathname==="/"?n.pathname=`_root.${a}`:t&&Ha(n.pathname,t)==="/"?n.pathname=`${t.replace(/\/$/,"")}/_root.${a}`:n.pathname=`${n.pathname.replace(/\/$/,"")}.${a}`,n}async function _E(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function kE(e){return e!=null&&typeof e.page=="string"}function RE(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function CE(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await _E(s,a);return i.links?i.links():[]}return[]}));return DE(n.flat(1).filter(RE).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function T0(e,t,a,n,r,s){let i=(u,c)=>a[c]?u.route.id!==a[c].route.id:!0,o=(u,c)=>a[c].pathname!==u.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==u.params["*"];return s==="assets"?t.filter((u,c)=>i(u,c)||o(u,c)):s==="data"?t.filter((u,c)=>{let d=n.routes[u.route.id];if(!d||!d.hasLoader)return!1;if(i(u,c)||o(u,c))return!0;if(u.route.shouldRevalidate){let f=u.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:u.params,defaultShouldRevalidate:!0});if(typeof f=="boolean")return f}return!0}):[]}function EE(e,t,{includeHydrateFallback:a}={}){return AE(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function AE(e){return[...new Set(e)]}function TE(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function DE(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!kE(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(TE(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function J0(){let e=ve.useContext(Ar);return fp(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function PE(){let e=ve.useContext(Bs);return fp(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var Po=ve.createContext(void 0);Po.displayName="FrameworkContext";function X0(){let e=ve.useContext(Po);return fp(e,"You must render this element inside a <HydratedRouter> element"),e}function UE(e,t){let a=ve.useContext(Po),[n,r]=ve.useState(!1),[s,i]=ve.useState(!1),{onFocus:o,onBlur:u,onMouseEnter:c,onMouseLeave:d,onTouchStart:f}=t,m=ve.useRef(null);ve.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let y=g=>{g.forEach(v=>{i(v.isIntersecting)})},$=new IntersectionObserver(y,{threshold:.5});return m.current&&$.observe(m.current),()=>{$.disconnect()}}},[e]),ve.useEffect(()=>{if(n){let y=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(y)}}},[n]);let p=()=>{r(!0)},b=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,m,{}]:[s,m,{onFocus:Oo(o,p),onBlur:Oo(u,b),onMouseEnter:Oo(c,p),onMouseLeave:Oo(d,b),onTouchStart:Oo(f,p)}]:[!1,m,{}]}function Oo(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function Z0({page:e,...t}){let{router:a}=J0(),n=ve.useMemo(()=>ep(a.routes,e,a.basename),[a.routes,e,a.basename]);return n?ve.createElement(FE,{page:e,matches:n,...t}):null}function jE(e){let{manifest:t,routeModules:a}=X0(),[n,r]=ve.useState([]);return ve.useEffect(()=>{let s=!1;return CE(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function FE({page:e,matches:t,...a}){let n=je(),{manifest:r,routeModules:s}=X0(),{basename:i}=J0(),{loaderData:o,matches:u}=PE(),c=ve.useMemo(()=>T0(e,t,u,r,n,"data"),[e,t,u,r,n]),d=ve.useMemo(()=>T0(e,t,u,r,n,"assets"),[e,t,u,r,n]),f=ve.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let b=new Set,y=!1;if(t.forEach(g=>{let v=r.routes[g.route.id];!v||!v.hasLoader||(!c.some(x=>x.route.id===g.route.id)&&g.route.id in o&&s[g.route.id]?.shouldRevalidate||v.hasClientLoader?y=!0:b.add(g.route.id))}),b.size===0)return[];let $=NE(e,i,"data");return y&&b.size>0&&$.searchParams.set("_routes",t.filter(g=>b.has(g.route.id)).map(g=>g.route.id).join(",")),[$.pathname+$.search]},[i,o,n,r,c,t,e,s]),m=ve.useMemo(()=>EE(d,r),[d,r]),p=jE(d);return ve.createElement(ve.Fragment,null,f.map(b=>ve.createElement("link",{key:b,rel:"prefetch",as:"fetch",href:b,...a})),m.map(b=>ve.createElement("link",{key:b,rel:"modulepreload",href:b,...a})),p.map(({key:b,link:y})=>ve.createElement("link",{key:b,nonce:a.nonce,...y})))}function qE(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var W0=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{W0&&(window.__reactRouterVersion="7.9.1")}catch{}function pp({basename:e,children:t,window:a}){let n=te.useRef();n.current==null&&(n.current=D0({window:a,v5Compat:!0}));let r=n.current,[s,i]=te.useState({action:r.action,location:r.location}),o=te.useCallback(u=>{te.startTransition(()=>i(u))},[i]);return te.useLayoutEffect(()=>r.listen(o),[r,o]),te.createElement(cp,{basename:e,children:t,location:s.location,navigationType:s.action,navigator:r})}function ex({basename:e,children:t,history:a}){let[n,r]=te.useState({action:a.action,location:a.location}),s=te.useCallback(i=>{te.startTransition(()=>r(i))},[r]);return te.useLayoutEffect(()=>a.listen(s),[a,s]),te.createElement(cp,{basename:e,children:t,location:n.location,navigationType:n.action,navigator:a})}ex.displayName="unstable_HistoryRouter";var tx=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i,Dr=te.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,state:o,target:u,to:c,preventScrollReset:d,viewTransition:f,...m},p){let{basename:b}=te.useContext(It),y=typeof c=="string"&&tx.test(c),$,g=!1;if(typeof c=="string"&&y&&($=c,W0))try{let M=new URL(window.location.href),P=c.startsWith("//")?new URL(M.protocol+c):new URL(c),G=Ha(P.pathname,b);P.origin===M.origin&&G!=null?c=G+P.search+P.hash:g=!0}catch{ta(!1,`<Link to="${c}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}let v=F0(c,{relative:r}),[x,w,S]=UE(n,m),R=sx(c,{replace:i,state:o,target:u,preventScrollReset:d,relative:r,viewTransition:f});function N(M){t&&t(M),M.defaultPrevented||R(M)}let A=te.createElement("a",{...m,...S,href:$||v,onClick:g||s?t:N,ref:qE(p,w),target:u,"data-discover":!y&&a==="render"?"true":void 0});return x&&!y?te.createElement(te.Fragment,null,A,te.createElement(Z0,{page:v})):A});Dr.displayName="Link";var Qa=te.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:u,...c},d){let f=Hs(i,{relative:c.relative}),m=je(),p=te.useContext(Bs),{navigator:b,basename:y}=te.useContext(It),$=p!=null&&ux(f)&&o===!0,g=b.encodeLocation?b.encodeLocation(f).pathname:f.pathname,v=m.pathname,x=p&&p.navigation&&p.navigation.location?p.navigation.location.pathname:null;a||(v=v.toLowerCase(),x=x?x.toLowerCase():null,g=g.toLowerCase()),x&&y&&(x=Ha(x,y)||x);let w=g!=="/"&&g.endsWith("/")?g.length-1:g.length,S=v===g||!r&&v.startsWith(g)&&v.charAt(w)==="/",R=x!=null&&(x===g||!r&&x.startsWith(g)&&x.charAt(g.length)==="/"),N={isActive:S,isPending:R,isTransitioning:$},A=S?t:void 0,M;typeof n=="function"?M=n(N):M=[n,S?"active":null,R?"pending":null,$?"transitioning":null].filter(Boolean).join(" ");let P=typeof s=="function"?s(N):s;return te.createElement(Dr,{...c,"aria-current":A,className:M,ref:d,style:P,to:i,viewTransition:o},typeof u=="function"?u(N):u)});Qa.displayName="NavLink";var ax=te.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=Ju,action:o,onSubmit:u,relative:c,preventScrollReset:d,viewTransition:f,...m},p)=>{let b=ix(),y=ox(o,{relative:c}),$=i.toLowerCase()==="get"?"get":"post",g=typeof o=="string"&&tx.test(o);return te.createElement("form",{ref:p,method:$,action:y,onSubmit:n?u:x=>{if(u&&u(x),x.defaultPrevented)return;x.preventDefault();let w=x.nativeEvent.submitter,S=w?.getAttribute("formmethod")||i;b(w||x.currentTarget,{fetcherKey:t,method:S,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:f})},...m,"data-discover":!g&&e==="render"?"true":void 0})});ax.displayName="Form";function nx({getKey:e,storageKey:t,...a}){let n=te.useContext(Po),{basename:r}=te.useContext(It),s=je(),i=lp();lx({getKey:e,storageKey:t});let o=te.useMemo(()=>{if(!n||!e)return null;let c=Wf(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let u=((c,d)=>{if(!window.history.state||!window.history.state.key){let f=Math.random().toString(32).slice(2);window.history.replaceState({key:f},"")}try{let m=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof m=="number"&&window.scrollTo(0,m)}catch(f){console.error(f),sessionStorage.removeItem(c)}}).toString();return te.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${u})(${JSON.stringify(t||Zf)}, ${JSON.stringify(o)})`}})}nx.displayName="ScrollRestoration";function rx(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function hp(e){let t=te.useContext(Ar);return Te(t,rx(e)),t}function zE(e){let t=te.useContext(Bs);return Te(t,rx(e)),t}function sx(e,{target:t,replace:a,state:n,preventScrollReset:r,relative:s,viewTransition:i}={}){let o=me(),u=je(),c=Hs(e,{relative:s});return te.useCallback(d=>{if(bE(d,t)){d.preventDefault();let f=a!==void 0?a:zs(u)===zs(c);o(e,{replace:f,state:n,preventScrollReset:r,relative:s,viewTransition:i})}},[u,o,c,a,n,t,e,r,s,i])}var BE=0,IE=()=>`__${String(++BE)}__`;function ix(){let{router:e}=hp("useSubmit"),{basename:t}=te.useContext(It),a=mE();return te.useCallback(async(n,r={})=>{let{action:s,method:i,encType:o,formData:u,body:c}=wE(n,t);if(r.navigate===!1){let d=r.fetcherKey||IE();await e.fetch(d,a,r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,flushSync:r.flushSync})}else await e.navigate(r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,replace:r.replace,state:r.state,fromRouteId:a,flushSync:r.flushSync,viewTransition:r.viewTransition})},[e,t,a])}function ox(e,{relative:t}={}){let{basename:a}=te.useContext(It),n=te.useContext(na);Te(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...Hs(e||".",{relative:t})},i=je();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),u=o.getAll("index");if(u.some(d=>d==="")){o.delete("index"),u.filter(f=>f).forEach(f=>o.append("index",f));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:vn([a,s.pathname])),zs(s)}var Zf="react-router-scroll-positions",Yu={};function Wf(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:Ha(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function lx({getKey:e,storageKey:t}={}){let{router:a}=hp("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=zE("useScrollRestoration"),{basename:s}=te.useContext(It),i=je(),o=lp(),u=Q0();te.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),HE(te.useCallback(()=>{if(u.state==="idle"){let c=Wf(i,o,s,e);Yu[c]=window.scrollY}try{sessionStorage.setItem(t||Zf,JSON.stringify(Yu))}catch(c){ta(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[u.state,e,s,i,o,t])),typeof document<"u"&&(te.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||Zf);c&&(Yu=JSON.parse(c))}catch{}},[t]),te.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(Yu,()=>window.scrollY,e?(d,f)=>Wf(d,f,s,e):void 0);return()=>c&&c()},[a,s,e]),te.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{ta(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function HE(e,t){let{capture:a}=t||{};te.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function ux(e,{relative:t}={}){let a=te.useContext(np);Te(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=hp("useViewTransitionState"),r=Hs(e,{relative:t});if(!a.isTransitioning)return!1;let s=Ha(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=Ha(a.nextLocation.pathname,n)||a.nextLocation.pathname;return Lo(r.pathname,i)!=null||Lo(r.pathname,s)!=null}var Tt=new pd({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var vp="ironclaw_token",Je="/api/webchat/v2",Mr=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function ba(){return sessionStorage.getItem(vp)||""}function Ks(e){e?sessionStorage.setItem(vp,e):sessionStorage.removeItem(vp)}function ec(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function mx(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function dx(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function fx({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=dx(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=dx(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function H(e,t={}){let a=ba(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await mx(r);throw new Mr(fx({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function tc(){return H(`${Je}/session`)}function ac({clientActionId:e,requestedThreadId:t,projectId:a}={}){let n={client_action_id:e||ec()};return t&&(n.requested_thread_id=t),a&&(n.project_id=a),H(`${Je}/threads`,{method:"POST",body:JSON.stringify(n)})}function px({limit:e,cursor:t}={}){let a=new URL(`${Je}/threads`,window.location.origin);return e!=null&&a.searchParams.set("limit",String(e)),t&&a.searchParams.set("cursor",t),H(a.pathname+a.search)}function hx({threadId:e}={}){return e?H(`${Je}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function gp(e){return`${Je}/threads/${encodeURIComponent(e)}/files`}function vx({threadId:e,path:t}={}){if(!e)return Promise.reject(new Error("threadId is required"));let a=new URL(gp(e),window.location.origin);return t&&a.searchParams.set("path",t),H(a.pathname+a.search)}function gx({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${gp(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),H(a.pathname+a.search)}function nc({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${gp(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function yx({limit:e,runLimit:t,includeCompleted:a}={}){let n=new URLSearchParams;e!=null&&n.set("limit",String(e)),t!=null&&n.set("run_limit",String(t)),a===!0&&n.set("include_completed","true");let r=n.toString();return H(`${Je}/automations${r?`?${r}`:""}`)}function bx({automationId:e}={}){return e?H(`${Je}/automations/${encodeURIComponent(e)}/pause`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function xx({automationId:e}={}){return e?H(`${Je}/automations/${encodeURIComponent(e)}/resume`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function $x({automationId:e}={}){return e?H(`${Je}/automations/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("automationId is required"))}var wx=`${Je}/projects`;function KE(e){return`${wx}/${encodeURIComponent(e)}`}function Sx({limit:e}={}){let t=new URL(wx,window.location.origin);return e!=null&&t.searchParams.set("limit",String(e)),H(t.pathname+t.search)}function Nx({projectId:e}={}){return e?H(KE(e)):Promise.reject(new Error("projectId is required"))}function _x(){return H(`${Je}/outbound/preferences`)}function kx(){return H(`${Je}/outbound/targets`)}function Rx({finalReplyTargetId:e}={}){return H(`${Je}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function Cx({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:u,source:c,tail:d,follow:f}={}){let m=new URL(`${Je}/operator/logs`,window.location.origin);return e!=null&&m.searchParams.set("limit",String(e)),t&&m.searchParams.set("cursor",t),a&&m.searchParams.set("level",a),n&&m.searchParams.set("target",n),r&&m.searchParams.set("thread_id",r),s&&m.searchParams.set("run_id",s),i&&m.searchParams.set("turn_id",i),o&&m.searchParams.set("tool_call_id",o),u&&m.searchParams.set("tool_name",u),c&&m.searchParams.set("source",c),d&&m.searchParams.set("tail","true"),f&&m.searchParams.set("follow","true"),H(m.pathname+m.search)}function Ex({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||ec(),content:t};return a.length>0&&(r.attachments=a),H(`${Je}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function Ax({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${Je}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),H(n.pathname+n.search)}function Tx({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${Je}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function Ra(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new Mr("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=ba(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await mx(r);throw new Mr(fx({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function yp(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function rc(e){return yp(await Ra(e))}function Dx({threadId:e,afterCursor:t}={}){let a=new URL(`${Je}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=ba();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function Mx({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||ec()};return a&&(r.reason=a),H(`${Je}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function bp({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let u={client_action_id:i||ec(),resolution:n};return r!=null&&(u.always=r),s&&(u.credential_ref=s),H(`${Je}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(u)})}function Ox({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return H("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function Lx(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),H(`${Je}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function Qs(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function Px(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function Ux(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new Mr("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new Mr("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function jx(){let e=ba();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var sc="anon",Fx=sc;function qx(e){Fx=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:sc}function kt(){return Fx}var zx="ironclaw:v2-thread-pins:",xp=new Set,gn=new Set,$p=null;function wp(){return`${zx}${kt()}`}function QE(){try{let e=window.localStorage.getItem(wp());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function VE(){try{gn.size===0?window.localStorage.removeItem(wp()):window.localStorage.setItem(wp(),JSON.stringify([...gn]))}catch{}}function Bx(){let e=kt();if(e!==$p){gn.clear();for(let t of QE())gn.add(t);$p=e}}function Ix(){return new Set(gn)}function Hx(){let e=Ix();for(let t of xp)try{t(e)}catch{}}function Kx(e){e&&(Bx(),gn.has(e)?gn.delete(e):gn.add(e),VE(),Hx())}function Qx(){return Bx(),Ix()}function Vx(e){return xp.add(e),()=>{xp.delete(e)}}function Gx(){gn.clear(),$p=kt();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(zx)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}Hx()}var GE=0,Or={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function Sp(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function Yx(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":YE(t)?"text":"download"}function YE(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function Uo(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function JE(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function XE(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function ZE(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function Jx(e,{limits:t,existing:a=[],t:n}){let r=t||Or,s=[],i=[],o=a.length,u=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!JE(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:Uo(r.maxFileBytes)}));continue}if(u+c.size>r.maxTotalBytes){let y=n("chat.attachmentTotalTooLarge",{max:Uo(r.maxTotalBytes)});i.includes(y)||i.push(y);continue}let d;try{d=await XE(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:f,base64:m}=ZE(d,c.type),p=f||"application/octet-stream",b=Sp(p);s.push({id:`staged-${GE++}`,filename:c.name||"attachment",mimeType:p,kind:b,sizeBytes:c.size,sizeLabel:Uo(c.size),dataBase64:m,previewUrl:b==="image"?d:null}),o+=1,u+=c.size}return{staged:s,errors:i}}function Xx(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function Zx(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}function WE(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||Sp(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?Tx({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?Uo(n.size_bytes):"",preview_url:null,fetch_url:s}})}function e$(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let c=n4(s);if(!c)continue;let d=`tool-${c.invocationId}`;if(n.has(d))continue;n.add(d),r.push({id:d,role:"tool_activity",...c,timestamp:Wx(s)||c.updatedAt||null,sequence:s.sequence,activityOrder:c.activityOrder,activityOrderSource:c.activityOrderSource,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=a4(s),u=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy");r.push({id:i,role:o,content:s.content||"",attachments:WE(s,a),timestamp:Wx(s),kind:s.kind,status:u?"error":s.status,...u&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:t4(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=e4(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function e4(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function t4(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function a4(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function Wx(e){return e.received_at||e.created_at||null}function n4(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:Np(t)}function Np(e){let t=e.status==="failed"||e.status==="killed",a=a$(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Lr(e.title||e.capability_id)||"tool",toolStatus:t$(e.status),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(e.output_summary||e.output_preview||e.result_ref)||null,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null,activityOrder:a,activityOrderSource:Number.isFinite(a)?"projection":null}}function _p(e){let t=a$(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Lr(e.capability_id)||"tool",toolStatus:t$(e.status),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:null,toolError:e.error_kind||null,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null,activityOrder:t,activityOrderSource:Number.isFinite(t)?"projection":null}}function Vs(e){return e==="success"||e==="error"}function Lr(e){let t=typeof e=="string"?e.trim():"";if(!t)return"";let a=t.split(".");return a[a.length-1]||t}function t$(e){switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}function a$(e){let t=Number(e);return Number.isFinite(t)?t:null}var r4=50,yn=new Map,s4=30;function kp(e,t){for(yn.delete(e),yn.set(e,t);yn.size>s4;){let a=yn.keys().next().value;yn.delete(a)}}function ic(e){return`${kt()}:${e}`}function r$(){yn.clear()}function s$(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?yn.get(ic(e)):null,[s,i]=h.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),o=h.default.useRef(new Set),u=h.default.useRef(e);u.current=e;let c=h.default.useCallback(async(d,f={})=>{let{preserveClientOnly:m=!1}=f;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(o.current.has(e))return;o.current.add(e);let p=kt(),b=ic(e);i(y=>({...y,isLoading:!0}));try{let y=await Ax({threadId:e,limit:r4,cursor:d});if(kt()!==p)return;let $=d?[]:a?.()||[],g=e$(y.messages||[],$,e),v=y.next_cursor||null;if(d||n?.([]),!d){let x=yn.get(b)?.messages||[],w=n$(g,x,{preserveClientOnly:m});kp(b,{messages:w,nextCursor:v})}i(x=>{if(u.current!==e)return x;let w;return d?w=i4(g,x.messages):w=n$(g,x.messages,{preserveClientOnly:m}),kp(b,{messages:w,nextCursor:v}),{messages:w,nextCursor:v,isLoading:!1,loadError:null}})}catch(y){if(console.error("Failed to load timeline:",y),kt()!==p)return;i($=>u.current===e?{...$,isLoading:!1,loadError:"Failed to load conversation history."}:$)}finally{o.current.delete(e)}},[e,a,n]);return h.default.useEffect(()=>{let d=e?yn.get(ic(e)):null;i({messages:d?.messages||[],nextCursor:d?.nextCursor||null,isLoading:!!e&&!d,loadError:null}),e&&c()},[e,c]),{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:c,setMessages:d=>i(f=>{let m=typeof d=="function"?d(f.messages):d;return e&&kp(ic(e),{messages:m,nextCursor:f.nextCursor}),{...f,messages:m}})}}function i4(e,t){let a=new Set(t.map(n=>n?.id).filter(Boolean));return[...e.filter(n=>!a.has(n?.id)),...t]}function n$(e,t,a={}){let{preserveClientOnly:n=!1}=a,r=new Set(e.map(i=>i?.id).filter(Boolean)),s=t.filter(i=>!i||typeof i.id!="string"||r.has(i.id)?!1:o4(i)?!0:n&&i.id.startsWith("err-"));return s.length>0?[...e,...s]:e}function o4(e){return e?.role==="tool_activity"||e?.role==="thinking"}var Fo="__new__",i$="ironclaw:v2-draft:";function Gs(e){return`${i$}${kt()}:${e||Fo}`}function Rp(e){try{return window.localStorage.getItem(Gs(e))||""}catch{return""}}function Cp(e,t){try{t?window.localStorage.setItem(Gs(e),t):window.localStorage.removeItem(Gs(e))}catch{}}function o$(e){Cp(e,"")}var jo=new Map;function Ep(e){return jo.get(Gs(e))||[]}function l$(e,t){let a=Gs(e);t&&t.length>0?jo.set(a,t):jo.delete(a)}function u$(e){jo.delete(Gs(e))}function c$(){jo.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(i$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function l4(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function u4(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function c4(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=l4(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?u4(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),ba()?"":(Ks(n),n)}function d4(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var m4={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function f4(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),m4[t]||"Could not complete sign-in. Please try again."):""}function d$(){let[e,t]=h.default.useState(()=>c4()||ba()),[a,n]=h.default.useState(()=>f4()),[r]=h.default.useState(()=>d4()),[s,i]=h.default.useState(null),[o,u]=h.default.useState(()=>!!(r&&!ba())),[c,d]=h.default.useState(()=>!!ba());h.default.useEffect(()=>{if(!r||ba()){u(!1);return}let b=!1;return Ux(r).then(y=>{b||(Ks(y),d(!0),t(y),i(null),n(""),u(!1),Tt.clear())}).catch(()=>{b||(n("Could not complete sign-in. Please try again."),u(!1))}),()=>{b=!0}},[r]),h.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let b=!1;return d(!0),tc().then(y=>{b||(i(y),d(!1))}).catch(y=>{b||(i(null),d(!1),(y?.status===401||y?.status===403)&&(Ks(""),t(""),n("Your session expired. Please sign in again."),Tt.clear()))}),()=>{b=!0}},[e,o]),qx(s);let f=h.default.useRef(null);h.default.useEffect(()=>{let b=kt();f.current&&f.current!==sc&&f.current!==b&&(r$(),c$(),Gx()),f.current=b},[s]);let m=h.default.useCallback(b=>{Ks(b),d(!!b),t(b),i(null),n(""),Tt.clear()},[]),p=h.default.useCallback(()=>{jx().catch(()=>{}),Ks(""),d(!1),t(""),i(null),n(""),Tt.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,rebornProjectsEnabled:!!s?.features?.reborn_projects,signIn:m,signOut:p}}var Pr="/chat",qo=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace"},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var p4=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],h4=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],v4=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],oc={settings:p4,extensions:h4,admin:v4};var m$="ironclaw:v2-theme";function g4(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(m$);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function lc(){let[e,t]=h.default.useState(g4);h.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(m$,e)}catch{}},[e]);let a=h.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function f$(e){return q({enabled:!!e,queryKey:["gateway-status",e],queryFn:Qs,refetchInterval:3e4})}function p$(){return Promise.resolve({settings:{},todo:!0})}function h$(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 settings endpoint"})}function v$(e){return Promise.resolve({success:!1,message:"TODO: requires v2 settings endpoint"})}function uc(){return H("/api/webchat/v2/llm/providers")}function g$(e){return H("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function y$(e){return H(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function zo(e){return H("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function b$(e){return H("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function x$(e){return H("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function $$(e){return H("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function w$(e){return H("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function S$(){return H("/api/webchat/v2/llm/codex/login",{method:"POST"})}function N$(){return Promise.resolve({tools:[],todo:!0})}function _$(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 tools endpoint"})}function k$(){return H("/api/webchat/v2/extensions")}function R$(){return H("/api/webchat/v2/extensions/registry")}function C$(){return H("/api/webchat/v2/skills")}function E$(e){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function A$(e){return H("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function T$(e,t){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function D$(e){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function M$(e,t){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}/auto-activate`,{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:t})})}function O$(e){return H("/api/webchat/v2/skills/auto-activate-learned",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function L$(e){return H("/api/webchat/v2/skills/learning-enabled",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function P$(e){return H("/api/webchat/v2/skills/require-review",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function U$(){return H("/api/webchat/v2/skills/pending")}function j$(e){return H("/api/webchat/v2/skills/pending/approve",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({name:e})})}function F$(e){return H("/api/webchat/v2/skills/pending/discard",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({name:e})})}function q$(){return H("/api/webchat/v2/traces/credit")}function z$(e){return H(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function B$(){return Promise.resolve({users:[],todo:!0})}function I$(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function H$(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var Ap="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",Tp=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function Bo(e){return Tp.find(t=>t.value===e)?.label||e}function Ys(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function K$(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function cc(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function Q$(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function Ur(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Ap||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?Ys(e,t).trim().length>0:!0:!1}function y4(e,t,a){return e.id===a?"active":Ur(e,t)?"ready":"setup"}function V$(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=y4(r,t,a);n[s]&&n[s].push(r)}return n}function dc(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Ap||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!Ys(e,t).trim()?"base_url":"ok"}function Dp(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===Ap&&(i.api_key=void 0),i}function G$(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function Y$(e){return/^[a-z0-9_-]+$/.test(e)}function J$(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var b4=Object.freeze({});function Js({settings:e,gatewayStatus:t,enabled:a=!0}){let n=Z(),r=q({queryKey:["llm-providers"],queryFn:uc,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=b4,u=(s.providers||[]).map(w=>({...w,name:w.description,has_api_key:w.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,f=d||"nearai",m=s.active?.model||t?.llm_model||"",p=u.filter(w=>w.builtin),b=u.filter(w=>!w.builtin),y=[...u].sort((w,S)=>w.id===d?-1:S.id===d?1:(w.name||w.id).localeCompare(S.name||S.id)),$=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=B({mutationFn:async w=>{if(!Ur(w,o)){let R=dc(w,o);throw new Error(R==="base_url"?"base_url":"api_key")}let S=cc(w,o);if(!S)throw new Error("model");return await zo({provider_id:w.id,model:S}),w},onSuccess:$}),v=B({mutationFn:async({provider:w,form:S,apiKey:R,editingProvider:N})=>{let A=!!w?.builtin,P={id:(A?w.id:S.id.trim()).trim(),name:A?w.name||w.id:S.name.trim(),adapter:A?w.adapter:S.adapter,base_url:S.baseUrl.trim()||w?.base_url||"",default_model:S.model.trim()||void 0};return R.trim()&&(P.api_key=R.trim()),(N||w)?.id===f&&P.default_model&&(P.set_active=!0,P.model=P.default_model),await g$(P),P},onSuccess:$}),x=B({mutationFn:async w=>(await y$(w.id),w),onSuccess:$});return{providers:y,builtinProviders:p,customProviders:b,builtinOverrides:o,activeProviderId:d,selectedModel:m,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:w=>g.mutateAsync(w),saveCustomProvider:w=>v.mutateAsync(w),saveBuiltinProvider:w=>v.mutateAsync(w),deleteCustomProvider:w=>x.mutateAsync(w),testConnection:b$,listModels:x$,isBusy:g.isPending||v.isPending||x.isPending}}function X$({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}function Z$({onNewChat:e}={}){let t=me(),[a,n]=h.default.useState(!1),r=h.default.useCallback(()=>n(!1),[]),s=h.default.useCallback(()=>n(u=>!u),[]),i=h.default.useCallback(async()=>{let u=await e?.(),c=typeof u=="string"&&u.length>0?u:null;t(c?`/chat/${c}`:"/chat"),r()},[t,r,e]),o=h.default.useCallback(u=>{t(`/chat/${u}`),r()},[t,r]);return{open:a,close:r,toggle:s,newChat:i,selectThread:o}}var Mp=new Set,x4=0;function Xs(e,t={}){let a={id:++x4,message:e,tone:t.tone||"info",duration:t.duration??2600};return Mp.forEach(n=>n(a)),a.id}function W$(e){return Mp.add(e),()=>Mp.delete(e)}function $4(e){return e?.status===409&&e?.payload?.kind==="busy"}function ew(e,t){return $4(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function tw(){let e=q({queryKey:["threads"],queryFn:()=>px({})}),[t,a]=h.default.useState(null),[n,r]=h.default.useState(!1),s=h.default.useRef(new Map),i=h.default.useCallback(async c=>{let d=c||"__global__",f=s.current.get(d);if(f)return f;r(!0);let m=(async()=>{try{let p=await ac(c?{projectId:c}:void 0);Tt.invalidateQueries({queryKey:["threads"]});let b=p?.thread?.thread_id;return b&&a(b),b}finally{s.current.delete(d),r(s.current.size>0)}})();return s.current.set(d,m),m},[]),o=h.default.useCallback(async c=>{await hx({threadId:c}),t===c&&a(null),Tt.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:h.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var aw={attach:l`<path
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
    />`,arrowDown:l`<path d="M12 5v14" /><path d="m6 13 6 6 6-6" />`,retry:l`<path d="M3.5 12a8.5 8.5 0 1 1 2.6 6.1" /><path d="M3.2 18.5v-5h5" />`};function D({name:e,className:t="",strokeWidth:a=1.7}){return l`
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
      ${aw[e]||aw.spark}
    </svg>
  `}function V(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=V(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function nw(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function w4(e){return nw(e).trim().charAt(0).toUpperCase()||"I"}function S4(){let[e,t]=h.default.useState(!1),a=h.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function rw({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=k(),s=S4(),i=nw(a),o=a?.email||a?.role||r("common.gatewaySession");return l`
    <div
      className="relative flex items-center gap-2 border-t border-[var(--v2-panel-border)] px-3 py-3"
    >
      ${s.open&&l`
        <div
          className=${V("absolute bottom-full left-3 right-3 mb-2 rounded-[10px] border p-3 shadow-lg","border-[var(--v2-panel-border)] bg-[var(--v2-surface)]")}
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
            />`:l`<span className="place-self-center">${w4(a)}</span>`}
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
  `}var sw={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",settings:"settings",admin:"shield"},N4=qo.filter(e=>e.id!=="chat"&&!e.hidden);function _4({route:e,label:t,onNavigate:a}){return l`
    <${Qa}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${D} name=${sw[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function k4({route:e,label:t,subRoutes:a,onNavigate:n}){let r=k(),s=je(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return l`
    <div className="flex flex-col">
      <${Qa}
        to=${o}
        onClick=${n}
        className=${()=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${D}
          name=${sw[e.id]||"bolt"}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1 truncate">${t}</span>
        <${D}
          name="chevron"
          className=${V("h-3.5 w-3.5 shrink-0 transition-transform duration-150",i&&"rotate-180")}
        />
      <//>

      ${i&&l`
        <div className="mt-0.5 flex flex-col gap-0.5 pl-3">
          ${a.map(u=>l`
              <${Qa}
                key=${u.id}
                to=${e.path+"/"+u.id}
                onClick=${n}
                className=${({isActive:c})=>V("flex items-center gap-2.5 rounded-[8px] py-1.5 pl-7 pr-3 text-[12px] font-medium",c?"text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
              >
                <${D} name=${u.icon} className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate">${r(u.labelKey)}</span>
              <//>
            `)}
        </div>
      `}
    </div>
  `}function iw({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=k(),s=h.default.useMemo(()=>N4.filter(i=>a||i.id!=="admin"),[a]);return l`
    <div className="flex flex-col px-3 py-2">
      <button
        onClick=${e}
        disabled=${t}
        className=${V("flex items-center gap-2.5 rounded-[10px] px-3 py-2","border border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]","bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]","hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)] disabled:opacity-50")}
      >
        <${D} name="plus" className="h-4 w-4 shrink-0" />
        <span className="text-[13px] font-medium">
          ${r(t?"chat.creating":"chat.newThread")}
        </span>
      </button>

      <nav className="mt-2 flex flex-col gap-1">
        ${s.map(i=>{let o=(oc[i.id]||[]).filter(u=>a||!(i.id==="settings"&&["users","inference"].includes(u.id)));return o.length>0?l`
              <${k4}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:l`
            <${_4}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var bn=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),Io=new Set([bn.NEEDS_ATTENTION,bn.FAILED]),Op="ironclaw:v2-thread-attention",Lp=new Set,Zs=new Map;function R4(){try{let e=window.localStorage.getItem(Op);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&Io.has(a[1])):[]}catch{return[]}}function ow(){let e=[];for(let[t,a]of Zs)Io.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(Op):window.localStorage.setItem(Op,JSON.stringify(e))}catch{}}for(let[e,t]of R4())Zs.set(e,t);function uw(){return new Map(Zs)}function lw(){let e=uw();for(let t of Lp)try{t(e)}catch{}}function mc(e,t){if(!e)return;let a=Zs.get(e);if(t==null){if(!Zs.delete(e))return;Io.has(a)&&ow(),lw();return}a!==t&&(Zs.set(e,t),(Io.has(t)||Io.has(a))&&ow(),lw())}function cw(e){mc(e,null)}function C4(){return uw()}function E4(e){return Lp.add(e),()=>{Lp.delete(e)}}function dw(){let[e,t]=h.default.useState(C4);return h.default.useEffect(()=>E4(t),[]),e}function fc(e){return e.updated_at||e.created_at||null}function Pp(e,t){let a=fc(e)||"",n=fc(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function mw(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function fw(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function A4(){let[e,t]=h.default.useState(Qx);return h.default.useEffect(()=>Vx(t),[]),e}var T4=Object.freeze({[bn.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-transparent"},[bn.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[bn.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function D4(e){return e&&T4[e]||null}function M4({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=k(),o=fc(e),u=mw(o),c=fw(o),d=h.default.useCallback(m=>{m.preventDefault(),m.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(p=>{window.alert(p?.message||"Unable to delete chat")})},[s,e.id]),f=h.default.useCallback(m=>{m.preventDefault(),m.stopPropagation(),Kx(e.id)},[e.id]);return l`
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
          ${n&&l`<span
            aria-label=${n.label}
            className=${V("h-1.5 w-1.5 shrink-0 rounded-full",n.dotClass)}
          />`}
        </div>
        ${(n||u)&&l`<span
          className=${V("block truncate text-[11px]",n?n.textClass:"text-[var(--v2-text-faint)]")}
        >
          ${n?n.label:u}
        </span>`}
      </button>
      <button
        type="button"
        onClick=${f}
        title=${i(a?"common.unpin":"common.pin")}
        aria-label=${i(a?"common.unpin":"common.pin")}
        aria-pressed=${a?"true":"false"}
        className=${V("my-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] transition",a?"text-[var(--v2-accent-text)]":"opacity-0 text-[var(--v2-text-faint)] group-hover:opacity-100 focus:opacity-100","hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-accent-text)]")}
      >
        <${D} name="pin" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>
      ${s&&l`<button
        type="button"
        onClick=${d}
        title=${i("common.deleteChat")}
        aria-label=${i("common.deleteChat")}
        className=${V("my-1 mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px]","opacity-0 transition group-hover:opacity-100 focus:opacity-100","text-[var(--v2-text-faint)] hover:bg-[var(--v2-danger-soft)] hover:text-[var(--v2-danger-text)]")}
      >
        <${D} name="trash" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>`}
    </div>
  `}function pw({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:l`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>l`
          <${M4}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${D4(n.get(o.id))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function hw({threads:e,activeThreadId:t,rebornProjectsEnabled:a=!1,onSelect:n,onDelete:r,onNavigate:s}){let[i,o]=h.default.useState(!1),[u,c]=h.default.useState(""),d=dw(),f=A4(),m=k(),{pinned:p,recent:b,totalMatches:y}=h.default.useMemo(()=>{let $=u.trim().toLowerCase(),g=$?e.filter(w=>(w.title||w.id||"").toLowerCase().includes($)):e,v=[],x=[];for(let w of g)f.has(w.id)?v.push(w):x.push(w);return v.sort(Pp),x.sort(Pp),{pinned:v,recent:x,totalMatches:v.length+x.length}},[e,u,f]);return l`
    <div className="flex min-h-0 flex-1 flex-col px-2">
      <button
        onClick=${()=>o($=>!$)}
        className="flex w-full items-center gap-1 rounded-[6px] px-2 py-1.5 hover:bg-[var(--v2-surface-muted)]"
      >
        <span
          className="flex-1 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]"
        >
          ${m("chat.conversations")}
        </span>
        <${D}
          name="chevron"
          className=${V("h-3.5 w-3.5 text-[var(--v2-text-faint)]",i?"-rotate-90":"")}
          strokeWidth=${2.2}
        />
      </button>

      ${!i&&l`
        ${e.length>0&&l`<div className="relative mb-1 mt-1 px-1">
          <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--v2-text-faint)]">
            <${D} name="search" className="h-3.5 w-3.5" />
          </span>
          <input
            type="text"
            value=${u}
            onInput=${$=>c($.currentTarget.value)}
            placeholder=${m("common.searchChats")}
            className="h-8 w-full rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] pl-8 pr-2 text-[12px] text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]"
          />
        </div>`}
        ${a&&l`<div className="mb-1 px-1">
          <${Qa}
            to="/projects"
            onClick=${s}
            className=${({isActive:$})=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",$?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
          >
            <${D} name="folder" className="h-4 w-4 shrink-0" />
            <span className="min-w-0 truncate">${m("nav.projects")}</span>
          <//>
        </div>`}
        <div
          className="mt-1 flex flex-col gap-2 overflow-y-auto [scrollbar-width:thin]"
        >
          ${e.length===0&&l`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            ${m("chat.noConversations")}
          </div>`}
          ${e.length>0&&y===0&&l`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            ${m("common.noChatsMatch").replace("{query}",u)}
          </div>`}

          <${pw}
            label=${m("common.pinned")}
            items=${p}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${f}
            onSelect=${n}
            onDelete=${r}
          />
          <${pw}
            label=${m("common.recent")}
            items=${b}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${f}
            onSelect=${n}
            onDelete=${r}
          />
        </div>
      `}
    </div>
  `}function pc(){let e=Z(),t=q({queryKey:["trace-credits"],queryFn:q$,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=B({mutationFn:z$,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function O4(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function vw(){let e=k(),{credits:t}=pc();if(!t||!t.enrolled)return null;let a=O4(t.final_credit),n=t.submissions_accepted||0,r=t.submissions_submitted||0,s=t.manual_review_hold_count||0;return l`
    <div className="px-3 pb-1">
      <${Dr}
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
        ${s>0&&l`
          <div className="mt-1 text-[11px] font-medium text-[var(--v2-accent-text)]">
            ${e("traceCommons.cardHeld",{count:s})}
          </div>
        `}
      <//>
    </div>
  `}function gw({threadsState:e,theme:t,toggleTheme:a,profile:n,isAdmin:r,rebornProjectsEnabled:s=!1,onSignOut:i,onClose:o,onNewChat:u,onSelectThread:c,onDeleteThread:d}){return l`
    <aside
      className="flex h-full w-[260px] shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
    >
      <div className="flex items-center gap-2.5 px-4 py-5">
        <${Dr}
          to="/chat"
          onClick=${o}
          className="flex items-center gap-2.5 opacity-90 hover:opacity-100"
          aria-label="IronClaw"
        >
          <img src="/v2/assets/logo.jpg" alt="IronClaw" className="h-7 w-auto" />
        <//>
      </div>

      <${iw}
        onNewChat=${u}
        isCreating=${e.isCreating}
        isAdmin=${r}
        onNavigate=${o}
      />

      <${vw} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${hw}
          threads=${e.threads}
          activeThreadId=${e.activeThreadId}
          rebornProjectsEnabled=${s}
          onSelect=${c}
          onDelete=${d}
          onNavigate=${o}
        />
      </div>

      <${rw}
        theme=${t}
        toggleTheme=${a}
        profile=${n}
        onSignOut=${i}
      />
    </aside>
  `}var L4="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",P4="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",yw="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",bw={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},xw={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function E({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=bw[n]??bw.md,u=r?"w-full":"";if(a==="primary")return l`
      <${s}
        style=${{background:L4,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${V(yw,o,u,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:P4}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=xw[a]??xw.outline;return l`
    <${s}
      className=${V(yw,o,u,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function $w(){let e=h.default.useMemo(()=>U4(window.location),[]),[t,a]=h.default.useState(null),[n,r]=h.default.useState(null),[s,i]=h.default.useState(!1),[o,u]=h.default.useState(""),[c,d]=h.default.useState(!1);h.default.useEffect(()=>{if(!e)return;let p=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:p.signal}).then(b=>{if(!b.ok)throw new Error(String(b.status));return b.json()}).then(a).catch(()=>{p.signal.aborted||a(null)}),()=>p.abort()},[e]);let f=h.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),u("");try{let p=await fetch(`${e.base}/attestation/report`);if(!p.ok)throw new Error(String(p.status));let b=await p.json();return r(b),b}catch(p){return u(p.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),m=h.default.useCallback(async()=>{let p=n||await f();return!p||!navigator.clipboard?!1:(await navigator.clipboard.writeText(JSON.stringify({...p,instance_attestation:t},null,2)),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[f,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:f,copyReport:m}}function U4(e){let t=e.hostname;if(!t||t==="localhost"||j4(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function j4(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}var F4=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function ww(){let e=k(),t=$w(),[a,n]=h.default.useState(!1),r=h.default.useCallback(()=>{n(o=>{let u=!o;return u&&t.loadReport(),u})},[t]),s=h.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=q4({teeInfo:t.teeInfo,report:t.report,t:e});return l`
    <div className="relative">
      <button
        type="button"
        onClick=${r}
        aria-expanded=${a}
        title=${e("tee.title")}
        className=${V("grid h-8 w-8 place-items-center rounded-[8px]","border border-[color-mix(in_srgb,var(--v2-positive-text)_28%,transparent)]","bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]","hover:border-[color-mix(in_srgb,var(--v2-positive-text)_52%,transparent)]")}
      >
        <${D} name="shield" className="h-4 w-4" />
      </button>

      ${a&&l`
        <div
          className=${V("absolute right-0 top-full z-40 mt-2 w-[min(22rem,calc(100vw-2rem))]","rounded-[14px] border border-[var(--v2-panel-border)]","bg-[var(--v2-surface)] p-3 shadow-[0_18px_48px_rgba(0,0,0,0.35)]")}
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
            <${E}
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
  `}function q4({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return F4.map(([r,s])=>({label:a(s),value:z4(n[r])||a("common.unknown")}))}function z4(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var B4="https://docs.ironclaw.com";function Sw({threadsState:e,onToggleSidebar:t}){let a=k(),n=je(),r=h.default.useMemo(()=>{for(let i of qo){let o=oc[i.id];if(!o)continue;let u=i.path+"/";if(n.pathname.startsWith(u)){let c=n.pathname.slice(u.length).split("/")[0],d=o.find(f=>f.id===c);if(d)return{parent:a(i.labelKey),current:a(d.labelKey)}}}return null},[n.pathname,a]),s=h.default.useMemo(()=>{if(r)return null;if(n.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(u=>u.id===e.activeThreadId)?.title||a("nav.chat");let i=qo.find(o=>n.pathname.startsWith(o.path));return i?a(i.labelKey):""},[n.pathname,e.activeThreadId,e.threads,a,r]);return l`
    <header
      className=${V("flex h-14 shrink-0 items-center gap-3 px-4","border-b border-[var(--v2-panel-border)]","bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] backdrop-blur-xl")}
    >
      <button
        onClick=${t}
        className="grid h-8 w-8 shrink-0 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] md:hidden"
        aria-label="Toggle sidebar"
      >
        <${D} name="list" className="h-4 w-4" />
      </button>

      ${r?l`
            <div className="flex min-w-0 items-center gap-2 text-[14px] font-semibold">
              <span className="shrink-0 text-[var(--v2-text-muted)]">
                ${r.parent}
              </span>
              <${D}
                name="chevron"
                className="h-3.5 w-3.5 shrink-0 -rotate-90 text-[var(--v2-text-muted)]"
              />
              <span className="truncate text-[var(--v2-text-strong)]">
                ${r.current}
              </span>
            </div>
          `:l`
            <span
              className="truncate text-[14px] font-semibold text-[var(--v2-text-strong)]"
            >
              ${s}
            </span>
          `}

      <div className="ml-auto flex shrink-0 items-center gap-1">
        <${ww} />
        <${Qa}
          to="/logs"
          className=${({isActive:i})=>V("inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",i&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
          title=${a("nav.logs")}
        >
          ${a("nav.logs")}
        <//>
        <a
          href=${B4}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${a("nav.docs")}
        >
          ${a("nav.docs")}
        </a>
      </div>
    </header>
  `}function Nw({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=me(),i=k(),[o,u]=h.default.useState(""),[c,d]=h.default.useState(0),f=h.default.useRef(null),m=h.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(x=>({id:`thread-${x.id}`,label:x.title||`Thread ${x.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${x.id}`)}));return[...g,...v]},[a,s,n,r]),p=h.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?m.filter(v=>v.label.toLowerCase().includes(g)):m},[m,o]);h.default.useEffect(()=>{if(!e)return;u(""),d(0);let g=window.requestAnimationFrame(()=>f.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),h.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,p.length-1)))},[p.length]);let b=h.default.useCallback(g=>{g&&(t(),g.run())},[t]),y=h.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,p.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),b(p[c])):g.key==="Escape"&&(g.preventDefault(),t())},[p,c,b,t]);if(!e)return null;let $=null;return l`
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh]" role="dialog" aria-modal="true" aria-label="Command palette">
      <button type="button" aria-label="Close" onClick=${t} className="absolute inset-0 bg-black/50"></button>
      <div className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_30px_60px_-20px_rgba(0,0,0,0.8)]">
        <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-3">
          <${D} name="search" className="h-4 w-4 text-[var(--v2-text-faint)]" />
          <input
            ref=${f}
            value=${o}
            onInput=${g=>u(g.currentTarget.value)}
            onKeyDown=${y}
            placeholder=${i("command.placeholder")}
            className="h-12 w-full border-0 bg-transparent text-sm text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)]"
          />
          <kbd className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]">esc</kbd>
        </div>
        <ul className="max-h-[50vh] overflow-y-auto p-1.5">
          ${p.length===0&&l`<li className="px-3 py-6 text-center text-sm text-[var(--v2-text-faint)]">No matches</li>`}
          ${p.map((g,v)=>{let x=g.group!==$;return $=g.group,l`
              ${x&&l`<li key=${`g-${g.group}`} className="px-2 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">${g.group}</li>`}
              <li key=${g.id}>
                <button
                  type="button"
                  onMouseEnter=${()=>d(v)}
                  onClick=${()=>b(g)}
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
  `}var _w={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},I4={info:"bolt",success:"check",error:"close"};function kw(){let[e,t]=h.default.useState([]);return h.default.useEffect(()=>W$(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:l`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${e.map(a=>l`
          <div
            key=${a.id}
            role="status"
            className=${["pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",_w[a.tone]||_w.info].join(" ")}
          >
            <${D} name=${I4[a.tone]||"bolt"} className="h-4 w-4 shrink-0" />
            <span>${a.message}</span>
          </div>
        `)}
    </div>
  `}function Rw({token:e,profile:t,isChecking:a=!1,isAdmin:n,rebornProjectsEnabled:r=!1,onSignOut:s}){let i=k(),{theme:o,toggleTheme:u}=lc(),c=f$(e),d=tw(),f=Z$({onNewChat:()=>d.setActiveThreadId(null)}),m=c.data,p=je(),b=me(),y=Js({settings:{},gatewayStatus:m,enabled:n}),$=n&&X$({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),g=p.pathname==="/welcome"||p.pathname.startsWith("/settings"),[v,x]=h.default.useState(!1);h.default.useEffect(()=>{let S=R=>{(R.metaKey||R.ctrlKey)&&R.key.toLowerCase()==="k"&&(R.preventDefault(),x(N=>!N))};return window.addEventListener("keydown",S),()=>window.removeEventListener("keydown",S)},[]);let w=h.default.useCallback(async S=>{let R=d.activeThreadId===S;try{await d.deleteThread(S),R&&b("/chat",{replace:!0})}catch(N){console.error("Failed to delete thread:",N),Xs(ew(N,i),{tone:"error"})}},[b,d,i]);return $&&!g?l`<${ct} to="/welcome" replace />`:l`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${f.open&&l`<button
        type="button"
        aria-label=${i("nav.close")}
        onClick=${f.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${V("fixed inset-y-0 left-0 z-50 md:relative md:z-auto",f.open?"flex":"hidden md:flex")}
      >
        <${gw}
          threadsState=${d}
          theme=${o}
          toggleTheme=${u}
          profile=${t}
          isAdmin=${n}
          rebornProjectsEnabled=${r}
          onSignOut=${s}
          onClose=${f.close}
          onNewChat=${f.newChat}
          onSelectThread=${f.selectThread}
          onDeleteThread=${w}
        />
      </div>

      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <${Sw}
          threadsState=${d}
          onToggleSidebar=${f.toggle}
        />
        <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
          ${c.error&&l`
            <div
              className=${V("m-4 rounded-[14px] border px-4 py-3 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >
              ${c.error.message||i("error.gatewayConnection")}
            </div>
          `}
          <${up}
            context=${{gatewayStatus:m,gatewayStatusQuery:c,currentUser:t,isChecking:a,isAdmin:n,threadsState:d}}
          />
        </main>
      </div>
      <${Nw}
        open=${v}
        onClose=${()=>x(!1)}
        threadsState=${d}
        onNewChat=${f.newChat}
        onToggleTheme=${u}
      />
      <${kw} />
    </div>
  `}var Ht=Ie(Ve(),1),Go=e=>e.type==="checkbox",jr=e=>e instanceof Date,Dt=e=>e==null,qw=e=>typeof e=="object",Xe=e=>!Dt(e)&&!Array.isArray(e)&&qw(e)&&!jr(e),H4=e=>Xe(e)&&e.target?Go(e.target)?e.target.checked:e.target.value:e,K4=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,Q4=(e,t)=>e.has(K4(t)),V4=e=>{let t=e.constructor&&e.constructor.prototype;return Xe(t)&&t.hasOwnProperty("isPrototypeOf")},Fp=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function ht(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(Fp&&(e instanceof Blob||n))&&(a||Xe(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!V4(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=ht(e[r]));else return e;return t}var bc=e=>/^\w*$/.test(e),at=e=>e===void 0,qp=e=>Array.isArray(e)?e.filter(Boolean):[],zp=e=>qp(e.replace(/["|']|\]/g,"").split(/\.|\[/)),X=(e,t,a)=>{if(!t||!Xe(e))return a;let n=(bc(t)?[t]:zp(t)).reduce((r,s)=>Dt(r)?r:r[s],e);return at(n)||n===e?at(e[t])?a:e[t]:n},Va=e=>typeof e=="boolean",Fe=(e,t,a)=>{let n=-1,r=bc(t)?[t]:zp(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],u=a;if(n!==i){let c=e[o];u=Xe(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=u,e=e[o]}},Cw={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},Ca={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},xn={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},G4=Ht.default.createContext(null);G4.displayName="HookFormContext";var Y4=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==Ca.all&&(t._proxyFormState[i]=!n||Ca.all),a&&(a[i]=!0),e[i]}});return r},J4=typeof window<"u"?Ht.default.useLayoutEffect:Ht.default.useEffect;var Ga=e=>typeof e=="string",X4=(e,t,a,n,r)=>Ga(e)?(n&&t.watch.add(e),X(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),X(a,s))):(n&&(t.watchAll=!0),a),jp=e=>Dt(e)||!qw(e);function er(e,t,a=new WeakSet){if(jp(e)||jp(t))return e===t;if(jr(e)&&jr(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(jr(i)&&jr(o)||Xe(i)&&Xe(o)||Array.isArray(i)&&Array.isArray(o)?!er(i,o,a):i!==o)return!1}}return!0}var Z4=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},Qo=e=>Array.isArray(e)?e:[e],Ew=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},Kt=e=>Xe(e)&&!Object.keys(e).length,Bp=e=>e.type==="file",Ea=e=>typeof e=="function",vc=e=>{if(!Fp)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},zw=e=>e.type==="select-multiple",Ip=e=>e.type==="radio",W4=e=>Ip(e)||Go(e),Up=e=>vc(e)&&e.isConnected;function eA(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=at(e)?n++:e[t[n++]];return e}function tA(e){for(let t in e)if(e.hasOwnProperty(t)&&!at(e[t]))return!1;return!0}function tt(e,t){let a=Array.isArray(t)?t:bc(t)?[t]:zp(t),n=a.length===1?e:eA(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Xe(n)&&Kt(n)||Array.isArray(n)&&tA(n))&&tt(e,a.slice(0,-1)),e}var Bw=e=>{for(let t in e)if(Ea(e[t]))return!0;return!1};function gc(e,t={}){let a=Array.isArray(e);if(Xe(e)||a)for(let n in e)Array.isArray(e[n])||Xe(e[n])&&!Bw(e[n])?(t[n]=Array.isArray(e[n])?[]:{},gc(e[n],t[n])):Dt(e[n])||(t[n]=!0);return t}function Iw(e,t,a){let n=Array.isArray(e);if(Xe(e)||n)for(let r in e)Array.isArray(e[r])||Xe(e[r])&&!Bw(e[r])?at(t)||jp(a[r])?a[r]=Array.isArray(e[r])?gc(e[r],[]):{...gc(e[r])}:Iw(e[r],Dt(t)?{}:t[r],a[r]):a[r]=!er(e[r],t[r]);return a}var Ho=(e,t)=>Iw(e,t,gc(t)),Aw={value:!1,isValid:!1},Tw={value:!0,isValid:!0},Hw=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!at(e[0].attributes.value)?at(e[0].value)||e[0].value===""?Tw:{value:e[0].value,isValid:!0}:Tw:Aw}return Aw},Kw=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>at(e)?e:t?e===""?NaN:e&&+e:a&&Ga(e)?new Date(e):n?n(e):e,Dw={isValid:!1,value:null},Qw=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,Dw):Dw;function Mw(e){let t=e.ref;return Bp(t)?t.files:Ip(t)?Qw(e.refs).value:zw(t)?[...t.selectedOptions].map(({value:a})=>a):Go(t)?Hw(e.refs).value:Kw(at(t.value)?e.ref.value:t.value,e)}var aA=(e,t,a,n)=>{let r={};for(let s of e){let i=X(t,s);i&&Fe(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},yc=e=>e instanceof RegExp,Ko=e=>at(e)?e:yc(e)?e.source:Xe(e)?yc(e.value)?e.value.source:e.value:e,Ow=e=>({isOnSubmit:!e||e===Ca.onSubmit,isOnBlur:e===Ca.onBlur,isOnChange:e===Ca.onChange,isOnAll:e===Ca.all,isOnTouch:e===Ca.onTouched}),Lw="AsyncFunction",nA=e=>!!e&&!!e.validate&&!!(Ea(e.validate)&&e.validate.constructor.name===Lw||Xe(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===Lw)),rA=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),Pw=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),Vo=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=X(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(Vo(o,t))break}else if(Xe(o)&&Vo(o,t))break}}};function Uw(e,t,a){let n=X(e,a);if(n||bc(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=X(t,s),o=X(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var sA=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return Kt(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||Ca.all))},iA=(e,t,a)=>!e||!t||e===t||Qo(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),oA=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,lA=(e,t)=>!qp(X(e,t)).length&&tt(e,t),uA=(e,t,a)=>{let n=Qo(X(e,a));return Fe(n,"root",t[a]),Fe(e,a,n),e},hc=e=>Ga(e);function jw(e,t,a="validate"){if(hc(e)||Array.isArray(e)&&e.every(hc)||Va(e)&&!e)return{type:a,message:hc(e)?e:"",ref:t}}var Ws=e=>Xe(e)&&!yc(e)?e:{value:e,message:""},Fw=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:u,maxLength:c,minLength:d,min:f,max:m,pattern:p,validate:b,name:y,valueAsNumber:$,mount:g}=e._f,v=X(a,y);if(!g||t.has(y))return{};let x=o?o[0]:i,w=C=>{r&&x.reportValidity&&(x.setCustomValidity(Va(C)?"":C||""),x.reportValidity())},S={},R=Ip(i),N=Go(i),A=R||N,M=($||Bp(i))&&at(i.value)&&at(v)||vc(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,P=Z4.bind(null,y,n,S),G=(C,Y,K,oe=xn.maxLength,Se=xn.minLength)=>{let Be=C?Y:K;S[y]={type:C?oe:Se,message:Be,ref:i,...P(C?oe:Se,Be)}};if(s?!Array.isArray(v)||!v.length:u&&(!A&&(M||Dt(v))||Va(v)&&!v||N&&!Hw(o).isValid||R&&!Qw(o).isValid)){let{value:C,message:Y}=hc(u)?{value:!!u,message:u}:Ws(u);if(C&&(S[y]={type:xn.required,message:Y,ref:x,...P(xn.required,Y)},!n))return w(Y),S}if(!M&&(!Dt(f)||!Dt(m))){let C,Y,K=Ws(m),oe=Ws(f);if(!Dt(v)&&!isNaN(v)){let Se=i.valueAsNumber||v&&+v;Dt(K.value)||(C=Se>K.value),Dt(oe.value)||(Y=Se<oe.value)}else{let Se=i.valueAsDate||new Date(v),Be=gt=>new Date(new Date().toDateString()+" "+gt),vt=i.type=="time",Ze=i.type=="week";Ga(K.value)&&v&&(C=vt?Be(v)>Be(K.value):Ze?v>K.value:Se>new Date(K.value)),Ga(oe.value)&&v&&(Y=vt?Be(v)<Be(oe.value):Ze?v<oe.value:Se<new Date(oe.value))}if((C||Y)&&(G(!!C,K.message,oe.message,xn.max,xn.min),!n))return w(S[y].message),S}if((c||d)&&!M&&(Ga(v)||s&&Array.isArray(v))){let C=Ws(c),Y=Ws(d),K=!Dt(C.value)&&v.length>+C.value,oe=!Dt(Y.value)&&v.length<+Y.value;if((K||oe)&&(G(K,C.message,Y.message),!n))return w(S[y].message),S}if(p&&!M&&Ga(v)){let{value:C,message:Y}=Ws(p);if(yc(C)&&!v.match(C)&&(S[y]={type:xn.pattern,message:Y,ref:i,...P(xn.pattern,Y)},!n))return w(Y),S}if(b){if(Ea(b)){let C=await b(v,a),Y=jw(C,x);if(Y&&(S[y]={...Y,...P(xn.validate,Y.message)},!n))return w(Y.message),S}else if(Xe(b)){let C={};for(let Y in b){if(!Kt(C)&&!n)break;let K=jw(await b[Y](v,a),x,Y);K&&(C={...K,...P(Y,K.message)},w(K.message),n&&(S[y]=C))}if(!Kt(C)&&(S[y]={ref:x,...C},!n))return S}}return w(!0),S},cA={mode:Ca.onSubmit,reValidateMode:Ca.onChange,shouldFocusError:!0};function dA(e={}){let t={...cA,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:Ea(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Xe(t.defaultValues)||Xe(t.values)?ht(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:ht(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},u,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},f={...d},m={array:Ew(),state:Ew()},p=t.criteriaMode===Ca.all,b=_=>T=>{clearTimeout(c),c=setTimeout(_,T)},y=async _=>{if(!t.disabled&&(d.isValid||f.isValid||_)){let T=t.resolver?Kt((await N()).errors):await M(n,!0);T!==a.isValid&&m.state.next({isValid:T})}},$=(_,T)=>{!t.disabled&&(d.isValidating||d.validatingFields||f.isValidating||f.validatingFields)&&((_||Array.from(o.mount)).forEach(O=>{O&&(T?Fe(a.validatingFields,O,T):tt(a.validatingFields,O))}),m.state.next({validatingFields:a.validatingFields,isValidating:!Kt(a.validatingFields)}))},g=(_,T=[],O,Q,z=!0,F=!0)=>{if(Q&&O&&!t.disabled){if(i.action=!0,F&&Array.isArray(X(n,_))){let ee=O(X(n,_),Q.argA,Q.argB);z&&Fe(n,_,ee)}if(F&&Array.isArray(X(a.errors,_))){let ee=O(X(a.errors,_),Q.argA,Q.argB);z&&Fe(a.errors,_,ee),lA(a.errors,_)}if((d.touchedFields||f.touchedFields)&&F&&Array.isArray(X(a.touchedFields,_))){let ee=O(X(a.touchedFields,_),Q.argA,Q.argB);z&&Fe(a.touchedFields,_,ee)}(d.dirtyFields||f.dirtyFields)&&(a.dirtyFields=Ho(r,s)),m.state.next({name:_,isDirty:G(_,T),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else Fe(s,_,T)},v=(_,T)=>{Fe(a.errors,_,T),m.state.next({errors:a.errors})},x=_=>{a.errors=_,m.state.next({errors:a.errors,isValid:!1})},w=(_,T,O,Q)=>{let z=X(n,_);if(z){let F=X(s,_,at(O)?X(r,_):O);at(F)||Q&&Q.defaultChecked||T?Fe(s,_,T?F:Mw(z._f)):K(_,F),i.mount&&y()}},S=(_,T,O,Q,z)=>{let F=!1,ee=!1,xe={name:_};if(!t.disabled){if(!O||Q){(d.isDirty||f.isDirty)&&(ee=a.isDirty,a.isDirty=xe.isDirty=G(),F=ee!==xe.isDirty);let Ee=er(X(r,_),T);ee=!!X(a.dirtyFields,_),Ee?tt(a.dirtyFields,_):Fe(a.dirtyFields,_,!0),xe.dirtyFields=a.dirtyFields,F=F||(d.dirtyFields||f.dirtyFields)&&ee!==!Ee}if(O){let Ee=X(a.touchedFields,_);Ee||(Fe(a.touchedFields,_,O),xe.touchedFields=a.touchedFields,F=F||(d.touchedFields||f.touchedFields)&&Ee!==O)}F&&z&&m.state.next(xe)}return F?xe:{}},R=(_,T,O,Q)=>{let z=X(a.errors,_),F=(d.isValid||f.isValid)&&Va(T)&&a.isValid!==T;if(t.delayError&&O?(u=b(()=>v(_,O)),u(t.delayError)):(clearTimeout(c),u=null,O?Fe(a.errors,_,O):tt(a.errors,_)),(O?!er(z,O):z)||!Kt(Q)||F){let ee={...Q,...F&&Va(T)?{isValid:T}:{},errors:a.errors,name:_};a={...a,...ee},m.state.next(ee)}},N=async _=>{$(_,!0);let T=await t.resolver(s,t.context,aA(_||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return $(_),T},A=async _=>{let{errors:T}=await N(_);if(_)for(let O of _){let Q=X(T,O);Q?Fe(a.errors,O,Q):tt(a.errors,O)}else a.errors=T;return T},M=async(_,T,O={valid:!0})=>{for(let Q in _){let z=_[Q];if(z){let{_f:F,...ee}=z;if(F){let xe=o.array.has(F.name),Ee=z._f&&nA(z._f);Ee&&d.validatingFields&&$([Q],!0);let la=await Fw(z,o.disabled,s,p,t.shouldUseNativeValidation&&!T,xe);if(Ee&&d.validatingFields&&$([Q]),la[F.name]&&(O.valid=!1,T))break;!T&&(X(la,F.name)?xe?uA(a.errors,la,F.name):Fe(a.errors,F.name,la[F.name]):tt(a.errors,F.name))}!Kt(ee)&&await M(ee,T,O)}}return O.valid},P=()=>{for(let _ of o.unMount){let T=X(n,_);T&&(T._f.refs?T._f.refs.every(O=>!Up(O)):!Up(T._f.ref))&&I(_)}o.unMount=new Set},G=(_,T)=>!t.disabled&&(_&&T&&Fe(s,_,T),!er(gt(),r)),C=(_,T,O)=>X4(_,o,{...i.mount?s:at(T)?r:Ga(_)?{[_]:T}:T},O,T),Y=_=>qp(X(i.mount?s:r,_,t.shouldUnregister?X(r,_,[]):[])),K=(_,T,O={})=>{let Q=X(n,_),z=T;if(Q){let F=Q._f;F&&(!F.disabled&&Fe(s,_,Kw(T,F)),z=vc(F.ref)&&Dt(T)?"":T,zw(F.ref)?[...F.ref.options].forEach(ee=>ee.selected=z.includes(ee.value)):F.refs?Go(F.ref)?F.refs.forEach(ee=>{(!ee.defaultChecked||!ee.disabled)&&(Array.isArray(z)?ee.checked=!!z.find(xe=>xe===ee.value):ee.checked=z===ee.value||!!z)}):F.refs.forEach(ee=>ee.checked=ee.value===z):Bp(F.ref)?F.ref.value="":(F.ref.value=z,F.ref.type||m.state.next({name:_,values:ht(s)})))}(O.shouldDirty||O.shouldTouch)&&S(_,z,O.shouldTouch,O.shouldDirty,!0),O.shouldValidate&&Ze(_)},oe=(_,T,O)=>{for(let Q in T){if(!T.hasOwnProperty(Q))return;let z=T[Q],F=_+"."+Q,ee=X(n,F);(o.array.has(_)||Xe(z)||ee&&!ee._f)&&!jr(z)?oe(F,z,O):K(F,z,O)}},Se=(_,T,O={})=>{let Q=X(n,_),z=o.array.has(_),F=ht(T);Fe(s,_,F),z?(m.array.next({name:_,values:ht(s)}),(d.isDirty||d.dirtyFields||f.isDirty||f.dirtyFields)&&O.shouldDirty&&m.state.next({name:_,dirtyFields:Ho(r,s),isDirty:G(_,F)})):Q&&!Q._f&&!Dt(F)?oe(_,F,O):K(_,F,O),Pw(_,o)&&m.state.next({...a,name:_}),m.state.next({name:i.mount?_:void 0,values:ht(s)})},Be=async _=>{i.mount=!0;let T=_.target,O=T.name,Q=!0,z=X(n,O),F=Ee=>{Q=Number.isNaN(Ee)||jr(Ee)&&isNaN(Ee.getTime())||er(Ee,X(s,O,Ee))},ee=Ow(t.mode),xe=Ow(t.reValidateMode);if(z){let Ee,la,al=T.type?Mw(z._f):H4(_),Sn=_.type===Cw.BLUR||_.type===Cw.FOCUS_OUT,Fk=!rA(z._f)&&!t.resolver&&!X(a.errors,O)&&!z._f.deps||oA(Sn,X(a.touchedFields,O),a.isSubmitted,xe,ee),nd=Pw(O,o,Sn);Fe(s,O,al),Sn?(!T||!T.readOnly)&&(z._f.onBlur&&z._f.onBlur(_),u&&u(0)):z._f.onChange&&z._f.onChange(_);let rd=S(O,al,Sn),qk=!Kt(rd)||nd;if(!Sn&&m.state.next({name:O,type:_.type,values:ht(s)}),Fk)return(d.isValid||f.isValid)&&(t.mode==="onBlur"?Sn&&y():Sn||y()),qk&&m.state.next({name:O,...nd?{}:rd});if(!Sn&&nd&&m.state.next({...a}),t.resolver){let{errors:Th}=await N([O]);if(F(al),Q){let zk=Uw(a.errors,n,O),Dh=Uw(Th,n,zk.name||O);Ee=Dh.error,O=Dh.name,la=Kt(Th)}}else $([O],!0),Ee=(await Fw(z,o.disabled,s,p,t.shouldUseNativeValidation))[O],$([O]),F(al),Q&&(Ee?la=!1:(d.isValid||f.isValid)&&(la=await M(n,!0)));Q&&(z._f.deps&&Ze(z._f.deps),R(O,la,Ee,rd))}},vt=(_,T)=>{if(X(a.errors,T)&&_.focus)return _.focus(),1},Ze=async(_,T={})=>{let O,Q,z=Qo(_);if(t.resolver){let F=await A(at(_)?_:z);O=Kt(F),Q=_?!z.some(ee=>X(F,ee)):O}else _?(Q=(await Promise.all(z.map(async F=>{let ee=X(n,F);return await M(ee&&ee._f?{[F]:ee}:ee)}))).every(Boolean),!(!Q&&!a.isValid)&&y()):Q=O=await M(n);return m.state.next({...!Ga(_)||(d.isValid||f.isValid)&&O!==a.isValid?{}:{name:_},...t.resolver||!_?{isValid:O}:{},errors:a.errors}),T.shouldFocus&&!Q&&Vo(n,vt,_?z:o.mount),Q},gt=_=>{let T={...i.mount?s:r};return at(_)?T:Ga(_)?X(T,_):_.map(O=>X(T,O))},xa=(_,T)=>({invalid:!!X((T||a).errors,_),isDirty:!!X((T||a).dirtyFields,_),error:X((T||a).errors,_),isValidating:!!X(a.validatingFields,_),isTouched:!!X((T||a).touchedFields,_)}),Ta=_=>{_&&Qo(_).forEach(T=>tt(a.errors,T)),m.state.next({errors:_?a.errors:{}})},oa=(_,T,O)=>{let Q=(X(n,_,{_f:{}})._f||{}).ref,z=X(a.errors,_)||{},{ref:F,message:ee,type:xe,...Ee}=z;Fe(a.errors,_,{...Ee,...T,ref:Q}),m.state.next({name:_,errors:a.errors,isValid:!1}),O&&O.shouldFocus&&Q&&Q.focus&&Q.focus()},$t=(_,T)=>Ea(_)?m.state.subscribe({next:O=>"values"in O&&_(C(void 0,T),O)}):C(_,T,!0),se=_=>m.state.subscribe({next:T=>{iA(_.name,T.name,_.exact)&&sA(T,_.formState||d,ne,_.reRenderRoot)&&_.callback({values:{...s},...a,...T,defaultValues:r})}}).unsubscribe,J=_=>(i.mount=!0,f={...f,..._.formState},se({..._,formState:f})),I=(_,T={})=>{for(let O of _?Qo(_):o.mount)o.mount.delete(O),o.array.delete(O),T.keepValue||(tt(n,O),tt(s,O)),!T.keepError&&tt(a.errors,O),!T.keepDirty&&tt(a.dirtyFields,O),!T.keepTouched&&tt(a.touchedFields,O),!T.keepIsValidating&&tt(a.validatingFields,O),!t.shouldUnregister&&!T.keepDefaultValue&&tt(r,O);m.state.next({values:ht(s)}),m.state.next({...a,...T.keepDirty?{isDirty:G()}:{}}),!T.keepIsValid&&y()},fe=({disabled:_,name:T})=>{(Va(_)&&i.mount||_||o.disabled.has(T))&&(_?o.disabled.add(T):o.disabled.delete(T))},be=(_,T={})=>{let O=X(n,_),Q=Va(T.disabled)||Va(t.disabled);return Fe(n,_,{...O||{},_f:{...O&&O._f?O._f:{ref:{name:_}},name:_,mount:!0,...T}}),o.mount.add(_),O?fe({disabled:Va(T.disabled)?T.disabled:t.disabled,name:_}):w(_,!0,T.value),{...Q?{disabled:T.disabled||t.disabled}:{},...t.progressive?{required:!!T.required,min:Ko(T.min),max:Ko(T.max),minLength:Ko(T.minLength),maxLength:Ko(T.maxLength),pattern:Ko(T.pattern)}:{},name:_,onChange:Be,onBlur:Be,ref:z=>{if(z){be(_,T),O=X(n,_);let F=at(z.value)&&z.querySelectorAll&&z.querySelectorAll("input,select,textarea")[0]||z,ee=W4(F),xe=O._f.refs||[];if(ee?xe.find(Ee=>Ee===F):F===O._f.ref)return;Fe(n,_,{_f:{...O._f,...ee?{refs:[...xe.filter(Up),F,...Array.isArray(X(r,_))?[{}]:[]],ref:{type:F.type,name:_}}:{ref:F}}}),w(_,!1,void 0,F)}else O=X(n,_,{}),O._f&&(O._f.mount=!1),(t.shouldUnregister||T.shouldUnregister)&&!(Q4(o.array,_)&&i.action)&&o.unMount.add(_)}}},st=()=>t.shouldFocusError&&Vo(n,vt,o.mount),Qe=_=>{Va(_)&&(m.state.next({disabled:_}),Vo(n,(T,O)=>{let Q=X(n,O);Q&&(T.disabled=Q._f.disabled||_,Array.isArray(Q._f.refs)&&Q._f.refs.forEach(z=>{z.disabled=Q._f.disabled||_}))},0,!1))},qe=(_,T)=>async O=>{let Q;O&&(O.preventDefault&&O.preventDefault(),O.persist&&O.persist());let z=ht(s);if(m.state.next({isSubmitting:!0}),t.resolver){let{errors:F,values:ee}=await N();a.errors=F,z=ht(ee)}else await M(n);if(o.disabled.size)for(let F of o.disabled)tt(z,F);if(tt(a.errors,"root"),Kt(a.errors)){m.state.next({errors:{}});try{await _(z,O)}catch(F){Q=F}}else T&&await T({...a.errors},O),st(),setTimeout(st);if(m.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:Kt(a.errors)&&!Q,submitCount:a.submitCount+1,errors:a.errors}),Q)throw Q},$a=(_,T={})=>{X(n,_)&&(at(T.defaultValue)?Se(_,ht(X(r,_))):(Se(_,T.defaultValue),Fe(r,_,ht(T.defaultValue))),T.keepTouched||tt(a.touchedFields,_),T.keepDirty||(tt(a.dirtyFields,_),a.isDirty=T.defaultValue?G(_,ht(X(r,_))):G()),T.keepError||(tt(a.errors,_),d.isValid&&y()),m.state.next({...a}))},Ot=(_,T={})=>{let O=_?ht(_):r,Q=ht(O),z=Kt(_),F=z?r:Q;if(T.keepDefaultValues||(r=O),!T.keepValues){if(T.keepDirtyValues){let ee=new Set([...o.mount,...Object.keys(Ho(r,s))]);for(let xe of Array.from(ee))X(a.dirtyFields,xe)?Fe(F,xe,X(s,xe)):Se(xe,X(F,xe))}else{if(Fp&&at(_))for(let ee of o.mount){let xe=X(n,ee);if(xe&&xe._f){let Ee=Array.isArray(xe._f.refs)?xe._f.refs[0]:xe._f.ref;if(vc(Ee)){let la=Ee.closest("form");if(la){la.reset();break}}}}if(T.keepFieldsRef)for(let ee of o.mount)Se(ee,X(F,ee));else n={}}s=t.shouldUnregister?T.keepDefaultValues?ht(r):{}:ht(F),m.array.next({values:{...F}}),m.state.next({values:{...F}})}o={mount:T.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!T.keepIsValid||!!T.keepDirtyValues,i.watch=!!t.shouldUnregister,m.state.next({submitCount:T.keepSubmitCount?a.submitCount:0,isDirty:z?!1:T.keepDirty?a.isDirty:!!(T.keepDefaultValues&&!er(_,r)),isSubmitted:T.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:z?{}:T.keepDirtyValues?T.keepDefaultValues&&s?Ho(r,s):a.dirtyFields:T.keepDefaultValues&&_?Ho(r,_):T.keepDirty?a.dirtyFields:{},touchedFields:T.keepTouched?a.touchedFields:{},errors:T.keepErrors?a.errors:{},isSubmitSuccessful:T.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},wa=(_,T)=>Ot(Ea(_)?_(s):_,T),Re=(_,T={})=>{let O=X(n,_),Q=O&&O._f;if(Q){let z=Q.refs?Q.refs[0]:Q.ref;z.focus&&(z.focus(),T.shouldSelect&&Ea(z.select)&&z.select())}},ne=_=>{a={...a,..._}},wt={control:{register:be,unregister:I,getFieldState:xa,handleSubmit:qe,setError:oa,_subscribe:se,_runSchema:N,_focusError:st,_getWatch:C,_getDirty:G,_setValid:y,_setFieldArray:g,_setDisabledField:fe,_setErrors:x,_getFieldArray:Y,_reset:Ot,_resetDefaultValues:()=>Ea(t.defaultValues)&&t.defaultValues().then(_=>{wa(_,t.resetOptions),m.state.next({isLoading:!1})}),_removeUnmounted:P,_disableForm:Qe,_subjects:m,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(_){i=_},get _defaultValues(){return r},get _names(){return o},set _names(_){o=_},get _formState(){return a},get _options(){return t},set _options(_){t={...t,..._}}},subscribe:J,trigger:Ze,register:be,handleSubmit:qe,watch:$t,setValue:Se,getValues:gt,reset:wa,resetField:$a,clearErrors:Ta,unregister:I,setError:oa,setFocus:Re,getFieldState:xa};return{...wt,formControl:wt}}function Vw(e={}){let t=Ht.default.useRef(void 0),a=Ht.default.useRef(void 0),[n,r]=Ht.default.useState({isDirty:!1,isValidating:!1,isLoading:Ea(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:Ea(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!Ea(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=dA(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,J4(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),Ht.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),Ht.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),Ht.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),Ht.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),Ht.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),Ht.default.useEffect(()=>{e.values&&!er(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),Ht.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=Y4(n,s),t.current}var Gw={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},Yw={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},mA={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function ae({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return l`
    <${s}
      className=${V(Gw[a]??Gw.default,Yw[n]??Yw.md,mA[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var Hp="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",xc={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function Mt({className:e="",size:t="md",error:a=!1,...n}){return l`
    <input
      className=${V(Hp,xc[t]??xc.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function $c({className:e="",error:t=!1,rows:a=4,...n}){return l`
    <textarea
      rows=${a}
      className=${V(Hp,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Kp({children:e,className:t="",size:a="md",error:n=!1,...r}){return l`
    <div className="relative w-full">
      <select
        className=${V(Hp,xc[a]??xc.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
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
  `}function fA({children:e,className:t="",required:a=!1,...n}){return l`
    <label
      className=${V("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&l`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function $n({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return l`
    <div className=${V("flex flex-col gap-2",s)}>
      ${e&&l`<${fA} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&l`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&l`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var pA={google:"Google",github:"GitHub",apple:"Apple"};function hA(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function Jw({providers:e,redirectAfter:t}){let a=k();return e.length?l`
    <div className="mt-6 space-y-3">
      <div className="flex items-center gap-3 text-[11px] uppercase text-[var(--v2-text-faint)]">
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
        <span>${a("login.oauthDivider")}</span>
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
      </div>
      <div className="grid gap-2">
        ${e.map(n=>l`
            <${E}
              key=${n}
              as="a"
              href=${hA(n,t)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <${D} name="shield" className="h-4 w-4" />
              ${a("login.oauthProvider",{provider:pA[n]||n})}
            <//>
          `)}
      </div>
    </div>
  `:null}var vA=["google","github","apple"];function Xw(){let[e,t]=h.default.useState([]);return h.default.useEffect(()=>{let a=!1;return Px().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(vA.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function Zw({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=k(),{theme:s,toggleTheme:i}=lc(),o=Xw(),{formState:{errors:u,isSubmitting:c},handleSubmit:d,register:f}=Vw({defaultValues:{token:e||""}});return l`
    <main
      className="relative flex min-h-[100dvh] items-center justify-center bg-[var(--v2-canvas)] px-4 py-8 sm:px-6 lg:px-12"
    >
      <!-- Theme toggle -->
      <${E}
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
          onSubmit=${d(({token:m})=>n(m))}
        >
          <${$n}
            label=${r("login.tokenLabel")}
            htmlFor="v2-token"
            error=${u.token?.message??""}
            hint=${r("login.tokenHint")}
          >
            <${Mt}
              id="v2-token"
              type="password"
              error=${!!u.token}
              ...${f("token",{required:r("login.tokenRequired"),setValueAs:m=>m.trim()})}
              placeholder=${r("login.tokenPlaceholder")}
              autocomplete="current-password"
            />
          <//>

          ${t&&l`<p
              className=${V("rounded-[10px] border px-3 py-2 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >${t}</p>`}

          <${E}
            type="submit"
            variant="primary"
            fullWidth
            disabled=${c}
          >
            ${r("login.connect")}
          <//>
        </form>

        <${Jw}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var Ww={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},e1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function U({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return l`
    <span
      className=${V("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",e1[n]??e1.md,Ww[e]??Ww.muted,r)}
    >
      ${a&&l`<span
          className=${V("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var gA=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,t1=/(bash|shell|exec|run|command|terminal|spawn|process)/,a1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function n1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return gA.test(n)?{tone:"danger",key:"tool.riskWrite"}:t1.test(n)?{tone:"warning",key:"tool.riskExec"}:a1.test(n)?{tone:"info",key:"tool.riskNetwork"}:t1.test(r)?{tone:"warning",key:"tool.riskExec"}:a1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}var wc=480;function yA(e,t){return t&&t.length>0?t.some(a=>typeof a?.value=="string"&&a.value.length>wc):typeof e=="string"&&e.length>wc}function r1(e,t){return typeof e!="string"||t||e.length<=wc?e:`${e.slice(0,wc).trimEnd()}
...`}function s1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=k(),{toolName:s,description:i,parameters:o,allowAlways:u,approvalDetails:c=[]}=e,[d,f]=h.default.useState(!1),[m,p]=h.default.useState(!1);h.default.useEffect(()=>{p(!1)},[e]);let b=h.default.useMemo(()=>n1(s,i,o),[s,i,o]),y=s||r("approval.thisTool"),$=yA(o,c),g=m?"max-h-72":"max-h-36",v=h.default.useCallback(()=>{d&&u?n?.():t?.()},[d,u,n,t]);return l`
    <div className="mx-auto max-w-lg rounded-xl border border-copper/30 bg-copper/10 p-4">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-copper/25 bg-copper/10 text-copper">
          <${D} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">${r("approval.title")}</span>
        <${U}
          tone=${b.tone}
          label=${r(b.key)}
          dot=${!1}
          size="sm"
          className="ml-auto"
        />
      </div>
      ${s&&l`<div className="mb-1 break-all font-mono text-sm font-medium text-iron-100">${s}</div>`}
      ${i&&l`<div className="mb-3 break-words text-sm text-iron-200">${i}</div>`}
      ${c.length>0?l`
            <dl className=${`mb-2 ${g} overflow-y-auto rounded-md border border-iron-800 bg-iron-950/80 text-xs`}>
              ${c.map(x=>l`
                  <div className="grid gap-1 border-b border-iron-800/70 px-3 py-2 last:border-b-0 sm:grid-cols-[7rem_1fr]">
                    <dt className="font-medium text-iron-400">${x.label}</dt>
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${r1(x.value,m)}</dd>
                  </div>
                `)}
            </dl>
          `:o&&l`<pre className=${`mb-2 ${g} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${r1(o,m)}</pre>`}

      ${$&&l`
        <${E}
          variant="ghost"
          size="sm"
          className="mb-3 px-0 text-[var(--v2-accent)] hover:bg-transparent"
          onClick=${()=>p(x=>!x)}
          type="button"
        >
          ${r(m?"approval.showCommandPreview":"approval.viewFullCommand")}
        <//>
      `}

      ${u&&l`
        <label className="mb-3 flex items-center gap-2 text-xs text-iron-200">
          <input
            type="checkbox"
            checked=${d}
            onChange=${x=>f(x.currentTarget.checked)}
            className="h-3.5 w-3.5 accent-[var(--v2-accent)]"
          />
          ${r("approval.alwaysAllowToolLabel",{tool:y})}
        </label>
      `}

      <div className="flex flex-wrap gap-2">
        <${E} variant="primary" onClick=${v}>
          ${r(d&&u?"approval.approveAndAlways":"approval.approve")}
        <//>
        <${E} variant="secondary" onClick=${()=>a?.()}>
          ${r("approval.deny")}
        <//>
      </div>
    </div>
  `}function ei({icon:e="lock",headline:t,provider:a,accountLabel:n,body:r,expiresAt:s,pillHint:i,defaultExpanded:o=!0,children:u}){let c=k(),[d,f]=h.default.useState(o),m=h.default.useId(),p=n||a||"";return l`
    <div className="mx-auto w-full max-w-lg rounded-xl border border-[rgba(76,167,230,0.34)] bg-[rgba(76,167,230,0.08)]">
      <button
        type="button"
        onClick=${()=>f(b=>!b)}
        aria-expanded=${d?"true":"false"}
        aria-controls=${m}
        className="flex w-full items-center gap-3 rounded-xl border-0 bg-transparent px-4 py-3 text-left"
      >
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-md border border-[rgba(76,167,230,0.28)] bg-[rgba(76,167,230,0.1)] text-[#8fc8f2]">
          <${D} name=${e} className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold text-white">
            ${t||c("authGate.title")}
          </span>
          ${p&&l`<span className="block truncate text-xs text-iron-300">${p}</span>`}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 text-xs font-medium text-[#8fc8f2]">
          ${i&&l`<span className="hidden sm:inline">${i}</span>`}
          <${D}
            name="chevron"
            className=${["h-4 w-4",d?"rotate-180":""].join(" ")}
          />
        </span>
      </button>

      ${d&&l`
        <div
          id=${m}
          className="border-t border-[rgba(76,167,230,0.2)] px-4 pb-4 pt-3"
        >
          ${r&&l`<div className="mb-3 text-sm text-iron-200">${r}</div>`}
          ${u}
          ${s&&l`
            <p className="mt-2 text-xs text-iron-300">
              ${c("authGate.expiresAt")}: ${new Date(s).toLocaleString()}
            </p>
          `}
        </div>
      `}
    </div>
  `}function i1({gate:e,onCancel:t}){let a=k();return l`
    <${ei}
      icon="lock"
      headline=${e?.headline||a("authGate.title")}
      body=${e?.body||""}
    >
      <form onSubmit=${n=>n.preventDefault()}>
        <div className="mb-3 text-sm text-iron-200">
          ${a("authGate.unsupportedChallenge")}
        </div>
        <div className="flex flex-wrap gap-2">
          <${E} type="button" variant="secondary" onClick=${()=>t?.()}>
            ${a("authGate.cancel")}
          <//>
        </div>
      </form>
    <//>
  `}function o1({gate:e,onCancel:t}){let a=k(),[n,r]=h.default.useState(!1),[s,i]=h.default.useState(""),o=h.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]);h.default.useEffect(()=>{i("")},[e.authorizationUrl,e.gateRef,e.runId]);let u=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),c=h.default.useCallback(()=>{if(!o){i(a("authGate.serviceUnavailable"));return}i(""),window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0)},[e.authorizationUrl,o]),d=n?a("authGate.reopenAuthorization",{provider:u}):a("authGate.openAuthorization",{provider:u});return l`
    <${ei}
      icon="link"
      headline=${e?.headline||a("authGate.oauthTitle")}
      provider=${e?.provider?u:""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      expiresAt=${e?.expiresAt||""}
      pillHint=${a("authGate.pillAuthorize")}
    >
      <div className="flex flex-wrap gap-2">
        <${E}
          as="a"
          href=${o?e.authorizationUrl:void 0}
          target="_blank"
          rel="noopener noreferrer"
          className="auth-oauth"
          variant="primary"
          onClick=${f=>{f.preventDefault(),c()}}
        >
          <${D} name="link" className="h-4 w-4" />
          ${d}
        <//>
        <${E}
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
  `}function l1({gate:e,onSubmit:t,onCancel:a}){let n=k(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState(!1),d=h.default.useCallback(async f=>{f.preventDefault();let m=r.trim();if(!m){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(m),s("")}catch(p){o(p?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return l`
    <${ei}
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
            disabled=${u}
            placeholder=${n("authGate.tokenPlaceholder")}
            aria-label=${n("authGate.tokenLabel")}
            error=${!!i}
            onInput=${f=>s(f.currentTarget.value)}
          />
          ${i&&l`
            <p className="mt-2 text-xs text-[var(--v2-danger-text)]" role="alert">
              ${i}
            </p>
          `}
        </div>
        <div className="flex flex-wrap gap-2">
          <${E} type="submit" variant="primary" disabled=${u}>
            ${n(u?"authGate.submitting":"authGate.submit")}
          <//>
          <${E}
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
  `}var bA="/api/webchat/v2/extensions/pairing/redeem";function u1(e){return H(bA,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function Sc({action:e}){let t=k(),a=Z(),n=B({mutationFn:({code:u})=>u1(u),onSuccess:()=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=h.default.useState(""),i=xA(e,t),o=()=>{let u=r.trim();u&&(n.mutate({code:u}),s(""))};return l`
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
          onChange=${u=>s(u.target.value)}
          onKeyDown=${u=>u.key==="Enter"&&o()}
          placeholder=${i.codePlaceholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${E}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${o}
          disabled=${n.isPending||!r.trim()}
        >
          ${i.submitLabel}
        <//>
      </div>

      ${n.isSuccess&&l`<p className="text-xs text-emerald-300">
        ${n.data?.message||i.successMessage}
      </p>`}
      ${n.isError&&l`<p className="text-xs text-red-300">
        ${$A(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function xA(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function $A(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function wA(e,t){return e?.channel==="slack"&&e.strategy===t}function c1({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return l`
    <div className="rounded-[16px] border border-white/[0.06] bg-white/[0.02] p-3">
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
            onClick=${t}
            className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-iron-400 hover:bg-white/[0.04] hover:text-iron-100"
          >
            <${D} name="close" className="h-4 w-4" />
          </button>
        `}
      </div>

      ${wA(e,"inbound_proof_code")?l`<${Sc} action=${e.action} />`:l`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function SA(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):Or.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:Or.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:Or.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:Or.maxTotalBytes}:Or}function d1(){let e=ba(),t=q({enabled:!!e,queryKey:["session"],queryFn:tc,staleTime:5*6e4});return SA(t.data)}function Nc({onSend:e,onCancel:t,disabled:a,canCancel:n=!1,initialText:r="",resetKey:s="",draftKey:i=Fo,variant:o="dock",context:u={},statusText:c=""}){let d=k(),f=o==="hero",m=d1(),[p,b]=h.default.useState(()=>Rp(i)),[y,$]=h.default.useState(()=>Ep(i)),[g,v]=h.default.useState(""),[x,w]=h.default.useState(!1),[S,R]=h.default.useState(!1),[N,A]=h.default.useState(!1),M=h.default.useRef(null),P=h.default.useRef(null),G=h.default.useRef([]),C=h.default.useRef(Promise.resolve());h.default.useEffect(()=>{G.current=y},[y]);let Y=h.default.useRef(null),K=h.default.useRef(null),oe=h.default.useCallback(()=>{K.current&&(window.clearTimeout(K.current),K.current=null);let ne=Y.current;Y.current=null,ne&&ne.scope===kt()&&Cp(ne.key,ne.text)},[]),Se=h.default.useCallback(()=>{K.current&&(window.clearTimeout(K.current),K.current=null),Y.current=null},[]),Be=h.default.useCallback(()=>{let ne=M.current;ne&&(ne.style.height="auto",ne.style.height=`${Math.min(ne.scrollHeight,200)}px`)},[]);h.default.useEffect(()=>{Be()},[p,Be]),h.default.useEffect(()=>(b(Rp(i)),()=>oe()),[i,oe]);let vt=h.default.useRef(i);h.default.useEffect(()=>{if(vt.current!==i){vt.current=i,$(Ep(i)),v("");return}l$(i,y)},[i,y]),h.default.useEffect(()=>{r&&(b(r),window.requestAnimationFrame(()=>{M.current&&(M.current.focus(),M.current.setSelectionRange(r.length,r.length))}))},[r,s]);let Ze=h.default.useCallback(ne=>{a||!ne||ne.length===0||(C.current=C.current.then(async()=>{let{staged:Ce,errors:wt}=await Jx(ne,{limits:m,existing:G.current,t:d});Ce.length>0&&$(_=>{let T=[..._,...Ce];return G.current=T,T}),v(wt.length>0?wt.join(" "):"")}).catch(()=>{v(d("chat.attachmentStagingFailed"))}))},[a,m,d]),gt=h.default.useCallback(ne=>{$(Ce=>{let wt=Ce.filter(_=>_.id!==ne);return G.current=wt,wt}),v("")},[]),xa=h.default.useCallback(()=>{a||P.current?.click()},[a]),Ta=h.default.useCallback(ne=>{let Ce=Array.from(ne.target.files||[]);Ze(Ce),ne.target.value=""},[Ze]),oa=h.default.useCallback(async()=>{if(!(!p.trim()||a||x)){w(!0);try{await e(p.trim(),{attachments:y}),b(""),$([]),G.current=[],v(""),Se(),o$(i),u$(i),M.current&&(M.current.style.height="auto")}catch{}finally{w(!1)}}},[p,y,a,x,e,i,Se]),$t=h.default.useCallback(ne=>{let Ce=ne.target.value;b(Ce),Y.current={key:i,text:Ce,scope:kt()},K.current&&window.clearTimeout(K.current),K.current=window.setTimeout(oe,300)},[i,oe]),se=h.default.useCallback(async()=>{if(!(!n||S||!t)){R(!0);try{await t()}finally{R(!1)}}},[n,S,t]),J=h.default.useCallback(ne=>{ne.key==="Enter"&&!ne.shiftKey&&(ne.preventDefault(),oa())},[oa]),I=h.default.useCallback(ne=>{let Ce=Array.from(ne.clipboardData?.files||[]);Ce.length>0&&(ne.preventDefault(),Ze(Ce))},[Ze]),fe=h.default.useCallback(ne=>{ne.preventDefault(),A(!1);let Ce=Array.from(ne.dataTransfer?.files||[]);Ce.length>0&&Ze(Ce)},[Ze]),be=h.default.useCallback(ne=>{ne.preventDefault(),!a&&A(!0)},[a]),st=h.default.useCallback(ne=>{ne.currentTarget.contains(ne.relatedTarget)||A(!1)},[]),Qe=p.trim(),qe=d(f?"chat.heroPlaceholder":"chat.followUpPlaceholder"),$a=m.accept.length>0?m.accept.join(","):void 0,Ot=f?"w-full":"px-4 py-3 sm:px-5 lg:px-8",wa=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",f?"min-h-[120px]":"",a?"opacity-70":""].join(" "),Re=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",f?"min-h-[72px]":"min-h-[40px]"].join(" ");return l`
    <div className=${Ot}>
      <div
        className=${wa}
        onDrop=${fe}
        onDragOver=${be}
        onDragLeave=${st}
      >
        ${N&&l`
          <div className="pointer-events-none absolute inset-1 z-10 flex items-center justify-center rounded-[16px] border border-dashed border-[color-mix(in_srgb,var(--v2-accent)_55%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-canvas)_82%,transparent)] text-sm font-medium text-[var(--v2-accent-text)]">
            ${d("chat.attachmentDropHint")}
          </div>
        `}
        ${g&&l`
          <div
            role="alert"
            className="mb-3 flex items-start gap-2 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-2 text-xs leading-5 text-[var(--v2-danger-text)]"
          >
            <span className="min-w-0 flex-1">${g}</span>
            <button
              type="button"
              onClick=${()=>v("")}
              aria-label=${d("common.dismiss")}
              title=${d("common.dismiss")}
              className="-mr-1 -mt-0.5 shrink-0 rounded p-0.5 text-[color-mix(in_srgb,var(--v2-danger-text)_80%,transparent)] transition hover:bg-[color-mix(in_srgb,var(--v2-danger-text)_14%,transparent)] hover:text-[var(--v2-danger-text)]"
            >
              <${D} name="close" className="h-3.5 w-3.5" strokeWidth=${2} />
            </button>
          </div>
        `}

        ${y.length>0&&l`
          <div className="mb-2 flex flex-wrap gap-2 px-1">
            ${y.map(ne=>l`
                <div
                  key=${ne.id}
                  className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
                >
                  ${ne.previewUrl?l`<img
                        src=${ne.previewUrl}
                        alt=${ne.filename}
                        className="h-9 w-9 shrink-0 rounded object-cover"
                      />`:l`<span
                        className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                      >
                        <${D} name="file" className="h-4 w-4" />
                      </span>`}
                  <span className="flex min-w-0 flex-col">
                    <span className="max-w-[12rem] truncate font-medium">
                      ${ne.filename}
                    </span>
                    <span className="text-[10px] text-iron-400">${ne.sizeLabel}</span>
                  </span>
                  <button
                    type="button"
                    onClick=${()=>gt(ne.id)}
                    aria-label=${d("chat.attachmentRemove")}
                    title=${d("chat.attachmentRemove")}
                    className="absolute right-1 top-1 grid h-5 w-5 place-items-center rounded-full text-iron-400 hover:bg-iron-700 hover:text-white"
                  >
                    <${D} name="close" className="h-3 w-3" />
                  </button>
                </div>
              `)}
          </div>
        `}

        <textarea
          ref=${M}
          data-testid="chat-composer"
          value=${p}
          onChange=${$t}
          onKeyDown=${J}
          onPaste=${I}
          placeholder=${qe}
          rows=${1}
          disabled=${a}
          className=${Re}
        />

        <input
          ref=${P}
          type="file"
          multiple
          accept=${$a}
          className="hidden"
          onChange=${Ta}
        />

        <div className="mt-2 flex items-center gap-2">
          ${a&&l`
            <span className="inline-flex items-center gap-2 text-xs text-[var(--v2-text-muted)]">
              <span className="h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
              ${c||d("chat.statusWorking")}
            </span>
          `}
          <div className="ml-auto flex items-center gap-1.5">
            <button
              type="button"
              onClick=${xa}
              disabled=${a}
              aria-label=${d("chat.attachFiles")}
              title=${d("chat.attachFiles")}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <${D} name="plus" className="h-5 w-5" />
            </button>
            ${n?l`
                <${E}
                  type="button"
                  variant="danger"
                  size="icon-sm"
                  onClick=${se}
                  disabled=${S}
                  aria-label=${d("common.cancel")}
                  title=${d("common.cancel")}
                  className="rounded-full"
                >
                  <${D} name="close" className="h-5 w-5" />
                <//>
              `:l`
                <${E}
                  type="button"
                  variant="primary"
                  size="icon-sm"
                  onClick=${oa}
                  disabled=${a||x||!Qe}
                  aria-label=${d("chat.send")}
                  className="rounded-full"
                >
                  <${D} name="send" className="h-5 w-5" />
                <//>
              `}
          </div>
        </div>
      </div>
    </div>
  `}var m1={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function f1({status:e}){let t=k();if(e==="idle"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return l`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",m1[e]||m1.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function p1({onSuggestion:e,onSend:t,disabled:a,initialText:n,resetKey:r,draftKey:s,context:i,statusText:o,canCancel:u,onCancel:c}){let d=k(),f=[{icon:"tool",title:d("chat.suggestion1"),detail:d("chat.suggestion1Desc")},{icon:"shield",title:d("chat.suggestion2"),detail:d("chat.suggestion2Desc")},{icon:"plug",title:d("chat.suggestion3"),detail:d("chat.suggestion3Desc")}];return l`
    <div
      className="v2-page-entrance flex min-h-0 flex-1 flex-col items-center justify-center px-4 py-8 sm:px-8 lg:px-12"
    >
      <div className="w-full max-w-5xl text-center">
        <h2
          className="mx-auto max-w-[16ch] text-4xl font-semibold leading-[1.04] text-white sm:text-5xl lg:text-6xl"
        >
          ${d("chat.heroTitle")}
        </h2>
        <p
          className="mx-auto mt-4 max-w-[64ch] text-base leading-relaxed text-iron-300"
        >
          ${d("chat.heroDesc")}
        </p>
      </div>

      <div className="mt-9 w-full max-w-5xl">
        <${Nc}
          onSend=${t}
          disabled=${a}
          initialText=${n}
          resetKey=${r}
          draftKey=${s}
          variant="hero"
          context=${i}
          statusText=${o}
          canCancel=${u}
          onCancel=${c}
        />
      </div>

      <div className="mt-8 grid w-full max-w-5xl gap-2">
        ${f.map(m=>l`
            <button
              type="button"
              key=${m.title}
              onClick=${()=>e(m.title)}
              className="v2-button group grid grid-cols-[auto_1fr_auto] items-center gap-3 border-t border-white/10 px-2 py-4 text-left hover:border-signal/35"
            >
              <span
                className="grid h-8 w-8 place-items-center rounded-full border border-white/10 bg-white/[0.035] text-iron-300 group-hover:border-signal/35 group-hover:text-signal"
              >
                <${D} name=${m.icon} className="h-4 w-4" />
              </span>
              <span className="min-w-0">
                <span className="block text-sm font-semibold text-iron-100">
                  ${m.title}
                </span>
                <span className="mt-0.5 block text-sm text-iron-300">
                  ${m.detail}
                </span>
              </span>
            </button>
          `)}
      </div>
    </div>
  `}var NA=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function h1({open:e,onClose:t}){let a=k();return e?l`
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
          ${NA.map((n,r)=>l`
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
  `:null}function g1(e){let t=0,a=0,n=0,r=0;for(let i of e){if(i.role==="thinking"&&(t+=1),i.role==="tool_activity"){let o=v1([i]);a+=o.tools,n+=o.failed,r+=o.running}if(_A(i)){let o=v1(i.toolCalls);a+=o.tools,n+=o.failed,r+=o.running}}let s=[];return t&&s.push(`${t} reasoning`),a&&s.push(`${a} ${a===1?"tool":"tools"}`),n&&s.push(`${n} failed`),!n&&r&&s.push("running"),{hasError:n>0,label:`Activity${s.length?` - ${s.join(", ")}`:""}`}}function v1(e){let t=0,a=0;for(let n of e)n.toolStatus==="error"&&(t+=1),n.toolStatus==="running"&&(a+=1);return{tools:e.length,failed:t,running:a}}function _A(e){return e.toolCalls&&e.toolCalls.length>0}var y1=!1;function kA(){y1||!window.DOMPurify||(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),y1=!0)}function b1(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}kA();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var Qp=360;function RA(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let u=s("Copy");if(u.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),u.textContent="Copied",Xs("Code copied",{tone:"success"}),setTimeout(()=>u.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(u),n.appendChild(r),t.scrollHeight>Qp){t.style.maxHeight=`${Qp}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${Qp}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function CA({content:e,className:t=""}){let a=h.default.useRef(null),n=h.default.useMemo(()=>b1(e),[e]);return h.default.useEffect(()=>{RA(a.current)},[n]),l`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var ra=h.default.memo(CA);var x1={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",error:"bg-[var(--v2-danger-text)]"},EA={success:"ok",error:"err",running:"run"},AA=2;function ti({activity:e}){return e.toolCalls&&e.toolCalls.length>0?l`<${DA} tools=${e.toolCalls} />`:l`<${MA} activity=${e} />`}function TA(e,t){let a=0,n=0,r=0,s=0;for(let u of t){let c=String(u.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function DA({tools:e}){let t=k(),a=e.some(i=>i.toolStatus==="error"),[n,r]=h.default.useState(a);if(h.default.useEffect(()=>{a&&r(!0)},[a]),e.length<=AA)return l`
      <div className="flex flex-col gap-3">
        ${e.map((i,o)=>l`<${ti}
            key=${i.id||i.callId||`${i.toolName}-${o}`}
            activity=${i}
          />`)}
      </div>
    `;let s=TA(t,e);return l`
    <div className="flex flex-col">
      <button
        type="button"
        onClick=${()=>r(i=>!i)}
        aria-expanded=${n?"true":"false"}
        className=${["v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",a?"text-[var(--v2-danger-text)]":"text-iron-400 hover:text-iron-200"].join(" ")}
      >
        <${D} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${s}</span>
        <${D}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0",n?"rotate-180":""].join(" ")}
        />
      </button>

      ${n&&l`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((i,o)=>l`<${ti}
              key=${i.id||i.callId||`${i.toolName}-${o}`}
              activity=${i}
            />`)}
        </div>
      `}
    </div>
  `}function MA({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:u}=e,[c,d]=h.default.useState(n==="error");h.default.useEffect(()=>{n==="error"&&d(!0)},[n]);let f=x1[n]||x1.running,m=i!=null,p=h.default.useId(),b=l`
    <button
      type="button"
      onClick=${()=>d(y=>!y)}
      aria-expanded=${c?"true":"false"}
      aria-controls=${p}
      className="v2-button flex w-full items-center gap-2.5 border-0 border-b border-iron-700/40 bg-transparent px-1 py-2 text-left text-sm"
    >
      <span className=${["h-2 w-2 shrink-0 rounded-full",f].join(" ")} />
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-wide text-iron-300"
        >${EA[n]||"run"}</span
      >
      <span className="shrink-0 truncate font-mono text-[13px] font-medium text-iron-100"
        >${a}</span
      >
      ${r&&l`<span className="min-w-0 truncate font-mono text-xs text-iron-400"
        >${r}</span
      >`}
      <span className="ml-auto flex shrink-0 items-center gap-2">
        ${m&&l`<span className="font-mono text-[11px] text-iron-300">${i}ms</span>`}
        <${D}
          name="chevron"
          className=${["h-3.5 w-3.5 text-iron-400",c?"rotate-180":""].join(" ")}
        />
      </span>
    </button>
  `;return l`
    <div className=${t?"":"flex gap-3"}>
      ${!t&&l`
        <div
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
        >
          <${D} name="tool" className="h-4 w-4" />
        </div>
      `}
      <div className=${t?"min-w-0 flex-1":"min-w-0 max-w-[85%] flex-1"}>
        ${b}
        ${c&&l`<${OA}
          controlsId=${p}
          toolDetail=${r}
          toolParameters=${o}
          toolResultPreview=${u}
          toolError=${s}
          toolStatus=${n}
          toolDurationMs=${m?i:null}
        />`}
      </div>
    </div>
  `}function OA({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=k(),u=h.default.useMemo(()=>{let m=[];return r&&m.push({id:"error",label:o("tool.tabError")}),t&&m.push({id:"details",label:o("tool.tabDetails")}),a&&m.push({id:"params",label:o("tool.tabParameters")}),n&&m.push({id:"result",label:o("tool.tabResult")}),m},[o,r,t,a,n]),[c,d]=h.default.useState(null),f=c&&u.some(m=>m.id===c)?c:u[0]?.id;return h.default.useEffect(()=>{r&&d("error")},[r]),u.length===0?l`
      <div
        id=${e}
        className="rounded-b-lg border-x border-b border-iron-700/40 bg-iron-950 px-3 py-2 font-mono text-xs text-iron-400"
      >
        ${o("tool.noDetail")}
      </div>
    `:l`
    <div
      id=${e}
      className="rounded-b-lg border-x border-b border-iron-700/40 bg-iron-950"
    >
      <div className="flex items-center gap-1 border-b border-iron-700/40 px-2 pt-1.5">
        ${u.map(m=>l`
            <button
              type="button"
              key=${m.id}
              onClick=${()=>d(m.id)}
              className=${["v2-button rounded-t-md px-2.5 py-1 font-mono text-[11px]",f===m.id?"bg-iron-900 text-iron-100":"text-iron-400 hover:text-iron-200"].join(" ")}
            >
              ${m.label}
            </button>
          `)}
        <span className="ml-auto px-1 py-1 font-mono text-[10px] text-iron-500">
          ${o(s==="error"?"tool.exitError":s==="running"?"tool.exitRunning":"tool.exitOk")}${i!==null?` \xB7 ${i}ms`:""}
        </span>
      </div>
      <div className="p-3 text-xs">
        ${f==="details"&&l`<div className="whitespace-pre-wrap text-iron-200">${t}</div>`}
        ${f==="params"&&l`<pre className="overflow-x-auto rounded bg-iron-900 p-2 font-mono text-iron-100">${a}</pre>`}
        ${f==="result"&&l`<${LA} text=${n} />`}
        ${f==="error"&&l`<pre className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-danger-text)]">${r}</pre>`}
      </div>
    </div>
  `}function LA({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return l`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(PA)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return l`
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
                  >${UA(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?l`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:l`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function PA(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function UA(e){return e==null?"":String(e)}function $1({activity:e}){let t=g1(e),a=qA(e),[n,r]=h.default.useState(a);return h.default.useEffect(()=>{a&&r(!0)},[a]),l`
    <div className="mr-auto flex w-full max-w-[85%] flex-col">
      <button
        type="button"
        onClick=${()=>r(s=>!s)}
        aria-expanded=${n?"true":"false"}
        className=${["v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",t.hasError?"text-[var(--v2-danger-text)]":"text-iron-400 hover:text-iron-200"].join(" ")}
      >
        <${D} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${t.label}</span>
        <${D}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0",n?"rotate-180":""].join(" ")}
        />
      </button>

      ${n&&l`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((s,i)=>l`
            <${jA}
              key=${s.id||`${s.role||"activity"}-${i}`}
              item=${s}
            />
          `)}
        </div>
      `}
    </div>
  `}function jA({item:e}){if(e.role==="thinking")return l`<${FA} content=${e.content} />`;if(e.role==="tool_activity"||Vp(e)){let t=Vp(e)?{id:e.id,toolCalls:e.toolCalls}:e;return l`<${ti} activity=${t} />`}return null}function FA({content:e}){return e?l`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <${D} name="spark" className="h-4 w-4" />
      </div>
      <div className="min-w-0 max-w-[85%] flex-1 border-l-2 border-white/10 pl-3 text-iron-300">
        <${ra} content=${e} className="text-[13px]" />
      </div>
    </div>
  `:null}function Vp(e){return e?.toolCalls&&e.toolCalls.length>0}function qA(e){return(e||[]).some(t=>t?.role==="thinking"||t?.toolStatus==="running"||t?.toolStatus==="error"?!0:Vp(t)?t.toolCalls.some(a=>a?.toolStatus==="running"||a?.toolStatus==="error"):!1)}function _c(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function zA({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=h.default.useState(()=>t&&e.preview_url||null);return h.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return rc(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?l`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:l`<${D} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var w1="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",S1="px-3 py-2";function kc({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=h.default.useState(!1),o=h.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await Ra(e.fetch_url);_c(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),u=l`
    <${zA} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?l`<div
      className=${`${w1} ${S1} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${u}
    </div>`:l`<div className=${`${w1} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${S1} text-left transition-colors hover:bg-iron-900/80`}
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
      <${D} name="download" className="h-3.5 w-3.5" />
    </button>`}
  </div>`}var N1={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function ai({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return h.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),h.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?l`
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
        className=${V("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",N1[n]??N1.md,r)}
      >
        ${a?l`<${Gp} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function Gp({children:e,onClose:t,className:a=""}){return l`
    <div
      className=${V("flex shrink-0 items-center justify-between gap-4","px-5 py-4 md:px-7 md:py-5","border-b border-[var(--v2-panel-border)]",a)}
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
            <${D} name="close" className="h-4 w-4" />
          </button>
        `}
    </div>
  `}function ni({children:e,className:t=""}){return l`
    <div className=${V("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function ri({children:e,className:t=""}){return l`
    <div
      className=${V("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var _1=1e5;function Rc({attachment:e,onClose:t}){let a=!!e,[n,r]=h.default.useState("loading"),[s,i]=h.default.useState({}),o=e?Yx(e.mime_type):"download";if(h.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return Ra(e.fetch_url).then(async f=>{d=URL.createObjectURL(f);let m={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")m.dataUrl=await yp(f);else if(o==="pdf")m.frameUrl=d;else if(o==="text"){let p=await f.text();m.truncated=p.length>_1,m.text=m.truncated?p.slice(0,_1):p}if(c){URL.revokeObjectURL(d);return}i(m),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let u=e.filename||"attachment";return l`
    <${ai} open=${a} onClose=${t} size="xl">
      <${Gp} onClose=${t}>
        <span className="block truncate">${u}</span>
      <//>
      <${ni} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&l`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&l`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&l`<${BA} mode=${o} view=${s} filename=${u} />`}
      <//>
      <${ri}>
        ${s.downloadUrl&&l`<a
          href=${s.downloadUrl}
          download=${u}
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
  `}function BA({mode:e,view:t,filename:a}){switch(e){case"image":return l`<img
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
        <${D} name="file" className="h-10 w-10 text-signal" />
        <div className="text-sm">This file type can't be previewed.</div>
      </div>`}}var IA=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function HA(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function k1(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of HA(e).matchAll(IA)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function R1(e){return e.split("/").filter(Boolean).pop()||e}function C1(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function KA({threadId:e,path:t,onPreview:a}){let[n,r]=h.default.useState({mime_type:"",size_label:""});h.default.useEffect(()=>{let i=!0;return gx({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:C1(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:R1(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:nc({threadId:e,path:t})};return l`<${kc}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function E1({threadId:e,content:t}){let a=h.default.useMemo(()=>k1(t),[t]),[n,r]=h.default.useState(null);return!e||a.length===0?null:l`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>l`<${KA}
          key=${s}
          threadId=${e}
          path=${s}
          onPreview=${r}
        />`)}
      <${Rc}
        attachment=${n}
        onClose=${()=>r(null)}
      />
    </div>
  `}var A1={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function QA(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function VA({content:e}){let[t,a]=h.default.useState(!1);return e?l`
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
      ${t&&l`
        <div className="mt-1 border-l-2 border-white/10 pl-3 text-iron-300">
          <${ra} content=${e} className="text-[13px]" />
        </div>
      `}
    </div>
  `:null}function GA({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:u,status:c,error:d,toolCalls:f,timestamp:m}=e,p=n==="user",[b,y]=h.default.useState(!1),[$,g]=h.default.useState(null),v=h.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),y(!0),Xs("Copied to clipboard",{tone:"success"}),setTimeout(()=>y(!1),1400)}catch{}},[r]);if(n==="tool_activity"||f&&f.length>0){let A=f&&f.length>0?{id:e.id,toolCalls:f}:e;return l`<${ti} activity=${A} />`}if(n==="thinking")return l`<${VA} content=${r} />`;if(n==="image")return l`
      <div className="flex">
        <div className="flex flex-wrap gap-2">
          ${(o||[]).map((M,P)=>M.data_url?l`<img key=${P} src=${M.data_url} className="max-h-64 rounded-lg border border-iron-700 object-cover" alt="Generated result" />`:l`
                  <div key=${P} className="rounded-lg border border-iron-700 bg-iron-900/70 px-4 py-3 text-sm text-iron-200">
                    <div>Generated image unavailable in history payload</div>
                    ${M.path&&l`<div className="mt-1 font-mono text-xs text-iron-300">${M.path}</div>`}
                  </div>
                `)}
        </div>
      </div>
    `;let x=QA(m),w=(n==="assistant"||n==="user")&&!u,R=p?"max-w-[85%]":n==="system"||n==="error"?"mx-auto max-w-[85%]":"w-full max-w-[85%]",N=p?"":"w-full min-w-0 max-w-full";return l`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",p?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col gap-2",R].join(" ")}>
        <div
          className=${["text-base leading-7",N,A1[n]||A1.assistant,u?"opacity-70":""].join(" ")}
        >
          ${n==="assistant"||n==="system"||n==="error"?l`<${ra} content=${r} />`:l`<div className="whitespace-pre-wrap">${r}</div>`}

          ${c==="error"&&l`
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-300">
              <span>${d}</span>
            </div>
          `}

          ${s&&s.length>0&&l`
            <div className="mt-2 flex flex-wrap gap-2">
              ${s.map((A,M)=>l`<img key=${M} src=${A} className="max-h-48 rounded-lg border border-iron-700 object-cover" alt="Message attachment" />`)}
            </div>
          `}

          ${i&&i.length>0&&l`
            <div className="mt-2 flex flex-col gap-1.5">
              ${i.map((A,M)=>l`<${kc}
                key=${A.id||M}
                att=${A}
                onPreview=${g}
              />`)}
            </div>
            <${Rc}
              attachment=${$}
              onClose=${()=>g(null)}
            />
          `}

          ${n==="assistant"&&l`<${E1}
            threadId=${a}
            content=${typeof r=="string"?r:""}
          />`}
        </div>

        ${(w||c==="error"||x)&&l`
          <div
            className=${["flex items-center gap-1.5 px-1 text-iron-400 opacity-0 group-hover:opacity-100 focus-within:opacity-100",p?"justify-end":"justify-start"].join(" ")}
          >
            ${w&&l`
              <button
                type="button"
                onClick=${v}
                aria-label="Copy message"
                className="v2-button inline-flex items-center gap-1 rounded-md border-0 bg-transparent px-1.5 py-1 text-[11px] hover:text-iron-100"
              >
                <${D} name=${b?"check":"copy"} className="h-3.5 w-3.5" />
                ${b?"Copied":"Copy"}
              </button>
            `}
            ${c==="error"&&t&&l`
              <button
                type="button"
                onClick=${()=>t(e)}
                aria-label="Retry message"
                className="v2-button inline-flex items-center gap-1 rounded-md border-0 bg-transparent px-1.5 py-1 text-[11px] text-red-300 hover:text-red-200"
              >
                <${D} name="retry" className="h-3.5 w-3.5" />
                Retry
              </button>
            `}
            ${x&&l`<span className="font-mono text-[10px] text-iron-500">${x}</span>`}
          </div>
        `}
      </div>
    </div>
  `}var T1=h.default.memo(GA);function U1(e){let t=YA(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(j1(r)){let s=D1(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){M1(a,s),O1(a,r),n+=s.length;continue}}if(Yp(r)){let s=D1(t,n);M1(a,s),n+=s.length-1;continue}O1(a,r)}return a}function YA(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=Cc(i);o&&j1(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!Yp(i))continue;let o=Cc(i),u=o?t.get(o):void 0;if(u===void 0||u>=s)continue;let c=a.get(u)||[];c.push(i),a.set(u,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function D1(e,t){let a=t,n=Cc(e[t]);for(;a<e.length&&Yp(e[a])&&JA(n,e[a]);)a+=1;return e.slice(t,a)}function JA(e,t){let a=Cc(t);return!e||!a||a===e}function M1(e,t){if(t.length===0)return;let a=XA(t);e.push({type:"activity-run",id:`activity-run-${a[0].id}`,activity:a})}function O1(e,t){e.push({type:"message",id:t.id,message:t})}function j1(e){return e.role==="assistant"&&!F1(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function Yp(e){return e.role==="thinking"||e.role==="tool_activity"||F1(e)}function F1(e){return e?.toolCalls&&e.toolCalls.length>0}function Cc(e){return e?.turnRunId||null}function XA(e){return[...e].sort((t,a)=>t?.role!=="tool_activity"||a?.role!=="tool_activity"?0:ZA(t,a))}function ZA(e,t){if(Number.isFinite(e.activityOrder)&&Number.isFinite(t.activityOrder)){let n=e.activityOrder-t.activityOrder;if(n!==0)return n}let a=L1(P1(e.updatedAt||e.timestamp),P1(t.updatedAt||t.timestamp));return a!==0?a:L1(e.sequence,t.sequence)}function L1(e,t){let a=Number.isFinite(e)?e:null,n=Number.isFinite(t)?t:null;return a===null&&n===null?0:a===null?1:n===null?-1:a-n}function P1(e){if(!e)return null;let t=Date.parse(e);return Number.isFinite(t)?t:null}function q1({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let u=k(),c=h.default.useRef(null),d=h.default.useRef(!0),[f,m]=h.default.useState(!0);h.default.useEffect(()=>{if(!c.current||!d.current)return;let g=window.requestAnimationFrame(()=>{let v=c.current;v&&(v.scrollTop=v.scrollHeight)});return()=>window.cancelAnimationFrame(g)},[e,i]);let p=h.default.useCallback(()=>{let $=c.current;if(!$)return;let g=100,v=$.scrollHeight-$.scrollTop-$.clientHeight;d.current=v<g,m(v<g),a&&$.scrollTop<g&&n&&!t&&n()},[a,n,t]),b=h.default.useCallback(()=>{let $=c.current;$&&($.scrollTop=$.scrollHeight,d.current=!0,m(!0))},[]),y=h.default.useMemo(()=>U1(e),[e]);return l`
    <div className="relative flex min-h-0 min-w-0 flex-1">
    <div
      ref=${c}
      onScroll=${p}
      className="flex min-w-0 flex-1 overflow-y-auto px-4 pt-6 pb-14 sm:px-5 lg:px-8"
    >
      <div className="mx-auto flex w-full min-w-0 max-w-5xl flex-col gap-5">
        ${a&&l`
          <div className="text-center">
            <button
              onClick=${n}
              disabled=${t}
              className="v2-button rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-300 hover:border-signal/35 hover:text-white disabled:opacity-50"
            >
              ${u(t?"chat.history.loading":"chat.history.loadOlder")}
            </button>
          </div>
        `}
        ${y.map($=>$.type==="activity-run"?l`<${$1} key=${$.id} activity=${$.activity} />`:l`<${T1}
                key=${$.id}
                message=${$.message}
                onRetry=${r}
                threadId=${s}
              />`)}
        ${o}
      </div>
    </div>
    ${!f&&l`
      <button
        type="button"
        onClick=${b}
        aria-label=${u("chat.jumpToLatest")}
        className="absolute bottom-4 left-1/2 inline-flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-3 py-1.5 text-xs font-medium text-[var(--v2-text-strong)] shadow-[0_10px_30px_-12px_rgba(0,0,0,0.7)] hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]"
      >
        <${D} name="arrowDown" className="h-3.5 w-3.5" />
        ${u("chat.jumpToLatest")}
      </button>
    `}
    </div>
  `}function z1({notice:e,onRecover:t}){return l`
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
  `}function B1({suggestions:e,onSelect:t}){return!e||e.length===0?null:l`
    <div className="px-4 pb-3 sm:px-5 lg:px-8">
      <div className="mx-auto flex max-w-5xl flex-wrap gap-2">
        ${e.map(a=>l`
            <button
              key=${a}
              onClick=${()=>t(a)}
              className="v2-button rounded-full border border-white/10 bg-white/[0.035] px-3 py-1.5 text-xs text-iron-100 hover:border-signal/40 hover:text-signal"
            >
              ${a}
            </button>
          `)}
      </div>
    </div>
  `}function I1(){return l`
    <div className="flex flex-col items-start">
      <div className="flex min-w-0 max-w-[85%] flex-col gap-2">
        <div
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
  `}function Ec(){return H("/api/webchat/v2/channels/connectable")}function H1(e,t){if(!Jp(e))return null;let a=Ac(e),n=aT(a),r=null;for(let s of t||[]){if(!tT(s))continue;let i=nT(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function Jp(e){let t=Ac(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function WA(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function eT(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>K1(Ac(n))):a}function tT(e){return e?.strategy!=="admin_managed_channels"}function aT(e){return Q1(e,"slack")&&K1(e)}function K1(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function Ac(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function nT(e,t,a={}){return(a.commandAliasesOnly?eT(t,{channelManagementOnly:!0}):WA(t)).reduce((r,s)=>{let i=Ac(s);return Q1(e,i)?Math.max(r,i.length):r},0)}function Q1(e,t){return t?` ${e} `.includes(` ${t} `):!1}function V1(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n=a?rT(a):[],r={kind:"gate",runId:t.turn_run_id,gateRef:t.gate_ref,invocationId:t.invocation_id||null,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return a?{...r,toolName:a.tool_name||null,description:a.reason||t.body,actionLabel:a.action?.label||null,destination:a.destination||null,approvalScope:a.scope||null,approvalDetails:n,parameters:n.length?n.map(s=>`${s.label}: ${s.value}`).join(`
`):null}:r}return e==="auth_required"?{kind:"auth_required",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function rT(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function G1({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function Y1(){return{terminalByInvocation:new Map}}function J1(e){e?.current?.terminalByInvocation?.clear()}function X1(e,t,a){let n=W1(t,{toolStatus:"running"});n&&si(e,n,a,{matchGate:!0})}function Z1(e,t,a,n="authorization"){let r=W1(t,{toolStatus:"error",toolError:n});r&&si(e,r,a,{matchGate:!0})}function si(e,t,a,n={}){if(!t)return;let r=cT(t);r=uT(r,a),e(s=>{let i=e2(r),o=sT(s,r,i,n);if(o>=0){let c=[...s];return c[o]=oT(c[o],r),Xp(c[o],a),c}let u={id:i,role:"tool_activity",...r};return Xp(u,a),[...s,u]})}function W1(e,t={}){if(!e?.runId||!e?.gateRef||e.kind!=="gate"||!e.toolName)return null;let a=e.invocationId||`gate:${e.runId}:${e.gateRef}`;return{invocationId:a,callId:a,capabilityId:e.toolName,toolName:Lr(e.toolName)||e.toolName,toolStatus:t.toolStatus||"running",toolDetail:null,toolParameters:null,toolResultPreview:null,toolError:t.toolError||null,toolDurationMs:null,updatedAt:t.updatedAt||new Date().toISOString(),resultRef:null,truncated:!1,outputBytes:null,outputKind:null,turnRunId:e.runId,gateRef:e.gateRef,gateActivity:!0}}function e2(e){return`tool-${e.invocationId}`}function sT(e,t,a,n){let r=e.findIndex(i=>i?.id===a);if(r>=0)return r;let s=t.gateRef||null;if(s){let i=e.findIndex(o=>o?.role==="tool_activity"&&o.turnRunId===t.turnRunId&&o.gateRef===s);if(i>=0)return i}if(!n.matchGate&&!t.gateActivity){let i=e.findIndex(o=>iT(o,t));if(i>=0)return i}if(n.matchGate||t.gateActivity){let i=e.findIndex(o=>o?.role==="tool_activity"&&!o.gateRef&&o.gateActivity!==!0&&!Vs(o.toolStatus)&&o.turnRunId===t.turnRunId&&t2(o.toolName,t.toolName));if(i>=0)return i}return-1}function iT(e,t){return e?.role==="tool_activity"&&e.gateActivity===!0&&e.turnRunId===t.turnRunId&&t2(e.toolName,t.toolName)}function oT(e,t){let a=Vs(e.toolStatus),n=Vs(t.toolStatus),r=a&&!n,s={...e,...t,id:e.id,role:"tool_activity",invocationId:e.gateActivity&&!t.gateActivity?t.invocationId:e.invocationId||t.invocationId,callId:e.gateActivity&&!t.gateActivity?t.callId:e.callId||t.callId,toolName:t.toolName||e.toolName,toolStatus:r?e.toolStatus:t.toolStatus,toolError:t.toolError||e.toolError,updatedAt:r?e.updatedAt||t.updatedAt:t.updatedAt||e.updatedAt,turnRunId:t.turnRunId||e.turnRunId||null,gateRef:t.gateRef||e.gateRef||null,gateActivity:e.gateActivity&&t.gateActivity,capabilityId:t.capabilityId||e.capabilityId||null,activityOrder:lT(e,t),activityOrderSource:t.activityOrderSource||e.activityOrderSource||null};return e.gateActivity&&!t.gateActivity&&(s.id=e2(t),s.gateActivity=!1),s}function lT(e,t){return Number.isFinite(t.activityOrder)?t.activityOrder:e.activityOrder}function uT(e,t){if(!e?.invocationId)return e;if(Vs(e.toolStatus))return Xp(e,t),e;let a=t?.current?.terminalByInvocation?.get(e.invocationId);return a?Number.isFinite(e.activityOrder)?{...a,activityOrder:e.activityOrder,activityOrderSource:e.activityOrderSource||a.activityOrderSource||null}:a:e}function Xp(e,t){!e?.invocationId||!Vs(e.toolStatus)||t?.current?.terminalByInvocation?.set(e.invocationId,e)}function t2(e,t){return!e||!t?!1:Lr(e)===Lr(t)}function cT(e){let t=Lr(e.toolName||e.capabilityId);return{...e,toolName:t||e.toolName||"tool"}}function s2({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o,onRunSettled:u}){let c=h.default.useRef(new Set),d=h.default.useRef(null),f=h.default.useRef(null);return h.default.useCallback(m=>{let{type:p,frame:b}=m||{};if(!(!p||!b))switch(p){case"accepted":{let y=b.ack||{};y.run_id&&(d.current=y.run_id),r?.({runId:y.run_id||null,threadId:y.thread_id||e,status:y.status||null}),a(!0);return}case"running":case"capability_progress":{let y=b.progress||{};y.turn_run_id&&(d.current=y.turn_run_id,r?.($=>$&&$.runId===y.turn_run_id?$:{runId:y.turn_run_id,threadId:e,status:"running"}),mT(n,y.turn_run_id,f)),a(!0);return}case"capability_activity":{let y=b.activity;if(!y||!y.invocation_id)return;si(t,_p(y),o);return}case"capability_display_preview":{let y=b.preview;if(!y||!y.invocation_id)return;let $=Np(y);si(t,$,o);return}case"gate":case"auth_required":{let y=V1(p,b.prompt);y&&(X1(t,y,o),n(y),r?.({runId:y.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let y=b.reply||{};t($=>[...$,{id:`reply-${y.turn_run_id||Date.now()}`,role:"assistant",content:y.text||"",timestamp:y.generated_at||new Date().toISOString(),turnRunId:y.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let y=b.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Tc(c,u,y,!1);return}case"failed":{let y=b.run_state||{},$=y.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),eh(t,{runId:$,status:y.status||"failed",failureCategory:hT(y),failureSummary:null}),Tc(c,u,$,!1);return}case"projection_snapshot":case"projection_update":{let y=b.state?.items||[];fT({items:y,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:u,settledRunsRef:c,latestRunIdRef:d,promptRunIdRef:f,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i,o,u])}function Tc(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var dT=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),a2=new Set(["completed","succeeded"]),Zp=new Set(["blocked_auth","blocked_approval","blocked_resource"]);function n2(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function mT(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function fT({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,activeRunRef:d,locallyResolvedGatesRef:f,toolActivityStateRef:m}){let p=u?.current??null;for(let b of e){if(b.run_status){let{run_id:y,status:$,failure_category:g,failure_summary:v}=b.run_status,x=dT.has($),w=d?.current?.source==="local"?d.current.runId:null,S=!!(y&&w&&w!==y),R=p??u?.current??null,N=!!(x&&y&&R&&R!==y),A=y&&Zp.has($)?r2(f,y):null;if(S)continue;if(N){r2(f,d?.current?.runId)?.outcome==="resumed"&&(pT({runId:y,activePromptRunId:d?.current?.runId,success:a2.has($),status:$,failureCategory:g,failureSummary:v,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,locallyResolvedGatesRef:f}),p=null);continue}if(A){n2(r,y,c),A.outcome==="resumed"?(n(!0),s?.(M=>M&&M.runId===y?{...M,status:M.status==="awaiting_gate"?"queued":M.status||"queued"}:{runId:y,threadId:t,status:"queued"}),p=y,u&&(u.current=y)):(n(!1),d?.current?.runId===y&&s?.(null),p=null,u?.current===y&&(u.current=null));continue}y&&(p=y,!x&&u&&(u.current=y),s?.(M=>M&&M.runId===y?{...M,status:$}:{runId:y,threadId:t,status:$})),y&&Zp.has($)?c&&(c.current=y):y&&c?.current===y&&(c.current=null),x?(n(!1),r(null),s?.(null),Wp(f,y),p=null,u&&(u.current=null),y&&c?.current===y&&(c.current=null),Tc(o,i,y,a2.has($)),($==="failed"||$==="recovery_required")&&eh(a,{runId:y,status:$,failureCategory:g,failureSummary:v})):Zp.has($)||(n2(r,y,c),Wp(f,y),n(!0))}if(b.text){let y=`text-${b.text.id}`;a($=>{let g=$.findIndex(x=>x.id===y),v={id:y,role:"assistant",content:b.text.body||"",timestamp:new Date().toISOString(),isFinalReply:!0};if(g>=0){let x=[...$];return x[g]=v,x}return[...$,v]}),n(!1)}if(b.thinking){let y=`thinking-${b.thinking.id}`;a($=>{let g=$.findIndex(x=>x.id===y),v={id:y,role:"thinking",content:b.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:b.thinking.run_id||null};if(g>=0){let x=[...$];return x[g]=v,x}return[...$,v]})}if(b.capability_activity){let y=b.capability_activity;y.invocation_id&&si(a,_p(y),m)}if(b.gate&&p&&c?.current===p&&!gT(f,p,b.gate.gate_ref)&&(r(y=>y||{kind:"gate",runId:p,gateRef:b.gate.gate_ref,headline:b.gate.headline,body:"",allowAlways:b.gate.allow_always===!0}),n(!1)),b.skill_activation){let{id:y,skill_names:$=[],feedback:g=[]}=b.skill_activation;if($.length||g.length){let v=`skill-${y||$.join("-")||"activation"}`,x=[$.length?`Skill activated: ${$.join(", ")}`:"",...g].filter(Boolean).join(`
`);a(w=>w.some(S=>S.id===v)?w:[...w,{id:v,role:"system",content:x,timestamp:new Date().toISOString()}])}}}u&&p&&(u.current=p)}function pT({runId:e,activePromptRunId:t,success:a,status:n,failureCategory:r,failureSummary:s,setMessages:i,setIsProcessing:o,setPendingGate:u,setActiveRun:c,onRunSettled:d,settledRunsRef:f,latestRunIdRef:m,promptRunIdRef:p,locallyResolvedGatesRef:b}){o(!1),u(null),c?.(null),Wp(b,t),m&&(m.current=null),p?.current===t&&(p.current=null),Tc(f,d,e,a),(n==="failed"||n==="recovery_required")&&eh(i,{runId:e,status:n,failureCategory:r,failureSummary:s})}function hT(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function eh(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`;e(i=>{let o=i.findIndex(c=>c.id===s),u=G1({status:a,failureCategory:n,failureSummary:r});if(o>=0){if(!r||i[o].content===u)return i;let c=[...i];return c[o]={...c[o],content:u},c}return[...i,{id:s,role:"error",content:u,timestamp:new Date().toISOString()}]})}function r2(e,t){if(!t)return null;let a=e?.current;if(!a)return null;for(let[n,r]of a.entries())if(n.startsWith(`${t}
`))return vT(r);return null}function vT(e){return e&&typeof e=="object"?{resolution:e.resolution||null,outcome:e.outcome||null}:{resolution:e||null,outcome:null}}function Wp(e,t){if(!t)return;let a=e?.current;if(a)for(let n of Array.from(a.keys()))n.startsWith(`${t}
`)&&a.delete(n)}function gT(e,t,a){return!t||!a?!1:!!e?.current?.has(`${t}
${a}`)}function i2(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function o2(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function l2(e,t,a,n){let r=bT(n);return r?(yT(e,t,a,{timelineMessageId:r}),r):null}function yT(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function bT(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var xT=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function u2({threadId:e,onEvent:t,enabled:a}){let[n,r]=h.default.useState("idle"),s=h.default.useRef(t);s.current=t;let i=h.default.useRef(null);return h.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,u=null,c=0,d=3e4;function f(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=Dx({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let y=Math.min(1e3*2**c,d);u=setTimeout(f,y)};let b=(y,$)=>{let g=null;try{g=JSON.parse(y.data)}catch{return}!g||typeof g!="object"||(y.lastEventId&&(i.current=y.lastEventId),s.current?.({type:g.type||$,frame:g,lastEventId:y.lastEventId||null}))};o.onmessage=y=>b(y,"message");for(let y of xT)o.addEventListener(y,$=>b($,y))}function m(){u&&(clearTimeout(u),u=null),o&&(o.close(),o=null),r("paused")}function p(){document.visibilityState==="hidden"?m():o||f()}return f(),document.addEventListener("visibilitychange",p),()=>{document.removeEventListener("visibilitychange",p),u&&clearTimeout(u),o&&o.close()}},[a,e]),{status:n}}var $T=3e4,wT="credential_stored_gate_resolution_failed",ST="ironclaw-product-auth",th="ironclaw:product-auth:oauth-complete",NT="ironclaw:product-auth:oauth-complete";async function c2(e){let t=new AbortController,a=setTimeout(()=>t.abort(),$T);try{return await e(t.signal)}finally{clearTimeout(a)}}function _T(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=wT,t.cause=e,t}function kT(e){let a=Tt.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function RT(e){return e?.continuation?.type==="turn_gate_resume"}function CT(e){if(e?.outcome)return e.outcome;let t=String(e?.status||"").toLowerCase();return t==="queued"||t==="running"?"resumed":t==="cancelled"||e?.already_terminal===!0?"cancelled":e?.already_terminal===!1?"resumed":null}function d2(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function ET(e){return e?.type===NT&&e?.status==="completed"}function AT(e,t,a){if(!ET(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function ah(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function TT(e){if(!Jp(e))return null;try{let a=(await Tt.fetchQuery({queryKey:["connectable-channels"],queryFn:Ec}))?.channels||[];return H1(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function m2(e){let t=h.default.useRef(new Map),a=h.default.useRef(1),[n,r]=h.default.useState(0),[s,i]=h.default.useState(Date.now()),[o,u]=h.default.useState(null),c=h.default.useRef(o),d=h.default.useCallback(J=>{let I=typeof J=="function"?J(c.current):J;c.current=I,u(I)},[]);h.default.useEffect(()=>{c.current=o},[o]);let[f,m]=h.default.useState(null),p=h.default.useCallback(()=>t.current.get(e||"__new__")||[],[e]),b=h.default.useCallback(J=>{let I=e||"__new__";J.length>0?t.current.set(I,J):t.current.delete(I)},[e]),{messages:y,hasMore:$,nextCursor:g,isLoading:v,loadError:x,loadHistory:w,setMessages:S}=s$(e,{getPendingMessages:p,setPendingMessages:b}),[R,N]=h.default.useState(!1),[A,M]=h.default.useState(null),[P,G]=h.default.useState(e),C=h.default.useRef(Y1()),Y=h.default.useRef(new Map),K=h.default.useRef({gateKey:null,credentialRef:null,inFlight:!1});P!==e&&(G(e),N(!1),M(null),u(null),m(null)),h.default.useEffect(()=>{J1(C),Y.current.clear()},[e]);let oe=Math.max(0,Math.ceil((n-s)/1e3)),Se=A?.runId&&A?.gateRef?`${A.runId}
${A.gateRef}`:null;h.default.useEffect(()=>{if(!n)return;let J=setInterval(()=>i(Date.now()),250);return()=>clearInterval(J)},[n]),h.default.useEffect(()=>{K.current.gateKey!==Se&&(K.current={gateKey:Se,credentialRef:null,inFlight:!1})},[Se]),h.default.useEffect(()=>{if(!d2(A))return;let J=Date.now(),I=Qe=>{AT(Qe,A,J)&&(M(qe=>d2(qe)?null:qe),N(!0))},fe=null;typeof window.BroadcastChannel=="function"&&(fe=new window.BroadcastChannel(ST),fe.onmessage=Qe=>I(Qe.data));let be=Qe=>{Qe.key===th&&I(ah(Qe.newValue))};window.addEventListener("storage",be),I(ah(window.localStorage?.getItem?.(th)));let st=window.setInterval(()=>{I(ah(window.localStorage?.getItem?.(th)))},500);return()=>{window.clearInterval(st),fe&&fe.close(),window.removeEventListener("storage",be)}},[A]);let Be=s2({threadId:e,setMessages:S,setIsProcessing:N,setPendingGate:M,setActiveRun:d,activeRunRef:c,locallyResolvedGatesRef:Y,toolActivityStateRef:C,onRunSettled:(J,{success:I})=>{I&&b([]),w(void 0,{preserveClientOnly:!0})}}),{status:vt}=u2({threadId:e,onEvent:Be,enabled:!!e}),Ze=h.default.useCallback(async(J,I={})=>{let{threadId:fe,attachments:be=[]}=I,st=be.map(Xx),Qe=be.map(Zx);if(be.length===0){let Re=await TT(J);if(Re)return m(Re),{channel_connect_action:Re}}m(null);let qe=fe||e;if(!qe){let Re=await ac();if(Tt.invalidateQueries({queryKey:["threads"]}),qe=Re?.thread?.thread_id,!qe)throw new Error("createThread returned no thread_id")}let $a=qe,Ot={id:`pending-${a.current++}`,role:"user",content:J,attachments:Qe,timestamp:new Date().toISOString(),isOptimistic:!0};i2(t.current,$a,Ot);let wa=Ot.id;S(Re=>[...Re,{id:wa,role:"user",content:J,attachments:Qe,timestamp:Ot.timestamp,isOptimistic:!0}]),N(!0),M(null);try{let Re=await Ex({threadId:qe,content:J,attachments:st});kT(qe)&&Tt.invalidateQueries({queryKey:["threads"]}),Re?.run_id&&d({runId:Re.run_id,threadId:Re.thread_id||qe,status:Re.status||null,source:"local"});let ne=l2(t.current,$a,wa,Re?.accepted_message_ref);return ne&&S(Ce=>Ce.map(wt=>wt.id===wa?{...wt,timelineMessageId:ne}:wt)),Re?.outcome==="rejected_busy"&&(S(Ce=>Ce.map(wt=>wt.id===wa?{...wt,isOptimistic:!1,status:"error"}:wt)),Re?.notice&&S(Ce=>[...Ce,{id:`system-rejected-${a.current++}`,role:"system",content:Re.notice,timestamp:new Date().toISOString(),isOptimistic:!1}]),N(!1)),Re}catch(Re){throw Re.status===429&&r(Date.now()+DT(Re)),S(ne=>ne.map(Ce=>Ce.id===wa?{...Ce,isOptimistic:!1,status:"error",error:Re.message}:Ce)),N(!1),Re}finally{o2(t.current,$a,wa)}},[e,S]),gt=h.default.useCallback(async(J,I={})=>{if(!A)return;let{runId:fe,gateRef:be}=A;if(!fe||!be)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");let st=await bp({threadId:e,runId:fe,gateRef:be,resolution:J,always:I.always,credentialRef:I.credentialRef}),Qe=CT(st);if(Y.current.set(`${fe}
${be}`,{resolution:J,outcome:Qe}),J==="denied"&&Qe==="resumed"&&Z1(S,A,C),M(null),Qe==="resumed"){N(!0),d({runId:st?.run_id||fe,threadId:st?.thread_id||e,status:st?.status||"queued"});return}N(!1),d(null)},[A,e,S,d]),xa=h.default.useCallback(async J=>{if(!A)throw new Error("auth gate is no longer pending");let{runId:I,gateRef:fe,provider:be}=A;if(!I||!fe||!be)throw new Error("auth gate is missing required credential metadata");let st=A.accountLabel||`${be} credential`,Qe=`${I}
${fe}`;if(K.current.gateKey!==Qe&&(K.current={gateKey:Qe,credentialRef:null,inFlight:!1}),K.current.inFlight)throw new Error("auth token submission already in progress");K.current.inFlight=!0;try{let qe=K.current.credentialRef,$a=null;if(!qe){if($a=await c2(Ot=>Ox({provider:be,accountLabel:st,token:J,threadId:e,runId:I,gateRef:fe,signal:Ot})),qe=$a?.credential_ref,!qe)throw new Error("manual token submit returned no credential_ref");K.current.credentialRef=qe}if(!RT($a))try{await c2(Ot=>bp({threadId:e,runId:I,gateRef:fe,resolution:"credential_provided",credentialRef:qe,signal:Ot}))}catch(Ot){throw _T(Ot)}K.current={gateKey:null,credentialRef:null,inFlight:!1},M(null),N(!0)}catch(qe){throw K.current.gateKey===Qe&&(K.current.inFlight=!1),qe}},[A,e]),Ta=h.default.useCallback(async J=>{let I=o?.runId;!I||!e||(M(null),N(!1),d(null),await Mx({threadId:e,runId:I,reason:J}))},[o,e]),oa=h.default.useCallback(()=>{$&&g&&w(g)},[$,g,w]),$t=h.default.useCallback(async(J,I,fe)=>{let be="approved",st=!1;I==="deny"?be="denied":I==="cancel"?be="cancelled":I==="always"&&(be="approved",st=!0),await gt(be,{always:st})},[gt]),se=h.default.useCallback(()=>{},[]);return{messages:y,isProcessing:R,pendingGate:A,channelConnectAction:f,activeRun:o,sseStatus:vt,historyLoading:v,historyLoadError:x,hasMore:$,cooldownSeconds:oe,send:Ze,resolveGate:gt,submitAuthToken:xa,cancelRun:Ta,loadMore:oa,dismissChannelConnectAction:()=>m(null),suggestions:[],setSuggestions:se,retryMessage:se,approve:$t,recoverHistory:se,recoveryNotice:null}}function DT(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function f2({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}var MT=1500;function p2({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let{messages:o,isProcessing:u,pendingGate:c,channelConnectAction:d,suggestions:f,sseStatus:m,historyLoading:p,historyLoadError:b,hasMore:y,cooldownSeconds:$,recoveryNotice:g,activeRun:v,send:x,cancelRun:w,retryMessage:S,approve:R,recoverHistory:N,loadMore:A,setSuggestions:M,submitAuthToken:P,dismissChannelConnectAction:G}=m2(t),C=h.default.useMemo(()=>e.find(se=>se.id===t)||null,[e,t]),Y=h.default.useMemo(()=>f2({gatewayStatus:i,activeThread:C}),[i,C]),K=o.length>0||u||!!c||!!d,oe=!p&&!K&&!b,Se=u&&!c||$>0,Be=$>0?`Retry in ${$}s`:void 0,vt=t||Fo,Ze=!!(t&&v?.runId&&v.threadId===t&&u&&!c),gt=h.default.useCallback(async(se,{images:J=[],attachments:I=[]}={})=>{let fe=await x(se,{images:J,attachments:I,threadId:t}),be=fe?.thread_id||t;return!t&&be&&a&&a(be,{replace:!0}),fe},[t,a,x]),xa=h.default.useCallback(async se=>{M([]),await gt(se)},[gt,M]),Ta=h.default.useCallback(()=>w("user_requested"),[w]);h.default.useEffect(()=>{if(!t)return;if(c){mc(t,bn.NEEDS_ATTENTION);return}if(u){mc(t,bn.RUNNING);return}let se=setTimeout(()=>cw(t),MT);return()=>clearTimeout(se)},[t,c,u]);let[oa,$t]=h.default.useState(!1);return h.default.useEffect(()=>{let se=J=>{if(J.key==="Escape"){$t(!1);return}if(J.key!=="?")return;let I=J.target,fe=I?.tagName;fe==="INPUT"||fe==="TEXTAREA"||I?.isContentEditable||(J.preventDefault(),$t(be=>!be))};return window.addEventListener("keydown",se),()=>window.removeEventListener("keydown",se)},[]),l`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${f1} status=${m} />

        ${b&&l`
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            ${b}
          </div>
        `}

        ${oe&&l`
          <${p1}
            onSuggestion=${xa}
            onSend=${gt}
            disabled=${Se}
            initialText=${r}
            resetKey=${s}
            draftKey=${vt}
            context=${Y}
            statusText=${Be}
            canCancel=${Ze}
            onCancel=${Ta}
          />
        `}
        ${!oe&&l`
          <${q1}
            messages=${o}
            isLoading=${p}
            hasMore=${y}
            onLoadMore=${A}
            onRetryMessage=${S}
            threadId=${t}
            pending=${u}
          >
            ${g&&l`
              <${z1}
                notice=${g}
                onRecover=${N}
              />
            `}
            ${u&&!c&&l`<${I1} />`}
            ${d&&l`
              <${c1}
                connectAction=${d}
                onDismiss=${G}
              />
            `}
            ${c&&(c.kind==="auth_required"?c.challengeKind==="oauth_url"?l`
                  <${o1}
                    gate=${c}
                    onCancel=${()=>R(c.requestId,"cancel",c.kind)}
                  />
                `:c.challengeKind==="manual_token"?l`
                  <${l1}
                    gate=${c}
                    onSubmit=${P}
                    onCancel=${()=>R(c.requestId,"cancel",c.kind)}
                  />
                `:l`
                  <${i1}
                    gate=${c}
                    onCancel=${()=>R(c.requestId,"cancel",c.kind)}
                  />
                `:l`
              <${s1}
                gate=${c}
                onApprove=${()=>R(c.requestId,"approve",c.kind)}
                onDeny=${()=>R(c.requestId,"deny",c.kind)}
                onAlways=${()=>R(c.requestId,"always",c.kind)}
              />
            `)}
          <//>

          <${B1}
            suggestions=${f}
            onSelect=${xa}
          />

          <${Nc}
            onSend=${gt}
            disabled=${Se}
            initialText=${r}
            resetKey=${s}
            draftKey=${vt}
            context=${Y}
            statusText=${Be}
            canCancel=${Ze}
            onCancel=${Ta}
          />
        `}
      </div>
      <${h1}
        open=${oa}
        onClose=${()=>$t(!1)}
      />
    </div>
  `}function nh(){let{threadsState:e,gatewayStatus:t}=Ka(),{threadId:a}=ut(),n=me(),r=je(),s=r.state?.composerDraft||"";h.default.useEffect(()=>{a&&a!==e.activeThreadId?e.setActiveThreadId(a):a||e.setActiveThreadId(null)},[a]);let i=h.default.useCallback((o,u={})=>{if(!o){e.setActiveThreadId(null),n("/chat",u);return}e.setActiveThreadId(o),n(`/chat/${o}`,u)},[e,n]);return l`
    <${p2}
      threads=${e.threads}
      activeThreadId=${e.activeThreadId}
      onSelectThread=${i}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function h2(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?Ys(e,t):"",model:e?cc(e,t):""}}function v2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u}){let[c,d]=h.default.useState(()=>h2(e,a)),[f,m]=h.default.useState(""),[p,b]=h.default.useState([]),[y,$]=h.default.useState(null),[g,v]=h.default.useState(""),x=h.default.useRef(!!e);h.default.useEffect(()=>{n&&(d(h2(e,a)),m(""),b([]),$(null),v(""),x.current=!!e)},[n,e,a]);let w=e?.builtin===!0,S=e&&!e.builtin,R=h.default.useCallback((G,C)=>{d(Y=>{let K={...Y,[G]:C};return G==="name"&&!x.current&&(K.id=G$(C)),K})},[]),N=h.default.useCallback(()=>!w&&(!c.name.trim()||!c.id.trim())?u("llm.fieldsRequired"):!w&&!Y$(c.id.trim())?u("llm.invalidId"):!S&&!w&&t.includes(c.id.trim())?u("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,w,S,u]),A=h.default.useCallback(async()=>{let G=N();if(G){$({tone:"error",text:G});return}v("save");try{await s({form:c,apiKey:f,provider:e}),r()}catch(C){$({tone:"error",text:C.message})}finally{v("")}},[f,c,r,s,e,N]),M=h.default.useCallback(async()=>{if(!c.model.trim()){$({tone:"error",text:u("llm.modelRequired")});return}v("test");try{let G=await i(Dp(e,c,f,a));$({tone:G.ok?"success":"error",text:G.message})}catch(G){$({tone:"error",text:G.message})}finally{v("")}},[f,a,c,i,e,u]),P=h.default.useCallback(async()=>{if((w?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){$({tone:"error",text:u("llm.baseUrlRequired")});return}v("models");try{let C=await o(Dp(e,c,f,a));if(!C.ok||!Array.isArray(C.models)||!C.models.length)$({tone:"error",text:C.message||u("llm.modelsFetchFailed")});else{b(C.models);let Y=J$(c.model,C.models);Y!==null&&R("model",Y),$({tone:"success",text:u("llm.modelsFetched",{count:C.models.length})})}}catch(C){$({tone:"error",text:C.message})}finally{v("")}},[f,a,c,w,o,e,u,R]);return{form:c,apiKey:f,models:p,message:y,busy:g,isBuiltin:w,isEditing:S,setApiKey:m,update:R,submit:A,runTest:M,fetchModels:P,markIdEdited:()=>{x.current=!0}}}function Dc({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let u=k(),c=v2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u});if(!n)return null;let{form:d,apiKey:f,models:m,message:p,busy:b,isBuiltin:y,isEditing:$}=c,g=y?u("llm.configureProvider",{name:e.name||e.id}):u($?"llm.editProvider":"llm.newProvider");return l`
    <${ai} open=${n} onClose=${r} title=${g} size="lg">
      <${ni} className="space-y-4">
        ${!y&&l`
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${u("llm.providerName")}
              <${Mt} value=${d.name} onChange=${v=>c.update("name",v.target.value)} />
            </label>
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${u("llm.providerId")}
              <${Mt}
                value=${d.id}
                disabled=${$}
                onChange=${v=>{c.markIdEdited(),c.update("id",v.target.value)}}
              />
            </label>
          </div>
          <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
            ${u("llm.adapter")}
            <${Kp} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${Tp.map(v=>l`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
            <//>
          </label>
        `}

        ${y&&l`
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            ${Bo(e.adapter)}
          </div>
        `}

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.baseUrl")}
          <${Mt} value=${d.baseUrl} placeholder=${e?.base_url||""} onChange=${v=>c.update("baseUrl",v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.apiKey")}
          <${Mt} type="password" value=${f} placeholder=${u("llm.apiKeyPlaceholder")} onChange=${v=>c.setApiKey(v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.defaultModel")}
          <div className="flex items-stretch gap-2">
            <${Mt} value=${d.model} onChange=${v=>c.update("model",v.target.value)} />
            <${E} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${b!==""} onClick=${c.fetchModels}>
              ${u(b==="models"?"llm.fetchingModels":"llm.fetchModels")}
            <//>
          </div>
        </label>

        ${m.length>0&&l`
          <${Kp} value=${d.model} onChange=${v=>c.update("model",v.target.value)}>
            ${m.map(v=>l`<option key=${v} value=${v}>${v}</option>`)}
          <//>
        `}

        ${p&&l`
          <div className=${p.tone==="error"?"text-sm text-red-200":"text-sm text-mint"} role="status">
            ${p.text}
          </div>
        `}
      <//>
      <${ri}>
        <${E} type="button" variant="secondary" disabled=${b!==""} onClick=${c.runTest}>
          ${u(b==="test"?"llm.testing":"llm.testConnection")}
        <//>
        <${E} type="button" variant="ghost" disabled=${b!==""} onClick=${r}>${u("common.cancel")}<//>
        <${E} type="button" disabled=${b!==""} onClick=${c.submit}>
          ${u(b==="save"?"common.saving":"common.save")}
        <//>
      <//>
    <//>
  `}function Mc({login:e}){let t=k(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return l`
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
  `}function OT(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function Oc({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=Js({settings:e,gatewayStatus:t}),[s,i]=h.default.useState(null),[o,u]=h.default.useState(!1),[c,d]=h.default.useState(null),f=h.default.useRef(null),m=h.default.useCallback((g,v)=>{f.current&&window.clearTimeout(f.current),d({tone:g,text:v}),f.current=window.setTimeout(()=>d(null),3500)},[]);h.default.useEffect(()=>()=>{f.current&&window.clearTimeout(f.current)},[]);let p=h.default.useCallback((g=null)=>{i(g),u(!0)},[]),b=h.default.useCallback(async g=>{try{await r.setActiveProvider(g),m("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(p(g),m("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):m("error",v.message)}},[p,r,m,n]),y=h.default.useCallback(async({form:g,apiKey:v,provider:x})=>{if(x?.builtin){await r.saveBuiltinProvider({provider:x,form:g,apiKey:v}),m("success",n("llm.providerConfigured",{name:x.name||x.id}));return}let w=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:x});m("success",n(x?"llm.providerUpdated":"llm.providerAdded",{name:w.name||w.id}))},[r,m,n]),$=h.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),m("success",n("llm.providerDeleted"))}catch(v){m("error",v.message)}},[r,m,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>OT(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:p,closeDialog:()=>u(!1),handleUse:b,handleSave:y,handleDelete:$}}var LT=3e5;function PT(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function UT(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function jT(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=u=>{let c=u.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},LT);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var FT=3e5,qT=9e5,zT=2e3;async function g2(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,zT)),(await uc().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function Lc({onSuccess:e}={}){let t=k(),a=Z(),[n,r]=h.default.useState(!1),[s,i]=h.default.useState(""),[o,u]=h.default.useState(!1),[c,d]=h.default.useState(""),[f,m]=h.default.useState(null),p=h.default.useCallback(()=>{i(""),d(""),m(null)},[]),b=h.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),y=h.default.useCallback(async v=>{if(p(),PT()){i(t("onboarding.nearaiLocalSso"));return}let x=window.open("about:blank","_blank");if(!x){i(t("onboarding.nearaiFailed"));return}try{x.opener=null}catch{}r(!0);try{let{auth_url:w}=await $$({provider:v,origin:window.location.origin});x.location.href=w;let S=await g2("nearai",FT,x);if(S==="active"){await b();return}x.close(),i(t(S==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{x.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[b,p,t]),$=h.default.useCallback(async()=>{p(),r(!0);try{let v=UT(),x=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!x){i(t("onboarding.nearaiFailed"));return}x.opener=null;let w=await jT(x,v);if(!w){i(t("onboarding.nearaiFailed"));return}await w$({account_id:w.accountId,public_key:w.publicKey,signature:w.signature,message:w.message,recipient:w.recipient,nonce:w.nonce}),await b()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[b,p,t]),g=h.default.useCallback(async()=>{p();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}u(!0);try{let{user_code:x,verification_uri:w}=await S$();m({userCode:x,verificationUri:w}),v&&(v.location.href=w);let S=await g2("openai_codex",qT,v);if(S==="active"){await b();return}v&&v.close(),d(t(S==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{u(!1)}},[b,p,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:f,startNearai:y,startNearaiWallet:$,startCodex:g}}var y2="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",BT="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",IT="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",HT="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",KT={nearai:{color:"#00ec97",path:BT},openai_codex:{color:"#10a37f",path:y2},openai:{color:"#10a37f",path:y2},anthropic:{color:"#d97757",path:IT},ollama:{color:null,path:HT}};function b2({id:e,name:t}){let a=KT[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return l`
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
  `}var QT=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function VT({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=h.default.useState(!1),o=h.default.useRef(null),u=t||a.nearaiBusy;h.default.useEffect(()=>{if(!s)return;let d=m=>{o.current&&!o.current.contains(m.target)&&i(!1)},f=m=>{m.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",f),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",f)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return l`
    <div ref=${o} className="relative shrink-0">
      <${E}
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
        <${D} name="chevron" className="h-3.5 w-3.5" />
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
  `}function GT({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let u=s(e.nameKey),c;return e.auth==="nearai"?c=l`<${VT} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=l`
      <${E} type="button" variant="secondary" size="sm" disabled=${r.codexBusy} onClick=${r.startCodex}>
        ${s("onboarding.signIn")}
      <//>
    `:a?c=l`<${E} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>i(t)}>
      ${s("llm.use")}
    <//>`:c=l`<${E} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>o(t)}>
      ${s("onboarding.setUp")}
    <//>`,l`
    <${ae} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-4">
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <${b2} id=${e.id} name=${u} />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">${u}</span>
            ${a&&l`<${U} tone="positive" label=${s("onboarding.ready")} size="sm" />`}
          </div>
          <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">${s(e.descKey)}</div>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap gap-2 sm:justify-end">${c}</div>
    <//>
  `}function x2(){let{isAdmin:e=!1,isChecking:t=!1}=Ka();return t?null:e?l`<${YT} />`:l`<${ct} to="/chat" replace />`}function YT(){let e=k(),t=me(),a=Z(),{gatewayStatus:n}=Ka(),r=Oc({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=QT.map(f=>({entry:f,provider:s.providers.find(m=>m.id===f.id)})).filter(f=>f.provider),o=h.default.useCallback(()=>t("/chat"),[t]),u=Lc({onSuccess:o}),c=h.default.useCallback(async f=>{let m=f.active_model||f.default_model||"";await zo({provider_id:f.id,model:m}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=h.default.useCallback(async({form:f,apiKey:m,provider:p})=>{await r.handleSave({form:f,apiKey:m,provider:p});let b=p?.id||f.id.trim(),y=f.model?.trim()||p?.default_model||"";await zo({provider_id:b,model:y}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?l`
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
          ${i.map(({entry:f,provider:m})=>l`
              <${GT}
                key=${f.id}
                entry=${f}
                provider=${m}
                configured=${Ur(m,s.builtinOverrides)}
                isBusy=${s.isBusy}
                login=${u}
                t=${e}
                onUse=${c}
                onSetUp=${r.openDialog}
              />
            `)}
        </div>

        <${Mc} login=${u} />

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

      <${Dc}
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
  `}function j({children:e,className:t="",...a}){return l`<${ae} className=${t} ...${a}>${e}<//>`}function nt({label:e,value:t,tone:a="muted",badgeLabel:n,detail:r,showDivider:s=!0,className:i="",valueClassName:o="text-[1.75rem] md:text-[2rem]"}){return l`
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
          ${r&&l`<div className="mt-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            ${r}
          </div>`}
        </div>
        <${U} tone=${a} label=${n??a} />
      </div>
    </div>
  `}function $2({items:e}){return l`
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
  `}function ye({title:e,description:t,children:a,boxed:n=!0}){let r=l`
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
  `;return n?l`<${ae} padding="lg">${r}<//>`:l`<div className="py-8">${r}</div>`}var w2={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function Ya({result:e,onDismiss:t}){return e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",w2[e.type]||w2.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}var S2="",JT={workspace:"home"};function Pc(e){return JT[e]||e}function Yo(e){return[...e||[]].sort((t,a)=>t.is_dir!==a.is_dir?t.is_dir?-1:1:t.name.localeCompare(a.name,void 0,{sensitivity:"base"}))}function ii(e){return e?e.split("/").filter(Boolean):[]}function Uc(e){return e?`/workspace/${ii(e).map(encodeURIComponent).join("/")}`:"/workspace"}function rh(e){let t=ii(e);return t.pop(),t.join("/")}function N2(e){return/\.mdx?$/i.test(e||"")}function jc({path:e,onNavigate:t}){let a=k(),n=ii(e),r="";return l`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button
        type="button"
        onClick=${()=>t("/workspace")}
        className="text-signal hover:underline"
      >
        ${a("workspace.breadcrumbRoot")}
      </button>
      ${n.map((s,i)=>{r=r?`${r}/${s}`:s;let o=r,u=i===0?Pc(s):s;return l`
          <span key=${o} className="text-iron-400">/</span>
          <button
            key=${`${o}-button`}
            type="button"
            onClick=${()=>t(Uc(o))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${u}
          </button>
        `})}
    </div>
  `}function XT(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function _2({path:e,entries:t,isLoading:a,filter:n,onOpen:r,onNavigate:s}){let i=k();if(a)return l`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;let o=(t||[]).filter(m=>!XT(m.path)),u=String(n||"").trim().toLowerCase(),c=u?o.filter(m=>m.name.toLowerCase().includes(u)):o,d=Yo(c),f;return o.length?d.length?f=l`
      <div className="divide-y divide-white/[0.06]">
        ${d.map(m=>l`
          <button
            key=${m.path}
            type="button"
            onClick=${()=>r(m.path)}
            className="flex w-full items-center gap-3 px-4 py-2.5 text-left text-sm text-iron-200 hover:bg-white/[0.05] hover:text-white"
          >
            <span className=${["w-4 text-center text-xs",m.is_dir?"text-signal":"text-iron-400"].join(" ")}>
              ${m.is_dir?"\u25A1":"\xB7"}
            </span>
            <span className="min-w-0 truncate ${m.is_dir?"font-semibold":""}">${m.name}</span>
          </button>
        `)}
      </div>
    `:f=l`<div className="px-4 py-10 text-center text-sm text-iron-300">${i("workspace.noMatches")}</div>`:f=l`<div className="px-4 py-10 text-center text-sm text-iron-300">${i("workspace.emptyDir")}</div>`,l`
    <${j} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${jc} path=${e} onNavigate=${s} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">${f}</div>
    <//>
  `}var Fc="/api/webchat/v2/fs",ZT=1024*1024,WT=8*1024*1024;function k2(e){let t=String(e||"").split("/").filter(Boolean);return{mount:t.shift()||"",path:t.join("/")}}function e5(e,t){return t?`${e}/${t}`:e}function t5(e){let t=String(e||"").toLowerCase();return t.startsWith("text/")||t==="application/json"||t==="application/javascript"||t==="application/xml"||t.endsWith("+json")||t.endsWith("+xml")}function a5(e){return String(e||"").toLowerCase().startsWith("image/")}function n5(e){let t=String(e||"").toLowerCase();return t.startsWith("audio/")||t.startsWith("video/")||t.startsWith("font/")||t==="application/pdf"||t==="application/zip"||t==="application/gzip"}function r5(e){if(e.subarray(0,Math.min(e.length,8192)).indexOf(0)!==-1)return!0;try{return new TextDecoder("utf-8",{fatal:!0}).decode(e),!1}catch{return!0}}function s5(e,t){let a=new URL(`${Fc}/content`,window.location.origin);return a.searchParams.set("mount",e),a.searchParams.set("path",t),a.pathname+a.search}async function i5(){return(await H(`${Fc}/mounts`))?.mounts||[]}async function oi(e=""){if(!e)return{entries:(await i5()).map(o=>({name:Pc(o.mount),path:o.mount,is_dir:!0}))};let{mount:t,path:a}=k2(e),n=new URL(`${Fc}/list`,window.location.origin);return n.searchParams.set("mount",t),a&&n.searchParams.set("path",a),{entries:((await H(n.pathname+n.search))?.entries||[]).map(i=>({name:i.name,path:e5(t,i.path),is_dir:i.kind==="directory"}))}}async function R2(e){let{mount:t,path:a}=k2(e);if(!t||!a)return{kind:"directory",path:e};let n=new URL(`${Fc}/stat`,window.location.origin);n.searchParams.set("mount",t),n.searchParams.set("path",a);let s=(await H(n.pathname+n.search))?.stat||{},i=s.mime_type||"application/octet-stream",o=Number(s.size_bytes||0),u=s5(t,a),c={path:e,mime:i,size_bytes:o,download_path:u};if(s.kind&&s.kind!=="file")return{...c,kind:"directory"};if(a5(i)){if(o>WT)return{...c,kind:"binary"};let p=await rc(u);return{...c,kind:"image",image_data_url:p}}if(n5(i)||o>ZT)return{...c,kind:"binary"};let d=await Ra(u),f=new Uint8Array(await d.arrayBuffer());if(!t5(i)&&r5(f))return{...c,kind:"binary"};let m=new TextDecoder("utf-8").decode(f);return{...c,kind:"text",content:m}}function C2(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function o5(e,t,a){let n=String(t||"").trim().toLowerCase(),r=(e||[]).filter(s=>!C2(s.path)).filter(s=>!n||s.name.toLowerCase().includes(n)?!0:s.is_dir&&a.has(s.path));return Yo(r)}function E2({entry:e,depth:t,selectedPath:a,expandedPaths:n,filter:r,onToggleDirectory:s,onSelectFile:i}){let o=k(),u=n.has(e.path),c=q({queryKey:["workspace-list",e.path],queryFn:()=>oi(e.path),enabled:e.is_dir&&u});if(e.is_dir){let d=o5(c.data?.entries,r,n);return l`
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
            ${c.isLoading?l`<div className="px-4 py-2 text-xs text-iron-400">${o("workspace.loading")}</div>`:c.isError?l`<div className="px-4 py-2 text-xs text-red-300">${o("workspace.unableOpenDirectory")}</div>`:d.map(f=>l`
                  <${E2}
                    key=${f.path}
                    entry=${f}
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
  `}function A2({entries:e,selectedPath:t,expandedPaths:a,filter:n,onToggleDirectory:r,onSelectFile:s,isLoading:i}){let o=k();if(i)return l`<div className="space-y-2 p-3">${[1,2,3,4].map(c=>l`<div key=${c} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let u=Yo(e.filter(c=>!C2(c.path)));return u.length?l`
    <div className="space-y-1 p-2">
      ${u.map(c=>l`
        <${E2}
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
  `:l`<div className="px-4 py-8 text-sm text-iron-300">${o("workspace.noFiles")}</div>`}function T2({rootEntries:e,selectedPath:t,expandedPaths:a,filter:n,onFilterChange:r,isLoadingTree:s,onToggleDirectory:i,onSelectFile:o}){let u=k();return l`
    <${j} className="flex min-h-[420px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="border-b border-white/10 p-3">
        <input
          value=${n}
          onInput=${c=>r(c.target.value)}
          placeholder=${u("workspace.filterPlaceholder")}
          className="h-9 w-full rounded-md border border-white/10 bg-iron-950/80 px-3 text-sm text-white outline-none placeholder:text-iron-400 focus:border-signal/45"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <${A2}
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
  `}function D2(e){return ii(e).pop()||"download"}function l5({path:e,file:t}){let a=k();return t.kind==="image"?l`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${t.image_data_url}
          alt=${D2(e)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `:t.kind==="text"?l`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${N2(e)?l`<${ra} content=${t.content} className="max-w-4xl text-base leading-7" />`:l`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
      </div>
    `:l`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${a("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `}function M2({path:e,file:t,isLoading:a,onNavigate:n}){let r=k(),[s,i]=h.default.useState(!1),o=h.default.useCallback(async()=>{if(t?.download_path){i(!0);try{let c=await Ra(t.download_path);_c(c,D2(e))}catch{}finally{i(!1)}}},[t,e]);if(a)return l`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;if(!t||t.kind==="directory")return l`
      <${ye}
        title=${r("workspace.pickFileTitle")}
        description=${r("workspace.pickFileDesc")}
      />
    `;let u=r("workspace.fileMeta",{mime:t.mime||"application/octet-stream",size:Number(t.size_bytes||0)});return l`
    <${j} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${jc} path=${e} onNavigate=${n} />
        <div className="flex items-center gap-2">
          <${U} tone="muted" label=${u} />
          <${E}
            variant="secondary"
            size="sm"
            onClick=${o}
            disabled=${s}
          >${r("workspace.download")}<//>
        </div>
      </div>

      <${l5} path=${e} file=${t} />

      ${rh(e)&&l`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${r("workspace.parent",{path:rh(e)})}
        </div>
      `}
    <//>
  `}function O2(e){let t=k(),a=Z(),[n,r]=h.default.useState(new Set),[s,i]=h.default.useState(""),[o,u]=h.default.useState(null),c=q({queryKey:["workspace-list",""],queryFn:()=>oi("")}),d=q({queryKey:["workspace-file",e],queryFn:()=>R2(e),enabled:!!e}),f=e===""||d.data?.kind==="directory",m=q({queryKey:["workspace-list",e],queryFn:()=>oi(e),enabled:f});h.default.useEffect(()=>{u(null)},[e]);let p=h.default.useCallback(y=>a.fetchQuery({queryKey:["workspace-list",y],queryFn:()=>oi(y)}),[a]),b=h.default.useCallback(async y=>{let $=new Set(n);if($.has(y)){$.delete(y),r($);return}$.add(y),r($);try{await p(y)}catch(g){u({type:"error",message:g.message||t("workspace.unableOpenDirectory")})}},[n,p,t]);return{rootEntries:c.data?.entries||[],file:d.data||null,selectionIsDirectory:f,currentEntries:m.data?.entries||[],expandedPaths:n,filter:s,setFilter:i,result:o,clearResult:()=>u(null),isLoadingTree:c.isLoading,isLoadingFile:d.isLoading,isLoadingListing:m.isLoading,isFetching:c.isFetching||d.isFetching||m.isFetching,error:c.error||d.error||m.error||null,loadDirectory:p,toggleDirectory:b,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function sh(){let e=k(),t=me(),n=ut()["*"]||S2,r=O2(n),s=h.default.useCallback(i=>{t(Uc(i))},[t]);return l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="flex h-full min-h-0 flex-col space-y-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1 className="text-lg font-semibold text-white">${e("workspace.title")}</h1>
                <${U} tone="muted" label=${e("workspace.readOnly")} />
              </div>
              <p className="mt-0.5 text-sm text-iron-400">${e("workspace.subtitle")}</p>
            </div>
            <${E}
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
          <${Ya}
            result=${r.result}
            onDismiss=${r.clearResult}
          />

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <${T2}
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
                  <${_2}
                    path=${n}
                    entries=${r.currentEntries}
                    isLoading=${r.isLoadingListing}
                    filter=${r.filter}
                    onOpen=${s}
                    onNavigate=${t}
                  />
                `:l`
                  <${M2}
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
  `}function L2(e){if(!e)return null;let t=e.metadata&&typeof e.metadata=="object"&&!Array.isArray(e.metadata)?e.metadata:{};return{id:e.project_id,name:e.name,description:e.description,goals:Array.isArray(t.goals)?t.goals:[],icon:e.icon||null,color:e.color||null,state:e.state,role:e.role,metadata:t,created_at:e.created_at,updated_at:e.updated_at,health:e.state==="archived"?"muted":"green"}}async function P2(){let t=((await Sx({limit:200}))?.projects||[]).map(L2);return{attention:[],projects:t}}async function U2(e){if(!e)return null;let t=await Nx({projectId:e});return L2(t?.project)}function j2(e){return Promise.resolve({missions:[],todo:!0})}function F2(e){return Promise.resolve({threads:[],todo:!0})}function q2(e){return Promise.resolve({widgets:[],todo:!0})}function z2(e){return Promise.resolve(null)}function B2(e){return Promise.resolve(null)}function I2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function H2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function K2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function Q2(){let e=Z(),t=q({queryKey:["projects-overview"],queryFn:P2,refetchInterval:5e3}),a=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function V2(e){let t=Z(),a=!!e,n=q({queryKey:["project-detail",e],queryFn:()=>U2(e),enabled:a,refetchInterval:a?7e3:!1}),r=q({queryKey:["project-missions",e],queryFn:()=>j2(e),enabled:a,refetchInterval:a?5e3:!1}),s=q({queryKey:["project-threads",e],queryFn:()=>F2(e),enabled:a,refetchInterval:a?4e3:!1}),i=q({queryKey:["project-widgets",e],queryFn:()=>q2(e),enabled:a,refetchInterval:a?15e3:!1}),o=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function G2({projectId:e,missionId:t,threadId:a}){let n=Z(),[r,s]=h.default.useState(null),i=q({queryKey:["project-mission-detail",t],queryFn:()=>z2(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=q({queryKey:["project-thread-detail",a],queryFn:()=>B2(a),enabled:!!a,refetchInterval:a?4e3:!1}),u=h.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=B({mutationFn:({targetMissionId:m})=>I2(m),onSuccess:m=>{s({type:"success",message:m?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to fire mission"})}}),d=B({mutationFn:({targetMissionId:m})=>H2(m),onSuccess:()=>{s({type:"success",message:"Mission paused."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to pause mission"})}}),f=B({mutationFn:({targetMissionId:m})=>K2(m),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:f.mutateAsync,isBusy:c.isPending||d.isPending||f.isPending}}function qc(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function zc(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function Y2(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function J2(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function u5(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function X2(e){let t=u5(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function Z2(e){let t=e?.projects||[],a=t.reduce((o,u)=>o+Number(u.cost_today_usd||0),0),n=t.reduce((o,u)=>o+Number(u.active_missions||0),0),r=t.reduce((o,u)=>o+Number(u.threads_today||0),0),s=t.reduce((o,u)=>o+Number(u.pending_gates||0),0),i=t.reduce((o,u)=>o+Number(u.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function Jo(e,t){return`${e} ${t}${e===1?"":"s"}`}var c5={projects:"muted",attention:"warning",spend:"success"};function W2({overview:e}){let t=Z2(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:zc(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return l`
    <${j} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${U} tone=${c5[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function d5(e){return e?.type==="failure"?"danger":"warning"}function m5(e){return e?.type==="failure"?"failure":"gate"}function eS({items:e,onOpenItem:t}){return e?.length?l`
    <${j} className="overflow-hidden border-amber-300/10 p-0">
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
              <${U} tone=${d5(a)} label=${m5(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function f5({project:e,onOpen:t,t:a}){return l`
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
        <${U} tone=${Y2(e.health)} label=${e.health||"unknown"} />
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
            ${a("projects.card.threadsToday",{count:Jo(e.threads_today||0,"thread")})}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${a("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">${Jo(e.pending_gates||0,"gate")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${a("projects.card.failures24h",{count:Jo(e.failures_24h||0,"failure")})}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>${a("projects.card.spendToday",{value:zc(e.cost_today_usd||0)})}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${qc(e.last_activity)}</div>
        </div>
        <${E}
          variant="secondary"
          onClick=${n=>{n.stopPropagation(),t(e.id)}}
        >${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function p5({project:e,onOpen:t,t:a}){return l`
    <${j}
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
            ${Jo(e.threads_today||0,"thread")} today
          </div>
          <${E}
            variant="secondary"
            onClick=${n=>{n.stopPropagation(),t(e.id)}}
          >${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function tS({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=k(),u=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return!e.length&&t>0?l`
      <${ye}
        title=${o("projects.empty.noMatchTitle")}
        description=${o("projects.empty.noMatchDesc")}
      />
    `:e.length?l`
    <div className="space-y-5">
      ${u&&l`<${p5} project=${u} onOpen=${r} t=${o} />`}

      <${j} className="p-4 sm:p-5">
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
            <${E} onClick=${s}>${o(i?"projects.preparingChat":"projects.newProject")}<//>
          </div>
        </div>
      <//>

      ${c.length?l`<div className="grid gap-4 xl:grid-cols-2 2xl:grid-cols-3">
            ${c.map(d=>l`<${f5} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
          </div>`:l`
            <${ye}
              title=${o("projects.scoped.onlyGeneralTitle")}
              description=${o("projects.scoped.onlyGeneralDesc")}
            >
              <${E} onClick=${s}>${o(i?"projects.preparingChat":"projects.startProject")}<//>
            <//>
          `}
    </div>
  `:l`
      <${ye}
        title=${o("projects.empty.noneTitle")}
        description=${o("projects.empty.noneDesc")}
      >
        <${E} onClick=${s}>${o("projects.createFromChat")}<//>
      <//>
    `}function aS({threads:e,selectedThreadId:t,onSelectThread:a,onNewConversation:n,isStartingConversation:r}){let s=[...e].sort((i,o)=>new Date(o.updated_at||o.created_at)-new Date(i.updated_at||i.created_at));return l`
    <${j} className="p-4 sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Conversations</div>
          <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">Project conversations</h2>
        </div>
        ${n&&l`
          <${E} onClick=${n} disabled=${r}>
            ${r?"Starting\u2026":"New conversation"}
          <//>
        `}
      </div>

      <div className="mt-5 space-y-3">
        ${s.length?s.slice(0,18).map(i=>{let o=X2(i);return l`
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
                    <${U} tone=${J2(i.state)} label=${i.state} />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                    <span>${i.step_count||0} steps</span>
                    <span>${i.total_tokens||0} tokens</span>
                    <span>${qc(i.updated_at||i.created_at)}</span>
                  </div>
                </button>
              `}):l`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                No project threads yet. When an automation runs or scoped chat work happens inside this project, activity will appear here.
              </div>
            `}
      </div>
    <//>
  `}var h5="/workspace";function v5(e){let t=a=>a.kind==="directory"?0:1;return[...e].sort((a,n)=>t(a)-t(n)||a.name.localeCompare(n.name,void 0,{sensitivity:"base"}))}function g5(e){return e?String(e).replace(/^\/workspace\/?/,"").split("/").filter(Boolean):[]}function nS({threadId:e}){let t=k(),[a,n]=h.default.useState(void 0),[r,s]=h.default.useState(null),i=q({queryKey:["project-files",e||"",a||""],queryFn:()=>vx({threadId:e,path:a}),enabled:!!e}),o=h.default.useMemo(()=>v5(i.data?.entries||[]),[i.data]),u=h.default.useCallback(async f=>{if(f.kind==="directory"){s(null),n(f.path);return}try{s(null);let m=await Ra(nc({threadId:e,path:f.path})),p=URL.createObjectURL(m),b=document.createElement("a");b.href=p,b.download=f.name,document.body.appendChild(b),b.click(),b.remove(),URL.revokeObjectURL(p)}catch(m){s(m?.message||"Unable to download file")}},[e]),c=g5(a),d=l`
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
          ${"Files"}
        </div>
        <${U} tone="muted" label=${t("workspace.readOnly")} />
      </div>
      <${E}
        variant="secondary"
        size="sm"
        onClick=${()=>i.refetch()}
        disabled=${!e||i.isFetching}
      >
        ${i.isFetching?t("workspace.refreshing"):t("workspace.refresh")}
      <//>
    </div>
  `;return e?l`
    <${j} className="p-4 sm:p-5">
      ${d}

      <div className="mt-3 flex min-w-0 flex-wrap items-center gap-1.5 font-mono text-xs text-iron-400">
        <button
          type="button"
          onClick=${()=>n(void 0)}
          className="text-signal hover:underline"
        >
          ${"workspace"}
        </button>
        ${c.map((f,m)=>{let p=`${h5}/${c.slice(0,m+1).join("/")}`;return l`
            <span key=${p} className="text-iron-500">/</span>
            <button
              key=${`${p}-button`}
              type="button"
              onClick=${()=>n(p)}
              className="max-w-[160px] truncate text-signal hover:underline"
            >
              ${f}
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
        ${i.isLoading?[1,2,3,4].map(f=>l`<div key=${f} className="v2-skeleton h-9 rounded-[12px]" />`):o.length?o.map(f=>l`
                <button
                  key=${f.path}
                  type="button"
                  onClick=${()=>u(f)}
                  className="flex w-full items-center gap-3 rounded-[12px] border border-transparent px-3 py-2 text-left hover:border-white/10 hover:bg-white/[0.04]"
                >
                  <${D}
                    name=${f.kind==="directory"?"folder":"file"}
                    className="h-4 w-4 shrink-0 text-iron-300"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm text-white">${f.name}</span>
                  ${f.kind==="directory"?l`<${D} name="chevron" className="h-3.5 w-3.5 shrink-0 -rotate-90 text-iron-500" />`:l`<${D} name="download" className="h-3.5 w-3.5 shrink-0 text-iron-500" />`}
                </button>
              `):l`
              <div className="rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                ${"This folder is empty."}
              </div>
            `}
      </div>
    <//>
  `:l`
      <${j} className="p-4 sm:p-5">
        ${d}
        <div className="mt-4 rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
          ${"No files yet \u2014 they appear once a thread has run in this project."}
        </div>
      <//>
    `}function y5(e){return[...e||[]].sort((a,n)=>new Date(n.updated_at||n.created_at)-new Date(a.updated_at||a.created_at))[0]?.id||null}function rS({project:e,threads:t,selectedThreadId:a,onSelectThread:n,onNewConversation:r,isStartingConversation:s}){let i=y5(t);return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]">
      <div className="space-y-5">
        <div className="min-w-0">
          <h2 className="text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
          ${e.description?l`<p className="mt-1 text-sm leading-6 text-iron-300">${e.description}</p>`:null}
        </div>

        <${aS}
          threads=${t}
          selectedThreadId=${a}
          onSelectThread=${n}
          onNewConversation=${r}
          isStartingConversation=${s}
        />
      </div>

      <${nS} threadId=${i} />
    </div>
  `}function Xo(){let e=k(),t=me(),{threadsState:a}=Ka(),{projectId:n=null,threadId:r=null}=ut(),[s,i]=h.default.useState(""),[o,u]=h.default.useState(null),c=Q2(),d=V2(n),f=G2({projectId:n,threadId:r}),m=h.default.useMemo(()=>{let N=s.trim().toLowerCase();return N?c.overview.projects.filter(A=>[A.name,A.description,...A.goals||[]].some(M=>String(M||"").toLowerCase().includes(N))):c.overview.projects},[c.overview.projects,s]),p=h.default.useMemo(()=>c.overview.projects.find(N=>N.id===n)||null,[c.overview.projects,n]),b=h.default.useCallback(()=>{c.invalidate(),d.invalidate()},[c,d]),y=h.default.useCallback(N=>{t(`/projects/${N}`)},[t]),$=h.default.useCallback(N=>{if(N.thread_id){t(`/projects/${N.project_id}/threads/${N.thread_id}`);return}t(`/projects/${N.project_id}`)},[t]),g=h.default.useCallback(async()=>{let N=null;u(null);try{N=await a.createThread()}catch(A){u({type:"error",message:A.message||e("projects.chatAutoFail")})}t("/chat",{state:{composerDraft:e("projects.creationDraft"),threadId:N}})},[t,a]),v=h.default.useCallback(N=>{t(`/projects/${n}/threads/${N}`)},[t,n]),x=h.default.useCallback(async()=>{u(null);try{let N=await a.createThread(n);t("/chat",{state:{threadId:N}}),d.invalidate()}catch(N){u({type:"error",message:N.message||e("projects.chatAutoFail")})}},[t,a,n,d,e]),w=h.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),S=l`
    ${n&&l`<${E} variant="ghost" onClick=${()=>t("/projects")}>${e("projects.allProjects")}<//>`}
  `,R=null;return n?d.isLoading?R=l`
        <div className="space-y-4">
          ${[1,2,3].map(N=>l`<div key=${N} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `:d.error||!d.project&&!p?R=l`
        <${ye}
          title=${e("projects.unavailable")}
          description=${d.error?.message||e("projects.unavailableDesc")}
        >
          <${E} variant="secondary" onClick=${()=>t("/projects")}>${e("projects.returnToProjects")}<//>
        <//>
      `:R=l`
        <${rS}
          project=${d.project||p}
          threads=${d.threads}
          selectedThreadId=${r}
          onSelectThread=${v}
          onNewConversation=${x}
          isStartingConversation=${a.isCreating}
        />
      `:R=c.isLoading?l`
          <div className="space-y-4">
            ${[1,2,3].map(N=>l`<div key=${N} className="v2-skeleton h-40 rounded-[20px]" />`)}
          </div>
        `:l`
          <${tS}
            projects=${m}
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
          <${Ya} result=${o} onDismiss=${()=>u(null)} />
          <${Ya} result=${f.actionResult} onDismiss=${f.clearActionResult} />
          ${!n&&l`
            <${W2} overview=${c.overview} />
            <${eS} items=${c.overview.attention} onOpenItem=${$} />
          `}
          ${R}
        </div>
      </div>
    </div>
  `}function Zo(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not scheduled"}function Wo(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function sS(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function iS(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);return r!==0?r:new Date(n.updated_at||0).getTime()-new Date(a.updated_at||0).getTime()})}function Bc({label:e,value:t}){return l`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function b5({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=k();return e.status==="Active"?l`
      <${E} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.fireNow")}<//>
      <${E} variant="secondary" onClick=${()=>n(e.id)} disabled=${t}>${s("missions.action.pause")}<//>
    `:e.status==="Paused"?l`
      <${E} onClick=${()=>r(e.id)} disabled=${t}>${s("missions.action.resume")}<//>
      <${E} variant="secondary" onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runOnce")}<//>
    `:l`<${E} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runAgain")}<//>`}function oS({mission:e,isLoading:t,error:a,isBusy:n,onFire:r,onPause:s,onResume:i,onOpenProject:o,onOpenThread:u}){let c=k();return t?l`
      <div className="space-y-4">
        ${[1,2,3].map(d=>l`<div key=${d} className="v2-skeleton h-36 rounded-xl" />`)}
      </div>
    `:a||!e?l`
      <${ye}
        title=${c("missions.unavailable")}
        description=${a?.message||c("missions.unavailableDesc")}
      />
    `:l`
    <div className="space-y-4">
      <${j} className="p-4 sm:p-5">
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
          <${U} tone=${Wo(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${Bc} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${Bc} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${Bc} label=${c("missions.meta.nextFire")} value=${Zo(e.next_fire_at)} />
          <${Bc} label=${c("missions.meta.updated")} value=${Zo(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${b5}
            mission=${e}
            isBusy=${n}
            onFire=${r}
            onPause=${s}
            onResume=${i}
          />
        </div>
      <//>

      <${j} className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.brief")}</div>
        <div className="mt-4 text-sm leading-6 text-iron-200">
          <${ra} content=${e.goal||c("missions.noGoal")} />
        </div>
      <//>

      ${e.current_focus&&l`
        <${j} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${ra} content=${e.current_focus} />
          </div>
        <//>
      `}

      ${e.success_criteria&&l`
        <${j} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${ra} content=${e.success_criteria} />
          </div>
        <//>
      `}

      ${e.threads?.length?l`
        <${j} className="p-4 sm:p-5">
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
                  <${U} tone=${Wo(d.state==="Running"?"Active":d.state==="Failed"?"Failed":"Completed")} label=${d.state} />
                </div>
              </button>
            `)}
          </div>
        <//>
      `:null}
    </div>
  `}function x5(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function lS({value:e,onChange:t,children:a,label:n}){return l`
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
  `}function $5({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=k(),s=t===e.id;return l`
    <div
      className=${["w-full rounded-xl border p-4 text-left",s?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80"].join(" ")}
    >
      <button type="button" onClick=${()=>a(e.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${e.name}</div>
              <${U} tone=${Wo(e.status)} label=${e.status} />
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
          ${r("missions.updated",{value:Zo(e.updated_at)})}
        </span>
        <${E}
          variant="ghost"
          onClick=${i=>{i.stopPropagation(),n(e.project.id)}}
        >
          ${e.project.name}
        <//>
      </div>
    </div>
  `}function ih({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:u,projectOptions:c,onSelectMission:d,onOpenProject:f}){let m=k(),p=x5(m);return l`
    <${j} className="p-4 sm:p-5">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${m("missions.title")}</div>
          <h1 className="mt-2 text-3xl font-semibold tracking-tight text-iron-100">${m("missions.subtitle")}</h1>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
            ${m("missions.summary",{missions:t,projects:c.length})}
          </p>
        </div>
      </div>

      <div className="mt-5 flex flex-wrap gap-3">
        <input
          value=${n}
          onChange=${b=>r(b.target.value)}
          placeholder=${m("missions.searchPlaceholder")}
          className="h-11 min-w-[220px] flex-1 rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/40"
        />
        <${lS} value=${s} onChange=${i} label=${m("missions.filter.status")}>
          ${p.map(b=>l`<option key=${b.value} value=${b.value}>${b.label}<//>`)}
        <//>
        <${lS} value=${o} onChange=${u} label=${m("missions.filter.project")}>
          <option value="all">${m("missions.filter.allProjects")}</option>
          ${c.map(b=>l`<option key=${b.id} value=${b.id}>${b.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(b=>l`
              <${$5}
                key=${b.id}
                mission=${b}
                selectedMissionId=${a}
                onSelectMission=${d}
                onOpenProject=${f}
              />
            `):l`
              <${ye}
                title=${m("missions.emptyTitle")}
                description=${m("missions.emptyDesc")}
                boxed=${!1}
              />
            `}
      </div>
    <//>
  `}function w5(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function uS({summary:e}){let t=k(),a=w5(t);return l`
    <${j} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${U} tone=${n.tone} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${e[n.key]||0}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">
              ${n.key==="total"?t("missions.summary.completedFailed",{completed:e.completed||0,failed:e.failed||0}):t("missions.summary.acrossProjects")}
            </p>
          </div>
        `)}
      </div>
    <//>
  `}function cS(){return Promise.resolve({projects:[],todo:!0})}function dS({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function mS(e){return Promise.resolve(null)}function fS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function pS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function hS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function vS(e){let t=q({queryKey:["mission-detail",e],queryFn:()=>mS(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function S5(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function gS(){let e=Z(),[t,a]=h.default.useState(null),n=q({queryKey:["projects-overview"],queryFn:cS,refetchInterval:7e3}),r=n.data?.projects||[],s=yd({queries:r.map(m=>({queryKey:["missions","project",m.id],queryFn:()=>dS({projectId:m.id}),refetchInterval:5e3,select:p=>p?.missions||[]}))}),i=s.flatMap((m,p)=>{let b=r[p];return(m.data||[]).map(y=>S5(y,b))}),o=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),u=(m,p)=>({mutationFn:({missionId:b})=>m(b),onSuccess:()=>{a({type:"success",message:p}),o()},onError:b=>{a({type:"error",message:b.message||"Unable to update mission"})}}),c=B(u(fS,"Mission fired and a run was queued.")),d=B(u(pS,"Mission paused.")),f=B(u(hS,"Mission resumed."));return{projects:r,missions:i,summary:sS(i),isLoading:n.isLoading||s.some(m=>m.isLoading),isRefreshing:n.isFetching||s.some(m=>m.isFetching),error:n.error||s.find(m=>m.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:f.mutateAsync,isBusy:c.isPending||d.isPending||f.isPending,invalidate:o}}function oh(){let e=k(),t=me(),{missionId:a=null}=ut(),[n,r]=h.default.useState(""),[s,i]=h.default.useState("all"),[o,u]=h.default.useState("all"),c=gS(),d=vS(a),f=h.default.useMemo(()=>{let g=n.trim().toLowerCase();return iS(c.missions).filter(v=>{let x=!g||[v.name,v.goal,v.project?.name].some(R=>String(R||"").toLowerCase().includes(g)),w=s==="all"||v.status===s,S=o==="all"||v.project?.id===o;return x&&w&&S})},[c.missions,o,n,s]),m=h.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),p=d.mission?{...m,...d.mission,project:m?.project||null}:m,b=h.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),y=h.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),$=a?l`
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <${ih}
            missions=${f}
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
          <${oS}
            mission=${p}
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
      `:l`
        <${ih}
          missions=${f}
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
            <${E}
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

          <${Ya}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${uS} summary=${c.summary} />

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
  `}var yS=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],N5=new Set(["pending","in_progress"]),bS=new Set(["failed","interrupted","stuck","cancelled"]);function tr(e){return e?String(e).replace(/_/g," "):"unknown"}function li(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":bS.has(e)?"danger":"muted":"muted"}function _5(e){return N5.has(e)}function Ic(e){return _5(e?.state)}function xS(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":bS.has(e.state):!1}function Fr(e,t=8){return e?String(e).slice(0,t):"unknown"}function sa(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function $S(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function lh(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${sa(e.started_at)}`:null].filter(Boolean).join(" / ")}var k5=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function wS(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function R5({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?l`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${t==="tool_use"?a.tool_name||"Tool call":a.tool_name||"Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${wS(t==="tool_use"?a.input:a.output||a.error||a)}</pre>
      </details>
    `:t==="message"?l`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a.role||"assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${a.content||""}</div>
      </div>
    `:l`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t.replace(/_/g," ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${a.message||a.status||wS(a)}</div>
    </div>
  `}function SS({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=k(),[s,i]=h.default.useState("all"),[o,u]=h.default.useState(""),[c,d]=h.default.useState(!0),f=h.default.useRef(null),m=h.default.useMemo(()=>s==="all"?t:t.filter(b=>b.event_type===s),[t,s]);h.default.useEffect(()=>{c&&f.current&&(f.current.scrollTop=f.current.scrollHeight)},[c,m.length]);let p=h.default.useCallback(async(b=!1)=>{let y=o.trim();if(!(!y&&!b))try{await a({content:y||"(done)",done:b}),u("")}catch{}},[o,a]);return l`
    <${j} className="p-5 sm:p-6">
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
            ${k5.map(b=>l`<option key=${b.value} value=${b.value}>${b.label}</option>`)}
          </select>
          <label className="flex items-center gap-2 text-sm text-iron-300">
            <input type="checkbox" checked=${c} onChange=${b=>d(b.target.checked)} />
            Auto-scroll
          </label>
        </div>
      </div>

      <div ref=${f} className="mt-5 max-h-[56vh] space-y-3 overflow-y-auto rounded-[18px] border border-white/10 bg-iron-950/78 p-4">
        ${m.length?m.map(b=>l`
              <div key=${b.id||`${b.event_type}-${b.created_at}`}>
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${sa(b.created_at)}</div>
                <${R5} event=${b} />
              </div>
            `):l`
              <${ye}
                title=${r("job.noActivityTitle")}
                description=${r("job.noActivityDesc")}
              />
            `}
      </div>

      ${e.can_prompt&&l`
        <div className="mt-5 grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto_auto]">
          <input
            value=${o}
            onInput=${b=>u(b.target.value)}
            onKeyDown=${b=>{b.key==="Enter"&&!b.shiftKey&&(b.preventDefault(),p(!1))}}
            placeholder=${r("job.followupPlaceholder")}
            className="h-11 rounded-md border border-white/10 bg-iron-950/90 px-3 text-sm text-white outline-none focus:border-signal/45"
          />
          <${E} variant="secondary" disabled=${n} onClick=${()=>p(!0)}>${r("common.done")}<//>
          <${E} variant="primary" disabled=${n} onClick=${()=>p(!1)}>${r("common.send")}<//>
        </div>
      `}
    <//>
  `}function NS({job:e,activeTab:t,onTabChange:a,onBack:n,onCancel:r,onRestart:s,isBusy:i,children:o}){return l`
    <div className="space-y-5">
      <${j} className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick=${n} className="text-sm text-signal hover:text-white">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-semibold tracking-tight text-white">${e.title||"Untitled job"}</h2>
              <${U} tone=${li(e.state)} label=${tr(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${Fr(e.id)}</span>
              <span>created ${sa(e.created_at)}</span>
              ${lh(e)&&l`<span>${lh(e)}</span>`}
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
            ${Ic(e)&&l`
              <${E} variant="secondary" disabled=${i} onClick=${()=>r(e.id)}>Cancel<//>
            `}
            ${xS(e)&&l`
              <${E} variant="primary" disabled=${i} onClick=${()=>s(e.id)}>Restart<//>
            `}
          </div>
        </div>
      <//>

      <div className="flex flex-wrap gap-2">
        ${yS.map(u=>l`
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
  `}function _S({nodes:e,depth:t=0,selectedPath:a,expandingPath:n,onToggleDirectory:r,onSelectPath:s}){return l`
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
        ${i.isDir&&i.expanded&&i.children?.length?l`<${_S}
              nodes=${i.children}
              depth=${t+1}
              selectedPath=${a}
              expandingPath=${n}
              onToggleDirectory=${r}
              onSelectPath=${s}
            />`:null}
      </div>
    `)}
  `}function kS({canBrowse:e,tree:t,selectedPath:a,selectedFile:n,fileError:r,isLoadingTree:s,isLoadingFile:i,expandingPath:o,treeError:u,onToggleDirectory:c,onSelectPath:d}){return e?l`
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <${j} className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          ${u&&l`<div className="mx-2 mb-3 rounded-md border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${u}</div>`}
          ${s?l`<div className="space-y-2 px-2">${[1,2,3,4].map(f=>l`<div key=${f} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?l`
                  <${_S}
                    nodes=${t}
                    selectedPath=${a}
                    expandingPath=${o}
                    onToggleDirectory=${c}
                    onSelectPath=${d}
                  />
                `:l`<div className="px-2 py-6 text-sm text-iron-300">No files were recorded for this workspace.</div>`}
        </div>
      <//>

      <${j} className="min-h-[440px] p-5 sm:p-6">
        <div className="border-b border-white/10 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">File preview</div>
          <p className="mt-2 break-all text-sm leading-6 text-iron-300">${n?.path||a||"Select a file from the tree to inspect its contents."}</p>
        </div>

        ${r&&!i?l`<div className="mt-5 rounded-md border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">${r}</div>`:i?l`<div className="mt-5 space-y-3">${[1,2,3,4,5].map(f=>l`<div key=${f} className="v2-skeleton h-4 rounded" />`)}</div>`:n?l`<pre className="mt-5 max-h-[60vh] overflow-auto whitespace-pre-wrap rounded-[18px] border border-white/10 bg-iron-950/90 p-4 font-mono text-xs leading-6 text-iron-100">${n.content}</pre>`:l`
                <${ye}
                  title="No file selected"
                  description="Pick a concrete file from the workspace tree to render it here."
                />
              `}
      <//>
    </div>
  `:l`
      <${ye}
        title="No project workspace"
        description="File browsing is only available for sandbox jobs that produced a mounted project directory."
      />
    `}function ui({label:e,value:t}){return l`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function RS({job:e}){let t=(e.transitions||[]).map(a=>({title:`${tr(a.from)} -> ${tr(a.to)}`,description:[sa(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${j} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${U} tone=${li(e.state)} label=${tr(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${ui} label="Created" value=${sa(e.created_at)} />
          <${ui} label="Started" value=${sa(e.started_at)} />
          <${ui} label="Completed" value=${sa(e.completed_at)} />
          <${ui} label="Duration" value=${$S(e.elapsed_secs)} />
          <${ui} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${ui} label="Mode" value=${e.job_mode||"Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${j} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${e.description?l`<${ra} content=${e.description} className="mt-4 text-sm leading-7 text-iron-200" />`:l`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
        <//>

        ${t.length?l`
              <${j} className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Transitions</div>
                <h3 className="mt-2 text-xl font-semibold text-white">State timeline</h3>
                <div className="mt-3">
                  <${$2} items=${t} />
                </div>
              <//>
            `:l`
              <${ye}
                title="No state history yet"
                description="Transitions appear here once the job advances or records a recovery event."
              />
            `}
      </div>
    </div>
  `}function CS({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:u,isBusy:c,isRefreshing:d}){let f=k(),m=[{value:"all",label:f("jobs.list.filter.all")},{value:"pending",label:f("jobs.list.filter.pending")},{value:"in_progress",label:f("jobs.list.filter.inProgress")},{value:"completed",label:f("jobs.list.filter.completed")},{value:"failed",label:f("jobs.list.filter.failed")},{value:"stuck",label:f("jobs.list.filter.stuck")}];if(!e.length){let p=!!n.trim()||s!=="all";return l`
      <${ye}
        title=${f(t&&p?"jobs.list.empty.noMatchTitle":"jobs.list.empty.noJobsTitle")}
        description=${f(t&&p?"jobs.list.empty.noMatchDesc":"jobs.list.empty.noJobsDesc")}
      />
    `}return l`
    <div className="space-y-5">
      <${j} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${f("jobs.list.explorer")}</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-iron-100">${f("jobs.list.queueTitle")}</h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${f("jobs.list.queueDesc")}
            </p>
          </div>
          <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
            <span>${f("jobs.list.visible",{count:e.length})}</span>
            <span>/</span>
            <span>${f(d?"jobs.list.state.refreshing":"jobs.list.state.live")}</span>
          </div>
        </div>

        <div className="mt-5 grid gap-3 md:grid-cols-[minmax(0,1fr)_220px]">
          <input
            value=${n}
            onInput=${p=>r(p.target.value)}
            placeholder=${f("jobs.list.searchPlaceholder")}
            className="h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          />
          <select
            value=${s}
            onChange=${p=>i(p.target.value)}
            className="v2-select h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          >
            ${m.map(p=>l`<option key=${p.value} value=${p.value}>${p.label}</option>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(p=>l`
          <article
            key=${p.id}
            className=${["group flex flex-col gap-4 rounded-[18px] border p-5",a===p.id?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
          >
            <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
              <button onClick=${()=>o(p.id)} className="min-w-0 text-left">
                <div className="flex flex-wrap items-center gap-2">
                  <h3 className="truncate text-lg font-semibold text-iron-100">${p.title||f("jobs.list.untitled")}</h3>
                  <${U} tone=${li(p.state)} label=${tr(p.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${Fr(p.id)}</span>
                  <span>${f("jobs.list.created",{value:sa(p.created_at)})}</span>
                  ${p.started_at&&l`<span>${f("jobs.list.started",{value:sa(p.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${Ic(p)&&l`
                  <${E}
                    variant="secondary"
                    className="h-9 px-3 text-xs"
                    disabled=${c}
                    onClick=${()=>u(p.id)}
                  >
                    ${f("jobs.action.cancel")}
                  <//>
                `}
                <${E} variant="ghost" className="h-9 px-3 text-xs" onClick=${()=>o(p.id)}>${f("jobs.action.open")}<//>
              </div>
            </div>
          </article>
        `)}
      </div>
    </div>
  `}var C5=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function ES({summary:e}){return l`
    <${j} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${C5.map(t=>l`
          <div
            key=${t.key}
            className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
          >
            <${nt}
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
  `}function AS(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function TS(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function DS(e){return Promise.resolve(null)}function MS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function OS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function LS(e){return Promise.resolve({events:[],todo:!0})}function PS(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function uh(e,t=""){return Promise.resolve({entries:[],todo:!0})}function US(e,t){return Promise.resolve({content:"",todo:!0})}function jS(e){let t=Z(),[a,n]=h.default.useState(null),r=q({queryKey:["job-detail",e],queryFn:()=>DS(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=q({queryKey:["job-events",e],queryFn:()=>LS(e),enabled:!!e,refetchInterval:e?2500:!1}),i=B({mutationFn:({content:o,done:u})=>PS(e,{content:o,done:u}),onSuccess:(o,{done:u})=>{n({type:"success",message:u?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return h.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function FS(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function qS(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=qS(a.children,t);if(n)return n}}return null}function Hc(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:Hc(n.children,t,a)}:n)}function zS(e){let[t,a]=h.default.useState([]),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState(""),c=!!(e?.project_dir&&e?.id),d=q({queryKey:["job-files-root",e?.id],queryFn:()=>uh(e.id,""),enabled:c}),f=q({queryKey:["job-file",e?.id,n],queryFn:()=>US(e.id,n),enabled:!!(c&&n)});h.default.useEffect(()=>{a([]),r(""),i(""),u("")},[e?.id]),h.default.useEffect(()=>{d.data?.entries?(a(FS(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let m=h.default.useCallback(async p=>{let b=qS(t,p);if(!(!b||!e?.id)){if(b.expanded){a(y=>Hc(y,p,$=>({...$,expanded:!1})));return}if(b.loaded){a(y=>Hc(y,p,$=>({...$,expanded:!0})));return}u(p);try{let y=await uh(e.id,p);a($=>Hc($,p,g=>({...g,expanded:!0,loaded:!0,children:FS(y.entries)}))),i("")}catch(y){i(y.message||"Unable to open folder")}finally{u("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:f.data||null,fileError:f.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:f.isLoading||f.isFetching,expandingPath:o,treeError:s,toggleDirectory:m}}function BS(){let e=Z(),[t,a]=h.default.useState(null),n=q({queryKey:["jobs-summary"],queryFn:TS,refetchInterval:5e3}),r=q({queryKey:["jobs"],queryFn:AS,refetchInterval:5e3}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=B({mutationFn:({jobId:u})=>MS(u),onSuccess:(u,{jobId:c})=>{a({type:"success",message:`Job ${Fr(c)} cancelled`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to cancel job"})}}),o=B({mutationFn:({jobId:u})=>OS(u),onSuccess:u=>{a({type:"success",message:`Restart queued as ${Fr(u?.new_job_id)}`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function IS({result:e,onDismiss:t}){let a=k();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return l`
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
  `}function ch(){let e=k(),t=me(),{jobId:a=null}=ut(),[n,r]=h.default.useState(""),[s,i]=h.default.useState("all"),[o,u]=h.default.useState(a?"activity":"overview"),c=BS(),d=jS(a),f=zS(d.job);h.default.useEffect(()=>{u(a?"activity":"overview")},[a]);let m=h.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(x=>{let w=!v||x.title.toLowerCase().includes(v)||x.id.toLowerCase().includes(v),S=s==="all"||x.state===s;return w&&S})},[c.jobs,n,s]),p=h.default.useCallback(v=>t(`/jobs/${v}`),[t]),b=h.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),y=h.default.useCallback(async v=>{try{let x=await c.restartJob({jobId:v});x?.new_job_id&&t(`/jobs/${x.new_job_id}`)}catch{}},[c,t]),$=l`
    ${a&&l`<${E} variant="ghost" onClick=${()=>t("/jobs")}
      >${e("jobs.allJobs")}<//
    >`}
  `,g=null;if(a)if(d.isLoading)g=l`
        <div className="space-y-4">
          ${[1,2,3].map(v=>l`<div key=${v} className="v2-skeleton h-32 rounded-[18px]" />`)}
        </div>
      `;else if(d.error||!d.job)g=l`
        <${ye}
          title=${e("jobs.unavailable")}
          description=${d.error?.message||e("jobs.unavailableDesc")}
        >
          <${E} variant="secondary" onClick=${()=>t("/jobs")}
            >${e("jobs.returnToJobs")}<//
          >
        <//>
      `;else{let v={overview:l`<${RS} job=${d.job} />`,activity:l`
          <${SS}
            job=${d.job}
            events=${d.events}
            onSendPrompt=${d.sendPrompt}
            isSendingPrompt=${d.isSendingPrompt}
          />
        `,files:l`
          <${kS}
            canBrowse=${f.canBrowse}
            tree=${f.tree}
            selectedPath=${f.selectedPath}
            selectedFile=${f.selectedFile}
            fileError=${f.fileError}
            isLoadingTree=${f.isLoadingTree}
            isLoadingFile=${f.isLoadingFile}
            expandingPath=${f.expandingPath}
            treeError=${f.treeError}
            onToggleDirectory=${f.toggleDirectory}
            onSelectPath=${f.selectPath}
          />
        `};g=l`
        <${NS}
          job=${d.job}
          activeTab=${o}
          onTabChange=${u}
          onBack=${()=>t("/jobs")}
          onCancel=${b}
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
          <${CS}
            jobs=${m}
            totalJobs=${c.jobs.length}
            selectedJobId=${a}
            search=${n}
            onSearchChange=${r}
            stateFilter=${s}
            onStateFilterChange=${i}
            onSelectJob=${p}
            onCancelJob=${b}
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
          <${IS}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${IS}
            result=${d.promptResult}
            onDismiss=${d.clearPromptResult}
          />
          <${ES} summary=${c.summary} />
          ${g}
        </div>
      </div>
    </div>
  `}function ar(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not scheduled"}function Kc(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function Qc(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function HS(e=[]){return[...e].sort((t,a)=>t.enabled!==a.enabled?t.enabled?-1:1:new Date(a.next_fire_at||a.last_run_at||0).getTime()-new Date(t.next_fire_at||t.last_run_at||0).getTime())}function KS(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function E5(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function QS({runs:e}){return e?.length?l`
    <div className="space-y-3">
      ${e.map(t=>l`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${U} tone=${E5(t.status)} label=${t.status} />
              <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                ${ar(t.started_at)}
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
    `}function nr({label:e,value:t}){return l`
    <div className="rounded-xl border border-iron-700 bg-iron-950/50 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div className="mt-2 min-w-0 break-words text-sm text-iron-100">
        ${t||"\u2014"}
      </div>
    </div>
  `}function VS({title:e,value:t}){return l`
    <div>
      <h3 className="text-sm font-semibold text-iron-100">${e}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >${JSON.stringify(t||{},null,2)}</pre>
    </div>
  `}function GS({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=me(),u=k();return t?l`
      <div className="space-y-4">
        ${[1,2,3].map(c=>l`<div key=${c} className="v2-skeleton h-32 rounded-xl" />`)}
      </div>
    `:a||!e?l`
      <${ye}
        title=${u("routine.unavailable")}
        description=${a?.message||u("routine.unavailableDesc")}
      />
    `:l`
    <${j} className="p-4 sm:p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="truncate text-2xl font-semibold tracking-tight text-iron-100">
              ${e.name}
            </h2>
            <${U}
              tone=${Kc(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${U}
              tone=${Qc(e.verification_status)}
              label=${e.verification_status||"unknown"}
            />
          </div>
          <p className="mt-2 max-w-3xl text-sm leading-6 text-iron-300">
            ${e.description||e.trigger_summary||"No description"}
          </p>
        </div>

        <div className="flex flex-wrap gap-2">
          <${E} variant="secondary" disabled=${n} onClick=${r}>Run<//>
          <${E} variant="ghost" disabled=${n} onClick=${s}>
            ${e.enabled?"Disable":"Enable"}
          <//>
          <${E} variant="ghost" onClick=${i}>Delete<//>
        </div>
      </div>

      <div className="mt-5 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <${nr} label="Trigger" value=${e.trigger_summary||e.trigger_type} />
        <${nr} label="Action" value=${KS(e.action)} />
        <${nr} label="Next fire" value=${ar(e.next_fire_at)} />
        <${nr} label="Last run" value=${ar(e.last_run_at)} />
        <${nr} label="Run count" value=${e.run_count} />
        <${nr} label="Failures" value=${e.consecutive_failures} />
        <${nr} label="Created" value=${ar(e.created_at)} />
        <${nr} label="Routine ID" value=${e.id} />
      </div>

      ${e.conversation_id&&l`
        <div className="mt-5">
          <${E} variant="secondary" onClick=${()=>o(`/chat/${e.conversation_id}`)}>
            Open routine thread
          <//>
        </div>
      `}

      <div className="mt-6 grid gap-6 xl:grid-cols-2">
        <${VS} title=${u("routine.triggerPayload")} value=${e.trigger} />
        <${VS} title=${u("routine.actionPayload")} value=${e.action} />
      </div>

      <div className="mt-6">
        <h3 className="mb-3 text-sm font-semibold text-iron-100">Recent runs</h3>
        <${QS} runs=${e.recent_runs} />
      </div>
    <//>
  `}function YS({routine:e,selectedRoutineId:t,onSelectRoutine:a,onTriggerRoutine:n,onToggleRoutine:r,isBusy:s}){let i=t===e.id;return l`
    <article
      className=${["group flex flex-col gap-4 rounded-[18px] border p-5",i?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick=${()=>a(e.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">${e.name}</h3>
            <${U}
              tone=${Kc(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${U}
              tone=${Qc(e.verification_status)}
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
            <span>next ${ar(e.next_fire_at)}</span>
          </div>
        </button>

        <div className="flex shrink-0 flex-wrap gap-2">
          <${E}
            variant="secondary"
            className="h-9 px-3 text-xs"
            disabled=${s}
            onClick=${()=>n(e.id)}
          >
            Run
          <//>
          <${E}
            variant="ghost"
            className="h-9 px-3 text-xs"
            disabled=${s}
            onClick=${()=>r(e.id)}
          >
            ${e.enabled?"Disable":"Enable"}
          <//>
          <${E}
            variant="ghost"
            className="h-9 px-3 text-xs"
            onClick=${()=>a(e.id)}
          >
            Open
          <//>
        </div>
      </div>
    </article>
  `}var A5=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function dh({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:u,onToggleRoutine:c,isBusy:d,isRefreshing:f}){let m=k();if(!e.length){let p=!!n.trim()||s!=="all";return l`
      <${ye}
        title=${t&&p?"No routines match":"No routines yet"}
        description=${t&&p?"Adjust the search or status filter to find a saved routine.":"Routines created from chat will appear here after they are saved."}
      />
    `}return l`
    <div className="space-y-5">
      <${j} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
              ${m("routines.explorer")}
            </div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-iron-100">
              ${m("routines.title")}
            </h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${m("routines.description")}
            </p>
          </div>
          <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
            <span>${e.length} visible</span>
            <span>/</span>
            <span>${f?"refreshing":"live"}</span>
          </div>
        </div>

        <div className="mt-5 grid gap-3 md:grid-cols-[minmax(0,1fr)_220px]">
          <input
            value=${n}
            onInput=${p=>r(p.target.value)}
            placeholder="Search routine name, trigger, or action"
            className="h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          />
          <select
            value=${s}
            onChange=${p=>i(p.target.value)}
            className="v2-select h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          >
            ${A5.map(p=>l`<option key=${p.value} value=${p.value}>${p.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(p=>l`
            <${YS}
              key=${p.id}
              routine=${p}
              selectedRoutineId=${a}
              onSelectRoutine=${o}
              onTriggerRoutine=${u}
              onToggleRoutine=${c}
              isBusy=${d}
            />
          `)}
      </div>
    </div>
  `}var T5=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function JS({summary:e}){return l`
    <${j} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${T5.map(t=>l`
            <div
              key=${t.key}
              className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
            >
              <${nt}
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
  `}function XS(e){let[t,a]=h.default.useState(""),[n,r]=h.default.useState("all");return{filteredRoutines:h.default.useMemo(()=>{let i=t.trim().toLowerCase();return HS(e).filter(o=>{let u=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||u.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function ZS(){return Promise.resolve({routines:[],todo:!0})}function WS(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function eN(e){return Promise.resolve(null)}function Vc(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function Gc(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function tN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function aN(e){let t=Z(),[a,n]=h.default.useState(null),r=q({queryKey:["routine-detail",e],queryFn:()=>eN(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:f=>{n({type:"error",message:f.message||"Unable to update routine"})}}),o=B(i(Vc,"Routine run queued.")),u=B(i(Gc,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,isBusy:o.isPending||u.isPending}}function nN(){let e=Z(),[t,a]=h.default.useState(null),n=q({queryKey:["routines-summary"],queryFn:WS,refetchInterval:5e3}),r=q({queryKey:["routines"],queryFn:ZS,refetchInterval:5e3}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,f)=>({mutationFn:({routineId:m})=>d(m),onSuccess:()=>{a({type:"success",message:f}),s()},onError:m=>{a({type:"error",message:m.message||"Unable to update routine"})}}),o=B(i(Vc,"Routine run queued.")),u=B(i(Gc,"Routine status updated.")),c=B(i(tN,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||u.isPending||c.isPending,invalidate:s}}function mh(){let e=me(),{routineId:t=null}=ut(),a=nN(),n=aN(t),r=XS(a.routines),s=h.default.useCallback(async(u,c)=>{try{await u({routineId:c})}catch{}},[]),i=h.default.useCallback(async(u,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:u}),e("/routines")}catch{}},[e,a]),o=t?l`
        <div className="grid gap-5 xl:grid-cols-[minmax(0,0.9fr)_minmax(440px,1.1fr)]">
          <${dh}
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
          <${GS}
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
        <${dh}
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
            <${E} variant="ghost" onClick=${()=>e("/routines")}>
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

          <${Ya}
            result=${a.actionResult}
            onDismiss=${a.clearActionResult}
          />
          <${Ya}
            result=${n.actionResult}
            onDismiss=${n.clearActionResult}
          />
          <${JS} summary=${a.summary} />

          ${a.isLoading?l`
                <div className="space-y-4">
                  ${[1,2,3].map(u=>l`<div key=${u} className="v2-skeleton h-32 rounded-xl" />`)}
                </div>
              `:o}
        </div>
      </div>
    </div>
  `}function D5(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function M5(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function rN({deliveryState:e}){let t=k(),a=e.currentTarget?.target_id||"",[n,r]=h.default.useState(a),[s,i]=h.default.useState(!1),o=h.default.useRef(null);h.default.useEffect(()=>{r(a)},[a]),h.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let u=n!==a,c=e.isLoading||e.isSaving,d=u&&!c,f=!!a&&!c,m=e.finalReplyTargets.length>0,p=e.targets.some(M=>M?.capabilities?.final_replies&&M?.target?.status==="unavailable"),b=m||p,y=M=>(o.current&&clearTimeout(o.current),i(!1),M.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),$=()=>{d&&y(e.saveFinalReplyTarget(n||null))},g=()=>{f&&(r(""),y(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),x=e.currentStatus,w=x==="available"?"success":x==="unavailable"?"warning":"muted",S=t(x==="available"?"automations.delivery.pill.ready":x==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),R=!!e.currentTarget,N=t(R?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),A=M5(t("automations.delivery.footnote"),{command:l`<code
        key="cmd"
        className="rounded px-1.5 py-0.5 font-mono text-[0.6875rem] bg-[var(--v2-surface-muted)] text-[var(--v2-accent-text)]"
      >
        approve &lt;code&gt;
      </code>`});return l`
    <${j} className="p-5 sm:p-6">
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
        ${R&&l`
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
              <${U} tone=${w} label=${S} />
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
            ${e.finalReplyTargets.map(M=>{let P=M?.target?.target_id??"",G=M?.target?.display_name||M?.target?.target_id||"",C=M?.target?.description||"",Y=M?.target?.status??"available",K=n===P;return l`
                <label
                  key=${P}
                  className=${V("flex items-start gap-3.5 rounded-xl border px-4 py-3.5 cursor-pointer","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]","hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",K&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
                >
                  <input
                    type="radio"
                    name="delivery-target"
                    value=${P}
                    checked=${K}
                    disabled=${c}
                    onChange=${()=>r(P)}
                    className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-semibold text-[var(--v2-text-strong)] leading-snug">
                      ${G}
                    </div>
                    ${C&&l`<div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                      ${C}
                    </div>`}
                  </div>
                  <${U}
                    tone=${D5(Y)}
                    label=${t(Y==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.ready")}
                    className="self-center shrink-0"
                  />
                </label>
              `})}

            <!-- Unpaired notice rows (targets present but status=unavailable
                 and NOT already shown above because they lack final_replies) -->
            ${p&&l`
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
                <${U}
                  tone="warning"
                  label=${t("automations.delivery.pill.notPaired")}
                  className="shrink-0"
                />
              </div>
            `}

            <!-- Web app only / fallback row -->
            <label
              className=${V("flex items-start gap-3.5 rounded-xl border px-4 py-3.5","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]",m?"cursor-pointer hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]":"cursor-default",n===""&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
            >
              <input
                type="radio"
                name="delivery-target"
                value=""
                checked=${n===""}
                disabled=${c||!m}
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
              <${U}
                tone="muted"
                label=${t("automations.delivery.pill.fallback")}
                className="self-center shrink-0"
              />
            </label>

          </div>
        </div>

        <!-- ── Save row ─────────────────────────────────────────────── -->
        <div className="flex flex-wrap items-center gap-3">
          <${E}
            variant="primary"
            size="sm"
            disabled=${!d}
            onClick=${$}
          >
            <${D} name="check" className="h-3.5 w-3.5" />
            ${t("automations.delivery.save")}
          <//>
          <${E}
            variant="secondary"
            size="sm"
            disabled=${!f}
            onClick=${g}
          >
            ${t("automations.delivery.clear")}
          <//>
          ${s&&l`
            <span
              role="status"
              className="flex items-center gap-1.5 text-xs font-semibold text-[var(--v2-positive-text)]"
            >
              <${D} name="check" className="h-3 w-3" />
              ${t("automations.delivery.saved")}
            </span>
          `}
          ${e.saveError&&!s&&l`
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
        ${b&&l`
          <div
            className="rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-faint)]"
          >
            ${A}
          </div>
        `}

      </div>
    <//>
  `}var O5=["schedule","once"],iN={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},oN={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},lN={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function ia(e){return typeof e=="function"?e:t=>t}var ph=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:wn},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:Y5},{value:"completed",labelKey:"automations.filter.completed",predicate:J5}];function uN(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>O5.includes(r?.source?.type)).map(r=>H5(r,t,a)).sort(G5)}function cN(e,t){let a=ph.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function dN(e){let t=e.filter(i=>i.state!=="completed"),a=t.filter(i=>wn(i)).length,n=t.filter(i=>i.has_running_run).length,r=t.filter(i=>i.has_failed_runs).length,s=t.filter(i=>wn(i)&&fh(i)!=null).sort((i,o)=>(i.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(o.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:t.length,active:a,running:n,failures:r,nextRun:s?.next_run_label||null}}function L5(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=eD(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:u,month:c,dayOfWeek:d,year:f}=s,m=t&&typeof t=="string"?t:null,p=m?` (${m})`:"",b=f==="*"&&u==="*"&&c==="*"&&d==="*";if(b&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=tD(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(rr(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let y=X5(o,i,n);if(!y)return r("automations.schedule.custom");if(b)return r("automations.schedule.everyDayAt",{time:y})+p;let $=aD(d);if(f==="*"&&u==="*"&&c==="*"&&$==="1-5")return r("automations.schedule.weekdaysAt",{time:y})+p;if(f==="*"&&u==="*"&&c==="*"&&rr($,0,7)){let g=Z5(Number($)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:y})+p}if(f==="*"&&rr(u,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(u),time:y})+p;if(rr(u,1,31)&&rr(c,1,12)&&d==="*"&&(f==="*"||rr(f,1970,9999))){let g=W5(Number(c),Number(u),f==="*"?null:Number(f),n);return r("automations.schedule.dateAt",{date:g,time:y})+p}return r("automations.schedule.custom")}function qr(e,t="Unknown",a,n){if(!e)return t;let r=new Date(e);if(Number.isNaN(r.getTime()))return t;let s=n&&typeof n=="string"?{timeZone:n}:{};try{return r.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...s})}catch{return r.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function mN(e,t){let a=iN[e]?.labelKey||"automations.state.unknown";return ia(t)(a)}function fN(e){return iN[e]?.tone||"muted"}function P5(e,t){return wn(e)&&e?.has_running_run?ia(t)("automations.status.running"):wn(e)&&e?.has_failed_runs?ia(t)("automations.status.needsReview"):mN(e?.state,t)}function U5(e){return wn(e)&&e?.has_running_run?"info":wn(e)&&e?.has_failed_runs?"danger":fN(e?.state)}function j5(e,t){let a=oN[e]?.labelKey||"automations.lastStatus.none";return ia(t)(a)}function F5(e){return oN[e]?.tone||"muted"}function q5(e,t){let a=lN[Yc(e)]?.labelKey||"automations.runStatus.unknown";return ia(t)(a)}function z5(e){return lN[Yc(e)]?.tone||"muted"}function B5(e,t,a,n){if(!e)return ia(a)("automations.schedule.custom");let r=qr(e,null,n,t);if(!r)return ia(a)("automations.schedule.custom");let s=t&&typeof t=="string"?` (${t})`:"";return ia(a)("automations.schedule.onceAt",{datetime:r})+s}function I5(e,t,a){return e?.type==="once"?B5(e.at,e.timezone,t,a):e?.type==="schedule"?L5(e.cron,e.timezone||"UTC",t,a):ia(t)("automations.schedule.custom")}function H5(e,t,a){let n=ia(t),r=K5(e.recent_runs,t,a),s=r[0]||null,i=r.find(f=>f.status==="running")||null,o=r.find(f=>f.status==="ok"||f.status==="error")||null,u=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null,d={...e,recent_runs:r,has_running_run:r.some(f=>f.status==="running"),has_failed_runs:r.some(f=>f.status==="error")};return{...d,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:I5(e.source,t,a),state_label:mN(e.state,t),state_tone:fN(e.state),primary_status_label:P5(d,t),primary_status_tone:U5(d),next_run_timestamp:hh(e.next_run_at),next_run_label:qr(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:qr(c,n("automations.date.noRuns"),a),last_status_label:j5(u,t),last_status_tone:F5(u),created_label:qr(e.created_at,n("automations.date.unknown"),a),latest_run:s,current_run:i,success_rate_label:V5(r,t)}}function K5(e,t,a){let n=ia(t);return Array.isArray(e)?e.map(r=>{let s=Yc(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=hh(i);return{...r,status:s,status_label:q5(s,t),status_tone:z5(s),timestamp:o,timestamp_source:i,fired_label:qr(i,n("automations.date.unscheduled"),a),submitted_label:qr(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:qr(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function Yc(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function pN(e){let t=Array.isArray(e)?e:[],a={total:t.length,ok:0,error:0,running:0,unknown:0};for(let n of t)a[Yc(n?.status)]+=1;return a}function Q5(e){let t=pN(e);return[{key:"ok",tone:"text-emerald-300",count:t.ok},{key:"error",tone:"text-red-300",count:t.error},{key:"running",tone:"text-sky-300",count:t.running},{key:"unknown",tone:"text-iron-400",count:t.unknown}].filter(a=>a.count>0)}function hN(e,t){let a=ia(t),n=pN(e),r=Q5(e).map(s=>({...s,text:a(`automations.runs.${s.key}`,{count:s.count})}));return{total:n.total,totalText:a("automations.runs.total",{count:n.total}),chips:r}}function V5(e,t){let a=ia(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function G5(e,t){let a=wn(e),n=wn(t);return a!==n?a?-1:1:(fh(e)??Number.MAX_SAFE_INTEGER)-(fh(t)??Number.MAX_SAFE_INTEGER)}function hh(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function wn(e){return e?.state==="active"||e?.state==="scheduled"}function Y5(e){return["paused","disabled","inactive"].includes(e?.state)}function J5(e){return e?.state==="completed"}function fh(e){return e?.next_run_timestamp??hh(e?.next_run_at)}function vh(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function X5(e,t,a){return!rr(e,0,23)||!rr(t,0,59)?null:vh(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function Z5(e,t){return vh(t,{weekday:"long"},new Date(2001,0,7+e))}function W5(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return vh(n,r,new Date(a??2e3,e-1,t))}function eD(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&sN(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&sN(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function sN(e){return/^0+$/.test(e)}function rr(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function tD(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function aD(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}function nD(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function vN({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let u=o.toString(),c=`/logs${u?`?${u}`:""}`;return i?`/v2${c}`:c}function gN(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(nD),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}var rD=8;function gh(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function Jc({runs:e=[]}){let t=k(),a=e.slice(0,rD);if(!a.length)return l`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;let n=e.length-a.length;return l`
    <div
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf",{shown:a.length,total:e.length})}
    >
      ${a.map(r=>l`
        <span
          key=${gh(r)}
          title=${`${r.status_label} \xB7 ${r.fired_label}`}
          className=${V("h-3 w-3 rounded-full border",r.status==="ok"&&"border-emerald-300/50 bg-emerald-400",r.status==="error"&&"border-red-300/50 bg-red-400",r.status==="running"&&"border-sky-300/60 bg-sky-400",r.status==="unknown"&&"border-iron-500 bg-iron-600")}
        />
      `)}
      ${n>0&&l`<span
        className="ml-0.5 font-mono text-[11px] text-iron-400"
        title=${t("automations.runs.showingOf",{shown:a.length,total:e.length})}
      >
        +${n}
      </span>`}
    </div>
  `}function Xc({runs:e=[],className:t=""}){let a=k(),n=hN(e,a);return n.total?l`
    <div className=${V("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]",t)}>
      <span className="text-iron-300">${n.totalText}</span>
      ${n.chips.map(r=>l`<span key=${r.key} className=${r.tone}>${r.text}</span>`)}
    </div>
  `:l`<span className=${V("text-[11px] text-iron-400",t)}>
      ${a("automations.table.noRuns")}
    </span>`}function yN({run:e,onOpenRun:t,onOpenLogs:a}){let n=k(),r=!!e.chat_path,s=vN({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return l`
    <div className="grid gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto] sm:items-center">
      <div>
        <${U} tone=${e.status_tone} label=${e.status_label} />
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
        <${E}
          variant="secondary"
          size="sm"
          disabled=${!r}
          onClick=${r?()=>t(e.chat_path):void 0}
        >
          <${D} name="chat" className="mr-1.5 h-4 w-4" />
          ${n("automations.detail.openRun")}
        <//>
        <${E}
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
  `}function Zc({label:e,value:t,tone:a}){return l`
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
  `}function bN({automation:e,isMutating:t=!1,onPauseAutomation:a,onResumeAutomation:n,onDeleteAutomation:r}){let s=k(),i=me();if(!e)return l`
      <${j} className="p-4 sm:p-5">
        <${ye}
          boxed=${!1}
          title=${s("automations.detail.emptyTitle")}
          description=${s("automations.detail.emptyDescription")}
        />
      <//>
    `;let o=e.current_run,u=e.state==="paused",c=e.state==="active"||e.state==="scheduled",f=`${s(u?"missions.action.resume":"missions.action.pause")}: ${e.display_name}`,m=()=>{if(u){n?.(e.automation_id);return}c&&a?.(e.automation_id)},p=`${s("common.delete")}: ${e.display_name}`,b=()=>{window.confirm(p)&&r?.(e.automation_id)};return l`
    <${j} className="overflow-hidden">
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
            <${U}
              tone=${e.primary_status_tone}
              label=${e.primary_status_label}
            />
            ${(c||u)&&l`
              <${E}
                type="button"
                variant=${u?"primary":"secondary"}
                size="icon-sm"
                aria-label=${f}
                title=${f}
                disabled=${t}
                onClick=${m}
              >
                <${D} name=${u?"play":"pause"} className="h-4 w-4" />
              <//>
            `}
            <${E}
              type="button"
              variant="danger"
              size="icon-sm"
              aria-label=${p}
              title=${p}
              disabled=${t}
              onClick=${b}
            >
              <${D} name="trash" className="h-4 w-4" />
            <//>
          </div>
        </div>
      </div>

      <div className="space-y-5 p-4 sm:p-5">
        <div className="grid gap-3 sm:grid-cols-2">
          <${Zc} label=${s("automations.detail.schedule")} value=${e.schedule_label} />
          <${Zc}
            label=${s("automations.detail.successRate")}
            value=${e.success_rate_label}
            tone=${e.has_failed_runs?"danger":"success"}
          />
          <${Zc} label=${s("automations.detail.lastCompleted")} value=${e.last_run_label} />
          <${Zc}
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
              <${Jc} runs=${e.recent_runs} />
              <${Xc} runs=${e.recent_runs} />
            </div>
          </div>

          ${e.recent_runs.length?l`
                <div>
                  ${e.recent_runs.map(y=>l`
                    <${yN}
                      key=${gh(y)}
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
  `}var sD=["automations.empty.example1","automations.empty.example2","automations.empty.example3"];function iD({promptKey:e}){let t=k(),a=t(e),[n,r]=h.default.useState(!1),s=h.default.useRef(null);return h.default.useEffect(()=>()=>clearTimeout(s.current),[]),l`
    <li
      className="flex items-center gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3"
    >
      <span className="min-w-0 flex-1 text-sm leading-6 text-iron-200">${a}</span>
      <button
        type="button"
        onClick=${async()=>{try{await navigator.clipboard.writeText(a),r(!0),clearTimeout(s.current),s.current=setTimeout(()=>r(!1),1500)}catch{}}}
        aria-label=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        title=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        className=${V("inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-[var(--v2-panel-border)] text-iron-300 hover:text-iron-100 hover:border-white/20","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",n&&"text-emerald-300")}
      >
        <${D} name=${n?"check":"copy"} className="h-4 w-4" />
      </button>
    </li>
  `}function xN(){let e=k(),t=me();return l`
    <${j} className="p-6 sm:p-8">
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
            ${sD.map(a=>l`<${iD} key=${a} promptKey=${a} />`)}
          </ul>
        </div>

        <div className="mt-6">
          <${E} variant="primary" size="sm" onClick=${()=>t("/chat")}>
            <${D} name="chat" className="mr-1.5 h-4 w-4" />
            ${e("automations.empty.startInChat")}
          <//>
        </div>
      </div>
    <//>
  `}function $N({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,isMutating:s,selectedAutomationId:i,onSelectAutomation:o,onPauseAutomation:u,onResumeAutomation:c,onDeleteAutomation:d}){let f=k(),m=cN(e,t),p=e.length>0,b=m.find(y=>y.automation_id===i)||m[0]||null;return l`
    <div className="space-y-5">
      <${j} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
              ${f("automations.eyebrow")}
            </div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-iron-100">
              ${f("automations.title")}
            </h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${f("automations.description")}
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <div
              className="inline-flex overflow-hidden rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]"
              role="group"
              aria-label=${f("automations.filterLabel")}
            >
              ${ph.map(y=>l`
                <button
                  key=${y.value}
                  type="button"
                  aria-pressed=${t===y.value}
                  onClick=${()=>a(y.value)}
                  className=${V("h-9 px-3 text-xs font-semibold",t===y.value?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
                >
                  ${f(y.labelKey)}
                </button>
              `)}
            </div>
            <${E}
              variant="secondary"
              size="icon-sm"
              aria-label=${f("automations.refresh")}
              title=${f(r?"automations.refreshing":"automations.refresh")}
              disabled=${r}
              onClick=${n}
            >
              <${D}
                name="retry"
                className=${V("h-4 w-4",r&&"v2-spin")}
              />
            <//>
          </div>
        </div>
      <//>

      ${m.length?l`
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1.12fr)_minmax(22rem,0.88fr)]">
              <${j} className="overflow-hidden">
                <div className="overflow-x-auto">
                  <table className="w-full min-w-[900px] border-collapse">
                    <thead>
                      <tr className="border-b border-[var(--v2-panel-border)] text-left">
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${f("automations.table.name")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${f("automations.table.schedule")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${f("automations.table.nextRun")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${f("automations.table.recentRuns")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${f("automations.table.status")}
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      ${m.map(y=>{let $=y.automation_id===b?.automation_id;return l`
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
                                <${Jc} runs=${y.recent_runs} />
                                <${Xc} runs=${y.recent_runs} />
                              </div>
                            </td>
                            <td className="px-5 py-4 align-top">
                              <${U}
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

              <${bN}
                automation=${b}
                isMutating=${s}
                onPauseAutomation=${u}
                onResumeAutomation=${c}
                onDeleteAutomation=${d}
              />
            </div>
          `:p?l`
              <${ye}
                title=${f("automations.empty.matchingTitle")}
                description=${f("automations.empty.matchingDescription")}
              />
            `:l`<${xN} />`}
    </div>
  `}function wN({summary:e,activeFilter:t,onSelectFilter:a}){let n=k(),r=[{key:"scheduled",label:n("automations.summary.scheduled"),value:e?.scheduled??0,tone:"muted",detail:n("automations.summary.scheduledDetail"),filter:"all"},{key:"active",label:n("automations.summary.active"),value:e?.active??0,tone:"signal",detail:n("automations.summary.activeDetail"),filter:"active"},{key:"running",label:n("automations.summary.running"),value:e?.running??0,tone:"info",detail:n("automations.summary.runningDetail"),filter:"running"},{key:"failures",label:n("automations.summary.failures"),value:e?.failures??0,tone:(e?.failures??0)>0?"danger":"success",detail:n("automations.summary.failuresDetail"),filter:(e?.failures??0)>0?"failures":null},{key:"nextRun",label:n("automations.summary.nextRun"),value:e?.nextRun||n("automations.summary.none"),tone:"info",detail:n("automations.summary.nextRunDetail"),valueClassName:"text-lg md:text-xl"}];return l`
    <${j} className="p-4 sm:p-5">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        ${r.map(s=>{let i=!!(s.filter&&a),o=i&&t===s.filter,u=l`
            <${nt}
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
              className=${V(c,"transition-colors hover:border-white/20 hover:bg-white/[0.05]","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",o&&"border-[var(--v2-accent)]/60 bg-[var(--v2-accent-soft)]/30")}
            >
              ${u}
            </button>
          `:l`<div key=${s.key} className=${c}>${u}</div>`})}
      </div>
    <//>
  `}function oD(e){return e==="active"||e==="scheduled"}function lD(e){return Number.isFinite(e)?e:null}function SN(e,t=Date.now()){let a=Array.isArray(e)?e:[],n=null;for(let r of a){if(!r||(r.has_running_run&&(n=n==null?5e3:Math.min(n,5e3)),!oD(r.state)))continue;let s=lD(r.next_run_timestamp);if(s==null)continue;let i=s-t,o=i<=0?2e3:i<3e4?Math.max(1e3,i+1200):null;o!=null&&(n=n==null?o:Math.min(n,o))}return n}var cD=50,dD=25;function NN(e=!1){let{t,lang:a}=nl(),n=Z(),r=q({queryKey:["automations",{includeCompleted:e}],queryFn:()=>yx({limit:cD,runLimit:dD,includeCompleted:e}),refetchInterval:3e4,refetchIntervalInBackground:!1}),s=h.default.useMemo(()=>uN(r.data,t,a),[r.data,t,a]),i=h.default.useMemo(()=>dN(s),[s]),o=h.default.useMemo(()=>SN(s),[s]);h.default.useEffect(()=>{if(o==null)return;let p=setTimeout(()=>{r.refetch()},o);return()=>clearTimeout(p)},[o,r.refetch]);let u=r.data?.scheduler_enabled!==!1,c=h.default.useCallback(()=>{n.invalidateQueries({queryKey:["automations"]})},[n]),d=B({mutationFn:p=>bx({automationId:p}),onSuccess:c}),f=B({mutationFn:p=>xx({automationId:p}),onSuccess:c}),m=B({mutationFn:p=>$x({automationId:p}),onSuccess:c});return{automations:s,summary:i,schedulerEnabled:u,isLoading:r.isLoading,isRefreshing:r.isFetching,isMutating:d.isPending||f.isPending||m.isPending,error:r.error||null,actionError:d.error||f.error||m.error||null,pauseAutomation:d.mutate,resumeAutomation:f.mutate,deleteAutomation:m.mutate,refetch:r.refetch}}var _N=["outbound-delivery","preferences"],kN=["outbound-delivery","targets"];function RN(){let e=Z(),t=q({queryKey:_N,queryFn:_x}),a=q({queryKey:kN,queryFn:kx}),n=B({mutationFn:({finalReplyTargetId:i})=>Rx({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(_N,i),e.invalidateQueries({queryKey:kN})}}),r=h.default.useMemo(()=>a.data?.targets??[],[a.data]),s=h.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function CN(){let e=k(),[t,a]=h.default.useState("all"),[n,r]=h.default.useState(null),i=NN(t==="completed"),o=RN(),[u,c]=h.default.useState(!1),d=h.default.useRef(null);h.default.useEffect(()=>()=>clearTimeout(d.current),[]);let f=h.default.useCallback(()=>{c(!0),clearTimeout(d.current),d.current=setTimeout(()=>c(!1),1e3),i.refetch()},[i.refetch]),m=i.isRefreshing||u,p=i.error&&!i.isLoading&&i.automations.length===0;return h.default.useEffect(()=>{if(!i.automations.length){r(null);return}i.automations.some(y=>y.automation_id===n)||r(i.automations[0].automation_id)},[i.automations,n]),l`
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

          ${p?null:l`
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
                <${wN}
                  summary=${i.summary}
                  activeFilter=${t}
                  onSelectFilter=${a}
                />
                <${rN} deliveryState=${o} />

                ${i.isLoading?l`
                      <div className="space-y-4">
                        ${[1,2,3].map(b=>l`<div
                              key=${b}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`)}
                      </div>
                    `:l`
                      <${$N}
                        automations=${i.automations}
                        filter=${t}
                        onFilterChange=${a}
                        onRefresh=${f}
                        isRefreshing=${m}
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
  `}var EN={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function AN({result:e,onDismiss:t}){return h.default.useEffect(()=>{if(!e)return;let a=setTimeout(t,4e3);return()=>clearTimeout(a)},[e,t]),e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",EN[e.type]||EN.info].join(" ")}>
      <${D}
        name=${e.type==="success"?"check":e.type==="error"?"close":"bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">
        <${D} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `:null}var DN="/api/webchat/v2/channels/slack/setup";function MN(){return H(DN)}function ON(e){let t={installation_id:String(e.installation_id||"").trim(),team_id:String(e.team_id||"").trim(),api_app_id:String(e.api_app_id||"").trim(),user_id:TN(e.user_id),shared_subject_user_id:TN(e.shared_subject_user_id)},a=String(e.bot_token||"").trim(),n=String(e.signing_secret||"").trim();return a&&(t.bot_token=a),n&&(t.signing_secret=n),H(DN,{method:"PUT",body:JSON.stringify(t)})}function yh(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function TN(e){let t=String(e||"").trim();return t||null}var LN="/api/webchat/v2/channels/slack/allowed",mD="/api/webchat/v2/channels/slack/subjects";function PN(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function UN(){return H(LN)}function jN(){return H(mD)}function FN(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return H(LN,{method:"PUT",body:JSON.stringify(n)})}function qN(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var zN=["slack-allowed-channels"];function IN({action:e}){let t=k(),a=Z(),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState([]),c=pD(e,t),d=q({queryKey:zN,queryFn:UN}),f=q({queryKey:["slack-routable-subjects"],queryFn:jN}),m=f.data?.subjects||[],p=BN(m),b=f.isSuccess||f.isError,y=m.length>0;h.default.useEffect(()=>{d.data&&u(bh(d.data.channels||[]))},[d.data]);let $=B({mutationFn:({channels:R})=>FN(R),onSuccess:R=>{u(bh(R.channels||[])),a.invalidateQueries({queryKey:zN}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let R=n.trim();!R||!f.isSuccess||(u(N=>bh([...N,{channel_id:R,subject_user_id:s}])),r(""))},v=R=>{u(N=>N.filter(A=>A.channel_id!==R))},x=(R,N)=>{u(A=>A.map(M=>M.channel_id===R?{...M,subject_user_id:N}:M))},w=()=>{$.mutate({channels:fD(o)})},S=f.isError&&o.some(R=>!R.subject_user_id);return l`
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
          onChange=${R=>r(R.target.value)}
          onKeyDown=${R=>R.key==="Enter"&&g()}
          placeholder=${c.inputPlaceholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <select
          value=${s}
          onChange=${R=>i(R.target.value)}
          disabled=${!y}
          className="h-9 min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
        >
          ${!y&&l`<option value="">${c.noSubjectsLabel}</option>`}
          ${y&&l`<option value="">${c.autoSubjectLabel}</option>`}
          ${p.map(R=>l`
              <option key=${R.subject_user_id} value=${R.subject_user_id}>
                ${R.display_name}
              </option>
            `)}
        </select>
        <${E}
          variant="secondary"
          size="sm"
          className="shrink-0"
          onClick=${g}
          disabled=${!n.trim()||!f.isSuccess}
        >
          ${c.addLabel}
        <//>
      </div>

      <div className="mb-3 rounded-lg border border-white/[0.06] bg-black/10">
        ${d.isLoading&&l`<div className="px-3 py-2 text-xs text-iron-400">${c.loadingMessage}</div>`}
        ${!d.isLoading&&o.length===0&&l`<div className="px-3 py-2 text-xs text-iron-500">
          ${c.emptyMessage}
        </div>`}
        ${o.map(R=>l`
            <label
              key=${R.channel_id}
              className="flex min-h-10 items-center justify-between gap-3 border-t border-white/[0.05] px-3 first:border-t-0"
            >
              <span className="min-w-0">
                <span className="block truncate font-mono text-xs text-iron-200">
                  ${R.channel_id}
                </span>
              </span>
              <div className="flex shrink-0 items-center gap-2">
                ${y?l`
                    <select
                      value=${R.subject_user_id}
                      onChange=${N=>x(R.channel_id,N.target.value)}
                      className="h-8 rounded-md border border-white/10 bg-white/[0.04] px-2 text-xs text-iron-100 outline-none focus:border-signal/45"
                    >
                      <option value="">${c.autoSubjectLabel}</option>
                      ${BN(m,R).map(N=>l`
                          <option key=${N.subject_user_id} value=${N.subject_user_id}>
                            ${N.display_name}
                          </option>
                        `)}
                    </select>
                  `:l`<span className="max-w-40 truncate text-xs text-iron-500">
                    ${R.subject_user_id?R.subject_display_name||R.subject_user_id:c.autoSubjectLabel}
                  </span>`}
                <input
                  type="checkbox"
                  checked=${!0}
                  aria-label=${c.allowLabel(R.channel_id)}
                  onChange=${()=>v(R.channel_id)}
                  className="h-4 w-4 rounded border-white/20 bg-white/[0.04] text-signal"
                />
              </div>
            </label>
          `)}
      </div>

      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <${E}
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick=${w}
          disabled=${!d.isSuccess||!b||$.isPending||S}
        >
          ${$.isPending?c.savingLabel:c.submitLabel}
        <//>
        ${$.isSuccess&&l`<p className="text-xs text-emerald-300">
          ${c.successMessage}
        </p>`}
        ${(d.isError||f.isError||$.isError)&&l`<p className="text-xs text-red-300">
          ${qN($.error||d.error||f.error,c.errorMessage)}
        </p>`}
      </div>
    </div>
  `}function BN(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function bh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return PN(Array.from(t.keys())).map(a=>t.get(a))}function fD(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function pD(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var xh=["slack-setup"],zr={installationId:{body:"Local IronClaw name for this Slack install. Choose one and keep it stable.",example:"Example: local-slack"},teamId:{body:"Slack workspace/team ID from the workspace that installed the app.",example:"Example: T0123456789"},appId:{body:"Slack app Basic Information > App Credentials.",example:"Example: A0123456789"},botUser:{body:"Optional Reborn user. Blank uses the current WebUI operator.",example:"Example: user:operator"},sharedSubject:{body:"Optional default team agent for shared channel turns. Usually blank.",example:"Example: user:slack-shared"},botToken:{body:"Slack app OAuth & Permissions > Bot User OAuth Token.",example:"Example: xoxb-..."},signingSecret:{body:"Slack app Basic Information > App Credentials > Signing Secret.",example:""}};function QN({action:e}){let t=q({queryKey:xh,queryFn:MN}),a=t.data?.configured===!0;return l`
    <div className="space-y-3">
      <${hD} action=${e} setupQuery=${t} />
      ${a&&l`<${IN} action=${e} />`}
    </div>
  `}function hD({action:e,setupQuery:t}){let a=Z(),[n,r]=h.default.useState(vD()),s=h.default.useRef(!1),i=h.default.useRef(!1),o=t.data,u=gD(e);h.default.useEffect(()=>{!o||s.current||i.current||(r(HN(o)),s.current=!0)},[o]);let c=B({mutationFn:ON,onSuccess:p=>{i.current=!1,r(HN(p)),s.current=!0,a.setQueryData(xh,p),a.invalidateQueries({queryKey:xh}),a.invalidateQueries({queryKey:["slack-allowed-channels"]}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["extensions"]})}}),d=p=>b=>{i.current=!0,r(y=>({...y,[p]:b.target.value}))},f=()=>c.mutate(n),m=n.installation_id.trim()&&n.team_id.trim()&&n.api_app_id.trim()&&(o?.bot_token_configured||n.bot_token.trim())&&(o?.signing_secret_configured||n.signing_secret.trim());return l`
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
        ${el("Installation ID",n.installation_id,d("installation_id"),"",zr.installationId)}
        ${el("Team ID",n.team_id,d("team_id"),"",zr.teamId)}
        ${el("App ID",n.api_app_id,d("api_app_id"),"",zr.appId)}
        ${el("Bot user",n.user_id,d("user_id"),"default operator",zr.botUser)}
        ${el("Shared subject",n.shared_subject_user_id,d("shared_subject_user_id"),"optional",zr.sharedSubject)}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        ${KN("Bot token",n.bot_token,d("bot_token"),o?.bot_token_configured,zr.botToken)}
        ${KN("Signing secret",n.signing_secret,d("signing_secret"),o?.signing_secret_configured,zr.signingSecret)}
      </div>

      <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <${E}
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick=${f}
          disabled=${!m||c.isPending}
        >
          ${c.isPending?"Saving...":u.submitLabel}
        <//>
        ${t.isError&&l`<p className="text-xs text-red-300">
          ${yh(t.error,u.errorMessage)}
        </p>`}
        ${c.isError&&l`<p className="text-xs text-red-300">
          ${yh(c.error,u.errorMessage)}
        </p>`}
        ${c.isSuccess&&l`<p className="text-xs text-emerald-300">${u.successMessage}</p>`}
      </div>
    </div>
  `}function HN(e){return{installation_id:e.installation_id||"",team_id:e.team_id||"",api_app_id:e.api_app_id||"",user_id:e.user_id||"",shared_subject_user_id:e.shared_subject_user_id||"",bot_token:"",signing_secret:""}}function vD(){return{installation_id:"",team_id:"",api_app_id:"",user_id:"",shared_subject_user_id:"",bot_token:"",signing_secret:""}}function el(e,t,a,n="",r=null){return l`
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-iron-500">${e}</span>
      <input
        type="text"
        value=${t}
        onChange=${a}
        placeholder=${n}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <${VN} help=${r} />
    </label>
  `}function KN(e,t,a,n,r=null){return l`
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
      <${VN} help=${r} />
    </label>
  `}function VN({help:e}){return e?l`
    <p className="mt-1.5 min-h-8 text-[11px] leading-4 text-iron-400">
      <span className="block">${e.body}</span>
      ${e.example&&l`<span className="mt-0.5 block font-mono text-iron-300">${e.example}</span>`}
    </p>
  `:null}function gD(e){return{title:"Slack setup",instructions:e?.instructions||"Configure the Slack app before assigning channels.",submitLabel:"Save setup",successMessage:"Slack setup saved.",errorMessage:"Slack setup update failed."}}var $h={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function Br(e){return e==="wasm_channel"||e==="channel"}var GN={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},YN={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function JN(e){let t=XN(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||Br(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function XN(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function wh(e){let t=XN(e);return t==="active"||t==="ready"}function ZN({extension:e,secrets:t=[],fields:a=[]}={}){return wh(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var WN="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",e_="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",t_="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",a_="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",n_="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",yD="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function r_(e){return e.package_ref?.id||""}function bD({actions:e,isBusy:t}){let a=k(),[n,r]=h.default.useState(!1),s=h.default.useRef(null);return h.default.useEffect(()=>{if(!n)return;let i=o=>{s.current&&!s.current.contains(o.target)&&r(!1)};return document.addEventListener("mousedown",i),()=>document.removeEventListener("mousedown",i)},[n]),l`
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
                <${D} name=${i.icon||"settings"} className="h-3.5 w-3.5" />
                ${i.label}
              </button>
            `)}
        </div>
      `}
    </div>
  `}function s_({items:e}){return!e||e.length===0?null:l`
    <div className="mt-3 flex flex-wrap gap-1">
      ${e.map(t=>l`<span key=${t} className=${yD}>${t}</span>`)}
    </div>
  `}function ci({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=k(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=GN[i]||"muted",u=s(`extensions.state.${i}`)||YN[i]||i,c=s(`extensions.kind.${e.kind}`)||$h[e.kind]||e.kind,d=e.display_name||r_(e),f=!!e.package_ref,m=e.tools||[],[p,b]=h.default.useState(!1),$=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],x=[],w=JN(e);w==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):w==="activate"&&v.push({id:"activate",label:"Activate",run:()=>t(g)}),f&&(e.needs_setup||e.has_auth)&&w!=="configure"&&x.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)}),f&&Br(e.kind)&&(i==="setup_required"||i==="failed")&&x.push({id:"setup",label:"Setup",icon:"settings",run:()=>a(g)}),f&&Br(e.kind)&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&x.push({id:"reconfigure",label:"Reconfigure",icon:"settings",run:()=>a(g)}),f&&x.push({id:"remove",label:s("common.remove")||"Remove",icon:"trash",danger:!0,run:()=>n(g)});let S=v[0];return l`
    <div className=${WN}>
      <div className="flex items-start gap-2">
        <${U} tone=${o} label=${u} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${x.length>0&&l`<${bD} actions=${x} isBusy=${r} />`}
      </div>

      <div className=${e_}>
        <span>${c}</span>
        ${e.version&&l`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&l`<p className=${t_}>${e.description}</p>`}

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

      <div className=${a_}>
        ${m.length>0?l`
              <button
                type="button"
                aria-expanded=${p?"true":"false"}
                onClick=${()=>b(R=>!R)}
                className=${n_}
              >
                <${D} name="layers" className="h-3.5 w-3.5" />
                <span>${m.length===1?s("extensions.oneCapability"):s("extensions.pluralCapabilities",{count:m.length})}</span>
                <${D}
                  name="chevron"
                  className=${["h-3 w-3",p?"rotate-180":""].join(" ")}
                />
              </button>
            `:l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">No capabilities</span>`}
        <span className="flex-1"></span>
        ${S&&l`
          <${E} variant="secondary" size="sm" onClick=${S.run} disabled=${r}>
            ${S.label}
          <//>
        `}
      </div>

      ${p&&l`<${s_} items=${m} />`}
    </div>
  `}function Ir({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=k(),s=r(`extensions.kind.${e.kind}`)||$h[e.kind]||e.kind,i=e.display_name||r_(e),o=!!(e.package_ref&&t),u=e.keywords||[],[c,d]=h.default.useState(!1);return l`
    <div className=${WN}>
      <div className="flex items-start gap-2">
        <${U}
          tone="muted"
          label=${n||r("extensions.state.available")||"available"}
          size="sm"
        />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${i}
        </span>
      </div>

      <div className=${e_}>
        <span>${s}</span>
        ${e.version&&l`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&l`<p className=${t_}>${e.description}</p>`}

      <div className=${a_}>
        ${u.length>0?l`
              <button
                type="button"
                aria-expanded=${c?"true":"false"}
                onClick=${()=>d(f=>!f)}
                className=${n_}
              >
                <${D} name="list" className="h-3.5 w-3.5" />
                <span>${u.length===1?r("extensions.oneKeyword"):r("extensions.pluralKeywords",{count:u.length})}</span>
                <${D}
                  name="chevron"
                  className=${["h-3 w-3",c?"rotate-180":""].join(" ")}
                />
              </button>
            `:l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]"></span>`}
        <span className="flex-1"></span>
        ${o&&l`
          <${E}
            variant="outline"
            size="sm"
            onClick=${()=>t({packageRef:e.package_ref,displayName:i})}
            disabled=${a}
          >
            <${D} name="plus" className="mr-1.5 h-3.5 w-3.5" />
            Install
          <//>
        `}
      </div>

      ${c&&l`<${s_} items=${u} />`}
    </div>
  `}function i_(){return H("/api/webchat/v2/extensions")}function o_(){return H("/api/webchat/v2/extensions/registry")}function l_(e){return H("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function u_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(tl(e))}/activate`,{method:"POST"})}function c_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(tl(e))}/remove`,{method:"POST"})}function d_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(tl(e))}/setup`)}function m_(e,t,a){return Lx(tl(e),{action:"submit",payload:{secrets:t,fields:a}})}function f_(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return H(`/api/webchat/v2/extensions/${encodeURIComponent(tl(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function p_(){return Promise.resolve({requests:[]})}function h_(){return Promise.resolve({success:!1,message:"Pairing requires a v2 pairing endpoint."})}function tl(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var xD=2e3,$D=10*60*1e3;function di(e){return e?.package_ref?.id||null}function Sh(e){return e?.display_name||di(e)||""}function v_(e,t,a){return di(t)||`${e}:${Sh(t)||"unknown"}:${a}`}function wD(e,t){return e.installed!==t.installed?e.installed?-1:1:Sh(e.entry||e.extension).localeCompare(Sh(t.entry||t.extension))}function g_(){let e=Z(),t=q({queryKey:["gateway-status-extensions"],queryFn:Qs,staleTime:1e4}),a=q({queryKey:["extensions"],queryFn:i_}),n=q({queryKey:["extension-registry"],queryFn:o_}),r=q({queryKey:["connectable-channels"],queryFn:Ec}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["extensions"]}),e.invalidateQueries({queryKey:["extension-registry"]}),e.invalidateQueries({queryKey:["gateway-status-extensions"]}),e.invalidateQueries({queryKey:["connectable-channels"]})},[e]),[i,o]=h.default.useState(null),u=h.default.useCallback(()=>o(null),[]),c=B({mutationFn:({packageRef:C})=>l_(C),onSuccess:(C,{displayName:Y})=>{C.success?(o({type:"success",message:C.message||C.instructions||`${Y||"Extension"} installed`}),C.auth_url&&window.open(C.auth_url,"_blank","noopener,noreferrer")):o({type:"error",message:C.message||"Install failed"}),s()},onError:C=>{o({type:"error",message:C.message}),s()}}),d=B({mutationFn:({packageRef:C})=>u_(C),onSuccess:(C,{displayName:Y})=>{C.success?(o({type:"success",message:C.message||C.instructions||`${Y||"Extension"} activated`}),C.auth_url&&window.open(C.auth_url,"_blank","noopener,noreferrer")):C.auth_url?(window.open(C.auth_url,"_blank","noopener,noreferrer"),o({type:"info",message:"Opening authentication\u2026"})):C.awaiting_token?o({type:"info",message:"Configuration required"}):o({type:"error",message:C.message||"Activation failed"}),s()},onError:C=>{o({type:"error",message:C.message})}}),f=B({mutationFn:({packageRef:C})=>c_(C),onSuccess:(C,{displayName:Y})=>{C.success?o({type:"success",message:`${Y||"Extension"} removed`}):o({type:"error",message:C.message||"Remove failed"}),s()},onError:C=>{o({type:"error",message:C.message})}}),m=t.data||{},p=a.data?.extensions||[],b=n.data?.entries||[],y=r.data?.channels||[],$=new Map(p.map(C=>[di(C),C]).filter(([C])=>!!C)),g=new Set(b.map(C=>di(C)).filter(Boolean)),v=[...b.map((C,Y)=>{let K=di(C),oe=K&&$.get(K)||null;return{id:v_("registry",C,Y),installed:!!(oe||C.installed),entry:C,extension:oe}}),...p.filter(C=>{let Y=di(C);return!Y||!g.has(Y)}).map((C,Y)=>({id:v_("installed",C,Y),installed:!0,entry:null,extension:C}))].sort(wD),x=C=>Br(C.kind),w=p.filter(x),S=p.filter(C=>C.kind==="mcp_server"),R=p.filter(C=>!x(C)&&C.kind!=="mcp_server"),N=b.filter(C=>x(C)&&!C.installed),A=b.filter(C=>C.kind==="mcp_server"&&!C.installed),M=b.filter(C=>C.kind!=="mcp_server"&&!x(C)&&!C.installed),P=a.isLoading||n.isLoading,G=c.isPending||d.isPending||f.isPending;return{status:m,extensions:p,channels:w,mcpServers:S,tools:R,channelRegistry:N,mcpRegistry:A,toolRegistry:M,registry:b,catalogEntries:v,connectableChannels:y,isLoading:P,isBusy:G,actionResult:i,clearResult:u,install:c.mutate,activate:d.mutate,remove:f.mutate,invalidate:s}}function y_(e){let t=q({queryKey:["extension-setup",e?.id||e],queryFn:()=>d_(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function b_(e,t){let a=Z(),n=e?.id||e;return B({mutationFn:({secrets:r,fields:s})=>m_(e,r,s),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function x_(e){let t=Z(),a=e?.id||e,n=h.default.useRef(null),r=h.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=h.default.useCallback(()=>{let u=t.getQueryData(["extension-setup",a]);if(u?.secrets?.length>0&&u.secrets.every(m=>m.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(m=>m.package_ref?.id===a),f=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return f==="active"||f==="ready"},[a,t]),o=h.default.useCallback(u=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||u&&u.closed||Date.now()-c>$D)&&(r(),s())},xD)},[r,s,i]);return h.default.useEffect(()=>r,[r]),B({mutationFn:({secret:u,popup:c})=>f_(e,u).then(d=>({res:d,popup:c})),onSuccess:({res:u,popup:c})=>{let d=c;u.authorization_url&&c&&!c.closed?c.location.href=u.authorization_url:u.authorization_url?d=window.open(u.authorization_url,"_blank","noopener,noreferrer"):c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(u,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function $_(e,t={}){let a=q({queryKey:["pairing",e],queryFn:()=>p_(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=Z(),r=B({mutationFn:({code:s})=>h_(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function w_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var SD={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function S_({channel:e,redeemFn:t,i18nKeys:a=SD,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=k(),o=typeof t=="function",u=$_(e,{enabled:!o}),c=Z(),[d,f]=h.default.useState(""),m=ND(i,a,r),p=B({mutationFn:({code:S})=>t(e,S),onSuccess:()=>{f("");for(let S of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:S})}}),b=h.default.useCallback(S=>u.approve({code:S}),[u.approve]),y=h.default.useCallback(()=>{let S=d.trim();S&&(o?p.mutate({code:S}):(u.approve({code:S}),f("")))},[o,d,u.approve,p]),$=o?[]:u.requests,g=o?!1:u.isLoading,v=o?p.isPending:u.isApproving,x=o?p.isSuccess?p.data:null:u.result,w=o?p.isError?p.error:null:u.error;return g?l`
      <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
        <div className="v2-skeleton h-3 w-24 rounded" />
      </div>
    `:l`
    <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
      <h4 className="mb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        ${m.title}
      </h4>
      <p className="mb-4 text-xs leading-5 text-iron-300">${m.instructions}</p>

      <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          type="text"
          value=${d}
          onChange=${S=>f(S.target.value)}
          onKeyDown=${S=>S.key==="Enter"&&y()}
          placeholder=${m.placeholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${E}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${y}
          disabled=${v||!d.trim()}
        >
          ${m.action}
        <//>
      </div>

      ${x?.success&&l`<p className="mb-3 text-xs text-emerald-300">
        ${x.message||m.success}
      </p>`}
      ${x&&!x.success&&l`<p className="mb-3 text-xs text-red-300">
        ${x.message||m.error}
      </p>`}
      ${w&&l`<p className="mb-3 text-xs text-red-300">
        ${w_(w,m.error)}
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
                  <${E}
                    variant="secondary"
                    className="h-7 px-2.5 text-xs"
                    onClick=${()=>b(S.code||S.id)}
                    disabled=${v}
                  >
                    ${m.action}
                  <//>
                </div>
              `)}
            </div>
          `:s&&l`<p className="text-xs text-iron-300">${i(a.empty)}</p>`}
    </div>
  `}function ND(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function Wc(e){return e.package_ref?.id||""}function N_(e){return Wc(e)==="slack"}function k_(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function R_(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function _D(e){let t=e||[],a=[t.find(k_),t.find(R_)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function __({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>k_(r)?l`<${QN} action=${r.action} />`:R_(r)?l`<${Sc} action=${r.action} />`:null).filter(Boolean);return n.length>0?l`<div className="space-y-3">${n}</div>`:null}function C_({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:u}){let c=k(),d=t||[],f=e.enabled_channels||[],m=_D(a),p=d.some(N_),b=m.length>0&&!p;return l`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          ${c("channels.builtIn")}
        </h3>
        <${mi}
          name="Web Gateway"
          description=${c("channels.webGatewayDesc")||"Browser-based chat with SSE streaming"}
          enabled=${!0}
          detail=${"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)}
        />
        <${mi}
          name="HTTP Webhook"
          description=${c("channels.httpWebhookDesc")||"Inbound webhook endpoint for external integrations"}
          enabled=${f.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${mi}
          name="CLI"
          description=${c("channels.cliDesc")||"Terminal interface with TUI or simple REPL"}
          enabled=${f.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${mi}
          name="REPL"
          description=${c("channels.replDesc")||"Minimal read-eval-print loop for testing"}
          enabled=${f.includes("repl")}
          detail="ironclaw run --repl"
        />
        ${b&&l`
          <${mi}
            name=${c("channels.slack")||"Slack"}
            description=${c("channels.slackDesc")||"Tenant app channel for DMs and app mentions"}
            enabled=${!1}
            statusLabel="legacy"
            statusTone="muted"
            detail=${c("channels.slackDetail")||"Tenant Slack app install"}
          >
            <${__}
              slackConnectActions=${m}
            />
          </${mi}>
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
                <div key=${Wc(y)} className="flex flex-col gap-3">
                  <${ci}
                    ext=${y}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${u}
                  />
                  ${N_(y)&&l`<${__}
                    slackConnectActions=${m}
                  />`}
                  ${(y.onboarding_state==="pairing_required"||y.onboarding_state==="pairing")&&l` <${S_} channel=${Wc(y)} /> `}
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
                <${Ir}
                  key=${Wc(y)}
                  entry=${y}
                  onInstall=${o}
                  isBusy=${u}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function mi({name:e,description:t,enabled:a,detail:n,children:r,statusLabel:s=a?"on":"off",statusTone:i=a?"success":"muted"}){return l`
    <div
      className="border-t border-white/[0.06] py-4 first:border-0 first:pt-0"
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-iron-200">${e}</span>
            <${U}
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
  `}function E_({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=k(),s=e?.displayName||e?.packageRef?.id||"Extension",{secrets:i=[],fields:o=[],onboarding:u,isLoading:c,error:d}=y_(e?.packageRef),[f,m]=h.default.useState({}),[p,b]=h.default.useState({}),y=x_(e?.packageRef),$=b_(e?.packageRef,N=>{N.success!==!1&&(n&&n(N),a())}),g=h.default.useCallback(()=>{let N={};for(let[A,M]of Object.entries(f)){let P=(M||"").trim();P&&(N[A]=P)}$.mutate({secrets:N,fields:p})},[f,p,$]),v=h.default.useCallback(N=>{let A=window.open("about:blank","_blank","width=600,height=600");A&&(A.opener=null),y.mutate({secret:N,popup:A})},[y]),w=i.filter(N=>(N.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,S=wh(e),R=ZN({extension:e,secrets:i,fields:o});return c?l`
      <${ed} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(N=>l`<div
                key=${N}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?l`
      <${ed} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")||"Failed to load setup:"} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?l`
      <${ed} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")||"No configuration required for this extension."}
        </p>
      <//>
    `:l`
    <${ed} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
      ${u?.credential_instructions&&l`
        <p className="mb-4 text-sm leading-6 text-iron-300">
          ${u.credential_instructions}
        </p>
      `}
      ${u?.setup_url&&l`
        <a
          href=${u.setup_url}
          target="_blank"
          rel="noopener noreferrer"
          className="mb-4 inline-flex items-center gap-1.5 text-sm text-signal hover:underline"
        >
          Get credentials
          <${D} name="bolt" className="h-3.5 w-3.5" />
        </a>
      `}

      <div className="space-y-4">
        ${i.map(N=>l`
            <div key=${N.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                ${N.prompt||N.name}
                ${N.optional&&l`
                  <span className="font-mono text-[10px] text-iron-700"
                    >${r("common.optional")||"optional"}</span
                  >
                `}
                ${N.provided&&l`
                  <span className="font-mono text-[10px] text-mint"
                    >${r("common.configured")||"configured"}</span
                  >
                `}
              </label>
              ${(N.setup?.kind||"manual_token")==="oauth"?l`
                    <div className="flex items-center justify-between gap-3 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2">
                      <span className="text-xs text-iron-300">
                        ${N.provided?r("extensions.authConfigured")||"Authorization is configured.":r("extensions.authPopup")||"Authorize this provider in a browser popup."}
                      </span>
                      <${E}
                        variant=${N.provided?"secondary":"primary"}
                        onClick=${()=>v(N)}
                        disabled=${y.isPending}
                      >
                        ${y.isPending?r("extensions.opening"):N.provided?r("extensions.reconnect"):r("extensions.authorize")}
                      <//>
                    </div>
                  `:l`
              <input
                type="password"
                placeholder=${N.provided?"\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (leave blank to keep)":""}
                value=${f[N.name]||""}
                onChange=${A=>m(M=>({...M,[N.name]:A.target.value}))}
                onKeyDown=${A=>A.key==="Enter"&&g()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
              ${N.auto_generate&&!N.provided&&l`
                <p className="mt-1 text-xs text-iron-700">
                  ${r("extensions.autoGenerated")||"Auto-generated if left blank"}
                </p>
              `}
                  `}
            </div>
          `)}
        ${o.map(N=>l`
            <div key=${N.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                ${N.prompt||N.name}
                ${N.optional&&l`
                  <span className="font-mono text-[10px] text-iron-700"
                    >${r("common.optional")||"optional"}</span
                  >
                `}
              </label>
              <input
                type="text"
                placeholder=${N.placeholder||""}
                value=${p[N.name]||""}
                onChange=${A=>b(M=>({...M,[N.name]:A.target.value}))}
                onKeyDown=${A=>A.key==="Enter"&&g()}
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
        <${E} variant="ghost" onClick=${a}>${r("common.cancel")||"Cancel"}<//>
        ${R&&l`
        <${E}
          variant="primary"
          onClick=${()=>t?.(e)}
        >
          Activate
        <//>
        `}
        ${w&&l`
        <${E}
          variant=${R?"secondary":"primary"}
          onClick=${g}
          disabled=${$.isPending}
        >
          ${$.isPending?"Saving\u2026":r("common.save")||"Save"}
        <//>
        `}
      </div>
    <//>
  `}function ed({onClose:e,title:t,children:a}){return h.default.useEffect(()=>{let n=r=>{r.key==="Escape"&&e()};return window.addEventListener("keydown",n),()=>window.removeEventListener("keydown",n)},[e]),l`
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
            <${D} name="close" className="h-4 w-4" />
          </button>
        </div>
        ${a}
      </div>
    </div>
  `}function A_(e){return e.package_ref?.id||""}function T_({mcpServers:e,mcpRegistry:t,onActivate:a,onConfigure:n,onRemove:r,onInstall:s,isBusy:i}){let o=k();return e.length===0&&t.length===0?l`
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
                <${ci}
                  key=${A_(u)}
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
                <${Ir}
                  key=${A_(u)}
                  entry=${u}
                  onInstall=${s}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function kD(e){return e?.package_ref?.id||""}function RD(e){return e.entry||e.extension||{}}function D_({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=k(),[o,u]=h.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(y=>{let $=RD(y);return($.display_name||kD($)).toLowerCase().includes(c)||($.description||"").toLowerCase().includes(c)||($.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,f=d.filter(y=>y.installed&&y.extension),m=d.filter(y=>y.installed&&!y.extension&&y.entry),p=f.length+m.length,b=d.filter(y=>!y.installed&&y.entry);return e.length===0?l`
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
              ${p>0&&l`
                <h3
                  className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
                >
                  ${i("extensions.installed")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${f.map(y=>l`
                      <${ci}
                        key=${y.id}
                        ext=${y.extension||y.entry}
                        onActivate=${a}
                        onConfigure=${n}
                        onRemove=${r}
                        isBusy=${s}
                      />
                    `)}
                  ${m.map(y=>l`
                      <${Ir}
                        key=${y.id}
                        entry=${y.entry}
                        statusLabel=${i("extensions.installed")}
                        isBusy=${s}
                      />
                    `)}
                </div>
              `}

              ${b.length>0&&l`
                <h3
                  className=${["mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal",p>0?"mt-6":""].join(" ")}
                >
                  ${i("ext.registry.availableTitle")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${b.map(y=>l`
                      <${Ir}
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
  `}function Nh(){let{tab:e="registry"}=ut(),[t,a]=h.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:u,connectableChannels:c,isLoading:d,isBusy:f,actionResult:m,clearResult:p,install:b,activate:y,remove:$,invalidate:g}=g_(),v=h.default.useCallback(N=>a(N),[]),x=h.default.useCallback(()=>a(null),[]),w=h.default.useCallback(()=>g(),[g]),S=h.default.useCallback(N=>{N&&(y(N),a(null))},[y]);if(d)return l`
      <div className="flex h-full flex-col overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${[1,2,3].map(N=>l`
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
    `;if(e==="installed")return l`<${ct} to="/extensions/registry" replace />`;let R={channels:l`<${C_}
      status=${n}
      channels=${r}
      connectableChannels=${c}
      channelRegistry=${i}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${b}
      isBusy=${f}
    />`,mcp:l`<${T_}
      mcpServers=${s}
      mcpRegistry=${o}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${b}
      isBusy=${f}
    />`,registry:l`<${D_}
      catalogEntries=${u}
      onInstall=${b}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      isBusy=${f}
    />`};return R[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <${AN} result=${m} onDismiss=${p} />
          ${R[e]}
        </div>
      </div>

      ${t&&l`
        <${E_}
          extension=${t}
          onActivate=${S}
          onClose=${x}
          onSaved=${w}
        />
      `}
    </div>
  `:l`<${ct} to="/extensions/registry" replace />`}var M_=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],O_=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.auto_approve_tools",labelKey:"settings.field.autoApproveTools",descKey:"settings.field.autoApproveToolsDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],L_=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],_h=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","agent.auto_approve_tools","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function P_(e){return String(e||"").trim().toLowerCase()}function U_(e){if(e==null)return"";if(Array.isArray(e))return e.map(U_).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function rt(e,t){let a=P_(e);return a?t.map(U_).join(" ").toLowerCase().includes(a):!0}function fi(e,t,a,n){let r=P_(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(u=>rt(r,[i,u.key,u.labelKey?n(u.labelKey):u.label,u.descKey?n(u.descKey):u.description,t[u.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function CD({visible:e}){let t=k();return e?l`
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      ${t("tools.saved")}
    </span>
  `:null}function ED({checked:e,onChange:t,label:a}){return l`
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
  `}function AD({field:e,value:t,onSave:a,isSaved:n}){let r=k(),[s,i]=h.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",u=e.descKey?r(e.descKey):e.description||"";h.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=h.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let f=parseInt(d,10);isNaN(f)||a(e.key,f)}else if(e.type==="float"){let f=parseFloat(d);isNaN(f)||a(e.key,f)}else a(e.key,d)},[e.key,e.type,a]);return l`
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">${o}</div>
        ${u&&l`<div className="mt-1 text-xs leading-5 text-iron-300">${u}</div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${e.type==="boolean"?l`
              <${ED}
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
        <${CD} visible=${n} />
      </div>
    </div>
  `}function pi({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=k(),o=t?i(t):e||"";return l`
    <${ae} className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${o}</h3>
      <div>
        ${a.map(u=>l`
              <${AD}
                key=${u.key}
                field=${u}
                value=${n[u.key]}
                onSave=${r}
                isSaved=${s[u.key]}
              />
            `)}
      </div>
    <//>
  `}function Rt({query:e}){let t=k();return l`
    <${ae} padding="lg">
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
  `}function j_({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=k();if(n)return l`<${TD} />`;let i=fi(O_,e,r,s);return i.length===0?l`<${Rt} query=${r} />`:l`
    <div className="space-y-5">
      ${i.map(o=>l`
            <${pi}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function TD(){return l`
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
  `}function F_(){let e=q({queryKey:["gateway-status-settings"],queryFn:Qs,staleTime:1e4}),t=q({queryKey:["extensions"],queryFn:k$}),a=q({queryKey:["extension-registry"],queryFn:R$}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(f=>f.kind==="wasm_channel"||f.kind==="channel"),o=s.filter(f=>(f.kind==="wasm_channel"||f.kind==="channel")&&!f.installed),u=r.filter(f=>f.kind==="mcp_server"),c=s.filter(f=>f.kind==="mcp_server"&&!f.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:u,mcpRegistry:c,extensions:r,isLoading:d}}function DD({name:e,description:t,enabled:a,detail:n}){let r=k();return l`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${e}</span>
          <${U}
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
  `}function q_({channel:e,registryEntry:t}){let a=k(),n=t?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},u={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return l`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${n}</span>
          ${s?l`<${U}
                tone=${o[i]||"muted"}
                label=${u[i]||i}
                size="sm"
              />`:l`<${U}
                tone="muted"
                label=${a("channels.available")}
                size="sm"
              />`}
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${r}</div>
      </div>
    </div>
  `}function MD(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function OD({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=MD(e,i).filter(b=>rt(s,[i("channels.builtIn"),b.id,b.name,b.description,b.detail])),u=new Set(t.map(b=>b.name)),c=t.filter(b=>rt(s,[i("channels.messaging"),b.name,b.display_name,b.description,b.onboarding_state])),d=a.filter(b=>!u.has(b.name)).filter(b=>rt(s,[i("channels.messaging"),b.name,b.display_name,b.description])),f=new Set(n.map(b=>b.name)),m=n.filter(b=>rt(s,[i("channels.mcpServers"),b.name,b.display_name,b.description,b.active?i("channels.active"):i("channels.inactive")])),p=r.filter(b=>!f.has(b.name)).filter(b=>rt(s,[i("channels.mcpServers"),b.name,b.display_name,b.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:p}}function z_({searchQuery:e=""}){let t=k(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=F_();if(o)return l`
      <div className="space-y-5">
        <${ae} padding="md">
          <div className="mb-4 h-3 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(p=>l`
              <div
                key=${p}
                className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0"
              >
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="h-6 w-16 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
              </div>
            `)}
        <//>
      </div>
    `;let{builtInChannels:u,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:m}=OD({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return u.length===0&&c.length===0&&d.length===0&&f.length===0&&m.length===0?l`<${Rt} query=${e} />`:l`
    <div className="space-y-5">
      ${u.length>0&&l`
      <${ae} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${u.map(p=>l`
            <${DD}
              key=${p.id}
              name=${p.name}
              description=${p.description}
              enabled=${p.enabled}
              detail=${p.detail}
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
          ${c.map(p=>l`
              <${q_}
                key=${p.name}
                channel=${p}
                registryEntry=${r.find(b=>b.name===p.name)}
              />
            `)}
          ${d.map(p=>l`
              <${q_} key=${p.name} registryEntry=${p} />
            `)}
        <//>
      `}
      ${(f.length>0||m.length>0)&&l`
        <${ae} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.mcpServers")}
          </h3>
          ${f.map(p=>l`
                <div
                  key=${p.name}
                  className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-[var(--v2-text)]"
                        >${p.display_name||p.name}</span
                      >
                      <${U}
                        tone=${p.active?"positive":"muted"}
                        label=${p.active?t("channels.active"):t("channels.inactive")}
                        size="sm"
                      />
                    </div>
                    <div className="mt-1 text-xs text-[var(--v2-text-muted)]">
                      ${p.description||""}
                    </div>
                  </div>
                </div>
              `)}
          ${m.map(p=>l`
                <div
                  key=${p.name}
                  className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-[var(--v2-text)]"
                        >${p.display_name||p.name}</span
                      >
                      <${U}
                        tone="muted"
                        label=${t("channels.available")}
                        size="sm"
                      />
                    </div>
                    <div className="mt-1 text-xs text-[var(--v2-text-muted)]">
                      ${p.description||""}
                    </div>
                  </div>
                </div>
              `)}
        <//>
      `}
    </div>
  `}function B_({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:u,onNearaiWallet:c,onCodexLogin:d,loginBusy:f}){let m=k(),p=e.id===t,b=Ur(e,n),y=Ys(e,n),$=K$(e,n,t,a),g=dc(e,n),v=Q$(e),x=m(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[w,S]=h.default.useState(p),R=h.default.useCallback(()=>S(vt=>!vt),[]);h.default.useEffect(()=>{S(p)},[p]);let N=b?l`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${Bo(e.adapter)} · ${$||e.default_model||m("llm.none")}
      </span>`:l`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${x}
      </span>`,A=e.id==="nearai"||e.id==="openai_codex",M=e.api_key_set===!0||e.has_api_key===!0,P=e.builtin?e.id==="nearai"&&v&&!M?m("llm.addApiKey"):m("llm.configure"):m("common.edit"),G=v&&e.builtin?l`
          <${E}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${r}
            onClick=${()=>i(e)}
          >
            ${P}
          <//>
        `:null,C=!p&&e.id==="nearai"?l`
          ${G}
          <${E} type="button" variant="secondary" size="sm" disabled=${f} onClick=${c}>
            ${m("onboarding.nearWallet")}
          <//>
          <${E} type="button" variant="secondary" size="sm" disabled=${f} onClick=${()=>u("github")}>
            GitHub
          <//>
          <${E} type="button" variant="secondary" size="sm" disabled=${f} onClick=${()=>u("google")}>
            Google
          <//>
        `:!p&&e.id==="openai_codex"?l`
          <${E} type="button" variant="secondary" size="sm" disabled=${f} onClick=${d}>
            ${m("onboarding.codexSignIn")}
          <//>
        `:null,K=!p&&b&&(!A||e.id==="nearai"&&e.has_api_key===!0)?l`
        <${E}
          type="button"
          variant="primary"
          size="sm"
          disabled=${r}
          onClick=${()=>s(e)}
        >
          ${m("llm.use")}
        <//>
      `:null,oe=b?null:l`
        <${E}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${r}
          onClick=${()=>i(e)}
        >
          ${m(g==="api_key"?"llm.addApiKey":"llm.configure")}
        <//>
      `,Se=p?null:K||(A?C:oe),Be=!A&&(e.builtin&&e.id!=="bedrock"||!e.builtin)||e.id==="nearai"&&v;return l`
    <${ae}
      padding="none"
      data-testid="llm-provider-card"
      data-provider-id=${e.id}
      className=${["transition-colors",p?"border-[color-mix(in_srgb,var(--v2-positive-text)_36%,var(--v2-panel-border))]":w?"border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))]":""].join(" ")}
    >
      <div className="flex w-full items-stretch hover:bg-[var(--v2-surface-soft)]">
        <button
          type="button"
          aria-expanded=${w?"true":"false"}
          aria-label=${m(w?"llm.collapseDetails":"llm.expandDetails")}
          data-testid="llm-provider-disclosure"
          onClick=${R}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 px-4 py-3 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)] sm:pl-5 sm:pr-3"
        >
          <span
            className=${["h-2 w-2 shrink-0 rounded-full",p?"bg-[var(--v2-positive-text)]":b?"bg-[var(--v2-accent)]":"bg-[var(--v2-warning-text)]"].join(" ")}
          />
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
            <span className="min-w-0 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
              ${e.name||e.id}
            </span>
            <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">${e.id}</span>
            ${p&&l`<${U} tone="positive" label=${m("llm.active")} size="sm" />`}
            ${e.builtin&&!p&&l`<${U} tone="muted" label=${m("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${N}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 py-3 pr-4 sm:pr-5">
          ${Se}
          <button
            type="button"
            onClick=${R}
            data-testid="llm-provider-chevron"
            aria-label=${m(w?"llm.collapseDetails":"llm.expandDetails")}
            className=${["grid h-7 w-7 place-items-center rounded-md text-[var(--v2-text-faint)] transition-transform hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]",w?"rotate-180":""].join(" ")}
          >
            <${D} name="chevron" className="h-4 w-4" />
          </button>
        </div>
      </div>

      ${w&&l`
        <div data-testid="llm-provider-details" className="border-t border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-4 sm:px-5">
          <div className="grid gap-3 text-xs text-[var(--v2-text-muted)] sm:grid-cols-3">
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${m("llm.adapter")}</div>
              <div className="mt-1 truncate">${Bo(e.adapter)}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${m("llm.baseUrl")}</div>
              <div className="mt-1 truncate font-mono">${y||m("llm.none")}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${m("llm.model")}</div>
              <div className="mt-1 truncate font-mono">${$||m("llm.none")}</div>
            </div>
          </div>

          <div className="mt-4 flex flex-wrap justify-end gap-2 border-t border-[var(--v2-panel-border)] pt-3">
            ${Be&&l`
              <${E}
                type="button"
                variant="secondary"
                size="sm"
                disabled=${r}
                onClick=${()=>i(e)}
              >
                ${P}
              <//>
            `}
            ${!e.builtin&&!p&&l`
              <${E}
                type="button"
                variant="danger"
                size="sm"
                disabled=${r}
                onClick=${()=>o(e)}
              >
                ${m("common.delete")}
              <//>
            `}
          </div>
        </div>
      `}
    <//>
  `}var LD=[{key:"active",labelKey:"llm.groupActive",dotClass:"bg-[var(--v2-positive-text)]"},{key:"ready",labelKey:"llm.groupReady",dotClass:"bg-[var(--v2-accent)]"},{key:"setup",labelKey:"llm.groupSetup",dotClass:"bg-[var(--v2-warning-text)]"}];function PD({label:e,count:t,dotClass:a}){return l`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full "+a} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${t}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `}function I_({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=k(),r=Oc({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=Lc(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return l`<${Rt} query=${a} />`;let u=V$(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return l`
    <${ae} className="p-4 sm:p-6">
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${n("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">${n("llm.providersDesc")}</p>
        </div>
        <${E} type="button" variant="secondary" size="sm" className="gap-2" onClick=${()=>r.openDialog(null)}>
          <${D} name="plus" className="h-3.5 w-3.5" />
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

      <${Mc} login=${i} />

      ${s.isLoading?l`<div className="text-sm text-[var(--v2-text-muted)]">${n("common.loading")}</div>`:s.error?l`<div className="text-sm text-red-200">${n("error.loadFailed",{what:n("llm.providers"),message:s.error.message})}</div>`:l`
            <div className="space-y-1">
              ${LD.flatMap(c=>{let d=u[c.key];return d.length?[l`
                    <section
                      key=${c.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${c.key}
                      className="mb-3"
                    >
                      <${PD}
                        label=${n(c.labelKey)}
                        count=${d.length}
                        dotClass=${c.dotClass}
                      />
                      <div className="space-y-2">
                      ${d.map(f=>l`
                          <${B_}
                            key=${f.id}
                            provider=${f}
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

      <${Dc}
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
  `}function H_({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=k(),{activeProviderId:o,selectedModel:u,providers:c,hasActiveProvider:d}=Js({settings:e,gatewayStatus:t});if(r)return l`<${UD} />`;let f=d?o:"",m=c.find(g=>g.id===o),p=d&&(u||m?.default_model||e.selected_model)||"",b=fi(M_,e,s,i),y=rt(s,[i("inference.provider"),i("inference.backend"),f,i("inference.model"),p]),$=rt(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!y&&!$&&b.length===0?l`<${Rt} query=${s} />`:l`
    <div className="space-y-5">
      ${y&&l`
      <${ae} padding="none" className="p-4 sm:p-5">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${i("inference.provider")}</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${i("inference.backend")}</div>
            <div className="mt-1 flex items-center gap-2">
              <span className="font-mono text-lg font-semibold text-[var(--v2-text-strong)]">${f||i("inference.none")}</span>
              ${d?l`<${U} tone="positive" label=${i("inference.active")} size="sm" />`:l`<${U} tone="muted" label=${i("llm.notConfigured")} size="sm" />`}
            </div>
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${i("inference.model")}</div>
            <div className="mt-1 font-mono text-lg font-semibold text-[var(--v2-text-strong)]">
              ${p||i("inference.none")}
            </div>
          </div>
        </div>
      <//>
      `}

      ${$&&l`
        <${I_}
          settings=${e}
          gatewayStatus=${t}
          searchQuery=${s}
        />
      `}

      ${b.map(g=>l`
            <${pi}
              key=${g.groupKey}
              groupKey=${g.groupKey}
              fields=${g.fields}
              settings=${e}
              onSave=${a}
              savedKeys=${n}
            />
          `)}
    </div>
  `}function sr({className:e=""}){return l`
    <div
      className=${"rounded animate-pulse bg-[var(--v2-surface-muted)] "+e}
    />
  `}function UD(){return l`
    <div className="space-y-5">
      <${ae} padding="md">
        <${sr} className="mb-4 h-3 w-24" />
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${sr} className="h-3 w-16" />
            <${sr} className="mt-2 h-6 w-28" />
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${sr} className="h-3 w-16" />
            <${sr} className="mt-2 h-6 w-40" />
          </div>
        </div>
      <//>
      ${[1,2].map(e=>l`
            <${ae} key=${e} padding="md">
              <${sr} className="mb-4 h-3 w-20" />
              ${[1,2,3].map(t=>l`
                    <div key=${t} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                      <${sr} className="h-4 w-32" />
                      <${sr} className="h-9 w-36" />
                    </div>
                  `)}
            <//>
          `)}
    </div>
  `}function K_({searchQuery:e=""}){let t=k(),{lang:a,setLang:n}=nl(),r=rl.find(i=>i.code===a)||rl[0],s=rl.filter(i=>rt(e,[i.code,i.name,i.native]));return s.length===0?l`<${Rt} query=${e} />`:l`
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
  `}function Q_({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=k();if(n)return l`
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
    `;let i=fi(L_,e,r,s);return i.length===0?l`<${Rt} query=${r} />`:l`
    <div className="space-y-5">
      ${i.map(o=>l`
            <${pi}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function V_(){let e=k(),[t,a]=h.default.useState(!1),n=h.default.useCallback(()=>a(!0),[]),r=h.default.useCallback(()=>a(!1),[]),s=h.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function G_({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=k(),r=V_({gatewayStatus:t,gatewayStatusQuery:a});return e?l`
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

        <${E}
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

    <${ai}
      open=${r.confirmOpen}
      onClose=${r.closeConfirm}
      title=${n("restart.title")}
      size="sm"
    >
      <${ni} className="space-y-3">
        <p className="text-sm text-[var(--v2-text)]">
          ${n("restart.description")}
        </p>
        <div className="rounded-xl border border-copper/25 bg-copper/10 px-3 py-2 text-xs text-copper">
          ${n("restart.warning")}
        </div>
      <//>
      <${ri}>
        <${E}
          type="button"
          variant="ghost"
          size="sm"
          disabled=${r.isRestarting}
          onClick=${r.closeConfirm}
        >
          ${n("restart.cancel")}
        <//>
        <${E}
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

    ${r.isRestarting&&l`
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
  `:null}function Y_(){let e=Z(),t=q({queryKey:["skills"],queryFn:C$}),a=q({queryKey:["skills","pending"],queryFn:U$}),n=()=>{e.invalidateQueries({queryKey:["skills"]})},r=()=>{e.invalidateQueries({queryKey:["skills"]}),e.invalidateQueries({queryKey:["skills","pending"]})},s=B({mutationFn:A$,onSuccess:n}),i=B({mutationFn:D$,onSuccess:n}),o=B({mutationFn:({name:x,content:w})=>T$(x,{content:w}),onSuccess:r}),u=B({mutationFn:({name:x,enabled:w})=>M$(x,w),onSuccess:n}),c=B({mutationFn:x=>O$(x),onSuccess:n}),d=B({mutationFn:x=>L$(x),onSuccess:n}),f=B({mutationFn:x=>P$(x),onSuccess:n}),m=B({mutationFn:x=>j$(x),onSuccess:r}),p=B({mutationFn:x=>F$(x),onSuccess:r}),b=t.data?.skills||[],y=t.data?.auto_activate_learned!==!1,$=t.data?.learning_enabled!==!1,g=t.data?.require_review!==!1,v=a.data?.pending||[];return{skills:b,query:t,autoActivateLearned:y,learningEnabled:$,requireReview:g,pendingSkills:v,pendingQuery:a,fetchSkillContent:E$,installSkill:s.mutateAsync,removeSkill:i.mutateAsync,updateSkill:o.mutateAsync,setSkillAutoActivate:u.mutateAsync,setAutoActivateLearned:c.mutateAsync,setLearningEnabled:d.mutateAsync,setRequireReview:f.mutateAsync,approvePendingSkill:m.mutateAsync,discardPendingSkill:p.mutateAsync,isInstalling:s.isPending,isRemoving:i.isPending,isUpdating:o.isPending,isSettingAutoActivate:u.isPending,isSettingAutoActivateLearned:c.isPending,isSettingLearningEnabled:d.isPending,isSettingRequireReview:f.isPending,isApprovingPending:m.isPending,isDiscardingPending:p.isPending}}function J_({skill:e,onEdit:t,onRemove:a,onUpdate:n,onSetAutoActivate:r,isRemoving:s,isUpdating:i,isSettingAutoActivate:o,globalAutoActivate:u=!0}){let c=k(),d=e.name||e.id,f=e.trust||e.trust_level||"installed",m=e.source_kind||"installed",p=!!e.can_edit,b=!!e.can_delete,y=e.auto_activate!==!1,g=!!e.is_learned&&!u,[v,x]=h.default.useState(!1),[w,S]=h.default.useState(""),[R,N]=h.default.useState(""),[A,M]=h.default.useState(!1);h.default.useEffect(()=>{v||(S(""),N(""))},[v]);let P=h.default.useCallback(async()=>{M(!0),N("");try{let C=await t(d);S(C?.content||""),x(!0)}catch(C){N(C.message||c("skills.contentLoadFailed"))}finally{M(!1)}},[d,t,c]),G=h.default.useCallback(async()=>{(await n(d,w))?.success&&x(!1)},[w,d,n]);return l`
    <div className="ext-card border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text)]">${d}</span>
            <${U}
              tone=${String(f).toLowerCase()==="trusted"?"positive":"muted"}
              label=${f}
              size="sm"
            />
            <${U}
              tone=${m==="system"?"positive":"muted"}
              label=${c(`skills.source.${m}`)}
              size="sm"
            />
            ${e.pending_review&&l`<${U}
              tone="warning"
              label=${c("skills.pendingReview")}
              size="sm"
            />`}
            ${e.version&&l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v${e.version}</span>`}
          </div>

          ${e.description&&l`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${e.description}</div>`}

          ${v?l`
                <div className="mt-3">
                  <${$c}
                    rows=${12}
                    value=${w}
                    className="font-mono text-xs leading-5"
                    onInput=${C=>S(C.currentTarget.value)}
                  />
                </div>
              `:l`<${jD} skill=${e} />`}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          ${p&&!v&&l`
            <${E}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${i||A}
              title=${c("skills.edit")}
              onClick=${P}
            >
              <${D} name="file" className="h-4 w-4" />
              ${c(A?"skills.loading":"skills.edit")}
            <//>
          `}
          ${v&&l`
            <${E}
              type="button"
              variant="ghost"
              size="sm"
              disabled=${i}
              onClick=${()=>{S(""),x(!1)}}
            >
              <${D} name="close" className="h-4 w-4" />
              ${c("skills.cancel")}
            <//>
            <${E}
              type="button"
              variant="primary"
              size="sm"
              disabled=${i}
              onClick=${G}
            >
              <${D} name="check" className="h-4 w-4" />
              ${c(i?"skills.saving":"skills.save")}
            <//>
          `}
          ${p&&!v&&l`
            <${E}
              type="button"
              variant=${y?"secondary":"ghost"}
              size="sm"
              disabled=${o||g}
              title=${c(g?"skills.autoActivate.titlePaused":y?"skills.autoActivate.titleOn":"skills.autoActivate.titleOff")}
              onClick=${()=>r(d,!y)}
            >
              <${D} name=${y?"check":"close"} className="h-4 w-4" />
              ${c(y?g?"skills.autoActivate.onPaused":"skills.autoActivate.on":"skills.autoActivate.off")}
            <//>
          `}
          ${b&&!v&&l`
            <${E}
              type="button"
              variant="danger"
              size="sm"
              disabled=${s}
              title=${c("skills.delete")}
              onClick=${()=>a(d)}
            >
              <${D} name="trash" className="h-4 w-4" />
              ${c("skills.delete")}
            <//>
          `}
        </div>
      </div>
      ${R&&l`<p className="mt-2 text-xs text-[var(--v2-danger-text)]">${R}</p>`}
    </div>
  `}function jD({skill:e}){let t=k();return l`
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
        ${e.has_requirements&&l`<${kh}>requirements.txt<//>`}
        ${e.has_scripts&&l`<${kh}>scripts/<//>`}
        ${e.install_source_url&&l`<${kh}>${t("skills.imported")}<//>`}
      </div>
    `}
  `}function kh({children:e}){return l`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${e}
    </span>
  `}function X_({skill:e,onApprove:t,onDiscard:a,isApproving:n,isDiscarding:r}){let s=k(),[i,o]=h.default.useState(!1),u=e.kind==="evolution",c=e.name,d=n||r;return l`
    <div className="border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text)]">${c}</span>
            <${U}
              tone=${u?"muted":"positive"}
              label=${s(u?"skills.learning.review.proposedBadge":"skills.learning.review.newBadge")}
              size="sm"
            />
          </div>
          ${e.description&&l`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${e.description}</div>`}
          <div className="mt-1 text-xs text-[var(--v2-text-muted)]">
            ${s(u?"skills.learning.review.evolutionDescription":"skills.learning.review.newDescription")}
          </div>
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          <${E}
            type="button"
            variant="secondary"
            size="sm"
            onClick=${()=>o(f=>!f)}
          >
            <${D} name="file" className="h-4 w-4" />
            ${s(i?"skills.learning.review.hide":"skills.learning.review.preview")}
          <//>
          <${E}
            type="button"
            variant="primary"
            size="sm"
            disabled=${d}
            onClick=${()=>t(c)}
          >
            <${D} name="check" className="h-4 w-4" />
            ${s(u?"skills.learning.review.applyUpdate":"skills.learning.review.approve")}
          <//>
          <${E}
            type="button"
            variant="danger"
            size="sm"
            disabled=${d}
            onClick=${()=>a(c)}
          >
            <${D} name="trash" className="h-4 w-4" />
            ${s("skills.learning.review.discard")}
          <//>
        </div>
      </div>

      ${i&&l`
        <div className="mt-3 space-y-3">
          ${u?l`
                <${Rh}
                  label=${s("skills.learning.review.yourVersion")}
                  value=${e.current_content}
                />
                <${Rh}
                  label=${s("skills.learning.review.proposedVersion")}
                  value=${e.proposed_content||""}
                />
              `:l`<${Rh}
                label=${s("skills.learning.review.skillContent")}
                value=${e.current_content}
              />`}
        </div>
      `}
    </div>
  `}function Rh({label:e,value:t}){return l`
    <div>
      <div className="mb-1 font-mono text-[10px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </div>
      <pre className=${"max-h-64 overflow-auto whitespace-pre-wrap rounded-lg border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3 font-mono text-xs leading-5 text-[var(--v2-text-muted)]"}>${t}</pre>
    </div>
  `}function Z_({onInstall:e,isInstalling:t}){let a=k(),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState({name:"",content:""}),[c,d]=h.default.useState(""),[f,m]=h.default.useState(""),p=h.default.useCallback((y,$)=>{u(g=>!g[y]||!$.trim()?g:{...g,[y]:""})},[]),b=h.default.useCallback(async()=>{let y=FD({name:n,content:s}),$=qD(y,a);if($.name||$.content){u($),d(""),m("");return}u({name:"",content:""}),d(""),m("");try{let g=await e(y);if(!g?.success){d(g?.message||a("skills.installFailed"));return}r(""),i(""),m(g.message||a("skills.installedSuccess",{name:y.name}))}catch(g){d(g.message||a("skills.installFailed"))}},[s,n,e,a]);return l`
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

      <${$n} label=${a("skills.name")} error=${o.name} required>
        <${Mt}
          size="sm"
          error=${!!o.name}
          aria-invalid=${o.name?"true":void 0}
          value=${n}
          placeholder=${a("skills.namePlaceholder")}
          onInput=${y=>{let $=y.currentTarget.value;r($),p("name",$)}}
        />
      <//>

      <${$n}
        className="mt-3"
        label=${a("skills.content")}
        error=${o.content}
        hint=${a("skills.contentHint")}
        required
      >
        <${$c}
          rows=${5}
          error=${!!o.content}
          aria-invalid=${o.content?"true":void 0}
          value=${s}
          placeholder=${a("skills.contentPlaceholder")}
          onInput=${y=>{let $=y.currentTarget.value;i($),p("content",$)}}
        />
      <//>

      ${c&&l`<p className="mt-3 text-sm text-[var(--v2-danger-text)]">${c}</p>`}
      ${f&&l`<p className="mt-3 text-sm text-[var(--v2-positive-text)]">${f}</p>`}

      <div className="mt-4 flex justify-end">
        <${E} type="button" size="sm" disabled=${t} onClick=${b}>
          <${D} name="upload" className="h-4 w-4" />
          ${a(t?"skills.installing":"skills.install")}
        <//>
      </div>
    <//>
  `}function FD({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function qD(e,t){return{name:e.name?"":t("skills.nameRequired"),content:e.content?"":t("skills.contentRequired")}}function W_({searchQuery:e=""}){let t=k(),{skills:a,query:n,autoActivateLearned:r,learningEnabled:s,requireReview:i,pendingSkills:o,fetchSkillContent:u,installSkill:c,removeSkill:d,updateSkill:f,setSkillAutoActivate:m,setAutoActivateLearned:p,setLearningEnabled:b,setRequireReview:y,approvePendingSkill:$,discardPendingSkill:g,isInstalling:v,isRemoving:x,isUpdating:w,isSettingAutoActivate:S,isSettingAutoActivateLearned:R,isSettingLearningEnabled:N,isSettingRequireReview:A,isApprovingPending:M,isDiscardingPending:P}=Y_(),[G,C]=h.default.useState(""),[Y,K]=h.default.useState(""),oe=h.default.useCallback(async(se,J)=>{C(""),K("");try{let I=await se(J);if(!I?.success){C(I?.message||t("skills.updateFailed"));return}K(I.message)}catch(I){C(I.message||t("skills.updateFailed"))}},[t]),Se=h.default.useCallback(async se=>{if(window.confirm(t("skills.confirmDelete",{name:se}))){C(""),K("");try{let J=await d(se);if(!J?.success){C(J?.message||t("skills.removeFailed"));return}K(J.message||t("skills.removed",{name:se}))}catch(J){C(J.message||t("skills.removeFailed"))}}},[d,t]),Be=h.default.useCallback(async(se,J)=>{if(!J.trim())return C(t("skills.contentRequired")),K(""),{success:!1,message:t("skills.contentRequired")};C(""),K("");try{let I=await f({name:se,content:J});return I?.success?(K(I.message||t("skills.updated",{name:se})),I):(C(I?.message||t("skills.updateFailed")),I)}catch(I){let fe=I.message||t("skills.updateFailed");return C(fe),{success:!1,message:fe}}},[t,f]),vt=h.default.useCallback(async(se,J)=>{C(""),K("");try{let I=await m({name:se,enabled:J});if(!I?.success){C(I?.message||t("skills.updateFailed"));return}K(I.message)}catch(I){C(I.message||t("skills.updateFailed"))}},[m,t]),Ze=h.default.useCallback(se=>oe(b,se),[oe,b]),gt=h.default.useCallback(se=>oe(y,se),[oe,y]),xa=h.default.useCallback(se=>oe(p,se),[oe,p]),Ta=h.default.useCallback(async se=>{C(""),K("");try{let J=await $(se);if(!J?.success){C(J?.message||t("skills.updateFailed"));return}K(J.message)}catch(J){C(J.message||t("skills.updateFailed"))}},[$,t]),oa=h.default.useCallback(async se=>{if(window.confirm(t("skills.confirmDiscardPending",{name:se}))){C(""),K("");try{let J=await g(se);if(!J?.success){C(J?.message||t("skills.updateFailed"));return}K(J.message)}catch(J){C(J.message||t("skills.updateFailed"))}}},[g,t]),$t;if(n.isLoading)$t=l`
      <${ae} padding="md">
          <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(se=>l`
            <div key=${se} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
              <div>
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              </div>
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
        <//>
    `;else if(n.error)$t=l`
      <${ae} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">${t("skills.failedLoad",{message:n.error.message})}</p>
        <//>
    `;else{let se=a.filter(I=>rt(e,[I.name,I.id,I.description,I.keywords,I.trust_level,I.source_kind,I.version])),J=HD(se);a.length===0?$t=l`
        <${ae} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:se.length===0?$t=l`<${Rt} query=${e} />`:$t=l`
        <div id="skills-list">
          ${J.map(I=>l`
              <${ID}
                key=${I.id}
                title=${t(I.labelKey)}
                skills=${I.skills}
                globalAutoActivate=${r}
                onEdit=${u}
                onRemove=${Se}
                onUpdate=${Be}
                onSetAutoActivate=${vt}
                isRemoving=${x}
                isUpdating=${w}
                isSettingAutoActivate=${S}
              />
            `)}
        </div>
      `}return l`
    <div className="space-y-4">
      <${zD}
        learningEnabled=${s}
        requireReview=${i}
        autoActivateLearned=${r}
        isSettingLearningEnabled=${N}
        isSettingRequireReview=${A}
        isSettingAutoActivateLearned=${R}
        onSetLearningEnabled=${Ze}
        onSetRequireReview=${gt}
        onSetAutoActivateLearned=${xa}
      />
      <${BD}
        pending=${o}
        onApprove=${Ta}
        onDiscard=${oa}
        isApproving=${M}
        isDiscarding=${P}
      />
      <${Z_} onInstall=${c} isInstalling=${v} />
      <${KD} error=${G} result=${Y} />
      ${$t}
    </div>
  `}function zD({learningEnabled:e,requireReview:t,autoActivateLearned:a,isSettingLearningEnabled:n,isSettingRequireReview:r,isSettingAutoActivateLearned:s,onSetLearningEnabled:i,onSetRequireReview:o,onSetAutoActivateLearned:u}){let c=k();return l`
    <div className="space-y-3">
      <${Ch}
        title=${c("skills.learning.selfLearning.title")}
        summary=${c(e?"skills.learning.selfLearning.summaryOn":"skills.learning.selfLearning.summaryOff")}
        labelOn=${c("skills.learning.selfLearning.labelOn")}
        labelOff=${c("skills.learning.selfLearning.labelOff")}
        enabled=${e}
        isSaving=${n}
        onToggle=${i}
      />
      <${Ch}
        title=${c("skills.learning.review.title")}
        summary=${c(t?"skills.learning.review.summaryOn":"skills.learning.review.summaryOff")}
        labelOn=${c("skills.learning.review.labelOn")}
        labelOff=${c("skills.learning.review.labelOff")}
        enabled=${t}
        isSaving=${r}
        onToggle=${o}
      />
      <${Ch}
        title=${c("skills.learning.activation.title")}
        summary=${c(a?"skills.learning.activation.summaryOn":"skills.learning.activation.summaryOff")}
        labelOn=${c("skills.learning.activation.labelOn")}
        labelOff=${c("skills.learning.activation.labelOff")}
        enabled=${a}
        isSaving=${s}
        onToggle=${u}
        dangerWhenOff=${!0}
      />
    </div>
  `}function Ch({title:e,summary:t,labelOn:a,labelOff:n,enabled:r,isSaving:s,onToggle:i,dangerWhenOff:o=!1}){return l`
    <${ae} padding="md" style=${o&&!r?{background:"var(--v2-danger-soft)"}:void 0}>
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="text-sm font-medium text-[var(--v2-text-strong)]">${e}</div>
          <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${t}</div>
        </div>
        <div className="shrink-0">
          <${E}
            type="button"
            variant=${r?"secondary":"ghost"}
            size="sm"
            disabled=${s}
            onClick=${()=>i(!r)}
          >
            ${r?a:n}
          <//>
        </div>
      </div>
    <//>
  `}function BD({pending:e,onApprove:t,onDiscard:a,isApproving:n,isDiscarding:r}){let s=k();return!e||e.length===0?null:l`
    <${ae} padding="md">
      <h3 className="mb-1 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${s("skills.learning.review.pendingTitle",{count:e.length})}
      </h3>
      <p className="mb-3 text-xs text-[var(--v2-text-muted)]">
        ${s("skills.learning.review.pendingDescription")}
      </p>
      ${e.map(i=>l`
          <${X_}
            key=${`${i.kind}:${i.name}`}
            skill=${i}
            onApprove=${t}
            onDiscard=${a}
            isApproving=${n}
            isDiscarding=${r}
          />
        `)}
    <//>
  `}function ID({title:e,skills:t,globalAutoActivate:a,onEdit:n,onRemove:r,onUpdate:s,onSetAutoActivate:i,isRemoving:o,isUpdating:u,isSettingAutoActivate:c}){return t.length===0?null:l`
    <${ae} padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${e}
      </h3>
      ${t.map(d=>l`
          <${J_}
            key=${`${d.source_kind||"skill"}:${d.name||d.id}`}
            skill=${d}
            globalAutoActivate=${a}
            onEdit=${n}
            onRemove=${r}
            onUpdate=${s}
            onSetAutoActivate=${i}
            isRemoving=${o}
            isUpdating=${u}
            isSettingAutoActivate=${c}
          />
        `)}
    <//>
  `}function HD(e){let t=[{id:"user",labelKey:"skills.group.user",skills:[]},{id:"system",labelKey:"skills.group.system",skills:[]},{id:"workspace",labelKey:"skills.group.workspace",skills:[]}],a=t[0];for(let n of e){let r=n.source_kind||"";(r==="system"?t[1]:r==="workspace"?t[2]:a).skills.push(n)}return t.filter(n=>n.skills.length>0)}function KD({error:e,result:t}){return!e&&!t?null:l`
    <div className="rounded-xl px-4 py-3 text-sm" style=${e?{background:"var(--v2-danger-soft)",color:"var(--v2-danger-text)",border:"1px solid var(--v2-danger-soft)"}:{background:"var(--v2-surface-soft)",color:"var(--v2-positive-text)",border:"1px solid var(--v2-card-border)"}}>
      ${e||t}
    </div>
  `}function td(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function ek(){let e=Z(),t=q({queryKey:["settings-tools"],queryFn:N$}),a=t.data?.tools||[],[n,r]=h.default.useState({}),s=B({mutationFn:async({name:o,state:u})=>td(await _$(o,u),"Save failed"),onSuccess:(o,{name:u,state:c})=>{e.setQueryData(["settings-tools"],d=>d&&{...d,tools:d.tools.map(f=>f.name===u?{...f,state:c}:f)}),r(d=>({...d,[u]:!0})),setTimeout(()=>r(d=>({...d,[u]:!1})),2e3)}}),i=h.default.useCallback((o,u)=>s.mutate({name:o,state:u}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}function QD({tool:e,onPermissionChange:t,isSaved:a}){let n=k(),r=[{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],s=e.locked,i=r.find(u=>u.value===e.state)||r[1],o=e.state===e.default_state;return l`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="flex min-w-0 items-center gap-3">
        ${s&&l`<${D}
          name="lock"
          className="h-3.5 w-3.5 shrink-0 text-[var(--v2-text-faint)]"
        />`}
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate font-mono text-sm text-[var(--v2-text)]"
              >${e.name}</span
            >
            ${o&&l`
              <span
                className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]"
              >
                ${n("tools.default")}
              </span>
            `}
          </div>
          ${e.description&&l`
            <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">
              ${e.description}
            </div>
          `}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${s?l`<${U} tone=${i.tone} label=${i.label} size="sm" />`:l`
              <select
                value=${e.state}
                onChange=${u=>t(e.name,u.target.value)}
                aria-label=${n("tools.permissionFor",{name:e.name})}
                className="v2-select h-8 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2.5 font-mono text-xs text-[var(--v2-text-strong)] outline-none focus:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]"
              >
                ${r.map(u=>l`<option key=${u.value} value=${u.value}>
                      ${u.label}
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
  `}function tk({searchQuery:e=""}){let t=k(),{tools:a,query:n,setPermission:r,savedTools:s}=ek();if(n.isLoading)return l`
      <${ae} padding="md">
        <div className="mb-4 h-3 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
        ${[1,2,3,4,5].map(o=>l`
            <div
              key=${o}
              className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-3.5 first:border-0"
            >
              <div className="h-4 w-36 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              <div className="h-8 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
      <//>
    `;if(n.error)return l`
      <${ae} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("tools.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let i=a.filter(o=>rt(e,[o.name,o.description,o.state,o.default_state,o.locked?t("tools.disabled"):""]));return l`
    <div className="space-y-4">
      ${e&&l`
        <div className="flex justify-end">
          <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">
            ${i.length} / ${a.length}
          </span>
        </div>
      `}

      <${ae} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("tools.permissions")}
        </h3>
        ${i.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("tools.noMatch")}
            </p>`:i.map(o=>l`
                  <${QD}
                    key=${o.name}
                    tool=${o}
                    onPermissionChange=${r}
                    isSaved=${s[o.name]}
                  />
                `)}
      <//>
    </div>
  `}function ak(e){return(Number(e)||0).toFixed(2)}function VD(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function nk(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function Hr({label:e,value:t,description:a}){return l`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&l`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function rk({searchQuery:e=""}){let t=k(),{credits:a,query:n,authorize:r}=pc();if(!rt(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return l`<${Rt} query=${e} />`;let s;if(n.isLoading)s=l`
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
        <${Hr}
          label=${t("traceCommons.enrollment")}
          value=${a.enrolled?t("traceCommons.enrolled"):t("traceCommons.notEnrolled")}
        />
        <${Hr}
          label=${t("traceCommons.pendingCredit")}
          description=${t("traceCommons.pendingCreditDesc")}
          value=${ak(a.pending_credit)}
        />
        <${Hr}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${ak(a.final_credit)}
        />
        <${Hr}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${VD(a.delayed_credit_delta)}
        />
        <${Hr}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${Hr}
          label=${t("traceCommons.lastSubmission")}
          value=${nk(a.last_submission_at,t)}
        />
        <${Hr}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${nk(a.last_credit_sync_at,t)}
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
  `}function sk(){let e=Z(),t=q({queryKey:["admin-users"],queryFn:B$,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=B({mutationFn:I$,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=B({mutationFn:({id:i,payload:o})=>H$(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function GD({onCreate:e,isCreating:t,error:a}){let n=k(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState("member"),[d,f]=h.default.useState(!1),m=p=>{p.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:u},{onSuccess:()=>{s(""),o(""),f(!1)}})};return d?l`
    <${ae} padding="md">
      <h3
        className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${n("users.newUser")}
      </h3>
      <form onSubmit=${m} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <${$n} label=${n("users.displayName")} htmlFor="user-name">
            <${Mt}
              id="user-name"
              type="text"
              value=${r}
              onChange=${p=>s(p.target.value)}
              required
            />
          <//>
          <${$n} label=${n("users.email")} htmlFor="user-email">
            <${Mt}
              id="user-email"
              type="email"
              value=${i}
              onChange=${p=>o(p.target.value)}
            />
          <//>
        </div>
        <${$n} label=${n("users.role")} htmlFor="user-role">
          <select
            id="user-role"
            value=${u}
            onChange=${p=>c(p.target.value)}
            className="v2-select h-9 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 text-sm text-[var(--v2-text-strong)] outline-none focus:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]"
          >
            <option value="member">${n("users.member")}</option>
            <option value="admin">${n("users.admin")}</option>
          </select>
        <//>
        ${a&&l` <p className="text-sm text-[var(--v2-danger-text)]">${a.message}</p> `}
        <div className="flex gap-2">
          <${E} type="submit" disabled=${t}>
            ${n(t?"users.creating":"users.createUser")}
          <//>
          <${E}
            variant="ghost"
            type="button"
            onClick=${()=>f(!1)}
            >${n("users.cancel")}<//
          >
        </div>
      </form>
    <//>
  `:l`
      <${E} variant="secondary" onClick=${()=>f(!0)}>
        <${D} name="plus" className="mr-2 h-4 w-4" />
        ${n("users.addUser")}
      <//>
    `}function YD({user:e}){let t=k(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return l`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]"
            >${e.display_name||e.id}</span
          >
          <${U}
            tone=${n}
            label=${e.role==="admin"?t("users.admin"):t("users.member")}
            size="sm"
          />
          <${U} tone=${a} label=${e.status||"active"} size="sm" />
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
  `}function ik({searchQuery:e=""}){let t=k(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=sk();if(n.isLoading)return l`
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
          <${D} name="lock" className="h-5 w-5 text-[var(--v2-text-faint)]" />
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
    `;let u=a.filter(c=>rt(e,[c.id,c.display_name,c.email,c.role,c.status,c.last_active]));return l`
    <div className="space-y-5">
      <${GD}
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
            </p>`:u.map(c=>l`<${YD} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function ok(){let e=Z(),t=q({queryKey:["settings-export"],queryFn:p$,staleTime:3e4}),a=t.data?.settings||{},[n,r]=h.default.useState({}),[s,i]=h.default.useState(!1),o=B({mutationFn:async({key:f,value:m})=>td(await h$(f,m),"Save failed"),onSuccess:(f,{key:m,value:p})=>{e.setQueryData(["settings-export"],b=>{if(!b)return b;let y={...b,settings:{...b.settings}};return p==null?delete y.settings[m]:y.settings[m]=p,y}),r(b=>({...b,[m]:!0})),setTimeout(()=>r(b=>({...b,[m]:!1})),2e3),_h.has(m)&&i(!0)}}),u=h.default.useCallback((f,m)=>o.mutate({key:f,value:m}),[o]),c=B({mutationFn:v$,onSuccess:(f,m)=>{e.invalidateQueries({queryKey:["settings-export"]}),Object.keys(m?.settings||{}).some(b=>_h.has(b))&&i(!0)}}),d=h.default.useCallback(f=>c.mutateAsync(f),[c]);return{settings:a,query:t,save:u,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error||c.error}}function Eh(){let e=k(),{tab:t}=ut(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=Ka(),s=r?"inference":"language",i=t||s,{settings:o,query:u,save:c,savedKeys:d,needsRestart:f,saveError:m}=ok(),[p,b]=h.default.useState("");h.default.useEffect(()=>{b("")},[i]);let y=u.isLoading,$={inference:l`<${H_}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,agent:l`<${j_}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,channels:l`<${z_} searchQuery=${p} />`,networking:l`<${Q_}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,tools:l`<${tk} searchQuery=${p} />`,skills:l`<${W_} searchQuery=${p} />`,traces:l`<${rk} searchQuery=${p} />`,users:l`<${ik} searchQuery=${p} />`,language:l`<${K_} searchQuery=${p} />`},g=R=>R==="users"||R==="inference",v=R=>Object.prototype.hasOwnProperty.call($,R),x=Object.keys($).filter(R=>r||!g(R)),S=v(s)&&x.includes(s)?s:x[0]||"language";return!v(i)||!r&&g(i)?l`<${ct} to=${`/settings/${S}`} replace />`:l`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${f&&l`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${G_}
                visible=${!0}
                gatewayStatus=${a}
                gatewayStatusQuery=${n}
              />
            </div>`}

            ${m&&l`
              <div
                className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
              >
                ${e("error.saveFailed",{message:m.message})}
              </div>
            `}

            ${$[i]}
          </div>
        </div>
      </div>
    </div>
  `}var Ah=Object.freeze({todo:!0});function lk(){return Promise.resolve({users:[],total:0,...Ah})}function uk(e){return Promise.resolve(null)}function ck(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function dk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function mk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function fk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function pk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function hk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function vk(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...Ah})}function gk(e="day",t){return Promise.resolve({entries:[],...Ah})}function yk(){return q({queryKey:["admin","usage-summary"],queryFn:vk,refetchInterval:3e4})}function ad(e="day",t){return q({queryKey:["admin","usage",e,t],queryFn:()=>gk(e,t),refetchInterval:3e4})}function hi(){let e=Z(),t=q({queryKey:["admin","users"],queryFn:lk,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=B({mutationFn:ck,onSuccess:s}),o=B({mutationFn:({id:m,payload:p})=>dk(m,p),onSuccess:s}),u=B({mutationFn:m=>mk(m),onSuccess:s}),c=B({mutationFn:m=>fk(m),onSuccess:s}),d=B({mutationFn:m=>pk(m),onSuccess:s}),f=B({mutationFn:({userId:m,name:p})=>hk(m,p)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(m,p)=>o.mutateAsync({id:m,payload:p}),deleteUser:u.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(m,p)=>f.mutateAsync({userId:m,name:p}),newToken:f.data,clearToken:()=>f.reset()}}function bk(e){return q({queryKey:["admin","user",e],queryFn:()=>uk(e),enabled:!!e,refetchInterval:1e4})}function Ja(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function Aa(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function xk(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function ir(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function vi(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function gi(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function yi(e){return e==="admin"?"signal":"muted"}function $k(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function wk(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function Sk(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function Nk(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function _k(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function JD({users:e,onSelectUser:t}){let a=k(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?l`
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
                <td className="py-3 pr-4"><${U} tone=${yi(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${U} tone=${gi(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${ir(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:l`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function kk({onSelectUser:e,onNavigateTab:t}){let a=k(),n=yk(),{users:r,query:s}=hi(),i=n.data||{},o=$k(r),u=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?l`
      <div className="space-y-5">
        <${j} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            ${[1,2,3,4].map(f=>l`<div key=${f} className="v2-skeleton h-28 rounded-lg" />`)}
          </div>
        <//>
      </div>
    `:l`
    <div className="space-y-5">
      <${j} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.systemOverview")}</h3>
          ${i.uptime_seconds!=null&&l`
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:xk(i.uptime_seconds)})}</span>
          `}
        </div>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <${nt}
            label=${a("admin.dashboard.totalUsers")}
            value=${String(o.total)}
            tone=${o.total>0?"success":"muted"}
          />
          <${nt}
            label=${a("admin.dashboard.activeUsers")}
            value=${String(o.active)}
            tone="success"
          />
          <${nt}
            label=${a("admin.dashboard.suspended")}
            value=${String(o.suspended)}
            tone=${o.suspended>0?"danger":"muted"}
          />
          <${nt}
            label=${a("admin.dashboard.admins")}
            value=${String(o.admins)}
            tone="signal"
          />
        </div>
      <//>

      <${j} className="p-5 sm:p-6">
        <h3 className="mb-5 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.usage30d")}</h3>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <${nt}
            label=${a("admin.dashboard.totalJobs")}
            value=${String(c.total||0)}
            tone="muted"
          />
          <${nt}
            label=${a("admin.dashboard.llmCalls")}
            value=${String(u.llm_calls||0)}
            tone="muted"
          />
          <${nt}
            label=${a("admin.dashboard.totalCost")}
            value=${Aa(u.total_cost)}
            tone="signal"
          />
          <${nt}
            label=${a("admin.dashboard.activeJobs")}
            value=${String(c.in_progress||0)}
            tone=${(c.in_progress||0)>0?"success":"muted"}
          />
        </div>
      <//>

      <${j} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.recentUsers")}</h3>
          <button
            onClick=${()=>t("users")}
            className="text-xs text-signal hover:underline"
          >
            ${a("admin.dashboard.viewAll")}
          </button>
        </div>
        <${JD} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var XD=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function ZD({value:e,max:t}){let a=t>0?e/t*100:0;return l`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function Rk({onSelectUser:e}){let t=k(),[a,n]=h.default.useState("day"),r=ad(a),s=r.data?.usage||[],i=Sk(s),o=Nk(s),u=_k(i),c=i.length>0?i[0].cost:0;return r.isLoading?l`
      <${j} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
        <div className="grid gap-4 sm:grid-cols-4">
          ${[1,2,3,4].map(d=>l`<div key=${d} className="v2-skeleton h-28 rounded-lg" />`)}
        </div>
      <//>
    `:l`
    <div className="space-y-5">
      <${j} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.overview")}</h3>
          <div className="flex gap-1">
            ${XD.map(d=>l`
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
                <${nt} label=${t("admin.usage.totalCalls")} value=${u.calls.toLocaleString()} tone="muted" />
                <${nt} label=${t("admin.usage.inputTokens")} value=${Ja(u.input_tokens)} tone="muted" />
                <${nt} label=${t("admin.usage.outputTokens")} value=${Ja(u.output_tokens)} tone="muted" />
                <${nt} label=${t("admin.usage.totalCost")} value=${Aa(u.cost.toFixed(2))} tone="signal" />
              </div>
            `}
      <//>

      ${i.length>0&&l`
        <${j} className="p-5 sm:p-6">
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
                          ${vi(d.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ja(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ja(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${Aa(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${ZD} value=${d.cost} max=${c} />
                      </td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}

      ${o.length>0&&l`
        <${j} className="p-5 sm:p-6">
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
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ja(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ja(d.output_tokens)}</td>
                      <td className="py-3 font-mono text-xs text-iron-100">${Aa(d.cost.toFixed(2))}</td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}
    </div>
  `}function or({label:e,children:t}){return l`
    <div className="flex items-start justify-between gap-4 border-t border-white/[0.06] py-3 first:border-0 first:pt-0">
      <span className="text-xs text-iron-300">${e}</span>
      <span className="text-right text-sm text-iron-100">${t}</span>
    </div>
  `}function Ck({userId:e,onBack:t}){let a=k(),n=bk(e),r=ad("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:u,createToken:c,newToken:d,clearToken:f}=hi(),[m,p]=h.default.useState(null),[b,y]=h.default.useState(!1),$=n.data,g=r.data?.usage||[];if(h.default.useEffect(()=>{$&&m===null&&p($.role)},[$]),n.isLoading)return l`
      <div className="space-y-5">
        <${j} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-2 h-6 w-48 rounded" />
          <div className="v2-skeleton h-4 w-32 rounded" />
        <//>
      </div>
    `;if(n.error)return l`
      <${j} className="p-5 sm:p-6">
        <p className="text-sm text-red-200">${a("error.loadFailed",{what:a("admin.users.user"),message:n.error.message})}</p>
      <//>
    `;if(!$)return null;let v=async()=>{m&&m!==$.role&&await o($.id,{role:m})},x=async()=>{await u($.id),t()},w=async()=>{let S=window.prompt(a("admin.users.tokenNamePrompt",{name:$.display_name||a("admin.users.userFallback")}));S&&await c($.id,S)};return l`
    <div className="space-y-5">
      <button
        onClick=${t}
        className="flex items-center gap-1.5 text-xs text-iron-300 hover:text-white"
      >
        <span>←</span>
        <span>${a("admin.users.backToUsers")}</span>
      </button>

      <${j} className="p-5 sm:p-6">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <h2 className="text-2xl font-semibold tracking-tight text-white">${$.display_name||$.id}</h2>
            <div className="mt-2 flex items-center gap-2">
              <${U} tone=${yi($.role)} label=${$.role||"member"} />
              <${U} tone=${gi($.status)} label=${$.status||"active"} />
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            ${$.status==="active"?l`<${E} variant="secondary" onClick=${()=>s($.id)}>${a("admin.users.suspend")}<//>`:l`<${E} variant="secondary" onClick=${()=>i($.id)}>${a("admin.users.activate")}<//>`}
            <${E} variant="secondary" onClick=${w}>${a("admin.users.createToken")}<//>
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
            <button onClick=${f} className="text-iron-300 hover:text-white">
              <${D} name="close" className="h-4 w-4" />
            </button>
          </div>
        </div>
      `}

      <div className="grid gap-5 lg:grid-cols-2">
        <${j} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.profile")}</h3>
          <${or} label=${a("admin.user.id")}>
            <span className="font-mono text-xs">${$.id}</span>
          <//>
          <${or} label=${a("admin.user.email")}>${$.email||a("admin.user.notSet")}<//>
          <${or} label=${a("admin.user.created")}>${ir($.created_at)}<//>
          <${or} label=${a("admin.user.lastLogin")}>${ir($.last_login_at)}<//>
          ${$.created_by&&l`
            <${or} label=${a("admin.user.createdBy")}>
              <span className="font-mono text-xs">${vi($.created_by)}</span>
            <//>
          `}
        <//>

        <${j} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${or} label=${a("admin.user.jobs")}>${$.job_count??0}<//>
          <${or} label=${a("admin.user.totalCost")}>${Aa($.total_cost)}<//>
          <${or} label=${a("admin.user.lastActive")}>${ir($.last_active_at)}<//>
        <//>
      </div>

      <${j} className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.roleManagement")}</h3>
        <div className="flex items-end gap-3">
          <div>
            <label className="mb-1 block text-xs text-iron-300">${a("admin.user.currentRole")}</label>
            <select
              value=${m||$.role}
              onChange=${S=>p(S.target.value)}
              className="v2-select h-9 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            >
              <option value="member">${a("admin.users.member")}</option>
              <option value="admin">${a("admin.users.admin")}</option>
            </select>
          </div>
          <${E} onClick=${v} disabled=${!m||m===$.role}>
            ${a("admin.user.saveRole")}
          <//>
        </div>
      <//>

      <${j} className="p-5 sm:p-6">
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
                    ${g.map((S,R)=>l`
                        <tr key=${R} className="border-b border-white/[0.06] last:border-0">
                          <td className="py-3 pr-4 font-mono text-xs text-iron-100">${S.model}</td>
                          <td className="py-3 pr-4 font-mono text-xs text-iron-300">${(S.call_count||0).toLocaleString()}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ja(S.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ja(S.output_tokens)}</td>
                          <td className="py-3 font-mono text-xs text-iron-100">${Aa(S.total_cost)}</td>
                        </tr>
                      `)}
                  </tbody>
                </table>
              </div>
            `}
      <//>

      ${b&&l`
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick=${()=>y(!1)}>
          <div className="w-full max-w-md rounded-xl border border-white/10 bg-iron-900 p-6" onClick=${S=>S.stopPropagation()}>
            <h3 className="text-lg font-semibold text-white">${a("admin.users.deleteUserTitle")}</h3>
            <p className="mt-2 text-sm text-iron-300">
              ${a("admin.users.deleteUserDesc",{name:$.display_name})}
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <${E} variant="ghost" onClick=${()=>y(!1)}>${a("admin.users.cancel")}<//>
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
  `}function WD(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function eM({token:e,onDismiss:t}){let a=k(),[n,r]=h.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return l`
    <div className="rounded-xl border border-signal/30 bg-signal/10 p-4 sm:p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-iron-100">${a("admin.users.tokenCreated")}</p>
          <p className="mt-1 text-xs text-iron-300">${a("admin.users.tokenCreatedDesc")}</p>
          <div className="mt-3 flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-md border border-iron-700 bg-iron-800/70 px-3 py-2 font-mono text-xs text-iron-100">
              ${e}
            </code>
            <${E} variant="secondary" onClick=${s}>
              ${a(n?"admin.users.copied":"admin.users.copy")}
            <//>
          </div>
        </div>
        <button onClick=${t} className="text-iron-300 hover:text-iron-100">
          <${D} name="close" className="h-4 w-4" />
        </button>
      </div>
    </div>
  `}function tM({onCreate:e,isCreating:t,error:a}){let n=k(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState("member"),[d,f]=h.default.useState(!1),m=async p=>{p.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:u}),s(""),o(""),f(!1))};return d?l`
    <${j} className="p-5 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${n("admin.users.createUser")}</h3>
      <form onSubmit=${m} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-3">
          <div>
            <label className="mb-1 block text-xs text-iron-300">${n("admin.users.displayName")}</label>
            <input
              type="text"
              value=${r}
              onChange=${p=>s(p.target.value)}
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
              onChange=${p=>o(p.target.value)}
              className="h-9 w-full rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
              placeholder=${n("admin.users.emailPlaceholder")}
            />
          </div>
          <div>
            <label className="mb-1 block text-xs text-iron-300">${n("admin.users.role")}</label>
            <select
              value=${u}
              onChange=${p=>c(p.target.value)}
              className="v2-select h-9 w-full rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            >
              <option value="member">${n("admin.users.member")}</option>
              <option value="admin">${n("admin.users.admin")}</option>
            </select>
          </div>
        </div>
        ${a&&l`<p className="text-sm text-[var(--v2-danger-text)]">${a.message}</p>`}
        <div className="flex gap-2">
          <${E} type="submit" disabled=${t}>
            ${n(t?"admin.users.creating":"admin.users.createUser")}
          <//>
          <${E} variant="ghost" type="button" onClick=${()=>f(!1)}>${n("admin.users.cancel")}<//>
        </div>
      </form>
    <//>
  `:l`
      <${E} variant="secondary" onClick=${()=>f(!0)}>
        <${D} name="plus" className="mr-2 h-4 w-4" />
        ${n("admin.users.newUser")}
      <//>
    `}function aM({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=k();return l`
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick=${r}>
      <div className="w-full max-w-md rounded-xl border border-iron-700 bg-iron-900 p-6" onClick=${i=>i.stopPropagation()}>
        <h3 className="text-lg font-semibold text-iron-100">${e}</h3>
        <p className="mt-2 text-sm text-iron-300">${t}</p>
        <div className="mt-5 flex justify-end gap-2">
          <${E} variant="ghost" onClick=${r}>${s("admin.users.cancel")}<//>
          <button
            onClick=${n}
            className="v2-button inline-flex h-10 items-center justify-center rounded-md bg-[var(--v2-danger-soft)] px-4 text-sm font-semibold text-[var(--v2-danger-text)] hover:bg-[color-mix(in_srgb,var(--v2-danger-soft)_65%,var(--v2-danger-text))]"
          >
            ${a}
          </button>
        </div>
      </div>
    </div>
  `}function nM({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=k();return l`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${U} tone=${yi(e.role)} label=${e.role||"member"} />
          <${U} tone=${gi(e.status)} label=${e.status||"active"} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          ${e.email&&l`<span className="font-mono text-xs text-iron-300">${e.email}</span>`}
          <span className="font-mono text-xs text-iron-700">${vi(e.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-iron-300 sm:inline">
          ${e.job_count!=null?i("admin.users.jobsCount",{count:e.job_count}):""}
          ${e.total_cost!=null?` \xB7 ${Aa(e.total_cost)}`:""}
        </span>
        <span className="hidden text-xs text-iron-700 lg:inline">${ir(e.last_active_at)}</span>
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
  `}function Ek({selectedUserId:e,onSelectUser:t}){let a=k(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:u,updateUser:c,deleteUser:d,suspendUser:f,activateUser:m,createToken:p,newToken:b,clearToken:y}=hi(),[$,g]=h.default.useState(""),[v,x]=h.default.useState("all"),[w,S]=h.default.useState(null),R=wk(n,{search:$,filter:v}),N=WD(a),A=P=>{S({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{f(P),S(null)}})},M=async(P,G)=>{let C=window.prompt(a("admin.users.tokenNamePrompt",{name:G||a("admin.users.userFallback")}));C&&await p(P,C)};return r.isLoading?l`
      <${j} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-3 w-24 rounded" />
        ${[1,2,3].map(P=>l`
          <div key=${P} className="flex items-center justify-between border-t border-iron-700 py-3.5 first:border-0">
            <div className="v2-skeleton h-4 w-32 rounded" />
            <div className="v2-skeleton h-6 w-20 rounded-full" />
          </div>
        `)}
      <//>
    `:s?l`
      <${j} className="p-6 sm:p-8">
        <div className="flex items-center gap-3">
          <${D} name="lock" className="h-5 w-5 text-iron-700" />
          <h3 className="text-lg font-semibold text-iron-100">${a("users.adminRequired")}</h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${a("users.adminRequiredDesc")}
        </p>
      <//>
    `:l`
    <div className="space-y-5">
      ${b&&l`
        <${eM}
          token=${b.token||b.plaintext_token}
          onDismiss=${y}
        />
      `}

      <${tM} onCreate=${i} isCreating=${o} error=${u} />

      <${j} className="p-5 sm:p-6">
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${a("admin.users.title",{count:R.length,total:n.length})}
          </h3>
          <div className="flex items-center gap-2">
            <input
              type="text"
              placeholder=${a("admin.users.searchPlaceholder")}
              value=${$}
              onChange=${P=>g(P.target.value)}
              className="h-8 w-48 rounded-md border border-iron-700 bg-iron-800/70 px-3 text-xs text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
            />
            <div className="flex gap-1">
              ${N.map(P=>l`
                  <button
                    key=${P.value}
                    onClick=${()=>x(P.value)}
                    className=${["rounded-md px-2.5 py-1.5 text-[11px] font-medium",v===P.value?"border border-signal/35 bg-signal/10 text-iron-100":"border border-transparent text-iron-300 hover:text-iron-100"].join(" ")}
                  >
                    ${P.label}
                  </button>
                `)}
            </div>
          </div>
        </div>

        ${R.length===0?l`<p className="py-4 text-sm text-iron-300">${a("admin.users.noMatch")}</p>`:R.map(P=>l`
                <${nM}
                  key=${P.id}
                  user=${P}
                  onSelect=${t}
                  onSuspend=${A}
                  onActivate=${m}
                  onChangeRole=${(G,C)=>c(G,{role:C})}
                  onCreateToken=${M}
                />
              `)}
      <//>

      ${w&&l`
        <${aM}
          title=${w.title}
          message=${w.message}
          confirmLabel=${w.confirmLabel}
          onConfirm=${w.onConfirm}
          onCancel=${()=>S(null)}
        />
      `}
    </div>
  `}function Ak(){let{tab:e="dashboard"}=ut(),t=me(),[a,n]=h.default.useState(null),r=h.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=h.default.useCallback(()=>{n(null)},[]),i={dashboard:l`<${kk}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?l`<${Ck} userId=${a} onBack=${s} />`:l`<${Ek}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:l`<${Rk} onSelectUser=${r} />`};return i[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:l`<${ct} to="/admin/dashboard" replace />`}var rM=2e3,sM=500,iM=2e3,oM=new Set([403,404]),lM=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function uM(e=globalThis.location){let t=new URLSearchParams(e?.search||"");return lM.reduce((a,[n,r,s])=>{let i=t.get(r)?.trim();return i?(a[n]=i,a.active.push({key:n,param:r,labelKey:s,value:i})):a[n]=null,a},{active:[]})}function Tk(){let e=je(),t=h.default.useMemo(()=>uM(e),[e.search]),[a,n]=h.default.useState([]),[r,s]=h.default.useState("all"),[i,o]=h.default.useState(""),[u,c]=h.default.useState(!1),[d,f]=h.default.useState(!0),[m,p]=h.default.useState(!0),[b,y]=h.default.useState(null),[$,g]=h.default.useState(!1),v=h.default.useRef(new Set),x=h.default.useRef(0),w=h.default.useCallback(async()=>{if($)return;let N=++x.current;p(!0);try{let A=await Cx({limit:sM,level:r==="all"?null:r,target:i.trim()||null,threadId:t.threadId,runId:t.runId,turnId:t.turnId,toolCallId:t.toolCallId,toolName:t.toolName,source:t.source});if(N!==x.current)return;let M=v.current,G=gN(A).entries.filter(C=>!M.has(C.id));n(G),y(null)}catch(A){if(N!==x.current)return;if(oM.has(A?.status)){n([]),y(null),g(!0);return}y(A)}finally{N===x.current&&p(!1)}},[$,r,t,i]);h.default.useEffect(()=>{w()},[w]),h.default.useEffect(()=>{if(u||$)return;let N=setInterval(w,rM);return()=>clearInterval(N)},[$,w,u]);let S=h.default.useCallback(()=>{c(N=>!N)},[]),R=h.default.useCallback(()=>{let N=[...v.current,...a.map(A=>A.id)].slice(-iM);v.current=new Set(N),n([])},[a]);return{entries:a,totalCount:a.length,paused:u,togglePause:S,clearEntries:R,levelFilter:r,setLevelFilter:s,targetFilter:i,setTargetFilter:o,autoScroll:d,setAutoScroll:f,serverLevel:null,changeServerLevel:async()=>{},scope:t,status:b?"error":m?"loading":"ready",isLoading:m,error:b}}var cM=["all","trace","debug","info","warn","error"],dM=["trace","debug","info","warn","error"],Dk={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},mM={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function fM({entry:e}){let t=k(),[a,n]=h.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=Dk[e.level]||Dk.info,i=mM[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(u=>!!u.value);return l`
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
  `}function Mk({value:e,onChange:t,options:a,labelKey:n,t:r}){return l`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>l`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function pM({label:e,value:t,scopeKey:a}){return l`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function Ok(){let e=k(),{entries:t,totalCount:a,paused:n,togglePause:r,clearEntries:s,levelFilter:i,setLevelFilter:o,targetFilter:u,setTargetFilter:c,autoScroll:d,setAutoScroll:f,serverLevel:m,changeServerLevel:p,scope:b,isLoading:y,error:$}=Tk(),g=h.default.useRef(null),v=h.default.useRef(!0);h.default.useEffect(()=>{d&&v.current&&g.current&&(g.current.scrollTop=0)},[t,d]);let x=h.default.useCallback(R=>{v.current=R.currentTarget.scrollTop<=48},[]),w=t.length>0,S=b?.active||[];return l`
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${Mk}
          value=${i}
          onChange=${o}
          options=${cM}
          labelKey=${R=>R==="all"?"logs.levelAll":`logs.level.${R}`}
          t=${e}
        />

        <!-- Target filter -->
        <input
          type="text"
          value=${u}
          onInput=${R=>c(R.target.value)}
          placeholder=${e("logs.filterTarget")}
          className="h-8 min-w-[10rem] flex-1 rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-3 text-xs text-[var(--v2-text-base)] placeholder:text-[var(--v2-text-muted)] focus:outline-none focus:ring-1 focus:ring-[var(--v2-accent)]"
        />

        <div className="flex items-center gap-2 ml-auto">
          <span className="hidden tabular-nums text-xs text-[var(--v2-text-muted)] sm:inline">
            ${e("logs.entryCount",{count:a})}
          </span>

          <!-- Auto-scroll toggle -->
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-[var(--v2-text-muted)]">
            <input
              type="checkbox"
              checked=${d}
              onChange=${R=>f(R.target.checked)}
              className="h-3.5 w-3.5 accent-[var(--v2-accent)]"
            />
            ${e("logs.autoScroll")}
          </label>

          <!-- Pause/Resume -->
          <button
            onClick=${r}
            className=${["h-8 rounded-[8px] px-3 text-xs font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)] hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)]":"border border-[var(--v2-panel-border)] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"].join(" ")}
          >
            ${e(n?"logs.resume":"logs.pause")}
          </button>

          <!-- Clear -->
          <button
            onClick=${()=>{confirm(e("logs.confirmClear"))&&s()}}
            className="h-8 rounded-[8px] border border-[var(--v2-panel-border)] px-3 text-xs text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          >
            ${e("logs.clear")}
          </button>
        </div>

        ${S.length>0&&l`
          <div
            data-testid="logs-scope-toolbar"
            className="flex w-full flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]"
          >
            <span className="font-medium text-[var(--v2-text-strong)]">${e("logs.scoped")}</span>
            ${S.map(R=>l`<${pM} key=${R.param} scopeKey=${R.param} label=${e(R.labelKey)} value=${R.value} />`)}
            <a
              href="/v2/logs"
              className="ml-auto rounded-[6px] px-2 py-1 text-xs text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
            >
              ${e("logs.clearScope")}
            </a>
          </div>
        `}

        <!-- Server log level -->
        ${m!=null&&l`
          <div className="flex w-full items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]">
            <span>${e("logs.serverLevel")}</span>
            <${Mk}
              value=${m}
              onChange=${p}
              options=${dM}
              labelKey=${R=>`logs.level.${R}`}
              t=${e}
            />
            <span className="ml-auto tabular-nums">
              ${e("logs.entryCount",{count:a})}
              ${n?l`<span className="ml-1 text-yellow-400">${e("logs.pausedBadge")}</span>`:null}
            </span>
          </div>
        `}
      </div>

      <!-- Log output -->
      <div
        ref=${g}
        onScroll=${x}
        className="min-h-0 flex-1 overflow-y-auto bg-[var(--v2-canvas)]"
      >
        ${$&&w?l`
              <div
                className="sticky top-0 z-10 border-b border-red-500/25 bg-red-950/70 px-4 py-2 text-xs text-red-100 backdrop-blur"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:$.message||$.statusText||"Request failed"})}
              </div>
            `:null}
        ${$&&!w?l`
              <div
                className="flex h-full items-center justify-center px-6 text-center text-sm text-red-300"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:$.message||$.statusText||"Request failed"})}
              </div>
            `:y&&!w?l`
                <div
                  className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
                >
                  ${e("common.loading")}
                </div>
              `:w?t.map(R=>l`<${fM} key=${R.id} entry=${R} />`):l`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function Pk(){return l`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function hM({auth:e}){let t=me(),n=je().state?.from,r=n?`${n.pathname||Pr}${n.search||""}${n.hash||""}`:Pr,s=`/v2${r==="/"?"":r}`,i=h.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?l`<${Pk} />`:e.isAuthenticated?l`<${ct} to=${r} replace />`:l`<${Zw}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function vM({auth:e,children:t}){let a=je();return e.isChecking?l`<${Pk} />`:e.isAuthenticated?t:l`<${ct} to="/login" replace state=${{from:a}} />`}function gM({auth:e}){return l`
    <${vM} auth=${e}>
      <${Rw}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        rebornProjectsEnabled=${e.rebornProjectsEnabled}
        onSignOut=${e.signOut}
      />
    <//>
  `}function Lk({auth:e}){return e.isAdmin?l`<${Ak} />`:l`<${ct} to=${Pr} replace />`}function Uk(){let e=d$();return l`
    <${pp} basename="/v2">
      <${dp}>
        <${ge} path="/login" element=${l`<${hM} auth=${e} />`} />
        <${ge} path="/" element=${l`<${gM} auth=${e} />`}>
          <${ge} index element=${l`<${ct} to=${Pr} replace />`} />
          <${ge} path="overview" element=${l`<${ct} to=${Pr} replace />`} />
          <${ge} path="welcome" element=${l`<${x2} />`} />
          <${ge} path="chat" element=${l`<${nh} />`} />
          <${ge} path="chat/:threadId" element=${l`<${nh} />`} />
          <${ge} path="workspace" element=${l`<${sh} />`} />
          <${ge} path="workspace/*" element=${l`<${sh} />`} />
          <${ge} path="projects" element=${l`<${Xo} />`} />
          <${ge} path="projects/:projectId" element=${l`<${Xo} />`} />
          <${ge} path="projects/:projectId/missions/:missionId" element=${l`<${Xo} />`} />
          <${ge} path="projects/:projectId/threads/:threadId" element=${l`<${Xo} />`} />
          <${ge} path="missions" element=${l`<${oh} />`} />
          <${ge} path="missions/:missionId" element=${l`<${oh} />`} />
          <${ge} path="jobs" element=${l`<${ch} />`} />
          <${ge} path="jobs/:jobId" element=${l`<${ch} />`} />
          <${ge} path="routines" element=${l`<${mh} />`} />
          <${ge} path="routines/:routineId" element=${l`<${mh} />`} />
          <${ge} path="automations" element=${l`<${CN} />`} />
          <${ge} path="extensions" element=${l`<${Nh} />`} />
          <${ge} path="extensions/:tab" element=${l`<${Nh} />`} />
          <${ge} path="logs" element=${l`<${Ok} />`} />
          <${ge} path="settings" element=${l`<${Eh} />`} />
          <${ge} path="settings/:tab" element=${l`<${Eh} />`} />
          <${ge} path="admin" element=${l`<${Lk} auth=${e} />`} />
          <${ge} path="admin/:tab" element=${l`<${Lk} auth=${e} />`} />
        <//>
        <${ge} path="*" element=${l`<${ct} to=${Pr} replace />`} />
      <//>
    <//>
  `}Mh("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","approval.viewFullCommand":"View full command","approval.showCommandPreview":"Show preview","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.serviceUnavailable":"Service unavailable","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveTools":"Auto-approve tools","settings.field.autoApproveToolsDesc":"Skip approval for all tool calls","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.pendingReview":"Pending review","skills.autoActivate.on":"Auto-activate: On","skills.autoActivate.onPaused":"Auto-activate: On (paused)","skills.autoActivate.off":"Auto-activate: Off","skills.autoActivate.titlePaused":"Learned-skill auto-activation is off \u2014 learned skills don't auto-fire until you turn it back on at the top of this page. This setting is saved for later.","skills.autoActivate.titleOn":"Auto-activation on \u2014 runs on matching requests. Click to make it explicit-only (/name).","skills.autoActivate.titleOff":"Explicit-only \u2014 runs only when you type /name. Click to enable auto-activation.","skills.learning.selfLearning.title":"Self-learning","skills.learning.selfLearning.summaryOn":"On \u2014 after a task that did real work (several tool steps), IronClaw makes one extra call to your provider to try distilling it into a reusable skill. Quick or trivial tasks are skipped, and a skill is saved only when it's genuinely reusable.","skills.learning.selfLearning.summaryOff":"Off \u2014 finished tasks are never sent for distillation, and no new skills are created.","skills.learning.selfLearning.labelOn":"On","skills.learning.selfLearning.labelOff":"Off","skills.learning.review.title":"When it learns a new skill","skills.learning.review.summaryOn":"Held for your review below before it's used.","skills.learning.review.summaryOff":"Used right away \u2014 turn on to review new skills first.","skills.learning.review.labelOn":"Hold for review","skills.learning.review.labelOff":"Use right away","skills.learning.review.pendingTitle":"Pending review ({count})","skills.learning.review.pendingDescription":"Skills IronClaw learned or wants to update, waiting for you to approve or discard.","skills.learning.review.newBadge":"New skill","skills.learning.review.proposedBadge":"Proposed update","skills.learning.review.newDescription":"Learned from a task and held for review \u2014 it won't auto-activate until you approve it.","skills.learning.review.evolutionDescription":"The assistant wants to update this skill, but you've edited it \u2014 review the change before it replaces your version.","skills.learning.review.preview":"Preview","skills.learning.review.hide":"Hide","skills.learning.review.approve":"Approve","skills.learning.review.applyUpdate":"Apply update","skills.learning.review.discard":"Discard","skills.learning.review.yourVersion":"Your current version","skills.learning.review.proposedVersion":"Proposed update","skills.learning.review.skillContent":"Skill content","skills.learning.activation.title":"When a learned skill matches your request","skills.learning.activation.summaryOn":"Skills IronClaw learned run automatically when they match.","skills.learning.activation.summaryOff":"Learned skills run only when you type /name. Your hand-written skills are unaffected.","skills.learning.activation.labelOn":"Use automatically","skills.learning.activation.labelOff":"Only on command","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.confirmDiscardPending":'Discard the pending skill "{name}"? A held new skill is deleted; a proposed update is dropped and your version is kept.',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Memory, files & attachments","workspace.readOnly":"Read-only","workspace.filterPlaceholder":"Filter by name\u2026","workspace.emptyDir":"This folder is empty.","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.noFiles":"No files here.","workspace.noMatches":"Nothing matches that filter.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a file","workspace.pickFileDesc":"Choose a file from the tree to preview or download it. This viewer is read-only.","workspace.parent":"Parent: {path}","workspace.download":"Download","workspace.binaryPreviewUnavailable":"No inline preview for this file type. Download it to view the contents.","workspace.fileMeta":"{mime} \xB7 {size} bytes","workspace.unableOpenDirectory":"Unable to open directory","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.filter.completed":"Completed","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.schedule.onceAt":"Once on {datetime}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.filterAction":"Show {label}","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.empty.onboardingTitle":"No automations yet","automations.empty.onboardingDescription":"Automations are created by chatting with your agent \u2014 there's no form to fill out. Ask it to do something on a schedule and it will set up a recurring automation for you.","automations.empty.examplesTitle":"Try asking your agent","automations.empty.example1":"Check the nearai/ironclaw repo every 10 minutes and summarize new issues, PRs, and commits.","automations.empty.example2":"Every weekday at 9am, send me a summary of my unread email.","automations.empty.example3":"Remind me to review open pull requests every afternoon at 3pm.","automations.empty.startInChat":"Start in chat","automations.empty.copyPrompt":"Copy prompt","automations.empty.copied":"Copied","automations.refreshing":"Refreshing\u2026","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.runs.total":"Recent runs: {count}","automations.runs.ok":"OK: {count}","automations.runs.error":"Failed: {count}","automations.runs.running":"Running: {count}","automations.runs.unknown":"Unknown: {count}","automations.runs.showingOf":"Showing {shown} of {total} recent runs","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up a project for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open project","projects.openGeneralWorkspace":"Open project","projects.noDescription":"No project description yet. The project is still being shaped by recent activity and thread history.","projects.general.label":"General project","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,jk.createRoot)(document.getElementById("v2-root")).render(l`
  <${Oh}>
    <${gd} client=${Tt}>
      <${Uk} />
    <//>
  <//>
`);
