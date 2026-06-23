import{a as Sn,b as qe,c as Ie,d as h,e as l,f as Rh,g as Ch,h as tl,i as k,j as al}from"./chunks/chunk-UQPKMZ52.js";var Vh=Sn(dl=>{"use strict";var _k=Symbol.for("react.transitional.element"),kk=Symbol.for("react.fragment");function Qh(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:_k,type:e,key:n,ref:t!==void 0?t:null,props:a}}dl.Fragment=kk;dl.jsx=Qh;dl.jsxs=Qh});var fd=Sn((xO,Gh)=>{"use strict";Gh.exports=Vh()});var lv=Sn(Oe=>{"use strict";function xd(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<xl(r,t))e[n]=t,e[a]=r,a=n;else break e}}function Da(e){return e.length===0?null:e[0]}function wl(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],u=i+1,c=e[u];if(0>xl(o,a))u<r&&0>xl(c,o)?(e[n]=c,e[u]=a,n=u):(e[n]=o,e[i]=a,n=i);else if(u<r&&0>xl(c,a))e[n]=c,e[u]=a,n=u;else break e}}return t}function xl(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}Oe.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(Wh=performance,Oe.unstable_now=function(){return Wh.now()}):(gd=Date,ev=gd.now(),Oe.unstable_now=function(){return gd.now()-ev});var Wh,gd,ev,Xa=[],kn=[],Tk=1,ia=null,xt=3,$d=!1,_i=!1,ki=!1,wd=!1,nv=typeof setTimeout=="function"?setTimeout:null,rv=typeof clearTimeout=="function"?clearTimeout:null,tv=typeof setImmediate<"u"?setImmediate:null;function $l(e){for(var t=Da(kn);t!==null;){if(t.callback===null)wl(kn);else if(t.startTime<=e)wl(kn),t.sortIndex=t.expirationTime,xd(Xa,t);else break;t=Da(kn)}}function Sd(e){if(ki=!1,$l(e),!_i)if(Da(Xa)!==null)_i=!0,Yr||(Yr=!0,Gr());else{var t=Da(kn);t!==null&&Nd(Sd,t.startTime-e)}}var Yr=!1,Ri=-1,sv=5,iv=-1;function ov(){return wd?!0:!(Oe.unstable_now()-iv<sv)}function yd(){if(wd=!1,Yr){var e=Oe.unstable_now();iv=e;var t=!0;try{e:{_i=!1,ki&&(ki=!1,rv(Ri),Ri=-1),$d=!0;var a=xt;try{t:{for($l(e),ia=Da(Xa);ia!==null&&!(ia.expirationTime>e&&ov());){var n=ia.callback;if(typeof n=="function"){ia.callback=null,xt=ia.priorityLevel;var r=n(ia.expirationTime<=e);if(e=Oe.unstable_now(),typeof r=="function"){ia.callback=r,$l(e),t=!0;break t}ia===Da(Xa)&&wl(Xa),$l(e)}else wl(Xa);ia=Da(Xa)}if(ia!==null)t=!0;else{var s=Da(kn);s!==null&&Nd(Sd,s.startTime-e),t=!1}}break e}finally{ia=null,xt=a,$d=!1}t=void 0}}finally{t?Gr():Yr=!1}}}var Gr;typeof tv=="function"?Gr=function(){tv(yd)}:typeof MessageChannel<"u"?(bd=new MessageChannel,av=bd.port2,bd.port1.onmessage=yd,Gr=function(){av.postMessage(null)}):Gr=function(){nv(yd,0)};var bd,av;function Nd(e,t){Ri=nv(function(){e(Oe.unstable_now())},t)}Oe.unstable_IdlePriority=5;Oe.unstable_ImmediatePriority=1;Oe.unstable_LowPriority=4;Oe.unstable_NormalPriority=3;Oe.unstable_Profiling=null;Oe.unstable_UserBlockingPriority=2;Oe.unstable_cancelCallback=function(e){e.callback=null};Oe.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):sv=0<e?Math.floor(1e3/e):5};Oe.unstable_getCurrentPriorityLevel=function(){return xt};Oe.unstable_next=function(e){switch(xt){case 1:case 2:case 3:var t=3;break;default:t=xt}var a=xt;xt=t;try{return e()}finally{xt=a}};Oe.unstable_requestPaint=function(){wd=!0};Oe.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=xt;xt=e;try{return t()}finally{xt=a}};Oe.unstable_scheduleCallback=function(e,t,a){var n=Oe.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:Tk++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,xd(kn,e),Da(Xa)===null&&e===Da(kn)&&(ki?(rv(Ri),Ri=-1):ki=!0,Nd(Sd,a-n))):(e.sortIndex=r,xd(Xa,e),_i||$d||(_i=!0,Yr||(Yr=!0,Gr()))),e};Oe.unstable_shouldYield=ov;Oe.unstable_wrapCallback=function(e){var t=xt;return function(){var a=xt;xt=t;try{return e.apply(this,arguments)}finally{xt=a}}}});var cv=Sn((aL,uv)=>{"use strict";uv.exports=lv()});var mv=Sn(Ct=>{"use strict";var Ak=Ie();function dv(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Rn(){}var Rt={d:{f:Rn,r:function(){throw Error(dv(522))},D:Rn,C:Rn,L:Rn,m:Rn,X:Rn,S:Rn,M:Rn},p:0,findDOMNode:null},Dk=Symbol.for("react.portal");function Mk(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:Dk,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var Ci=Ak.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function Sl(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}Ct.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=Rt;Ct.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(dv(299));return Mk(e,t,null,a)};Ct.flushSync=function(e){var t=Ci.T,a=Rt.p;try{if(Ci.T=null,Rt.p=2,e)return e()}finally{Ci.T=t,Rt.p=a,Rt.d.f()}};Ct.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,Rt.d.C(e,t))};Ct.prefetchDNS=function(e){typeof e=="string"&&Rt.d.D(e)};Ct.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=Sl(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?Rt.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&Rt.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};Ct.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=Sl(t.as,t.crossOrigin);Rt.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&Rt.d.M(e)};Ct.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=Sl(a,t.crossOrigin);Rt.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};Ct.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=Sl(t.as,t.crossOrigin);Rt.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else Rt.d.m(e)};Ct.requestFormReset=function(e){Rt.d.r(e)};Ct.unstable_batchedUpdates=function(e,t){return e(t)};Ct.useFormState=function(e,t,a){return Ci.H.useFormState(e,t,a)};Ct.useFormStatus=function(){return Ci.H.useHostTransitionStatus()};Ct.version="19.1.0"});var hv=Sn((rL,pv)=>{"use strict";function fv(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(fv)}catch(e){console.error(e)}}fv(),pv.exports=mv()});var g0=Sn(Iu=>{"use strict";var it=cv(),jg=Ie(),Ok=hv();function L(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Ug(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function vo(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function Fg(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function vv(e){if(vo(e)!==e)throw Error(L(188))}function Lk(e){var t=e.alternate;if(!t){if(t=vo(e),t===null)throw Error(L(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return vv(r),e;if(s===n)return vv(r),t;s=s.sibling}throw Error(L(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(L(189))}}if(a.alternate!==n)throw Error(L(190))}if(a.tag!==3)throw Error(L(188));return a.stateNode.current===a?e:t}function zg(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=zg(e),t!==null)return t;e=e.sibling}return null}var De=Object.assign,Pk=Symbol.for("react.element"),Nl=Symbol.for("react.transitional.element"),ji=Symbol.for("react.portal"),as=Symbol.for("react.fragment"),qg=Symbol.for("react.strict_mode"),am=Symbol.for("react.profiler"),jk=Symbol.for("react.provider"),Bg=Symbol.for("react.consumer"),an=Symbol.for("react.context"),Xm=Symbol.for("react.forward_ref"),nm=Symbol.for("react.suspense"),rm=Symbol.for("react.suspense_list"),Zm=Symbol.for("react.memo"),Tn=Symbol.for("react.lazy");Symbol.for("react.scope");var sm=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var Uk=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var gv=Symbol.iterator;function Ei(e){return e===null||typeof e!="object"?null:(e=gv&&e[gv]||e["@@iterator"],typeof e=="function"?e:null)}var Fk=Symbol.for("react.client.reference");function im(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===Fk?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case as:return"Fragment";case am:return"Profiler";case qg:return"StrictMode";case nm:return"Suspense";case rm:return"SuspenseList";case sm:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case ji:return"Portal";case an:return(e.displayName||"Context")+".Provider";case Bg:return(e._context.displayName||"Context")+".Consumer";case Xm:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case Zm:return t=e.displayName||null,t!==null?t:im(e.type)||"Memo";case Tn:t=e._payload,e=e._init;try{return im(e(t))}catch{}}return null}var Ui=Array.isArray,ae=jg.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,me=Ok.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,fr={pending:!1,data:null,method:null,action:null},om=[],ns=-1;function Fa(e){return{current:e}}function ft(e){0>ns||(e.current=om[ns],om[ns]=null,ns--)}function Pe(e,t){ns++,om[ns]=e.current,e.current=t}var Pa=Fa(null),to=Fa(null),zn=Fa(null),eu=Fa(null);function tu(e,t){switch(Pe(zn,t),Pe(to,e),Pe(Pa,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?Sg(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=Sg(t),e=s0(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}ft(Pa),Pe(Pa,e)}function ws(){ft(Pa),ft(to),ft(zn)}function lm(e){e.memoizedState!==null&&Pe(eu,e);var t=Pa.current,a=s0(t,e.type);t!==a&&(Pe(to,e),Pe(Pa,a))}function au(e){to.current===e&&(ft(Pa),ft(to)),eu.current===e&&(ft(eu),mo._currentValue=fr)}var um=Object.prototype.hasOwnProperty,Wm=it.unstable_scheduleCallback,_d=it.unstable_cancelCallback,zk=it.unstable_shouldYield,qk=it.unstable_requestPaint,ja=it.unstable_now,Bk=it.unstable_getCurrentPriorityLevel,Hg=it.unstable_ImmediatePriority,Kg=it.unstable_UserBlockingPriority,nu=it.unstable_NormalPriority,Hk=it.unstable_LowPriority,Ig=it.unstable_IdlePriority,Kk=it.log,Ik=it.unstable_setDisableYieldValue,go=null,Yt=null;function Pn(e){if(typeof Kk=="function"&&Ik(e),Yt&&typeof Yt.setStrictMode=="function")try{Yt.setStrictMode(go,e)}catch{}}var Jt=Math.clz32?Math.clz32:Gk,Qk=Math.log,Vk=Math.LN2;function Gk(e){return e>>>=0,e===0?32:31-(Qk(e)/Vk|0)|0}var _l=256,kl=4194304;function cr(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Tu(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=cr(n):(i&=o,i!==0?r=cr(i):a||(a=o&~e,a!==0&&(r=cr(a))))):(o=n&~s,o!==0?r=cr(o):i!==0?r=cr(i):a||(a=n&~e,a!==0&&(r=cr(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function yo(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function Yk(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function Qg(){var e=_l;return _l<<=1,(_l&4194048)===0&&(_l=256),e}function Vg(){var e=kl;return kl<<=1,(kl&62914560)===0&&(kl=4194304),e}function kd(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function bo(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function Jk(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,u=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Jt(a),f=1<<d;o[d]=0,u[d]=-1;var m=c[d];if(m!==null)for(c[d]=null,d=0;d<m.length;d++){var p=m[d];p!==null&&(p.lane&=-536870913)}a&=~f}n!==0&&Gg(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function Gg(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Jt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function Yg(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Jt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function ef(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function tf(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function Jg(){var e=me.p;return e!==0?e:(e=window.event,e===void 0?32:h0(e.type))}function Xk(e,t){var a=me.p;try{return me.p=e,t()}finally{me.p=a}}var Xn=Math.random().toString(36).slice(2),$t="__reactFiber$"+Xn,zt="__reactProps$"+Xn,Ms="__reactContainer$"+Xn,cm="__reactEvents$"+Xn,Zk="__reactListeners$"+Xn,Wk="__reactHandles$"+Xn,yv="__reactResources$"+Xn,xo="__reactMarker$"+Xn;function af(e){delete e[$t],delete e[zt],delete e[cm],delete e[Zk],delete e[Wk]}function rs(e){var t=e[$t];if(t)return t;for(var a=e.parentNode;a;){if(t=a[Ms]||a[$t]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=kg(e);e!==null;){if(a=e[$t])return a;e=kg(e)}return t}e=a,a=e.parentNode}return null}function Os(e){if(e=e[$t]||e[Ms]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function Fi(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(L(33))}function ps(e){var t=e[yv];return t||(t=e[yv]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function dt(e){e[xo]=!0}var Xg=new Set,Zg={};function Nr(e,t){Ss(e,t),Ss(e+"Capture",t)}function Ss(e,t){for(Zg[e]=t,e=0;e<t.length;e++)Xg.add(t[e])}var eR=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),bv={},xv={};function tR(e){return um.call(xv,e)?!0:um.call(bv,e)?!1:eR.test(e)?xv[e]=!0:(bv[e]=!0,!1)}function ql(e,t,a){if(tR(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function Rl(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function Za(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var Rd,$v;function Wr(e){if(Rd===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);Rd=t&&t[1]||"",$v=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+Rd+e+$v}var Cd=!1;function Ed(e,t){if(!e||Cd)return"";Cd=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var f=function(){throw Error()};if(Object.defineProperty(f.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(f,[])}catch(p){var m=p}Reflect.construct(e,[],f)}else{try{f.call()}catch(p){m=p}e.call(f.prototype)}}else{try{throw Error()}catch(p){m=p}(f=e())&&typeof f.catch=="function"&&f.catch(function(){})}}catch(p){if(p&&m&&typeof p.stack=="string")return[p.stack,m.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var u=i.split(`
`),c=o.split(`
`);for(r=n=0;n<u.length&&!u[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===u.length||r===c.length)for(n=u.length-1,r=c.length-1;1<=n&&0<=r&&u[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(u[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||u[n]!==c[r]){var d=`
`+u[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{Cd=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?Wr(a):""}function aR(e){switch(e.tag){case 26:case 27:case 5:return Wr(e.type);case 16:return Wr("Lazy");case 13:return Wr("Suspense");case 19:return Wr("SuspenseList");case 0:case 15:return Ed(e.type,!1);case 11:return Ed(e.type.render,!1);case 1:return Ed(e.type,!0);case 31:return Wr("Activity");default:return""}}function wv(e){try{var t="";do t+=aR(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function la(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function Wg(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function nR(e){var t=Wg(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function ru(e){e._valueTracker||(e._valueTracker=nR(e))}function ey(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=Wg(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function su(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var rR=/[\n"\\]/g;function da(e){return e.replace(rR,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function dm(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+la(t)):e.value!==""+la(t)&&(e.value=""+la(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?mm(e,i,la(t)):a!=null?mm(e,i,la(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+la(o):e.removeAttribute("name")}function ty(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+la(a):"",t=t!=null?""+la(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function mm(e,t,a){t==="number"&&su(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function hs(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+la(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function ay(e,t,a){if(t!=null&&(t=""+la(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+la(a):""}function ny(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(L(92));if(Ui(n)){if(1<n.length)throw Error(L(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=la(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function Ns(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var sR=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function Sv(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||sR.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function ry(e,t,a){if(t!=null&&typeof t!="object")throw Error(L(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&Sv(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&Sv(e,s,t[s])}function nf(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var iR=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),oR=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function Bl(e){return oR.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var fm=null;function rf(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var ss=null,vs=null;function Nv(e){var t=Os(e);if(t&&(e=t.stateNode)){var a=e[zt]||null;e:switch(e=t.stateNode,t.type){case"input":if(dm(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+da(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[zt]||null;if(!r)throw Error(L(90));dm(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&ey(n)}break e;case"textarea":ay(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&hs(e,!!a.multiple,t,!1)}}}var Td=!1;function sy(e,t,a){if(Td)return e(t,a);Td=!0;try{var n=e(t);return n}finally{if(Td=!1,(ss!==null||vs!==null)&&(zu(),ss&&(t=ss,e=vs,vs=ss=null,Nv(t),e)))for(t=0;t<e.length;t++)Nv(e[t])}}function ao(e,t){var a=e.stateNode;if(a===null)return null;var n=a[zt]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(L(231,t,typeof a));return a}var cn=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),pm=!1;if(cn)try{Jr={},Object.defineProperty(Jr,"passive",{get:function(){pm=!0}}),window.addEventListener("test",Jr,Jr),window.removeEventListener("test",Jr,Jr)}catch{pm=!1}var Jr,jn=null,sf=null,Hl=null;function iy(){if(Hl)return Hl;var e,t=sf,a=t.length,n,r="value"in jn?jn.value:jn.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return Hl=r.slice(e,1<n?1-n:void 0)}function Kl(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function Cl(){return!0}function _v(){return!1}function qt(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?Cl:_v,this.isPropagationStopped=_v,this}return De(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=Cl)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=Cl)},persist:function(){},isPersistent:Cl}),t}var _r={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Au=qt(_r),$o=De({},_r,{view:0,detail:0}),lR=qt($o),Ad,Dd,Ti,Du=De({},$o,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:of,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==Ti&&(Ti&&e.type==="mousemove"?(Ad=e.screenX-Ti.screenX,Dd=e.screenY-Ti.screenY):Dd=Ad=0,Ti=e),Ad)},movementY:function(e){return"movementY"in e?e.movementY:Dd}}),kv=qt(Du),uR=De({},Du,{dataTransfer:0}),cR=qt(uR),dR=De({},$o,{relatedTarget:0}),Md=qt(dR),mR=De({},_r,{animationName:0,elapsedTime:0,pseudoElement:0}),fR=qt(mR),pR=De({},_r,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),hR=qt(pR),vR=De({},_r,{data:0}),Rv=qt(vR),gR={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},yR={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},bR={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function xR(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=bR[e])?!!t[e]:!1}function of(){return xR}var $R=De({},$o,{key:function(e){if(e.key){var t=gR[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=Kl(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?yR[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:of,charCode:function(e){return e.type==="keypress"?Kl(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?Kl(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),wR=qt($R),SR=De({},Du,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),Cv=qt(SR),NR=De({},$o,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:of}),_R=qt(NR),kR=De({},_r,{propertyName:0,elapsedTime:0,pseudoElement:0}),RR=qt(kR),CR=De({},Du,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),ER=qt(CR),TR=De({},_r,{newState:0,oldState:0}),AR=qt(TR),DR=[9,13,27,32],lf=cn&&"CompositionEvent"in window,qi=null;cn&&"documentMode"in document&&(qi=document.documentMode);var MR=cn&&"TextEvent"in window&&!qi,oy=cn&&(!lf||qi&&8<qi&&11>=qi),Ev=" ",Tv=!1;function ly(e,t){switch(e){case"keyup":return DR.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function uy(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var is=!1;function OR(e,t){switch(e){case"compositionend":return uy(t);case"keypress":return t.which!==32?null:(Tv=!0,Ev);case"textInput":return e=t.data,e===Ev&&Tv?null:e;default:return null}}function LR(e,t){if(is)return e==="compositionend"||!lf&&ly(e,t)?(e=iy(),Hl=sf=jn=null,is=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return oy&&t.locale!=="ko"?null:t.data;default:return null}}var PR={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function Av(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!PR[e.type]:t==="textarea"}function cy(e,t,a,n){ss?vs?vs.push(n):vs=[n]:ss=n,t=Su(t,"onChange"),0<t.length&&(a=new Au("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var Bi=null,no=null;function jR(e){a0(e,0)}function Mu(e){var t=Fi(e);if(ey(t))return e}function Dv(e,t){if(e==="change")return t}var dy=!1;cn&&(cn?(Tl="oninput"in document,Tl||(Od=document.createElement("div"),Od.setAttribute("oninput","return;"),Tl=typeof Od.oninput=="function"),El=Tl):El=!1,dy=El&&(!document.documentMode||9<document.documentMode));var El,Tl,Od;function Mv(){Bi&&(Bi.detachEvent("onpropertychange",my),no=Bi=null)}function my(e){if(e.propertyName==="value"&&Mu(no)){var t=[];cy(t,no,e,rf(e)),sy(jR,t)}}function UR(e,t,a){e==="focusin"?(Mv(),Bi=t,no=a,Bi.attachEvent("onpropertychange",my)):e==="focusout"&&Mv()}function FR(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return Mu(no)}function zR(e,t){if(e==="click")return Mu(t)}function qR(e,t){if(e==="input"||e==="change")return Mu(t)}function BR(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var Wt=typeof Object.is=="function"?Object.is:BR;function ro(e,t){if(Wt(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!um.call(t,r)||!Wt(e[r],t[r]))return!1}return!0}function Ov(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function Lv(e,t){var a=Ov(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=Ov(a)}}function fy(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?fy(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function py(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=su(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=su(e.document)}return t}function uf(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var HR=cn&&"documentMode"in document&&11>=document.documentMode,os=null,hm=null,Hi=null,vm=!1;function Pv(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;vm||os==null||os!==su(n)||(n=os,"selectionStart"in n&&uf(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),Hi&&ro(Hi,n)||(Hi=n,n=Su(hm,"onSelect"),0<n.length&&(t=new Au("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=os)))}function ur(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var ls={animationend:ur("Animation","AnimationEnd"),animationiteration:ur("Animation","AnimationIteration"),animationstart:ur("Animation","AnimationStart"),transitionrun:ur("Transition","TransitionRun"),transitionstart:ur("Transition","TransitionStart"),transitioncancel:ur("Transition","TransitionCancel"),transitionend:ur("Transition","TransitionEnd")},Ld={},hy={};cn&&(hy=document.createElement("div").style,"AnimationEvent"in window||(delete ls.animationend.animation,delete ls.animationiteration.animation,delete ls.animationstart.animation),"TransitionEvent"in window||delete ls.transitionend.transition);function kr(e){if(Ld[e])return Ld[e];if(!ls[e])return e;var t=ls[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in hy)return Ld[e]=t[a];return e}var vy=kr("animationend"),gy=kr("animationiteration"),yy=kr("animationstart"),KR=kr("transitionrun"),IR=kr("transitionstart"),QR=kr("transitioncancel"),by=kr("transitionend"),xy=new Map,gm="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");gm.push("scrollEnd");function wa(e,t){xy.set(e,t),Nr(t,[e])}var jv=new WeakMap;function ma(e,t){if(typeof e=="object"&&e!==null){var a=jv.get(e);return a!==void 0?a:(t={value:e,source:t,stack:wv(t)},jv.set(e,t),t)}return{value:e,source:t,stack:wv(t)}}var oa=[],us=0,cf=0;function Ou(){for(var e=us,t=cf=us=0;t<e;){var a=oa[t];oa[t++]=null;var n=oa[t];oa[t++]=null;var r=oa[t];oa[t++]=null;var s=oa[t];if(oa[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&$y(a,r,s)}}function Lu(e,t,a,n){oa[us++]=e,oa[us++]=t,oa[us++]=a,oa[us++]=n,cf|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function df(e,t,a,n){return Lu(e,t,a,n),iu(e)}function Ls(e,t){return Lu(e,null,null,t),iu(e)}function $y(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Jt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function iu(e){if(50<Wi)throw Wi=0,Um=null,Error(L(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var cs={};function VR(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Gt(e,t,a,n){return new VR(e,t,a,n)}function mf(e){return e=e.prototype,!(!e||!e.isReactComponent)}function ln(e,t){var a=e.alternate;return a===null?(a=Gt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function wy(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function Il(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")mf(e)&&(i=1);else if(typeof e=="string")i=VC(e,a,Pa.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case sm:return e=Gt(31,a,t,r),e.elementType=sm,e.lanes=s,e;case as:return pr(a.children,r,s,t);case qg:i=8,r|=24;break;case am:return e=Gt(12,a,t,r|2),e.elementType=am,e.lanes=s,e;case nm:return e=Gt(13,a,t,r),e.elementType=nm,e.lanes=s,e;case rm:return e=Gt(19,a,t,r),e.elementType=rm,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case jk:case an:i=10;break e;case Bg:i=9;break e;case Xm:i=11;break e;case Zm:i=14;break e;case Tn:i=16,n=null;break e}i=29,a=Error(L(130,e===null?"null":typeof e,"")),n=null}return t=Gt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function pr(e,t,a,n){return e=Gt(7,e,n,t),e.lanes=a,e}function Pd(e,t,a){return e=Gt(6,e,null,t),e.lanes=a,e}function jd(e,t,a){return t=Gt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var ds=[],ms=0,ou=null,lu=0,ua=[],ca=0,hr=null,nn=1,rn="";function dr(e,t){ds[ms++]=lu,ds[ms++]=ou,ou=e,lu=t}function Sy(e,t,a){ua[ca++]=nn,ua[ca++]=rn,ua[ca++]=hr,hr=e;var n=nn;e=rn;var r=32-Jt(n)-1;n&=~(1<<r),a+=1;var s=32-Jt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,nn=1<<32-Jt(t)+r|a<<r|n,rn=s+e}else nn=1<<s|a<<r|n,rn=e}function ff(e){e.return!==null&&(dr(e,1),Sy(e,1,0))}function pf(e){for(;e===ou;)ou=ds[--ms],ds[ms]=null,lu=ds[--ms],ds[ms]=null;for(;e===hr;)hr=ua[--ca],ua[ca]=null,rn=ua[--ca],ua[ca]=null,nn=ua[--ca],ua[ca]=null}var Et=null,Be=null,de=!1,vr=null,Oa=!1,ym=Error(L(519));function xr(e){var t=Error(L(418,""));throw so(ma(t,e)),ym}function Uv(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[$t]=e,t[zt]=n,a){case"dialog":se("cancel",t),se("close",t);break;case"iframe":case"object":case"embed":se("load",t);break;case"video":case"audio":for(a=0;a<lo.length;a++)se(lo[a],t);break;case"source":se("error",t);break;case"img":case"image":case"link":se("error",t),se("load",t);break;case"details":se("toggle",t);break;case"input":se("invalid",t),ty(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),ru(t);break;case"select":se("invalid",t);break;case"textarea":se("invalid",t),ny(t,n.value,n.defaultValue,n.children),ru(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||r0(t.textContent,a)?(n.popover!=null&&(se("beforetoggle",t),se("toggle",t)),n.onScroll!=null&&se("scroll",t),n.onScrollEnd!=null&&se("scrollend",t),n.onClick!=null&&(t.onclick=Hu),t=!0):t=!1,t||xr(e)}function Fv(e){for(Et=e.return;Et;)switch(Et.tag){case 5:case 13:Oa=!1;return;case 27:case 3:Oa=!0;return;default:Et=Et.return}}function Ai(e){if(e!==Et)return!1;if(!de)return Fv(e),de=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||Km(e.type,e.memoizedProps)),a=!a),a&&Be&&xr(e),Fv(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(L(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){Be=$a(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}Be=null}}else t===27?(t=Be,Zn(e.type)?(e=Vm,Vm=null,Be=e):Be=t):Be=Et?$a(e.stateNode.nextSibling):null;return!0}function wo(){Be=Et=null,de=!1}function zv(){var e=vr;return e!==null&&(Ft===null?Ft=e:Ft.push.apply(Ft,e),vr=null),e}function so(e){vr===null?vr=[e]:vr.push(e)}var bm=Fa(null),Rr=null,sn=null;function Dn(e,t,a){Pe(bm,t._currentValue),t._currentValue=a}function un(e){e._currentValue=bm.current,ft(bm)}function xm(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function $m(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var u=0;u<t.length;u++)if(o.context===t[u]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),xm(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(L(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),xm(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function So(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(L(387));if(i=i.memoizedProps,i!==null){var o=r.type;Wt(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===eu.current){if(i=r.alternate,i===null)throw Error(L(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(mo):e=[mo])}r=r.return}e!==null&&$m(t,e,a,n),t.flags|=262144}function uu(e){for(e=e.firstContext;e!==null;){if(!Wt(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function $r(e){Rr=e,sn=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function wt(e){return Ny(Rr,e)}function Al(e,t){return Rr===null&&$r(e),Ny(e,t)}function Ny(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},sn===null){if(e===null)throw Error(L(308));sn=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else sn=sn.next=t;return a}var GR=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},YR=it.unstable_scheduleCallback,JR=it.unstable_NormalPriority,rt={$$typeof:an,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function hf(){return{controller:new GR,data:new Map,refCount:0}}function No(e){e.refCount--,e.refCount===0&&YR(JR,function(){e.controller.abort()})}var Ki=null,wm=0,_s=0,gs=null;function XR(e,t){if(Ki===null){var a=Ki=[];wm=0,_s=Uf(),gs={status:"pending",value:void 0,then:function(n){a.push(n)}}}return wm++,t.then(qv,qv),t}function qv(){if(--wm===0&&Ki!==null){gs!==null&&(gs.status="fulfilled");var e=Ki;Ki=null,_s=0,gs=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function ZR(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var Bv=ae.S;ae.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&XR(e,t),Bv!==null&&Bv(e,t)};var gr=Fa(null);function vf(){var e=gr.current;return e!==null?e:Ee.pooledCache}function Ql(e,t){t===null?Pe(gr,gr.current):Pe(gr,t.pool)}function _y(){var e=vf();return e===null?null:{parent:rt._currentValue,pool:e}}var _o=Error(L(460)),ky=Error(L(474)),Pu=Error(L(542)),Sm={then:function(){}};function Hv(e){return e=e.status,e==="fulfilled"||e==="rejected"}function Dl(){}function Ry(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(Dl,Dl),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,Iv(e),e;default:if(typeof t.status=="string")t.then(Dl,Dl);else{if(e=Ee,e!==null&&100<e.shellSuspendCounter)throw Error(L(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,Iv(e),e}throw Ii=t,_o}}var Ii=null;function Kv(){if(Ii===null)throw Error(L(459));var e=Ii;return Ii=null,e}function Iv(e){if(e===_o||e===Pu)throw Error(L(483))}var An=!1;function gf(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function Nm(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function qn(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function Bn(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,(xe&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=iu(e),$y(e,null,a),t}return Lu(e,n,t,a),iu(e)}function Qi(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,Yg(e,a)}}function Ud(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var _m=!1;function Vi(){if(_m){var e=gs;if(e!==null)throw e}}function Gi(e,t,a,n){_m=!1;var r=e.updateQueue;An=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var u=o,c=u.next;u.next=null,i===null?s=c:i.next=c,i=u;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=u))}if(s!==null){var f=r.baseState;i=0,d=c=u=null,o=s;do{var m=o.lane&-536870913,p=m!==o.lane;if(p?(ue&m)===m:(n&m)===m){m!==0&&m===_s&&(_m=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var b=e,y=o;m=t;var $=a;switch(y.tag){case 1:if(b=y.payload,typeof b=="function"){f=b.call($,f,m);break e}f=b;break e;case 3:b.flags=b.flags&-65537|128;case 0:if(b=y.payload,m=typeof b=="function"?b.call($,f,m):b,m==null)break e;f=De({},f,m);break e;case 2:An=!0}}m=o.callback,m!==null&&(e.flags|=64,p&&(e.flags|=8192),p=r.callbacks,p===null?r.callbacks=[m]:p.push(m))}else p={lane:m,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=p,u=f):d=d.next=p,i|=m;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;p=o,o=p.next,p.next=null,r.lastBaseUpdate=p,r.shared.pending=null}}while(!0);d===null&&(u=f),r.baseState=u,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),Jn|=i,e.lanes=i,e.memoizedState=f}}function Cy(e,t){if(typeof e!="function")throw Error(L(191,e));e.call(t)}function Ey(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)Cy(a[e],t)}var ks=Fa(null),cu=Fa(0);function Qv(e,t){e=fn,Pe(cu,e),Pe(ks,t),fn=e|t.baseLanes}function km(){Pe(cu,fn),Pe(ks,ks.current)}function yf(){fn=cu.current,ft(ks),ft(cu)}var Gn=0,re=null,Se=null,Je=null,du=!1,ys=!1,wr=!1,mu=0,io=0,bs=null,WR=0;function Qe(){throw Error(L(321))}function bf(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!Wt(e[a],t[a]))return!1;return!0}function xf(e,t,a,n,r,s){return Gn=s,re=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,ae.H=e===null||e.memoizedState===null?ib:ob,wr=!1,s=a(n,r),wr=!1,ys&&(s=Ay(t,a,n,r)),Ty(e),s}function Ty(e){ae.H=fu;var t=Se!==null&&Se.next!==null;if(Gn=0,Je=Se=re=null,du=!1,io=0,bs=null,t)throw Error(L(300));e===null||mt||(e=e.dependencies,e!==null&&uu(e)&&(mt=!0))}function Ay(e,t,a,n){re=e;var r=0;do{if(ys&&(bs=null),io=0,ys=!1,25<=r)throw Error(L(301));if(r+=1,Je=Se=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}ae.H=iC,s=t(a,n)}while(ys);return s}function eC(){var e=ae.H,t=e.useState()[0];return t=typeof t.then=="function"?ko(t):t,e=e.useState()[0],(Se!==null?Se.memoizedState:null)!==e&&(re.flags|=1024),t}function $f(){var e=mu!==0;return mu=0,e}function wf(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function Sf(e){if(du){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}du=!1}Gn=0,Je=Se=re=null,ys=!1,io=mu=0,bs=null}function jt(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return Je===null?re.memoizedState=Je=e:Je=Je.next=e,Je}function Xe(){if(Se===null){var e=re.alternate;e=e!==null?e.memoizedState:null}else e=Se.next;var t=Je===null?re.memoizedState:Je.next;if(t!==null)Je=t,Se=e;else{if(e===null)throw re.alternate===null?Error(L(467)):Error(L(310));Se=e,e={memoizedState:Se.memoizedState,baseState:Se.baseState,baseQueue:Se.baseQueue,queue:Se.queue,next:null},Je===null?re.memoizedState=Je=e:Je=Je.next=e}return Je}function Nf(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function ko(e){var t=io;return io+=1,bs===null&&(bs=[]),e=Ry(bs,e,t),t=re,(Je===null?t.memoizedState:Je.next)===null&&(t=t.alternate,ae.H=t===null||t.memoizedState===null?ib:ob),e}function ju(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return ko(e);if(e.$$typeof===an)return wt(e)}throw Error(L(438,String(e)))}function _f(e){var t=null,a=re.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=re.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=Nf(),re.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=Uk;return t.index++,a}function dn(e,t){return typeof t=="function"?t(e):t}function Vl(e){var t=Xe();return kf(t,Se,e)}function kf(e,t,a){var n=e.queue;if(n===null)throw Error(L(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,u=null,c=t,d=!1;do{var f=c.lane&-536870913;if(f!==c.lane?(ue&f)===f:(Gn&f)===f){var m=c.revertLane;if(m===0)u!==null&&(u=u.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),f===_s&&(d=!0);else if((Gn&m)===m){c=c.next,m===_s&&(d=!0);continue}else f={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=f,i=s):u=u.next=f,re.lanes|=m,Jn|=m;f=c.action,wr&&a(s,f),s=c.hasEagerState?c.eagerState:a(s,f)}else m={lane:f,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=m,i=s):u=u.next=m,re.lanes|=f,Jn|=f;c=c.next}while(c!==null&&c!==t);if(u===null?i=s:u.next=o,!Wt(s,e.memoizedState)&&(mt=!0,d&&(a=gs,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=u,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function Fd(e){var t=Xe(),a=t.queue;if(a===null)throw Error(L(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);Wt(s,t.memoizedState)||(mt=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function Dy(e,t,a){var n=re,r=Xe(),s=de;if(s){if(a===void 0)throw Error(L(407));a=a()}else a=t();var i=!Wt((Se||r).memoizedState,a);i&&(r.memoizedState=a,mt=!0),r=r.queue;var o=Ly.bind(null,n,r,e);if(Ro(2048,8,o,[e]),r.getSnapshot!==t||i||Je!==null&&Je.memoizedState.tag&1){if(n.flags|=2048,Rs(9,Uu(),Oy.bind(null,n,r,a,t),null),Ee===null)throw Error(L(349));s||(Gn&124)!==0||My(n,t,a)}return a}function My(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=re.updateQueue,t===null?(t=Nf(),re.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function Oy(e,t,a,n){t.value=a,t.getSnapshot=n,Py(t)&&jy(e)}function Ly(e,t,a){return a(function(){Py(t)&&jy(e)})}function Py(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!Wt(e,a)}catch{return!0}}function jy(e){var t=Ls(e,2);t!==null&&Zt(t,e,2)}function Rm(e){var t=jt();if(typeof e=="function"){var a=e;if(e=a(),wr){Pn(!0);try{a()}finally{Pn(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:dn,lastRenderedState:e},t}function Uy(e,t,a,n){return e.baseState=a,kf(e,Se,typeof n=="function"?n:dn)}function tC(e,t,a,n,r){if(Fu(e))throw Error(L(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};ae.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,Fy(t,s)):(s.next=a.next,t.pending=a.next=s)}}function Fy(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=ae.T,i={};ae.T=i;try{var o=a(r,n),u=ae.S;u!==null&&u(i,o),Vv(e,t,o)}catch(c){Cm(e,t,c)}finally{ae.T=s}}else try{s=a(r,n),Vv(e,t,s)}catch(c){Cm(e,t,c)}}function Vv(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){Gv(e,t,n)},function(n){return Cm(e,t,n)}):Gv(e,t,a)}function Gv(e,t,a){t.status="fulfilled",t.value=a,zy(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,Fy(e,a)))}function Cm(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,zy(t),t=t.next;while(t!==n)}e.action=null}function zy(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function qy(e,t){return t}function Yv(e,t){if(de){var a=Ee.formState;if(a!==null){e:{var n=re;if(de){if(Be){t:{for(var r=Be,s=Oa;r.nodeType!==8;){if(!s){r=null;break t}if(r=$a(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){Be=$a(r.nextSibling),n=r.data==="F!";break e}}xr(n)}n=!1}n&&(t=a[0])}}return a=jt(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:qy,lastRenderedState:t},a.queue=n,a=nb.bind(null,re,n),n.dispatch=a,n=Rm(!1),s=Tf.bind(null,re,!1,n.queue),n=jt(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=tC.bind(null,re,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function Jv(e){var t=Xe();return By(t,Se,e)}function By(e,t,a){if(t=kf(e,t,qy)[0],e=Vl(dn)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=ko(t)}catch(i){throw i===_o?Pu:i}else n=t;t=Xe();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(re.flags|=2048,Rs(9,Uu(),aC.bind(null,r,a),null)),[n,s,e]}function aC(e,t){e.action=t}function Xv(e){var t=Xe(),a=Se;if(a!==null)return By(t,a,e);Xe(),t=t.memoizedState,a=Xe();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function Rs(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=re.updateQueue,t===null&&(t=Nf(),re.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function Uu(){return{destroy:void 0,resource:void 0}}function Hy(){return Xe().memoizedState}function Gl(e,t,a,n){var r=jt();n=n===void 0?null:n,re.flags|=e,r.memoizedState=Rs(1|t,Uu(),a,n)}function Ro(e,t,a,n){var r=Xe();n=n===void 0?null:n;var s=r.memoizedState.inst;Se!==null&&n!==null&&bf(n,Se.memoizedState.deps)?r.memoizedState=Rs(t,s,a,n):(re.flags|=e,r.memoizedState=Rs(1|t,s,a,n))}function Zv(e,t){Gl(8390656,8,e,t)}function Ky(e,t){Ro(2048,8,e,t)}function Iy(e,t){return Ro(4,2,e,t)}function Qy(e,t){return Ro(4,4,e,t)}function Vy(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function Gy(e,t,a){a=a!=null?a.concat([e]):null,Ro(4,4,Vy.bind(null,t,e),a)}function Rf(){}function Yy(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&bf(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function Jy(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&bf(t,n[1]))return n[0];if(n=e(),wr){Pn(!0);try{e()}finally{Pn(!1)}}return a.memoizedState=[n,t],n}function Cf(e,t,a){return a===void 0||(Gn&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=zb(),re.lanes|=e,Jn|=e,a)}function Xy(e,t,a,n){return Wt(a,t)?a:ks.current!==null?(e=Cf(e,a,n),Wt(e,t)||(mt=!0),e):(Gn&42)===0?(mt=!0,e.memoizedState=a):(e=zb(),re.lanes|=e,Jn|=e,t)}function Zy(e,t,a,n,r){var s=me.p;me.p=s!==0&&8>s?s:8;var i=ae.T,o={};ae.T=o,Tf(e,!1,t,a);try{var u=r(),c=ae.S;if(c!==null&&c(o,u),u!==null&&typeof u=="object"&&typeof u.then=="function"){var d=ZR(u,n);Yi(e,t,d,Xt(e))}else Yi(e,t,n,Xt(e))}catch(f){Yi(e,t,{then:function(){},status:"rejected",reason:f},Xt())}finally{me.p=s,ae.T=i}}function nC(){}function Em(e,t,a,n){if(e.tag!==5)throw Error(L(476));var r=Wy(e).queue;Zy(e,r,t,fr,a===null?nC:function(){return eb(e),a(n)})}function Wy(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:fr,baseState:fr,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:dn,lastRenderedState:fr},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:dn,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function eb(e){var t=Wy(e).next.queue;Yi(e,t,{},Xt())}function Ef(){return wt(mo)}function tb(){return Xe().memoizedState}function ab(){return Xe().memoizedState}function rC(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=Xt();e=qn(a);var n=Bn(t,e,a);n!==null&&(Zt(n,t,a),Qi(n,t,a)),t={cache:hf()},e.payload=t;return}t=t.return}}function sC(e,t,a){var n=Xt();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},Fu(e)?rb(t,a):(a=df(e,t,a,n),a!==null&&(Zt(a,e,n),sb(a,t,n)))}function nb(e,t,a){var n=Xt();Yi(e,t,a,n)}function Yi(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(Fu(e))rb(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,Wt(o,i))return Lu(e,t,r,0),Ee===null&&Ou(),!1}catch{}finally{}if(a=df(e,t,r,n),a!==null)return Zt(a,e,n),sb(a,t,n),!0}return!1}function Tf(e,t,a,n){if(n={lane:2,revertLane:Uf(),action:n,hasEagerState:!1,eagerState:null,next:null},Fu(e)){if(t)throw Error(L(479))}else t=df(e,a,n,2),t!==null&&Zt(t,e,2)}function Fu(e){var t=e.alternate;return e===re||t!==null&&t===re}function rb(e,t){ys=du=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function sb(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,Yg(e,a)}}var fu={readContext:wt,use:ju,useCallback:Qe,useContext:Qe,useEffect:Qe,useImperativeHandle:Qe,useLayoutEffect:Qe,useInsertionEffect:Qe,useMemo:Qe,useReducer:Qe,useRef:Qe,useState:Qe,useDebugValue:Qe,useDeferredValue:Qe,useTransition:Qe,useSyncExternalStore:Qe,useId:Qe,useHostTransitionStatus:Qe,useFormState:Qe,useActionState:Qe,useOptimistic:Qe,useMemoCache:Qe,useCacheRefresh:Qe},ib={readContext:wt,use:ju,useCallback:function(e,t){return jt().memoizedState=[e,t===void 0?null:t],e},useContext:wt,useEffect:Zv,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,Gl(4194308,4,Vy.bind(null,t,e),a)},useLayoutEffect:function(e,t){return Gl(4194308,4,e,t)},useInsertionEffect:function(e,t){Gl(4,2,e,t)},useMemo:function(e,t){var a=jt();t=t===void 0?null:t;var n=e();if(wr){Pn(!0);try{e()}finally{Pn(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=jt();if(a!==void 0){var r=a(t);if(wr){Pn(!0);try{a(t)}finally{Pn(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=sC.bind(null,re,e),[n.memoizedState,e]},useRef:function(e){var t=jt();return e={current:e},t.memoizedState=e},useState:function(e){e=Rm(e);var t=e.queue,a=nb.bind(null,re,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:Rf,useDeferredValue:function(e,t){var a=jt();return Cf(a,e,t)},useTransition:function(){var e=Rm(!1);return e=Zy.bind(null,re,e.queue,!0,!1),jt().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=re,r=jt();if(de){if(a===void 0)throw Error(L(407));a=a()}else{if(a=t(),Ee===null)throw Error(L(349));(ue&124)!==0||My(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,Zv(Ly.bind(null,n,s,e),[e]),n.flags|=2048,Rs(9,Uu(),Oy.bind(null,n,s,a,t),null),a},useId:function(){var e=jt(),t=Ee.identifierPrefix;if(de){var a=rn,n=nn;a=(n&~(1<<32-Jt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=mu++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=WR++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:Ef,useFormState:Yv,useActionState:Yv,useOptimistic:function(e){var t=jt();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=Tf.bind(null,re,!0,a),a.dispatch=t,[e,t]},useMemoCache:_f,useCacheRefresh:function(){return jt().memoizedState=rC.bind(null,re)}},ob={readContext:wt,use:ju,useCallback:Yy,useContext:wt,useEffect:Ky,useImperativeHandle:Gy,useInsertionEffect:Iy,useLayoutEffect:Qy,useMemo:Jy,useReducer:Vl,useRef:Hy,useState:function(){return Vl(dn)},useDebugValue:Rf,useDeferredValue:function(e,t){var a=Xe();return Xy(a,Se.memoizedState,e,t)},useTransition:function(){var e=Vl(dn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:ko(e),t]},useSyncExternalStore:Dy,useId:tb,useHostTransitionStatus:Ef,useFormState:Jv,useActionState:Jv,useOptimistic:function(e,t){var a=Xe();return Uy(a,Se,e,t)},useMemoCache:_f,useCacheRefresh:ab},iC={readContext:wt,use:ju,useCallback:Yy,useContext:wt,useEffect:Ky,useImperativeHandle:Gy,useInsertionEffect:Iy,useLayoutEffect:Qy,useMemo:Jy,useReducer:Fd,useRef:Hy,useState:function(){return Fd(dn)},useDebugValue:Rf,useDeferredValue:function(e,t){var a=Xe();return Se===null?Cf(a,e,t):Xy(a,Se.memoizedState,e,t)},useTransition:function(){var e=Fd(dn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:ko(e),t]},useSyncExternalStore:Dy,useId:tb,useHostTransitionStatus:Ef,useFormState:Xv,useActionState:Xv,useOptimistic:function(e,t){var a=Xe();return Se!==null?Uy(a,Se,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:_f,useCacheRefresh:ab},xs=null,oo=0;function Ml(e){var t=oo;return oo+=1,xs===null&&(xs=[]),Ry(xs,e,t)}function Di(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function Ol(e,t){throw t.$$typeof===Pk?Error(L(525)):(e=Object.prototype.toString.call(t),Error(L(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function Wv(e){var t=e._init;return t(e._payload)}function lb(e){function t(g,v){if(e){var x=g.deletions;x===null?(g.deletions=[v],g.flags|=16):x.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=ln(g,v),g.index=0,g.sibling=null,g}function s(g,v,x){return g.index=x,e?(x=g.alternate,x!==null?(x=x.index,x<v?(g.flags|=67108866,v):x):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,x,w){return v===null||v.tag!==6?(v=Pd(x,g.mode,w),v.return=g,v):(v=r(v,x),v.return=g,v)}function u(g,v,x,w){var S=x.type;return S===as?d(g,v,x.props.children,w,x.key):v!==null&&(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Tn&&Wv(S)===v.type)?(v=r(v,x.props),Di(v,x),v.return=g,v):(v=Il(x.type,x.key,x.props,null,g.mode,w),Di(v,x),v.return=g,v)}function c(g,v,x,w){return v===null||v.tag!==4||v.stateNode.containerInfo!==x.containerInfo||v.stateNode.implementation!==x.implementation?(v=jd(x,g.mode,w),v.return=g,v):(v=r(v,x.children||[]),v.return=g,v)}function d(g,v,x,w,S){return v===null||v.tag!==7?(v=pr(x,g.mode,w,S),v.return=g,v):(v=r(v,x),v.return=g,v)}function f(g,v,x){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=Pd(""+v,g.mode,x),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case Nl:return x=Il(v.type,v.key,v.props,null,g.mode,x),Di(x,v),x.return=g,x;case ji:return v=jd(v,g.mode,x),v.return=g,v;case Tn:var w=v._init;return v=w(v._payload),f(g,v,x)}if(Ui(v)||Ei(v))return v=pr(v,g.mode,x,null),v.return=g,v;if(typeof v.then=="function")return f(g,Ml(v),x);if(v.$$typeof===an)return f(g,Al(g,v),x);Ol(g,v)}return null}function m(g,v,x,w){var S=v!==null?v.key:null;if(typeof x=="string"&&x!==""||typeof x=="number"||typeof x=="bigint")return S!==null?null:o(g,v,""+x,w);if(typeof x=="object"&&x!==null){switch(x.$$typeof){case Nl:return x.key===S?u(g,v,x,w):null;case ji:return x.key===S?c(g,v,x,w):null;case Tn:return S=x._init,x=S(x._payload),m(g,v,x,w)}if(Ui(x)||Ei(x))return S!==null?null:d(g,v,x,w,null);if(typeof x.then=="function")return m(g,v,Ml(x),w);if(x.$$typeof===an)return m(g,v,Al(g,x),w);Ol(g,x)}return null}function p(g,v,x,w,S){if(typeof w=="string"&&w!==""||typeof w=="number"||typeof w=="bigint")return g=g.get(x)||null,o(v,g,""+w,S);if(typeof w=="object"&&w!==null){switch(w.$$typeof){case Nl:return g=g.get(w.key===null?x:w.key)||null,u(v,g,w,S);case ji:return g=g.get(w.key===null?x:w.key)||null,c(v,g,w,S);case Tn:var R=w._init;return w=R(w._payload),p(g,v,x,w,S)}if(Ui(w)||Ei(w))return g=g.get(x)||null,d(v,g,w,S,null);if(typeof w.then=="function")return p(g,v,x,Ml(w),S);if(w.$$typeof===an)return p(g,v,x,Al(v,w),S);Ol(v,w)}return null}function b(g,v,x,w){for(var S=null,R=null,_=v,E=v=0,M=null;_!==null&&E<x.length;E++){_.index>E?(M=_,_=null):M=_.sibling;var P=m(g,_,x[E],w);if(P===null){_===null&&(_=M);break}e&&_&&P.alternate===null&&t(g,_),v=s(P,v,E),R===null?S=P:R.sibling=P,R=P,_=M}if(E===x.length)return a(g,_),de&&dr(g,E),S;if(_===null){for(;E<x.length;E++)_=f(g,x[E],w),_!==null&&(v=s(_,v,E),R===null?S=_:R.sibling=_,R=_);return de&&dr(g,E),S}for(_=n(_);E<x.length;E++)M=p(_,g,E,x[E],w),M!==null&&(e&&M.alternate!==null&&_.delete(M.key===null?E:M.key),v=s(M,v,E),R===null?S=M:R.sibling=M,R=M);return e&&_.forEach(function(I){return t(g,I)}),de&&dr(g,E),S}function y(g,v,x,w){if(x==null)throw Error(L(151));for(var S=null,R=null,_=v,E=v=0,M=null,P=x.next();_!==null&&!P.done;E++,P=x.next()){_.index>E?(M=_,_=null):M=_.sibling;var I=m(g,_,P.value,w);if(I===null){_===null&&(_=M);break}e&&_&&I.alternate===null&&t(g,_),v=s(I,v,E),R===null?S=I:R.sibling=I,R=I,_=M}if(P.done)return a(g,_),de&&dr(g,E),S;if(_===null){for(;!P.done;E++,P=x.next())P=f(g,P.value,w),P!==null&&(v=s(P,v,E),R===null?S=P:R.sibling=P,R=P);return de&&dr(g,E),S}for(_=n(_);!P.done;E++,P=x.next())P=p(_,g,E,P.value,w),P!==null&&(e&&P.alternate!==null&&_.delete(P.key===null?E:P.key),v=s(P,v,E),R===null?S=P:R.sibling=P,R=P);return e&&_.forEach(function(D){return t(g,D)}),de&&dr(g,E),S}function $(g,v,x,w){if(typeof x=="object"&&x!==null&&x.type===as&&x.key===null&&(x=x.props.children),typeof x=="object"&&x!==null){switch(x.$$typeof){case Nl:e:{for(var S=x.key;v!==null;){if(v.key===S){if(S=x.type,S===as){if(v.tag===7){a(g,v.sibling),w=r(v,x.props.children),w.return=g,g=w;break e}}else if(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Tn&&Wv(S)===v.type){a(g,v.sibling),w=r(v,x.props),Di(w,x),w.return=g,g=w;break e}a(g,v);break}else t(g,v);v=v.sibling}x.type===as?(w=pr(x.props.children,g.mode,w,x.key),w.return=g,g=w):(w=Il(x.type,x.key,x.props,null,g.mode,w),Di(w,x),w.return=g,g=w)}return i(g);case ji:e:{for(S=x.key;v!==null;){if(v.key===S)if(v.tag===4&&v.stateNode.containerInfo===x.containerInfo&&v.stateNode.implementation===x.implementation){a(g,v.sibling),w=r(v,x.children||[]),w.return=g,g=w;break e}else{a(g,v);break}else t(g,v);v=v.sibling}w=jd(x,g.mode,w),w.return=g,g=w}return i(g);case Tn:return S=x._init,x=S(x._payload),$(g,v,x,w)}if(Ui(x))return b(g,v,x,w);if(Ei(x)){if(S=Ei(x),typeof S!="function")throw Error(L(150));return x=S.call(x),y(g,v,x,w)}if(typeof x.then=="function")return $(g,v,Ml(x),w);if(x.$$typeof===an)return $(g,v,Al(g,x),w);Ol(g,x)}return typeof x=="string"&&x!==""||typeof x=="number"||typeof x=="bigint"?(x=""+x,v!==null&&v.tag===6?(a(g,v.sibling),w=r(v,x),w.return=g,g=w):(a(g,v),w=Pd(x,g.mode,w),w.return=g,g=w),i(g)):a(g,v)}return function(g,v,x,w){try{oo=0;var S=$(g,v,x,w);return xs=null,S}catch(_){if(_===_o||_===Pu)throw _;var R=Gt(29,_,null,g.mode);return R.lanes=w,R.return=g,R}finally{}}}var Cs=lb(!0),ub=lb(!1),pa=Fa(null),Ua=null;function Mn(e){var t=e.alternate;Pe(st,st.current&1),Pe(pa,e),Ua===null&&(t===null||ks.current!==null||t.memoizedState!==null)&&(Ua=e)}function cb(e){if(e.tag===22){if(Pe(st,st.current),Pe(pa,e),Ua===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(Ua=e)}}else On(e)}function On(){Pe(st,st.current),Pe(pa,pa.current)}function on(e){ft(pa),Ua===e&&(Ua=null),ft(st)}var st=Fa(0);function pu(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||Qm(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function zd(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:De({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var Tm={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=Xt(),r=qn(n);r.payload=t,a!=null&&(r.callback=a),t=Bn(e,r,n),t!==null&&(Zt(t,e,n),Qi(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=Xt(),r=qn(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=Bn(e,r,n),t!==null&&(Zt(t,e,n),Qi(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=Xt(),n=qn(a);n.tag=2,t!=null&&(n.callback=t),t=Bn(e,n,a),t!==null&&(Zt(t,e,a),Qi(t,e,a))}};function eg(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!ro(a,n)||!ro(r,s):!0}function tg(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&Tm.enqueueReplaceState(t,t.state,null)}function Sr(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=De({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var hu=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function db(e){hu(e)}function mb(e){console.error(e)}function fb(e){hu(e)}function vu(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function ag(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function Am(e,t,a){return a=qn(a),a.tag=3,a.payload={element:null},a.callback=function(){vu(e,t)},a}function pb(e){return e=qn(e),e.tag=3,e}function hb(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){ag(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){ag(t,a,n),typeof r!="function"&&(Hn===null?Hn=new Set([this]):Hn.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function oC(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&So(t,a,r,!0),a=pa.current,a!==null){switch(a.tag){case 13:return Ua===null?Fm():a.alternate===null&&He===0&&(He=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===Sm?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),Xd(e,n,r)),!1;case 22:return a.flags|=65536,n===Sm?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),Xd(e,n,r)),!1}throw Error(L(435,a.tag))}return Xd(e,n,r),Fm(),!1}if(de)return t=pa.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==ym&&(e=Error(L(422),{cause:n}),so(ma(e,a)))):(n!==ym&&(t=Error(L(423),{cause:n}),so(ma(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=ma(n,a),r=Am(e.stateNode,n,r),Ud(e,r),He!==4&&(He=2)),!1;var s=Error(L(520),{cause:n});if(s=ma(s,a),Zi===null?Zi=[s]:Zi.push(s),He!==4&&(He=2),t===null)return!0;n=ma(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=Am(a.stateNode,n,e),Ud(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(Hn===null||!Hn.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=pb(r),hb(r,e,a,n),Ud(a,r),!1}a=a.return}while(a!==null);return!1}var vb=Error(L(461)),mt=!1;function vt(e,t,a,n){t.child=e===null?ub(t,null,a,n):Cs(t,e.child,a,n)}function ng(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return $r(t),n=xf(e,t,a,i,s,r),o=$f(),e!==null&&!mt?(wf(e,t,r),mn(e,t,r)):(de&&o&&ff(t),t.flags|=1,vt(e,t,n,r),t.child)}function rg(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!mf(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,gb(e,t,s,n,r)):(e=Il(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!Af(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:ro,a(i,n)&&e.ref===t.ref)return mn(e,t,r)}return t.flags|=1,e=ln(s,n),e.ref=t.ref,e.return=t,t.child=e}function gb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if(ro(s,n)&&e.ref===t.ref)if(mt=!1,t.pendingProps=n=s,Af(e,r))(e.flags&131072)!==0&&(mt=!0);else return t.lanes=e.lanes,mn(e,t,r)}return Dm(e,t,a,n,r)}function yb(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return sg(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&Ql(t,s!==null?s.cachePool:null),s!==null?Qv(t,s):km(),cb(t);else return t.lanes=t.childLanes=536870912,sg(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(Ql(t,s.cachePool),Qv(t,s),On(t),t.memoizedState=null):(e!==null&&Ql(t,null),km(),On(t));return vt(e,t,r,a),t.child}function sg(e,t,a,n){var r=vf();return r=r===null?null:{parent:rt._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&Ql(t,null),km(),cb(t),e!==null&&So(e,t,n,!0),null}function Yl(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(L(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function Dm(e,t,a,n,r){return $r(t),a=xf(e,t,a,n,void 0,r),n=$f(),e!==null&&!mt?(wf(e,t,r),mn(e,t,r)):(de&&n&&ff(t),t.flags|=1,vt(e,t,a,r),t.child)}function ig(e,t,a,n,r,s){return $r(t),t.updateQueue=null,a=Ay(t,n,a,r),Ty(e),n=$f(),e!==null&&!mt?(wf(e,t,s),mn(e,t,s)):(de&&n&&ff(t),t.flags|=1,vt(e,t,a,s),t.child)}function og(e,t,a,n,r){if($r(t),t.stateNode===null){var s=cs,i=a.contextType;typeof i=="object"&&i!==null&&(s=wt(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=Tm,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},gf(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?wt(i):cs,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(zd(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&Tm.enqueueReplaceState(s,s.state,null),Gi(t,n,s,r),Vi(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,u=Sr(a,o);s.props=u;var c=s.context,d=a.contextType;i=cs,typeof d=="object"&&d!==null&&(i=wt(d));var f=a.getDerivedStateFromProps;d=typeof f=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&tg(t,s,n,i),An=!1;var m=t.memoizedState;s.state=m,Gi(t,n,s,r),Vi(),c=t.memoizedState,o||m!==c||An?(typeof f=="function"&&(zd(t,a,f,n),c=t.memoizedState),(u=An||eg(t,a,u,n,m,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=u):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,Nm(e,t),i=t.memoizedProps,d=Sr(a,i),s.props=d,f=t.pendingProps,m=s.context,c=a.contextType,u=cs,typeof c=="object"&&c!==null&&(u=wt(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==f||m!==u)&&tg(t,s,n,u),An=!1,m=t.memoizedState,s.state=m,Gi(t,n,s,r),Vi();var p=t.memoizedState;i!==f||m!==p||An||e!==null&&e.dependencies!==null&&uu(e.dependencies)?(typeof o=="function"&&(zd(t,a,o,n),p=t.memoizedState),(d=An||eg(t,a,d,n,m,p,u)||e!==null&&e.dependencies!==null&&uu(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,p,u),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,p,u)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=p),s.props=n,s.state=p,s.context=u,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,Yl(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=Cs(t,e.child,null,r),t.child=Cs(t,null,a,r)):vt(e,t,a,r),t.memoizedState=s.state,e=t.child):e=mn(e,t,r),e}function lg(e,t,a,n){return wo(),t.flags|=256,vt(e,t,a,n),t.child}var qd={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function Bd(e){return{baseLanes:e,cachePool:_y()}}function Hd(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=fa),e}function bb(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(st.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(de){if(r?Mn(t):On(t),de){var o=Be,u;if(u=o){e:{for(u=o,o=Oa;u.nodeType!==8;){if(!o){o=null;break e}if(u=$a(u.nextSibling),u===null){o=null;break e}}o=u}o!==null?(t.memoizedState={dehydrated:o,treeContext:hr!==null?{id:nn,overflow:rn}:null,retryLane:536870912,hydrationErrors:null},u=Gt(18,null,null,0),u.stateNode=o,u.return=t,t.child=u,Et=t,Be=null,u=!0):u=!1}u||xr(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return Qm(o)?t.lanes=32:t.lanes=536870912,null;on(t)}return o=n.children,n=n.fallback,r?(On(t),r=t.mode,o=gu({mode:"hidden",children:o},r),n=pr(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=Bd(a),r.childLanes=Hd(e,i,a),t.memoizedState=qd,n):(Mn(t),Mm(t,o))}if(u=e.memoizedState,u!==null&&(o=u.dehydrated,o!==null)){if(s)t.flags&256?(Mn(t),t.flags&=-257,t=Kd(e,t,a)):t.memoizedState!==null?(On(t),t.child=e.child,t.flags|=128,t=null):(On(t),r=n.fallback,o=t.mode,n=gu({mode:"visible",children:n.children},o),r=pr(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,Cs(t,e.child,null,a),n=t.child,n.memoizedState=Bd(a),n.childLanes=Hd(e,i,a),t.memoizedState=qd,t=r);else if(Mn(t),Qm(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(L(419)),n.stack="",n.digest=i,so({value:n,source:null,stack:null}),t=Kd(e,t,a)}else if(mt||So(e,t,a,!1),i=(a&e.childLanes)!==0,mt||i){if(i=Ee,i!==null&&(n=a&-a,n=(n&42)!==0?1:ef(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==u.retryLane))throw u.retryLane=n,Ls(e,n),Zt(i,e,n),vb;o.data==="$?"||Fm(),t=Kd(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=u.treeContext,Be=$a(o.nextSibling),Et=t,de=!0,vr=null,Oa=!1,e!==null&&(ua[ca++]=nn,ua[ca++]=rn,ua[ca++]=hr,nn=e.id,rn=e.overflow,hr=t),t=Mm(t,n.children),t.flags|=4096);return t}return r?(On(t),r=n.fallback,o=t.mode,u=e.child,c=u.sibling,n=ln(u,{mode:"hidden",children:n.children}),n.subtreeFlags=u.subtreeFlags&65011712,c!==null?r=ln(c,r):(r=pr(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=Bd(a):(u=o.cachePool,u!==null?(c=rt._currentValue,u=u.parent!==c?{parent:c,pool:c}:u):u=_y(),o={baseLanes:o.baseLanes|a,cachePool:u}),r.memoizedState=o,r.childLanes=Hd(e,i,a),t.memoizedState=qd,n):(Mn(t),a=e.child,e=a.sibling,a=ln(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function Mm(e,t){return t=gu({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function gu(e,t){return e=Gt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function Kd(e,t,a){return Cs(t,e.child,null,a),e=Mm(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function ug(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),xm(e.return,t,a)}function Id(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function xb(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(vt(e,t,n.children,a),n=st.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&ug(e,a,t);else if(e.tag===19)ug(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Pe(st,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&pu(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),Id(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&pu(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}Id(t,!0,a,null,s);break;case"together":Id(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function mn(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),Jn|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(So(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(L(153));if(t.child!==null){for(e=t.child,a=ln(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=ln(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function Af(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&uu(e)))}function lC(e,t,a){switch(t.tag){case 3:tu(t,t.stateNode.containerInfo),Dn(t,rt,e.memoizedState.cache),wo();break;case 27:case 5:lm(t);break;case 4:tu(t,t.stateNode.containerInfo);break;case 10:Dn(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(Mn(t),t.flags|=128,null):(a&t.child.childLanes)!==0?bb(e,t,a):(Mn(t),e=mn(e,t,a),e!==null?e.sibling:null);Mn(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(So(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return xb(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Pe(st,st.current),n)break;return null;case 22:case 23:return t.lanes=0,yb(e,t,a);case 24:Dn(t,rt,e.memoizedState.cache)}return mn(e,t,a)}function $b(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)mt=!0;else{if(!Af(e,a)&&(t.flags&128)===0)return mt=!1,lC(e,t,a);mt=(e.flags&131072)!==0}else mt=!1,de&&(t.flags&1048576)!==0&&Sy(t,lu,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")mf(n)?(e=Sr(n,e),t.tag=1,t=og(null,t,n,e,a)):(t.tag=0,t=Dm(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===Xm){t.tag=11,t=ng(null,t,n,e,a);break e}else if(r===Zm){t.tag=14,t=rg(null,t,n,e,a);break e}}throw t=im(n)||n,Error(L(306,t,""))}}return t;case 0:return Dm(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Sr(n,t.pendingProps),og(e,t,n,r,a);case 3:e:{if(tu(t,t.stateNode.containerInfo),e===null)throw Error(L(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,Nm(e,t),Gi(t,n,null,a);var i=t.memoizedState;if(n=i.cache,Dn(t,rt,n),n!==s.cache&&$m(t,[rt],a,!0),Vi(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=lg(e,t,n,a);break e}else if(n!==r){r=ma(Error(L(424)),t),so(r),t=lg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(Be=$a(e.firstChild),Et=t,de=!0,vr=null,Oa=!0,a=ub(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if(wo(),n===r){t=mn(e,t,a);break e}vt(e,t,n,a)}t=t.child}return t;case 26:return Yl(e,t),e===null?(a=Cg(t.type,null,t.pendingProps,null))?t.memoizedState=a:de||(a=t.type,e=t.pendingProps,n=Nu(zn.current).createElement(a),n[$t]=t,n[zt]=e,yt(n,a,e),dt(n),t.stateNode=n):t.memoizedState=Cg(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return lm(t),e===null&&de&&(n=t.stateNode=o0(t.type,t.pendingProps,zn.current),Et=t,Oa=!0,r=Be,Zn(t.type)?(Vm=r,Be=$a(n.firstChild)):Be=r),vt(e,t,t.pendingProps.children,a),Yl(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&de&&((r=n=Be)&&(n=OC(n,t.type,t.pendingProps,Oa),n!==null?(t.stateNode=n,Et=t,Be=$a(n.firstChild),Oa=!1,r=!0):r=!1),r||xr(t)),lm(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,Km(r,s)?n=null:i!==null&&Km(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=xf(e,t,eC,null,null,a),mo._currentValue=r),Yl(e,t),vt(e,t,n,a),t.child;case 6:return e===null&&de&&((e=a=Be)&&(a=LC(a,t.pendingProps,Oa),a!==null?(t.stateNode=a,Et=t,Be=null,e=!0):e=!1),e||xr(t)),null;case 13:return bb(e,t,a);case 4:return tu(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=Cs(t,null,n,a):vt(e,t,n,a),t.child;case 11:return ng(e,t,t.type,t.pendingProps,a);case 7:return vt(e,t,t.pendingProps,a),t.child;case 8:return vt(e,t,t.pendingProps.children,a),t.child;case 12:return vt(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,Dn(t,t.type,n.value),vt(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,$r(t),r=wt(r),n=n(r),t.flags|=1,vt(e,t,n,a),t.child;case 14:return rg(e,t,t.type,t.pendingProps,a);case 15:return gb(e,t,t.type,t.pendingProps,a);case 19:return xb(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=gu(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=ln(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return yb(e,t,a);case 24:return $r(t),n=wt(rt),e===null?(r=vf(),r===null&&(r=Ee,s=hf(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},gf(t),Dn(t,rt,r)):((e.lanes&a)!==0&&(Nm(e,t),Gi(t,null,null,a),Vi()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),Dn(t,rt,n)):(n=s.cache,Dn(t,rt,n),n!==r.cache&&$m(t,[rt],a,!0))),vt(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(L(156,t.tag))}function Wa(e){e.flags|=4}function cg(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!c0(t)){if(t=pa.current,t!==null&&((ue&4194048)===ue?Ua!==null:(ue&62914560)!==ue&&(ue&536870912)===0||t!==Ua))throw Ii=Sm,ky;e.flags|=8192}}function Ll(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?Vg():536870912,e.lanes|=t,Es|=t)}function Mi(e,t){if(!de)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function ze(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function uC(e,t,a){var n=t.pendingProps;switch(pf(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return ze(t),null;case 1:return ze(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),un(rt),ws(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Ai(t)?Wa(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,zv())),ze(t),null;case 26:return a=t.memoizedState,e===null?(Wa(t),a!==null?(ze(t),cg(t,a)):(ze(t),t.flags&=-16777217)):a?a!==e.memoizedState?(Wa(t),ze(t),cg(t,a)):(ze(t),t.flags&=-16777217):(e.memoizedProps!==n&&Wa(t),ze(t),t.flags&=-16777217),null;case 27:au(t),a=zn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&Wa(t);else{if(!n){if(t.stateNode===null)throw Error(L(166));return ze(t),null}e=Pa.current,Ai(t)?Uv(t,e):(e=o0(r,n,a),t.stateNode=e,Wa(t))}return ze(t),null;case 5:if(au(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&Wa(t);else{if(!n){if(t.stateNode===null)throw Error(L(166));return ze(t),null}if(e=Pa.current,Ai(t))Uv(t,e);else{switch(r=Nu(zn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[$t]=t,e[zt]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(yt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&Wa(t)}}return ze(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&Wa(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(L(166));if(e=zn.current,Ai(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=Et,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[$t]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||r0(e.nodeValue,a)),e||xr(t)}else e=Nu(e).createTextNode(n),e[$t]=t,t.stateNode=e}return ze(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Ai(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(L(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(L(317));r[$t]=t}else wo(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;ze(t),r=!1}else r=zv(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(on(t),t):(on(t),null)}if(on(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),Ll(t,t.updateQueue),ze(t),null;case 4:return ws(),e===null&&Ff(t.stateNode.containerInfo),ze(t),null;case 10:return un(t.type),ze(t),null;case 19:if(ft(st),r=t.memoizedState,r===null)return ze(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)Mi(r,!1);else{if(He!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=pu(e),s!==null){for(t.flags|=128,Mi(r,!1),e=s.updateQueue,t.updateQueue=e,Ll(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)wy(a,e),a=a.sibling;return Pe(st,st.current&1|2),t.child}e=e.sibling}r.tail!==null&&ja()>bu&&(t.flags|=128,n=!0,Mi(r,!1),t.lanes=4194304)}else{if(!n)if(e=pu(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,Ll(t,e),Mi(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!de)return ze(t),null}else 2*ja()-r.renderingStartTime>bu&&a!==536870912&&(t.flags|=128,n=!0,Mi(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=ja(),t.sibling=null,e=st.current,Pe(st,n?e&1|2:e&1),t):(ze(t),null);case 22:case 23:return on(t),yf(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(ze(t),t.subtreeFlags&6&&(t.flags|=8192)):ze(t),a=t.updateQueue,a!==null&&Ll(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&ft(gr),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),un(rt),ze(t),null;case 25:return null;case 30:return null}throw Error(L(156,t.tag))}function cC(e,t){switch(pf(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return un(rt),ws(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return au(t),null;case 13:if(on(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(L(340));wo()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return ft(st),null;case 4:return ws(),null;case 10:return un(t.type),null;case 22:case 23:return on(t),yf(),e!==null&&ft(gr),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return un(rt),null;case 25:return null;default:return null}}function wb(e,t){switch(pf(t),t.tag){case 3:un(rt),ws();break;case 26:case 27:case 5:au(t);break;case 4:ws();break;case 13:on(t);break;case 19:ft(st);break;case 10:un(t.type);break;case 22:case 23:on(t),yf(),e!==null&&ft(gr);break;case 24:un(rt)}}function Co(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){Ne(t,t.return,o)}}function Yn(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var u=a,c=o;try{c()}catch(d){Ne(r,u,d)}}}n=n.next}while(n!==s)}}catch(d){Ne(t,t.return,d)}}function Sb(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{Ey(t,a)}catch(n){Ne(e,e.return,n)}}}function Nb(e,t,a){a.props=Sr(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){Ne(e,t,n)}}function Ji(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){Ne(e,t,r)}}function La(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){Ne(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){Ne(e,t,r)}else a.current=null}function _b(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){Ne(e,e.return,r)}}function Qd(e,t,a){try{var n=e.stateNode;EC(n,e.type,a,t),n[zt]=t}catch(r){Ne(e,e.return,r)}}function kb(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&Zn(e.type)||e.tag===4}function Vd(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||kb(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&Zn(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function Om(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=Hu));else if(n!==4&&(n===27&&Zn(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(Om(e,t,a),e=e.sibling;e!==null;)Om(e,t,a),e=e.sibling}function yu(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&Zn(e.type)&&(a=e.stateNode),e=e.child,e!==null))for(yu(e,t,a),e=e.sibling;e!==null;)yu(e,t,a),e=e.sibling}function Rb(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);yt(t,n,a),t[$t]=e,t[zt]=a}catch(s){Ne(e,e.return,s)}}var tn=!1,Ve=!1,Gd=!1,dg=typeof WeakSet=="function"?WeakSet:Set,ct=null;function dC(e,t){if(e=e.containerInfo,Bm=Cu,e=py(e),uf(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,u=-1,c=0,d=0,f=e,m=null;t:for(;;){for(var p;f!==a||r!==0&&f.nodeType!==3||(o=i+r),f!==s||n!==0&&f.nodeType!==3||(u=i+n),f.nodeType===3&&(i+=f.nodeValue.length),(p=f.firstChild)!==null;)m=f,f=p;for(;;){if(f===e)break t;if(m===a&&++c===r&&(o=i),m===s&&++d===n&&(u=i),(p=f.nextSibling)!==null)break;f=m,m=f.parentNode}f=p}a=o===-1||u===-1?null:{start:o,end:u}}else a=null}a=a||{start:0,end:0}}else a=null;for(Hm={focusedElem:e,selectionRange:a},Cu=!1,ct=t;ct!==null;)if(t=ct,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,ct=e;else for(;ct!==null;){switch(t=ct,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var b=Sr(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(b,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(y){Ne(a,a.return,y)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)Im(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":Im(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(L(163))}if(e=t.sibling,e!==null){e.return=t.return,ct=e;break}ct=t.return}}function Cb(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:Cn(e,a),n&4&&Co(5,a);break;case 1:if(Cn(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){Ne(a,a.return,i)}else{var r=Sr(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){Ne(a,a.return,i)}}n&64&&Sb(a),n&512&&Ji(a,a.return);break;case 3:if(Cn(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{Ey(e,t)}catch(i){Ne(a,a.return,i)}}break;case 27:t===null&&n&4&&Rb(a);case 26:case 5:Cn(e,a),t===null&&n&4&&_b(a),n&512&&Ji(a,a.return);break;case 12:Cn(e,a);break;case 13:Cn(e,a),n&4&&Ab(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=xC.bind(null,a),PC(e,a))));break;case 22:if(n=a.memoizedState!==null||tn,!n){t=t!==null&&t.memoizedState!==null||Ve,r=tn;var s=Ve;tn=n,(Ve=t)&&!s?En(e,a,(a.subtreeFlags&8772)!==0):Cn(e,a),tn=r,Ve=s}break;case 30:break;default:Cn(e,a)}}function Eb(e){var t=e.alternate;t!==null&&(e.alternate=null,Eb(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&af(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var Le=null,Ut=!1;function en(e,t,a){for(a=a.child;a!==null;)Tb(e,t,a),a=a.sibling}function Tb(e,t,a){if(Yt&&typeof Yt.onCommitFiberUnmount=="function")try{Yt.onCommitFiberUnmount(go,a)}catch{}switch(a.tag){case 26:Ve||La(a,t),en(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:Ve||La(a,t);var n=Le,r=Ut;Zn(a.type)&&(Le=a.stateNode,Ut=!1),en(e,t,a),eo(a.stateNode),Le=n,Ut=r;break;case 5:Ve||La(a,t);case 6:if(n=Le,r=Ut,Le=null,en(e,t,a),Le=n,Ut=r,Le!==null)if(Ut)try{(Le.nodeType===9?Le.body:Le.nodeName==="HTML"?Le.ownerDocument.body:Le).removeChild(a.stateNode)}catch(s){Ne(a,t,s)}else try{Le.removeChild(a.stateNode)}catch(s){Ne(a,t,s)}break;case 18:Le!==null&&(Ut?(e=Le,_g(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),ho(e)):_g(Le,a.stateNode));break;case 4:n=Le,r=Ut,Le=a.stateNode.containerInfo,Ut=!0,en(e,t,a),Le=n,Ut=r;break;case 0:case 11:case 14:case 15:Ve||Yn(2,a,t),Ve||Yn(4,a,t),en(e,t,a);break;case 1:Ve||(La(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&Nb(a,t,n)),en(e,t,a);break;case 21:en(e,t,a);break;case 22:Ve=(n=Ve)||a.memoizedState!==null,en(e,t,a),Ve=n;break;default:en(e,t,a)}}function Ab(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{ho(e)}catch(a){Ne(t,t.return,a)}}function mC(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new dg),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new dg),t;default:throw Error(L(435,e.tag))}}function Yd(e,t){var a=mC(e);t.forEach(function(n){var r=$C.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function It(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(Zn(o.type)){Le=o.stateNode,Ut=!1;break e}break;case 5:Le=o.stateNode,Ut=!1;break e;case 3:case 4:Le=o.stateNode.containerInfo,Ut=!0;break e}o=o.return}if(Le===null)throw Error(L(160));Tb(s,i,r),Le=null,Ut=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)Db(t,e),t=t.sibling}var xa=null;function Db(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:It(t,e),Qt(e),n&4&&(Yn(3,e,e.return),Co(3,e),Yn(5,e,e.return));break;case 1:It(t,e),Qt(e),n&512&&(Ve||a===null||La(a,a.return)),n&64&&tn&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=xa;if(It(t,e),Qt(e),n&512&&(Ve||a===null||La(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[xo]||s[$t]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),yt(s,n,a),s[$t]=e,dt(s),n=s;break e;case"link":var i=Tg("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),yt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=Tg("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),yt(s,n,a),r.head.appendChild(s);break;default:throw Error(L(468,n))}s[$t]=e,dt(s),n=s}e.stateNode=n}else Ag(r,e.type,e.stateNode);else e.stateNode=Eg(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?Ag(r,e.type,e.stateNode):Eg(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&Qd(e,e.memoizedProps,a.memoizedProps)}break;case 27:It(t,e),Qt(e),n&512&&(Ve||a===null||La(a,a.return)),a!==null&&n&4&&Qd(e,e.memoizedProps,a.memoizedProps);break;case 5:if(It(t,e),Qt(e),n&512&&(Ve||a===null||La(a,a.return)),e.flags&32){r=e.stateNode;try{Ns(r,"")}catch(p){Ne(e,e.return,p)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,Qd(e,r,a!==null?a.memoizedProps:r)),n&1024&&(Gd=!0);break;case 6:if(It(t,e),Qt(e),n&4){if(e.stateNode===null)throw Error(L(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(p){Ne(e,e.return,p)}}break;case 3:if(Zl=null,r=xa,xa=_u(t.containerInfo),It(t,e),xa=r,Qt(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{ho(t.containerInfo)}catch(p){Ne(e,e.return,p)}Gd&&(Gd=!1,Mb(e));break;case 4:n=xa,xa=_u(e.stateNode.containerInfo),It(t,e),Qt(e),xa=n;break;case 12:It(t,e),Qt(e);break;case 13:It(t,e),Qt(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(Pf=ja()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,Yd(e,n)));break;case 22:r=e.memoizedState!==null;var u=a!==null&&a.memoizedState!==null,c=tn,d=Ve;if(tn=c||r,Ve=d||u,It(t,e),Ve=d,tn=c,Qt(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||u||tn||Ve||mr(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){u=a=t;try{if(s=u.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=u.stateNode;var f=u.memoizedProps.style,m=f!=null&&f.hasOwnProperty("display")?f.display:null;o.style.display=m==null||typeof m=="boolean"?"":(""+m).trim()}}catch(p){Ne(u,u.return,p)}}}else if(t.tag===6){if(a===null){u=t;try{u.stateNode.nodeValue=r?"":u.memoizedProps}catch(p){Ne(u,u.return,p)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,Yd(e,a))));break;case 19:It(t,e),Qt(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,Yd(e,n)));break;case 30:break;case 21:break;default:It(t,e),Qt(e)}}function Qt(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(kb(n)){a=n;break}n=n.return}if(a==null)throw Error(L(160));switch(a.tag){case 27:var r=a.stateNode,s=Vd(e);yu(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(Ns(i,""),a.flags&=-33);var o=Vd(e);yu(e,o,i);break;case 3:case 4:var u=a.stateNode.containerInfo,c=Vd(e);Om(e,c,u);break;default:throw Error(L(161))}}catch(d){Ne(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function Mb(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;Mb(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function Cn(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)Cb(e,t.alternate,t),t=t.sibling}function mr(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:Yn(4,t,t.return),mr(t);break;case 1:La(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&Nb(t,t.return,a),mr(t);break;case 27:eo(t.stateNode);case 26:case 5:La(t,t.return),mr(t);break;case 22:t.memoizedState===null&&mr(t);break;case 30:mr(t);break;default:mr(t)}e=e.sibling}}function En(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:En(r,s,a),Co(4,s);break;case 1:if(En(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){Ne(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var u=r.shared.hiddenCallbacks;if(u!==null)for(r.shared.hiddenCallbacks=null,r=0;r<u.length;r++)Cy(u[r],o)}catch(c){Ne(n,n.return,c)}}a&&i&64&&Sb(s),Ji(s,s.return);break;case 27:Rb(s);case 26:case 5:En(r,s,a),a&&n===null&&i&4&&_b(s),Ji(s,s.return);break;case 12:En(r,s,a);break;case 13:En(r,s,a),a&&i&4&&Ab(r,s);break;case 22:s.memoizedState===null&&En(r,s,a),Ji(s,s.return);break;case 30:break;default:En(r,s,a)}t=t.sibling}}function Df(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&No(a))}function Mf(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&No(e))}function Ma(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)Ob(e,t,a,n),t=t.sibling}function Ob(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:Ma(e,t,a,n),r&2048&&Co(9,t);break;case 1:Ma(e,t,a,n);break;case 3:Ma(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&No(e)));break;case 12:if(r&2048){Ma(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(u){Ne(t,t.return,u)}}else Ma(e,t,a,n);break;case 13:Ma(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?Ma(e,t,a,n):Xi(e,t):s._visibility&2?Ma(e,t,a,n):(s._visibility|=2,es(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&Df(i,t);break;case 24:Ma(e,t,a,n),r&2048&&Mf(t.alternate,t);break;default:Ma(e,t,a,n)}}function es(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,u=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:es(s,i,o,u,r),Co(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?es(s,i,o,u,r):Xi(s,i):(d._visibility|=2,es(s,i,o,u,r)),r&&c&2048&&Df(i.alternate,i);break;case 24:es(s,i,o,u,r),r&&c&2048&&Mf(i.alternate,i);break;default:es(s,i,o,u,r)}t=t.sibling}}function Xi(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:Xi(a,n),r&2048&&Df(n.alternate,n);break;case 24:Xi(a,n),r&2048&&Mf(n.alternate,n);break;default:Xi(a,n)}t=t.sibling}}var zi=8192;function Xr(e){if(e.subtreeFlags&zi)for(e=e.child;e!==null;)Lb(e),e=e.sibling}function Lb(e){switch(e.tag){case 26:Xr(e),e.flags&zi&&e.memoizedState!==null&&YC(xa,e.memoizedState,e.memoizedProps);break;case 5:Xr(e);break;case 3:case 4:var t=xa;xa=_u(e.stateNode.containerInfo),Xr(e),xa=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=zi,zi=16777216,Xr(e),zi=t):Xr(e));break;default:Xr(e)}}function Pb(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function Oi(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ct=n,Ub(n,e)}Pb(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)jb(e),e=e.sibling}function jb(e){switch(e.tag){case 0:case 11:case 15:Oi(e),e.flags&2048&&Yn(9,e,e.return);break;case 3:Oi(e);break;case 12:Oi(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,Jl(e)):Oi(e);break;default:Oi(e)}}function Jl(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ct=n,Ub(n,e)}Pb(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:Yn(8,t,t.return),Jl(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,Jl(t));break;default:Jl(t)}e=e.sibling}}function Ub(e,t){for(;ct!==null;){var a=ct;switch(a.tag){case 0:case 11:case 15:Yn(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:No(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,ct=n;else e:for(a=e;ct!==null;){n=ct;var r=n.sibling,s=n.return;if(Eb(n),n===a){ct=null;break e}if(r!==null){r.return=s,ct=r;break e}ct=s}}}var fC={getCacheForType:function(e){var t=wt(rt),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},pC=typeof WeakMap=="function"?WeakMap:Map,xe=0,Ee=null,ie=null,ue=0,be=0,Vt=null,Un=!1,Ps=!1,Of=!1,fn=0,He=0,Jn=0,yr=0,Lf=0,fa=0,Es=0,Zi=null,Ft=null,Lm=!1,Pf=0,bu=1/0,xu=null,Hn=null,gt=0,Kn=null,Ts=null,$s=0,Pm=0,jm=null,Fb=null,Wi=0,Um=null;function Xt(){if((xe&2)!==0&&ue!==0)return ue&-ue;if(ae.T!==null){var e=_s;return e!==0?e:Uf()}return Jg()}function zb(){fa===0&&(fa=(ue&536870912)===0||de?Qg():536870912);var e=pa.current;return e!==null&&(e.flags|=32),fa}function Zt(e,t,a){(e===Ee&&(be===2||be===9)||e.cancelPendingCommit!==null)&&(As(e,0),Fn(e,ue,fa,!1)),bo(e,a),((xe&2)===0||e!==Ee)&&(e===Ee&&((xe&2)===0&&(yr|=a),He===4&&Fn(e,ue,fa,!1)),za(e))}function qb(e,t,a){if((xe&6)!==0)throw Error(L(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||yo(e,t),r=n?gC(e,t):Jd(e,t,!0),s=n;do{if(r===0){Ps&&!n&&Fn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!hC(a)){r=Jd(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=Zi;var u=o.current.memoizedState.isDehydrated;if(u&&(As(o,i).flags|=256),i=Jd(o,i,!1),i!==2){if(Of&&!u){o.errorRecoveryDisabledLanes|=s,yr|=s,r=4;break e}s=Ft,Ft=r,s!==null&&(Ft===null?Ft=s:Ft.push.apply(Ft,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){As(e,0),Fn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(L(345));case 4:if((t&4194048)!==t)break;case 6:Fn(n,t,fa,!Un);break e;case 2:Ft=null;break;case 3:case 5:break;default:throw Error(L(329))}if((t&62914560)===t&&(r=Pf+300-ja(),10<r)){if(Fn(n,t,fa,!Un),Tu(n,0,!0)!==0)break e;n.timeoutHandle=i0(mg.bind(null,n,a,Ft,xu,Lm,t,fa,yr,Es,Un,s,2,-0,0),r);break e}mg(n,a,Ft,xu,Lm,t,fa,yr,Es,Un,s,0,-0,0)}}break}while(!0);za(e)}function mg(e,t,a,n,r,s,i,o,u,c,d,f,m,p){if(e.timeoutHandle=-1,f=t.subtreeFlags,(f&8192||(f&16785408)===16785408)&&(co={stylesheets:null,count:0,unsuspend:GC},Lb(t),f=JC(),f!==null)){e.cancelPendingCommit=f(pg.bind(null,e,t,s,a,n,r,i,o,u,d,1,m,p)),Fn(e,s,i,!c);return}pg(e,t,s,a,n,r,i,o,u)}function hC(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!Wt(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function Fn(e,t,a,n){t&=~Lf,t&=~yr,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Jt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&Gg(e,a,t)}function zu(){return(xe&6)===0?(Eo(0,!1),!1):!0}function jf(){if(ie!==null){if(be===0)var e=ie.return;else e=ie,sn=Rr=null,Sf(e),xs=null,oo=0,e=ie;for(;e!==null;)wb(e.alternate,e),e=e.return;ie=null}}function As(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,AC(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),jf(),Ee=e,ie=a=ln(e.current,null),ue=t,be=0,Vt=null,Un=!1,Ps=yo(e,t),Of=!1,Es=fa=Lf=yr=Jn=He=0,Ft=Zi=null,Lm=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Jt(n),s=1<<r;t|=e[r],n&=~s}return fn=t,Ou(),a}function Bb(e,t){re=null,ae.H=fu,t===_o||t===Pu?(t=Kv(),be=3):t===ky?(t=Kv(),be=4):be=t===vb?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Vt=t,ie===null&&(He=1,vu(e,ma(t,e.current)))}function Hb(){var e=ae.H;return ae.H=fu,e===null?fu:e}function Kb(){var e=ae.A;return ae.A=fC,e}function Fm(){He=4,Un||(ue&4194048)!==ue&&pa.current!==null||(Ps=!0),(Jn&134217727)===0&&(yr&134217727)===0||Ee===null||Fn(Ee,ue,fa,!1)}function Jd(e,t,a){var n=xe;xe|=2;var r=Hb(),s=Kb();(Ee!==e||ue!==t)&&(xu=null,As(e,t)),t=!1;var i=He;e:do try{if(be!==0&&ie!==null){var o=ie,u=Vt;switch(be){case 8:jf(),i=6;break e;case 3:case 2:case 9:case 6:pa.current===null&&(t=!0);var c=be;if(be=0,Vt=null,fs(e,o,u,c),a&&Ps){i=0;break e}break;default:c=be,be=0,Vt=null,fs(e,o,u,c)}}vC(),i=He;break}catch(d){Bb(e,d)}while(!0);return t&&e.shellSuspendCounter++,sn=Rr=null,xe=n,ae.H=r,ae.A=s,ie===null&&(Ee=null,ue=0,Ou()),i}function vC(){for(;ie!==null;)Ib(ie)}function gC(e,t){var a=xe;xe|=2;var n=Hb(),r=Kb();Ee!==e||ue!==t?(xu=null,bu=ja()+500,As(e,t)):Ps=yo(e,t);e:do try{if(be!==0&&ie!==null){t=ie;var s=Vt;t:switch(be){case 1:be=0,Vt=null,fs(e,t,s,1);break;case 2:case 9:if(Hv(s)){be=0,Vt=null,fg(t);break}t=function(){be!==2&&be!==9||Ee!==e||(be=7),za(e)},s.then(t,t);break e;case 3:be=7;break e;case 4:be=5;break e;case 7:Hv(s)?(be=0,Vt=null,fg(t)):(be=0,Vt=null,fs(e,t,s,7));break;case 5:var i=null;switch(ie.tag){case 26:i=ie.memoizedState;case 5:case 27:var o=ie;if(!i||c0(i)){be=0,Vt=null;var u=o.sibling;if(u!==null)ie=u;else{var c=o.return;c!==null?(ie=c,qu(c)):ie=null}break t}}be=0,Vt=null,fs(e,t,s,5);break;case 6:be=0,Vt=null,fs(e,t,s,6);break;case 8:jf(),He=6;break e;default:throw Error(L(462))}}yC();break}catch(d){Bb(e,d)}while(!0);return sn=Rr=null,ae.H=n,ae.A=r,xe=a,ie!==null?0:(Ee=null,ue=0,Ou(),He)}function yC(){for(;ie!==null&&!zk();)Ib(ie)}function Ib(e){var t=$b(e.alternate,e,fn);e.memoizedProps=e.pendingProps,t===null?qu(e):ie=t}function fg(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=ig(a,t,t.pendingProps,t.type,void 0,ue);break;case 11:t=ig(a,t,t.pendingProps,t.type.render,t.ref,ue);break;case 5:Sf(t);default:wb(a,t),t=ie=wy(t,fn),t=$b(a,t,fn)}e.memoizedProps=e.pendingProps,t===null?qu(e):ie=t}function fs(e,t,a,n){sn=Rr=null,Sf(t),xs=null,oo=0;var r=t.return;try{if(oC(e,r,t,a,ue)){He=1,vu(e,ma(a,e.current)),ie=null;return}}catch(s){if(r!==null)throw ie=r,s;He=1,vu(e,ma(a,e.current)),ie=null;return}t.flags&32768?(de||n===1?e=!0:Ps||(ue&536870912)!==0?e=!1:(Un=e=!0,(n===2||n===9||n===3||n===6)&&(n=pa.current,n!==null&&n.tag===13&&(n.flags|=16384))),Qb(t,e)):qu(t)}function qu(e){var t=e;do{if((t.flags&32768)!==0){Qb(t,Un);return}e=t.return;var a=uC(t.alternate,t,fn);if(a!==null){ie=a;return}if(t=t.sibling,t!==null){ie=t;return}ie=t=e}while(t!==null);He===0&&(He=5)}function Qb(e,t){do{var a=cC(e.alternate,e);if(a!==null){a.flags&=32767,ie=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){ie=e;return}ie=e=a}while(e!==null);He=6,ie=null}function pg(e,t,a,n,r,s,i,o,u){e.cancelPendingCommit=null;do Bu();while(gt!==0);if((xe&6)!==0)throw Error(L(327));if(t!==null){if(t===e.current)throw Error(L(177));if(s=t.lanes|t.childLanes,s|=cf,Jk(e,a,s,i,o,u),e===Ee&&(ie=Ee=null,ue=0),Ts=t,Kn=e,$s=a,Pm=s,jm=r,Fb=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,wC(nu,function(){return Xb(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=ae.T,ae.T=null,r=me.p,me.p=2,i=xe,xe|=4;try{dC(e,t,a)}finally{xe=i,me.p=r,ae.T=n}}gt=1,Vb(),Gb(),Yb()}}function Vb(){if(gt===1){gt=0;var e=Kn,t=Ts,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=ae.T,ae.T=null;var n=me.p;me.p=2;var r=xe;xe|=4;try{Db(t,e);var s=Hm,i=py(e.containerInfo),o=s.focusedElem,u=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&fy(o.ownerDocument.documentElement,o)){if(u!==null&&uf(o)){var c=u.start,d=u.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var f=o.ownerDocument||document,m=f&&f.defaultView||window;if(m.getSelection){var p=m.getSelection(),b=o.textContent.length,y=Math.min(u.start,b),$=u.end===void 0?y:Math.min(u.end,b);!p.extend&&y>$&&(i=$,$=y,y=i);var g=Lv(o,y),v=Lv(o,$);if(g&&v&&(p.rangeCount!==1||p.anchorNode!==g.node||p.anchorOffset!==g.offset||p.focusNode!==v.node||p.focusOffset!==v.offset)){var x=f.createRange();x.setStart(g.node,g.offset),p.removeAllRanges(),y>$?(p.addRange(x),p.extend(v.node,v.offset)):(x.setEnd(v.node,v.offset),p.addRange(x))}}}}for(f=[],p=o;p=p.parentNode;)p.nodeType===1&&f.push({element:p,left:p.scrollLeft,top:p.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<f.length;o++){var w=f[o];w.element.scrollLeft=w.left,w.element.scrollTop=w.top}}Cu=!!Bm,Hm=Bm=null}finally{xe=r,me.p=n,ae.T=a}}e.current=t,gt=2}}function Gb(){if(gt===2){gt=0;var e=Kn,t=Ts,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=ae.T,ae.T=null;var n=me.p;me.p=2;var r=xe;xe|=4;try{Cb(e,t.alternate,t)}finally{xe=r,me.p=n,ae.T=a}}gt=3}}function Yb(){if(gt===4||gt===3){gt=0,qk();var e=Kn,t=Ts,a=$s,n=Fb;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?gt=5:(gt=0,Ts=Kn=null,Jb(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(Hn=null),tf(a),t=t.stateNode,Yt&&typeof Yt.onCommitFiberRoot=="function")try{Yt.onCommitFiberRoot(go,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=ae.T,r=me.p,me.p=2,ae.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{ae.T=t,me.p=r}}($s&3)!==0&&Bu(),za(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===Um?Wi++:(Wi=0,Um=e):Wi=0,Eo(0,!1)}}function Jb(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,No(t)))}function Bu(e){return Vb(),Gb(),Yb(),Xb(e)}function Xb(){if(gt!==5)return!1;var e=Kn,t=Pm;Pm=0;var a=tf($s),n=ae.T,r=me.p;try{me.p=32>a?32:a,ae.T=null,a=jm,jm=null;var s=Kn,i=$s;if(gt=0,Ts=Kn=null,$s=0,(xe&6)!==0)throw Error(L(331));var o=xe;if(xe|=4,jb(s.current),Ob(s,s.current,i,a),xe=o,Eo(0,!1),Yt&&typeof Yt.onPostCommitFiberRoot=="function")try{Yt.onPostCommitFiberRoot(go,s)}catch{}return!0}finally{me.p=r,ae.T=n,Jb(e,t)}}function hg(e,t,a){t=ma(a,t),t=Am(e.stateNode,t,2),e=Bn(e,t,2),e!==null&&(bo(e,2),za(e))}function Ne(e,t,a){if(e.tag===3)hg(e,e,a);else for(;t!==null;){if(t.tag===3){hg(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(Hn===null||!Hn.has(n))){e=ma(a,e),a=pb(2),n=Bn(t,a,2),n!==null&&(hb(a,n,t,e),bo(n,2),za(n));break}}t=t.return}}function Xd(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new pC;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(Of=!0,r.add(a),e=bC.bind(null,e,t,a),t.then(e,e))}function bC(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,Ee===e&&(ue&a)===a&&(He===4||He===3&&(ue&62914560)===ue&&300>ja()-Pf?(xe&2)===0&&As(e,0):Lf|=a,Es===ue&&(Es=0)),za(e)}function Zb(e,t){t===0&&(t=Vg()),e=Ls(e,t),e!==null&&(bo(e,t),za(e))}function xC(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),Zb(e,a)}function $C(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(L(314))}n!==null&&n.delete(t),Zb(e,a)}function wC(e,t){return Wm(e,t)}var $u=null,ts=null,zm=!1,wu=!1,Zd=!1,br=0;function za(e){e!==ts&&e.next===null&&(ts===null?$u=ts=e:ts=ts.next=e),wu=!0,zm||(zm=!0,NC())}function Eo(e,t){if(!Zd&&wu){Zd=!0;do for(var a=!1,n=$u;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Jt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,vg(n,s))}else s=ue,s=Tu(n,n===Ee?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||yo(n,s)||(a=!0,vg(n,s));n=n.next}while(a);Zd=!1}}function SC(){Wb()}function Wb(){wu=zm=!1;var e=0;br!==0&&(TC()&&(e=br),br=0);for(var t=ja(),a=null,n=$u;n!==null;){var r=n.next,s=e0(n,t);s===0?(n.next=null,a===null?$u=r:a.next=r,r===null&&(ts=a)):(a=n,(e!==0||(s&3)!==0)&&(wu=!0)),n=r}Eo(e,!1)}function e0(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Jt(s),o=1<<i,u=r[i];u===-1?((o&a)===0||(o&n)!==0)&&(r[i]=Yk(o,t)):u<=t&&(e.expiredLanes|=o),s&=~o}if(t=Ee,a=ue,a=Tu(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&(be===2||be===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&_d(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||yo(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&_d(n),tf(a)){case 2:case 8:a=Kg;break;case 32:a=nu;break;case 268435456:a=Ig;break;default:a=nu}return n=t0.bind(null,e),a=Wm(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&_d(n),e.callbackPriority=2,e.callbackNode=null,2}function t0(e,t){if(gt!==0&&gt!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(Bu(!0)&&e.callbackNode!==a)return null;var n=ue;return n=Tu(e,e===Ee?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:(qb(e,n,t),e0(e,ja()),e.callbackNode!=null&&e.callbackNode===a?t0.bind(null,e):null)}function vg(e,t){if(Bu())return null;qb(e,t,!0)}function NC(){DC(function(){(xe&6)!==0?Wm(Hg,SC):Wb()})}function Uf(){return br===0&&(br=Qg()),br}function gg(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:Bl(""+e)}function yg(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function _C(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=gg((r[zt]||null).action),i=n.submitter;i&&(t=(t=i[zt]||null)?gg(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Au("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(br!==0){var u=i?yg(r,i):new FormData(r);Em(a,{pending:!0,data:u,method:r.method,action:s},null,u)}}else typeof s=="function"&&(o.preventDefault(),u=i?yg(r,i):new FormData(r),Em(a,{pending:!0,data:u,method:r.method,action:s},s,u))},currentTarget:r}]})}}for(Pl=0;Pl<gm.length;Pl++)jl=gm[Pl],bg=jl.toLowerCase(),xg=jl[0].toUpperCase()+jl.slice(1),wa(bg,"on"+xg);var jl,bg,xg,Pl;wa(vy,"onAnimationEnd");wa(gy,"onAnimationIteration");wa(yy,"onAnimationStart");wa("dblclick","onDoubleClick");wa("focusin","onFocus");wa("focusout","onBlur");wa(KR,"onTransitionRun");wa(IR,"onTransitionStart");wa(QR,"onTransitionCancel");wa(by,"onTransitionEnd");Ss("onMouseEnter",["mouseout","mouseover"]);Ss("onMouseLeave",["mouseout","mouseover"]);Ss("onPointerEnter",["pointerout","pointerover"]);Ss("onPointerLeave",["pointerout","pointerover"]);Nr("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Nr("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Nr("onBeforeInput",["compositionend","keypress","textInput","paste"]);Nr("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Nr("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Nr("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var lo="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),kC=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(lo));function a0(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],u=o.instance,c=o.currentTarget;if(o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){hu(d)}r.currentTarget=null,s=u}else for(i=0;i<n.length;i++){if(o=n[i],u=o.instance,c=o.currentTarget,o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){hu(d)}r.currentTarget=null,s=u}}}}function se(e,t){var a=t[cm];a===void 0&&(a=t[cm]=new Set);var n=e+"__bubble";a.has(n)||(n0(t,e,2,!1),a.add(n))}function Wd(e,t,a){var n=0;t&&(n|=4),n0(a,e,n,t)}var Ul="_reactListening"+Math.random().toString(36).slice(2);function Ff(e){if(!e[Ul]){e[Ul]=!0,Xg.forEach(function(a){a!=="selectionchange"&&(kC.has(a)||Wd(a,!1,e),Wd(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[Ul]||(t[Ul]=!0,Wd("selectionchange",!1,t))}}function n0(e,t,a,n){switch(h0(t)){case 2:var r=WC;break;case 8:r=e3;break;default:r=Hf}a=r.bind(null,t,a,e),r=void 0,!pm||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function em(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var u=i.tag;if((u===3||u===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=rs(o),i===null)return;if(u=i.tag,u===5||u===6||u===26||u===27){n=s=i;continue e}o=o.parentNode}}n=n.return}sy(function(){var c=s,d=rf(a),f=[];e:{var m=xy.get(e);if(m!==void 0){var p=Au,b=e;switch(e){case"keypress":if(Kl(a)===0)break e;case"keydown":case"keyup":p=wR;break;case"focusin":b="focus",p=Md;break;case"focusout":b="blur",p=Md;break;case"beforeblur":case"afterblur":p=Md;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":p=kv;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":p=cR;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":p=_R;break;case vy:case gy:case yy:p=fR;break;case by:p=RR;break;case"scroll":case"scrollend":p=lR;break;case"wheel":p=ER;break;case"copy":case"cut":case"paste":p=hR;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":p=Cv;break;case"toggle":case"beforetoggle":p=AR}var y=(t&4)!==0,$=!y&&(e==="scroll"||e==="scrollend"),g=y?m!==null?m+"Capture":null:m;y=[];for(var v=c,x;v!==null;){var w=v;if(x=w.stateNode,w=w.tag,w!==5&&w!==26&&w!==27||x===null||g===null||(w=ao(v,g),w!=null&&y.push(uo(v,w,x))),$)break;v=v.return}0<y.length&&(m=new p(m,b,null,a,d),f.push({event:m,listeners:y}))}}if((t&7)===0){e:{if(m=e==="mouseover"||e==="pointerover",p=e==="mouseout"||e==="pointerout",m&&a!==fm&&(b=a.relatedTarget||a.fromElement)&&(rs(b)||b[Ms]))break e;if((p||m)&&(m=d.window===d?d:(m=d.ownerDocument)?m.defaultView||m.parentWindow:window,p?(b=a.relatedTarget||a.toElement,p=c,b=b?rs(b):null,b!==null&&($=vo(b),y=b.tag,b!==$||y!==5&&y!==27&&y!==6)&&(b=null)):(p=null,b=c),p!==b)){if(y=kv,w="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(y=Cv,w="onPointerLeave",g="onPointerEnter",v="pointer"),$=p==null?m:Fi(p),x=b==null?m:Fi(b),m=new y(w,v+"leave",p,a,d),m.target=$,m.relatedTarget=x,w=null,rs(d)===c&&(y=new y(g,v+"enter",b,a,d),y.target=x,y.relatedTarget=$,w=y),$=w,p&&b)t:{for(y=p,g=b,v=0,x=y;x;x=Zr(x))v++;for(x=0,w=g;w;w=Zr(w))x++;for(;0<v-x;)y=Zr(y),v--;for(;0<x-v;)g=Zr(g),x--;for(;v--;){if(y===g||g!==null&&y===g.alternate)break t;y=Zr(y),g=Zr(g)}y=null}else y=null;p!==null&&$g(f,m,p,y,!1),b!==null&&$!==null&&$g(f,$,b,y,!0)}}e:{if(m=c?Fi(c):window,p=m.nodeName&&m.nodeName.toLowerCase(),p==="select"||p==="input"&&m.type==="file")var S=Dv;else if(Av(m))if(dy)S=qR;else{S=FR;var R=UR}else p=m.nodeName,!p||p.toLowerCase()!=="input"||m.type!=="checkbox"&&m.type!=="radio"?c&&nf(c.elementType)&&(S=Dv):S=zR;if(S&&(S=S(e,c))){cy(f,S,a,d);break e}R&&R(e,m,c),e==="focusout"&&c&&m.type==="number"&&c.memoizedProps.value!=null&&mm(m,"number",m.value)}switch(R=c?Fi(c):window,e){case"focusin":(Av(R)||R.contentEditable==="true")&&(os=R,hm=c,Hi=null);break;case"focusout":Hi=hm=os=null;break;case"mousedown":vm=!0;break;case"contextmenu":case"mouseup":case"dragend":vm=!1,Pv(f,a,d);break;case"selectionchange":if(HR)break;case"keydown":case"keyup":Pv(f,a,d)}var _;if(lf)e:{switch(e){case"compositionstart":var E="onCompositionStart";break e;case"compositionend":E="onCompositionEnd";break e;case"compositionupdate":E="onCompositionUpdate";break e}E=void 0}else is?ly(e,a)&&(E="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(E="onCompositionStart");E&&(oy&&a.locale!=="ko"&&(is||E!=="onCompositionStart"?E==="onCompositionEnd"&&is&&(_=iy()):(jn=d,sf="value"in jn?jn.value:jn.textContent,is=!0)),R=Su(c,E),0<R.length&&(E=new Rv(E,e,null,a,d),f.push({event:E,listeners:R}),_?E.data=_:(_=uy(a),_!==null&&(E.data=_)))),(_=MR?OR(e,a):LR(e,a))&&(E=Su(c,"onBeforeInput"),0<E.length&&(R=new Rv("onBeforeInput","beforeinput",null,a,d),f.push({event:R,listeners:E}),R.data=_)),_C(f,e,c,a,d)}a0(f,t)})}function uo(e,t,a){return{instance:e,listener:t,currentTarget:a}}function Su(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=ao(e,a),r!=null&&n.unshift(uo(e,r,s)),r=ao(e,t),r!=null&&n.push(uo(e,r,s))),e.tag===3)return n;e=e.return}return[]}function Zr(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function $g(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,u=o.alternate,c=o.stateNode;if(o=o.tag,u!==null&&u===n)break;o!==5&&o!==26&&o!==27||c===null||(u=c,r?(c=ao(a,s),c!=null&&i.unshift(uo(a,c,u))):r||(c=ao(a,s),c!=null&&i.push(uo(a,c,u)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var RC=/\r\n?/g,CC=/\u0000|\uFFFD/g;function wg(e){return(typeof e=="string"?e:""+e).replace(RC,`
`).replace(CC,"")}function r0(e,t){return t=wg(t),wg(e)===t}function Hu(){}function we(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||Ns(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&Ns(e,""+n);break;case"className":Rl(e,"class",n);break;case"tabIndex":Rl(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":Rl(e,a,n);break;case"style":ry(e,n,s);break;case"data":if(t!=="object"){Rl(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=Bl(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&we(e,t,"name",r.name,r,null),we(e,t,"formEncType",r.formEncType,r,null),we(e,t,"formMethod",r.formMethod,r,null),we(e,t,"formTarget",r.formTarget,r,null)):(we(e,t,"encType",r.encType,r,null),we(e,t,"method",r.method,r,null),we(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=Bl(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=Hu);break;case"onScroll":n!=null&&se("scroll",e);break;case"onScrollEnd":n!=null&&se("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(L(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(L(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=Bl(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":se("beforetoggle",e),se("toggle",e),ql(e,"popover",n);break;case"xlinkActuate":Za(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":Za(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":Za(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":Za(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":Za(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":Za(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":Za(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":Za(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":Za(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":ql(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=iR.get(a)||a,ql(e,a,n))}}function qm(e,t,a,n,r,s){switch(a){case"style":ry(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(L(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(L(60));e.innerHTML=a}}break;case"children":typeof n=="string"?Ns(e,n):(typeof n=="number"||typeof n=="bigint")&&Ns(e,""+n);break;case"onScroll":n!=null&&se("scroll",e);break;case"onScrollEnd":n!=null&&se("scrollend",e);break;case"onClick":n!=null&&(e.onclick=Hu);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!Zg.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[zt]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):ql(e,a,n)}}}function yt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":se("error",e),se("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(L(137,t));default:we(e,t,s,i,a,null)}}r&&we(e,t,"srcSet",a.srcSet,a,null),n&&we(e,t,"src",a.src,a,null);return;case"input":se("invalid",e);var o=s=i=r=null,u=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":u=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(L(137,t));break;default:we(e,t,n,d,a,null)}}ty(e,s,o,u,c,i,r,!1),ru(e);return;case"select":se("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:we(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?hs(e,!!n,t,!1):a!=null&&hs(e,!!n,a,!0);return;case"textarea":se("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(L(91));break;default:we(e,t,i,o,a,null)}ny(e,n,r,s),ru(e);return;case"option":for(u in a)if(a.hasOwnProperty(u)&&(n=a[u],n!=null))switch(u){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:we(e,t,u,n,a,null)}return;case"dialog":se("beforetoggle",e),se("toggle",e),se("cancel",e),se("close",e);break;case"iframe":case"object":se("load",e);break;case"video":case"audio":for(n=0;n<lo.length;n++)se(lo[n],e);break;case"image":se("error",e),se("load",e);break;case"details":se("toggle",e);break;case"embed":case"source":case"link":se("error",e),se("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(L(137,t));default:we(e,t,c,n,a,null)}return;default:if(nf(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&qm(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&we(e,t,o,n,a,null))}function EC(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,u=null,c=null,d=null;for(p in a){var f=a[p];if(a.hasOwnProperty(p)&&f!=null)switch(p){case"checked":break;case"value":break;case"defaultValue":u=f;default:n.hasOwnProperty(p)||we(e,t,p,null,n,f)}}for(var m in n){var p=n[m];if(f=a[m],n.hasOwnProperty(m)&&(p!=null||f!=null))switch(m){case"type":s=p;break;case"name":r=p;break;case"checked":c=p;break;case"defaultChecked":d=p;break;case"value":i=p;break;case"defaultValue":o=p;break;case"children":case"dangerouslySetInnerHTML":if(p!=null)throw Error(L(137,t));break;default:p!==f&&we(e,t,m,p,n,f)}}dm(e,i,o,u,c,d,s,r);return;case"select":p=i=o=m=null;for(s in a)if(u=a[s],a.hasOwnProperty(s)&&u!=null)switch(s){case"value":break;case"multiple":p=u;default:n.hasOwnProperty(s)||we(e,t,s,null,n,u)}for(r in n)if(s=n[r],u=a[r],n.hasOwnProperty(r)&&(s!=null||u!=null))switch(r){case"value":m=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==u&&we(e,t,r,s,n,u)}t=o,a=i,n=p,m!=null?hs(e,!!a,m,!1):!!n!=!!a&&(t!=null?hs(e,!!a,t,!0):hs(e,!!a,a?[]:"",!1));return;case"textarea":p=m=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:we(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":m=r;break;case"defaultValue":p=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(L(91));break;default:r!==s&&we(e,t,i,r,n,s)}ay(e,m,p);return;case"option":for(var b in a)if(m=a[b],a.hasOwnProperty(b)&&m!=null&&!n.hasOwnProperty(b))switch(b){case"selected":e.selected=!1;break;default:we(e,t,b,null,n,m)}for(u in n)if(m=n[u],p=a[u],n.hasOwnProperty(u)&&m!==p&&(m!=null||p!=null))switch(u){case"selected":e.selected=m&&typeof m!="function"&&typeof m!="symbol";break;default:we(e,t,u,m,n,p)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var y in a)m=a[y],a.hasOwnProperty(y)&&m!=null&&!n.hasOwnProperty(y)&&we(e,t,y,null,n,m);for(c in n)if(m=n[c],p=a[c],n.hasOwnProperty(c)&&m!==p&&(m!=null||p!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(m!=null)throw Error(L(137,t));break;default:we(e,t,c,m,n,p)}return;default:if(nf(t)){for(var $ in a)m=a[$],a.hasOwnProperty($)&&m!==void 0&&!n.hasOwnProperty($)&&qm(e,t,$,void 0,n,m);for(d in n)m=n[d],p=a[d],!n.hasOwnProperty(d)||m===p||m===void 0&&p===void 0||qm(e,t,d,m,n,p);return}}for(var g in a)m=a[g],a.hasOwnProperty(g)&&m!=null&&!n.hasOwnProperty(g)&&we(e,t,g,null,n,m);for(f in n)m=n[f],p=a[f],!n.hasOwnProperty(f)||m===p||m==null&&p==null||we(e,t,f,m,n,p)}var Bm=null,Hm=null;function Nu(e){return e.nodeType===9?e:e.ownerDocument}function Sg(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function s0(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function Km(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var tm=null;function TC(){var e=window.event;return e&&e.type==="popstate"?e===tm?!1:(tm=e,!0):(tm=null,!1)}var i0=typeof setTimeout=="function"?setTimeout:void 0,AC=typeof clearTimeout=="function"?clearTimeout:void 0,Ng=typeof Promise=="function"?Promise:void 0,DC=typeof queueMicrotask=="function"?queueMicrotask:typeof Ng<"u"?function(e){return Ng.resolve(null).then(e).catch(MC)}:i0;function MC(e){setTimeout(function(){throw e})}function Zn(e){return e==="head"}function _g(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&eo(i.documentElement),a&2&&eo(i.body),a&4)for(a=i.head,eo(a),i=a.firstChild;i;){var o=i.nextSibling,u=i.nodeName;i[xo]||u==="SCRIPT"||u==="STYLE"||u==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),ho(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);ho(t)}function Im(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":Im(a),af(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function OC(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[xo])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=$a(e.nextSibling),e===null)break}return null}function LC(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=$a(e.nextSibling),e===null))return null;return e}function Qm(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function PC(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function $a(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var Vm=null;function kg(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function o0(e,t,a){switch(t=Nu(a),e){case"html":if(e=t.documentElement,!e)throw Error(L(452));return e;case"head":if(e=t.head,!e)throw Error(L(453));return e;case"body":if(e=t.body,!e)throw Error(L(454));return e;default:throw Error(L(451))}}function eo(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);af(e)}var ha=new Map,Rg=new Set;function _u(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var pn=me.d;me.d={f:jC,r:UC,D:FC,C:zC,L:qC,m:BC,X:KC,S:HC,M:IC};function jC(){var e=pn.f(),t=zu();return e||t}function UC(e){var t=Os(e);t!==null&&t.tag===5&&t.type==="form"?eb(t):pn.r(e)}var js=typeof document>"u"?null:document;function l0(e,t,a){var n=js;if(n&&typeof t=="string"&&t){var r=da(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),Rg.has(r)||(Rg.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),yt(t,"link",e),dt(t),n.head.appendChild(t)))}}function FC(e){pn.D(e),l0("dns-prefetch",e,null)}function zC(e,t){pn.C(e,t),l0("preconnect",e,t)}function qC(e,t,a){pn.L(e,t,a);var n=js;if(n&&e&&t){var r='link[rel="preload"][as="'+da(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+da(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+da(a.imageSizes)+'"]')):r+='[href="'+da(e)+'"]';var s=r;switch(t){case"style":s=Ds(e);break;case"script":s=Us(e)}ha.has(s)||(e=De({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),ha.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(To(s))||t==="script"&&n.querySelector(Ao(s))||(t=n.createElement("link"),yt(t,"link",e),dt(t),n.head.appendChild(t)))}}function BC(e,t){pn.m(e,t);var a=js;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+da(n)+'"][href="'+da(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=Us(e)}if(!ha.has(s)&&(e=De({rel:"modulepreload",href:e},t),ha.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(Ao(s)))return}n=a.createElement("link"),yt(n,"link",e),dt(n),a.head.appendChild(n)}}}function HC(e,t,a){pn.S(e,t,a);var n=js;if(n&&e){var r=ps(n).hoistableStyles,s=Ds(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(To(s)))o.loading=5;else{e=De({rel:"stylesheet",href:e,"data-precedence":t},a),(a=ha.get(s))&&zf(e,a);var u=i=n.createElement("link");dt(u),yt(u,"link",e),u._p=new Promise(function(c,d){u.onload=c,u.onerror=d}),u.addEventListener("load",function(){o.loading|=1}),u.addEventListener("error",function(){o.loading|=2}),o.loading|=4,Xl(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function KC(e,t){pn.X(e,t);var a=js;if(a&&e){var n=ps(a).hoistableScripts,r=Us(e),s=n.get(r);s||(s=a.querySelector(Ao(r)),s||(e=De({src:e,async:!0},t),(t=ha.get(r))&&qf(e,t),s=a.createElement("script"),dt(s),yt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function IC(e,t){pn.M(e,t);var a=js;if(a&&e){var n=ps(a).hoistableScripts,r=Us(e),s=n.get(r);s||(s=a.querySelector(Ao(r)),s||(e=De({src:e,async:!0,type:"module"},t),(t=ha.get(r))&&qf(e,t),s=a.createElement("script"),dt(s),yt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function Cg(e,t,a,n){var r=(r=zn.current)?_u(r):null;if(!r)throw Error(L(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=Ds(a.href),a=ps(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=Ds(a.href);var s=ps(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(To(e)))&&!s._p&&(i.instance=s,i.state.loading=5),ha.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},ha.set(e,a),s||QC(r,e,a,i.state))),t&&n===null)throw Error(L(528,""));return i}if(t&&n!==null)throw Error(L(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=Us(a),a=ps(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(L(444,e))}}function Ds(e){return'href="'+da(e)+'"'}function To(e){return'link[rel="stylesheet"]['+e+"]"}function u0(e){return De({},e,{"data-precedence":e.precedence,precedence:null})}function QC(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),yt(t,"link",a),dt(t),e.head.appendChild(t))}function Us(e){return'[src="'+da(e)+'"]'}function Ao(e){return"script[async]"+e}function Eg(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+da(a.href)+'"]');if(n)return t.instance=n,dt(n),n;var r=De({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),dt(n),yt(n,"style",r),Xl(n,a.precedence,e),t.instance=n;case"stylesheet":r=Ds(a.href);var s=e.querySelector(To(r));if(s)return t.state.loading|=4,t.instance=s,dt(s),s;n=u0(a),(r=ha.get(r))&&zf(n,r),s=(e.ownerDocument||e).createElement("link"),dt(s);var i=s;return i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),yt(s,"link",n),t.state.loading|=4,Xl(s,a.precedence,e),t.instance=s;case"script":return s=Us(a.src),(r=e.querySelector(Ao(s)))?(t.instance=r,dt(r),r):(n=a,(r=ha.get(s))&&(n=De({},a),qf(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),dt(r),yt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(L(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,Xl(n,a.precedence,e));return t.instance}function Xl(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function zf(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function qf(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var Zl=null;function Tg(e,t,a){if(Zl===null){var n=new Map,r=Zl=new Map;r.set(a,n)}else r=Zl,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[xo]||s[$t]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function Ag(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function VC(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function c0(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var co=null;function GC(){}function YC(e,t,a){if(co===null)throw Error(L(475));var n=co;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=Ds(a.href),s=e.querySelector(To(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=ku.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,dt(s);return}s=e.ownerDocument||e,a=u0(a),(r=ha.get(r))&&zf(a,r),s=s.createElement("link"),dt(s);var i=s;i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),yt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=ku.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function JC(){if(co===null)throw Error(L(475));var e=co;return e.stylesheets&&e.count===0&&Gm(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&Gm(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function ku(){if(this.count--,this.count===0){if(this.stylesheets)Gm(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var Ru=null;function Gm(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,Ru=new Map,t.forEach(XC,e),Ru=null,ku.call(e))}function XC(e,t){if(!(t.state.loading&4)){var a=Ru.get(e);if(a)var n=a.get(null);else{a=new Map,Ru.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=ku.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var mo={$$typeof:an,Provider:null,Consumer:null,_currentValue:fr,_currentValue2:fr,_threadCount:0};function ZC(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=kd(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=kd(0),this.hiddenUpdates=kd(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function d0(e,t,a,n,r,s,i,o,u,c,d,f){return e=new ZC(e,t,a,i,o,u,c,f),t=1,s===!0&&(t|=24),s=Gt(3,null,null,t),e.current=s,s.stateNode=e,t=hf(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},gf(s),e}function m0(e){return e?(e=cs,e):cs}function f0(e,t,a,n,r,s){r=m0(r),n.context===null?n.context=r:n.pendingContext=r,n=qn(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=Bn(e,n,t),a!==null&&(Zt(a,e,t),Qi(a,e,t))}function Dg(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function Bf(e,t){Dg(e,t),(e=e.alternate)&&Dg(e,t)}function p0(e){if(e.tag===13){var t=Ls(e,67108864);t!==null&&Zt(t,e,67108864),Bf(e,67108864)}}var Cu=!0;function WC(e,t,a,n){var r=ae.T;ae.T=null;var s=me.p;try{me.p=2,Hf(e,t,a,n)}finally{me.p=s,ae.T=r}}function e3(e,t,a,n){var r=ae.T;ae.T=null;var s=me.p;try{me.p=8,Hf(e,t,a,n)}finally{me.p=s,ae.T=r}}function Hf(e,t,a,n){if(Cu){var r=Ym(n);if(r===null)em(e,t,n,Eu,a),Mg(e,n);else if(a3(r,e,t,a,n))n.stopPropagation();else if(Mg(e,n),t&4&&-1<t3.indexOf(e)){for(;r!==null;){var s=Os(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=cr(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var u=1<<31-Jt(i);o.entanglements[1]|=u,i&=~u}za(s),(xe&6)===0&&(bu=ja()+500,Eo(0,!1))}}break;case 13:o=Ls(s,2),o!==null&&Zt(o,s,2),zu(),Bf(s,2)}if(s=Ym(n),s===null&&em(e,t,n,Eu,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else em(e,t,n,null,a)}}function Ym(e){return e=rf(e),Kf(e)}var Eu=null;function Kf(e){if(Eu=null,e=rs(e),e!==null){var t=vo(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=Fg(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return Eu=e,null}function h0(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(Bk()){case Hg:return 2;case Kg:return 8;case nu:case Hk:return 32;case Ig:return 268435456;default:return 32}default:return 32}}var Jm=!1,In=null,Qn=null,Vn=null,fo=new Map,po=new Map,Ln=[],t3="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function Mg(e,t){switch(e){case"focusin":case"focusout":In=null;break;case"dragenter":case"dragleave":Qn=null;break;case"mouseover":case"mouseout":Vn=null;break;case"pointerover":case"pointerout":fo.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":po.delete(t.pointerId)}}function Li(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Os(t),t!==null&&p0(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function a3(e,t,a,n,r){switch(t){case"focusin":return In=Li(In,e,t,a,n,r),!0;case"dragenter":return Qn=Li(Qn,e,t,a,n,r),!0;case"mouseover":return Vn=Li(Vn,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return fo.set(s,Li(fo.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,po.set(s,Li(po.get(s)||null,e,t,a,n,r)),!0}return!1}function v0(e){var t=rs(e.target);if(t!==null){var a=vo(t);if(a!==null){if(t=a.tag,t===13){if(t=Fg(a),t!==null){e.blockedOn=t,Xk(e.priority,function(){if(a.tag===13){var n=Xt();n=ef(n);var r=Ls(a,n);r!==null&&Zt(r,a,n),Bf(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function Wl(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=Ym(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);fm=n,a.target.dispatchEvent(n),fm=null}else return t=Os(a),t!==null&&p0(t),e.blockedOn=a,!1;t.shift()}return!0}function Og(e,t,a){Wl(e)&&a.delete(t)}function n3(){Jm=!1,In!==null&&Wl(In)&&(In=null),Qn!==null&&Wl(Qn)&&(Qn=null),Vn!==null&&Wl(Vn)&&(Vn=null),fo.forEach(Og),po.forEach(Og)}function Fl(e,t){e.blockedOn===t&&(e.blockedOn=null,Jm||(Jm=!0,it.unstable_scheduleCallback(it.unstable_NormalPriority,n3)))}var zl=null;function Lg(e){zl!==e&&(zl=e,it.unstable_scheduleCallback(it.unstable_NormalPriority,function(){zl===e&&(zl=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(Kf(n||a)===null)continue;break}var s=Os(a);s!==null&&(e.splice(t,3),t-=3,Em(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function ho(e){function t(u){return Fl(u,e)}In!==null&&Fl(In,e),Qn!==null&&Fl(Qn,e),Vn!==null&&Fl(Vn,e),fo.forEach(t),po.forEach(t);for(var a=0;a<Ln.length;a++){var n=Ln[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<Ln.length&&(a=Ln[0],a.blockedOn===null);)v0(a),a.blockedOn===null&&Ln.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[zt]||null;if(typeof s=="function")i||Lg(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[zt]||null)o=i.formAction;else if(Kf(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),Lg(a)}}}function If(e){this._internalRoot=e}Ku.prototype.render=If.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(L(409));var a=t.current,n=Xt();f0(a,n,e,t,null,null)};Ku.prototype.unmount=If.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;f0(e.current,2,null,e,null,null),zu(),t[Ms]=null}};function Ku(e){this._internalRoot=e}Ku.prototype.unstable_scheduleHydration=function(e){if(e){var t=Jg();e={blockedOn:null,target:e,priority:t};for(var a=0;a<Ln.length&&t!==0&&t<Ln[a].priority;a++);Ln.splice(a,0,e),a===0&&v0(e)}};var Pg=jg.version;if(Pg!=="19.1.0")throw Error(L(527,Pg,"19.1.0"));me.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(L(188)):(e=Object.keys(e).join(","),Error(L(268,e)));return e=Lk(t),e=e!==null?zg(e):null,e=e===null?null:e.stateNode,e};var r3={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:ae,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(Pi=__REACT_DEVTOOLS_GLOBAL_HOOK__,!Pi.isDisabled&&Pi.supportsFiber))try{go=Pi.inject(r3),Yt=Pi}catch{}var Pi;Iu.createRoot=function(e,t){if(!Ug(e))throw Error(L(299));var a=!1,n="",r=db,s=mb,i=fb,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=d0(e,1,!1,null,null,a,n,r,s,i,o,null),e[Ms]=t.current,Ff(e),new If(t)};Iu.hydrateRoot=function(e,t,a){if(!Ug(e))throw Error(L(299));var n=!1,r="",s=db,i=mb,o=fb,u=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(u=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=d0(e,1,!0,t,a??null,n,r,s,i,o,u,c),t.context=m0(null),a=t.current,n=Xt(),n=ef(n),r=qn(n),r.callback=null,Bn(a,r,n),a=n,t.current.lanes=a,bo(t,a),za(t),e[Ms]=t.current,Ff(e),new Ku(t)};Iu.version="19.1.0"});var x0=Sn((iL,b0)=>{"use strict";function y0(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(y0)}catch(e){console.error(e)}}y0(),b0.exports=g0()});var Ot=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var pk={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},hk=class{#t=pk;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},Ea=new hk;function Eh(e){setTimeout(e,0)}var Lt=typeof window>"u"||"Deno"in globalThis;function Me(){}function Dh(e,t){return typeof e=="function"?e(t):e}function gi(e){return typeof e=="number"&&e>=0&&e!==1/0}function nl(e,t){return Math.max(e+(t||0)-Date.now(),0)}function ba(e,t){return typeof e=="function"?e(t):e}function Pt(e,t){return typeof e=="function"?e(t):e}function rl(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==yi(i,t.options))return!1}else if(!or(t.queryKey,i))return!1}if(a!=="all"){let u=t.isActive();if(a==="active"&&!u||a==="inactive"&&u)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function sl(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(Ta(t.options.mutationKey)!==Ta(s))return!1}else if(!or(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function yi(e,t){return(t?.queryKeyHashFn||Ta)(e)}function Ta(e){return JSON.stringify(e,(t,a)=>nd(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function or(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>or(e[a],t[a])):!1}var vk=Object.prototype.hasOwnProperty;function bi(e,t){if(e===t)return e;let a=Th(e)&&Th(t);if(!a&&!(nd(e)&&nd(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},u=0;for(let c=0;c<i;c++){let d=a?c:s[c],f=e[d],m=t[d];if(f===m){o[d]=f,(a?c<r:vk.call(e,d))&&u++;continue}if(f===null||m===null||typeof f!="object"||typeof m!="object"){o[d]=m;continue}let p=bi(f,m);o[d]=p,p===f&&u++}return r===i&&u===r?e:o}function Nn(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function Th(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function nd(e){if(!Ah(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!Ah(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function Ah(e){return Object.prototype.toString.call(e)==="[object Object]"}function Mh(e){return new Promise(t=>{Ea.setTimeout(t,e)})}function xi(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?bi(e,t):t}function Oh(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function Lh(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var Hr=Symbol();function il(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===Hr?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function $i(e,t){return typeof e=="function"?e(...t):!!e}var gk=class extends Ot{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!Lt&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},Kr=new gk;function wi(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var Ph=Eh;function yk(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=Ph,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(u=>{a(u)})})})};return{batch:o=>{let u;t++;try{u=o()}finally{t--,t||i()}return u},batchCalls:o=>(...u)=>{s(()=>{o(...u)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var le=yk();var bk=class extends Ot{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!Lt&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},Ir=new bk;function xk(e){return Math.min(1e3*2**e,3e4)}function rd(e){return(e??"online")==="online"?Ir.isOnline():!0}var ol=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function ll(e){let t=!1,a=0,n,r=wi(),s=()=>r.status!=="pending",i=y=>{if(!s()){let $=new ol(y);m($),e.onCancel?.($)}},o=()=>{t=!0},u=()=>{t=!1},c=()=>Kr.isFocused()&&(e.networkMode==="always"||Ir.isOnline())&&e.canRun(),d=()=>rd(e.networkMode)&&e.canRun(),f=y=>{s()||(n?.(),r.resolve(y))},m=y=>{s()||(n?.(),r.reject(y))},p=()=>new Promise(y=>{n=$=>{(s()||c())&&y($)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),b=()=>{if(s())return;let y,$=a===0?e.initialPromise:void 0;try{y=$??e.fn()}catch(g){y=Promise.reject(g)}Promise.resolve(y).then(f).catch(g=>{if(s())return;let v=e.retry??(Lt?0:3),x=e.retryDelay??xk,w=typeof x=="function"?x(a,g):x,S=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!S){m(g);return}a++,e.onFail?.(a,g),Mh(w).then(()=>c()?void 0:p()).then(()=>{t?m(g):b()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:u,canStart:d,start:()=>(d()?b():p().then(b),r)}}var ul=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),gi(this.gcTime)&&(this.#t=Ea.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(Lt?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(Ea.clearTimeout(this.#t),this.#t=void 0)}};var Uh=class extends ul{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=jh(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=jh(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=xi(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(Me).catch(Me):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>Pt(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===Hr||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>ba(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!nl(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(u=>u.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=il(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=ll({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof ol&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,u)=>{this.#i({type:"failed",failureCount:o,error:u})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof ol){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,...sd(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),le.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function sd(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:rd(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function jh(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var lr=class extends Ot{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=wi(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),Fh(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return id(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return id(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof Pt(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!Nn(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&zh(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||Pt(this.options.enabled,this.#e)!==Pt(t.enabled,this.#e)||ba(this.options.staleTime,this.#e)!==ba(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||Pt(this.options.enabled,this.#e)!==Pt(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return wk(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(Me)),t}#v(){this.#x();let e=ba(this.options.staleTime,this.#e);if(Lt||this.#n.isStale||!gi(e))return;let a=nl(this.#n.dataUpdatedAt,e)+1;this.#u=Ea.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(Lt||Pt(this.options.enabled,this.#e)===!1||!gi(this.#l)||this.#l===0)&&(this.#c=Ea.setInterval(()=>{(this.options.refetchIntervalInBackground||Kr.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(Ea.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(Ea.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,u=e!==a?e.state:this.#a,{state:c}=e,d={...c},f=!1,m;if(t._optimisticResults){let E=this.hasListeners(),M=!E&&Fh(e,t),P=E&&zh(e,a,t,n);(M||P)&&(d={...d,...sd(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:p,errorUpdatedAt:b,status:y}=d;m=d.data;let $=!1;if(t.placeholderData!==void 0&&m===void 0&&y==="pending"){let E;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(E=r.data,$=!0):E=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,E!==void 0&&(y="success",m=xi(r?.data,E,t),f=!0)}if(t.select&&m!==void 0&&!$)if(r&&m===s?.data&&t.select===this.#f)m=this.#d;else try{this.#f=t.select,m=t.select(m),m=xi(r?.data,m,t),this.#d=m,this.#i=null}catch(E){this.#i=E}this.#i&&(p=this.#i,m=this.#d,b=Date.now(),y="error");let g=d.fetchStatus==="fetching",v=y==="pending",x=y==="error",w=v&&g,S=m!==void 0,_={status:y,fetchStatus:d.fetchStatus,isPending:v,isSuccess:y==="success",isError:x,isInitialLoading:w,isLoading:w,data:m,dataUpdatedAt:d.dataUpdatedAt,error:p,errorUpdatedAt:b,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>u.dataUpdateCount||d.errorUpdateCount>u.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:x&&!S,isPaused:d.fetchStatus==="paused",isPlaceholderData:f,isRefetchError:x&&S,isStale:od(e,t),refetch:this.refetch,promise:this.#o,isEnabled:Pt(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let E=I=>{_.status==="error"?I.reject(_.error):_.data!==void 0&&I.resolve(_.data)},M=()=>{let I=this.#o=_.promise=wi();E(I)},P=this.#o;switch(P.status){case"pending":e.queryHash===a.queryHash&&E(P);break;case"fulfilled":(_.status==="error"||_.data!==P.value)&&M();break;case"rejected":(_.status!=="error"||_.error!==P.reason)&&M();break}}return _}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),Nn(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){le.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function $k(e,t){return Pt(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function Fh(e,t){return $k(e,t)||e.state.data!==void 0&&id(e,t,t.refetchOnMount)}function id(e,t,a){if(Pt(t.enabled,e)!==!1&&ba(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&od(e,t)}return!1}function zh(e,t,a,n){return(e!==t||Pt(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&od(e,a)}function od(e,t){return Pt(t.enabled,e)!==!1&&e.isStaleByTime(ba(t.staleTime,e))}function wk(e,t){return!Nn(e.getCurrentResult(),t)}function ld(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},u=0,c=async()=>{let d=!1,f=b=>{Object.defineProperty(b,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},m=il(t.options,t.fetchOptions),p=async(b,y,$)=>{if(d)return Promise.reject();if(y==null&&b.pages.length)return Promise.resolve(b);let v=(()=>{let R={client:t.client,queryKey:t.queryKey,pageParam:y,direction:$?"backward":"forward",meta:t.options.meta};return f(R),R})(),x=await m(v),{maxPages:w}=t.options,S=$?Lh:Oh;return{pages:S(b.pages,x,w),pageParams:S(b.pageParams,y,w)}};if(r&&s.length){let b=r==="backward",y=b?Sk:qh,$={pages:s,pageParams:i},g=y(n,$);o=await p($,g,b)}else{let b=e??s.length;do{let y=u===0?i[0]??n.initialPageParam:qh(n,o);if(u>0&&y==null)break;o=await p(o,y),u++}while(u<b)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function qh(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function Sk(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var Bh=class extends ul{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||ud(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=ll({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),le.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function ud(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var Hh=class extends Ot{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new Bh({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=cl(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=cl(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=cl(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=cl(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){le.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>sl(t,a))}findAll(e={}){return this.getAll().filter(t=>sl(e,t))}notify(e){le.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return le.batch(()=>Promise.all(e.map(t=>t.continue().catch(Me))))}};function cl(e){return e.options.scope?.id}var cd=class extends Ot{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),Nn(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&Ta(t.mutationKey)!==Ta(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??ud();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){le.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function Kh(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function Nk(e,t,a){let n=e.slice(0);return n[t]=a,n}var dd=class extends Ot{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,le.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,f)=>d!==a[f]),u=i||o,c=u?!0:s.some((d,f)=>{let m=this.#e[f];return!m||!Nn(d,m)});!u&&!c||(u&&(this.#r=r),this.#e=s,this.hasListeners()&&(u&&(Kh(a,r).forEach(d=>{d.destroy()}),Kh(r,a).forEach(d=>{d.subscribe(f=>{this.#c(d,f)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=bi(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new lr(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=Nk(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&le.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var Ih=class extends Ot{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??yi(n,t),s=this.get(r);return s||(s=new Uh({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){le.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>rl(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>rl(e,a)):t}notify(e){le.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){le.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){le.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var md=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new Ih,this.#e=e.mutationCache||new Hh,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=Kr.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=Ir.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime(ba(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=Dh(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return le.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;le.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return le.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=le.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(Me).catch(Me)}invalidateQueries(e,t={}){return le.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=le.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(Me)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(Me)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime(ba(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(Me).catch(Me)}fetchInfiniteQuery(e){return e.behavior=ld(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(Me).catch(Me)}ensureInfiniteQueryData(e){return e.behavior=ld(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return Ir.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(Ta(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{or(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(Ta(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{or(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=yi(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===Hr&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var Aa=qe(Ie(),1);var Qr=qe(Ie(),1),Yh=qe(fd(),1),pd=Qr.createContext(void 0),X=e=>{let t=Qr.useContext(pd);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},hd=({client:e,children:t})=>(Qr.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,Yh.jsx)(pd.Provider,{value:e,children:t}));var ml=qe(Ie(),1),Jh=ml.createContext(!1),fl=()=>ml.useContext(Jh),wO=Jh.Provider;var Si=qe(Ie(),1),Rk=qe(fd(),1);function Ck(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var Ek=Si.createContext(Ck()),pl=()=>Si.useContext(Ek);var Xh=qe(Ie(),1);var hl=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},vl=e=>{Xh.useEffect(()=>{e.clearReset()},[e])},gl=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||$i(a,[e.error,n]));var yl=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},bl=(e,t)=>e.isLoading&&e.isFetching&&!t,Ni=(e,t)=>e?.suspense&&t.isPending,Vr=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function vd({queries:e,...t},a){let n=X(a),r=fl(),s=pl(),i=Aa.useMemo(()=>e.map(y=>{let $=n.defaultQueryOptions(y);return $._optimisticResults=r?"isRestoring":"optimistic",$}),[e,n,r]);i.forEach(y=>{yl(y),hl(y,s)}),vl(s);let[o]=Aa.useState(()=>new dd(n,i,t)),[u,c,d]=o.getOptimisticResult(i,t.combine),f=!r&&t.subscribed!==!1;Aa.useSyncExternalStore(Aa.useCallback(y=>f?o.subscribe(le.batchCalls(y)):Me,[o,f]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),Aa.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let p=u.some((y,$)=>Ni(i[$],y))?u.flatMap((y,$)=>{let g=i[$];if(g){let v=new lr(n,g);if(Ni(g,y))return Vr(g,v,s);bl(y,r)&&Vr(g,v,s)}return[]}):[];if(p.length>0)throw Promise.all(p);let b=u.find((y,$)=>{let g=i[$];return g&&gl({result:y,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(b?.error)throw b.error;return c(d())}var _n=qe(Ie(),1);function Zh(e,t,a){let n=fl(),r=pl(),s=X(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",yl(i),hl(i,r),vl(r);let o=!s.getQueryCache().get(i.queryHash),[u]=_n.useState(()=>new t(s,i)),c=u.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(_n.useSyncExternalStore(_n.useCallback(f=>{let m=d?u.subscribe(le.batchCalls(f)):Me;return u.updateResult(),m},[u,d]),()=>u.getCurrentResult(),()=>u.getCurrentResult()),_n.useEffect(()=>{u.setOptions(i)},[i,u]),Ni(i,c))throw Vr(i,u,r);if(gl({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!Lt&&bl(c,n)&&(o?Vr(i,u,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(Me).finally(()=>{u.updateResult()}),i.notifyOnChangeProps?c:u.trackResult(c)}function z(e,t){return Zh(e,lr,t)}var Ja=qe(Ie(),1);function Q(e,t){let a=X(t),[n]=Ja.useState(()=>new cd(a,e));Ja.useEffect(()=>{n.setOptions(e)},[n,e]);let r=Ja.useSyncExternalStore(Ja.useCallback(i=>n.subscribe(le.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=Ja.useCallback((i,o)=>{n.mutate(i,o).catch(Me)},[n]);if(r.error&&$i(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var ck=qe(x0());var ta=qe(Ie(),1),G=qe(Ie(),1),Ae=qe(Ie(),1),cp=qe(Ie(),1),H0=qe(Ie(),1),fe=qe(Ie(),1),sE=qe(Ie(),1),iE=qe(Ie(),1),oE=qe(Ie(),1),W=qe(Ie(),1),rx=qe(Ie(),1);var $0="popstate";function k0(e={}){function t(n,r){let{pathname:s,search:i,hash:o}=n.location;return Gf("",{pathname:s,search:i,hash:o},r.state&&r.state.usr||null,r.state&&r.state.key||"default")}function a(n,r){return typeof r=="string"?r:Fs(r)}return i3(t,a,null,e)}function Te(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function ea(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function s3(){return Math.random().toString(36).substring(2,10)}function w0(e,t){return{usr:e.state,key:e.key,idx:t}}function Gf(e,t,a=null,n){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?Cr(t):t,state:a,key:t&&t.key||n||s3()}}function Fs({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function Cr(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function i3(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",u=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function f(){o="POP";let $=d(),g=$==null?null:$-c;c=$,u&&u({action:o,location:y.location,delta:g})}function m($,g){o="PUSH";let v=Gf(y.location,$,g);a&&a(v,$),c=d()+1;let x=w0(v,c),w=y.createHref(v);try{i.pushState(x,"",w)}catch(S){if(S instanceof DOMException&&S.name==="DataCloneError")throw S;r.location.assign(w)}s&&u&&u({action:o,location:y.location,delta:1})}function p($,g){o="REPLACE";let v=Gf(y.location,$,g);a&&a(v,$),c=d();let x=w0(v,c),w=y.createHref(v);i.replaceState(x,"",w),s&&u&&u({action:o,location:y.location,delta:0})}function b($){return o3($)}let y={get action(){return o},get location(){return e(r,i)},listen($){if(u)throw new Error("A history only accepts one active listener");return r.addEventListener($0,f),u=$,()=>{r.removeEventListener($0,f),u=null}},createHref($){return t(r,$)},createURL:b,encodeLocation($){let g=b($);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:m,replace:p,go($){return i.go($)}};return y}function o3(e,t=!1){let a="http://localhost";typeof window<"u"&&(a=window.location.origin!=="null"?window.location.origin:window.location.href),Te(a,"No window.location.(origin|href) available to create URL");let n=typeof e=="string"?e:Fs(e);return n=n.replace(/ $/,"%20"),!t&&n.startsWith("//")&&(n=a+n),new URL(n,a)}var l3;l3=new WeakMap;function Zf(e,t,a="/"){return u3(e,t,a,!1)}function u3(e,t,a,n){let r=typeof t=="string"?Cr(t):t,s=qa(r.pathname||"/",a);if(s==null)return null;let i=R0(e);d3(i);let o=null;for(let u=0;o==null&&u<i.length;++u){let c=w3(s);o=x3(i[u],c,n)}return o}function c3(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function R0(e,t=[],a=[],n="",r=!1){let s=(i,o,u=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&u)return;Te(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let f=hn([n,d.relativePath]),m=a.concat(d);i.children&&i.children.length>0&&(Te(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${f}".`),R0(i.children,t,m,f,u)),!(i.path==null&&!i.index)&&t.push({path:f,score:y3(f,i.index),routesMeta:m})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let u of C0(i.path))s(i,o,!0,u)}),t}function C0(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=C0(n.join("/")),o=[];return o.push(...i.map(u=>u===""?s:[s,u].join("/"))),r&&o.push(...i),o.map(u=>e.startsWith("/")&&u===""?"/":u)}function d3(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:b3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var m3=/^:[\w-]+$/,f3=3,p3=2,h3=1,v3=10,g3=-2,S0=e=>e==="*";function y3(e,t){let a=e.split("/"),n=a.length;return a.some(S0)&&(n+=g3),t&&(n+=p3),a.filter(r=>!S0(r)).reduce((r,s)=>r+(m3.test(s)?f3:s===""?h3:v3),n)}function b3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function x3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let u=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",f=Mo({path:u.relativePath,caseSensitive:u.caseSensitive,end:c},d),m=u.route;if(!f&&c&&a&&!n[n.length-1].route.index&&(f=Mo({path:u.relativePath,caseSensitive:u.caseSensitive,end:!1},d)),!f)return null;Object.assign(r,f.params),i.push({params:r,pathname:hn([s,f.pathname]),pathnameBase:_3(hn([s,f.pathnameBase])),route:m}),f.pathnameBase!=="/"&&(s=hn([s,f.pathnameBase]))}return i}function Mo(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=$3(e.path,e.caseSensitive,e.end),r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:f},m)=>{if(d==="*"){let b=o[m]||"";i=s.slice(0,s.length-b.length).replace(/(.)\/+$/,"$1")}let p=o[m];return f&&!p?c[d]=void 0:c[d]=(p||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function $3(e,t=!1,a=!0){ea(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,u)=>(n.push({paramName:o,isOptional:u!=null}),u?"/?([^\\/]+)?":"/([^\\/]+)")).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function w3(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return ea(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function qa(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}function E0(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?Cr(e):e;return{pathname:a?a.startsWith("/")?a:S3(a,t):t,search:k3(n),hash:R3(r)}}function S3(e,t){let a=t.replace(/\/+$/,"").split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function Qf(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function N3(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function Wf(e){let t=N3(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function ep(e,t,a,n=!1){let r;typeof e=="string"?r=Cr(e):(r={...e},Te(!r.pathname||!r.pathname.includes("?"),Qf("?","pathname","search",r)),Te(!r.pathname||!r.pathname.includes("#"),Qf("#","pathname","hash",r)),Te(!r.search||!r.search.includes("#"),Qf("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let f=t.length-1;if(!n&&i.startsWith("..")){let m=i.split("/");for(;m[0]==="..";)m.shift(),f-=1;r.pathname=m.join("/")}o=f>=0?t[f]:"/"}let u=E0(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!u.pathname.endsWith("/")&&(c||d)&&(u.pathname+="/"),u}var hn=e=>e.join("/").replace(/\/\/+/g,"/"),_3=e=>e.replace(/\/+$/,"").replace(/^\/*/,"/"),k3=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,R3=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;function T0(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}var A0=["POST","PUT","PATCH","DELETE"],oL=new Set(A0),C3=["GET",...A0],lL=new Set(C3);var uL=Symbol("ResetLoaderData");var Er=ta.createContext(null);Er.displayName="DataRouter";var zs=ta.createContext(null);zs.displayName="DataRouterState";var cL=ta.createContext(!1);var tp=ta.createContext({isTransitioning:!1});tp.displayName="ViewTransition";var D0=ta.createContext(new Map);D0.displayName="Fetchers";var E3=ta.createContext(null);E3.displayName="Await";var Bt=ta.createContext(null);Bt.displayName="Navigation";var qs=ta.createContext(null);qs.displayName="Location";var aa=ta.createContext({outlet:null,matches:[],isDataRoute:!1});aa.displayName="Route";var ap=ta.createContext(null);ap.displayName="RouteError";var Yf=!0;function M0(e,{relative:t}={}){Te(Tr(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=G.useContext(Bt),{hash:r,pathname:s,search:i}=Bs(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:hn([a,s])),n.createHref({pathname:o,search:i,hash:r})}function Tr(){return G.useContext(qs)!=null}function je(){return Te(Tr(),"useLocation() may be used only in the context of a <Router> component."),G.useContext(qs).location}var O0="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function L0(e){G.useContext(Bt).static||G.useLayoutEffect(e)}function ce(){let{isDataRoute:e}=G.useContext(aa);return e?F3():T3()}function T3(){Te(Tr(),"useNavigate() may be used only in the context of a <Router> component.");let e=G.useContext(Er),{basename:t,navigator:a}=G.useContext(Bt),{matches:n}=G.useContext(aa),{pathname:r}=je(),s=JSON.stringify(Wf(n)),i=G.useRef(!1);return L0(()=>{i.current=!0}),G.useCallback((u,c={})=>{if(ea(i.current,O0),!i.current)return;if(typeof u=="number"){a.go(u);return}let d=ep(u,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:hn([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var P0=G.createContext(null);function Ba(){return G.useContext(P0)}function j0(e){let t=G.useContext(aa).outlet;return t&&G.createElement(P0.Provider,{value:e},t)}function ot(){let{matches:e}=G.useContext(aa),t=e[e.length-1];return t?t.params:{}}function Bs(e,{relative:t}={}){let{matches:a}=G.useContext(aa),{pathname:n}=je(),r=JSON.stringify(Wf(a));return G.useMemo(()=>ep(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function U0(e,t){return F0(e,t)}function F0(e,t,a,n,r){Te(Tr(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:s}=G.useContext(Bt),{matches:i}=G.useContext(aa),o=i[i.length-1],u=o?o.params:{},c=o?o.pathname:"/",d=o?o.pathnameBase:"/",f=o&&o.route;if(Yf){let v=f&&f.path||"";B0(c,!f||v.endsWith("*")||v.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${c}" (under <Route path="${v}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${v}"> to <Route path="${v==="/"?"*":`${v}/*`}">.`)}let m=je(),p;if(t){let v=typeof t=="string"?Cr(t):t;Te(d==="/"||v.pathname?.startsWith(d),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${d}" but pathname "${v.pathname}" was given in the \`location\` prop.`),p=v}else p=m;let b=p.pathname||"/",y=b;if(d!=="/"){let v=d.replace(/^\//,"").split("/");y="/"+b.replace(/^\//,"").split("/").slice(v.length).join("/")}let $=Zf(e,{pathname:y});Yf&&(ea(f||$!=null,`No routes matched location "${p.pathname}${p.search}${p.hash}" `),ea($==null||$[$.length-1].route.element!==void 0||$[$.length-1].route.Component!==void 0||$[$.length-1].route.lazy!==void 0,`Matched leaf route at location "${p.pathname}${p.search}${p.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let g=L3($&&$.map(v=>Object.assign({},v,{params:Object.assign({},u,v.params),pathname:hn([d,s.encodeLocation?s.encodeLocation(v.pathname).pathname:v.pathname]),pathnameBase:v.pathnameBase==="/"?d:hn([d,s.encodeLocation?s.encodeLocation(v.pathnameBase).pathname:v.pathnameBase])})),i,a,n,r);return t&&g?G.createElement(qs.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",...p},navigationType:"POP"}},g):g}function A3(){let e=q0(),t=T0(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return Yf&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=G.createElement(G.Fragment,null,G.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),G.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",G.createElement("code",{style:s},"ErrorBoundary")," or"," ",G.createElement("code",{style:s},"errorElement")," prop on your route."))),G.createElement(G.Fragment,null,G.createElement("h2",null,"Unexpected Application Error!"),G.createElement("h3",{style:{fontStyle:"italic"}},t),a?G.createElement("pre",{style:r},a):null,i)}var D3=G.createElement(A3,null),M3=class extends G.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.unstable_onError?this.props.unstable_onError(e,t):console.error("React Router caught the following error during render",e)}render(){return this.state.error!==void 0?G.createElement(aa.Provider,{value:this.props.routeContext},G.createElement(ap.Provider,{value:this.state.error,children:this.props.component})):this.props.children}};function O3({routeContext:e,match:t,children:a}){let n=G.useContext(Er);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),G.createElement(aa.Provider,{value:e},a)}function L3(e,t=[],a=null,n=null,r=null){if(e==null){if(!a)return null;if(a.errors)e=a.matches;else if(t.length===0&&!a.initialized&&a.matches.length>0)e=a.matches;else return null}let s=e,i=a?.errors;if(i!=null){let c=s.findIndex(d=>d.route.id&&i?.[d.route.id]!==void 0);Te(c>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(i).join(",")}`),s=s.slice(0,Math.min(s.length,c+1))}let o=!1,u=-1;if(a)for(let c=0;c<s.length;c++){let d=s[c];if((d.route.HydrateFallback||d.route.hydrateFallbackElement)&&(u=c),d.route.id){let{loaderData:f,errors:m}=a,p=d.route.loader&&!f.hasOwnProperty(d.route.id)&&(!m||m[d.route.id]===void 0);if(d.route.lazy||p){o=!0,u>=0?s=s.slice(0,u+1):s=[s[0]];break}}}return s.reduceRight((c,d,f)=>{let m,p=!1,b=null,y=null;a&&(m=i&&d.route.id?i[d.route.id]:void 0,b=d.route.errorElement||D3,o&&(u<0&&f===0?(B0("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),p=!0,y=null):u===f&&(p=!0,y=d.route.hydrateFallbackElement||null)));let $=t.concat(s.slice(0,f+1)),g=()=>{let v;return m?v=b:p?v=y:d.route.Component?v=G.createElement(d.route.Component,null):d.route.element?v=d.route.element:v=c,G.createElement(O3,{match:d,routeContext:{outlet:c,matches:$,isDataRoute:a!=null},children:v})};return a&&(d.route.ErrorBoundary||d.route.errorElement||f===0)?G.createElement(M3,{location:a.location,revalidation:a.revalidation,component:b,error:m,children:g(),routeContext:{outlet:null,matches:$,isDataRoute:!0},unstable_onError:n}):g()},null)}function np(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function P3(e){let t=G.useContext(Er);return Te(t,np(e)),t}function rp(e){let t=G.useContext(zs);return Te(t,np(e)),t}function j3(e){let t=G.useContext(aa);return Te(t,np(e)),t}function sp(e){let t=j3(e),a=t.matches[t.matches.length-1];return Te(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function U3(){return sp("useRouteId")}function z0(){return rp("useNavigation").navigation}function ip(){let{matches:e,loaderData:t}=rp("useMatches");return G.useMemo(()=>e.map(a=>c3(a,t)),[e,t])}function q0(){let e=G.useContext(ap),t=rp("useRouteError"),a=sp("useRouteError");return e!==void 0?e:t.errors?.[a]}function F3(){let{router:e}=P3("useNavigate"),t=sp("useNavigate"),a=G.useRef(!1);return L0(()=>{a.current=!0}),G.useCallback(async(r,s={})=>{ea(a.current,O0),a.current&&(typeof r=="number"?e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var N0={};function B0(e,t,a){!t&&!N0[e]&&(N0[e]=!0,ea(!1,a))}var dL=Ae.memo(z3);function z3({routes:e,future:t,state:a,unstable_onError:n}){return F0(e,void 0,a,n,t)}function lt({to:e,replace:t,state:a,relative:n}){Te(Tr(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=Ae.useContext(Bt);ea(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=Ae.useContext(aa),{pathname:i}=je(),o=ce(),u=ep(e,Wf(s),i,n==="path"),c=JSON.stringify(u);return Ae.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function op(e){return j0(e.context)}function pe(e){Te(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function lp({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1}){Te(!Tr(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let i=e.replace(/^\/*/,"/"),o=Ae.useMemo(()=>({basename:i,navigator:r,static:s,future:{}}),[i,r,s]);typeof a=="string"&&(a=Cr(a));let{pathname:u="/",search:c="",hash:d="",state:f=null,key:m="default"}=a,p=Ae.useMemo(()=>{let b=qa(u,i);return b==null?null:{location:{pathname:b,search:c,hash:d,state:f,key:m},navigationType:n}},[i,u,c,d,f,m,n]);return ea(p!=null,`<Router basename="${i}"> is not able to match the URL "${u}${c}${d}" because it does not start with the basename, so the <Router> won't render anything.`),p==null?null:Ae.createElement(Bt.Provider,{value:o},Ae.createElement(qs.Provider,{children:t,value:p}))}function up({children:e,location:t}){return U0(Ju(e),t)}function Ju(e,t=[]){let a=[];return Ae.Children.forEach(e,(n,r)=>{if(!Ae.isValidElement(n))return;let s=[...t,r];if(n.type===Ae.Fragment){a.push.apply(a,Ju(n.props.children,s));return}Te(n.type===pe,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),Te(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=Ju(n.props.children,s)),a.push(i)}),a}var Gu="get",Yu="application/x-www-form-urlencoded";function Xu(e){return e!=null&&typeof e.tagName=="string"}function q3(e){return Xu(e)&&e.tagName.toLowerCase()==="button"}function B3(e){return Xu(e)&&e.tagName.toLowerCase()==="form"}function H3(e){return Xu(e)&&e.tagName.toLowerCase()==="input"}function K3(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function I3(e,t){return e.button===0&&(!t||t==="_self")&&!K3(e)}var Qu=null;function Q3(){if(Qu===null)try{new FormData(document.createElement("form"),0),Qu=!1}catch{Qu=!0}return Qu}var V3=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function Vf(e){return e!=null&&!V3.has(e)?(ea(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${Yu}"`),null):e}function G3(e,t){let a,n,r,s,i;if(B3(e)){let o=e.getAttribute("action");n=o?qa(o,t):null,a=e.getAttribute("method")||Gu,r=Vf(e.getAttribute("enctype"))||Yu,s=new FormData(e)}else if(q3(e)||H3(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let u=e.getAttribute("formaction")||o.getAttribute("action");if(n=u?qa(u,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||Gu,r=Vf(e.getAttribute("formenctype"))||Vf(o.getAttribute("enctype"))||Yu,s=new FormData(o,e),!Q3()){let{name:c,type:d,value:f}=e;if(d==="image"){let m=c?`${c}.`:"";s.append(`${m}x`,"0"),s.append(`${m}y`,"0")}else c&&s.append(c,f)}}else{if(Xu(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=Gu,n=null,r=Yu,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var mL=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");function dp(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var Y3=Symbol("SingleFetchRedirect");function J3(e,t,a){let n=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return n.pathname==="/"?n.pathname=`_root.${a}`:t&&qa(n.pathname,t)==="/"?n.pathname=`${t.replace(/\/$/,"")}/_root.${a}`:n.pathname=`${n.pathname.replace(/\/$/,"")}.${a}`,n}async function X3(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function Z3(e){return e!=null&&typeof e.page=="string"}function W3(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function eE(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await X3(s,a);return i.links?i.links():[]}return[]}));return rE(n.flat(1).filter(W3).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function _0(e,t,a,n,r,s){let i=(u,c)=>a[c]?u.route.id!==a[c].route.id:!0,o=(u,c)=>a[c].pathname!==u.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==u.params["*"];return s==="assets"?t.filter((u,c)=>i(u,c)||o(u,c)):s==="data"?t.filter((u,c)=>{let d=n.routes[u.route.id];if(!d||!d.hasLoader)return!1;if(i(u,c)||o(u,c))return!0;if(u.route.shouldRevalidate){let f=u.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:u.params,defaultShouldRevalidate:!0});if(typeof f=="boolean")return f}return!0}):[]}function tE(e,t,{includeHydrateFallback:a}={}){return aE(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function aE(e){return[...new Set(e)]}function nE(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function rE(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!Z3(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(nE(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function K0(){let e=fe.useContext(Er);return dp(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function lE(){let e=fe.useContext(zs);return dp(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var Oo=fe.createContext(void 0);Oo.displayName="FrameworkContext";function I0(){let e=fe.useContext(Oo);return dp(e,"You must render this element inside a <HydratedRouter> element"),e}function uE(e,t){let a=fe.useContext(Oo),[n,r]=fe.useState(!1),[s,i]=fe.useState(!1),{onFocus:o,onBlur:u,onMouseEnter:c,onMouseLeave:d,onTouchStart:f}=t,m=fe.useRef(null);fe.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let y=g=>{g.forEach(v=>{i(v.isIntersecting)})},$=new IntersectionObserver(y,{threshold:.5});return m.current&&$.observe(m.current),()=>{$.disconnect()}}},[e]),fe.useEffect(()=>{if(n){let y=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(y)}}},[n]);let p=()=>{r(!0)},b=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,m,{}]:[s,m,{onFocus:Do(o,p),onBlur:Do(u,b),onMouseEnter:Do(c,p),onMouseLeave:Do(d,b),onTouchStart:Do(f,p)}]:[!1,m,{}]}function Do(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function Q0({page:e,...t}){let{router:a}=K0(),n=fe.useMemo(()=>Zf(a.routes,e,a.basename),[a.routes,e,a.basename]);return n?fe.createElement(dE,{page:e,matches:n,...t}):null}function cE(e){let{manifest:t,routeModules:a}=I0(),[n,r]=fe.useState([]);return fe.useEffect(()=>{let s=!1;return eE(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function dE({page:e,matches:t,...a}){let n=je(),{manifest:r,routeModules:s}=I0(),{basename:i}=K0(),{loaderData:o,matches:u}=lE(),c=fe.useMemo(()=>_0(e,t,u,r,n,"data"),[e,t,u,r,n]),d=fe.useMemo(()=>_0(e,t,u,r,n,"assets"),[e,t,u,r,n]),f=fe.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let b=new Set,y=!1;if(t.forEach(g=>{let v=r.routes[g.route.id];!v||!v.hasLoader||(!c.some(x=>x.route.id===g.route.id)&&g.route.id in o&&s[g.route.id]?.shouldRevalidate||v.hasClientLoader?y=!0:b.add(g.route.id))}),b.size===0)return[];let $=J3(e,i,"data");return y&&b.size>0&&$.searchParams.set("_routes",t.filter(g=>b.has(g.route.id)).map(g=>g.route.id).join(",")),[$.pathname+$.search]},[i,o,n,r,c,t,e,s]),m=fe.useMemo(()=>tE(d,r),[d,r]),p=cE(d);return fe.createElement(fe.Fragment,null,f.map(b=>fe.createElement("link",{key:b,rel:"prefetch",as:"fetch",href:b,...a})),m.map(b=>fe.createElement("link",{key:b,rel:"modulepreload",href:b,...a})),p.map(({key:b,link:y})=>fe.createElement("link",{key:b,nonce:a.nonce,...y})))}function mE(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var V0=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{V0&&(window.__reactRouterVersion="7.9.1")}catch{}function mp({basename:e,children:t,window:a}){let n=W.useRef();n.current==null&&(n.current=k0({window:a,v5Compat:!0}));let r=n.current,[s,i]=W.useState({action:r.action,location:r.location}),o=W.useCallback(u=>{W.startTransition(()=>i(u))},[i]);return W.useLayoutEffect(()=>r.listen(o),[r,o]),W.createElement(lp,{basename:e,children:t,location:s.location,navigationType:s.action,navigator:r})}function G0({basename:e,children:t,history:a}){let[n,r]=W.useState({action:a.action,location:a.location}),s=W.useCallback(i=>{W.startTransition(()=>r(i))},[r]);return W.useLayoutEffect(()=>a.listen(s),[a,s]),W.createElement(lp,{basename:e,children:t,location:n.location,navigationType:n.action,navigator:a})}G0.displayName="unstable_HistoryRouter";var Y0=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i,Ar=W.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,state:o,target:u,to:c,preventScrollReset:d,viewTransition:f,...m},p){let{basename:b}=W.useContext(Bt),y=typeof c=="string"&&Y0.test(c),$,g=!1;if(typeof c=="string"&&y&&($=c,V0))try{let M=new URL(window.location.href),P=c.startsWith("//")?new URL(M.protocol+c):new URL(c),I=qa(P.pathname,b);P.origin===M.origin&&I!=null?c=I+P.search+P.hash:g=!0}catch{ea(!1,`<Link to="${c}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}let v=M0(c,{relative:r}),[x,w,S]=uE(n,m),R=W0(c,{replace:i,state:o,target:u,preventScrollReset:d,relative:r,viewTransition:f});function _(M){t&&t(M),M.defaultPrevented||R(M)}let E=W.createElement("a",{...m,...S,href:$||v,onClick:g||s?t:_,ref:mE(p,w),target:u,"data-discover":!y&&a==="render"?"true":void 0});return x&&!y?W.createElement(W.Fragment,null,E,W.createElement(Q0,{page:v})):E});Ar.displayName="Link";var Ha=W.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:u,...c},d){let f=Bs(i,{relative:c.relative}),m=je(),p=W.useContext(zs),{navigator:b,basename:y}=W.useContext(Bt),$=p!=null&&nx(f)&&o===!0,g=b.encodeLocation?b.encodeLocation(f).pathname:f.pathname,v=m.pathname,x=p&&p.navigation&&p.navigation.location?p.navigation.location.pathname:null;a||(v=v.toLowerCase(),x=x?x.toLowerCase():null,g=g.toLowerCase()),x&&y&&(x=qa(x,y)||x);let w=g!=="/"&&g.endsWith("/")?g.length-1:g.length,S=v===g||!r&&v.startsWith(g)&&v.charAt(w)==="/",R=x!=null&&(x===g||!r&&x.startsWith(g)&&x.charAt(g.length)==="/"),_={isActive:S,isPending:R,isTransitioning:$},E=S?t:void 0,M;typeof n=="function"?M=n(_):M=[n,S?"active":null,R?"pending":null,$?"transitioning":null].filter(Boolean).join(" ");let P=typeof s=="function"?s(_):s;return W.createElement(Ar,{...c,"aria-current":E,className:M,ref:d,style:P,to:i,viewTransition:o},typeof u=="function"?u(_):u)});Ha.displayName="NavLink";var J0=W.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=Gu,action:o,onSubmit:u,relative:c,preventScrollReset:d,viewTransition:f,...m},p)=>{let b=ex(),y=tx(o,{relative:c}),$=i.toLowerCase()==="get"?"get":"post",g=typeof o=="string"&&Y0.test(o);return W.createElement("form",{ref:p,method:$,action:y,onSubmit:n?u:x=>{if(u&&u(x),x.defaultPrevented)return;x.preventDefault();let w=x.nativeEvent.submitter,S=w?.getAttribute("formmethod")||i;b(w||x.currentTarget,{fetcherKey:t,method:S,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:f})},...m,"data-discover":!g&&e==="render"?"true":void 0})});J0.displayName="Form";function X0({getKey:e,storageKey:t,...a}){let n=W.useContext(Oo),{basename:r}=W.useContext(Bt),s=je(),i=ip();ax({getKey:e,storageKey:t});let o=W.useMemo(()=>{if(!n||!e)return null;let c=Xf(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let u=((c,d)=>{if(!window.history.state||!window.history.state.key){let f=Math.random().toString(32).slice(2);window.history.replaceState({key:f},"")}try{let m=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof m=="number"&&window.scrollTo(0,m)}catch(f){console.error(f),sessionStorage.removeItem(c)}}).toString();return W.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${u})(${JSON.stringify(t||Jf)}, ${JSON.stringify(o)})`}})}X0.displayName="ScrollRestoration";function Z0(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function fp(e){let t=W.useContext(Er);return Te(t,Z0(e)),t}function fE(e){let t=W.useContext(zs);return Te(t,Z0(e)),t}function W0(e,{target:t,replace:a,state:n,preventScrollReset:r,relative:s,viewTransition:i}={}){let o=ce(),u=je(),c=Bs(e,{relative:s});return W.useCallback(d=>{if(I3(d,t)){d.preventDefault();let f=a!==void 0?a:Fs(u)===Fs(c);o(e,{replace:f,state:n,preventScrollReset:r,relative:s,viewTransition:i})}},[u,o,c,a,n,t,e,r,s,i])}var pE=0,hE=()=>`__${String(++pE)}__`;function ex(){let{router:e}=fp("useSubmit"),{basename:t}=W.useContext(Bt),a=U3();return W.useCallback(async(n,r={})=>{let{action:s,method:i,encType:o,formData:u,body:c}=G3(n,t);if(r.navigate===!1){let d=r.fetcherKey||hE();await e.fetch(d,a,r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,flushSync:r.flushSync})}else await e.navigate(r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,replace:r.replace,state:r.state,fromRouteId:a,flushSync:r.flushSync,viewTransition:r.viewTransition})},[e,t,a])}function tx(e,{relative:t}={}){let{basename:a}=W.useContext(Bt),n=W.useContext(aa);Te(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...Bs(e||".",{relative:t})},i=je();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),u=o.getAll("index");if(u.some(d=>d==="")){o.delete("index"),u.filter(f=>f).forEach(f=>o.append("index",f));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:hn([a,s.pathname])),Fs(s)}var Jf="react-router-scroll-positions",Vu={};function Xf(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:qa(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function ax({getKey:e,storageKey:t}={}){let{router:a}=fp("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=fE("useScrollRestoration"),{basename:s}=W.useContext(Bt),i=je(),o=ip(),u=z0();W.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),vE(W.useCallback(()=>{if(u.state==="idle"){let c=Xf(i,o,s,e);Vu[c]=window.scrollY}try{sessionStorage.setItem(t||Jf,JSON.stringify(Vu))}catch(c){ea(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[u.state,e,s,i,o,t])),typeof document<"u"&&(W.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||Jf);c&&(Vu=JSON.parse(c))}catch{}},[t]),W.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(Vu,()=>window.scrollY,e?(d,f)=>Xf(d,f,s,e):void 0);return()=>c&&c()},[a,s,e]),W.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{ea(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function vE(e,t){let{capture:a}=t||{};W.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function nx(e,{relative:t}={}){let a=W.useContext(tp);Te(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=fp("useViewTransitionState"),r=Bs(e,{relative:t});if(!a.isTransitioning)return!1;let s=qa(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=qa(a.nextLocation.pathname,n)||a.nextLocation.pathname;return Mo(r.pathname,i)!=null||Mo(r.pathname,s)!=null}var Tt=new md({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var pp="ironclaw_token",pt="/api/webchat/v2",Dr=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function va(){return sessionStorage.getItem(pp)||""}function Hs(e){e?sessionStorage.setItem(pp,e):sessionStorage.removeItem(pp)}function Zu(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function ix(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function sx(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function ox({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=sx(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=sx(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function Y(e,t={}){let a=va(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await ix(r);throw new Dr(ox({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function Wu(){return Y(`${pt}/session`)}function ec({clientActionId:e,requestedThreadId:t,projectId:a}={}){let n={client_action_id:e||Zu()};return t&&(n.requested_thread_id=t),a&&(n.project_id=a),Y(`${pt}/threads`,{method:"POST",body:JSON.stringify(n)})}function lx({limit:e,cursor:t}={}){let a=new URL(`${pt}/threads`,window.location.origin);return e!=null&&a.searchParams.set("limit",String(e)),t&&a.searchParams.set("cursor",t),Y(a.pathname+a.search)}function ux({threadId:e}={}){return e?Y(`${pt}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function hp(e){return`${pt}/threads/${encodeURIComponent(e)}/files`}function cx({threadId:e,path:t}={}){if(!e)return Promise.reject(new Error("threadId is required"));let a=new URL(hp(e),window.location.origin);return t&&a.searchParams.set("path",t),Y(a.pathname+a.search)}function dx({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${hp(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),Y(a.pathname+a.search)}function tc({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${hp(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function mx({limit:e,runLimit:t,includeCompleted:a}={}){let n=new URLSearchParams;e!=null&&n.set("limit",String(e)),t!=null&&n.set("run_limit",String(t)),a===!0&&n.set("include_completed","true");let r=n.toString();return Y(`${pt}/automations${r?`?${r}`:""}`)}var fx=`${pt}/projects`;function gE(e){return`${fx}/${encodeURIComponent(e)}`}function px({limit:e}={}){let t=new URL(fx,window.location.origin);return e!=null&&t.searchParams.set("limit",String(e)),Y(t.pathname+t.search)}function hx({projectId:e}={}){return e?Y(gE(e)):Promise.reject(new Error("projectId is required"))}function vx(){return Y(`${pt}/outbound/preferences`)}function gx(){return Y(`${pt}/outbound/targets`)}function yx({finalReplyTargetId:e}={}){return Y(`${pt}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function bx({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:u,source:c}={}){let d=new URL(`${pt}/operator/logs`,window.location.origin);return e!=null&&d.searchParams.set("limit",String(e)),t&&d.searchParams.set("cursor",t),a&&d.searchParams.set("level",a),n&&d.searchParams.set("target",n),r&&d.searchParams.set("thread_id",r),s&&d.searchParams.set("run_id",s),i&&d.searchParams.set("turn_id",i),o&&d.searchParams.set("tool_call_id",o),u&&d.searchParams.set("tool_name",u),c&&d.searchParams.set("source",c),Y(d.pathname+d.search)}function xx({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||Zu(),content:t};return a.length>0&&(r.attachments=a),Y(`${pt}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function $x({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${pt}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),Y(n.pathname+n.search)}function wx({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${pt}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function Sa(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new Dr("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=va(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await ix(r);throw new Dr(ox({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function vp(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function ac(e){return vp(await Sa(e))}function Sx({threadId:e,afterCursor:t}={}){let a=new URL(`${pt}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=va();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function Nx({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||Zu()};return a&&(r.reason=a),Y(`${pt}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function gp({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let u={client_action_id:i||Zu(),resolution:n};return r!=null&&(u.always=r),s&&(u.credential_ref=s),Y(`${pt}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(u)})}function _x({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return Y("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function kx(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),Y(`${pt}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function Ks(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function Rx(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function Cx(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new Dr("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new Dr("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function Ex(){let e=va();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var nc="anon",Tx=nc;function Ax(e){Tx=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:nc}function St(){return Tx}var Dx="ironclaw:v2-thread-pins:",yp=new Set,vn=new Set,bp=null;function xp(){return`${Dx}${St()}`}function yE(){try{let e=window.localStorage.getItem(xp());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function bE(){try{vn.size===0?window.localStorage.removeItem(xp()):window.localStorage.setItem(xp(),JSON.stringify([...vn]))}catch{}}function Mx(){let e=St();if(e!==bp){vn.clear();for(let t of yE())vn.add(t);bp=e}}function Ox(){return new Set(vn)}function Lx(){let e=Ox();for(let t of yp)try{t(e)}catch{}}function Px(e){e&&(Mx(),vn.has(e)?vn.delete(e):vn.add(e),bE(),Lx())}function jx(){return Mx(),Ox()}function Ux(e){return yp.add(e),()=>{yp.delete(e)}}function Fx(){vn.clear(),bp=St();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(Dx)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}Lx()}var xE=0,Mr={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function $p(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function zx(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":$E(t)?"text":"download"}function $E(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function Lo(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function wE(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function SE(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function NE(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function qx(e,{limits:t,existing:a=[],t:n}){let r=t||Mr,s=[],i=[],o=a.length,u=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!wE(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:Lo(r.maxFileBytes)}));continue}if(u+c.size>r.maxTotalBytes){let y=n("chat.attachmentTotalTooLarge",{max:Lo(r.maxTotalBytes)});i.includes(y)||i.push(y);continue}let d;try{d=await SE(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:f,base64:m}=NE(d,c.type),p=f||"application/octet-stream",b=$p(p);s.push({id:`staged-${xE++}`,filename:c.name||"attachment",mimeType:p,kind:b,sizeBytes:c.size,sizeLabel:Lo(c.size),dataBase64:m,previewUrl:b==="image"?d:null}),o+=1,u+=c.size}return{staged:s,errors:i}}function Bx(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function Hx(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}function _E(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||$p(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?wx({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?Lo(n.size_bytes):"",preview_url:null,fetch_url:s}})}function Ix(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let c=EE(s);if(!c)continue;let d=`tool-${c.invocationId}`;if(n.has(d))continue;n.add(d),r.push({id:d,role:"tool_activity",...c,timestamp:Kx(s)||c.updatedAt||null,sequence:s.sequence,activityOrder:c.activityOrder,activityOrderSource:c.activityOrderSource,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=CE(s),u=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy");r.push({id:i,role:o,content:s.content||"",attachments:_E(s,a),timestamp:Kx(s),kind:s.kind,status:u?"error":s.status,...u&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:RE(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=kE(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function kE(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function RE(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function CE(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function Kx(e){return e.received_at||e.created_at||null}function EE(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:wp(t)}function wp(e){let t=e.status==="failed"||e.status==="killed",a=Vx(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Or(e.title||e.capability_id)||"tool",toolStatus:Qx(e.status),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(e.output_summary||e.output_preview||e.result_ref)||null,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null,activityOrder:a,activityOrderSource:Number.isFinite(a)?"projection":null}}function Sp(e){let t=Vx(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Or(e.capability_id)||"tool",toolStatus:Qx(e.status),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:null,toolError:e.error_kind||null,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null,activityOrder:t,activityOrderSource:Number.isFinite(t)?"projection":null}}function Is(e){return e==="success"||e==="error"}function Or(e){let t=typeof e=="string"?e.trim():"";if(!t)return"";let a=t.split(".");return a[a.length-1]||t}function Qx(e){switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}function Vx(e){let t=Number(e);return Number.isFinite(t)?t:null}var TE=50,gn=new Map,AE=30;function Np(e,t){for(gn.delete(e),gn.set(e,t);gn.size>AE;){let a=gn.keys().next().value;gn.delete(a)}}function rc(e){return`${St()}:${e}`}function Yx(){gn.clear()}function Jx(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?gn.get(rc(e)):null,[s,i]=h.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),o=h.default.useRef(new Set),u=h.default.useRef(e);u.current=e;let c=h.default.useCallback(async(d,f={})=>{let{preserveClientOnly:m=!1}=f;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(o.current.has(e))return;o.current.add(e);let p=St(),b=rc(e);i(y=>({...y,isLoading:!0}));try{let y=await $x({threadId:e,limit:TE,cursor:d});if(St()!==p)return;let $=d?[]:a?.()||[],g=Ix(y.messages||[],$,e),v=y.next_cursor||null;if(d||n?.([]),!d){let x=gn.get(b)?.messages||[],w=Gx(g,x,{preserveClientOnly:m});Np(b,{messages:w,nextCursor:v})}i(x=>{if(u.current!==e)return x;let w;return d?w=DE(g,x.messages):w=Gx(g,x.messages,{preserveClientOnly:m}),Np(b,{messages:w,nextCursor:v}),{messages:w,nextCursor:v,isLoading:!1,loadError:null}})}catch(y){if(console.error("Failed to load timeline:",y),St()!==p)return;i($=>u.current===e?{...$,isLoading:!1,loadError:"Failed to load conversation history."}:$)}finally{o.current.delete(e)}},[e,a,n]);return h.default.useEffect(()=>{let d=e?gn.get(rc(e)):null;i({messages:d?.messages||[],nextCursor:d?.nextCursor||null,isLoading:!!e&&!d,loadError:null}),e&&c()},[e,c]),{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:c,setMessages:d=>i(f=>{let m=typeof d=="function"?d(f.messages):d;return e&&Np(rc(e),{messages:m,nextCursor:f.nextCursor}),{...f,messages:m}})}}function DE(e,t){let a=new Set(t.map(n=>n?.id).filter(Boolean));return[...e.filter(n=>!a.has(n?.id)),...t]}function Gx(e,t,a={}){let{preserveClientOnly:n=!1}=a,r=new Set(e.map(i=>i?.id).filter(Boolean)),s=t.filter(i=>!i||typeof i.id!="string"||r.has(i.id)?!1:ME(i)?!0:n&&i.id.startsWith("err-"));return s.length>0?[...e,...s]:e}function ME(e){return e?.role==="tool_activity"||e?.role==="thinking"}var jo="__new__",Xx="ironclaw:v2-draft:";function Qs(e){return`${Xx}${St()}:${e||jo}`}function _p(e){try{return window.localStorage.getItem(Qs(e))||""}catch{return""}}function kp(e,t){try{t?window.localStorage.setItem(Qs(e),t):window.localStorage.removeItem(Qs(e))}catch{}}function Zx(e){kp(e,"")}var Po=new Map;function Rp(e){return Po.get(Qs(e))||[]}function Wx(e,t){let a=Qs(e);t&&t.length>0?Po.set(a,t):Po.delete(a)}function e$(e){Po.delete(Qs(e))}function t$(){Po.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(Xx)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function OE(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function LE(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function PE(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=OE(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?LE(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),va()?"":(Hs(n),n)}function jE(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var UE={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function FE(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),UE[t]||"Could not complete sign-in. Please try again."):""}function a$(){let[e,t]=h.default.useState(()=>PE()||va()),[a,n]=h.default.useState(()=>FE()),[r]=h.default.useState(()=>jE()),[s,i]=h.default.useState(null),[o,u]=h.default.useState(()=>!!(r&&!va())),[c,d]=h.default.useState(()=>!!va());h.default.useEffect(()=>{if(!r||va()){u(!1);return}let b=!1;return Cx(r).then(y=>{b||(Hs(y),d(!0),t(y),i(null),n(""),u(!1),Tt.clear())}).catch(()=>{b||(n("Could not complete sign-in. Please try again."),u(!1))}),()=>{b=!0}},[r]),h.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let b=!1;return d(!0),Wu().then(y=>{b||(i(y),d(!1))}).catch(y=>{b||(i(null),d(!1),(y?.status===401||y?.status===403)&&(Hs(""),t(""),n("Your session expired. Please sign in again."),Tt.clear()))}),()=>{b=!0}},[e,o]),Ax(s);let f=h.default.useRef(null);h.default.useEffect(()=>{let b=St();f.current&&f.current!==nc&&f.current!==b&&(Yx(),t$(),Fx()),f.current=b},[s]);let m=h.default.useCallback(b=>{Hs(b),d(!!b),t(b),i(null),n(""),Tt.clear()},[]),p=h.default.useCallback(()=>{Ex().catch(()=>{}),Hs(""),d(!1),t(""),i(null),n(""),Tt.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,rebornProjectsEnabled:!!s?.features?.reborn_projects,signIn:m,signOut:p}}var Lr="/chat",Uo=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace"},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var zE=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],qE=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],BE=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],sc={settings:zE,extensions:qE,admin:BE};var n$="ironclaw:v2-theme";function HE(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(n$);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function ic(){let[e,t]=h.default.useState(HE);h.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(n$,e)}catch{}},[e]);let a=h.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function r$(e){return z({enabled:!!e,queryKey:["gateway-status",e],queryFn:Ks,refetchInterval:3e4})}function s$(){return Promise.resolve({settings:{},todo:!0})}function i$(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 settings endpoint"})}function o$(e){return Promise.resolve({success:!1,message:"TODO: requires v2 settings endpoint"})}function oc(){return Y("/api/webchat/v2/llm/providers")}function l$(e){return Y("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function u$(e){return Y(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function Fo(e){return Y("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function c$(e){return Y("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function d$(e){return Y("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function m$(e){return Y("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function f$(e){return Y("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function p$(){return Y("/api/webchat/v2/llm/codex/login",{method:"POST"})}function h$(){return Promise.resolve({tools:[],todo:!0})}function v$(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 tools endpoint"})}function g$(){return Y("/api/webchat/v2/extensions")}function y$(){return Y("/api/webchat/v2/extensions/registry")}function b$(){return Y("/api/webchat/v2/skills")}function x$(e){return Y(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function $$(e){return Y("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function w$(e,t){return Y(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function S$(e){return Y(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function N$(){return Y("/api/webchat/v2/traces/credit")}function _$(e){return Y(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function k$(){return Promise.resolve({users:[],todo:!0})}function R$(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function C$(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var Cp="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",Ep=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function zo(e){return Ep.find(t=>t.value===e)?.label||e}function Vs(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function E$(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function lc(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function T$(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function Pr(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Cp||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?Vs(e,t).trim().length>0:!0:!1}function KE(e,t,a){return e.id===a?"active":Pr(e,t)?"ready":"setup"}function A$(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=KE(r,t,a);n[s]&&n[s].push(r)}return n}function uc(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Cp||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!Vs(e,t).trim()?"base_url":"ok"}function Tp(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===Cp&&(i.api_key=void 0),i}function D$(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function M$(e){return/^[a-z0-9_-]+$/.test(e)}function O$(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var IE=Object.freeze({});function Gs({settings:e,gatewayStatus:t,enabled:a=!0}){let n=X(),r=z({queryKey:["llm-providers"],queryFn:oc,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=IE,u=(s.providers||[]).map(w=>({...w,name:w.description,has_api_key:w.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,f=d||"nearai",m=s.active?.model||t?.llm_model||"",p=u.filter(w=>w.builtin),b=u.filter(w=>!w.builtin),y=[...u].sort((w,S)=>w.id===d?-1:S.id===d?1:(w.name||w.id).localeCompare(S.name||S.id)),$=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=Q({mutationFn:async w=>{if(!Pr(w,o)){let R=uc(w,o);throw new Error(R==="base_url"?"base_url":"api_key")}let S=lc(w,o);if(!S)throw new Error("model");return await Fo({provider_id:w.id,model:S}),w},onSuccess:$}),v=Q({mutationFn:async({provider:w,form:S,apiKey:R,editingProvider:_})=>{let E=!!w?.builtin,P={id:(E?w.id:S.id.trim()).trim(),name:E?w.name||w.id:S.name.trim(),adapter:E?w.adapter:S.adapter,base_url:S.baseUrl.trim()||w?.base_url||"",default_model:S.model.trim()||void 0};return R.trim()&&(P.api_key=R.trim()),(_||w)?.id===f&&P.default_model&&(P.set_active=!0,P.model=P.default_model),await l$(P),P},onSuccess:$}),x=Q({mutationFn:async w=>(await u$(w.id),w),onSuccess:$});return{providers:y,builtinProviders:p,customProviders:b,builtinOverrides:o,activeProviderId:d,selectedModel:m,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:w=>g.mutateAsync(w),saveCustomProvider:w=>v.mutateAsync(w),saveBuiltinProvider:w=>v.mutateAsync(w),deleteCustomProvider:w=>x.mutateAsync(w),testConnection:c$,listModels:d$,isBusy:g.isPending||v.isPending||x.isPending}}function L$({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}function P$({onNewChat:e}={}){let t=ce(),[a,n]=h.default.useState(!1),r=h.default.useCallback(()=>n(!1),[]),s=h.default.useCallback(()=>n(u=>!u),[]),i=h.default.useCallback(async()=>{let u=await e?.(),c=typeof u=="string"&&u.length>0?u:null;t(c?`/chat/${c}`:"/chat"),r()},[t,r,e]),o=h.default.useCallback(u=>{t(`/chat/${u}`),r()},[t,r]);return{open:a,close:r,toggle:s,newChat:i,selectThread:o}}var Ap=new Set,QE=0;function Ys(e,t={}){let a={id:++QE,message:e,tone:t.tone||"info",duration:t.duration??2600};return Ap.forEach(n=>n(a)),a.id}function j$(e){return Ap.add(e),()=>Ap.delete(e)}function VE(e){return e?.status===409&&e?.payload?.kind==="busy"}function U$(e,t){return VE(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function F$(){let e=z({queryKey:["threads"],queryFn:()=>lx({})}),[t,a]=h.default.useState(null),[n,r]=h.default.useState(!1),s=h.default.useRef(new Map),i=h.default.useCallback(async c=>{let d=c||"__global__",f=s.current.get(d);if(f)return f;r(!0);let m=(async()=>{try{let p=await ec(c?{projectId:c}:void 0);Tt.invalidateQueries({queryKey:["threads"]});let b=p?.thread?.thread_id;return b&&a(b),b}finally{s.current.delete(d),r(s.current.size>0)}})();return s.current.set(d,m),m},[]),o=h.default.useCallback(async c=>{await ux({threadId:c}),t===c&&a(null),Tt.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:h.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var z$={attach:l`<path
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
    />`,flag:l`<path d="M6.5 21V4.5" /><path d="M6.5 5h10.7l-1.4 4 1.4 4H6.5" />`,pin:l`<path d="M9 3.5h6l-1 5 3 3.5H7l3-3.5-1-5Z" /><path d="M12 15.5V21" />`,folder:l`<path
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
    />`,arrowDown:l`<path d="M12 5v14" /><path d="m6 13 6 6 6-6" />`,retry:l`<path d="M3.5 12a8.5 8.5 0 1 1 2.6 6.1" /><path d="M3.2 18.5v-5h5" />`};function A({name:e,className:t="",strokeWidth:a=1.7}){return l`
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
      ${z$[e]||z$.spark}
    </svg>
  `}function H(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=H(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function q$(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function GE(e){return q$(e).trim().charAt(0).toUpperCase()||"I"}function YE(){let[e,t]=h.default.useState(!1),a=h.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function B$({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=k(),s=YE(),i=q$(a),o=a?.email||a?.role||r("common.gatewaySession");return l`
    <div
      className="relative flex items-center gap-2 border-t border-[var(--v2-panel-border)] px-3 py-3"
    >
      ${s.open&&l`
        <div
          className=${H("absolute bottom-full left-3 right-3 mb-2 rounded-[10px] border p-3 shadow-lg","border-[var(--v2-panel-border)] bg-[var(--v2-surface)]")}
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
            />`:l`<span className="place-self-center">${GE(a)}</span>`}
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
        <${A} name=${e==="dark"?"sun":"moon"} className="h-4 w-4" />
      </button>
      <button
        onClick=${n}
        className="grid h-8 w-8 shrink-0 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
        title=${r("header.signOut")}
      >
        <${A} name="logout" className="h-4 w-4" />
      </button>
    </div>
  `}var H$={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",settings:"settings",admin:"shield"},JE=Uo.filter(e=>e.id!=="chat"&&!e.hidden);function XE({route:e,label:t,onNavigate:a}){return l`
    <${Ha}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>H("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${A} name=${H$[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function ZE({route:e,label:t,subRoutes:a,onNavigate:n}){let r=k(),s=je(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return l`
    <div className="flex flex-col">
      <${Ha}
        to=${o}
        onClick=${n}
        className=${()=>H("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${A}
          name=${H$[e.id]||"bolt"}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1 truncate">${t}</span>
        <${A}
          name="chevron"
          className=${H("h-3.5 w-3.5 shrink-0 transition-transform duration-150",i&&"rotate-180")}
        />
      <//>

      ${i&&l`
        <div className="mt-0.5 flex flex-col gap-0.5 pl-3">
          ${a.map(u=>l`
              <${Ha}
                key=${u.id}
                to=${e.path+"/"+u.id}
                onClick=${n}
                className=${({isActive:c})=>H("flex items-center gap-2.5 rounded-[8px] py-1.5 pl-7 pr-3 text-[12px] font-medium",c?"text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
              >
                <${A} name=${u.icon} className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate">${r(u.labelKey)}</span>
              <//>
            `)}
        </div>
      `}
    </div>
  `}function K$({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=k(),s=h.default.useMemo(()=>JE.filter(i=>a||i.id!=="admin"),[a]);return l`
    <div className="flex flex-col px-3 py-2">
      <button
        onClick=${e}
        disabled=${t}
        className=${H("flex items-center gap-2.5 rounded-[10px] px-3 py-2","border border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]","bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]","hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)] disabled:opacity-50")}
      >
        <${A} name="plus" className="h-4 w-4 shrink-0" />
        <span className="text-[13px] font-medium">
          ${r(t?"chat.creating":"chat.newThread")}
        </span>
      </button>

      <nav className="mt-2 flex flex-col gap-1">
        ${s.map(i=>{let o=(sc[i.id]||[]).filter(u=>a||!(i.id==="settings"&&["users","inference"].includes(u.id)));return o.length>0?l`
              <${ZE}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:l`
            <${XE}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var yn=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),qo=new Set([yn.NEEDS_ATTENTION,yn.FAILED]),Dp="ironclaw:v2-thread-attention",Mp=new Set,Js=new Map;function WE(){try{let e=window.localStorage.getItem(Dp);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&qo.has(a[1])):[]}catch{return[]}}function I$(){let e=[];for(let[t,a]of Js)qo.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(Dp):window.localStorage.setItem(Dp,JSON.stringify(e))}catch{}}for(let[e,t]of WE())Js.set(e,t);function V$(){return new Map(Js)}function Q$(){let e=V$();for(let t of Mp)try{t(e)}catch{}}function cc(e,t){if(!e)return;let a=Js.get(e);if(t==null){if(!Js.delete(e))return;qo.has(a)&&I$(),Q$();return}a!==t&&(Js.set(e,t),(qo.has(t)||qo.has(a))&&I$(),Q$())}function G$(e){cc(e,null)}function e4(){return V$()}function t4(e){return Mp.add(e),()=>{Mp.delete(e)}}function Y$(){let[e,t]=h.default.useState(e4);return h.default.useEffect(()=>t4(t),[]),e}function dc(e){return e.updated_at||e.created_at||null}function Op(e,t){let a=dc(e)||"",n=dc(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function J$(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function X$(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function a4(){let[e,t]=h.default.useState(jx);return h.default.useEffect(()=>Ux(t),[]),e}var n4=Object.freeze({[yn.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-transparent"},[yn.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[yn.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function r4(e){return e&&n4[e]||null}function s4({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=k(),o=dc(e),u=J$(o),c=X$(o),d=h.default.useCallback(m=>{m.preventDefault(),m.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(p=>{window.alert(p?.message||"Unable to delete chat")})},[s,e.id]),f=h.default.useCallback(m=>{m.preventDefault(),m.stopPropagation(),Px(e.id)},[e.id]);return l`
    <div
      className=${H("group flex w-full items-stretch rounded-[8px] border-l-2",n?n.borderClass:t?"border-[var(--v2-accent)]":"border-transparent",t?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
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
            className=${H("h-1.5 w-1.5 shrink-0 rounded-full",n.dotClass)}
          />`}
        </div>
        ${(n||u)&&l`<span
          className=${H("block truncate text-[11px]",n?n.textClass:"text-[var(--v2-text-faint)]")}
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
        className=${H("my-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] transition",a?"text-[var(--v2-accent-text)]":"opacity-0 text-[var(--v2-text-faint)] group-hover:opacity-100 focus:opacity-100","hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-accent-text)]")}
      >
        <${A} name="pin" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>
      ${s&&l`<button
        type="button"
        onClick=${d}
        title=${i("common.deleteChat")}
        aria-label=${i("common.deleteChat")}
        className=${H("my-1 mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px]","opacity-0 transition group-hover:opacity-100 focus:opacity-100","text-[var(--v2-text-faint)] hover:bg-[var(--v2-danger-soft)] hover:text-[var(--v2-danger-text)]")}
      >
        <${A} name="trash" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>`}
    </div>
  `}function Z$({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:l`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>l`
          <${s4}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${r4(n.get(o.id))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function W$({threads:e,activeThreadId:t,rebornProjectsEnabled:a=!1,onSelect:n,onDelete:r,onNavigate:s}){let[i,o]=h.default.useState(!1),[u,c]=h.default.useState(""),d=Y$(),f=a4(),m=k(),{pinned:p,recent:b,totalMatches:y}=h.default.useMemo(()=>{let $=u.trim().toLowerCase(),g=$?e.filter(w=>(w.title||w.id||"").toLowerCase().includes($)):e,v=[],x=[];for(let w of g)f.has(w.id)?v.push(w):x.push(w);return v.sort(Op),x.sort(Op),{pinned:v,recent:x,totalMatches:v.length+x.length}},[e,u,f]);return l`
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
        <${A}
          name="chevron"
          className=${H("h-3.5 w-3.5 text-[var(--v2-text-faint)]",i?"-rotate-90":"")}
          strokeWidth=${2.2}
        />
      </button>

      ${!i&&l`
        ${e.length>0&&l`<div className="relative mb-1 mt-1 px-1">
          <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--v2-text-faint)]">
            <${A} name="search" className="h-3.5 w-3.5" />
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
          <${Ha}
            to="/projects"
            onClick=${s}
            className=${({isActive:$})=>H("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",$?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
          >
            <${A} name="folder" className="h-4 w-4 shrink-0" />
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

          <${Z$}
            label=${m("common.pinned")}
            items=${p}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${f}
            onSelect=${n}
            onDelete=${r}
          />
          <${Z$}
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
  `}function mc(){let e=X(),t=z({queryKey:["trace-credits"],queryFn:N$,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=Q({mutationFn:_$,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function i4(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function e1(){let e=k(),{credits:t}=mc();if(!t||!t.enrolled)return null;let a=i4(t.final_credit),n=t.submissions_accepted||0,r=t.submissions_submitted||0,s=t.manual_review_hold_count||0;return l`
    <div className="px-3 pb-1">
      <${Ar}
        to="/settings/traces"
        className="block rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2.5 transition-colors hover:border-[var(--v2-accent-soft)] hover:bg-[var(--v2-surface-muted)]"
      >
        <div className="flex items-center gap-2 text-[var(--v2-accent-text)]">
          <${A} name="layers" className="h-3.5 w-3.5 shrink-0" />
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
  `}function t1({threadsState:e,theme:t,toggleTheme:a,profile:n,isAdmin:r,rebornProjectsEnabled:s=!1,onSignOut:i,onClose:o,onNewChat:u,onSelectThread:c,onDeleteThread:d}){return l`
    <aside
      className="flex h-full w-[260px] shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
    >
      <div className="flex items-center gap-2.5 px-4 py-5">
        <${Ar}
          to="/chat"
          onClick=${o}
          className="flex items-center gap-2.5 opacity-90 hover:opacity-100"
          aria-label="IronClaw"
        >
          <img src="/v2/assets/logo.jpg" alt="IronClaw" className="h-7 w-auto" />
        <//>
      </div>

      <${K$}
        onNewChat=${u}
        isCreating=${e.isCreating}
        isAdmin=${r}
        onNavigate=${o}
      />

      <${e1} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${W$}
          threads=${e.threads}
          activeThreadId=${e.activeThreadId}
          rebornProjectsEnabled=${s}
          onSelect=${c}
          onDelete=${d}
          onNavigate=${o}
        />
      </div>

      <${B$}
        theme=${t}
        toggleTheme=${a}
        profile=${n}
        onSignOut=${i}
      />
    </aside>
  `}var o4="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",l4="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",a1="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",n1={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},r1={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function T({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=n1[n]??n1.md,u=r?"w-full":"";if(a==="primary")return l`
      <${s}
        style=${{background:o4,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${H(a1,o,u,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:l4}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=r1[a]??r1.outline;return l`
    <${s}
      className=${H(a1,o,u,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function s1(){let e=h.default.useMemo(()=>u4(window.location),[]),[t,a]=h.default.useState(null),[n,r]=h.default.useState(null),[s,i]=h.default.useState(!1),[o,u]=h.default.useState(""),[c,d]=h.default.useState(!1);h.default.useEffect(()=>{if(!e)return;let p=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:p.signal}).then(b=>{if(!b.ok)throw new Error(String(b.status));return b.json()}).then(a).catch(()=>{p.signal.aborted||a(null)}),()=>p.abort()},[e]);let f=h.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),u("");try{let p=await fetch(`${e.base}/attestation/report`);if(!p.ok)throw new Error(String(p.status));let b=await p.json();return r(b),b}catch(p){return u(p.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),m=h.default.useCallback(async()=>{let p=n||await f();return!p||!navigator.clipboard?!1:(await navigator.clipboard.writeText(JSON.stringify({...p,instance_attestation:t},null,2)),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[f,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:f,copyReport:m}}function u4(e){let t=e.hostname;if(!t||t==="localhost"||c4(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function c4(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}var d4=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function i1(){let e=k(),t=s1(),[a,n]=h.default.useState(!1),r=h.default.useCallback(()=>{n(o=>{let u=!o;return u&&t.loadReport(),u})},[t]),s=h.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=m4({teeInfo:t.teeInfo,report:t.report,t:e});return l`
    <div className="relative">
      <button
        type="button"
        onClick=${r}
        aria-expanded=${a}
        title=${e("tee.title")}
        className=${H("grid h-8 w-8 place-items-center rounded-[8px]","border border-[color-mix(in_srgb,var(--v2-positive-text)_28%,transparent)]","bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]","hover:border-[color-mix(in_srgb,var(--v2-positive-text)_52%,transparent)]")}
      >
        <${A} name="shield" className="h-4 w-4" />
      </button>

      ${a&&l`
        <div
          className=${H("absolute right-0 top-full z-40 mt-2 w-[min(22rem,calc(100vw-2rem))]","rounded-[14px] border border-[var(--v2-panel-border)]","bg-[var(--v2-surface)] p-3 shadow-[0_18px_48px_rgba(0,0,0,0.35)]")}
        >
          <div className="flex items-center gap-2">
            <span className="grid h-8 w-8 place-items-center rounded-[10px] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]">
              <${A} name="shield" className="h-4 w-4" />
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
            <${T}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${t.reportLoading}
              onClick=${s}
            >
              <${A} name="check" className="h-4 w-4" />
              ${t.copied?e("tee.copied"):e("tee.copyReport")}
            <//>
          </div>
        </div>
      `}
    </div>
  `}function m4({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return d4.map(([r,s])=>({label:a(s),value:f4(n[r])||a("common.unknown")}))}function f4(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var p4="https://docs.ironclaw.com";function o1({threadsState:e,onToggleSidebar:t}){let a=k(),n=je(),r=h.default.useMemo(()=>{for(let i of Uo){let o=sc[i.id];if(!o)continue;let u=i.path+"/";if(n.pathname.startsWith(u)){let c=n.pathname.slice(u.length).split("/")[0],d=o.find(f=>f.id===c);if(d)return{parent:a(i.labelKey),current:a(d.labelKey)}}}return null},[n.pathname,a]),s=h.default.useMemo(()=>{if(r)return null;if(n.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(u=>u.id===e.activeThreadId)?.title||a("nav.chat");let i=Uo.find(o=>n.pathname.startsWith(o.path));return i?a(i.labelKey):""},[n.pathname,e.activeThreadId,e.threads,a,r]);return l`
    <header
      className=${H("flex h-14 shrink-0 items-center gap-3 px-4","border-b border-[var(--v2-panel-border)]","bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] backdrop-blur-xl")}
    >
      <button
        onClick=${t}
        className="grid h-8 w-8 shrink-0 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] md:hidden"
        aria-label="Toggle sidebar"
      >
        <${A} name="list" className="h-4 w-4" />
      </button>

      ${r?l`
            <div className="flex min-w-0 items-center gap-2 text-[14px] font-semibold">
              <span className="shrink-0 text-[var(--v2-text-muted)]">
                ${r.parent}
              </span>
              <${A}
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
        <${i1} />
        <${Ha}
          to="/logs"
          className=${({isActive:i})=>H("inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",i&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
          title=${a("nav.logs")}
        >
          ${a("nav.logs")}
        <//>
        <a
          href=${p4}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${a("nav.docs")}
        >
          ${a("nav.docs")}
        </a>
      </div>
    </header>
  `}function l1({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=ce(),i=k(),[o,u]=h.default.useState(""),[c,d]=h.default.useState(0),f=h.default.useRef(null),m=h.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(x=>({id:`thread-${x.id}`,label:x.title||`Thread ${x.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${x.id}`)}));return[...g,...v]},[a,s,n,r]),p=h.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?m.filter(v=>v.label.toLowerCase().includes(g)):m},[m,o]);h.default.useEffect(()=>{if(!e)return;u(""),d(0);let g=window.requestAnimationFrame(()=>f.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),h.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,p.length-1)))},[p.length]);let b=h.default.useCallback(g=>{g&&(t(),g.run())},[t]),y=h.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,p.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),b(p[c])):g.key==="Escape"&&(g.preventDefault(),t())},[p,c,b,t]);if(!e)return null;let $=null;return l`
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh]" role="dialog" aria-modal="true" aria-label="Command palette">
      <button type="button" aria-label="Close" onClick=${t} className="absolute inset-0 bg-black/50"></button>
      <div className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_30px_60px_-20px_rgba(0,0,0,0.8)]">
        <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-3">
          <${A} name="search" className="h-4 w-4 text-[var(--v2-text-faint)]" />
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
                  <${A} name=${g.icon} className="h-4 w-4 shrink-0" />
                  <span className="min-w-0 truncate">${g.label}</span>
                </button>
              </li>
            `})}
        </ul>
      </div>
    </div>
  `}var u1={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},h4={info:"bolt",success:"check",error:"close"};function c1(){let[e,t]=h.default.useState([]);return h.default.useEffect(()=>j$(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:l`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${e.map(a=>l`
          <div
            key=${a.id}
            role="status"
            className=${["pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",u1[a.tone]||u1.info].join(" ")}
          >
            <${A} name=${h4[a.tone]||"bolt"} className="h-4 w-4 shrink-0" />
            <span>${a.message}</span>
          </div>
        `)}
    </div>
  `}function d1({token:e,profile:t,isChecking:a=!1,isAdmin:n,rebornProjectsEnabled:r=!1,onSignOut:s}){let i=k(),{theme:o,toggleTheme:u}=ic(),c=r$(e),d=F$(),f=P$({onNewChat:()=>d.setActiveThreadId(null)}),m=c.data,p=je(),b=ce(),y=Gs({settings:{},gatewayStatus:m,enabled:n}),$=n&&L$({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),g=p.pathname==="/welcome"||p.pathname.startsWith("/settings"),[v,x]=h.default.useState(!1);h.default.useEffect(()=>{let S=R=>{(R.metaKey||R.ctrlKey)&&R.key.toLowerCase()==="k"&&(R.preventDefault(),x(_=>!_))};return window.addEventListener("keydown",S),()=>window.removeEventListener("keydown",S)},[]);let w=h.default.useCallback(async S=>{let R=d.activeThreadId===S;try{await d.deleteThread(S),R&&b("/chat",{replace:!0})}catch(_){console.error("Failed to delete thread:",_),Ys(U$(_,i),{tone:"error"})}},[b,d,i]);return $&&!g?l`<${lt} to="/welcome" replace />`:l`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${f.open&&l`<button
        type="button"
        aria-label=${i("nav.close")}
        onClick=${f.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${H("fixed inset-y-0 left-0 z-50 md:relative md:z-auto",f.open?"flex":"hidden md:flex")}
      >
        <${t1}
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
        <${o1}
          threadsState=${d}
          onToggleSidebar=${f.toggle}
        />
        <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
          ${c.error&&l`
            <div
              className=${H("m-4 rounded-[14px] border px-4 py-3 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >
              ${c.error.message||i("error.gatewayConnection")}
            </div>
          `}
          <${op}
            context=${{gatewayStatus:m,gatewayStatusQuery:c,currentUser:t,isChecking:a,isAdmin:n,threadsState:d}}
          />
        </main>
      </div>
      <${l1}
        open=${v}
        onClose=${()=>x(!1)}
        threadsState=${d}
        onNewChat=${f.newChat}
        onToggleTheme=${u}
      />
      <${c1} />
    </div>
  `}var Ht=qe(Ie(),1),Qo=e=>e.type==="checkbox",jr=e=>e instanceof Date,At=e=>e==null,N1=e=>typeof e=="object",Ge=e=>!At(e)&&!Array.isArray(e)&&N1(e)&&!jr(e),v4=e=>Ge(e)&&e.target?Qo(e.target)?e.target.checked:e.target.value:e,g4=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,y4=(e,t)=>e.has(g4(t)),b4=e=>{let t=e.constructor&&e.constructor.prototype;return Ge(t)&&t.hasOwnProperty("isPrototypeOf")},jp=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function ht(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(jp&&(e instanceof Blob||n))&&(a||Ge(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!b4(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=ht(e[r]));else return e;return t}var gc=e=>/^\w*$/.test(e),We=e=>e===void 0,Up=e=>Array.isArray(e)?e.filter(Boolean):[],Fp=e=>Up(e.replace(/["|']|\]/g,"").split(/\.|\[/)),V=(e,t,a)=>{if(!t||!Ge(e))return a;let n=(gc(t)?[t]:Fp(t)).reduce((r,s)=>At(r)?r:r[s],e);return We(n)||n===e?We(e[t])?a:e[t]:n},Ka=e=>typeof e=="boolean",Ue=(e,t,a)=>{let n=-1,r=gc(t)?[t]:Fp(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],u=a;if(n!==i){let c=e[o];u=Ge(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=u,e=e[o]}},m1={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},Na={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},bn={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},x4=Ht.default.createContext(null);x4.displayName="HookFormContext";var $4=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==Na.all&&(t._proxyFormState[i]=!n||Na.all),a&&(a[i]=!0),e[i]}});return r},w4=typeof window<"u"?Ht.default.useLayoutEffect:Ht.default.useEffect;var Ia=e=>typeof e=="string",S4=(e,t,a,n,r)=>Ia(e)?(n&&t.watch.add(e),V(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),V(a,s))):(n&&(t.watchAll=!0),a),Pp=e=>At(e)||!N1(e);function Wn(e,t,a=new WeakSet){if(Pp(e)||Pp(t))return e===t;if(jr(e)&&jr(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(jr(i)&&jr(o)||Ge(i)&&Ge(o)||Array.isArray(i)&&Array.isArray(o)?!Wn(i,o,a):i!==o)return!1}}return!0}var N4=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},Ko=e=>Array.isArray(e)?e:[e],f1=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},Kt=e=>Ge(e)&&!Object.keys(e).length,zp=e=>e.type==="file",_a=e=>typeof e=="function",pc=e=>{if(!jp)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},_1=e=>e.type==="select-multiple",qp=e=>e.type==="radio",_4=e=>qp(e)||Qo(e),Lp=e=>pc(e)&&e.isConnected;function k4(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=We(e)?n++:e[t[n++]];return e}function R4(e){for(let t in e)if(e.hasOwnProperty(t)&&!We(e[t]))return!1;return!0}function Ze(e,t){let a=Array.isArray(t)?t:gc(t)?[t]:Fp(t),n=a.length===1?e:k4(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Ge(n)&&Kt(n)||Array.isArray(n)&&R4(n))&&Ze(e,a.slice(0,-1)),e}var k1=e=>{for(let t in e)if(_a(e[t]))return!0;return!1};function hc(e,t={}){let a=Array.isArray(e);if(Ge(e)||a)for(let n in e)Array.isArray(e[n])||Ge(e[n])&&!k1(e[n])?(t[n]=Array.isArray(e[n])?[]:{},hc(e[n],t[n])):At(e[n])||(t[n]=!0);return t}function R1(e,t,a){let n=Array.isArray(e);if(Ge(e)||n)for(let r in e)Array.isArray(e[r])||Ge(e[r])&&!k1(e[r])?We(t)||Pp(a[r])?a[r]=Array.isArray(e[r])?hc(e[r],[]):{...hc(e[r])}:R1(e[r],At(t)?{}:t[r],a[r]):a[r]=!Wn(e[r],t[r]);return a}var Bo=(e,t)=>R1(e,t,hc(t)),p1={value:!1,isValid:!1},h1={value:!0,isValid:!0},C1=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!We(e[0].attributes.value)?We(e[0].value)||e[0].value===""?h1:{value:e[0].value,isValid:!0}:h1:p1}return p1},E1=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>We(e)?e:t?e===""?NaN:e&&+e:a&&Ia(e)?new Date(e):n?n(e):e,v1={isValid:!1,value:null},T1=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,v1):v1;function g1(e){let t=e.ref;return zp(t)?t.files:qp(t)?T1(e.refs).value:_1(t)?[...t.selectedOptions].map(({value:a})=>a):Qo(t)?C1(e.refs).value:E1(We(t.value)?e.ref.value:t.value,e)}var C4=(e,t,a,n)=>{let r={};for(let s of e){let i=V(t,s);i&&Ue(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},vc=e=>e instanceof RegExp,Ho=e=>We(e)?e:vc(e)?e.source:Ge(e)?vc(e.value)?e.value.source:e.value:e,y1=e=>({isOnSubmit:!e||e===Na.onSubmit,isOnBlur:e===Na.onBlur,isOnChange:e===Na.onChange,isOnAll:e===Na.all,isOnTouch:e===Na.onTouched}),b1="AsyncFunction",E4=e=>!!e&&!!e.validate&&!!(_a(e.validate)&&e.validate.constructor.name===b1||Ge(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===b1)),T4=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),x1=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),Io=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=V(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(Io(o,t))break}else if(Ge(o)&&Io(o,t))break}}};function $1(e,t,a){let n=V(e,a);if(n||gc(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=V(t,s),o=V(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var A4=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return Kt(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||Na.all))},D4=(e,t,a)=>!e||!t||e===t||Ko(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),M4=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,O4=(e,t)=>!Up(V(e,t)).length&&Ze(e,t),L4=(e,t,a)=>{let n=Ko(V(e,a));return Ue(n,"root",t[a]),Ue(e,a,n),e},fc=e=>Ia(e);function w1(e,t,a="validate"){if(fc(e)||Array.isArray(e)&&e.every(fc)||Ka(e)&&!e)return{type:a,message:fc(e)?e:"",ref:t}}var Xs=e=>Ge(e)&&!vc(e)?e:{value:e,message:""},S1=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:u,maxLength:c,minLength:d,min:f,max:m,pattern:p,validate:b,name:y,valueAsNumber:$,mount:g}=e._f,v=V(a,y);if(!g||t.has(y))return{};let x=o?o[0]:i,w=D=>{r&&x.reportValidity&&(x.setCustomValidity(Ka(D)?"":D||""),x.reportValidity())},S={},R=qp(i),_=Qo(i),E=R||_,M=($||zp(i))&&We(i.value)&&We(v)||pc(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,P=N4.bind(null,y,n,S),I=(D,K,J,ve=bn.maxLength,_e=bn.minLength)=>{let Ye=D?K:J;S[y]={type:D?ve:_e,message:Ye,ref:i,...P(D?ve:_e,Ye)}};if(s?!Array.isArray(v)||!v.length:u&&(!E&&(M||At(v))||Ka(v)&&!v||_&&!C1(o).isValid||R&&!T1(o).isValid)){let{value:D,message:K}=fc(u)?{value:!!u,message:u}:Xs(u);if(D&&(S[y]={type:bn.required,message:K,ref:x,...P(bn.required,K)},!n))return w(K),S}if(!M&&(!At(f)||!At(m))){let D,K,J=Xs(m),ve=Xs(f);if(!At(v)&&!isNaN(v)){let _e=i.valueAsNumber||v&&+v;At(J.value)||(D=_e>J.value),At(ve.value)||(K=_e<ve.value)}else{let _e=i.valueAsDate||new Date(v),Ye=kt=>new Date(new Date().toDateString()+" "+kt),_t=i.type=="time",ut=i.type=="week";Ia(J.value)&&v&&(D=_t?Ye(v)>Ye(J.value):ut?v>J.value:_e>new Date(J.value)),Ia(ve.value)&&v&&(K=_t?Ye(v)<Ye(ve.value):ut?v<ve.value:_e<new Date(ve.value))}if((D||K)&&(I(!!D,J.message,ve.message,bn.max,bn.min),!n))return w(S[y].message),S}if((c||d)&&!M&&(Ia(v)||s&&Array.isArray(v))){let D=Xs(c),K=Xs(d),J=!At(D.value)&&v.length>+D.value,ve=!At(K.value)&&v.length<+K.value;if((J||ve)&&(I(J,D.message,K.message),!n))return w(S[y].message),S}if(p&&!M&&Ia(v)){let{value:D,message:K}=Xs(p);if(vc(D)&&!v.match(D)&&(S[y]={type:bn.pattern,message:K,ref:i,...P(bn.pattern,K)},!n))return w(K),S}if(b){if(_a(b)){let D=await b(v,a),K=w1(D,x);if(K&&(S[y]={...K,...P(bn.validate,K.message)},!n))return w(K.message),S}else if(Ge(b)){let D={};for(let K in b){if(!Kt(D)&&!n)break;let J=w1(await b[K](v,a),x,K);J&&(D={...J,...P(K,J.message)},w(J.message),n&&(S[y]=D))}if(!Kt(D)&&(S[y]={ref:x,...D},!n))return S}}return w(!0),S},P4={mode:Na.onSubmit,reValidateMode:Na.onChange,shouldFocusError:!0};function j4(e={}){let t={...P4,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:_a(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Ge(t.defaultValues)||Ge(t.values)?ht(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:ht(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},u,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},f={...d},m={array:f1(),state:f1()},p=t.criteriaMode===Na.all,b=N=>C=>{clearTimeout(c),c=setTimeout(N,C)},y=async N=>{if(!t.disabled&&(d.isValid||f.isValid||N)){let C=t.resolver?Kt((await _()).errors):await M(n,!0);C!==a.isValid&&m.state.next({isValid:C})}},$=(N,C)=>{!t.disabled&&(d.isValidating||d.validatingFields||f.isValidating||f.validatingFields)&&((N||Array.from(o.mount)).forEach(O=>{O&&(C?Ue(a.validatingFields,O,C):Ze(a.validatingFields,O))}),m.state.next({validatingFields:a.validatingFields,isValidating:!Kt(a.validatingFields)}))},g=(N,C=[],O,B,q=!0,F=!0)=>{if(B&&O&&!t.disabled){if(i.action=!0,F&&Array.isArray(V(n,N))){let Z=O(V(n,N),B.argA,B.argB);q&&Ue(n,N,Z)}if(F&&Array.isArray(V(a.errors,N))){let Z=O(V(a.errors,N),B.argA,B.argB);q&&Ue(a.errors,N,Z),O4(a.errors,N)}if((d.touchedFields||f.touchedFields)&&F&&Array.isArray(V(a.touchedFields,N))){let Z=O(V(a.touchedFields,N),B.argA,B.argB);q&&Ue(a.touchedFields,N,Z)}(d.dirtyFields||f.dirtyFields)&&(a.dirtyFields=Bo(r,s)),m.state.next({name:N,isDirty:I(N,C),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else Ue(s,N,C)},v=(N,C)=>{Ue(a.errors,N,C),m.state.next({errors:a.errors})},x=N=>{a.errors=N,m.state.next({errors:a.errors,isValid:!1})},w=(N,C,O,B)=>{let q=V(n,N);if(q){let F=V(s,N,We(O)?V(r,N):O);We(F)||B&&B.defaultChecked||C?Ue(s,N,C?F:g1(q._f)):J(N,F),i.mount&&y()}},S=(N,C,O,B,q)=>{let F=!1,Z=!1,ye={name:N};if(!t.disabled){if(!O||B){(d.isDirty||f.isDirty)&&(Z=a.isDirty,a.isDirty=ye.isDirty=I(),F=Z!==ye.isDirty);let Ce=Wn(V(r,N),C);Z=!!V(a.dirtyFields,N),Ce?Ze(a.dirtyFields,N):Ue(a.dirtyFields,N,!0),ye.dirtyFields=a.dirtyFields,F=F||(d.dirtyFields||f.dirtyFields)&&Z!==!Ce}if(O){let Ce=V(a.touchedFields,N);Ce||(Ue(a.touchedFields,N,O),ye.touchedFields=a.touchedFields,F=F||(d.touchedFields||f.touchedFields)&&Ce!==O)}F&&q&&m.state.next(ye)}return F?ye:{}},R=(N,C,O,B)=>{let q=V(a.errors,N),F=(d.isValid||f.isValid)&&Ka(C)&&a.isValid!==C;if(t.delayError&&O?(u=b(()=>v(N,O)),u(t.delayError)):(clearTimeout(c),u=null,O?Ue(a.errors,N,O):Ze(a.errors,N)),(O?!Wn(q,O):q)||!Kt(B)||F){let Z={...B,...F&&Ka(C)?{isValid:C}:{},errors:a.errors,name:N};a={...a,...Z},m.state.next(Z)}},_=async N=>{$(N,!0);let C=await t.resolver(s,t.context,C4(N||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return $(N),C},E=async N=>{let{errors:C}=await _(N);if(N)for(let O of N){let B=V(C,O);B?Ue(a.errors,O,B):Ze(a.errors,O)}else a.errors=C;return C},M=async(N,C,O={valid:!0})=>{for(let B in N){let q=N[B];if(q){let{_f:F,...Z}=q;if(F){let ye=o.array.has(F.name),Ce=q._f&&E4(q._f);Ce&&d.validatingFields&&$([B],!0);let sa=await S1(q,o.disabled,s,p,t.shouldUseNativeValidation&&!C,ye);if(Ce&&d.validatingFields&&$([B]),sa[F.name]&&(O.valid=!1,C))break;!C&&(V(sa,F.name)?ye?L4(a.errors,sa,F.name):Ue(a.errors,F.name,sa[F.name]):Ze(a.errors,F.name))}!Kt(Z)&&await M(Z,C,O)}}return O.valid},P=()=>{for(let N of o.unMount){let C=V(n,N);C&&(C._f.refs?C._f.refs.every(O=>!Lp(O)):!Lp(C._f.ref))&&ne(N)}o.unMount=new Set},I=(N,C)=>!t.disabled&&(N&&C&&Ue(s,N,C),!Wn(kt(),r)),D=(N,C,O)=>S4(N,o,{...i.mount?s:We(C)?r:Ia(N)?{[N]:C}:C},O,C),K=N=>Up(V(i.mount?s:r,N,t.shouldUnregister?V(r,N,[]):[])),J=(N,C,O={})=>{let B=V(n,N),q=C;if(B){let F=B._f;F&&(!F.disabled&&Ue(s,N,E1(C,F)),q=pc(F.ref)&&At(C)?"":C,_1(F.ref)?[...F.ref.options].forEach(Z=>Z.selected=q.includes(Z.value)):F.refs?Qo(F.ref)?F.refs.forEach(Z=>{(!Z.defaultChecked||!Z.disabled)&&(Array.isArray(q)?Z.checked=!!q.find(ye=>ye===Z.value):Z.checked=q===Z.value||!!q)}):F.refs.forEach(Z=>Z.checked=Z.value===q):zp(F.ref)?F.ref.value="":(F.ref.value=q,F.ref.type||m.state.next({name:N,values:ht(s)})))}(O.shouldDirty||O.shouldTouch)&&S(N,q,O.shouldTouch,O.shouldDirty,!0),O.shouldValidate&&ut(N)},ve=(N,C,O)=>{for(let B in C){if(!C.hasOwnProperty(B))return;let q=C[B],F=N+"."+B,Z=V(n,F);(o.array.has(N)||Ge(q)||Z&&!Z._f)&&!jr(q)?ve(F,q,O):J(F,q,O)}},_e=(N,C,O={})=>{let B=V(n,N),q=o.array.has(N),F=ht(C);Ue(s,N,F),q?(m.array.next({name:N,values:ht(s)}),(d.isDirty||d.dirtyFields||f.isDirty||f.dirtyFields)&&O.shouldDirty&&m.state.next({name:N,dirtyFields:Bo(r,s),isDirty:I(N,F)})):B&&!B._f&&!At(F)?ve(N,F,O):J(N,F,O),x1(N,o)&&m.state.next({...a,name:N}),m.state.next({name:i.mount?N:void 0,values:ht(s)})},Ye=async N=>{i.mount=!0;let C=N.target,O=C.name,B=!0,q=V(n,O),F=Ce=>{B=Number.isNaN(Ce)||jr(Ce)&&isNaN(Ce.getTime())||Wn(Ce,V(s,O,Ce))},Z=y1(t.mode),ye=y1(t.reValidateMode);if(q){let Ce,sa,el=C.type?g1(q._f):v4(N),wn=N.type===m1.BLUR||N.type===m1.FOCUS_OUT,dk=!T4(q._f)&&!t.resolver&&!V(a.errors,O)&&!q._f.deps||M4(wn,V(a.touchedFields,O),a.isSubmitted,ye,Z),td=x1(O,o,wn);Ue(s,O,el),wn?(!C||!C.readOnly)&&(q._f.onBlur&&q._f.onBlur(N),u&&u(0)):q._f.onChange&&q._f.onChange(N);let ad=S(O,el,wn),mk=!Kt(ad)||td;if(!wn&&m.state.next({name:O,type:N.type,values:ht(s)}),dk)return(d.isValid||f.isValid)&&(t.mode==="onBlur"?wn&&y():wn||y()),mk&&m.state.next({name:O,...td?{}:ad});if(!wn&&td&&m.state.next({...a}),t.resolver){let{errors:_h}=await _([O]);if(F(el),B){let fk=$1(a.errors,n,O),kh=$1(_h,n,fk.name||O);Ce=kh.error,O=kh.name,sa=Kt(_h)}}else $([O],!0),Ce=(await S1(q,o.disabled,s,p,t.shouldUseNativeValidation))[O],$([O]),F(el),B&&(Ce?sa=!1:(d.isValid||f.isValid)&&(sa=await M(n,!0)));B&&(q._f.deps&&ut(q._f.deps),R(O,sa,Ce,ad))}},_t=(N,C)=>{if(V(a.errors,C)&&N.focus)return N.focus(),1},ut=async(N,C={})=>{let O,B,q=Ko(N);if(t.resolver){let F=await E(We(N)?N:q);O=Kt(F),B=N?!q.some(Z=>V(F,Z)):O}else N?(B=(await Promise.all(q.map(async F=>{let Z=V(n,F);return await M(Z&&Z._f?{[F]:Z}:Z)}))).every(Boolean),!(!B&&!a.isValid)&&y()):B=O=await M(n);return m.state.next({...!Ia(N)||(d.isValid||f.isValid)&&O!==a.isValid?{}:{name:N},...t.resolver||!N?{isValid:O}:{},errors:a.errors}),C.shouldFocus&&!B&&Io(n,_t,N?q:o.mount),B},kt=N=>{let C={...i.mount?s:r};return We(N)?C:Ia(N)?V(C,N):N.map(O=>V(C,O))},Ga=(N,C)=>({invalid:!!V((C||a).errors,N),isDirty:!!V((C||a).dirtyFields,N),error:V((C||a).errors,N),isValidating:!!V(a.validatingFields,N),isTouched:!!V((C||a).touchedFields,N)}),$n=N=>{N&&Ko(N).forEach(C=>Ze(a.errors,C)),m.state.next({errors:N?a.errors:{}})},Ca=(N,C,O)=>{let B=(V(n,N,{_f:{}})._f||{}).ref,q=V(a.errors,N)||{},{ref:F,message:Z,type:ye,...Ce}=q;Ue(a.errors,N,{...Ce,...C,ref:B}),m.state.next({name:N,errors:a.errors,isValid:!1}),O&&O.shouldFocus&&B&&B.focus&&B.focus()},Ya=(N,C)=>_a(N)?m.state.subscribe({next:O=>"values"in O&&N(D(void 0,C),O)}):D(N,C,!0),at=N=>m.state.subscribe({next:C=>{D4(N.name,C.name,N.exact)&&A4(C,N.formState||d,ee,N.reRenderRoot)&&N.callback({values:{...s},...a,...C,defaultValues:r})}}).unsubscribe,oe=N=>(i.mount=!0,f={...f,...N.formState},at({...N,formState:f})),ne=(N,C={})=>{for(let O of N?Ko(N):o.mount)o.mount.delete(O),o.array.delete(O),C.keepValue||(Ze(n,O),Ze(s,O)),!C.keepError&&Ze(a.errors,O),!C.keepDirty&&Ze(a.dirtyFields,O),!C.keepTouched&&Ze(a.touchedFields,O),!C.keepIsValidating&&Ze(a.validatingFields,O),!t.shouldUnregister&&!C.keepDefaultValue&&Ze(r,O);m.state.next({values:ht(s)}),m.state.next({...a,...C.keepDirty?{isDirty:I()}:{}}),!C.keepIsValid&&y()},$e=({disabled:N,name:C})=>{(Ka(N)&&i.mount||N||o.disabled.has(C))&&(N?o.disabled.add(C):o.disabled.delete(C))},ge=(N,C={})=>{let O=V(n,N),B=Ka(C.disabled)||Ka(t.disabled);return Ue(n,N,{...O||{},_f:{...O&&O._f?O._f:{ref:{name:N}},name:N,mount:!0,...C}}),o.mount.add(N),O?$e({disabled:Ka(C.disabled)?C.disabled:t.disabled,name:N}):w(N,!0,C.value),{...B?{disabled:C.disabled||t.disabled}:{},...t.progressive?{required:!!C.required,min:Ho(C.min),max:Ho(C.max),minLength:Ho(C.minLength),maxLength:Ho(C.maxLength),pattern:Ho(C.pattern)}:{},name:N,onChange:Ye,onBlur:Ye,ref:q=>{if(q){ge(N,C),O=V(n,N);let F=We(q.value)&&q.querySelectorAll&&q.querySelectorAll("input,select,textarea")[0]||q,Z=_4(F),ye=O._f.refs||[];if(Z?ye.find(Ce=>Ce===F):F===O._f.ref)return;Ue(n,N,{_f:{...O._f,...Z?{refs:[...ye.filter(Lp),F,...Array.isArray(V(r,N))?[{}]:[]],ref:{type:F.type,name:N}}:{ref:F}}}),w(N,!1,void 0,F)}else O=V(n,N,{}),O._f&&(O._f.mount=!1),(t.shouldUnregister||C.shouldUnregister)&&!(y4(o.array,N)&&i.action)&&o.unMount.add(N)}}},nt=()=>t.shouldFocusError&&Io(n,_t,o.mount),Ke=N=>{Ka(N)&&(m.state.next({disabled:N}),Io(n,(C,O)=>{let B=V(n,O);B&&(C.disabled=B._f.disabled||N,Array.isArray(B._f.refs)&&B._f.refs.forEach(q=>{q.disabled=B._f.disabled||N}))},0,!1))},Fe=(N,C)=>async O=>{let B;O&&(O.preventDefault&&O.preventDefault(),O.persist&&O.persist());let q=ht(s);if(m.state.next({isSubmitting:!0}),t.resolver){let{errors:F,values:Z}=await _();a.errors=F,q=ht(Z)}else await M(n);if(o.disabled.size)for(let F of o.disabled)Ze(q,F);if(Ze(a.errors,"root"),Kt(a.errors)){m.state.next({errors:{}});try{await N(q,O)}catch(F){B=F}}else C&&await C({...a.errors},O),nt(),setTimeout(nt);if(m.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:Kt(a.errors)&&!B,submitCount:a.submitCount+1,errors:a.errors}),B)throw B},ga=(N,C={})=>{V(n,N)&&(We(C.defaultValue)?_e(N,ht(V(r,N))):(_e(N,C.defaultValue),Ue(r,N,ht(C.defaultValue))),C.keepTouched||Ze(a.touchedFields,N),C.keepDirty||(Ze(a.dirtyFields,N),a.isDirty=C.defaultValue?I(N,ht(V(r,N))):I()),C.keepError||(Ze(a.errors,N),d.isValid&&y()),m.state.next({...a}))},Mt=(N,C={})=>{let O=N?ht(N):r,B=ht(O),q=Kt(N),F=q?r:B;if(C.keepDefaultValues||(r=O),!C.keepValues){if(C.keepDirtyValues){let Z=new Set([...o.mount,...Object.keys(Bo(r,s))]);for(let ye of Array.from(Z))V(a.dirtyFields,ye)?Ue(F,ye,V(s,ye)):_e(ye,V(F,ye))}else{if(jp&&We(N))for(let Z of o.mount){let ye=V(n,Z);if(ye&&ye._f){let Ce=Array.isArray(ye._f.refs)?ye._f.refs[0]:ye._f.ref;if(pc(Ce)){let sa=Ce.closest("form");if(sa){sa.reset();break}}}}if(C.keepFieldsRef)for(let Z of o.mount)_e(Z,V(F,Z));else n={}}s=t.shouldUnregister?C.keepDefaultValues?ht(r):{}:ht(F),m.array.next({values:{...F}}),m.state.next({values:{...F}})}o={mount:C.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!C.keepIsValid||!!C.keepDirtyValues,i.watch=!!t.shouldUnregister,m.state.next({submitCount:C.keepSubmitCount?a.submitCount:0,isDirty:q?!1:C.keepDirty?a.isDirty:!!(C.keepDefaultValues&&!Wn(N,r)),isSubmitted:C.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:q?{}:C.keepDirtyValues?C.keepDefaultValues&&s?Bo(r,s):a.dirtyFields:C.keepDefaultValues&&N?Bo(r,N):C.keepDirty?a.dirtyFields:{},touchedFields:C.keepTouched?a.touchedFields:{},errors:C.keepErrors?a.errors:{},isSubmitSuccessful:C.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},ya=(N,C)=>Mt(_a(N)?N(s):N,C),ke=(N,C={})=>{let O=V(n,N),B=O&&O._f;if(B){let q=B.refs?B.refs[0]:B.ref;q.focus&&(q.focus(),C.shouldSelect&&_a(q.select)&&q.select())}},ee=N=>{a={...a,...N}},bt={control:{register:ge,unregister:ne,getFieldState:Ga,handleSubmit:Fe,setError:Ca,_subscribe:at,_runSchema:_,_focusError:nt,_getWatch:D,_getDirty:I,_setValid:y,_setFieldArray:g,_setDisabledField:$e,_setErrors:x,_getFieldArray:K,_reset:Mt,_resetDefaultValues:()=>_a(t.defaultValues)&&t.defaultValues().then(N=>{ya(N,t.resetOptions),m.state.next({isLoading:!1})}),_removeUnmounted:P,_disableForm:Ke,_subjects:m,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(N){i=N},get _defaultValues(){return r},get _names(){return o},set _names(N){o=N},get _formState(){return a},get _options(){return t},set _options(N){t={...t,...N}}},subscribe:oe,trigger:ut,register:ge,handleSubmit:Fe,watch:Ya,setValue:_e,getValues:kt,reset:ya,resetField:ga,clearErrors:$n,unregister:ne,setError:Ca,setFocus:ke,getFieldState:Ga};return{...bt,formControl:bt}}function A1(e={}){let t=Ht.default.useRef(void 0),a=Ht.default.useRef(void 0),[n,r]=Ht.default.useState({isDirty:!1,isValidating:!1,isLoading:_a(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:_a(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!_a(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=j4(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,w4(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),Ht.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),Ht.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),Ht.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),Ht.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),Ht.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),Ht.default.useEffect(()=>{e.values&&!Wn(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),Ht.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=$4(n,s),t.current}var D1={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},M1={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},U4={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function te({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return l`
    <${s}
      className=${H(D1[a]??D1.default,M1[n]??M1.md,U4[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var Bp="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",yc={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function Dt({className:e="",size:t="md",error:a=!1,...n}){return l`
    <input
      className=${H(Bp,yc[t]??yc.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function bc({className:e="",error:t=!1,rows:a=4,...n}){return l`
    <textarea
      rows=${a}
      className=${H(Bp,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Hp({children:e,className:t="",size:a="md",error:n=!1,...r}){return l`
    <div className="relative w-full">
      <select
        className=${H(Bp,yc[a]??yc.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
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
  `}function F4({children:e,className:t="",required:a=!1,...n}){return l`
    <label
      className=${H("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&l`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function xn({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return l`
    <div className=${H("flex flex-col gap-2",s)}>
      ${e&&l`<${F4} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&l`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&l`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var z4={google:"Google",github:"GitHub",apple:"Apple"};function q4(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function O1({providers:e,redirectAfter:t}){let a=k();return e.length?l`
    <div className="mt-6 space-y-3">
      <div className="flex items-center gap-3 text-[11px] uppercase text-[var(--v2-text-faint)]">
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
        <span>${a("login.oauthDivider")}</span>
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
      </div>
      <div className="grid gap-2">
        ${e.map(n=>l`
            <${T}
              key=${n}
              as="a"
              href=${q4(n,t)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <${A} name="shield" className="h-4 w-4" />
              ${a("login.oauthProvider",{provider:z4[n]||n})}
            <//>
          `)}
      </div>
    </div>
  `:null}var B4=["google","github","apple"];function L1(){let[e,t]=h.default.useState([]);return h.default.useEffect(()=>{let a=!1;return Rx().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(B4.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function P1({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=k(),{theme:s,toggleTheme:i}=ic(),o=L1(),{formState:{errors:u,isSubmitting:c},handleSubmit:d,register:f}=A1({defaultValues:{token:e||""}});return l`
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
        <${A} name=${s==="dark"?"sun":"moon"} className="h-4 w-4" />
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
          onSubmit=${d(({token:m})=>n(m))}
        >
          <${xn}
            label=${r("login.tokenLabel")}
            htmlFor="v2-token"
            error=${u.token?.message??""}
            hint=${r("login.tokenHint")}
          >
            <${Dt}
              id="v2-token"
              type="password"
              error=${!!u.token}
              ...${f("token",{required:r("login.tokenRequired"),setValueAs:m=>m.trim()})}
              placeholder=${r("login.tokenPlaceholder")}
              autocomplete="current-password"
            />
          <//>

          ${t&&l`<p
              className=${H("rounded-[10px] border px-3 py-2 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
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

        <${O1}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var j1={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},U1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function j({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return l`
    <span
      className=${H("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",U1[n]??U1.md,j1[e]??j1.muted,r)}
    >
      ${a&&l`<span
          className=${H("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var H4=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,F1=/(bash|shell|exec|run|command|terminal|spawn|process)/,z1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function q1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return H4.test(n)?{tone:"danger",key:"tool.riskWrite"}:F1.test(n)?{tone:"warning",key:"tool.riskExec"}:z1.test(n)?{tone:"info",key:"tool.riskNetwork"}:F1.test(r)?{tone:"warning",key:"tool.riskExec"}:z1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}var xc=480;function K4(e,t){return t&&t.length>0?t.some(a=>typeof a?.value=="string"&&a.value.length>xc):typeof e=="string"&&e.length>xc}function B1(e,t){return typeof e!="string"||t||e.length<=xc?e:`${e.slice(0,xc).trimEnd()}
...`}function H1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=k(),{toolName:s,description:i,parameters:o,allowAlways:u,approvalDetails:c=[]}=e,[d,f]=h.default.useState(!1),[m,p]=h.default.useState(!1);h.default.useEffect(()=>{p(!1)},[e]);let b=h.default.useMemo(()=>q1(s,i,o),[s,i,o]),y=s||r("approval.thisTool"),$=K4(o,c),g=m?"max-h-72":"max-h-36",v=h.default.useCallback(()=>{d&&u?n?.():t?.()},[d,u,n,t]);return l`
    <div className="mx-auto max-w-lg rounded-xl border border-copper/30 bg-copper/10 p-4">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-copper/25 bg-copper/10 text-copper">
          <${A} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">${r("approval.title")}</span>
        <${j}
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
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${B1(x.value,m)}</dd>
                  </div>
                `)}
            </dl>
          `:o&&l`<pre className=${`mb-2 ${g} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${B1(o,m)}</pre>`}

      ${$&&l`
        <${T}
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
        <${T} variant="primary" onClick=${v}>
          ${r(d&&u?"approval.approveAndAlways":"approval.approve")}
        <//>
        <${T} variant="secondary" onClick=${()=>a?.()}>
          ${r("approval.deny")}
        <//>
      </div>
    </div>
  `}function Zs({icon:e="lock",headline:t,provider:a,accountLabel:n,body:r,expiresAt:s,pillHint:i,defaultExpanded:o=!0,children:u}){let c=k(),[d,f]=h.default.useState(o),m=h.default.useId(),p=n||a||"";return l`
    <div className="mx-auto w-full max-w-lg rounded-xl border border-[rgba(76,167,230,0.34)] bg-[rgba(76,167,230,0.08)]">
      <button
        type="button"
        onClick=${()=>f(b=>!b)}
        aria-expanded=${d?"true":"false"}
        aria-controls=${m}
        className="flex w-full items-center gap-3 rounded-xl border-0 bg-transparent px-4 py-3 text-left"
      >
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-md border border-[rgba(76,167,230,0.28)] bg-[rgba(76,167,230,0.1)] text-[#8fc8f2]">
          <${A} name=${e} className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold text-white">
            ${t||c("authGate.title")}
          </span>
          ${p&&l`<span className="block truncate text-xs text-iron-300">${p}</span>`}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 text-xs font-medium text-[#8fc8f2]">
          ${i&&l`<span className="hidden sm:inline">${i}</span>`}
          <${A}
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
  `}function K1({gate:e,onCancel:t}){let a=k();return l`
    <${Zs}
      icon="lock"
      headline=${e?.headline||a("authGate.title")}
      body=${e?.body||""}
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
  `}function I1({gate:e,onCancel:t}){let a=k(),[n,r]=h.default.useState(!1),[s,i]=h.default.useState(""),o=h.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]);h.default.useEffect(()=>{i("")},[e.authorizationUrl,e.gateRef,e.runId]);let u=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),c=h.default.useCallback(()=>{if(!o){i(a("authGate.serviceUnavailable"));return}i(""),window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0)},[e.authorizationUrl,o]),d=n?a("authGate.reopenAuthorization",{provider:u}):a("authGate.openAuthorization",{provider:u});return l`
    <${Zs}
      icon="link"
      headline=${e?.headline||a("authGate.oauthTitle")}
      provider=${e?.provider?u:""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      expiresAt=${e?.expiresAt||""}
      pillHint=${a("authGate.pillAuthorize")}
    >
      <div className="flex flex-wrap gap-2">
        <${T}
          as="a"
          href=${o?e.authorizationUrl:void 0}
          target="_blank"
          rel="noopener noreferrer"
          className="auth-oauth"
          variant="primary"
          onClick=${f=>{f.preventDefault(),c()}}
        >
          <${A} name="link" className="h-4 w-4" />
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
  `}function Q1({gate:e,onSubmit:t,onCancel:a}){let n=k(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState(!1),d=h.default.useCallback(async f=>{f.preventDefault();let m=r.trim();if(!m){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(m),s("")}catch(p){o(p?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return l`
    <${Zs}
      icon="lock"
      headline=${e?.headline||n("authGate.title")}
      provider=${e?.provider||""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      pillHint=${n("authGate.pillEnterToken")}
    >
      <form onSubmit=${d}>
        <div className="mb-3">
          <${Dt}
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
          <${T} type="submit" variant="primary" disabled=${u}>
            ${n(u?"authGate.submitting":"authGate.submit")}
          <//>
          <${T}
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
  `}var I4="/api/webchat/v2/extensions/pairing/redeem";function V1(e){return Y(I4,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function $c({action:e}){let t=k(),a=X(),n=Q({mutationFn:({code:u})=>V1(u),onSuccess:()=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=h.default.useState(""),i=Q4(e,t),o=()=>{let u=r.trim();u&&(n.mutate({code:u}),s(""))};return l`
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
        <${T}
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
        ${V4(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function Q4(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function V4(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function G4(e,t){return e?.channel==="slack"&&e.strategy===t}function G1({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return l`
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
            <${A} name="close" className="h-4 w-4" />
          </button>
        `}
      </div>

      ${G4(e,"inbound_proof_code")?l`<${$c} action=${e.action} />`:l`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function Y4(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):Mr.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:Mr.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:Mr.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:Mr.maxTotalBytes}:Mr}function Y1(){let e=va(),t=z({enabled:!!e,queryKey:["session"],queryFn:Wu,staleTime:5*6e4});return Y4(t.data)}function wc({onSend:e,onCancel:t,disabled:a,canCancel:n=!1,initialText:r="",resetKey:s="",draftKey:i=jo,variant:o="dock",context:u={},statusText:c=""}){let d=k(),f=o==="hero",m=Y1(),[p,b]=h.default.useState(()=>_p(i)),[y,$]=h.default.useState(()=>Rp(i)),[g,v]=h.default.useState(""),[x,w]=h.default.useState(!1),[S,R]=h.default.useState(!1),[_,E]=h.default.useState(!1),M=h.default.useRef(null),P=h.default.useRef(null),I=h.default.useRef([]),D=h.default.useRef(Promise.resolve());h.default.useEffect(()=>{I.current=y},[y]);let K=h.default.useRef(null),J=h.default.useRef(null),ve=h.default.useCallback(()=>{J.current&&(window.clearTimeout(J.current),J.current=null);let ee=K.current;K.current=null,ee&&ee.scope===St()&&kp(ee.key,ee.text)},[]),_e=h.default.useCallback(()=>{J.current&&(window.clearTimeout(J.current),J.current=null),K.current=null},[]),Ye=h.default.useCallback(()=>{let ee=M.current;ee&&(ee.style.height="auto",ee.style.height=`${Math.min(ee.scrollHeight,200)}px`)},[]);h.default.useEffect(()=>{Ye()},[p,Ye]),h.default.useEffect(()=>(b(_p(i)),()=>ve()),[i,ve]);let _t=h.default.useRef(i);h.default.useEffect(()=>{if(_t.current!==i){_t.current=i,$(Rp(i)),v("");return}Wx(i,y)},[i,y]),h.default.useEffect(()=>{r&&(b(r),window.requestAnimationFrame(()=>{M.current&&(M.current.focus(),M.current.setSelectionRange(r.length,r.length))}))},[r,s]);let ut=h.default.useCallback(ee=>{a||!ee||ee.length===0||(D.current=D.current.then(async()=>{let{staged:Re,errors:bt}=await qx(ee,{limits:m,existing:I.current,t:d});Re.length>0&&$(N=>{let C=[...N,...Re];return I.current=C,C}),v(bt.length>0?bt.join(" "):"")}).catch(()=>{v(d("chat.attachmentStagingFailed"))}))},[a,m,d]),kt=h.default.useCallback(ee=>{$(Re=>{let bt=Re.filter(N=>N.id!==ee);return I.current=bt,bt}),v("")},[]),Ga=h.default.useCallback(()=>{a||P.current?.click()},[a]),$n=h.default.useCallback(ee=>{let Re=Array.from(ee.target.files||[]);ut(Re),ee.target.value=""},[ut]),Ca=h.default.useCallback(async()=>{if(!(!p.trim()||a||x)){w(!0);try{await e(p.trim(),{attachments:y}),b(""),$([]),I.current=[],v(""),_e(),Zx(i),e$(i),M.current&&(M.current.style.height="auto")}catch{}finally{w(!1)}}},[p,y,a,x,e,i,_e]),Ya=h.default.useCallback(ee=>{let Re=ee.target.value;b(Re),K.current={key:i,text:Re,scope:St()},J.current&&window.clearTimeout(J.current),J.current=window.setTimeout(ve,300)},[i,ve]),at=h.default.useCallback(async()=>{if(!(!n||S||!t)){R(!0);try{await t()}finally{R(!1)}}},[n,S,t]),oe=h.default.useCallback(ee=>{ee.key==="Enter"&&!ee.shiftKey&&(ee.preventDefault(),Ca())},[Ca]),ne=h.default.useCallback(ee=>{let Re=Array.from(ee.clipboardData?.files||[]);Re.length>0&&(ee.preventDefault(),ut(Re))},[ut]),$e=h.default.useCallback(ee=>{ee.preventDefault(),E(!1);let Re=Array.from(ee.dataTransfer?.files||[]);Re.length>0&&ut(Re)},[ut]),ge=h.default.useCallback(ee=>{ee.preventDefault(),!a&&E(!0)},[a]),nt=h.default.useCallback(ee=>{ee.currentTarget.contains(ee.relatedTarget)||E(!1)},[]),Ke=p.trim(),Fe=d(f?"chat.heroPlaceholder":"chat.followUpPlaceholder"),ga=m.accept.length>0?m.accept.join(","):void 0,Mt=f?"w-full":"px-4 py-3 sm:px-5 lg:px-8",ya=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",f?"min-h-[120px]":"",a?"opacity-70":""].join(" "),ke=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",f?"min-h-[72px]":"min-h-[40px]"].join(" ");return l`
    <div className=${Mt}>
      <div
        className=${ya}
        onDrop=${$e}
        onDragOver=${ge}
        onDragLeave=${nt}
      >
        ${_&&l`
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
              <${A} name="close" className="h-3.5 w-3.5" strokeWidth=${2} />
            </button>
          </div>
        `}

        ${y.length>0&&l`
          <div className="mb-2 flex flex-wrap gap-2 px-1">
            ${y.map(ee=>l`
                <div
                  key=${ee.id}
                  className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
                >
                  ${ee.previewUrl?l`<img
                        src=${ee.previewUrl}
                        alt=${ee.filename}
                        className="h-9 w-9 shrink-0 rounded object-cover"
                      />`:l`<span
                        className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                      >
                        <${A} name="file" className="h-4 w-4" />
                      </span>`}
                  <span className="flex min-w-0 flex-col">
                    <span className="max-w-[12rem] truncate font-medium">
                      ${ee.filename}
                    </span>
                    <span className="text-[10px] text-iron-400">${ee.sizeLabel}</span>
                  </span>
                  <button
                    type="button"
                    onClick=${()=>kt(ee.id)}
                    aria-label=${d("chat.attachmentRemove")}
                    title=${d("chat.attachmentRemove")}
                    className="absolute right-1 top-1 grid h-5 w-5 place-items-center rounded-full text-iron-400 hover:bg-iron-700 hover:text-white"
                  >
                    <${A} name="close" className="h-3 w-3" />
                  </button>
                </div>
              `)}
          </div>
        `}

        <textarea
          ref=${M}
          data-testid="chat-composer"
          value=${p}
          onChange=${Ya}
          onKeyDown=${oe}
          onPaste=${ne}
          placeholder=${Fe}
          rows=${1}
          disabled=${a}
          className=${ke}
        />

        <input
          ref=${P}
          type="file"
          multiple
          accept=${ga}
          className="hidden"
          onChange=${$n}
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
              onClick=${Ga}
              disabled=${a}
              aria-label=${d("chat.attachFiles")}
              title=${d("chat.attachFiles")}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <${A} name="plus" className="h-5 w-5" />
            </button>
            ${n?l`
                <${T}
                  type="button"
                  variant="danger"
                  size="icon-sm"
                  onClick=${at}
                  disabled=${S}
                  aria-label=${d("common.cancel")}
                  title=${d("common.cancel")}
                  className="rounded-full"
                >
                  <${A} name="close" className="h-5 w-5" />
                <//>
              `:l`
                <${T}
                  type="button"
                  variant="primary"
                  size="icon-sm"
                  onClick=${Ca}
                  disabled=${a||x||!Ke}
                  aria-label=${d("chat.send")}
                  className="rounded-full"
                >
                  <${A} name="send" className="h-5 w-5" />
                <//>
              `}
          </div>
        </div>
      </div>
    </div>
  `}var J1={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function X1({status:e}){let t=k();if(e==="idle"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return l`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",J1[e]||J1.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function Z1({onSuggestion:e,onSend:t,disabled:a,initialText:n,resetKey:r,draftKey:s,context:i,statusText:o,canCancel:u,onCancel:c}){let d=k(),f=[{icon:"tool",title:d("chat.suggestion1"),detail:d("chat.suggestion1Desc")},{icon:"shield",title:d("chat.suggestion2"),detail:d("chat.suggestion2Desc")},{icon:"plug",title:d("chat.suggestion3"),detail:d("chat.suggestion3Desc")}];return l`
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
        <${wc}
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
                <${A} name=${m.icon} className="h-4 w-4" />
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
  `}var J4=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function W1({open:e,onClose:t}){let a=k();return e?l`
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
            <${A} name="bolt" className="h-4 w-4" />
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
            <${A} name="close" className="h-4 w-4" />
          </button>
        </div>
        <ul className="flex flex-col gap-2">
          ${J4.map((n,r)=>l`
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
  `:null}function tw(e){let t=0,a=0,n=0,r=0;for(let i of e){if(i.role==="thinking"&&(t+=1),i.role==="tool_activity"){let o=ew([i]);a+=o.tools,n+=o.failed,r+=o.running}if(X4(i)){let o=ew(i.toolCalls);a+=o.tools,n+=o.failed,r+=o.running}}let s=[];return t&&s.push(`${t} reasoning`),a&&s.push(`${a} ${a===1?"tool":"tools"}`),n&&s.push(`${n} failed`),!n&&r&&s.push("running"),{hasError:n>0,label:`Activity${s.length?` - ${s.join(", ")}`:""}`}}function ew(e){let t=0,a=0;for(let n of e)n.toolStatus==="error"&&(t+=1),n.toolStatus==="running"&&(a+=1);return{tools:e.length,failed:t,running:a}}function X4(e){return e.toolCalls&&e.toolCalls.length>0}var aw=!1;function Z4(){aw||!window.DOMPurify||(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),aw=!0)}function nw(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}Z4();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var Kp=360;function W4(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let u=s("Copy");if(u.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),u.textContent="Copied",Ys("Code copied",{tone:"success"}),setTimeout(()=>u.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(u),n.appendChild(r),t.scrollHeight>Kp){t.style.maxHeight=`${Kp}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${Kp}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function eT({content:e,className:t=""}){let a=h.default.useRef(null),n=h.default.useMemo(()=>nw(e),[e]);return h.default.useEffect(()=>{W4(a.current)},[n]),l`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var na=h.default.memo(eT);var rw={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",error:"bg-[var(--v2-danger-text)]"},tT={success:"ok",error:"err",running:"run"},aT=2;function Ws({activity:e}){return e.toolCalls&&e.toolCalls.length>0?l`<${rT} tools=${e.toolCalls} />`:l`<${sT} activity=${e} />`}function nT(e,t){let a=0,n=0,r=0,s=0;for(let u of t){let c=String(u.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function rT({tools:e}){let t=k(),a=e.some(i=>i.toolStatus==="error"),[n,r]=h.default.useState(a);if(h.default.useEffect(()=>{a&&r(!0)},[a]),e.length<=aT)return l`
      <div className="flex flex-col gap-3">
        ${e.map((i,o)=>l`<${Ws}
            key=${i.id||i.callId||`${i.toolName}-${o}`}
            activity=${i}
          />`)}
      </div>
    `;let s=nT(t,e);return l`
    <div className="flex flex-col">
      <button
        type="button"
        onClick=${()=>r(i=>!i)}
        aria-expanded=${n?"true":"false"}
        className=${["v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",a?"text-[var(--v2-danger-text)]":"text-iron-400 hover:text-iron-200"].join(" ")}
      >
        <${A} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${s}</span>
        <${A}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0",n?"rotate-180":""].join(" ")}
        />
      </button>

      ${n&&l`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((i,o)=>l`<${Ws}
              key=${i.id||i.callId||`${i.toolName}-${o}`}
              activity=${i}
            />`)}
        </div>
      `}
    </div>
  `}function sT({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:u}=e,[c,d]=h.default.useState(n==="error");h.default.useEffect(()=>{n==="error"&&d(!0)},[n]);let f=rw[n]||rw.running,m=i!=null,p=h.default.useId(),b=l`
    <button
      type="button"
      onClick=${()=>d(y=>!y)}
      aria-expanded=${c?"true":"false"}
      aria-controls=${p}
      className="v2-button flex w-full items-center gap-2.5 border-0 border-b border-iron-700/40 bg-transparent px-1 py-2 text-left text-sm"
    >
      <span className=${["h-2 w-2 shrink-0 rounded-full",f].join(" ")} />
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-wide text-iron-300"
        >${tT[n]||"run"}</span
      >
      <span className="shrink-0 truncate font-mono text-[13px] font-medium text-iron-100"
        >${a}</span
      >
      ${r&&l`<span className="min-w-0 truncate font-mono text-xs text-iron-400"
        >${r}</span
      >`}
      <span className="ml-auto flex shrink-0 items-center gap-2">
        ${m&&l`<span className="font-mono text-[11px] text-iron-300">${i}ms</span>`}
        <${A}
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
          <${A} name="tool" className="h-4 w-4" />
        </div>
      `}
      <div className=${t?"min-w-0 flex-1":"min-w-0 max-w-[85%] flex-1"}>
        ${b}
        ${c&&l`<${iT}
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
  `}function iT({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=k(),u=h.default.useMemo(()=>{let m=[];return r&&m.push({id:"error",label:o("tool.tabError")}),t&&m.push({id:"details",label:o("tool.tabDetails")}),a&&m.push({id:"params",label:o("tool.tabParameters")}),n&&m.push({id:"result",label:o("tool.tabResult")}),m},[o,r,t,a,n]),[c,d]=h.default.useState(null),f=c&&u.some(m=>m.id===c)?c:u[0]?.id;return h.default.useEffect(()=>{r&&d("error")},[r]),u.length===0?l`
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
        ${f==="result"&&l`<${oT} text=${n} />`}
        ${f==="error"&&l`<pre className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-danger-text)]">${r}</pre>`}
      </div>
    </div>
  `}function oT({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return l`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(lT)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return l`
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
                  >${uT(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?l`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:l`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function lT(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function uT(e){return e==null?"":String(e)}function sw({activity:e}){let t=tw(e),a=mT(e),[n,r]=h.default.useState(a);return h.default.useEffect(()=>{a&&r(!0)},[a]),l`
    <div className="mr-auto flex w-full max-w-[85%] flex-col">
      <button
        type="button"
        onClick=${()=>r(s=>!s)}
        aria-expanded=${n?"true":"false"}
        className=${["v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",t.hasError?"text-[var(--v2-danger-text)]":"text-iron-400 hover:text-iron-200"].join(" ")}
      >
        <${A} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${t.label}</span>
        <${A}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0",n?"rotate-180":""].join(" ")}
        />
      </button>

      ${n&&l`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((s,i)=>l`
            <${cT}
              key=${s.id||`${s.role||"activity"}-${i}`}
              item=${s}
            />
          `)}
        </div>
      `}
    </div>
  `}function cT({item:e}){if(e.role==="thinking")return l`<${dT} content=${e.content} />`;if(e.role==="tool_activity"||Ip(e)){let t=Ip(e)?{id:e.id,toolCalls:e.toolCalls}:e;return l`<${Ws} activity=${t} />`}return null}function dT({content:e}){return e?l`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <${A} name="spark" className="h-4 w-4" />
      </div>
      <div className="min-w-0 max-w-[85%] flex-1 border-l-2 border-white/10 pl-3 text-iron-300">
        <${na} content=${e} className="text-[13px]" />
      </div>
    </div>
  `:null}function Ip(e){return e?.toolCalls&&e.toolCalls.length>0}function mT(e){return(e||[]).some(t=>t?.role==="thinking"||t?.toolStatus==="running"||t?.toolStatus==="error"?!0:Ip(t)?t.toolCalls.some(a=>a?.toolStatus==="running"||a?.toolStatus==="error"):!1)}function Sc(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function fT({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=h.default.useState(()=>t&&e.preview_url||null);return h.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return ac(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?l`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:l`<${A} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var iw="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",ow="px-3 py-2";function Nc({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=h.default.useState(!1),o=h.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await Sa(e.fetch_url);Sc(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),u=l`
    <${fT} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?l`<div
      className=${`${iw} ${ow} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${u}
    </div>`:l`<div className=${`${iw} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${ow} text-left transition-colors hover:bg-iron-900/80`}
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
      <${A} name="download" className="h-3.5 w-3.5" />
    </button>`}
  </div>`}var lw={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function ei({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return h.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),h.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?l`
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
        className=${H("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",lw[n]??lw.md,r)}
      >
        ${a?l`<${Qp} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function Qp({children:e,onClose:t,className:a=""}){return l`
    <div
      className=${H("flex shrink-0 items-center justify-between gap-4","px-5 py-4 md:px-7 md:py-5","border-b border-[var(--v2-panel-border)]",a)}
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
            <${A} name="close" className="h-4 w-4" />
          </button>
        `}
    </div>
  `}function ti({children:e,className:t=""}){return l`
    <div className=${H("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function ai({children:e,className:t=""}){return l`
    <div
      className=${H("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var uw=1e5;function _c({attachment:e,onClose:t}){let a=!!e,[n,r]=h.default.useState("loading"),[s,i]=h.default.useState({}),o=e?zx(e.mime_type):"download";if(h.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return Sa(e.fetch_url).then(async f=>{d=URL.createObjectURL(f);let m={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")m.dataUrl=await vp(f);else if(o==="pdf")m.frameUrl=d;else if(o==="text"){let p=await f.text();m.truncated=p.length>uw,m.text=m.truncated?p.slice(0,uw):p}if(c){URL.revokeObjectURL(d);return}i(m),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let u=e.filename||"attachment";return l`
    <${ei} open=${a} onClose=${t} size="xl">
      <${Qp} onClose=${t}>
        <span className="block truncate">${u}</span>
      <//>
      <${ti} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&l`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&l`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&l`<${pT} mode=${o} view=${s} filename=${u} />`}
      <//>
      <${ai}>
        ${s.downloadUrl&&l`<a
          href=${s.downloadUrl}
          download=${u}
          data-testid="attachment-download"
          className="v2-button inline-flex items-center gap-1.5 rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-200 hover:border-signal/35 hover:text-white"
        >
          <${A} name="download" className="h-3.5 w-3.5" />
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
  `}function pT({mode:e,view:t,filename:a}){switch(e){case"image":return l`<img
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
        <${A} name="file" className="h-10 w-10 text-signal" />
        <div className="text-sm">This file type can't be previewed.</div>
      </div>`}}var hT=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function vT(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function cw(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of vT(e).matchAll(hT)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function dw(e){return e.split("/").filter(Boolean).pop()||e}function mw(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function gT({threadId:e,path:t,onPreview:a}){let[n,r]=h.default.useState({mime_type:"",size_label:""});h.default.useEffect(()=>{let i=!0;return dx({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:mw(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:dw(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:tc({threadId:e,path:t})};return l`<${Nc}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function fw({threadId:e,content:t}){let a=h.default.useMemo(()=>cw(t),[t]),[n,r]=h.default.useState(null);return!e||a.length===0?null:l`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>l`<${gT}
          key=${s}
          threadId=${e}
          path=${s}
          onPreview=${r}
        />`)}
      <${_c}
        attachment=${n}
        onClose=${()=>r(null)}
      />
    </div>
  `}var pw={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function yT(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function bT({content:e}){let[t,a]=h.default.useState(!1);return e?l`
    <div className="flex flex-col items-start">
      <button
        type="button"
        onClick=${()=>a(n=>!n)}
        aria-expanded=${t?"true":"false"}
        className="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent px-1 py-1 text-xs font-medium text-iron-400 hover:text-iron-200"
      >
        <${A} name="spark" className="h-3.5 w-3.5" />
        <span>${t?"Hide reasoning":"Reasoning"}</span>
        <${A}
          name="chevron"
          className=${["h-3 w-3",t?"rotate-180":""].join(" ")}
        />
      </button>
      ${t&&l`
        <div className="mt-1 border-l-2 border-white/10 pl-3 text-iron-300">
          <${na} content=${e} className="text-[13px]" />
        </div>
      `}
    </div>
  `:null}function xT({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:u,status:c,error:d,toolCalls:f,timestamp:m}=e,p=n==="user",[b,y]=h.default.useState(!1),[$,g]=h.default.useState(null),v=h.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),y(!0),Ys("Copied to clipboard",{tone:"success"}),setTimeout(()=>y(!1),1400)}catch{}},[r]);if(n==="tool_activity"||f&&f.length>0){let E=f&&f.length>0?{id:e.id,toolCalls:f}:e;return l`<${Ws} activity=${E} />`}if(n==="thinking")return l`<${bT} content=${r} />`;if(n==="image")return l`
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
    `;let x=yT(m),w=(n==="assistant"||n==="user")&&!u,R=p?"max-w-[85%]":n==="system"||n==="error"?"mx-auto max-w-[85%]":"w-full max-w-[85%]",_=p?"":"w-full min-w-0 max-w-full";return l`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",p?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col gap-2",R].join(" ")}>
        <div
          className=${["text-base leading-7",_,pw[n]||pw.assistant,u?"opacity-70":""].join(" ")}
        >
          ${n==="assistant"||n==="system"||n==="error"?l`<${na} content=${r} />`:l`<div className="whitespace-pre-wrap">${r}</div>`}

          ${c==="error"&&l`
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-300">
              <span>${d}</span>
            </div>
          `}

          ${s&&s.length>0&&l`
            <div className="mt-2 flex flex-wrap gap-2">
              ${s.map((E,M)=>l`<img key=${M} src=${E} className="max-h-48 rounded-lg border border-iron-700 object-cover" alt="Message attachment" />`)}
            </div>
          `}

          ${i&&i.length>0&&l`
            <div className="mt-2 flex flex-col gap-1.5">
              ${i.map((E,M)=>l`<${Nc}
                key=${E.id||M}
                att=${E}
                onPreview=${g}
              />`)}
            </div>
            <${_c}
              attachment=${$}
              onClose=${()=>g(null)}
            />
          `}

          ${n==="assistant"&&l`<${fw}
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
                <${A} name=${b?"check":"copy"} className="h-3.5 w-3.5" />
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
                <${A} name="retry" className="h-3.5 w-3.5" />
                Retry
              </button>
            `}
            ${x&&l`<span className="font-mono text-[10px] text-iron-500">${x}</span>`}
          </div>
        `}
      </div>
    </div>
  `}var hw=h.default.memo(xT);function $w(e){let t=$T(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(ww(r)){let s=vw(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){gw(a,s),yw(a,r),n+=s.length;continue}}if(Vp(r)){let s=vw(t,n);gw(a,s),n+=s.length-1;continue}yw(a,r)}return a}function $T(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=kc(i);o&&ww(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!Vp(i))continue;let o=kc(i),u=o?t.get(o):void 0;if(u===void 0||u>=s)continue;let c=a.get(u)||[];c.push(i),a.set(u,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function vw(e,t){let a=t,n=kc(e[t]);for(;a<e.length&&Vp(e[a])&&wT(n,e[a]);)a+=1;return e.slice(t,a)}function wT(e,t){let a=kc(t);return!e||!a||a===e}function gw(e,t){if(t.length===0)return;let a=ST(t);e.push({type:"activity-run",id:`activity-run-${a[0].id}`,activity:a})}function yw(e,t){e.push({type:"message",id:t.id,message:t})}function ww(e){return e.role==="assistant"&&!Sw(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function Vp(e){return e.role==="thinking"||e.role==="tool_activity"||Sw(e)}function Sw(e){return e?.toolCalls&&e.toolCalls.length>0}function kc(e){return e?.turnRunId||null}function ST(e){return[...e].sort((t,a)=>t?.role!=="tool_activity"||a?.role!=="tool_activity"?0:NT(t,a))}function NT(e,t){if(Number.isFinite(e.activityOrder)&&Number.isFinite(t.activityOrder)){let n=e.activityOrder-t.activityOrder;if(n!==0)return n}let a=bw(xw(e.updatedAt||e.timestamp),xw(t.updatedAt||t.timestamp));return a!==0?a:bw(e.sequence,t.sequence)}function bw(e,t){let a=Number.isFinite(e)?e:null,n=Number.isFinite(t)?t:null;return a===null&&n===null?0:a===null?1:n===null?-1:a-n}function xw(e){if(!e)return null;let t=Date.parse(e);return Number.isFinite(t)?t:null}function Nw({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let u=k(),c=h.default.useRef(null),d=h.default.useRef(!0),[f,m]=h.default.useState(!0);h.default.useEffect(()=>{if(!c.current||!d.current)return;let g=window.requestAnimationFrame(()=>{let v=c.current;v&&(v.scrollTop=v.scrollHeight)});return()=>window.cancelAnimationFrame(g)},[e,i]);let p=h.default.useCallback(()=>{let $=c.current;if(!$)return;let g=100,v=$.scrollHeight-$.scrollTop-$.clientHeight;d.current=v<g,m(v<g),a&&$.scrollTop<g&&n&&!t&&n()},[a,n,t]),b=h.default.useCallback(()=>{let $=c.current;$&&($.scrollTop=$.scrollHeight,d.current=!0,m(!0))},[]),y=h.default.useMemo(()=>$w(e),[e]);return l`
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
        ${y.map($=>$.type==="activity-run"?l`<${sw} key=${$.id} activity=${$.activity} />`:l`<${hw}
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
        <${A} name="arrowDown" className="h-3.5 w-3.5" />
        ${u("chat.jumpToLatest")}
      </button>
    `}
    </div>
  `}function _w({notice:e,onRecover:t}){return l`
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
  `}function kw({suggestions:e,onSelect:t}){return!e||e.length===0?null:l`
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
  `}function Rw(){return l`
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
  `}function Rc(){return Y("/api/webchat/v2/channels/connectable")}function Cw(e,t){if(!Gp(e))return null;let a=Cc(e),n=CT(a),r=null;for(let s of t||[]){if(!RT(s))continue;let i=ET(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function Gp(e){let t=Cc(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function _T(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function kT(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>Ew(Cc(n))):a}function RT(e){return e?.strategy!=="admin_managed_channels"}function CT(e){return Tw(e,"slack")&&Ew(e)}function Ew(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function Cc(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function ET(e,t,a={}){return(a.commandAliasesOnly?kT(t,{channelManagementOnly:!0}):_T(t)).reduce((r,s)=>{let i=Cc(s);return Tw(e,i)?Math.max(r,i.length):r},0)}function Tw(e,t){return t?` ${e} `.includes(` ${t} `):!1}function Aw(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n=a?TT(a):[],r={kind:"gate",runId:t.turn_run_id,gateRef:t.gate_ref,invocationId:t.invocation_id||null,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return a?{...r,toolName:a.tool_name||null,description:a.reason||t.body,actionLabel:a.action?.label||null,destination:a.destination||null,approvalScope:a.scope||null,approvalDetails:n,parameters:n.length?n.map(s=>`${s.label}: ${s.value}`).join(`
`):null}:r}return e==="auth_required"?{kind:"auth_required",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function TT(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function Dw({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function Mw(){return{terminalByInvocation:new Map}}function Ow(e){e?.current?.terminalByInvocation?.clear()}function Lw(e,t,a){let n=jw(t,{toolStatus:"running"});n&&ni(e,n,a,{matchGate:!0})}function Pw(e,t,a,n="authorization"){let r=jw(t,{toolStatus:"error",toolError:n});r&&ni(e,r,a,{matchGate:!0})}function ni(e,t,a,n={}){if(!t)return;let r=PT(t);r=LT(r,a),e(s=>{let i=Uw(r),o=AT(s,r,i,n);if(o>=0){let c=[...s];return c[o]=MT(c[o],r),Yp(c[o],a),c}let u={id:i,role:"tool_activity",...r};return Yp(u,a),[...s,u]})}function jw(e,t={}){if(!e?.runId||!e?.gateRef||e.kind!=="gate"||!e.toolName)return null;let a=e.invocationId||`gate:${e.runId}:${e.gateRef}`;return{invocationId:a,callId:a,capabilityId:e.toolName,toolName:Or(e.toolName)||e.toolName,toolStatus:t.toolStatus||"running",toolDetail:null,toolParameters:null,toolResultPreview:null,toolError:t.toolError||null,toolDurationMs:null,updatedAt:t.updatedAt||new Date().toISOString(),resultRef:null,truncated:!1,outputBytes:null,outputKind:null,turnRunId:e.runId,gateRef:e.gateRef,gateActivity:!0}}function Uw(e){return`tool-${e.invocationId}`}function AT(e,t,a,n){let r=e.findIndex(i=>i?.id===a);if(r>=0)return r;let s=t.gateRef||null;if(s){let i=e.findIndex(o=>o?.role==="tool_activity"&&o.turnRunId===t.turnRunId&&o.gateRef===s);if(i>=0)return i}if(!n.matchGate&&!t.gateActivity){let i=e.findIndex(o=>DT(o,t));if(i>=0)return i}if(n.matchGate||t.gateActivity){let i=e.findIndex(o=>o?.role==="tool_activity"&&!o.gateRef&&o.gateActivity!==!0&&!Is(o.toolStatus)&&o.turnRunId===t.turnRunId&&Fw(o.toolName,t.toolName));if(i>=0)return i}return-1}function DT(e,t){return e?.role==="tool_activity"&&e.gateActivity===!0&&e.turnRunId===t.turnRunId&&Fw(e.toolName,t.toolName)}function MT(e,t){let a=Is(e.toolStatus),n=Is(t.toolStatus),r=a&&!n,s={...e,...t,id:e.id,role:"tool_activity",invocationId:e.gateActivity&&!t.gateActivity?t.invocationId:e.invocationId||t.invocationId,callId:e.gateActivity&&!t.gateActivity?t.callId:e.callId||t.callId,toolName:t.toolName||e.toolName,toolStatus:r?e.toolStatus:t.toolStatus,toolError:t.toolError||e.toolError,updatedAt:r?e.updatedAt||t.updatedAt:t.updatedAt||e.updatedAt,turnRunId:t.turnRunId||e.turnRunId||null,gateRef:t.gateRef||e.gateRef||null,gateActivity:e.gateActivity&&t.gateActivity,capabilityId:t.capabilityId||e.capabilityId||null,activityOrder:OT(e,t),activityOrderSource:t.activityOrderSource||e.activityOrderSource||null};return e.gateActivity&&!t.gateActivity&&(s.id=Uw(t),s.gateActivity=!1),s}function OT(e,t){return Number.isFinite(t.activityOrder)?t.activityOrder:e.activityOrder}function LT(e,t){if(!e?.invocationId)return e;if(Is(e.toolStatus))return Yp(e,t),e;let a=t?.current?.terminalByInvocation?.get(e.invocationId);return a?Number.isFinite(e.activityOrder)?{...a,activityOrder:e.activityOrder,activityOrderSource:e.activityOrderSource||a.activityOrderSource||null}:a:e}function Yp(e,t){!e?.invocationId||!Is(e.toolStatus)||t?.current?.terminalByInvocation?.set(e.invocationId,e)}function Fw(e,t){return!e||!t?!1:Or(e)===Or(t)}function PT(e){let t=Or(e.toolName||e.capabilityId);return{...e,toolName:t||e.toolName||"tool"}}function Hw({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o,onRunSettled:u}){let c=h.default.useRef(new Set),d=h.default.useRef(null),f=h.default.useRef(null);return h.default.useCallback(m=>{let{type:p,frame:b}=m||{};if(!(!p||!b))switch(p){case"accepted":{let y=b.ack||{};y.run_id&&(d.current=y.run_id),r?.({runId:y.run_id||null,threadId:y.thread_id||e,status:y.status||null}),a(!0);return}case"running":case"capability_progress":{let y=b.progress||{};y.turn_run_id&&(d.current=y.turn_run_id,r?.($=>$&&$.runId===y.turn_run_id?$:{runId:y.turn_run_id,threadId:e,status:"running"}),UT(n,y.turn_run_id,f)),a(!0);return}case"capability_activity":{let y=b.activity;if(!y||!y.invocation_id)return;ni(t,Sp(y),o);return}case"capability_display_preview":{let y=b.preview;if(!y||!y.invocation_id)return;let $=wp(y);ni(t,$,o);return}case"gate":case"auth_required":{let y=Aw(p,b.prompt);y&&(Lw(t,y,o),n(y),r?.({runId:y.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let y=b.reply||{};t($=>[...$,{id:`reply-${y.turn_run_id||Date.now()}`,role:"assistant",content:y.text||"",timestamp:y.generated_at||new Date().toISOString(),turnRunId:y.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let y=b.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Ec(c,u,y,!1);return}case"failed":{let y=b.run_state||{},$=y.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Zp(t,{runId:$,status:y.status||"failed",failureCategory:qT(y),failureSummary:null}),Ec(c,u,$,!1);return}case"projection_snapshot":case"projection_update":{let y=b.state?.items||[];FT({items:y,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:u,settledRunsRef:c,latestRunIdRef:d,promptRunIdRef:f,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i,o,u])}function Ec(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var jT=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),zw=new Set(["completed","succeeded"]),Jp=new Set(["blocked_auth","blocked_approval","blocked_resource"]);function qw(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function UT(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function FT({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,activeRunRef:d,locallyResolvedGatesRef:f,toolActivityStateRef:m}){let p=u?.current??null;for(let b of e){if(b.run_status){let{run_id:y,status:$,failure_category:g,failure_summary:v}=b.run_status,x=jT.has($),w=d?.current?.source==="local"?d.current.runId:null,S=!!(y&&w&&w!==y),R=p??u?.current??null,_=!!(x&&y&&R&&R!==y),E=y&&Jp.has($)?Bw(f,y):null;if(S)continue;if(_){Bw(f,d?.current?.runId)?.outcome==="resumed"&&(zT({runId:y,activePromptRunId:d?.current?.runId,success:zw.has($),status:$,failureCategory:g,failureSummary:v,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,locallyResolvedGatesRef:f}),p=null);continue}if(E){qw(r,y,c),E.outcome==="resumed"?(n(!0),s?.(M=>M&&M.runId===y?{...M,status:M.status==="awaiting_gate"?"queued":M.status||"queued"}:{runId:y,threadId:t,status:"queued"}),p=y,u&&(u.current=y)):(n(!1),d?.current?.runId===y&&s?.(null),p=null,u?.current===y&&(u.current=null));continue}y&&(p=y,!x&&u&&(u.current=y),s?.(M=>M&&M.runId===y?{...M,status:$}:{runId:y,threadId:t,status:$})),y&&Jp.has($)?c&&(c.current=y):y&&c?.current===y&&(c.current=null),x?(n(!1),r(null),s?.(null),Xp(f,y),p=null,u&&(u.current=null),y&&c?.current===y&&(c.current=null),Ec(o,i,y,zw.has($)),($==="failed"||$==="recovery_required")&&Zp(a,{runId:y,status:$,failureCategory:g,failureSummary:v})):Jp.has($)||(qw(r,y,c),Xp(f,y),n(!0))}if(b.text){let y=`text-${b.text.id}`;a($=>{let g=$.findIndex(x=>x.id===y),v={id:y,role:"assistant",content:b.text.body||"",timestamp:new Date().toISOString(),isFinalReply:!0};if(g>=0){let x=[...$];return x[g]=v,x}return[...$,v]}),n(!1)}if(b.thinking){let y=`thinking-${b.thinking.id}`;a($=>{let g=$.findIndex(x=>x.id===y),v={id:y,role:"thinking",content:b.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:b.thinking.run_id||null};if(g>=0){let x=[...$];return x[g]=v,x}return[...$,v]})}if(b.capability_activity){let y=b.capability_activity;y.invocation_id&&ni(a,Sp(y),m)}if(b.gate&&p&&c?.current===p&&!HT(f,p,b.gate.gate_ref)&&(r(y=>y||{kind:"gate",runId:p,gateRef:b.gate.gate_ref,headline:b.gate.headline,body:"",allowAlways:b.gate.allow_always===!0}),n(!1)),b.skill_activation){let{id:y,skill_names:$=[],feedback:g=[]}=b.skill_activation;if($.length||g.length){let v=`skill-${y||$.join("-")||"activation"}`,x=[$.length?`Skill activated: ${$.join(", ")}`:"",...g].filter(Boolean).join(`
`);a(w=>w.some(S=>S.id===v)?w:[...w,{id:v,role:"system",content:x,timestamp:new Date().toISOString()}])}}}u&&p&&(u.current=p)}function zT({runId:e,activePromptRunId:t,success:a,status:n,failureCategory:r,failureSummary:s,setMessages:i,setIsProcessing:o,setPendingGate:u,setActiveRun:c,onRunSettled:d,settledRunsRef:f,latestRunIdRef:m,promptRunIdRef:p,locallyResolvedGatesRef:b}){o(!1),u(null),c?.(null),Xp(b,t),m&&(m.current=null),p?.current===t&&(p.current=null),Ec(f,d,e,a),(n==="failed"||n==="recovery_required")&&Zp(i,{runId:e,status:n,failureCategory:r,failureSummary:s})}function qT(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function Zp(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`;e(i=>{let o=i.findIndex(c=>c.id===s),u=Dw({status:a,failureCategory:n,failureSummary:r});if(o>=0){if(!r||i[o].content===u)return i;let c=[...i];return c[o]={...c[o],content:u},c}return[...i,{id:s,role:"error",content:u,timestamp:new Date().toISOString()}]})}function Bw(e,t){if(!t)return null;let a=e?.current;if(!a)return null;for(let[n,r]of a.entries())if(n.startsWith(`${t}
`))return BT(r);return null}function BT(e){return e&&typeof e=="object"?{resolution:e.resolution||null,outcome:e.outcome||null}:{resolution:e||null,outcome:null}}function Xp(e,t){if(!t)return;let a=e?.current;if(a)for(let n of Array.from(a.keys()))n.startsWith(`${t}
`)&&a.delete(n)}function HT(e,t,a){return!t||!a?!1:!!e?.current?.has(`${t}
${a}`)}function Kw(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function Iw(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function Qw(e,t,a,n){let r=IT(n);return r?(KT(e,t,a,{timelineMessageId:r}),r):null}function KT(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function IT(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var QT=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function Vw({threadId:e,onEvent:t,enabled:a}){let[n,r]=h.default.useState("idle"),s=h.default.useRef(t);s.current=t;let i=h.default.useRef(null);return h.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,u=null,c=0,d=3e4;function f(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=Sx({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let y=Math.min(1e3*2**c,d);u=setTimeout(f,y)};let b=(y,$)=>{let g=null;try{g=JSON.parse(y.data)}catch{return}!g||typeof g!="object"||(y.lastEventId&&(i.current=y.lastEventId),s.current?.({type:g.type||$,frame:g,lastEventId:y.lastEventId||null}))};o.onmessage=y=>b(y,"message");for(let y of QT)o.addEventListener(y,$=>b($,y))}function m(){u&&(clearTimeout(u),u=null),o&&(o.close(),o=null),r("paused")}function p(){document.visibilityState==="hidden"?m():o||f()}return f(),document.addEventListener("visibilitychange",p),()=>{document.removeEventListener("visibilitychange",p),u&&clearTimeout(u),o&&o.close()}},[a,e]),{status:n}}var VT=3e4,GT="credential_stored_gate_resolution_failed",YT="ironclaw-product-auth",Wp="ironclaw:product-auth:oauth-complete",JT="ironclaw:product-auth:oauth-complete";async function Gw(e){let t=new AbortController,a=setTimeout(()=>t.abort(),VT);try{return await e(t.signal)}finally{clearTimeout(a)}}function XT(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=GT,t.cause=e,t}function ZT(e){let a=Tt.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function WT(e){return e?.continuation?.type==="turn_gate_resume"}function eA(e){if(e?.outcome)return e.outcome;let t=String(e?.status||"").toLowerCase();return t==="queued"||t==="running"?"resumed":t==="cancelled"||e?.already_terminal===!0?"cancelled":e?.already_terminal===!1?"resumed":null}function Yw(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function tA(e){return e?.type===JT&&e?.status==="completed"}function aA(e,t,a){if(!tA(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function eh(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function nA(e){if(!Gp(e))return null;try{let a=(await Tt.fetchQuery({queryKey:["connectable-channels"],queryFn:Rc}))?.channels||[];return Cw(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function Jw(e){let t=h.default.useRef(new Map),a=h.default.useRef(1),[n,r]=h.default.useState(0),[s,i]=h.default.useState(Date.now()),[o,u]=h.default.useState(null),c=h.default.useRef(o),d=h.default.useCallback(oe=>{let ne=typeof oe=="function"?oe(c.current):oe;c.current=ne,u(ne)},[]);h.default.useEffect(()=>{c.current=o},[o]);let[f,m]=h.default.useState(null),p=h.default.useCallback(()=>t.current.get(e||"__new__")||[],[e]),b=h.default.useCallback(oe=>{let ne=e||"__new__";oe.length>0?t.current.set(ne,oe):t.current.delete(ne)},[e]),{messages:y,hasMore:$,nextCursor:g,isLoading:v,loadError:x,loadHistory:w,setMessages:S}=Jx(e,{getPendingMessages:p,setPendingMessages:b}),[R,_]=h.default.useState(!1),[E,M]=h.default.useState(null),[P,I]=h.default.useState(e),D=h.default.useRef(Mw()),K=h.default.useRef(new Map),J=h.default.useRef({gateKey:null,credentialRef:null,inFlight:!1});P!==e&&(I(e),_(!1),M(null),u(null),m(null)),h.default.useEffect(()=>{Ow(D),K.current.clear()},[e]);let ve=Math.max(0,Math.ceil((n-s)/1e3)),_e=E?.runId&&E?.gateRef?`${E.runId}
${E.gateRef}`:null;h.default.useEffect(()=>{if(!n)return;let oe=setInterval(()=>i(Date.now()),250);return()=>clearInterval(oe)},[n]),h.default.useEffect(()=>{J.current.gateKey!==_e&&(J.current={gateKey:_e,credentialRef:null,inFlight:!1})},[_e]),h.default.useEffect(()=>{if(!Yw(E))return;let oe=Date.now(),ne=Ke=>{aA(Ke,E,oe)&&(M(Fe=>Yw(Fe)?null:Fe),_(!0))},$e=null;typeof window.BroadcastChannel=="function"&&($e=new window.BroadcastChannel(YT),$e.onmessage=Ke=>ne(Ke.data));let ge=Ke=>{Ke.key===Wp&&ne(eh(Ke.newValue))};window.addEventListener("storage",ge),ne(eh(window.localStorage?.getItem?.(Wp)));let nt=window.setInterval(()=>{ne(eh(window.localStorage?.getItem?.(Wp)))},500);return()=>{window.clearInterval(nt),$e&&$e.close(),window.removeEventListener("storage",ge)}},[E]);let Ye=Hw({threadId:e,setMessages:S,setIsProcessing:_,setPendingGate:M,setActiveRun:d,activeRunRef:c,locallyResolvedGatesRef:K,toolActivityStateRef:D,onRunSettled:(oe,{success:ne})=>{ne&&b([]),w(void 0,{preserveClientOnly:!0})}}),{status:_t}=Vw({threadId:e,onEvent:Ye,enabled:!!e}),ut=h.default.useCallback(async(oe,ne={})=>{let{threadId:$e,attachments:ge=[]}=ne,nt=ge.map(Bx),Ke=ge.map(Hx);if(ge.length===0){let ke=await nA(oe);if(ke)return m(ke),{channel_connect_action:ke}}m(null);let Fe=$e||e;if(!Fe){let ke=await ec();if(Tt.invalidateQueries({queryKey:["threads"]}),Fe=ke?.thread?.thread_id,!Fe)throw new Error("createThread returned no thread_id")}let ga=Fe,Mt={id:`pending-${a.current++}`,role:"user",content:oe,attachments:Ke,timestamp:new Date().toISOString(),isOptimistic:!0};Kw(t.current,ga,Mt);let ya=Mt.id;S(ke=>[...ke,{id:ya,role:"user",content:oe,attachments:Ke,timestamp:Mt.timestamp,isOptimistic:!0}]),_(!0),M(null);try{let ke=await xx({threadId:Fe,content:oe,attachments:nt});ZT(Fe)&&Tt.invalidateQueries({queryKey:["threads"]}),ke?.run_id&&d({runId:ke.run_id,threadId:ke.thread_id||Fe,status:ke.status||null,source:"local"});let ee=Qw(t.current,ga,ya,ke?.accepted_message_ref);return ee&&S(Re=>Re.map(bt=>bt.id===ya?{...bt,timelineMessageId:ee}:bt)),ke?.outcome==="rejected_busy"&&(S(Re=>Re.map(bt=>bt.id===ya?{...bt,isOptimistic:!1,status:"error"}:bt)),ke?.notice&&S(Re=>[...Re,{id:`system-rejected-${a.current++}`,role:"system",content:ke.notice,timestamp:new Date().toISOString(),isOptimistic:!1}]),_(!1)),ke}catch(ke){throw ke.status===429&&r(Date.now()+rA(ke)),S(ee=>ee.map(Re=>Re.id===ya?{...Re,isOptimistic:!1,status:"error",error:ke.message}:Re)),_(!1),ke}finally{Iw(t.current,ga,ya)}},[e,S]),kt=h.default.useCallback(async(oe,ne={})=>{if(!E)return;let{runId:$e,gateRef:ge}=E;if(!$e||!ge)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");let nt=await gp({threadId:e,runId:$e,gateRef:ge,resolution:oe,always:ne.always,credentialRef:ne.credentialRef}),Ke=eA(nt);if(K.current.set(`${$e}
${ge}`,{resolution:oe,outcome:Ke}),oe==="denied"&&Ke==="resumed"&&Pw(S,E,D),M(null),Ke==="resumed"){_(!0),d({runId:nt?.run_id||$e,threadId:nt?.thread_id||e,status:nt?.status||"queued"});return}_(!1),d(null)},[E,e,S,d]),Ga=h.default.useCallback(async oe=>{if(!E)throw new Error("auth gate is no longer pending");let{runId:ne,gateRef:$e,provider:ge}=E;if(!ne||!$e||!ge)throw new Error("auth gate is missing required credential metadata");let nt=E.accountLabel||`${ge} credential`,Ke=`${ne}
${$e}`;if(J.current.gateKey!==Ke&&(J.current={gateKey:Ke,credentialRef:null,inFlight:!1}),J.current.inFlight)throw new Error("auth token submission already in progress");J.current.inFlight=!0;try{let Fe=J.current.credentialRef,ga=null;if(!Fe){if(ga=await Gw(Mt=>_x({provider:ge,accountLabel:nt,token:oe,threadId:e,runId:ne,gateRef:$e,signal:Mt})),Fe=ga?.credential_ref,!Fe)throw new Error("manual token submit returned no credential_ref");J.current.credentialRef=Fe}if(!WT(ga))try{await Gw(Mt=>gp({threadId:e,runId:ne,gateRef:$e,resolution:"credential_provided",credentialRef:Fe,signal:Mt}))}catch(Mt){throw XT(Mt)}J.current={gateKey:null,credentialRef:null,inFlight:!1},M(null),_(!0)}catch(Fe){throw J.current.gateKey===Ke&&(J.current.inFlight=!1),Fe}},[E,e]),$n=h.default.useCallback(async oe=>{let ne=o?.runId;!ne||!e||(M(null),_(!1),d(null),await Nx({threadId:e,runId:ne,reason:oe}))},[o,e]),Ca=h.default.useCallback(()=>{$&&g&&w(g)},[$,g,w]),Ya=h.default.useCallback(async(oe,ne,$e)=>{let ge="approved",nt=!1;ne==="deny"?ge="denied":ne==="cancel"?ge="cancelled":ne==="always"&&(ge="approved",nt=!0),await kt(ge,{always:nt})},[kt]),at=h.default.useCallback(()=>{},[]);return{messages:y,isProcessing:R,pendingGate:E,channelConnectAction:f,activeRun:o,sseStatus:_t,historyLoading:v,historyLoadError:x,hasMore:$,cooldownSeconds:ve,send:ut,resolveGate:kt,submitAuthToken:Ga,cancelRun:$n,loadMore:Ca,dismissChannelConnectAction:()=>m(null),suggestions:[],setSuggestions:at,retryMessage:at,approve:Ya,recoverHistory:at,recoveryNotice:null}}function rA(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function Xw({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}var sA=1500;function Zw({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let{messages:o,isProcessing:u,pendingGate:c,channelConnectAction:d,suggestions:f,sseStatus:m,historyLoading:p,historyLoadError:b,hasMore:y,cooldownSeconds:$,recoveryNotice:g,activeRun:v,send:x,cancelRun:w,retryMessage:S,approve:R,recoverHistory:_,loadMore:E,setSuggestions:M,submitAuthToken:P,dismissChannelConnectAction:I}=Jw(t),D=h.default.useMemo(()=>e.find(at=>at.id===t)||null,[e,t]),K=h.default.useMemo(()=>Xw({gatewayStatus:i,activeThread:D}),[i,D]),J=o.length>0||u||!!c||!!d,ve=!p&&!J&&!b,_e=u&&!c||$>0,Ye=$>0?`Retry in ${$}s`:void 0,_t=t||jo,ut=!!(t&&v?.runId&&v.threadId===t&&u&&!c),kt=h.default.useCallback(async(at,{images:oe=[],attachments:ne=[]}={})=>{let $e=await x(at,{images:oe,attachments:ne,threadId:t}),ge=$e?.thread_id||t;return!t&&ge&&a&&a(ge,{replace:!0}),$e},[t,a,x]),Ga=h.default.useCallback(async at=>{M([]),await kt(at)},[kt,M]),$n=h.default.useCallback(()=>w("user_requested"),[w]);h.default.useEffect(()=>{if(!t)return;if(c){cc(t,yn.NEEDS_ATTENTION);return}if(u){cc(t,yn.RUNNING);return}let at=setTimeout(()=>G$(t),sA);return()=>clearTimeout(at)},[t,c,u]);let[Ca,Ya]=h.default.useState(!1);return h.default.useEffect(()=>{let at=oe=>{if(oe.key==="Escape"){Ya(!1);return}if(oe.key!=="?")return;let ne=oe.target,$e=ne?.tagName;$e==="INPUT"||$e==="TEXTAREA"||ne?.isContentEditable||(oe.preventDefault(),Ya(ge=>!ge))};return window.addEventListener("keydown",at),()=>window.removeEventListener("keydown",at)},[]),l`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${X1} status=${m} />

        ${b&&l`
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            ${b}
          </div>
        `}

        ${ve&&l`
          <${Z1}
            onSuggestion=${Ga}
            onSend=${kt}
            disabled=${_e}
            initialText=${r}
            resetKey=${s}
            draftKey=${_t}
            context=${K}
            statusText=${Ye}
            canCancel=${ut}
            onCancel=${$n}
          />
        `}
        ${!ve&&l`
          <${Nw}
            messages=${o}
            isLoading=${p}
            hasMore=${y}
            onLoadMore=${E}
            onRetryMessage=${S}
            threadId=${t}
            pending=${u}
          >
            ${g&&l`
              <${_w}
                notice=${g}
                onRecover=${_}
              />
            `}
            ${u&&!c&&l`<${Rw} />`}
            ${d&&l`
              <${G1}
                connectAction=${d}
                onDismiss=${I}
              />
            `}
            ${c&&(c.kind==="auth_required"?c.challengeKind==="oauth_url"?l`
                  <${I1}
                    gate=${c}
                    onCancel=${()=>R(c.requestId,"cancel",c.kind)}
                  />
                `:c.challengeKind==="manual_token"?l`
                  <${Q1}
                    gate=${c}
                    onSubmit=${P}
                    onCancel=${()=>R(c.requestId,"cancel",c.kind)}
                  />
                `:l`
                  <${K1}
                    gate=${c}
                    onCancel=${()=>R(c.requestId,"cancel",c.kind)}
                  />
                `:l`
              <${H1}
                gate=${c}
                onApprove=${()=>R(c.requestId,"approve",c.kind)}
                onDeny=${()=>R(c.requestId,"deny",c.kind)}
                onAlways=${()=>R(c.requestId,"always",c.kind)}
              />
            `)}
          <//>

          <${kw}
            suggestions=${f}
            onSelect=${Ga}
          />

          <${wc}
            onSend=${kt}
            disabled=${_e}
            initialText=${r}
            resetKey=${s}
            draftKey=${_t}
            context=${K}
            statusText=${Ye}
            canCancel=${ut}
            onCancel=${$n}
          />
        `}
      </div>
      <${W1}
        open=${Ca}
        onClose=${()=>Ya(!1)}
      />
    </div>
  `}function th(){let{threadsState:e,gatewayStatus:t}=Ba(),{threadId:a}=ot(),n=ce(),r=je(),s=r.state?.composerDraft||"";h.default.useEffect(()=>{a&&a!==e.activeThreadId?e.setActiveThreadId(a):a||e.setActiveThreadId(null)},[a]);let i=h.default.useCallback((o,u={})=>{if(!o){e.setActiveThreadId(null),n("/chat",u);return}e.setActiveThreadId(o),n(`/chat/${o}`,u)},[e,n]);return l`
    <${Zw}
      threads=${e.threads}
      activeThreadId=${e.activeThreadId}
      onSelectThread=${i}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function Ww(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?Vs(e,t):"",model:e?lc(e,t):""}}function e2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u}){let[c,d]=h.default.useState(()=>Ww(e,a)),[f,m]=h.default.useState(""),[p,b]=h.default.useState([]),[y,$]=h.default.useState(null),[g,v]=h.default.useState(""),x=h.default.useRef(!!e);h.default.useEffect(()=>{n&&(d(Ww(e,a)),m(""),b([]),$(null),v(""),x.current=!!e)},[n,e,a]);let w=e?.builtin===!0,S=e&&!e.builtin,R=h.default.useCallback((I,D)=>{d(K=>{let J={...K,[I]:D};return I==="name"&&!x.current&&(J.id=D$(D)),J})},[]),_=h.default.useCallback(()=>!w&&(!c.name.trim()||!c.id.trim())?u("llm.fieldsRequired"):!w&&!M$(c.id.trim())?u("llm.invalidId"):!S&&!w&&t.includes(c.id.trim())?u("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,w,S,u]),E=h.default.useCallback(async()=>{let I=_();if(I){$({tone:"error",text:I});return}v("save");try{await s({form:c,apiKey:f,provider:e}),r()}catch(D){$({tone:"error",text:D.message})}finally{v("")}},[f,c,r,s,e,_]),M=h.default.useCallback(async()=>{if(!c.model.trim()){$({tone:"error",text:u("llm.modelRequired")});return}v("test");try{let I=await i(Tp(e,c,f,a));$({tone:I.ok?"success":"error",text:I.message})}catch(I){$({tone:"error",text:I.message})}finally{v("")}},[f,a,c,i,e,u]),P=h.default.useCallback(async()=>{if((w?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){$({tone:"error",text:u("llm.baseUrlRequired")});return}v("models");try{let D=await o(Tp(e,c,f,a));if(!D.ok||!Array.isArray(D.models)||!D.models.length)$({tone:"error",text:D.message||u("llm.modelsFetchFailed")});else{b(D.models);let K=O$(c.model,D.models);K!==null&&R("model",K),$({tone:"success",text:u("llm.modelsFetched",{count:D.models.length})})}}catch(D){$({tone:"error",text:D.message})}finally{v("")}},[f,a,c,w,o,e,u,R]);return{form:c,apiKey:f,models:p,message:y,busy:g,isBuiltin:w,isEditing:S,setApiKey:m,update:R,submit:E,runTest:M,fetchModels:P,markIdEdited:()=>{x.current=!0}}}function Tc({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let u=k(),c=e2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u});if(!n)return null;let{form:d,apiKey:f,models:m,message:p,busy:b,isBuiltin:y,isEditing:$}=c,g=y?u("llm.configureProvider",{name:e.name||e.id}):u($?"llm.editProvider":"llm.newProvider");return l`
    <${ei} open=${n} onClose=${r} title=${g} size="lg">
      <${ti} className="space-y-4">
        ${!y&&l`
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${u("llm.providerName")}
              <${Dt} value=${d.name} onChange=${v=>c.update("name",v.target.value)} />
            </label>
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${u("llm.providerId")}
              <${Dt}
                value=${d.id}
                disabled=${$}
                onChange=${v=>{c.markIdEdited(),c.update("id",v.target.value)}}
              />
            </label>
          </div>
          <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
            ${u("llm.adapter")}
            <${Hp} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${Ep.map(v=>l`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
            <//>
          </label>
        `}

        ${y&&l`
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            ${zo(e.adapter)}
          </div>
        `}

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.baseUrl")}
          <${Dt} value=${d.baseUrl} placeholder=${e?.base_url||""} onChange=${v=>c.update("baseUrl",v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.apiKey")}
          <${Dt} type="password" value=${f} placeholder=${u("llm.apiKeyPlaceholder")} onChange=${v=>c.setApiKey(v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.defaultModel")}
          <div className="flex items-stretch gap-2">
            <${Dt} value=${d.model} onChange=${v=>c.update("model",v.target.value)} />
            <${T} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${b!==""} onClick=${c.fetchModels}>
              ${u(b==="models"?"llm.fetchingModels":"llm.fetchModels")}
            <//>
          </div>
        </label>

        ${m.length>0&&l`
          <${Hp} value=${d.model} onChange=${v=>c.update("model",v.target.value)}>
            ${m.map(v=>l`<option key=${v} value=${v}>${v}</option>`)}
          <//>
        `}

        ${p&&l`
          <div className=${p.tone==="error"?"text-sm text-red-200":"text-sm text-mint"} role="status">
            ${p.text}
          </div>
        `}
      <//>
      <${ai}>
        <${T} type="button" variant="secondary" disabled=${b!==""} onClick=${c.runTest}>
          ${u(b==="test"?"llm.testing":"llm.testConnection")}
        <//>
        <${T} type="button" variant="ghost" disabled=${b!==""} onClick=${r}>${u("common.cancel")}<//>
        <${T} type="button" disabled=${b!==""} onClick=${c.submit}>
          ${u(b==="save"?"common.saving":"common.save")}
        <//>
      <//>
    <//>
  `}function Ac({login:e}){let t=k(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return l`
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
  `}function iA(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function Dc({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=Gs({settings:e,gatewayStatus:t}),[s,i]=h.default.useState(null),[o,u]=h.default.useState(!1),[c,d]=h.default.useState(null),f=h.default.useRef(null),m=h.default.useCallback((g,v)=>{f.current&&window.clearTimeout(f.current),d({tone:g,text:v}),f.current=window.setTimeout(()=>d(null),3500)},[]);h.default.useEffect(()=>()=>{f.current&&window.clearTimeout(f.current)},[]);let p=h.default.useCallback((g=null)=>{i(g),u(!0)},[]),b=h.default.useCallback(async g=>{try{await r.setActiveProvider(g),m("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(p(g),m("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):m("error",v.message)}},[p,r,m,n]),y=h.default.useCallback(async({form:g,apiKey:v,provider:x})=>{if(x?.builtin){await r.saveBuiltinProvider({provider:x,form:g,apiKey:v}),m("success",n("llm.providerConfigured",{name:x.name||x.id}));return}let w=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:x});m("success",n(x?"llm.providerUpdated":"llm.providerAdded",{name:w.name||w.id}))},[r,m,n]),$=h.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),m("success",n("llm.providerDeleted"))}catch(v){m("error",v.message)}},[r,m,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>iA(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:p,closeDialog:()=>u(!1),handleUse:b,handleSave:y,handleDelete:$}}var oA=3e5;function lA(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function uA(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function cA(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=u=>{let c=u.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},oA);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var dA=3e5,mA=9e5,fA=2e3;async function t2(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,fA)),(await oc().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function Mc({onSuccess:e}={}){let t=k(),a=X(),[n,r]=h.default.useState(!1),[s,i]=h.default.useState(""),[o,u]=h.default.useState(!1),[c,d]=h.default.useState(""),[f,m]=h.default.useState(null),p=h.default.useCallback(()=>{i(""),d(""),m(null)},[]),b=h.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),y=h.default.useCallback(async v=>{if(p(),lA()){i(t("onboarding.nearaiLocalSso"));return}let x=window.open("about:blank","_blank");if(!x){i(t("onboarding.nearaiFailed"));return}try{x.opener=null}catch{}r(!0);try{let{auth_url:w}=await m$({provider:v,origin:window.location.origin});x.location.href=w;let S=await t2("nearai",dA,x);if(S==="active"){await b();return}x.close(),i(t(S==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{x.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[b,p,t]),$=h.default.useCallback(async()=>{p(),r(!0);try{let v=uA(),x=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!x){i(t("onboarding.nearaiFailed"));return}x.opener=null;let w=await cA(x,v);if(!w){i(t("onboarding.nearaiFailed"));return}await f$({account_id:w.accountId,public_key:w.publicKey,signature:w.signature,message:w.message,recipient:w.recipient,nonce:w.nonce}),await b()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[b,p,t]),g=h.default.useCallback(async()=>{p();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}u(!0);try{let{user_code:x,verification_uri:w}=await p$();m({userCode:x,verificationUri:w}),v&&(v.location.href=w);let S=await t2("openai_codex",mA,v);if(S==="active"){await b();return}v&&v.close(),d(t(S==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{u(!1)}},[b,p,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:f,startNearai:y,startNearaiWallet:$,startCodex:g}}var a2="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",pA="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",hA="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",vA="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",gA={nearai:{color:"#00ec97",path:pA},openai_codex:{color:"#10a37f",path:a2},openai:{color:"#10a37f",path:a2},anthropic:{color:"#d97757",path:hA},ollama:{color:null,path:vA}};function n2({id:e,name:t}){let a=gA[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return l`
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
  `}var yA=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function bA({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=h.default.useState(!1),o=h.default.useRef(null),u=t||a.nearaiBusy;h.default.useEffect(()=>{if(!s)return;let d=m=>{o.current&&!o.current.contains(m.target)&&i(!1)},f=m=>{m.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",f),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",f)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return l`
    <div ref=${o} className="relative shrink-0">
      <${T}
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
        <${A} name="chevron" className="h-3.5 w-3.5" />
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
  `}function xA({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let u=s(e.nameKey),c;return e.auth==="nearai"?c=l`<${bA} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=l`
      <${T} type="button" variant="secondary" size="sm" disabled=${r.codexBusy} onClick=${r.startCodex}>
        ${s("onboarding.signIn")}
      <//>
    `:a?c=l`<${T} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>i(t)}>
      ${s("llm.use")}
    <//>`:c=l`<${T} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>o(t)}>
      ${s("onboarding.setUp")}
    <//>`,l`
    <${te} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-4">
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <${n2} id=${e.id} name=${u} />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">${u}</span>
            ${a&&l`<${j} tone="positive" label=${s("onboarding.ready")} size="sm" />`}
          </div>
          <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">${s(e.descKey)}</div>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap gap-2 sm:justify-end">${c}</div>
    <//>
  `}function r2(){let{isAdmin:e=!1,isChecking:t=!1}=Ba();return t?null:e?l`<${$A} />`:l`<${lt} to="/chat" replace />`}function $A(){let e=k(),t=ce(),a=X(),{gatewayStatus:n}=Ba(),r=Dc({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=yA.map(f=>({entry:f,provider:s.providers.find(m=>m.id===f.id)})).filter(f=>f.provider),o=h.default.useCallback(()=>t("/chat"),[t]),u=Mc({onSuccess:o}),c=h.default.useCallback(async f=>{let m=f.active_model||f.default_model||"";await Fo({provider_id:f.id,model:m}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=h.default.useCallback(async({form:f,apiKey:m,provider:p})=>{await r.handleSave({form:f,apiKey:m,provider:p});let b=p?.id||f.id.trim(),y=f.model?.trim()||p?.default_model||"";await Fo({provider_id:b,model:y}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?l`
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
              <${xA}
                key=${f.id}
                entry=${f}
                provider=${m}
                configured=${Pr(m,s.builtinOverrides)}
                isBusy=${s.isBusy}
                login=${u}
                t=${e}
                onUse=${c}
                onSetUp=${r.openDialog}
              />
            `)}
        </div>

        <${Ac} login=${u} />

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

      <${Tc}
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
  `}function U({children:e,className:t="",...a}){return l`<${te} className=${t} ...${a}>${e}<//>`}function et({label:e,value:t,tone:a="muted",badgeLabel:n,detail:r,showDivider:s=!0,className:i="",valueClassName:o="text-[1.75rem] md:text-[2rem]"}){return l`
    <div
      className=${H("px-1 py-4",s&&"border-t border-[var(--v2-panel-border)]",i)}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div
            className="font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]"
          >
            ${e}
          </div>
          <div
            className=${H("mt-3 truncate font-medium tracking-[-0.05em] text-[var(--v2-text-strong)]",o)}
          >
            ${t}
          </div>
          ${r&&l`<div className="mt-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            ${r}
          </div>`}
        </div>
        <${j} tone=${a} label=${n??a} />
      </div>
    </div>
  `}function s2({items:e}){return l`
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
  `}function he({title:e,description:t,children:a,boxed:n=!0}){let r=l`
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
  `;return n?l`<${te} padding="lg">${r}<//>`:l`<div className="py-8">${r}</div>`}var i2={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function Qa({result:e,onDismiss:t}){return e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",i2[e.type]||i2.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}var o2="",wA={workspace:"home"};function Oc(e){return wA[e]||e}function Vo(e){return[...e||[]].sort((t,a)=>t.is_dir!==a.is_dir?t.is_dir?-1:1:t.name.localeCompare(a.name,void 0,{sensitivity:"base"}))}function ri(e){return e?e.split("/").filter(Boolean):[]}function Lc(e){return e?`/workspace/${ri(e).map(encodeURIComponent).join("/")}`:"/workspace"}function ah(e){let t=ri(e);return t.pop(),t.join("/")}function l2(e){return/\.mdx?$/i.test(e||"")}function Pc({path:e,onNavigate:t}){let a=k(),n=ri(e),r="";return l`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button
        type="button"
        onClick=${()=>t("/workspace")}
        className="text-signal hover:underline"
      >
        ${a("workspace.breadcrumbRoot")}
      </button>
      ${n.map((s,i)=>{r=r?`${r}/${s}`:s;let o=r,u=i===0?Oc(s):s;return l`
          <span key=${o} className="text-iron-400">/</span>
          <button
            key=${`${o}-button`}
            type="button"
            onClick=${()=>t(Lc(o))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${u}
          </button>
        `})}
    </div>
  `}function SA(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function u2({path:e,entries:t,isLoading:a,filter:n,onOpen:r,onNavigate:s}){let i=k();if(a)return l`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;let o=(t||[]).filter(m=>!SA(m.path)),u=String(n||"").trim().toLowerCase(),c=u?o.filter(m=>m.name.toLowerCase().includes(u)):o,d=Vo(c),f;return o.length?d.length?f=l`
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
    <${U} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${Pc} path=${e} onNavigate=${s} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">${f}</div>
    <//>
  `}var jc="/api/webchat/v2/fs",NA=1024*1024,_A=8*1024*1024;function c2(e){let t=String(e||"").split("/").filter(Boolean);return{mount:t.shift()||"",path:t.join("/")}}function kA(e,t){return t?`${e}/${t}`:e}function RA(e){let t=String(e||"").toLowerCase();return t.startsWith("text/")||t==="application/json"||t==="application/javascript"||t==="application/xml"||t.endsWith("+json")||t.endsWith("+xml")}function CA(e){return String(e||"").toLowerCase().startsWith("image/")}function EA(e){let t=String(e||"").toLowerCase();return t.startsWith("audio/")||t.startsWith("video/")||t.startsWith("font/")||t==="application/pdf"||t==="application/zip"||t==="application/gzip"}function TA(e){if(e.subarray(0,Math.min(e.length,8192)).indexOf(0)!==-1)return!0;try{return new TextDecoder("utf-8",{fatal:!0}).decode(e),!1}catch{return!0}}function AA(e,t){let a=new URL(`${jc}/content`,window.location.origin);return a.searchParams.set("mount",e),a.searchParams.set("path",t),a.pathname+a.search}async function DA(){return(await Y(`${jc}/mounts`))?.mounts||[]}async function si(e=""){if(!e)return{entries:(await DA()).map(o=>({name:Oc(o.mount),path:o.mount,is_dir:!0}))};let{mount:t,path:a}=c2(e),n=new URL(`${jc}/list`,window.location.origin);return n.searchParams.set("mount",t),a&&n.searchParams.set("path",a),{entries:((await Y(n.pathname+n.search))?.entries||[]).map(i=>({name:i.name,path:kA(t,i.path),is_dir:i.kind==="directory"}))}}async function d2(e){let{mount:t,path:a}=c2(e);if(!t||!a)return{kind:"directory",path:e};let n=new URL(`${jc}/stat`,window.location.origin);n.searchParams.set("mount",t),n.searchParams.set("path",a);let s=(await Y(n.pathname+n.search))?.stat||{},i=s.mime_type||"application/octet-stream",o=Number(s.size_bytes||0),u=AA(t,a),c={path:e,mime:i,size_bytes:o,download_path:u};if(s.kind&&s.kind!=="file")return{...c,kind:"directory"};if(CA(i)){if(o>_A)return{...c,kind:"binary"};let p=await ac(u);return{...c,kind:"image",image_data_url:p}}if(EA(i)||o>NA)return{...c,kind:"binary"};let d=await Sa(u),f=new Uint8Array(await d.arrayBuffer());if(!RA(i)&&TA(f))return{...c,kind:"binary"};let m=new TextDecoder("utf-8").decode(f);return{...c,kind:"text",content:m}}function m2(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function MA(e,t,a){let n=String(t||"").trim().toLowerCase(),r=(e||[]).filter(s=>!m2(s.path)).filter(s=>!n||s.name.toLowerCase().includes(n)?!0:s.is_dir&&a.has(s.path));return Vo(r)}function f2({entry:e,depth:t,selectedPath:a,expandedPaths:n,filter:r,onToggleDirectory:s,onSelectFile:i}){let o=k(),u=n.has(e.path),c=z({queryKey:["workspace-list",e.path],queryFn:()=>si(e.path),enabled:e.is_dir&&u});if(e.is_dir){let d=MA(c.data?.entries,r,n);return l`
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
                  <${f2}
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
  `}function p2({entries:e,selectedPath:t,expandedPaths:a,filter:n,onToggleDirectory:r,onSelectFile:s,isLoading:i}){let o=k();if(i)return l`<div className="space-y-2 p-3">${[1,2,3,4].map(c=>l`<div key=${c} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let u=Vo(e.filter(c=>!m2(c.path)));return u.length?l`
    <div className="space-y-1 p-2">
      ${u.map(c=>l`
        <${f2}
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
  `:l`<div className="px-4 py-8 text-sm text-iron-300">${o("workspace.noFiles")}</div>`}function h2({rootEntries:e,selectedPath:t,expandedPaths:a,filter:n,onFilterChange:r,isLoadingTree:s,onToggleDirectory:i,onSelectFile:o}){let u=k();return l`
    <${U} className="flex min-h-[420px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="border-b border-white/10 p-3">
        <input
          value=${n}
          onInput=${c=>r(c.target.value)}
          placeholder=${u("workspace.filterPlaceholder")}
          className="h-9 w-full rounded-md border border-white/10 bg-iron-950/80 px-3 text-sm text-white outline-none placeholder:text-iron-400 focus:border-signal/45"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <${p2}
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
  `}function v2(e){return ri(e).pop()||"download"}function OA({path:e,file:t}){let a=k();return t.kind==="image"?l`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${t.image_data_url}
          alt=${v2(e)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `:t.kind==="text"?l`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${l2(e)?l`<${na} content=${t.content} className="max-w-4xl text-base leading-7" />`:l`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
      </div>
    `:l`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${a("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `}function g2({path:e,file:t,isLoading:a,onNavigate:n}){let r=k(),[s,i]=h.default.useState(!1),o=h.default.useCallback(async()=>{if(t?.download_path){i(!0);try{let c=await Sa(t.download_path);Sc(c,v2(e))}catch{}finally{i(!1)}}},[t,e]);if(a)return l`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;if(!t||t.kind==="directory")return l`
      <${he}
        title=${r("workspace.pickFileTitle")}
        description=${r("workspace.pickFileDesc")}
      />
    `;let u=r("workspace.fileMeta",{mime:t.mime||"application/octet-stream",size:Number(t.size_bytes||0)});return l`
    <${U} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${Pc} path=${e} onNavigate=${n} />
        <div className="flex items-center gap-2">
          <${j} tone="muted" label=${u} />
          <${T}
            variant="secondary"
            size="sm"
            onClick=${o}
            disabled=${s}
          >${r("workspace.download")}<//>
        </div>
      </div>

      <${OA} path=${e} file=${t} />

      ${ah(e)&&l`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${r("workspace.parent",{path:ah(e)})}
        </div>
      `}
    <//>
  `}function y2(e){let t=k(),a=X(),[n,r]=h.default.useState(new Set),[s,i]=h.default.useState(""),[o,u]=h.default.useState(null),c=z({queryKey:["workspace-list",""],queryFn:()=>si("")}),d=z({queryKey:["workspace-file",e],queryFn:()=>d2(e),enabled:!!e}),f=e===""||d.data?.kind==="directory",m=z({queryKey:["workspace-list",e],queryFn:()=>si(e),enabled:f});h.default.useEffect(()=>{u(null)},[e]);let p=h.default.useCallback(y=>a.fetchQuery({queryKey:["workspace-list",y],queryFn:()=>si(y)}),[a]),b=h.default.useCallback(async y=>{let $=new Set(n);if($.has(y)){$.delete(y),r($);return}$.add(y),r($);try{await p(y)}catch(g){u({type:"error",message:g.message||t("workspace.unableOpenDirectory")})}},[n,p,t]);return{rootEntries:c.data?.entries||[],file:d.data||null,selectionIsDirectory:f,currentEntries:m.data?.entries||[],expandedPaths:n,filter:s,setFilter:i,result:o,clearResult:()=>u(null),isLoadingTree:c.isLoading,isLoadingFile:d.isLoading,isLoadingListing:m.isLoading,isFetching:c.isFetching||d.isFetching||m.isFetching,error:c.error||d.error||m.error||null,loadDirectory:p,toggleDirectory:b,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function nh(){let e=k(),t=ce(),n=ot()["*"]||o2,r=y2(n),s=h.default.useCallback(i=>{t(Lc(i))},[t]);return l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="flex h-full min-h-0 flex-col space-y-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1 className="text-lg font-semibold text-white">${e("workspace.title")}</h1>
                <${j} tone="muted" label=${e("workspace.readOnly")} />
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

          ${r.error&&l`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${r.error.message}
            </div>
          `}
          <${Qa}
            result=${r.result}
            onDismiss=${r.clearResult}
          />

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <${h2}
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
                  <${u2}
                    path=${n}
                    entries=${r.currentEntries}
                    isLoading=${r.isLoadingListing}
                    filter=${r.filter}
                    onOpen=${s}
                    onNavigate=${t}
                  />
                `:l`
                  <${g2}
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
  `}function b2(e){if(!e)return null;let t=e.metadata&&typeof e.metadata=="object"&&!Array.isArray(e.metadata)?e.metadata:{};return{id:e.project_id,name:e.name,description:e.description,goals:Array.isArray(t.goals)?t.goals:[],icon:e.icon||null,color:e.color||null,state:e.state,role:e.role,metadata:t,created_at:e.created_at,updated_at:e.updated_at,health:e.state==="archived"?"muted":"green"}}async function x2(){let t=((await px({limit:200}))?.projects||[]).map(b2);return{attention:[],projects:t}}async function $2(e){if(!e)return null;let t=await hx({projectId:e});return b2(t?.project)}function w2(e){return Promise.resolve({missions:[],todo:!0})}function S2(e){return Promise.resolve({threads:[],todo:!0})}function N2(e){return Promise.resolve({widgets:[],todo:!0})}function _2(e){return Promise.resolve(null)}function k2(e){return Promise.resolve(null)}function R2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function C2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function E2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function T2(){let e=X(),t=z({queryKey:["projects-overview"],queryFn:x2,refetchInterval:5e3}),a=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function A2(e){let t=X(),a=!!e,n=z({queryKey:["project-detail",e],queryFn:()=>$2(e),enabled:a,refetchInterval:a?7e3:!1}),r=z({queryKey:["project-missions",e],queryFn:()=>w2(e),enabled:a,refetchInterval:a?5e3:!1}),s=z({queryKey:["project-threads",e],queryFn:()=>S2(e),enabled:a,refetchInterval:a?4e3:!1}),i=z({queryKey:["project-widgets",e],queryFn:()=>N2(e),enabled:a,refetchInterval:a?15e3:!1}),o=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function D2({projectId:e,missionId:t,threadId:a}){let n=X(),[r,s]=h.default.useState(null),i=z({queryKey:["project-mission-detail",t],queryFn:()=>_2(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=z({queryKey:["project-thread-detail",a],queryFn:()=>k2(a),enabled:!!a,refetchInterval:a?4e3:!1}),u=h.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=Q({mutationFn:({targetMissionId:m})=>R2(m),onSuccess:m=>{s({type:"success",message:m?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to fire mission"})}}),d=Q({mutationFn:({targetMissionId:m})=>C2(m),onSuccess:()=>{s({type:"success",message:"Mission paused."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to pause mission"})}}),f=Q({mutationFn:({targetMissionId:m})=>E2(m),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:f.mutateAsync,isBusy:c.isPending||d.isPending||f.isPending}}function Uc(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function Fc(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function M2(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function O2(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function LA(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function L2(e){let t=LA(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function P2(e){let t=e?.projects||[],a=t.reduce((o,u)=>o+Number(u.cost_today_usd||0),0),n=t.reduce((o,u)=>o+Number(u.active_missions||0),0),r=t.reduce((o,u)=>o+Number(u.threads_today||0),0),s=t.reduce((o,u)=>o+Number(u.pending_gates||0),0),i=t.reduce((o,u)=>o+Number(u.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function Go(e,t){return`${e} ${t}${e===1?"":"s"}`}var PA={projects:"muted",attention:"warning",spend:"success"};function j2({overview:e}){let t=P2(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:Fc(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return l`
    <${U} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${j} tone=${PA[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function jA(e){return e?.type==="failure"?"danger":"warning"}function UA(e){return e?.type==="failure"?"failure":"gate"}function U2({items:e,onOpenItem:t}){return e?.length?l`
    <${U} className="overflow-hidden border-amber-300/10 p-0">
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
              <${j} tone=${jA(a)} label=${UA(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function FA({project:e,onOpen:t,t:a}){return l`
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
        <${j} tone=${M2(e.health)} label=${e.health||"unknown"} />
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
            ${a("projects.card.threadsToday",{count:Go(e.threads_today||0,"thread")})}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${a("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">${Go(e.pending_gates||0,"gate")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${a("projects.card.failures24h",{count:Go(e.failures_24h||0,"failure")})}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>${a("projects.card.spendToday",{value:Fc(e.cost_today_usd||0)})}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${Uc(e.last_activity)}</div>
        </div>
        <${T}
          variant="secondary"
          onClick=${n=>{n.stopPropagation(),t(e.id)}}
        >${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function zA({project:e,onOpen:t,t:a}){return l`
    <${U}
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
            ${Go(e.threads_today||0,"thread")} today
          </div>
          <${T}
            variant="secondary"
            onClick=${n=>{n.stopPropagation(),t(e.id)}}
          >${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function F2({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=k(),u=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return!e.length&&t>0?l`
      <${he}
        title=${o("projects.empty.noMatchTitle")}
        description=${o("projects.empty.noMatchDesc")}
      />
    `:e.length?l`
    <div className="space-y-5">
      ${u&&l`<${zA} project=${u} onOpen=${r} t=${o} />`}

      <${U} className="p-4 sm:p-5">
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

      ${c.length?l`<div className="grid gap-4 xl:grid-cols-2 2xl:grid-cols-3">
            ${c.map(d=>l`<${FA} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
          </div>`:l`
            <${he}
              title=${o("projects.scoped.onlyGeneralTitle")}
              description=${o("projects.scoped.onlyGeneralDesc")}
            >
              <${T} onClick=${s}>${o(i?"projects.preparingChat":"projects.startProject")}<//>
            <//>
          `}
    </div>
  `:l`
      <${he}
        title=${o("projects.empty.noneTitle")}
        description=${o("projects.empty.noneDesc")}
      >
        <${T} onClick=${s}>${o("projects.createFromChat")}<//>
      <//>
    `}function z2({threads:e,selectedThreadId:t,onSelectThread:a,onNewConversation:n,isStartingConversation:r}){let s=[...e].sort((i,o)=>new Date(o.updated_at||o.created_at)-new Date(i.updated_at||i.created_at));return l`
    <${U} className="p-4 sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Conversations</div>
          <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">Project conversations</h2>
        </div>
        ${n&&l`
          <${T} onClick=${n} disabled=${r}>
            ${r?"Starting\u2026":"New conversation"}
          <//>
        `}
      </div>

      <div className="mt-5 space-y-3">
        ${s.length?s.slice(0,18).map(i=>{let o=L2(i);return l`
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
                    <${j} tone=${O2(i.state)} label=${i.state} />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                    <span>${i.step_count||0} steps</span>
                    <span>${i.total_tokens||0} tokens</span>
                    <span>${Uc(i.updated_at||i.created_at)}</span>
                  </div>
                </button>
              `}):l`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                No project threads yet. When an automation runs or scoped chat work happens inside this project, activity will appear here.
              </div>
            `}
      </div>
    <//>
  `}var qA="/workspace";function BA(e){let t=a=>a.kind==="directory"?0:1;return[...e].sort((a,n)=>t(a)-t(n)||a.name.localeCompare(n.name,void 0,{sensitivity:"base"}))}function HA(e){return e?String(e).replace(/^\/workspace\/?/,"").split("/").filter(Boolean):[]}function q2({threadId:e}){let t=k(),[a,n]=h.default.useState(void 0),[r,s]=h.default.useState(null),i=z({queryKey:["project-files",e||"",a||""],queryFn:()=>cx({threadId:e,path:a}),enabled:!!e}),o=h.default.useMemo(()=>BA(i.data?.entries||[]),[i.data]),u=h.default.useCallback(async f=>{if(f.kind==="directory"){s(null),n(f.path);return}try{s(null);let m=await Sa(tc({threadId:e,path:f.path})),p=URL.createObjectURL(m),b=document.createElement("a");b.href=p,b.download=f.name,document.body.appendChild(b),b.click(),b.remove(),URL.revokeObjectURL(p)}catch(m){s(m?.message||"Unable to download file")}},[e]),c=HA(a),d=l`
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
          ${"Files"}
        </div>
        <${j} tone="muted" label=${t("workspace.readOnly")} />
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
  `;return e?l`
    <${U} className="p-4 sm:p-5">
      ${d}

      <div className="mt-3 flex min-w-0 flex-wrap items-center gap-1.5 font-mono text-xs text-iron-400">
        <button
          type="button"
          onClick=${()=>n(void 0)}
          className="text-signal hover:underline"
        >
          ${"workspace"}
        </button>
        ${c.map((f,m)=>{let p=`${qA}/${c.slice(0,m+1).join("/")}`;return l`
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
                  <${A}
                    name=${f.kind==="directory"?"folder":"file"}
                    className="h-4 w-4 shrink-0 text-iron-300"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm text-white">${f.name}</span>
                  ${f.kind==="directory"?l`<${A} name="chevron" className="h-3.5 w-3.5 shrink-0 -rotate-90 text-iron-500" />`:l`<${A} name="download" className="h-3.5 w-3.5 shrink-0 text-iron-500" />`}
                </button>
              `):l`
              <div className="rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                ${"This folder is empty."}
              </div>
            `}
      </div>
    <//>
  `:l`
      <${U} className="p-4 sm:p-5">
        ${d}
        <div className="mt-4 rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
          ${"No files yet \u2014 they appear once a thread has run in this project."}
        </div>
      <//>
    `}function KA(e){return[...e||[]].sort((a,n)=>new Date(n.updated_at||n.created_at)-new Date(a.updated_at||a.created_at))[0]?.id||null}function B2({project:e,threads:t,selectedThreadId:a,onSelectThread:n,onNewConversation:r,isStartingConversation:s}){let i=KA(t);return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]">
      <div className="space-y-5">
        <div className="min-w-0">
          <h2 className="text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
          ${e.description?l`<p className="mt-1 text-sm leading-6 text-iron-300">${e.description}</p>`:null}
        </div>

        <${z2}
          threads=${t}
          selectedThreadId=${a}
          onSelectThread=${n}
          onNewConversation=${r}
          isStartingConversation=${s}
        />
      </div>

      <${q2} threadId=${i} />
    </div>
  `}function Yo(){let e=k(),t=ce(),{threadsState:a}=Ba(),{projectId:n=null,threadId:r=null}=ot(),[s,i]=h.default.useState(""),[o,u]=h.default.useState(null),c=T2(),d=A2(n),f=D2({projectId:n,threadId:r}),m=h.default.useMemo(()=>{let _=s.trim().toLowerCase();return _?c.overview.projects.filter(E=>[E.name,E.description,...E.goals||[]].some(M=>String(M||"").toLowerCase().includes(_))):c.overview.projects},[c.overview.projects,s]),p=h.default.useMemo(()=>c.overview.projects.find(_=>_.id===n)||null,[c.overview.projects,n]),b=h.default.useCallback(()=>{c.invalidate(),d.invalidate()},[c,d]),y=h.default.useCallback(_=>{t(`/projects/${_}`)},[t]),$=h.default.useCallback(_=>{if(_.thread_id){t(`/projects/${_.project_id}/threads/${_.thread_id}`);return}t(`/projects/${_.project_id}`)},[t]),g=h.default.useCallback(async()=>{let _=null;u(null);try{_=await a.createThread()}catch(E){u({type:"error",message:E.message||e("projects.chatAutoFail")})}t("/chat",{state:{composerDraft:e("projects.creationDraft"),threadId:_}})},[t,a]),v=h.default.useCallback(_=>{t(`/projects/${n}/threads/${_}`)},[t,n]),x=h.default.useCallback(async()=>{u(null);try{let _=await a.createThread(n);t("/chat",{state:{threadId:_}}),d.invalidate()}catch(_){u({type:"error",message:_.message||e("projects.chatAutoFail")})}},[t,a,n,d,e]),w=h.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),S=l`
    ${n&&l`<${T} variant="ghost" onClick=${()=>t("/projects")}>${e("projects.allProjects")}<//>`}
  `,R=null;return n?d.isLoading?R=l`
        <div className="space-y-4">
          ${[1,2,3].map(_=>l`<div key=${_} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `:d.error||!d.project&&!p?R=l`
        <${he}
          title=${e("projects.unavailable")}
          description=${d.error?.message||e("projects.unavailableDesc")}
        >
          <${T} variant="secondary" onClick=${()=>t("/projects")}>${e("projects.returnToProjects")}<//>
        <//>
      `:R=l`
        <${B2}
          project=${d.project||p}
          threads=${d.threads}
          selectedThreadId=${r}
          onSelectThread=${v}
          onNewConversation=${x}
          isStartingConversation=${a.isCreating}
        />
      `:R=c.isLoading?l`
          <div className="space-y-4">
            ${[1,2,3].map(_=>l`<div key=${_} className="v2-skeleton h-40 rounded-[20px]" />`)}
          </div>
        `:l`
          <${F2}
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
          <${Qa} result=${o} onDismiss=${()=>u(null)} />
          <${Qa} result=${f.actionResult} onDismiss=${f.clearActionResult} />
          ${!n&&l`
            <${j2} overview=${c.overview} />
            <${U2} items=${c.overview.attention} onOpenItem=${$} />
          `}
          ${R}
        </div>
      </div>
    </div>
  `}function Jo(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not scheduled"}function Xo(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function H2(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function K2(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);return r!==0?r:new Date(n.updated_at||0).getTime()-new Date(a.updated_at||0).getTime()})}function zc({label:e,value:t}){return l`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function IA({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=k();return e.status==="Active"?l`
      <${T} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.fireNow")}<//>
      <${T} variant="secondary" onClick=${()=>n(e.id)} disabled=${t}>${s("missions.action.pause")}<//>
    `:e.status==="Paused"?l`
      <${T} onClick=${()=>r(e.id)} disabled=${t}>${s("missions.action.resume")}<//>
      <${T} variant="secondary" onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runOnce")}<//>
    `:l`<${T} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runAgain")}<//>`}function I2({mission:e,isLoading:t,error:a,isBusy:n,onFire:r,onPause:s,onResume:i,onOpenProject:o,onOpenThread:u}){let c=k();return t?l`
      <div className="space-y-4">
        ${[1,2,3].map(d=>l`<div key=${d} className="v2-skeleton h-36 rounded-xl" />`)}
      </div>
    `:a||!e?l`
      <${he}
        title=${c("missions.unavailable")}
        description=${a?.message||c("missions.unavailableDesc")}
      />
    `:l`
    <div className="space-y-4">
      <${U} className="p-4 sm:p-5">
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
          <${j} tone=${Xo(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${zc} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${zc} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${zc} label=${c("missions.meta.nextFire")} value=${Jo(e.next_fire_at)} />
          <${zc} label=${c("missions.meta.updated")} value=${Jo(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${IA}
            mission=${e}
            isBusy=${n}
            onFire=${r}
            onPause=${s}
            onResume=${i}
          />
        </div>
      <//>

      <${U} className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.brief")}</div>
        <div className="mt-4 text-sm leading-6 text-iron-200">
          <${na} content=${e.goal||c("missions.noGoal")} />
        </div>
      <//>

      ${e.current_focus&&l`
        <${U} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${na} content=${e.current_focus} />
          </div>
        <//>
      `}

      ${e.success_criteria&&l`
        <${U} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${na} content=${e.success_criteria} />
          </div>
        <//>
      `}

      ${e.threads?.length?l`
        <${U} className="p-4 sm:p-5">
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
                  <${j} tone=${Xo(d.state==="Running"?"Active":d.state==="Failed"?"Failed":"Completed")} label=${d.state} />
                </div>
              </button>
            `)}
          </div>
        <//>
      `:null}
    </div>
  `}function QA(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function Q2({value:e,onChange:t,children:a,label:n}){return l`
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
  `}function VA({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=k(),s=t===e.id;return l`
    <div
      className=${["w-full rounded-xl border p-4 text-left",s?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80"].join(" ")}
    >
      <button type="button" onClick=${()=>a(e.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${e.name}</div>
              <${j} tone=${Xo(e.status)} label=${e.status} />
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
          ${r("missions.updated",{value:Jo(e.updated_at)})}
        </span>
        <${T}
          variant="ghost"
          onClick=${i=>{i.stopPropagation(),n(e.project.id)}}
        >
          ${e.project.name}
        <//>
      </div>
    </div>
  `}function rh({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:u,projectOptions:c,onSelectMission:d,onOpenProject:f}){let m=k(),p=QA(m);return l`
    <${U} className="p-4 sm:p-5">
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
        <${Q2} value=${s} onChange=${i} label=${m("missions.filter.status")}>
          ${p.map(b=>l`<option key=${b.value} value=${b.value}>${b.label}<//>`)}
        <//>
        <${Q2} value=${o} onChange=${u} label=${m("missions.filter.project")}>
          <option value="all">${m("missions.filter.allProjects")}</option>
          ${c.map(b=>l`<option key=${b.id} value=${b.id}>${b.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(b=>l`
              <${VA}
                key=${b.id}
                mission=${b}
                selectedMissionId=${a}
                onSelectMission=${d}
                onOpenProject=${f}
              />
            `):l`
              <${he}
                title=${m("missions.emptyTitle")}
                description=${m("missions.emptyDesc")}
                boxed=${!1}
              />
            `}
      </div>
    <//>
  `}function GA(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function V2({summary:e}){let t=k(),a=GA(t);return l`
    <${U} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${j} tone=${n.tone} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${e[n.key]||0}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">
              ${n.key==="total"?t("missions.summary.completedFailed",{completed:e.completed||0,failed:e.failed||0}):t("missions.summary.acrossProjects")}
            </p>
          </div>
        `)}
      </div>
    <//>
  `}function G2(){return Promise.resolve({projects:[],todo:!0})}function Y2({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function J2(e){return Promise.resolve(null)}function X2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function Z2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function W2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function eS(e){let t=z({queryKey:["mission-detail",e],queryFn:()=>J2(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function YA(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function tS(){let e=X(),[t,a]=h.default.useState(null),n=z({queryKey:["projects-overview"],queryFn:G2,refetchInterval:7e3}),r=n.data?.projects||[],s=vd({queries:r.map(m=>({queryKey:["missions","project",m.id],queryFn:()=>Y2({projectId:m.id}),refetchInterval:5e3,select:p=>p?.missions||[]}))}),i=s.flatMap((m,p)=>{let b=r[p];return(m.data||[]).map(y=>YA(y,b))}),o=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),u=(m,p)=>({mutationFn:({missionId:b})=>m(b),onSuccess:()=>{a({type:"success",message:p}),o()},onError:b=>{a({type:"error",message:b.message||"Unable to update mission"})}}),c=Q(u(X2,"Mission fired and a run was queued.")),d=Q(u(Z2,"Mission paused.")),f=Q(u(W2,"Mission resumed."));return{projects:r,missions:i,summary:H2(i),isLoading:n.isLoading||s.some(m=>m.isLoading),isRefreshing:n.isFetching||s.some(m=>m.isFetching),error:n.error||s.find(m=>m.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:f.mutateAsync,isBusy:c.isPending||d.isPending||f.isPending,invalidate:o}}function sh(){let e=k(),t=ce(),{missionId:a=null}=ot(),[n,r]=h.default.useState(""),[s,i]=h.default.useState("all"),[o,u]=h.default.useState("all"),c=tS(),d=eS(a),f=h.default.useMemo(()=>{let g=n.trim().toLowerCase();return K2(c.missions).filter(v=>{let x=!g||[v.name,v.goal,v.project?.name].some(R=>String(R||"").toLowerCase().includes(g)),w=s==="all"||v.status===s,S=o==="all"||v.project?.id===o;return x&&w&&S})},[c.missions,o,n,s]),m=h.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),p=d.mission?{...m,...d.mission,project:m?.project||null}:m,b=h.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),y=h.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),$=a?l`
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <${rh}
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
          <${I2}
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
        <${rh}
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
            <${T}
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

          <${Qa}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${V2} summary=${c.summary} />

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
  `}var aS=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],JA=new Set(["pending","in_progress"]),nS=new Set(["failed","interrupted","stuck","cancelled"]);function er(e){return e?String(e).replace(/_/g," "):"unknown"}function ii(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":nS.has(e)?"danger":"muted":"muted"}function XA(e){return JA.has(e)}function qc(e){return XA(e?.state)}function rS(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":nS.has(e.state):!1}function Ur(e,t=8){return e?String(e).slice(0,t):"unknown"}function ra(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function sS(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function ih(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${ra(e.started_at)}`:null].filter(Boolean).join(" / ")}var ZA=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function iS(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function WA({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?l`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${t==="tool_use"?a.tool_name||"Tool call":a.tool_name||"Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${iS(t==="tool_use"?a.input:a.output||a.error||a)}</pre>
      </details>
    `:t==="message"?l`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a.role||"assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${a.content||""}</div>
      </div>
    `:l`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t.replace(/_/g," ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${a.message||a.status||iS(a)}</div>
    </div>
  `}function oS({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=k(),[s,i]=h.default.useState("all"),[o,u]=h.default.useState(""),[c,d]=h.default.useState(!0),f=h.default.useRef(null),m=h.default.useMemo(()=>s==="all"?t:t.filter(b=>b.event_type===s),[t,s]);h.default.useEffect(()=>{c&&f.current&&(f.current.scrollTop=f.current.scrollHeight)},[c,m.length]);let p=h.default.useCallback(async(b=!1)=>{let y=o.trim();if(!(!y&&!b))try{await a({content:y||"(done)",done:b}),u("")}catch{}},[o,a]);return l`
    <${U} className="p-5 sm:p-6">
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
            ${ZA.map(b=>l`<option key=${b.value} value=${b.value}>${b.label}</option>`)}
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
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${ra(b.created_at)}</div>
                <${WA} event=${b} />
              </div>
            `):l`
              <${he}
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
          <${T} variant="secondary" disabled=${n} onClick=${()=>p(!0)}>${r("common.done")}<//>
          <${T} variant="primary" disabled=${n} onClick=${()=>p(!1)}>${r("common.send")}<//>
        </div>
      `}
    <//>
  `}function lS({job:e,activeTab:t,onTabChange:a,onBack:n,onCancel:r,onRestart:s,isBusy:i,children:o}){return l`
    <div className="space-y-5">
      <${U} className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick=${n} className="text-sm text-signal hover:text-white">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-semibold tracking-tight text-white">${e.title||"Untitled job"}</h2>
              <${j} tone=${ii(e.state)} label=${er(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${Ur(e.id)}</span>
              <span>created ${ra(e.created_at)}</span>
              ${ih(e)&&l`<span>${ih(e)}</span>`}
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
            ${qc(e)&&l`
              <${T} variant="secondary" disabled=${i} onClick=${()=>r(e.id)}>Cancel<//>
            `}
            ${rS(e)&&l`
              <${T} variant="primary" disabled=${i} onClick=${()=>s(e.id)}>Restart<//>
            `}
          </div>
        </div>
      <//>

      <div className="flex flex-wrap gap-2">
        ${aS.map(u=>l`
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
  `}function uS({nodes:e,depth:t=0,selectedPath:a,expandingPath:n,onToggleDirectory:r,onSelectPath:s}){return l`
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
        ${i.isDir&&i.expanded&&i.children?.length?l`<${uS}
              nodes=${i.children}
              depth=${t+1}
              selectedPath=${a}
              expandingPath=${n}
              onToggleDirectory=${r}
              onSelectPath=${s}
            />`:null}
      </div>
    `)}
  `}function cS({canBrowse:e,tree:t,selectedPath:a,selectedFile:n,fileError:r,isLoadingTree:s,isLoadingFile:i,expandingPath:o,treeError:u,onToggleDirectory:c,onSelectPath:d}){return e?l`
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <${U} className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          ${u&&l`<div className="mx-2 mb-3 rounded-md border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${u}</div>`}
          ${s?l`<div className="space-y-2 px-2">${[1,2,3,4].map(f=>l`<div key=${f} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?l`
                  <${uS}
                    nodes=${t}
                    selectedPath=${a}
                    expandingPath=${o}
                    onToggleDirectory=${c}
                    onSelectPath=${d}
                  />
                `:l`<div className="px-2 py-6 text-sm text-iron-300">No files were recorded for this workspace.</div>`}
        </div>
      <//>

      <${U} className="min-h-[440px] p-5 sm:p-6">
        <div className="border-b border-white/10 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">File preview</div>
          <p className="mt-2 break-all text-sm leading-6 text-iron-300">${n?.path||a||"Select a file from the tree to inspect its contents."}</p>
        </div>

        ${r&&!i?l`<div className="mt-5 rounded-md border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">${r}</div>`:i?l`<div className="mt-5 space-y-3">${[1,2,3,4,5].map(f=>l`<div key=${f} className="v2-skeleton h-4 rounded" />`)}</div>`:n?l`<pre className="mt-5 max-h-[60vh] overflow-auto whitespace-pre-wrap rounded-[18px] border border-white/10 bg-iron-950/90 p-4 font-mono text-xs leading-6 text-iron-100">${n.content}</pre>`:l`
                <${he}
                  title="No file selected"
                  description="Pick a concrete file from the workspace tree to render it here."
                />
              `}
      <//>
    </div>
  `:l`
      <${he}
        title="No project workspace"
        description="File browsing is only available for sandbox jobs that produced a mounted project directory."
      />
    `}function oi({label:e,value:t}){return l`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function dS({job:e}){let t=(e.transitions||[]).map(a=>({title:`${er(a.from)} -> ${er(a.to)}`,description:[ra(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${U} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${j} tone=${ii(e.state)} label=${er(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${oi} label="Created" value=${ra(e.created_at)} />
          <${oi} label="Started" value=${ra(e.started_at)} />
          <${oi} label="Completed" value=${ra(e.completed_at)} />
          <${oi} label="Duration" value=${sS(e.elapsed_secs)} />
          <${oi} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${oi} label="Mode" value=${e.job_mode||"Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${U} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${e.description?l`<${na} content=${e.description} className="mt-4 text-sm leading-7 text-iron-200" />`:l`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
        <//>

        ${t.length?l`
              <${U} className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Transitions</div>
                <h3 className="mt-2 text-xl font-semibold text-white">State timeline</h3>
                <div className="mt-3">
                  <${s2} items=${t} />
                </div>
              <//>
            `:l`
              <${he}
                title="No state history yet"
                description="Transitions appear here once the job advances or records a recovery event."
              />
            `}
      </div>
    </div>
  `}function mS({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:u,isBusy:c,isRefreshing:d}){let f=k(),m=[{value:"all",label:f("jobs.list.filter.all")},{value:"pending",label:f("jobs.list.filter.pending")},{value:"in_progress",label:f("jobs.list.filter.inProgress")},{value:"completed",label:f("jobs.list.filter.completed")},{value:"failed",label:f("jobs.list.filter.failed")},{value:"stuck",label:f("jobs.list.filter.stuck")}];if(!e.length){let p=!!n.trim()||s!=="all";return l`
      <${he}
        title=${f(t&&p?"jobs.list.empty.noMatchTitle":"jobs.list.empty.noJobsTitle")}
        description=${f(t&&p?"jobs.list.empty.noMatchDesc":"jobs.list.empty.noJobsDesc")}
      />
    `}return l`
    <div className="space-y-5">
      <${U} className="p-4 sm:p-5">
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
                  <${j} tone=${ii(p.state)} label=${er(p.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${Ur(p.id)}</span>
                  <span>${f("jobs.list.created",{value:ra(p.created_at)})}</span>
                  ${p.started_at&&l`<span>${f("jobs.list.started",{value:ra(p.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${qc(p)&&l`
                  <${T}
                    variant="secondary"
                    className="h-9 px-3 text-xs"
                    disabled=${c}
                    onClick=${()=>u(p.id)}
                  >
                    ${f("jobs.action.cancel")}
                  <//>
                `}
                <${T} variant="ghost" className="h-9 px-3 text-xs" onClick=${()=>o(p.id)}>${f("jobs.action.open")}<//>
              </div>
            </div>
          </article>
        `)}
      </div>
    </div>
  `}var e5=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function fS({summary:e}){return l`
    <${U} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${e5.map(t=>l`
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
  `}function pS(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function hS(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function vS(e){return Promise.resolve(null)}function gS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function yS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function bS(e){return Promise.resolve({events:[],todo:!0})}function xS(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function oh(e,t=""){return Promise.resolve({entries:[],todo:!0})}function $S(e,t){return Promise.resolve({content:"",todo:!0})}function wS(e){let t=X(),[a,n]=h.default.useState(null),r=z({queryKey:["job-detail",e],queryFn:()=>vS(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=z({queryKey:["job-events",e],queryFn:()=>bS(e),enabled:!!e,refetchInterval:e?2500:!1}),i=Q({mutationFn:({content:o,done:u})=>xS(e,{content:o,done:u}),onSuccess:(o,{done:u})=>{n({type:"success",message:u?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return h.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function SS(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function NS(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=NS(a.children,t);if(n)return n}}return null}function Bc(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:Bc(n.children,t,a)}:n)}function _S(e){let[t,a]=h.default.useState([]),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState(""),c=!!(e?.project_dir&&e?.id),d=z({queryKey:["job-files-root",e?.id],queryFn:()=>oh(e.id,""),enabled:c}),f=z({queryKey:["job-file",e?.id,n],queryFn:()=>$S(e.id,n),enabled:!!(c&&n)});h.default.useEffect(()=>{a([]),r(""),i(""),u("")},[e?.id]),h.default.useEffect(()=>{d.data?.entries?(a(SS(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let m=h.default.useCallback(async p=>{let b=NS(t,p);if(!(!b||!e?.id)){if(b.expanded){a(y=>Bc(y,p,$=>({...$,expanded:!1})));return}if(b.loaded){a(y=>Bc(y,p,$=>({...$,expanded:!0})));return}u(p);try{let y=await oh(e.id,p);a($=>Bc($,p,g=>({...g,expanded:!0,loaded:!0,children:SS(y.entries)}))),i("")}catch(y){i(y.message||"Unable to open folder")}finally{u("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:f.data||null,fileError:f.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:f.isLoading||f.isFetching,expandingPath:o,treeError:s,toggleDirectory:m}}function kS(){let e=X(),[t,a]=h.default.useState(null),n=z({queryKey:["jobs-summary"],queryFn:hS,refetchInterval:5e3}),r=z({queryKey:["jobs"],queryFn:pS,refetchInterval:5e3}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=Q({mutationFn:({jobId:u})=>gS(u),onSuccess:(u,{jobId:c})=>{a({type:"success",message:`Job ${Ur(c)} cancelled`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to cancel job"})}}),o=Q({mutationFn:({jobId:u})=>yS(u),onSuccess:u=>{a({type:"success",message:`Restart queued as ${Ur(u?.new_job_id)}`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function RS({result:e,onDismiss:t}){let a=k();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return l`
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
  `}function lh(){let e=k(),t=ce(),{jobId:a=null}=ot(),[n,r]=h.default.useState(""),[s,i]=h.default.useState("all"),[o,u]=h.default.useState(a?"activity":"overview"),c=kS(),d=wS(a),f=_S(d.job);h.default.useEffect(()=>{u(a?"activity":"overview")},[a]);let m=h.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(x=>{let w=!v||x.title.toLowerCase().includes(v)||x.id.toLowerCase().includes(v),S=s==="all"||x.state===s;return w&&S})},[c.jobs,n,s]),p=h.default.useCallback(v=>t(`/jobs/${v}`),[t]),b=h.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),y=h.default.useCallback(async v=>{try{let x=await c.restartJob({jobId:v});x?.new_job_id&&t(`/jobs/${x.new_job_id}`)}catch{}},[c,t]),$=l`
    ${a&&l`<${T} variant="ghost" onClick=${()=>t("/jobs")}
      >${e("jobs.allJobs")}<//
    >`}
  `,g=null;if(a)if(d.isLoading)g=l`
        <div className="space-y-4">
          ${[1,2,3].map(v=>l`<div key=${v} className="v2-skeleton h-32 rounded-[18px]" />`)}
        </div>
      `;else if(d.error||!d.job)g=l`
        <${he}
          title=${e("jobs.unavailable")}
          description=${d.error?.message||e("jobs.unavailableDesc")}
        >
          <${T} variant="secondary" onClick=${()=>t("/jobs")}
            >${e("jobs.returnToJobs")}<//
          >
        <//>
      `;else{let v={overview:l`<${dS} job=${d.job} />`,activity:l`
          <${oS}
            job=${d.job}
            events=${d.events}
            onSendPrompt=${d.sendPrompt}
            isSendingPrompt=${d.isSendingPrompt}
          />
        `,files:l`
          <${cS}
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
        <${lS}
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
          <${mS}
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
          <${RS}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${RS}
            result=${d.promptResult}
            onDismiss=${d.clearPromptResult}
          />
          <${fS} summary=${c.summary} />
          ${g}
        </div>
      </div>
    </div>
  `}function tr(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not scheduled"}function Hc(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function Kc(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function CS(e=[]){return[...e].sort((t,a)=>t.enabled!==a.enabled?t.enabled?-1:1:new Date(a.next_fire_at||a.last_run_at||0).getTime()-new Date(t.next_fire_at||t.last_run_at||0).getTime())}function ES(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function t5(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function TS({runs:e}){return e?.length?l`
    <div className="space-y-3">
      ${e.map(t=>l`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${j} tone=${t5(t.status)} label=${t.status} />
              <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                ${tr(t.started_at)}
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
    `}function ar({label:e,value:t}){return l`
    <div className="rounded-xl border border-iron-700 bg-iron-950/50 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div className="mt-2 min-w-0 break-words text-sm text-iron-100">
        ${t||"\u2014"}
      </div>
    </div>
  `}function AS({title:e,value:t}){return l`
    <div>
      <h3 className="text-sm font-semibold text-iron-100">${e}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >${JSON.stringify(t||{},null,2)}</pre>
    </div>
  `}function DS({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=ce(),u=k();return t?l`
      <div className="space-y-4">
        ${[1,2,3].map(c=>l`<div key=${c} className="v2-skeleton h-32 rounded-xl" />`)}
      </div>
    `:a||!e?l`
      <${he}
        title=${u("routine.unavailable")}
        description=${a?.message||u("routine.unavailableDesc")}
      />
    `:l`
    <${U} className="p-4 sm:p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="truncate text-2xl font-semibold tracking-tight text-iron-100">
              ${e.name}
            </h2>
            <${j}
              tone=${Hc(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${j}
              tone=${Kc(e.verification_status)}
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
        <${ar} label="Trigger" value=${e.trigger_summary||e.trigger_type} />
        <${ar} label="Action" value=${ES(e.action)} />
        <${ar} label="Next fire" value=${tr(e.next_fire_at)} />
        <${ar} label="Last run" value=${tr(e.last_run_at)} />
        <${ar} label="Run count" value=${e.run_count} />
        <${ar} label="Failures" value=${e.consecutive_failures} />
        <${ar} label="Created" value=${tr(e.created_at)} />
        <${ar} label="Routine ID" value=${e.id} />
      </div>

      ${e.conversation_id&&l`
        <div className="mt-5">
          <${T} variant="secondary" onClick=${()=>o(`/chat/${e.conversation_id}`)}>
            Open routine thread
          <//>
        </div>
      `}

      <div className="mt-6 grid gap-6 xl:grid-cols-2">
        <${AS} title=${u("routine.triggerPayload")} value=${e.trigger} />
        <${AS} title=${u("routine.actionPayload")} value=${e.action} />
      </div>

      <div className="mt-6">
        <h3 className="mb-3 text-sm font-semibold text-iron-100">Recent runs</h3>
        <${TS} runs=${e.recent_runs} />
      </div>
    <//>
  `}function MS({routine:e,selectedRoutineId:t,onSelectRoutine:a,onTriggerRoutine:n,onToggleRoutine:r,isBusy:s}){let i=t===e.id;return l`
    <article
      className=${["group flex flex-col gap-4 rounded-[18px] border p-5",i?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick=${()=>a(e.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">${e.name}</h3>
            <${j}
              tone=${Hc(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${j}
              tone=${Kc(e.verification_status)}
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
            <span>next ${tr(e.next_fire_at)}</span>
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
  `}var a5=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function uh({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:u,onToggleRoutine:c,isBusy:d,isRefreshing:f}){let m=k();if(!e.length){let p=!!n.trim()||s!=="all";return l`
      <${he}
        title=${t&&p?"No routines match":"No routines yet"}
        description=${t&&p?"Adjust the search or status filter to find a saved routine.":"Routines created from chat will appear here after they are saved."}
      />
    `}return l`
    <div className="space-y-5">
      <${U} className="p-4 sm:p-5">
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
            ${a5.map(p=>l`<option key=${p.value} value=${p.value}>${p.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(p=>l`
            <${MS}
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
  `}var n5=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function OS({summary:e}){return l`
    <${U} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${n5.map(t=>l`
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
  `}function LS(e){let[t,a]=h.default.useState(""),[n,r]=h.default.useState("all");return{filteredRoutines:h.default.useMemo(()=>{let i=t.trim().toLowerCase();return CS(e).filter(o=>{let u=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||u.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function PS(){return Promise.resolve({routines:[],todo:!0})}function jS(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function US(e){return Promise.resolve(null)}function Ic(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function Qc(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function FS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function zS(e){let t=X(),[a,n]=h.default.useState(null),r=z({queryKey:["routine-detail",e],queryFn:()=>US(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:f=>{n({type:"error",message:f.message||"Unable to update routine"})}}),o=Q(i(Ic,"Routine run queued.")),u=Q(i(Qc,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,isBusy:o.isPending||u.isPending}}function qS(){let e=X(),[t,a]=h.default.useState(null),n=z({queryKey:["routines-summary"],queryFn:jS,refetchInterval:5e3}),r=z({queryKey:["routines"],queryFn:PS,refetchInterval:5e3}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,f)=>({mutationFn:({routineId:m})=>d(m),onSuccess:()=>{a({type:"success",message:f}),s()},onError:m=>{a({type:"error",message:m.message||"Unable to update routine"})}}),o=Q(i(Ic,"Routine run queued.")),u=Q(i(Qc,"Routine status updated.")),c=Q(i(FS,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||u.isPending||c.isPending,invalidate:s}}function ch(){let e=ce(),{routineId:t=null}=ot(),a=qS(),n=zS(t),r=LS(a.routines),s=h.default.useCallback(async(u,c)=>{try{await u({routineId:c})}catch{}},[]),i=h.default.useCallback(async(u,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:u}),e("/routines")}catch{}},[e,a]),o=t?l`
        <div className="grid gap-5 xl:grid-cols-[minmax(0,0.9fr)_minmax(440px,1.1fr)]">
          <${uh}
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
          <${DS}
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
        <${uh}
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
            <${T} variant="ghost" onClick=${()=>e("/routines")}>
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

          <${Qa}
            result=${a.actionResult}
            onDismiss=${a.clearActionResult}
          />
          <${Qa}
            result=${n.actionResult}
            onDismiss=${n.clearActionResult}
          />
          <${OS} summary=${a.summary} />

          ${a.isLoading?l`
                <div className="space-y-4">
                  ${[1,2,3].map(u=>l`<div key=${u} className="v2-skeleton h-32 rounded-xl" />`)}
                </div>
              `:o}
        </div>
      </div>
    </div>
  `}function r5(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function s5(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function BS({deliveryState:e}){let t=k(),a=e.currentTarget?.target_id||"",[n,r]=h.default.useState(a),[s,i]=h.default.useState(!1),o=h.default.useRef(null);h.default.useEffect(()=>{r(a)},[a]),h.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let u=n!==a,c=e.isLoading||e.isSaving,d=u&&!c,f=!!a&&!c,m=e.finalReplyTargets.length>0,p=e.targets.some(M=>M?.capabilities?.final_replies&&M?.target?.status==="unavailable"),b=m||p,y=M=>(o.current&&clearTimeout(o.current),i(!1),M.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),$=()=>{d&&y(e.saveFinalReplyTarget(n||null))},g=()=>{f&&(r(""),y(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),x=e.currentStatus,w=x==="available"?"success":x==="unavailable"?"warning":"muted",S=t(x==="available"?"automations.delivery.pill.ready":x==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),R=!!e.currentTarget,_=t(R?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),E=s5(t("automations.delivery.footnote"),{command:l`<code
        key="cmd"
        className="rounded px-1.5 py-0.5 font-mono text-[0.6875rem] bg-[var(--v2-surface-muted)] text-[var(--v2-accent-text)]"
      >
        approve &lt;code&gt;
      </code>`});return l`
    <${U} className="p-5 sm:p-6">
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
              <${j} tone=${w} label=${S} />
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
            ${e.finalReplyTargets.map(M=>{let P=M?.target?.target_id??"",I=M?.target?.display_name||M?.target?.target_id||"",D=M?.target?.description||"",K=M?.target?.status??"available",J=n===P;return l`
                <label
                  key=${P}
                  className=${H("flex items-start gap-3.5 rounded-xl border px-4 py-3.5 cursor-pointer","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]","hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",J&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
                >
                  <input
                    type="radio"
                    name="delivery-target"
                    value=${P}
                    checked=${J}
                    disabled=${c}
                    onChange=${()=>r(P)}
                    className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-semibold text-[var(--v2-text-strong)] leading-snug">
                      ${I}
                    </div>
                    ${D&&l`<div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                      ${D}
                    </div>`}
                  </div>
                  <${j}
                    tone=${r5(K)}
                    label=${t(K==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.ready")}
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
                <${j}
                  tone="warning"
                  label=${t("automations.delivery.pill.notPaired")}
                  className="shrink-0"
                />
              </div>
            `}

            <!-- Web app only / fallback row -->
            <label
              className=${H("flex items-start gap-3.5 rounded-xl border px-4 py-3.5","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]",m?"cursor-pointer hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]":"cursor-default",n===""&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
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
              <${j}
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
            <${A} name="check" className="h-3.5 w-3.5" />
            ${t("automations.delivery.save")}
          <//>
          <${T}
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
              <${A} name="check" className="h-3 w-3" />
              ${t("automations.delivery.saved")}
            </span>
          `}
          ${e.saveError&&!s&&l`
            <span
              role="alert"
              className="flex items-center gap-1.5 text-xs font-semibold text-red-300"
            >
              <${A} name="close" className="h-3 w-3" />
              ${t("automations.delivery.saveFailed")}
            </span>
          `}
        </div>

        <!-- ── Footnote (only when an external Slack-style target exists) ── -->
        ${b&&l`
          <div
            className="rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-faint)]"
          >
            ${E}
          </div>
        `}

      </div>
    <//>
  `}var i5=["schedule","once"],KS={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},IS={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},QS={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function ka(e){return typeof e=="function"?e:t=>t}var mh=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:Zo},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:$5},{value:"completed",labelKey:"automations.filter.completed",predicate:w5}];function VS(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>i5.includes(r?.source?.type)).map(r=>v5(r,t,a)).sort(x5)}function GS(e,t){let a=mh.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function YS(e){let t=e.filter(i=>i.state!=="completed"),a=t.filter(i=>Zo(i)).length,n=t.filter(i=>i.has_running_run).length,r=t.filter(i=>i.has_failed_runs).length,s=t.filter(i=>Zo(i)&&dh(i)!=null).sort((i,o)=>(i.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(o.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:t.length,active:a,running:n,failures:r,nextRun:s?.next_run_label||null}}function o5(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=k5(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:u,month:c,dayOfWeek:d,year:f}=s,m=t&&typeof t=="string"?t:null,p=m?` (${m})`:"",b=f==="*"&&u==="*"&&c==="*"&&d==="*";if(b&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=R5(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(nr(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let y=S5(o,i,n);if(!y)return r("automations.schedule.custom");if(b)return r("automations.schedule.everyDayAt",{time:y})+p;let $=C5(d);if(f==="*"&&u==="*"&&c==="*"&&$==="1-5")return r("automations.schedule.weekdaysAt",{time:y})+p;if(f==="*"&&u==="*"&&c==="*"&&nr($,0,7)){let g=N5(Number($)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:y})+p}if(f==="*"&&nr(u,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(u),time:y})+p;if(nr(u,1,31)&&nr(c,1,12)&&d==="*"&&(f==="*"||nr(f,1970,9999))){let g=_5(Number(c),Number(u),f==="*"?null:Number(f),n);return r("automations.schedule.dateAt",{date:g,time:y})+p}return r("automations.schedule.custom")}function Fr(e,t="Unknown",a,n){if(!e)return t;let r=new Date(e);if(Number.isNaN(r.getTime()))return t;let s=n&&typeof n=="string"?{timeZone:n}:{};try{return r.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...s})}catch{return r.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function l5(e,t){let a=KS[e]?.labelKey||"automations.state.unknown";return ka(t)(a)}function u5(e){return KS[e]?.tone||"muted"}function c5(e,t){let a=IS[e]?.labelKey||"automations.lastStatus.none";return ka(t)(a)}function d5(e){return IS[e]?.tone||"muted"}function m5(e,t){let a=QS[Vc(e)]?.labelKey||"automations.runStatus.unknown";return ka(t)(a)}function f5(e){return QS[Vc(e)]?.tone||"muted"}function p5(e,t,a,n){if(!e)return ka(a)("automations.schedule.custom");let r=Fr(e,null,n,t);if(!r)return ka(a)("automations.schedule.custom");let s=t&&typeof t=="string"?` (${t})`:"";return ka(a)("automations.schedule.onceAt",{datetime:r})+s}function h5(e,t,a){return e?.type==="once"?p5(e.at,e.timezone,t,a):e?.type==="schedule"?o5(e.cron,e.timezone||"UTC",t,a):ka(t)("automations.schedule.custom")}function v5(e,t,a){let n=ka(t),r=g5(e.recent_runs,t,a),s=r[0]||null,i=r.find(d=>d.status==="running")||null,o=r.find(d=>d.status==="ok"||d.status==="error")||null,u=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null;return{...e,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:h5(e.source,t,a),state_label:l5(e.state,t),state_tone:u5(e.state),next_run_timestamp:fh(e.next_run_at),next_run_label:Fr(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:Fr(c,n("automations.date.noRuns"),a),last_status_label:c5(u,t),last_status_tone:d5(u),created_label:Fr(e.created_at,n("automations.date.unknown"),a),recent_runs:r,latest_run:s,current_run:i,has_running_run:r.some(d=>d.status==="running"),has_failed_runs:r.some(d=>d.status==="error"),success_rate_label:b5(r,t)}}function g5(e,t,a){let n=ka(t);return Array.isArray(e)?e.map(r=>{let s=Vc(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=fh(i);return{...r,status:s,status_label:m5(s,t),status_tone:f5(s),timestamp:o,timestamp_source:i,fired_label:Fr(i,n("automations.date.unscheduled"),a),submitted_label:Fr(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:Fr(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function Vc(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function JS(e){let t=Array.isArray(e)?e:[],a={total:t.length,ok:0,error:0,running:0,unknown:0};for(let n of t)a[Vc(n?.status)]+=1;return a}function y5(e){let t=JS(e);return[{key:"ok",tone:"text-emerald-300",count:t.ok},{key:"error",tone:"text-red-300",count:t.error},{key:"running",tone:"text-sky-300",count:t.running},{key:"unknown",tone:"text-iron-400",count:t.unknown}].filter(a=>a.count>0)}function XS(e,t){let a=ka(t),n=JS(e),r=y5(e).map(s=>({...s,text:a(`automations.runs.${s.key}`,{count:s.count})}));return{total:n.total,totalText:a("automations.runs.total",{count:n.total}),chips:r}}function b5(e,t){let a=ka(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function x5(e,t){let a=Zo(e),n=Zo(t);return a!==n?a?-1:1:(dh(e)??Number.MAX_SAFE_INTEGER)-(dh(t)??Number.MAX_SAFE_INTEGER)}function fh(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function Zo(e){return e?.state==="active"||e?.state==="scheduled"}function $5(e){return["paused","disabled","inactive"].includes(e?.state)}function w5(e){return e?.state==="completed"}function dh(e){return e?.next_run_timestamp??fh(e?.next_run_at)}function ph(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function S5(e,t,a){return!nr(e,0,23)||!nr(t,0,59)?null:ph(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function N5(e,t){return ph(t,{weekday:"long"},new Date(2001,0,7+e))}function _5(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return ph(n,r,new Date(a??2e3,e-1,t))}function k5(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&HS(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&HS(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function HS(e){return/^0+$/.test(e)}function nr(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function R5(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function C5(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}function E5(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function ZS({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let u=o.toString(),c=`/logs${u?`?${u}`:""}`;return i?`/v2${c}`:c}function WS(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(E5),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}var T5=8;function hh(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function Gc({runs:e=[]}){let t=k(),a=e.slice(0,T5);if(!a.length)return l`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;let n=e.length-a.length;return l`
    <div
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf",{shown:a.length,total:e.length})}
    >
      ${a.map(r=>l`
        <span
          key=${hh(r)}
          title=${`${r.status_label} \xB7 ${r.fired_label}`}
          className=${H("h-3 w-3 rounded-full border",r.status==="ok"&&"border-emerald-300/50 bg-emerald-400",r.status==="error"&&"border-red-300/50 bg-red-400",r.status==="running"&&"border-sky-300/60 bg-sky-400",r.status==="unknown"&&"border-iron-500 bg-iron-600")}
        />
      `)}
      ${n>0&&l`<span
        className="ml-0.5 font-mono text-[11px] text-iron-400"
        title=${t("automations.runs.showingOf",{shown:a.length,total:e.length})}
      >
        +${n}
      </span>`}
    </div>
  `}function Yc({runs:e=[],className:t=""}){let a=k(),n=XS(e,a);return n.total?l`
    <div className=${H("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]",t)}>
      <span className="text-iron-300">${n.totalText}</span>
      ${n.chips.map(r=>l`<span key=${r.key} className=${r.tone}>${r.text}</span>`)}
    </div>
  `:l`<span className=${H("text-[11px] text-iron-400",t)}>
      ${a("automations.table.noRuns")}
    </span>`}function eN({run:e,onOpenRun:t,onOpenLogs:a}){let n=k(),r=!!e.chat_path,s=ZS({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return l`
    <div className="grid gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto] sm:items-center">
      <div>
        <${j} tone=${e.status_tone} label=${e.status_label} />
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
        <${T}
          variant="secondary"
          size="sm"
          disabled=${!r}
          onClick=${r?()=>t(e.chat_path):void 0}
        >
          <${A} name="chat" className="mr-1.5 h-4 w-4" />
          ${n("automations.detail.openRun")}
        <//>
        <${T}
          variant="ghost"
          size="sm"
          disabled=${!i}
          onClick=${i?()=>a(s):void 0}
        >
          <${A} name="file" className="mr-1.5 h-4 w-4" />
          ${n("nav.logs")}
        <//>
      </div>
    </div>
  `}function Jc({label:e,value:t,tone:a}){return l`
    <div className="min-w-0 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div
        className=${H("mt-2 min-w-0 break-words text-sm text-iron-100",a==="success"&&"text-emerald-200",a==="danger"&&"text-red-200",a==="info"&&"text-sky-200")}
      >
        ${t||"\u2014"}
      </div>
    </div>
  `}function tN({automation:e}){let t=k(),a=ce();if(!e)return l`
      <${U} className="p-4 sm:p-5">
        <${he}
          boxed=${!1}
          title=${t("automations.detail.emptyTitle")}
          description=${t("automations.detail.emptyDescription")}
        />
      <//>
    `;let n=e.current_run;return l`
    <${U} className="overflow-hidden">
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
          <${j}
            tone=${e.has_running_run?"info":e.state_tone}
            label=${e.has_running_run?t("automations.status.running"):e.state_label}
          />
        </div>
      </div>

      <div className="space-y-5 p-4 sm:p-5">
        <div className="grid gap-3 sm:grid-cols-2">
          <${Jc} label=${t("automations.detail.schedule")} value=${e.schedule_label} />
          <${Jc}
            label=${t("automations.detail.successRate")}
            value=${e.success_rate_label}
            tone=${e.has_failed_runs?"danger":"success"}
          />
          <${Jc} label=${t("automations.detail.lastCompleted")} value=${e.last_run_label} />
          <${Jc}
            label=${t("automations.detail.currentRun")}
            value=${n?.run_id||n?.thread_id||t("automations.detail.noCurrentRun")}
            tone=${e.has_running_run?"info":null}
          />
        </div>

        <div>
          <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
            <h4 className="text-sm font-semibold text-iron-100">
              ${t("automations.detail.recentRuns")}
            </h4>
            <div className="flex flex-col items-end gap-1">
              <${Gc} runs=${e.recent_runs} />
              <${Yc} runs=${e.recent_runs} />
            </div>
          </div>

          ${e.recent_runs.length?l`
                <div>
                  ${e.recent_runs.map(r=>l`
                    <${eN}
                      key=${hh(r)}
                      run=${r}
                      onOpenRun=${a}
                      onOpenLogs=${a}
                    />
                  `)}
                </div>
              `:l`
                <div className="rounded-xl border border-dashed border-[var(--v2-panel-border)] p-4 text-sm text-iron-300">
                  ${t("automations.detail.noRuns")}
                </div>
              `}
        </div>
      </div>
    <//>
  `}var A5=["automations.empty.example1","automations.empty.example2","automations.empty.example3"];function D5({promptKey:e}){let t=k(),a=t(e),[n,r]=h.default.useState(!1),s=h.default.useRef(null);return h.default.useEffect(()=>()=>clearTimeout(s.current),[]),l`
    <li
      className="flex items-center gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3"
    >
      <span className="min-w-0 flex-1 text-sm leading-6 text-iron-200">${a}</span>
      <button
        type="button"
        onClick=${async()=>{try{await navigator.clipboard.writeText(a),r(!0),clearTimeout(s.current),s.current=setTimeout(()=>r(!1),1500)}catch{}}}
        aria-label=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        title=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        className=${H("inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-[var(--v2-panel-border)] text-iron-300 hover:text-iron-100 hover:border-white/20","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",n&&"text-emerald-300")}
      >
        <${A} name=${n?"check":"copy"} className="h-4 w-4" />
      </button>
    </li>
  `}function aN(){let e=k(),t=ce();return l`
    <${U} className="p-6 sm:p-8">
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
            ${A5.map(a=>l`<${D5} key=${a} promptKey=${a} />`)}
          </ul>
        </div>

        <div className="mt-6">
          <${T} variant="primary" size="sm" onClick=${()=>t("/chat")}>
            <${A} name="chat" className="mr-1.5 h-4 w-4" />
            ${e("automations.empty.startInChat")}
          <//>
        </div>
      </div>
    <//>
  `}function nN({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,selectedAutomationId:s,onSelectAutomation:i}){let o=k(),u=GS(e,t),c=e.length>0,d=u.find(f=>f.automation_id===s)||u[0]||null;return l`
    <div className="space-y-5">
      <${U} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
              ${o("automations.eyebrow")}
            </div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-iron-100">
              ${o("automations.title")}
            </h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${o("automations.description")}
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <div
              className="inline-flex overflow-hidden rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]"
              role="group"
              aria-label=${o("automations.filterLabel")}
            >
              ${mh.map(f=>l`
                <button
                  key=${f.value}
                  type="button"
                  aria-pressed=${t===f.value}
                  onClick=${()=>a(f.value)}
                  className=${H("h-9 px-3 text-xs font-semibold",t===f.value?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
                >
                  ${o(f.labelKey)}
                </button>
              `)}
            </div>
            <${T}
              variant="secondary"
              size="icon-sm"
              aria-label=${o("automations.refresh")}
              title=${o(r?"automations.refreshing":"automations.refresh")}
              disabled=${r}
              onClick=${n}
            >
              <${A}
                name="retry"
                className=${H("h-4 w-4",r&&"v2-spin")}
              />
            <//>
          </div>
        </div>
      <//>

      ${u.length?l`
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1.12fr)_minmax(22rem,0.88fr)]">
              <${U} className="overflow-hidden">
                <div className="overflow-x-auto">
                  <table className="w-full min-w-[900px] border-collapse">
                    <thead>
                      <tr className="border-b border-[var(--v2-panel-border)] text-left">
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${o("automations.table.name")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${o("automations.table.schedule")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${o("automations.table.nextRun")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${o("automations.table.recentRuns")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${o("automations.table.status")}
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      ${u.map(f=>{let m=f.automation_id===d?.automation_id;return l`
                          <tr
                            key=${f.automation_id}
                            className=${H("border-b border-[var(--v2-panel-border)] last:border-0 hover:bg-white/[0.03]",m&&"bg-[var(--v2-accent-soft)]/30")}
                          >
                            <td className="max-w-[280px] px-5 py-4 align-top">
                              <button
                                type="button"
                                aria-pressed=${m}
                                onClick=${()=>i(f.automation_id)}
                                className="block w-full min-w-0 rounded text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]"
                              >
                                <div className="truncate text-sm font-semibold text-iron-100">
                                  ${f.display_name}
                                </div>
                                <div className="mt-1 truncate font-mono text-[11px] uppercase tracking-[0.12em] text-iron-400">
                                  ${f.automation_id}
                                </div>
                              </button>
                            </td>
                            <td className="px-5 py-4 align-top text-sm text-iron-200">
                              ${f.schedule_label}
                            </td>
                            <td className="px-5 py-4 align-top text-sm text-iron-200">
                              ${f.next_run_label}
                            </td>
                            <td className="px-5 py-4 align-top">
                              <div className="space-y-2">
                                <${Gc} runs=${f.recent_runs} />
                                <${Yc} runs=${f.recent_runs} />
                              </div>
                            </td>
                            <td className="px-5 py-4 align-top">
                              <${j}
                                tone=${f.has_running_run?"info":f.has_failed_runs?"danger":f.state_tone}
                                label=${f.has_running_run?o("automations.status.running"):f.has_failed_runs?o("automations.status.needsReview"):f.state_label}
                              />
                            </td>
                          </tr>
                        `})}
                    </tbody>
                  </table>
                </div>
              <//>

              <${tN} automation=${d} />
            </div>
          `:c?l`
              <${he}
                title=${o("automations.empty.matchingTitle")}
                description=${o("automations.empty.matchingDescription")}
              />
            `:l`<${aN} />`}
    </div>
  `}function rN({summary:e,activeFilter:t,onSelectFilter:a}){let n=k(),r=[{key:"scheduled",label:n("automations.summary.scheduled"),value:e?.scheduled??0,tone:"muted",detail:n("automations.summary.scheduledDetail"),filter:"all"},{key:"active",label:n("automations.summary.active"),value:e?.active??0,tone:"signal",detail:n("automations.summary.activeDetail"),filter:"active"},{key:"running",label:n("automations.summary.running"),value:e?.running??0,tone:"info",detail:n("automations.summary.runningDetail"),filter:"running"},{key:"failures",label:n("automations.summary.failures"),value:e?.failures??0,tone:(e?.failures??0)>0?"danger":"success",detail:n("automations.summary.failuresDetail"),filter:(e?.failures??0)>0?"failures":null},{key:"nextRun",label:n("automations.summary.nextRun"),value:e?.nextRun||n("automations.summary.none"),tone:"info",detail:n("automations.summary.nextRunDetail"),valueClassName:"text-lg md:text-xl"}];return l`
    <${U} className="p-4 sm:p-5">
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
              className=${H(c,"transition-colors hover:border-white/20 hover:bg-white/[0.05]","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",o&&"border-[var(--v2-accent)]/60 bg-[var(--v2-accent-soft)]/30")}
            >
              ${u}
            </button>
          `:l`<div key=${s.key} className=${c}>${u}</div>`})}
      </div>
    <//>
  `}var M5=50,O5=25;function sN(e=!1){let{t,lang:a}=tl(),n=z({queryKey:["automations",{includeCompleted:e}],queryFn:()=>mx({limit:M5,runLimit:O5,includeCompleted:e}),refetchInterval:3e4,refetchIntervalInBackground:!1}),r=h.default.useMemo(()=>VS(n.data,t,a),[n.data,t,a]),s=h.default.useMemo(()=>YS(r),[r]),i=n.data?.scheduler_enabled!==!1;return{automations:r,summary:s,schedulerEnabled:i,isLoading:n.isLoading,isRefreshing:n.isFetching,error:n.error||null,refetch:n.refetch}}var iN=["outbound-delivery","preferences"],oN=["outbound-delivery","targets"];function lN(){let e=X(),t=z({queryKey:iN,queryFn:vx}),a=z({queryKey:oN,queryFn:gx}),n=Q({mutationFn:({finalReplyTargetId:i})=>yx({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(iN,i),e.invalidateQueries({queryKey:oN})}}),r=h.default.useMemo(()=>a.data?.targets??[],[a.data]),s=h.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function uN(){let e=k(),[t,a]=h.default.useState("all"),[n,r]=h.default.useState(null),i=sN(t==="completed"),o=lN(),[u,c]=h.default.useState(!1),d=h.default.useRef(null);h.default.useEffect(()=>()=>clearTimeout(d.current),[]);let f=h.default.useCallback(()=>{c(!0),clearTimeout(d.current),d.current=setTimeout(()=>c(!1),1e3),i.refetch()},[i.refetch]),m=i.isRefreshing||u,p=i.error&&!i.isLoading&&i.automations.length===0;return h.default.useEffect(()=>{if(!i.automations.length){r(null);return}i.automations.some(y=>y.automation_id===n)||r(i.automations[0].automation_id)},[i.automations,n]),l`
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
                <${rN}
                  summary=${i.summary}
                  activeFilter=${t}
                  onSelectFilter=${a}
                />
                <${BS} deliveryState=${o} />

                ${i.isLoading?l`
                      <div className="space-y-4">
                        ${[1,2,3].map(b=>l`<div
                              key=${b}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`)}
                      </div>
                    `:l`
                      <${nN}
                        automations=${i.automations}
                        filter=${t}
                        onFilterChange=${a}
                        onRefresh=${f}
                        isRefreshing=${m}
                        selectedAutomationId=${n}
                        onSelectAutomation=${r}
                      />
                    `}
              `}
        </div>
      </div>
    </div>
  `}var cN={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function dN({result:e,onDismiss:t}){return h.default.useEffect(()=>{if(!e)return;let a=setTimeout(t,4e3);return()=>clearTimeout(a)},[e,t]),e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",cN[e.type]||cN.info].join(" ")}>
      <${A}
        name=${e.type==="success"?"check":e.type==="error"?"close":"bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">
        <${A} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `:null}var mN="/api/webchat/v2/channels/slack/allowed",L5="/api/webchat/v2/channels/slack/subjects";function fN(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function pN(){return Y(mN)}function hN(){return Y(L5)}function vN(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return Y(mN,{method:"PUT",body:JSON.stringify(n)})}function gN(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var yN=["slack-allowed-channels"];function xN({action:e}){let t=k(),a=X(),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState([]),c=j5(e,t),d=z({queryKey:yN,queryFn:pN}),f=z({queryKey:["slack-routable-subjects"],queryFn:hN}),m=f.data?.subjects||[],p=bN(m),b=f.isSuccess||f.isError,y=m.length>0;h.default.useEffect(()=>{d.data&&u(vh(d.data.channels||[]))},[d.data]);let $=Q({mutationFn:({channels:R})=>vN(R),onSuccess:R=>{u(vh(R.channels||[])),a.invalidateQueries({queryKey:yN}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let R=n.trim();!R||!f.isSuccess||(u(_=>vh([..._,{channel_id:R,subject_user_id:s}])),r(""))},v=R=>{u(_=>_.filter(E=>E.channel_id!==R))},x=(R,_)=>{u(E=>E.map(M=>M.channel_id===R?{...M,subject_user_id:_}:M))},w=()=>{$.mutate({channels:P5(o)})},S=f.isError&&o.some(R=>!R.subject_user_id);return l`
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
        <${T}
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
                      onChange=${_=>x(R.channel_id,_.target.value)}
                      className="h-8 rounded-md border border-white/10 bg-white/[0.04] px-2 text-xs text-iron-100 outline-none focus:border-signal/45"
                    >
                      <option value="">${c.autoSubjectLabel}</option>
                      ${bN(m,R).map(_=>l`
                          <option key=${_.subject_user_id} value=${_.subject_user_id}>
                            ${_.display_name}
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
        <${T}
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
          ${gN($.error||d.error||f.error,c.errorMessage)}
        </p>`}
      </div>
    </div>
  `}function bN(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function vh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return fN(Array.from(t.keys())).map(a=>t.get(a))}function P5(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function j5(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var gh={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function zr(e){return e==="wasm_channel"||e==="channel"}var $N={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},wN={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function SN(e){let t=NN(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||zr(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function NN(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function yh(e){let t=NN(e);return t==="active"||t==="ready"}function _N({extension:e,secrets:t=[],fields:a=[]}={}){return yh(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var kN="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",RN="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",CN="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",EN="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",TN="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",U5="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function AN(e){return e.package_ref?.id||""}function F5({actions:e,isBusy:t}){let a=k(),[n,r]=h.default.useState(!1),s=h.default.useRef(null);return h.default.useEffect(()=>{if(!n)return;let i=o=>{s.current&&!s.current.contains(o.target)&&r(!1)};return document.addEventListener("mousedown",i),()=>document.removeEventListener("mousedown",i)},[n]),l`
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
        <${A} name="more" className="h-4 w-4" strokeWidth=${2.4} />
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
                <${A} name=${i.icon||"settings"} className="h-3.5 w-3.5" />
                ${i.label}
              </button>
            `)}
        </div>
      `}
    </div>
  `}function DN({items:e}){return!e||e.length===0?null:l`
    <div className="mt-3 flex flex-wrap gap-1">
      ${e.map(t=>l`<span key=${t} className=${U5}>${t}</span>`)}
    </div>
  `}function li({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=k(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=$N[i]||"muted",u=s(`extensions.state.${i}`)||wN[i]||i,c=s(`extensions.kind.${e.kind}`)||gh[e.kind]||e.kind,d=e.display_name||AN(e),f=!!e.package_ref,m=e.tools||[],[p,b]=h.default.useState(!1),$=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],x=[],w=SN(e);w==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):w==="activate"&&v.push({id:"activate",label:"Activate",run:()=>t(g)}),f&&(e.needs_setup||e.has_auth)&&w!=="configure"&&x.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)}),f&&zr(e.kind)&&(i==="setup_required"||i==="failed")&&x.push({id:"setup",label:"Setup",icon:"settings",run:()=>a(g)}),f&&zr(e.kind)&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&x.push({id:"reconfigure",label:"Reconfigure",icon:"settings",run:()=>a(g)}),f&&x.push({id:"remove",label:s("common.remove")||"Remove",icon:"trash",danger:!0,run:()=>n(g)});let S=v[0];return l`
    <div className=${kN}>
      <div className="flex items-start gap-2">
        <${j} tone=${o} label=${u} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${x.length>0&&l`<${F5} actions=${x} isBusy=${r} />`}
      </div>

      <div className=${RN}>
        <span>${c}</span>
        ${e.version&&l`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&l`<p className=${CN}>${e.description}</p>`}

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

      <div className=${EN}>
        ${m.length>0?l`
              <button
                type="button"
                aria-expanded=${p?"true":"false"}
                onClick=${()=>b(R=>!R)}
                className=${TN}
              >
                <${A} name="layers" className="h-3.5 w-3.5" />
                <span>${m.length===1?s("extensions.oneCapability"):s("extensions.pluralCapabilities",{count:m.length})}</span>
                <${A}
                  name="chevron"
                  className=${["h-3 w-3",p?"rotate-180":""].join(" ")}
                />
              </button>
            `:l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">No capabilities</span>`}
        <span className="flex-1"></span>
        ${S&&l`
          <${T} variant="secondary" size="sm" onClick=${S.run} disabled=${r}>
            ${S.label}
          <//>
        `}
      </div>

      ${p&&l`<${DN} items=${m} />`}
    </div>
  `}function qr({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=k(),s=r(`extensions.kind.${e.kind}`)||gh[e.kind]||e.kind,i=e.display_name||AN(e),o=!!(e.package_ref&&t),u=e.keywords||[],[c,d]=h.default.useState(!1);return l`
    <div className=${kN}>
      <div className="flex items-start gap-2">
        <${j}
          tone="muted"
          label=${n||r("extensions.state.available")||"available"}
          size="sm"
        />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${i}
        </span>
      </div>

      <div className=${RN}>
        <span>${s}</span>
        ${e.version&&l`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&l`<p className=${CN}>${e.description}</p>`}

      <div className=${EN}>
        ${u.length>0?l`
              <button
                type="button"
                aria-expanded=${c?"true":"false"}
                onClick=${()=>d(f=>!f)}
                className=${TN}
              >
                <${A} name="list" className="h-3.5 w-3.5" />
                <span>${u.length===1?r("extensions.oneKeyword"):r("extensions.pluralKeywords",{count:u.length})}</span>
                <${A}
                  name="chevron"
                  className=${["h-3 w-3",c?"rotate-180":""].join(" ")}
                />
              </button>
            `:l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]"></span>`}
        <span className="flex-1"></span>
        ${o&&l`
          <${T}
            variant="outline"
            size="sm"
            onClick=${()=>t({packageRef:e.package_ref,displayName:i})}
            disabled=${a}
          >
            <${A} name="plus" className="mr-1.5 h-3.5 w-3.5" />
            Install
          <//>
        `}
      </div>

      ${c&&l`<${DN} items=${u} />`}
    </div>
  `}function MN(){return Y("/api/webchat/v2/extensions")}function ON(){return Y("/api/webchat/v2/extensions/registry")}function LN(e){return Y("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function PN(e){return Y(`/api/webchat/v2/extensions/${encodeURIComponent(Wo(e))}/activate`,{method:"POST"})}function jN(e){return Y(`/api/webchat/v2/extensions/${encodeURIComponent(Wo(e))}/remove`,{method:"POST"})}function UN(e){return Y(`/api/webchat/v2/extensions/${encodeURIComponent(Wo(e))}/setup`)}function FN(e,t,a){return kx(Wo(e),{action:"submit",payload:{secrets:t,fields:a}})}function zN(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return Y(`/api/webchat/v2/extensions/${encodeURIComponent(Wo(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function qN(){return Promise.resolve({requests:[]})}function BN(){return Promise.resolve({success:!1,message:"Pairing requires a v2 pairing endpoint."})}function Wo(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var z5=2e3,q5=10*60*1e3;function ui(e){return e?.package_ref?.id||null}function bh(e){return e?.display_name||ui(e)||""}function HN(e,t,a){return ui(t)||`${e}:${bh(t)||"unknown"}:${a}`}function B5(e,t){return e.installed!==t.installed?e.installed?-1:1:bh(e.entry||e.extension).localeCompare(bh(t.entry||t.extension))}function KN(){let e=X(),t=z({queryKey:["gateway-status-extensions"],queryFn:Ks,staleTime:1e4}),a=z({queryKey:["extensions"],queryFn:MN}),n=z({queryKey:["extension-registry"],queryFn:ON}),r=z({queryKey:["connectable-channels"],queryFn:Rc}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["extensions"]}),e.invalidateQueries({queryKey:["extension-registry"]}),e.invalidateQueries({queryKey:["gateway-status-extensions"]}),e.invalidateQueries({queryKey:["connectable-channels"]})},[e]),[i,o]=h.default.useState(null),u=h.default.useCallback(()=>o(null),[]),c=Q({mutationFn:({packageRef:D})=>LN(D),onSuccess:(D,{displayName:K})=>{D.success?(o({type:"success",message:D.message||D.instructions||`${K||"Extension"} installed`}),D.auth_url&&window.open(D.auth_url,"_blank","noopener,noreferrer")):o({type:"error",message:D.message||"Install failed"}),s()},onError:D=>{o({type:"error",message:D.message}),s()}}),d=Q({mutationFn:({packageRef:D})=>PN(D),onSuccess:(D,{displayName:K})=>{D.success?(o({type:"success",message:D.message||D.instructions||`${K||"Extension"} activated`}),D.auth_url&&window.open(D.auth_url,"_blank","noopener,noreferrer")):D.auth_url?(window.open(D.auth_url,"_blank","noopener,noreferrer"),o({type:"info",message:"Opening authentication\u2026"})):D.awaiting_token?o({type:"info",message:"Configuration required"}):o({type:"error",message:D.message||"Activation failed"}),s()},onError:D=>{o({type:"error",message:D.message})}}),f=Q({mutationFn:({packageRef:D})=>jN(D),onSuccess:(D,{displayName:K})=>{D.success?o({type:"success",message:`${K||"Extension"} removed`}):o({type:"error",message:D.message||"Remove failed"}),s()},onError:D=>{o({type:"error",message:D.message})}}),m=t.data||{},p=a.data?.extensions||[],b=n.data?.entries||[],y=r.data?.channels||[],$=new Map(p.map(D=>[ui(D),D]).filter(([D])=>!!D)),g=new Set(b.map(D=>ui(D)).filter(Boolean)),v=[...b.map((D,K)=>{let J=ui(D),ve=J&&$.get(J)||null;return{id:HN("registry",D,K),installed:!!(ve||D.installed),entry:D,extension:ve}}),...p.filter(D=>{let K=ui(D);return!K||!g.has(K)}).map((D,K)=>({id:HN("installed",D,K),installed:!0,entry:null,extension:D}))].sort(B5),x=D=>zr(D.kind),w=p.filter(x),S=p.filter(D=>D.kind==="mcp_server"),R=p.filter(D=>!x(D)&&D.kind!=="mcp_server"),_=b.filter(D=>x(D)&&!D.installed),E=b.filter(D=>D.kind==="mcp_server"&&!D.installed),M=b.filter(D=>D.kind!=="mcp_server"&&!x(D)&&!D.installed),P=a.isLoading||n.isLoading,I=c.isPending||d.isPending||f.isPending;return{status:m,extensions:p,channels:w,mcpServers:S,tools:R,channelRegistry:_,mcpRegistry:E,toolRegistry:M,registry:b,catalogEntries:v,connectableChannels:y,isLoading:P,isBusy:I,actionResult:i,clearResult:u,install:c.mutate,activate:d.mutate,remove:f.mutate,invalidate:s}}function IN(e){let t=z({queryKey:["extension-setup",e?.id||e],queryFn:()=>UN(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function QN(e,t){let a=X(),n=e?.id||e;return Q({mutationFn:({secrets:r,fields:s})=>FN(e,r,s),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function VN(e){let t=X(),a=e?.id||e,n=h.default.useRef(null),r=h.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=h.default.useCallback(()=>{let u=t.getQueryData(["extension-setup",a]);if(u?.secrets?.length>0&&u.secrets.every(m=>m.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(m=>m.package_ref?.id===a),f=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return f==="active"||f==="ready"},[a,t]),o=h.default.useCallback(u=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||u&&u.closed||Date.now()-c>q5)&&(r(),s())},z5)},[r,s,i]);return h.default.useEffect(()=>r,[r]),Q({mutationFn:({secret:u,popup:c})=>zN(e,u).then(d=>({res:d,popup:c})),onSuccess:({res:u,popup:c})=>{let d=c;u.authorization_url&&c&&!c.closed?c.location.href=u.authorization_url:u.authorization_url?d=window.open(u.authorization_url,"_blank","noopener,noreferrer"):c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(u,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function GN(e,t={}){let a=z({queryKey:["pairing",e],queryFn:()=>qN(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=X(),r=Q({mutationFn:({code:s})=>BN(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function YN(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var H5={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function JN({channel:e,redeemFn:t,i18nKeys:a=H5,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=k(),o=typeof t=="function",u=GN(e,{enabled:!o}),c=X(),[d,f]=h.default.useState(""),m=K5(i,a,r),p=Q({mutationFn:({code:S})=>t(e,S),onSuccess:()=>{f("");for(let S of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:S})}}),b=h.default.useCallback(S=>u.approve({code:S}),[u.approve]),y=h.default.useCallback(()=>{let S=d.trim();S&&(o?p.mutate({code:S}):(u.approve({code:S}),f("")))},[o,d,u.approve,p]),$=o?[]:u.requests,g=o?!1:u.isLoading,v=o?p.isPending:u.isApproving,x=o?p.isSuccess?p.data:null:u.result,w=o?p.isError?p.error:null:u.error;return g?l`
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
        <${T}
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
        ${YN(w,m.error)}
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
                  <${T}
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
  `}function K5(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function Xc(e){return e.package_ref?.id||""}function XN(e){return Xc(e)==="slack"}function WN(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function e_(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function I5(e){let t=e||[],a=[t.find(WN),t.find(e_)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function ZN({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>WN(r)?l`<${xN} action=${r.action} />`:e_(r)?l`<${$c} action=${r.action} />`:null).filter(Boolean);return n.length>0?l`<div className="space-y-3">${n}</div>`:null}function t_({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:u}){let c=k(),d=t||[],f=e.enabled_channels||[],m=I5(a),p=d.some(XN),b=m.length>0&&!p;return l`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          ${c("channels.builtIn")}
        </h3>
        <${ci}
          name="Web Gateway"
          description=${c("channels.webGatewayDesc")||"Browser-based chat with SSE streaming"}
          enabled=${!0}
          detail=${"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)}
        />
        <${ci}
          name="HTTP Webhook"
          description=${c("channels.httpWebhookDesc")||"Inbound webhook endpoint for external integrations"}
          enabled=${f.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${ci}
          name="CLI"
          description=${c("channels.cliDesc")||"Terminal interface with TUI or simple REPL"}
          enabled=${f.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${ci}
          name="REPL"
          description=${c("channels.replDesc")||"Minimal read-eval-print loop for testing"}
          enabled=${f.includes("repl")}
          detail="ironclaw run --repl"
        />
        ${b&&l`
          <${ci}
            name=${c("channels.slack")||"Slack"}
            description=${c("channels.slackDesc")||"Tenant app channel for DMs and app mentions"}
            enabled=${!1}
            statusLabel="legacy"
            statusTone="muted"
            detail=${c("channels.slackDetail")||"Tenant Slack app install"}
          >
            <${ZN}
              slackConnectActions=${m}
            />
          </${ci}>
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
                <div key=${Xc(y)} className="flex flex-col gap-3">
                  <${li}
                    ext=${y}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${u}
                  />
                  ${XN(y)&&l`<${ZN}
                    slackConnectActions=${m}
                  />`}
                  ${(y.onboarding_state==="pairing_required"||y.onboarding_state==="pairing")&&l` <${JN} channel=${Xc(y)} /> `}
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
                <${qr}
                  key=${Xc(y)}
                  entry=${y}
                  onInstall=${o}
                  isBusy=${u}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function ci({name:e,description:t,enabled:a,detail:n,children:r,statusLabel:s=a?"on":"off",statusTone:i=a?"success":"muted"}){return l`
    <div
      className="border-t border-white/[0.06] py-4 first:border-0 first:pt-0"
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-iron-200">${e}</span>
            <${j}
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
  `}function a_({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=k(),s=e?.displayName||e?.packageRef?.id||"Extension",{secrets:i=[],fields:o=[],onboarding:u,isLoading:c,error:d}=IN(e?.packageRef),[f,m]=h.default.useState({}),[p,b]=h.default.useState({}),y=VN(e?.packageRef),$=QN(e?.packageRef,_=>{_.success!==!1&&(n&&n(_),a())}),g=h.default.useCallback(()=>{let _={};for(let[E,M]of Object.entries(f)){let P=(M||"").trim();P&&(_[E]=P)}$.mutate({secrets:_,fields:p})},[f,p,$]),v=h.default.useCallback(_=>{let E=window.open("about:blank","_blank","width=600,height=600");E&&(E.opener=null),y.mutate({secret:_,popup:E})},[y]),w=i.filter(_=>(_.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,S=yh(e),R=_N({extension:e,secrets:i,fields:o});return c?l`
      <${Zc} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(_=>l`<div
                key=${_}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?l`
      <${Zc} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")||"Failed to load setup:"} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?l`
      <${Zc} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")||"No configuration required for this extension."}
        </p>
      <//>
    `:l`
    <${Zc} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
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
          <${A} name="bolt" className="h-3.5 w-3.5" />
        </a>
      `}

      <div className="space-y-4">
        ${i.map(_=>l`
            <div key=${_.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                ${_.prompt||_.name}
                ${_.optional&&l`
                  <span className="font-mono text-[10px] text-iron-700"
                    >${r("common.optional")||"optional"}</span
                  >
                `}
                ${_.provided&&l`
                  <span className="font-mono text-[10px] text-mint"
                    >${r("common.configured")||"configured"}</span
                  >
                `}
              </label>
              ${(_.setup?.kind||"manual_token")==="oauth"?l`
                    <div className="flex items-center justify-between gap-3 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2">
                      <span className="text-xs text-iron-300">
                        ${_.provided?r("extensions.authConfigured")||"Authorization is configured.":r("extensions.authPopup")||"Authorize this provider in a browser popup."}
                      </span>
                      <${T}
                        variant=${_.provided?"secondary":"primary"}
                        onClick=${()=>v(_)}
                        disabled=${y.isPending}
                      >
                        ${y.isPending?r("extensions.opening"):_.provided?r("extensions.reconnect"):r("extensions.authorize")}
                      <//>
                    </div>
                  `:l`
              <input
                type="password"
                placeholder=${_.provided?"\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (leave blank to keep)":""}
                value=${f[_.name]||""}
                onChange=${E=>m(M=>({...M,[_.name]:E.target.value}))}
                onKeyDown=${E=>E.key==="Enter"&&g()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
              ${_.auto_generate&&!_.provided&&l`
                <p className="mt-1 text-xs text-iron-700">
                  ${r("extensions.autoGenerated")||"Auto-generated if left blank"}
                </p>
              `}
                  `}
            </div>
          `)}
        ${o.map(_=>l`
            <div key=${_.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                ${_.prompt||_.name}
                ${_.optional&&l`
                  <span className="font-mono text-[10px] text-iron-700"
                    >${r("common.optional")||"optional"}</span
                  >
                `}
              </label>
              <input
                type="text"
                placeholder=${_.placeholder||""}
                value=${p[_.name]||""}
                onChange=${E=>b(M=>({...M,[_.name]:E.target.value}))}
                onKeyDown=${E=>E.key==="Enter"&&g()}
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
        <${T} variant="ghost" onClick=${a}>${r("common.cancel")||"Cancel"}<//>
        ${R&&l`
        <${T}
          variant="primary"
          onClick=${()=>t?.(e)}
        >
          Activate
        <//>
        `}
        ${w&&l`
        <${T}
          variant=${R?"secondary":"primary"}
          onClick=${g}
          disabled=${$.isPending}
        >
          ${$.isPending?"Saving\u2026":r("common.save")||"Save"}
        <//>
        `}
      </div>
    <//>
  `}function Zc({onClose:e,title:t,children:a}){return h.default.useEffect(()=>{let n=r=>{r.key==="Escape"&&e()};return window.addEventListener("keydown",n),()=>window.removeEventListener("keydown",n)},[e]),l`
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
            <${A} name="close" className="h-4 w-4" />
          </button>
        </div>
        ${a}
      </div>
    </div>
  `}function n_(e){return e.package_ref?.id||""}function r_({mcpServers:e,mcpRegistry:t,onActivate:a,onConfigure:n,onRemove:r,onInstall:s,isBusy:i}){let o=k();return e.length===0&&t.length===0?l`
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
                <${li}
                  key=${n_(u)}
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
                <${qr}
                  key=${n_(u)}
                  entry=${u}
                  onInstall=${s}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function Q5(e){return e?.package_ref?.id||""}function V5(e){return e.entry||e.extension||{}}function s_({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=k(),[o,u]=h.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(y=>{let $=V5(y);return($.display_name||Q5($)).toLowerCase().includes(c)||($.description||"").toLowerCase().includes(c)||($.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,f=d.filter(y=>y.installed&&y.extension),m=d.filter(y=>y.installed&&!y.extension&&y.entry),p=f.length+m.length,b=d.filter(y=>!y.installed&&y.entry);return e.length===0?l`
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
                      <${li}
                        key=${y.id}
                        ext=${y.extension||y.entry}
                        onActivate=${a}
                        onConfigure=${n}
                        onRemove=${r}
                        isBusy=${s}
                      />
                    `)}
                  ${m.map(y=>l`
                      <${qr}
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
                      <${qr}
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
  `}function xh(){let{tab:e="registry"}=ot(),[t,a]=h.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:u,connectableChannels:c,isLoading:d,isBusy:f,actionResult:m,clearResult:p,install:b,activate:y,remove:$,invalidate:g}=KN(),v=h.default.useCallback(_=>a(_),[]),x=h.default.useCallback(()=>a(null),[]),w=h.default.useCallback(()=>g(),[g]),S=h.default.useCallback(_=>{_&&(y(_),a(null))},[y]);if(d)return l`
      <div className="flex h-full flex-col overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${[1,2,3].map(_=>l`
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
    `;if(e==="installed")return l`<${lt} to="/extensions/registry" replace />`;let R={channels:l`<${t_}
      status=${n}
      channels=${r}
      connectableChannels=${c}
      channelRegistry=${i}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${b}
      isBusy=${f}
    />`,mcp:l`<${r_}
      mcpServers=${s}
      mcpRegistry=${o}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${b}
      isBusy=${f}
    />`,registry:l`<${s_}
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
          <${dN} result=${m} onDismiss=${p} />
          ${R[e]}
        </div>
      </div>

      ${t&&l`
        <${a_}
          extension=${t}
          onActivate=${S}
          onClose=${x}
          onSaved=${w}
        />
      `}
    </div>
  `:l`<${lt} to="/extensions/registry" replace />`}var i_=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],o_=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.auto_approve_tools",labelKey:"settings.field.autoApproveTools",descKey:"settings.field.autoApproveToolsDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],l_=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],$h=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","agent.auto_approve_tools","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function u_(e){return String(e||"").trim().toLowerCase()}function c_(e){if(e==null)return"";if(Array.isArray(e))return e.map(c_).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function tt(e,t){let a=u_(e);return a?t.map(c_).join(" ").toLowerCase().includes(a):!0}function di(e,t,a,n){let r=u_(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(u=>tt(r,[i,u.key,u.labelKey?n(u.labelKey):u.label,u.descKey?n(u.descKey):u.description,t[u.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function G5({visible:e}){let t=k();return e?l`
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      ${t("tools.saved")}
    </span>
  `:null}function Y5({checked:e,onChange:t,label:a}){return l`
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
  `}function J5({field:e,value:t,onSave:a,isSaved:n}){let r=k(),[s,i]=h.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",u=e.descKey?r(e.descKey):e.description||"";h.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=h.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let f=parseInt(d,10);isNaN(f)||a(e.key,f)}else if(e.type==="float"){let f=parseFloat(d);isNaN(f)||a(e.key,f)}else a(e.key,d)},[e.key,e.type,a]);return l`
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">${o}</div>
        ${u&&l`<div className="mt-1 text-xs leading-5 text-iron-300">${u}</div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${e.type==="boolean"?l`
              <${Y5}
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
        <${G5} visible=${n} />
      </div>
    </div>
  `}function mi({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=k(),o=t?i(t):e||"";return l`
    <${te} className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${o}</h3>
      <div>
        ${a.map(u=>l`
              <${J5}
                key=${u.key}
                field=${u}
                value=${n[u.key]}
                onSave=${r}
                isSaved=${s[u.key]}
              />
            `)}
      </div>
    <//>
  `}function Nt({query:e}){let t=k();return l`
    <${te} padding="lg">
      <div className="flex items-center gap-3">
        <span
          className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-faint)]"
        >
          <${A} name="search" className="h-4 w-4" />
        </span>
        <div className="min-w-0">
          <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
            ${t("settings.noMatchingSettings",{query:e})}
          </h3>
        </div>
      </div>
    <//>
  `}function d_({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=k();if(n)return l`<${X5} />`;let i=di(o_,e,r,s);return i.length===0?l`<${Nt} query=${r} />`:l`
    <div className="space-y-5">
      ${i.map(o=>l`
            <${mi}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function X5(){return l`
    <div className="space-y-5">
      ${[1,2,3].map(e=>l`
            <${te} key=${e} padding="md">
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
  `}function m_(){let e=z({queryKey:["gateway-status-settings"],queryFn:Ks,staleTime:1e4}),t=z({queryKey:["extensions"],queryFn:g$}),a=z({queryKey:["extension-registry"],queryFn:y$}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(f=>f.kind==="wasm_channel"||f.kind==="channel"),o=s.filter(f=>(f.kind==="wasm_channel"||f.kind==="channel")&&!f.installed),u=r.filter(f=>f.kind==="mcp_server"),c=s.filter(f=>f.kind==="mcp_server"&&!f.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:u,mcpRegistry:c,extensions:r,isLoading:d}}function Z5({name:e,description:t,enabled:a,detail:n}){let r=k();return l`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${e}</span>
          <${j}
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
  `}function f_({channel:e,registryEntry:t}){let a=k(),n=t?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},u={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return l`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${n}</span>
          ${s?l`<${j}
                tone=${o[i]||"muted"}
                label=${u[i]||i}
                size="sm"
              />`:l`<${j}
                tone="muted"
                label=${a("channels.available")}
                size="sm"
              />`}
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${r}</div>
      </div>
    </div>
  `}function W5(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function eD({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=W5(e,i).filter(b=>tt(s,[i("channels.builtIn"),b.id,b.name,b.description,b.detail])),u=new Set(t.map(b=>b.name)),c=t.filter(b=>tt(s,[i("channels.messaging"),b.name,b.display_name,b.description,b.onboarding_state])),d=a.filter(b=>!u.has(b.name)).filter(b=>tt(s,[i("channels.messaging"),b.name,b.display_name,b.description])),f=new Set(n.map(b=>b.name)),m=n.filter(b=>tt(s,[i("channels.mcpServers"),b.name,b.display_name,b.description,b.active?i("channels.active"):i("channels.inactive")])),p=r.filter(b=>!f.has(b.name)).filter(b=>tt(s,[i("channels.mcpServers"),b.name,b.display_name,b.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:p}}function p_({searchQuery:e=""}){let t=k(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=m_();if(o)return l`
      <div className="space-y-5">
        <${te} padding="md">
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
    `;let{builtInChannels:u,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:m}=eD({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return u.length===0&&c.length===0&&d.length===0&&f.length===0&&m.length===0?l`<${Nt} query=${e} />`:l`
    <div className="space-y-5">
      ${u.length>0&&l`
      <${te} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${u.map(p=>l`
            <${Z5}
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
        <${te} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.messaging")}
          </h3>
          ${c.map(p=>l`
              <${f_}
                key=${p.name}
                channel=${p}
                registryEntry=${r.find(b=>b.name===p.name)}
              />
            `)}
          ${d.map(p=>l`
              <${f_} key=${p.name} registryEntry=${p} />
            `)}
        <//>
      `}
      ${(f.length>0||m.length>0)&&l`
        <${te} padding="md">
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
                      <${j}
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
                      <${j}
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
  `}function h_({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:u,onNearaiWallet:c,onCodexLogin:d,loginBusy:f}){let m=k(),p=e.id===t,b=Pr(e,n),y=Vs(e,n),$=E$(e,n,t,a),g=uc(e,n),v=T$(e),x=m(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[w,S]=h.default.useState(p),R=h.default.useCallback(()=>S(_t=>!_t),[]);h.default.useEffect(()=>{S(p)},[p]);let _=b?l`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${zo(e.adapter)} · ${$||e.default_model||m("llm.none")}
      </span>`:l`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${x}
      </span>`,E=e.id==="nearai"||e.id==="openai_codex",M=e.api_key_set===!0||e.has_api_key===!0,P=e.builtin?e.id==="nearai"&&v&&!M?m("llm.addApiKey"):m("llm.configure"):m("common.edit"),I=v&&e.builtin?l`
          <${T}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${r}
            onClick=${()=>i(e)}
          >
            ${P}
          <//>
        `:null,D=!p&&e.id==="nearai"?l`
          ${I}
          <${T} type="button" variant="secondary" size="sm" disabled=${f} onClick=${c}>
            ${m("onboarding.nearWallet")}
          <//>
          <${T} type="button" variant="secondary" size="sm" disabled=${f} onClick=${()=>u("github")}>
            GitHub
          <//>
          <${T} type="button" variant="secondary" size="sm" disabled=${f} onClick=${()=>u("google")}>
            Google
          <//>
        `:!p&&e.id==="openai_codex"?l`
          <${T} type="button" variant="secondary" size="sm" disabled=${f} onClick=${d}>
            ${m("onboarding.codexSignIn")}
          <//>
        `:null,J=!p&&b&&(!E||e.id==="nearai"&&e.has_api_key===!0)?l`
        <${T}
          type="button"
          variant="primary"
          size="sm"
          disabled=${r}
          onClick=${()=>s(e)}
        >
          ${m("llm.use")}
        <//>
      `:null,ve=b?null:l`
        <${T}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${r}
          onClick=${()=>i(e)}
        >
          ${m(g==="api_key"?"llm.addApiKey":"llm.configure")}
        <//>
      `,_e=p?null:J||(E?D:ve),Ye=!E&&(e.builtin&&e.id!=="bedrock"||!e.builtin)||e.id==="nearai"&&v;return l`
    <${te}
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
            ${p&&l`<${j} tone="positive" label=${m("llm.active")} size="sm" />`}
            ${e.builtin&&!p&&l`<${j} tone="muted" label=${m("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${_}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 py-3 pr-4 sm:pr-5">
          ${_e}
          <button
            type="button"
            onClick=${R}
            data-testid="llm-provider-chevron"
            aria-label=${m(w?"llm.collapseDetails":"llm.expandDetails")}
            className=${["grid h-7 w-7 place-items-center rounded-md text-[var(--v2-text-faint)] transition-transform hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]",w?"rotate-180":""].join(" ")}
          >
            <${A} name="chevron" className="h-4 w-4" />
          </button>
        </div>
      </div>

      ${w&&l`
        <div data-testid="llm-provider-details" className="border-t border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-4 sm:px-5">
          <div className="grid gap-3 text-xs text-[var(--v2-text-muted)] sm:grid-cols-3">
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${m("llm.adapter")}</div>
              <div className="mt-1 truncate">${zo(e.adapter)}</div>
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
            ${Ye&&l`
              <${T}
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
              <${T}
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
  `}var tD=[{key:"active",labelKey:"llm.groupActive",dotClass:"bg-[var(--v2-positive-text)]"},{key:"ready",labelKey:"llm.groupReady",dotClass:"bg-[var(--v2-accent)]"},{key:"setup",labelKey:"llm.groupSetup",dotClass:"bg-[var(--v2-warning-text)]"}];function aD({label:e,count:t,dotClass:a}){return l`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full "+a} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${t}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `}function v_({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=k(),r=Dc({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=Mc(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return l`<${Nt} query=${a} />`;let u=A$(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return l`
    <${te} className="p-4 sm:p-6">
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${n("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">${n("llm.providersDesc")}</p>
        </div>
        <${T} type="button" variant="secondary" size="sm" className="gap-2" onClick=${()=>r.openDialog(null)}>
          <${A} name="plus" className="h-3.5 w-3.5" />
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

      <${Ac} login=${i} />

      ${s.isLoading?l`<div className="text-sm text-[var(--v2-text-muted)]">${n("common.loading")}</div>`:s.error?l`<div className="text-sm text-red-200">${n("error.loadFailed",{what:n("llm.providers"),message:s.error.message})}</div>`:l`
            <div className="space-y-1">
              ${tD.flatMap(c=>{let d=u[c.key];return d.length?[l`
                    <section
                      key=${c.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${c.key}
                      className="mb-3"
                    >
                      <${aD}
                        label=${n(c.labelKey)}
                        count=${d.length}
                        dotClass=${c.dotClass}
                      />
                      <div className="space-y-2">
                      ${d.map(f=>l`
                          <${h_}
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

      <${Tc}
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
  `}function g_({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=k(),{activeProviderId:o,selectedModel:u,providers:c,hasActiveProvider:d}=Gs({settings:e,gatewayStatus:t});if(r)return l`<${nD} />`;let f=d?o:"",m=c.find(g=>g.id===o),p=d&&(u||m?.default_model||e.selected_model)||"",b=di(i_,e,s,i),y=tt(s,[i("inference.provider"),i("inference.backend"),f,i("inference.model"),p]),$=tt(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!y&&!$&&b.length===0?l`<${Nt} query=${s} />`:l`
    <div className="space-y-5">
      ${y&&l`
      <${te} padding="none" className="p-4 sm:p-5">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${i("inference.provider")}</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${i("inference.backend")}</div>
            <div className="mt-1 flex items-center gap-2">
              <span className="font-mono text-lg font-semibold text-[var(--v2-text-strong)]">${f||i("inference.none")}</span>
              ${d?l`<${j} tone="positive" label=${i("inference.active")} size="sm" />`:l`<${j} tone="muted" label=${i("llm.notConfigured")} size="sm" />`}
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
        <${v_}
          settings=${e}
          gatewayStatus=${t}
          searchQuery=${s}
        />
      `}

      ${b.map(g=>l`
            <${mi}
              key=${g.groupKey}
              groupKey=${g.groupKey}
              fields=${g.fields}
              settings=${e}
              onSave=${a}
              savedKeys=${n}
            />
          `)}
    </div>
  `}function rr({className:e=""}){return l`
    <div
      className=${"rounded animate-pulse bg-[var(--v2-surface-muted)] "+e}
    />
  `}function nD(){return l`
    <div className="space-y-5">
      <${te} padding="md">
        <${rr} className="mb-4 h-3 w-24" />
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${rr} className="h-3 w-16" />
            <${rr} className="mt-2 h-6 w-28" />
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${rr} className="h-3 w-16" />
            <${rr} className="mt-2 h-6 w-40" />
          </div>
        </div>
      <//>
      ${[1,2].map(e=>l`
            <${te} key=${e} padding="md">
              <${rr} className="mb-4 h-3 w-20" />
              ${[1,2,3].map(t=>l`
                    <div key=${t} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                      <${rr} className="h-4 w-32" />
                      <${rr} className="h-9 w-36" />
                    </div>
                  `)}
            <//>
          `)}
    </div>
  `}function y_({searchQuery:e=""}){let t=k(),{lang:a,setLang:n}=tl(),r=al.find(i=>i.code===a)||al[0],s=al.filter(i=>tt(e,[i.code,i.name,i.native]));return s.length===0?l`<${Nt} query=${e} />`:l`
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
  `}function b_({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=k();if(n)return l`
      <div className="space-y-5">
        ${[1,2].map(o=>l`
              <${te} key=${o} padding="md">
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
    `;let i=di(l_,e,r,s);return i.length===0?l`<${Nt} query=${r} />`:l`
    <div className="space-y-5">
      ${i.map(o=>l`
            <${mi}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function x_(){let e=k(),[t,a]=h.default.useState(!1),n=h.default.useCallback(()=>a(!0),[]),r=h.default.useCallback(()=>a(!1),[]),s=h.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function $_({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=k(),r=x_({gatewayStatus:t,gatewayStatusQuery:a});return e?l`
    <div className="space-y-3">
      <div
        role="alert"
        className="flex flex-col gap-3 rounded-xl border border-copper/30 bg-copper/10 px-4 py-3 sm:flex-row sm:items-center"
      >
        <div className="flex min-w-0 flex-1 items-start gap-3">
          <${A} name="bolt" className="mt-0.5 h-4 w-4 shrink-0 text-copper" />
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

        <${T}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${!r.restartEnabled||r.isRestarting}
          onClick=${r.openConfirm}
          title=${r.restartEnabled?void 0:r.unavailableReason}
          className="w-full sm:w-auto"
        >
          <${A} name=${r.isRestarting?"pulse":"bolt"} className="h-4 w-4" />
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

    <${ei}
      open=${r.confirmOpen}
      onClose=${r.closeConfirm}
      title=${n("restart.title")}
      size="sm"
    >
      <${ti} className="space-y-3">
        <p className="text-sm text-[var(--v2-text)]">
          ${n("restart.description")}
        </p>
        <div className="rounded-xl border border-copper/25 bg-copper/10 px-3 py-2 text-xs text-copper">
          ${n("restart.warning")}
        </div>
      <//>
      <${ai}>
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
          <${A} name="bolt" className="h-4 w-4" />
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
            <${A} name="pulse" className="h-5 w-5 animate-pulse" />
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
  `:null}function w_(){let e=X(),t=z({queryKey:["skills"],queryFn:b$}),a=Q({mutationFn:$$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),n=Q({mutationFn:S$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),r=Q({mutationFn:({name:i,content:o})=>w$(i,{content:o}),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}});return{skills:t.data?.skills||[],query:t,fetchSkillContent:x$,installSkill:a.mutateAsync,removeSkill:n.mutateAsync,updateSkill:r.mutateAsync,isInstalling:a.isPending,isRemoving:n.isPending,isUpdating:r.isPending}}function S_({skill:e,onEdit:t,onRemove:a,onUpdate:n,isRemoving:r,isUpdating:s}){let i=k(),o=e.name||e.id,u=e.trust||e.trust_level||"installed",c=e.source_kind||"installed",d=!!e.can_edit,f=!!e.can_delete,[m,p]=h.default.useState(!1),[b,y]=h.default.useState(""),[$,g]=h.default.useState(""),[v,x]=h.default.useState(!1);h.default.useEffect(()=>{m||(y(""),g(""))},[m]);let w=h.default.useCallback(async()=>{x(!0),g("");try{let R=await t(o);y(R?.content||""),p(!0)}catch(R){g(R.message||i("skills.contentLoadFailed"))}finally{x(!1)}},[o,t,i]),S=h.default.useCallback(async()=>{(await n(o,b))?.success&&p(!1)},[b,o,n]);return l`
    <div className="ext-card border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text)]">${o}</span>
            <${j}
              tone=${String(u).toLowerCase()==="trusted"?"positive":"muted"}
              label=${u}
              size="sm"
            />
            <${j}
              tone=${c==="system"?"positive":"muted"}
              label=${i(`skills.source.${c}`)}
              size="sm"
            />
            ${e.version&&l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v${e.version}</span>`}
          </div>

          ${e.description&&l`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${e.description}</div>`}

          ${m?l`
                <div className="mt-3">
                  <${bc}
                    rows=${12}
                    value=${b}
                    className="font-mono text-xs leading-5"
                    onInput=${R=>y(R.currentTarget.value)}
                  />
                </div>
              `:l`<${rD} skill=${e} />`}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          ${d&&!m&&l`
            <${T}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${s||v}
              title=${i("skills.edit")}
              onClick=${w}
            >
              <${A} name="file" className="h-4 w-4" />
              ${i(v?"skills.loading":"skills.edit")}
            <//>
          `}
          ${m&&l`
            <${T}
              type="button"
              variant="ghost"
              size="sm"
              disabled=${s}
              onClick=${()=>{y(""),p(!1)}}
            >
              <${A} name="close" className="h-4 w-4" />
              ${i("skills.cancel")}
            <//>
            <${T}
              type="button"
              variant="primary"
              size="sm"
              disabled=${s}
              onClick=${S}
            >
              <${A} name="check" className="h-4 w-4" />
              ${i(s?"skills.saving":"skills.save")}
            <//>
          `}
          ${f&&!m&&l`
            <${T}
              type="button"
              variant="danger"
              size="sm"
              disabled=${r}
              title=${i("skills.delete")}
              onClick=${()=>a(o)}
            >
              <${A} name="trash" className="h-4 w-4" />
              ${i("skills.delete")}
            <//>
          `}
        </div>
      </div>
      ${$&&l`<p className="mt-2 text-xs text-[var(--v2-danger-text)]">${$}</p>`}
    </div>
  `}function rD({skill:e}){let t=k();return l`
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
        ${e.has_requirements&&l`<${wh}>requirements.txt<//>`}
        ${e.has_scripts&&l`<${wh}>scripts/<//>`}
        ${e.install_source_url&&l`<${wh}>${t("skills.imported")}<//>`}
      </div>
    `}
  `}function wh({children:e}){return l`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${e}
    </span>
  `}function N_({onInstall:e,isInstalling:t}){let a=k(),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState({name:"",content:""}),[c,d]=h.default.useState(""),[f,m]=h.default.useState(""),p=h.default.useCallback((y,$)=>{u(g=>!g[y]||!$.trim()?g:{...g,[y]:""})},[]),b=h.default.useCallback(async()=>{let y=sD({name:n,content:s}),$=iD(y,a);if($.name||$.content){u($),d(""),m("");return}u({name:"",content:""}),d(""),m("");try{let g=await e(y);if(!g?.success){d(g?.message||a("skills.installFailed"));return}r(""),i(""),m(g.message||a("skills.installedSuccess",{name:y.name}))}catch(g){d(g.message||a("skills.installFailed"))}},[s,n,e,a]);return l`
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

      <${xn} label=${a("skills.name")} error=${o.name} required>
        <${Dt}
          size="sm"
          error=${!!o.name}
          aria-invalid=${o.name?"true":void 0}
          value=${n}
          placeholder=${a("skills.namePlaceholder")}
          onInput=${y=>{let $=y.currentTarget.value;r($),p("name",$)}}
        />
      <//>

      <${xn}
        className="mt-3"
        label=${a("skills.content")}
        error=${o.content}
        hint=${a("skills.contentHint")}
        required
      >
        <${bc}
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
        <${T} type="button" size="sm" disabled=${t} onClick=${b}>
          <${A} name="upload" className="h-4 w-4" />
          ${a(t?"skills.installing":"skills.install")}
        <//>
      </div>
    <//>
  `}function sD({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function iD(e,t){return{name:e.name?"":t("skills.nameRequired"),content:e.content?"":t("skills.contentRequired")}}function __({searchQuery:e=""}){let t=k(),{skills:a,query:n,fetchSkillContent:r,installSkill:s,removeSkill:i,updateSkill:o,isInstalling:u,isRemoving:c,isUpdating:d}=w_(),[f,m]=h.default.useState(""),[p,b]=h.default.useState(""),y=h.default.useCallback(async v=>{if(window.confirm(t("skills.confirmDelete",{name:v}))){m(""),b("");try{let x=await i(v);if(!x?.success){m(x?.message||t("skills.removeFailed"));return}b(x.message||t("skills.removed",{name:v}))}catch(x){m(x.message||t("skills.removeFailed"))}}},[i,t]),$=h.default.useCallback(async(v,x)=>{if(!x.trim())return m(t("skills.contentRequired")),b(""),{success:!1,message:t("skills.contentRequired")};m(""),b("");try{let w=await o({name:v,content:x});return w?.success?(b(w.message||t("skills.updated",{name:v})),w):(m(w?.message||t("skills.updateFailed")),w)}catch(w){let S=w.message||t("skills.updateFailed");return m(S),{success:!1,message:S}}},[t,o]),g;if(n.isLoading)g=l`
      <${te} padding="md">
          <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(v=>l`
            <div key=${v} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
              <div>
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              </div>
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
        <//>
    `;else if(n.error)g=l`
      <${te} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">${t("skills.failedLoad",{message:n.error.message})}</p>
        <//>
    `;else{let v=a.filter(w=>tt(e,[w.name,w.id,w.description,w.keywords,w.trust_level,w.source_kind,w.version])),x=lD(v);a.length===0?g=l`
        <${te} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:v.length===0?g=l`<${Nt} query=${e} />`:g=l`
        <div id="skills-list">
          ${x.map(w=>l`
              <${oD}
                key=${w.id}
                title=${t(w.labelKey)}
                skills=${w.skills}
                onEdit=${r}
                onRemove=${y}
                onUpdate=${$}
                isRemoving=${c}
                isUpdating=${d}
              />
            `)}
        </div>
      `}return l`
    <div className="space-y-4">
      <${N_} onInstall=${s} isInstalling=${u} />
      <${uD} error=${f} result=${p} />
      ${g}
    </div>
  `}function oD({title:e,skills:t,onEdit:a,onRemove:n,onUpdate:r,isRemoving:s,isUpdating:i}){return t.length===0?null:l`
    <${te} padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${e}
      </h3>
      ${t.map(o=>l`
          <${S_}
            key=${`${o.source_kind||"skill"}:${o.name||o.id}`}
            skill=${o}
            onEdit=${a}
            onRemove=${n}
            onUpdate=${r}
            isRemoving=${s}
            isUpdating=${i}
          />
        `)}
    <//>
  `}function lD(e){let t=[{id:"user",labelKey:"skills.group.user",skills:[]},{id:"system",labelKey:"skills.group.system",skills:[]},{id:"workspace",labelKey:"skills.group.workspace",skills:[]}],a=t[0];for(let n of e){let r=n.source_kind||"";(r==="system"?t[1]:r==="workspace"?t[2]:a).skills.push(n)}return t.filter(n=>n.skills.length>0)}function uD({error:e,result:t}){return!e&&!t?null:l`
    <div
      className=${e?"rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200":"rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200"}
    >
      ${e||t}
    </div>
  `}function Wc(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function k_(){let e=X(),t=z({queryKey:["settings-tools"],queryFn:h$}),a=t.data?.tools||[],[n,r]=h.default.useState({}),s=Q({mutationFn:async({name:o,state:u})=>Wc(await v$(o,u),"Save failed"),onSuccess:(o,{name:u,state:c})=>{e.setQueryData(["settings-tools"],d=>d&&{...d,tools:d.tools.map(f=>f.name===u?{...f,state:c}:f)}),r(d=>({...d,[u]:!0})),setTimeout(()=>r(d=>({...d,[u]:!1})),2e3)}}),i=h.default.useCallback((o,u)=>s.mutate({name:o,state:u}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}function cD({tool:e,onPermissionChange:t,isSaved:a}){let n=k(),r=[{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],s=e.locked,i=r.find(u=>u.value===e.state)||r[1],o=e.state===e.default_state;return l`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="flex min-w-0 items-center gap-3">
        ${s&&l`<${A}
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
        ${s?l`<${j} tone=${i.tone} label=${i.label} size="sm" />`:l`
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
  `}function R_({searchQuery:e=""}){let t=k(),{tools:a,query:n,setPermission:r,savedTools:s}=k_();if(n.isLoading)return l`
      <${te} padding="md">
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
      <${te} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("tools.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let i=a.filter(o=>tt(e,[o.name,o.description,o.state,o.default_state,o.locked?t("tools.disabled"):""]));return l`
    <div className="space-y-4">
      ${e&&l`
        <div className="flex justify-end">
          <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">
            ${i.length} / ${a.length}
          </span>
        </div>
      `}

      <${te} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("tools.permissions")}
        </h3>
        ${i.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("tools.noMatch")}
            </p>`:i.map(o=>l`
                  <${cD}
                    key=${o.name}
                    tool=${o}
                    onPermissionChange=${r}
                    isSaved=${s[o.name]}
                  />
                `)}
      <//>
    </div>
  `}function C_(e){return(Number(e)||0).toFixed(2)}function dD(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function E_(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function Br({label:e,value:t,description:a}){return l`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&l`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function T_({searchQuery:e=""}){let t=k(),{credits:a,query:n,authorize:r}=mc();if(!tt(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return l`<${Nt} query=${e} />`;let s;if(n.isLoading)s=l`
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
        <${Br}
          label=${t("traceCommons.enrollment")}
          value=${a.enrolled?t("traceCommons.enrolled"):t("traceCommons.notEnrolled")}
        />
        <${Br}
          label=${t("traceCommons.pendingCredit")}
          description=${t("traceCommons.pendingCreditDesc")}
          value=${C_(a.pending_credit)}
        />
        <${Br}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${C_(a.final_credit)}
        />
        <${Br}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${dD(a.delayed_credit_delta)}
        />
        <${Br}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${Br}
          label=${t("traceCommons.lastSubmission")}
          value=${E_(a.last_submission_at,t)}
        />
        <${Br}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${E_(a.last_credit_sync_at,t)}
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
  `}function A_(){let e=X(),t=z({queryKey:["admin-users"],queryFn:k$,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=Q({mutationFn:R$,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=Q({mutationFn:({id:i,payload:o})=>C$(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function mD({onCreate:e,isCreating:t,error:a}){let n=k(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState("member"),[d,f]=h.default.useState(!1),m=p=>{p.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:u},{onSuccess:()=>{s(""),o(""),f(!1)}})};return d?l`
    <${te} padding="md">
      <h3
        className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${n("users.newUser")}
      </h3>
      <form onSubmit=${m} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <${xn} label=${n("users.displayName")} htmlFor="user-name">
            <${Dt}
              id="user-name"
              type="text"
              value=${r}
              onChange=${p=>s(p.target.value)}
              required
            />
          <//>
          <${xn} label=${n("users.email")} htmlFor="user-email">
            <${Dt}
              id="user-email"
              type="email"
              value=${i}
              onChange=${p=>o(p.target.value)}
            />
          <//>
        </div>
        <${xn} label=${n("users.role")} htmlFor="user-role">
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
          <${T} type="submit" disabled=${t}>
            ${n(t?"users.creating":"users.createUser")}
          <//>
          <${T}
            variant="ghost"
            type="button"
            onClick=${()=>f(!1)}
            >${n("users.cancel")}<//
          >
        </div>
      </form>
    <//>
  `:l`
      <${T} variant="secondary" onClick=${()=>f(!0)}>
        <${A} name="plus" className="mr-2 h-4 w-4" />
        ${n("users.addUser")}
      <//>
    `}function fD({user:e}){let t=k(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return l`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]"
            >${e.display_name||e.id}</span
          >
          <${j}
            tone=${n}
            label=${e.role==="admin"?t("users.admin"):t("users.member")}
            size="sm"
          />
          <${j} tone=${a} label=${e.status||"active"} size="sm" />
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
  `}function D_({searchQuery:e=""}){let t=k(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=A_();if(n.isLoading)return l`
      <${te} padding="md">
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
      <${te} padding="lg">
        <div className="flex items-center gap-3">
          <${A} name="lock" className="h-5 w-5 text-[var(--v2-text-faint)]" />
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">
            ${t("users.adminRequired")}
          </h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
          ${t("users.adminRequiredDesc")}
        </p>
      <//>
    `;if(n.error)return l`
      <${te} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("users.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let u=a.filter(c=>tt(e,[c.id,c.display_name,c.email,c.role,c.status,c.last_active]));return l`
    <div className="space-y-5">
      <${mD}
        onCreate=${s}
        isCreating=${o}
        error=${i}
      />

      <${te} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("users.title",{count:u.length})}
        </h3>
        ${a.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("users.noUsers")}
            </p>`:u.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("settings.noMatchingSettings",{query:e})}
            </p>`:u.map(c=>l`<${fD} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function M_(){let e=X(),t=z({queryKey:["settings-export"],queryFn:s$,staleTime:3e4}),a=t.data?.settings||{},[n,r]=h.default.useState({}),[s,i]=h.default.useState(!1),o=Q({mutationFn:async({key:f,value:m})=>Wc(await i$(f,m),"Save failed"),onSuccess:(f,{key:m,value:p})=>{e.setQueryData(["settings-export"],b=>{if(!b)return b;let y={...b,settings:{...b.settings}};return p==null?delete y.settings[m]:y.settings[m]=p,y}),r(b=>({...b,[m]:!0})),setTimeout(()=>r(b=>({...b,[m]:!1})),2e3),$h.has(m)&&i(!0)}}),u=h.default.useCallback((f,m)=>o.mutate({key:f,value:m}),[o]),c=Q({mutationFn:o$,onSuccess:(f,m)=>{e.invalidateQueries({queryKey:["settings-export"]}),Object.keys(m?.settings||{}).some(b=>$h.has(b))&&i(!0)}}),d=h.default.useCallback(f=>c.mutateAsync(f),[c]);return{settings:a,query:t,save:u,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error||c.error}}function Sh(){let e=k(),{tab:t}=ot(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=Ba(),s=r?"inference":"language",i=t||s,{settings:o,query:u,save:c,savedKeys:d,needsRestart:f,saveError:m}=M_(),[p,b]=h.default.useState("");h.default.useEffect(()=>{b("")},[i]);let y=u.isLoading,$={inference:l`<${g_}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,agent:l`<${d_}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,channels:l`<${p_} searchQuery=${p} />`,networking:l`<${b_}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,tools:l`<${R_} searchQuery=${p} />`,skills:l`<${__} searchQuery=${p} />`,traces:l`<${T_} searchQuery=${p} />`,users:l`<${D_} searchQuery=${p} />`,language:l`<${y_} searchQuery=${p} />`},g=R=>R==="users"||R==="inference",v=R=>Object.prototype.hasOwnProperty.call($,R),x=Object.keys($).filter(R=>r||!g(R)),S=v(s)&&x.includes(s)?s:x[0]||"language";return!v(i)||!r&&g(i)?l`<${lt} to=${`/settings/${S}`} replace />`:l`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${f&&l`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${$_}
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
  `}var Nh=Object.freeze({todo:!0});function O_(){return Promise.resolve({users:[],total:0,...Nh})}function L_(e){return Promise.resolve(null)}function P_(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function j_(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function U_(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function F_(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function z_(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function q_(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function B_(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...Nh})}function H_(e="day",t){return Promise.resolve({entries:[],...Nh})}function K_(){return z({queryKey:["admin","usage-summary"],queryFn:B_,refetchInterval:3e4})}function ed(e="day",t){return z({queryKey:["admin","usage",e,t],queryFn:()=>H_(e,t),refetchInterval:3e4})}function fi(){let e=X(),t=z({queryKey:["admin","users"],queryFn:O_,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=Q({mutationFn:P_,onSuccess:s}),o=Q({mutationFn:({id:m,payload:p})=>j_(m,p),onSuccess:s}),u=Q({mutationFn:m=>U_(m),onSuccess:s}),c=Q({mutationFn:m=>F_(m),onSuccess:s}),d=Q({mutationFn:m=>z_(m),onSuccess:s}),f=Q({mutationFn:({userId:m,name:p})=>q_(m,p)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(m,p)=>o.mutateAsync({id:m,payload:p}),deleteUser:u.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(m,p)=>f.mutateAsync({userId:m,name:p}),newToken:f.data,clearToken:()=>f.reset()}}function I_(e){return z({queryKey:["admin","user",e],queryFn:()=>L_(e),enabled:!!e,refetchInterval:1e4})}function Va(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function Ra(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function Q_(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function sr(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function pi(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function hi(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function vi(e){return e==="admin"?"signal":"muted"}function V_(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function G_(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function Y_(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function J_(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function X_(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function pD({users:e,onSelectUser:t}){let a=k(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?l`
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
                <td className="py-3 pr-4"><${j} tone=${vi(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${j} tone=${hi(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${sr(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:l`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function Z_({onSelectUser:e,onNavigateTab:t}){let a=k(),n=K_(),{users:r,query:s}=fi(),i=n.data||{},o=V_(r),u=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?l`
      <div className="space-y-5">
        <${U} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            ${[1,2,3,4].map(f=>l`<div key=${f} className="v2-skeleton h-28 rounded-lg" />`)}
          </div>
        <//>
      </div>
    `:l`
    <div className="space-y-5">
      <${U} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.systemOverview")}</h3>
          ${i.uptime_seconds!=null&&l`
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:Q_(i.uptime_seconds)})}</span>
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

      <${U} className="p-5 sm:p-6">
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
            value=${Ra(u.total_cost)}
            tone="signal"
          />
          <${et}
            label=${a("admin.dashboard.activeJobs")}
            value=${String(c.in_progress||0)}
            tone=${(c.in_progress||0)>0?"success":"muted"}
          />
        </div>
      <//>

      <${U} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.recentUsers")}</h3>
          <button
            onClick=${()=>t("users")}
            className="text-xs text-signal hover:underline"
          >
            ${a("admin.dashboard.viewAll")}
          </button>
        </div>
        <${pD} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var hD=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function vD({value:e,max:t}){let a=t>0?e/t*100:0;return l`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function W_({onSelectUser:e}){let t=k(),[a,n]=h.default.useState("day"),r=ed(a),s=r.data?.usage||[],i=Y_(s),o=J_(s),u=X_(i),c=i.length>0?i[0].cost:0;return r.isLoading?l`
      <${U} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
        <div className="grid gap-4 sm:grid-cols-4">
          ${[1,2,3,4].map(d=>l`<div key=${d} className="v2-skeleton h-28 rounded-lg" />`)}
        </div>
      <//>
    `:l`
    <div className="space-y-5">
      <${U} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.overview")}</h3>
          <div className="flex gap-1">
            ${hD.map(d=>l`
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
                <${et} label=${t("admin.usage.inputTokens")} value=${Va(u.input_tokens)} tone="muted" />
                <${et} label=${t("admin.usage.outputTokens")} value=${Va(u.output_tokens)} tone="muted" />
                <${et} label=${t("admin.usage.totalCost")} value=${Ra(u.cost.toFixed(2))} tone="signal" />
              </div>
            `}
      <//>

      ${i.length>0&&l`
        <${U} className="p-5 sm:p-6">
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
                          ${pi(d.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Va(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Va(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${Ra(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${vD} value=${d.cost} max=${c} />
                      </td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}

      ${o.length>0&&l`
        <${U} className="p-5 sm:p-6">
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
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Va(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Va(d.output_tokens)}</td>
                      <td className="py-3 font-mono text-xs text-iron-100">${Ra(d.cost.toFixed(2))}</td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}
    </div>
  `}function ir({label:e,children:t}){return l`
    <div className="flex items-start justify-between gap-4 border-t border-white/[0.06] py-3 first:border-0 first:pt-0">
      <span className="text-xs text-iron-300">${e}</span>
      <span className="text-right text-sm text-iron-100">${t}</span>
    </div>
  `}function ek({userId:e,onBack:t}){let a=k(),n=I_(e),r=ed("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:u,createToken:c,newToken:d,clearToken:f}=fi(),[m,p]=h.default.useState(null),[b,y]=h.default.useState(!1),$=n.data,g=r.data?.usage||[];if(h.default.useEffect(()=>{$&&m===null&&p($.role)},[$]),n.isLoading)return l`
      <div className="space-y-5">
        <${U} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-2 h-6 w-48 rounded" />
          <div className="v2-skeleton h-4 w-32 rounded" />
        <//>
      </div>
    `;if(n.error)return l`
      <${U} className="p-5 sm:p-6">
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

      <${U} className="p-5 sm:p-6">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <h2 className="text-2xl font-semibold tracking-tight text-white">${$.display_name||$.id}</h2>
            <div className="mt-2 flex items-center gap-2">
              <${j} tone=${vi($.role)} label=${$.role||"member"} />
              <${j} tone=${hi($.status)} label=${$.status||"active"} />
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            ${$.status==="active"?l`<${T} variant="secondary" onClick=${()=>s($.id)}>${a("admin.users.suspend")}<//>`:l`<${T} variant="secondary" onClick=${()=>i($.id)}>${a("admin.users.activate")}<//>`}
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
              <${A} name="close" className="h-4 w-4" />
            </button>
          </div>
        </div>
      `}

      <div className="grid gap-5 lg:grid-cols-2">
        <${U} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.profile")}</h3>
          <${ir} label=${a("admin.user.id")}>
            <span className="font-mono text-xs">${$.id}</span>
          <//>
          <${ir} label=${a("admin.user.email")}>${$.email||a("admin.user.notSet")}<//>
          <${ir} label=${a("admin.user.created")}>${sr($.created_at)}<//>
          <${ir} label=${a("admin.user.lastLogin")}>${sr($.last_login_at)}<//>
          ${$.created_by&&l`
            <${ir} label=${a("admin.user.createdBy")}>
              <span className="font-mono text-xs">${pi($.created_by)}</span>
            <//>
          `}
        <//>

        <${U} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${ir} label=${a("admin.user.jobs")}>${$.job_count??0}<//>
          <${ir} label=${a("admin.user.totalCost")}>${Ra($.total_cost)}<//>
          <${ir} label=${a("admin.user.lastActive")}>${sr($.last_active_at)}<//>
        <//>
      </div>

      <${U} className="p-5 sm:p-6">
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
          <${T} onClick=${v} disabled=${!m||m===$.role}>
            ${a("admin.user.saveRole")}
          <//>
        </div>
      <//>

      <${U} className="p-5 sm:p-6">
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
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Va(S.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Va(S.output_tokens)}</td>
                          <td className="py-3 font-mono text-xs text-iron-100">${Ra(S.total_cost)}</td>
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
              <${T} variant="ghost" onClick=${()=>y(!1)}>${a("admin.users.cancel")}<//>
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
  `}function gD(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function yD({token:e,onDismiss:t}){let a=k(),[n,r]=h.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return l`
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
          <${A} name="close" className="h-4 w-4" />
        </button>
      </div>
    </div>
  `}function bD({onCreate:e,isCreating:t,error:a}){let n=k(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState("member"),[d,f]=h.default.useState(!1),m=async p=>{p.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:u}),s(""),o(""),f(!1))};return d?l`
    <${U} className="p-5 sm:p-6">
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
          <${T} type="submit" disabled=${t}>
            ${n(t?"admin.users.creating":"admin.users.createUser")}
          <//>
          <${T} variant="ghost" type="button" onClick=${()=>f(!1)}>${n("admin.users.cancel")}<//>
        </div>
      </form>
    <//>
  `:l`
      <${T} variant="secondary" onClick=${()=>f(!0)}>
        <${A} name="plus" className="mr-2 h-4 w-4" />
        ${n("admin.users.newUser")}
      <//>
    `}function xD({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=k();return l`
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
  `}function $D({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=k();return l`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${j} tone=${vi(e.role)} label=${e.role||"member"} />
          <${j} tone=${hi(e.status)} label=${e.status||"active"} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          ${e.email&&l`<span className="font-mono text-xs text-iron-300">${e.email}</span>`}
          <span className="font-mono text-xs text-iron-700">${pi(e.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-iron-300 sm:inline">
          ${e.job_count!=null?i("admin.users.jobsCount",{count:e.job_count}):""}
          ${e.total_cost!=null?` \xB7 ${Ra(e.total_cost)}`:""}
        </span>
        <span className="hidden text-xs text-iron-700 lg:inline">${sr(e.last_active_at)}</span>
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
  `}function tk({selectedUserId:e,onSelectUser:t}){let a=k(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:u,updateUser:c,deleteUser:d,suspendUser:f,activateUser:m,createToken:p,newToken:b,clearToken:y}=fi(),[$,g]=h.default.useState(""),[v,x]=h.default.useState("all"),[w,S]=h.default.useState(null),R=G_(n,{search:$,filter:v}),_=gD(a),E=P=>{S({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{f(P),S(null)}})},M=async(P,I)=>{let D=window.prompt(a("admin.users.tokenNamePrompt",{name:I||a("admin.users.userFallback")}));D&&await p(P,D)};return r.isLoading?l`
      <${U} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-3 w-24 rounded" />
        ${[1,2,3].map(P=>l`
          <div key=${P} className="flex items-center justify-between border-t border-iron-700 py-3.5 first:border-0">
            <div className="v2-skeleton h-4 w-32 rounded" />
            <div className="v2-skeleton h-6 w-20 rounded-full" />
          </div>
        `)}
      <//>
    `:s?l`
      <${U} className="p-6 sm:p-8">
        <div className="flex items-center gap-3">
          <${A} name="lock" className="h-5 w-5 text-iron-700" />
          <h3 className="text-lg font-semibold text-iron-100">${a("users.adminRequired")}</h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${a("users.adminRequiredDesc")}
        </p>
      <//>
    `:l`
    <div className="space-y-5">
      ${b&&l`
        <${yD}
          token=${b.token||b.plaintext_token}
          onDismiss=${y}
        />
      `}

      <${bD} onCreate=${i} isCreating=${o} error=${u} />

      <${U} className="p-5 sm:p-6">
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
              ${_.map(P=>l`
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
                <${$D}
                  key=${P.id}
                  user=${P}
                  onSelect=${t}
                  onSuspend=${E}
                  onActivate=${m}
                  onChangeRole=${(I,D)=>c(I,{role:D})}
                  onCreateToken=${M}
                />
              `)}
      <//>

      ${w&&l`
        <${xD}
          title=${w.title}
          message=${w.message}
          confirmLabel=${w.confirmLabel}
          onConfirm=${w.onConfirm}
          onCancel=${()=>S(null)}
        />
      `}
    </div>
  `}function ak(){let{tab:e="dashboard"}=ot(),t=ce(),[a,n]=h.default.useState(null),r=h.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=h.default.useCallback(()=>{n(null)},[]),i={dashboard:l`<${Z_}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?l`<${ek} userId=${a} onBack=${s} />`:l`<${tk}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:l`<${W_} onSelectUser=${r} />`};return i[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:l`<${lt} to="/admin/dashboard" replace />`}var wD=2e3,SD=500,ND=2e3,_D=new Set([403,404]),kD=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function RD(e=globalThis.location){let t=new URLSearchParams(e?.search||"");return kD.reduce((a,[n,r,s])=>{let i=t.get(r)?.trim();return i?(a[n]=i,a.active.push({key:n,param:r,labelKey:s,value:i})):a[n]=null,a},{active:[]})}function nk(){let e=je(),t=h.default.useMemo(()=>RD(e),[e.search]),[a,n]=h.default.useState([]),[r,s]=h.default.useState("all"),[i,o]=h.default.useState(""),[u,c]=h.default.useState(!1),[d,f]=h.default.useState(!0),[m,p]=h.default.useState(!0),[b,y]=h.default.useState(null),[$,g]=h.default.useState(!1),v=h.default.useRef(new Set),x=h.default.useRef(0),w=h.default.useCallback(async()=>{if($)return;let _=++x.current;p(!0);try{let E=await bx({limit:SD,level:r==="all"?null:r,target:i.trim()||null,threadId:t.threadId,runId:t.runId,turnId:t.turnId,toolCallId:t.toolCallId,toolName:t.toolName,source:t.source});if(_!==x.current)return;let M=v.current,I=WS(E).entries.filter(D=>!M.has(D.id));n(I),y(null)}catch(E){if(_!==x.current)return;if(_D.has(E?.status)){n([]),y(null),g(!0);return}y(E)}finally{_===x.current&&p(!1)}},[$,r,t,i]);h.default.useEffect(()=>{w()},[w]),h.default.useEffect(()=>{if(u||$)return;let _=setInterval(w,wD);return()=>clearInterval(_)},[$,w,u]);let S=h.default.useCallback(()=>{c(_=>!_)},[]),R=h.default.useCallback(()=>{let _=[...v.current,...a.map(E=>E.id)].slice(-ND);v.current=new Set(_),n([])},[a]);return{entries:a,totalCount:a.length,paused:u,togglePause:S,clearEntries:R,levelFilter:r,setLevelFilter:s,targetFilter:i,setTargetFilter:o,autoScroll:d,setAutoScroll:f,serverLevel:null,changeServerLevel:async()=>{},scope:t,status:b?"error":m?"loading":"ready",isLoading:m,error:b}}var CD=["all","trace","debug","info","warn","error"],ED=["trace","debug","info","warn","error"],rk={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},TD={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function AD({entry:e}){let t=k(),[a,n]=h.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=rk[e.level]||rk.info,i=TD[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(u=>!!u.value);return l`
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
  `}function sk({value:e,onChange:t,options:a,labelKey:n,t:r}){return l`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>l`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function DD({label:e,value:t,scopeKey:a}){return l`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function ik(){let e=k(),{entries:t,totalCount:a,paused:n,togglePause:r,clearEntries:s,levelFilter:i,setLevelFilter:o,targetFilter:u,setTargetFilter:c,autoScroll:d,setAutoScroll:f,serverLevel:m,changeServerLevel:p,scope:b,isLoading:y,error:$}=nk(),g=h.default.useRef(null),v=h.default.useRef(!0);h.default.useEffect(()=>{d&&v.current&&g.current&&(g.current.scrollTop=0)},[t,d]);let x=h.default.useCallback(R=>{v.current=R.currentTarget.scrollTop<=48},[]),w=t.length>0,S=b?.active||[];return l`
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${sk}
          value=${i}
          onChange=${o}
          options=${CD}
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
            ${S.map(R=>l`<${DD} key=${R.param} scopeKey=${R.param} label=${e(R.labelKey)} value=${R.value} />`)}
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
            <${sk}
              value=${m}
              onChange=${p}
              options=${ED}
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
              `:w?t.map(R=>l`<${AD} key=${R.id} entry=${R} />`):l`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function lk(){return l`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function MD({auth:e}){let t=ce(),n=je().state?.from,r=n?`${n.pathname||Lr}${n.search||""}${n.hash||""}`:Lr,s=`/v2${r==="/"?"":r}`,i=h.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?l`<${lk} />`:e.isAuthenticated?l`<${lt} to=${r} replace />`:l`<${P1}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function OD({auth:e,children:t}){let a=je();return e.isChecking?l`<${lk} />`:e.isAuthenticated?t:l`<${lt} to="/login" replace state=${{from:a}} />`}function LD({auth:e}){return l`
    <${OD} auth=${e}>
      <${d1}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        rebornProjectsEnabled=${e.rebornProjectsEnabled}
        onSignOut=${e.signOut}
      />
    <//>
  `}function ok({auth:e}){return e.isAdmin?l`<${ak} />`:l`<${lt} to=${Lr} replace />`}function uk(){let e=a$();return l`
    <${mp} basename="/v2">
      <${up}>
        <${pe} path="/login" element=${l`<${MD} auth=${e} />`} />
        <${pe} path="/" element=${l`<${LD} auth=${e} />`}>
          <${pe} index element=${l`<${lt} to=${Lr} replace />`} />
          <${pe} path="overview" element=${l`<${lt} to=${Lr} replace />`} />
          <${pe} path="welcome" element=${l`<${r2} />`} />
          <${pe} path="chat" element=${l`<${th} />`} />
          <${pe} path="chat/:threadId" element=${l`<${th} />`} />
          <${pe} path="workspace" element=${l`<${nh} />`} />
          <${pe} path="workspace/*" element=${l`<${nh} />`} />
          <${pe} path="projects" element=${l`<${Yo} />`} />
          <${pe} path="projects/:projectId" element=${l`<${Yo} />`} />
          <${pe} path="projects/:projectId/missions/:missionId" element=${l`<${Yo} />`} />
          <${pe} path="projects/:projectId/threads/:threadId" element=${l`<${Yo} />`} />
          <${pe} path="missions" element=${l`<${sh} />`} />
          <${pe} path="missions/:missionId" element=${l`<${sh} />`} />
          <${pe} path="jobs" element=${l`<${lh} />`} />
          <${pe} path="jobs/:jobId" element=${l`<${lh} />`} />
          <${pe} path="routines" element=${l`<${ch} />`} />
          <${pe} path="routines/:routineId" element=${l`<${ch} />`} />
          <${pe} path="automations" element=${l`<${uN} />`} />
          <${pe} path="extensions" element=${l`<${xh} />`} />
          <${pe} path="extensions/:tab" element=${l`<${xh} />`} />
          <${pe} path="logs" element=${l`<${ik} />`} />
          <${pe} path="settings" element=${l`<${Sh} />`} />
          <${pe} path="settings/:tab" element=${l`<${Sh} />`} />
          <${pe} path="admin" element=${l`<${ok} auth=${e} />`} />
          <${pe} path="admin/:tab" element=${l`<${ok} auth=${e} />`} />
        <//>
        <${pe} path="*" element=${l`<${lt} to=${Lr} replace />`} />
      <//>
    <//>
  `}Rh("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","approval.viewFullCommand":"View full command","approval.showCommandPreview":"Show preview","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.serviceUnavailable":"Service unavailable","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveTools":"Auto-approve tools","settings.field.autoApproveToolsDesc":"Skip approval for all tool calls","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Memory, files & attachments","workspace.readOnly":"Read-only","workspace.filterPlaceholder":"Filter by name\u2026","workspace.emptyDir":"This folder is empty.","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.noFiles":"No files here.","workspace.noMatches":"Nothing matches that filter.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a file","workspace.pickFileDesc":"Choose a file from the tree to preview or download it. This viewer is read-only.","workspace.parent":"Parent: {path}","workspace.download":"Download","workspace.binaryPreviewUnavailable":"No inline preview for this file type. Download it to view the contents.","workspace.fileMeta":"{mime} \xB7 {size} bytes","workspace.unableOpenDirectory":"Unable to open directory","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.filter.completed":"Completed","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.schedule.onceAt":"Once on {datetime}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.filterAction":"Show {label}","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.empty.onboardingTitle":"No automations yet","automations.empty.onboardingDescription":"Automations are created by chatting with your agent \u2014 there's no form to fill out. Ask it to do something on a schedule and it will set up a recurring automation for you.","automations.empty.examplesTitle":"Try asking your agent","automations.empty.example1":"Check the nearai/ironclaw repo every 10 minutes and summarize new issues, PRs, and commits.","automations.empty.example2":"Every weekday at 9am, send me a summary of my unread email.","automations.empty.example3":"Remind me to review open pull requests every afternoon at 3pm.","automations.empty.startInChat":"Start in chat","automations.empty.copyPrompt":"Copy prompt","automations.empty.copied":"Copied","automations.refreshing":"Refreshing\u2026","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.runs.total":"Recent runs: {count}","automations.runs.ok":"OK: {count}","automations.runs.error":"Failed: {count}","automations.runs.running":"Running: {count}","automations.runs.unknown":"Unknown: {count}","automations.runs.showingOf":"Showing {shown} of {total} recent runs","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up a project for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open project","projects.openGeneralWorkspace":"Open project","projects.noDescription":"No project description yet. The project is still being shaped by recent activity and thread history.","projects.general.label":"General project","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,ck.createRoot)(document.getElementById("v2-root")).render(l`
  <${Ch}>
    <${hd} client=${Tt}>
      <${uk} />
    <//>
  <//>
`);
