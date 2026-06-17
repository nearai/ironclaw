import{a as $n,b as qe,c as Ie,d as h,e as l,f as xh,g as $h,h as tl,i as k,j as al}from"./chunks/chunk-CHHX6LNQ.js";var zh=$n(dl=>{"use strict";var uk=Symbol.for("react.transitional.element"),ck=Symbol.for("react.fragment");function Fh(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:uk,type:e,key:n,ref:t!==void 0?t:null,props:a}}dl.Fragment=ck;dl.jsx=Fh;dl.jsxs=Fh});var ud=$n((wM,qh)=>{"use strict";qh.exports=zh()});var tv=$n(Oe=>{"use strict";function vd(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<xl(r,t))e[n]=t,e[a]=r,a=n;else break e}}function Da(e){return e.length===0?null:e[0]}function wl(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],u=i+1,c=e[u];if(0>xl(o,a))u<r&&0>xl(c,o)?(e[n]=c,e[u]=a,n=u):(e[n]=o,e[i]=a,n=i);else if(u<r&&0>xl(c,a))e[n]=c,e[u]=a,n=u;else break e}}return t}function xl(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}Oe.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(Qh=performance,Oe.unstable_now=function(){return Qh.now()}):(fd=Date,Vh=fd.now(),Oe.unstable_now=function(){return fd.now()-Vh});var Qh,fd,Vh,Ga=[],Nn=[],pk=1,la=null,$t=3,gd=!1,Ni=!1,_i=!1,yd=!1,Jh=typeof setTimeout=="function"?setTimeout:null,Xh=typeof clearTimeout=="function"?clearTimeout:null,Gh=typeof setImmediate<"u"?setImmediate:null;function $l(e){for(var t=Da(Nn);t!==null;){if(t.callback===null)wl(Nn);else if(t.startTime<=e)wl(Nn),t.sortIndex=t.expirationTime,vd(Ga,t);else break;t=Da(Nn)}}function bd(e){if(_i=!1,$l(e),!Ni)if(Da(Ga)!==null)Ni=!0,Vr||(Vr=!0,Qr());else{var t=Da(Nn);t!==null&&xd(bd,t.startTime-e)}}var Vr=!1,ki=-1,Zh=5,Wh=-1;function ev(){return yd?!0:!(Oe.unstable_now()-Wh<Zh)}function pd(){if(yd=!1,Vr){var e=Oe.unstable_now();Wh=e;var t=!0;try{e:{Ni=!1,_i&&(_i=!1,Xh(ki),ki=-1),gd=!0;var a=$t;try{t:{for($l(e),la=Da(Ga);la!==null&&!(la.expirationTime>e&&ev());){var n=la.callback;if(typeof n=="function"){la.callback=null,$t=la.priorityLevel;var r=n(la.expirationTime<=e);if(e=Oe.unstable_now(),typeof r=="function"){la.callback=r,$l(e),t=!0;break t}la===Da(Ga)&&wl(Ga),$l(e)}else wl(Ga);la=Da(Ga)}if(la!==null)t=!0;else{var s=Da(Nn);s!==null&&xd(bd,s.startTime-e),t=!1}}break e}finally{la=null,$t=a,gd=!1}t=void 0}}finally{t?Qr():Vr=!1}}}var Qr;typeof Gh=="function"?Qr=function(){Gh(pd)}:typeof MessageChannel<"u"?(hd=new MessageChannel,Yh=hd.port2,hd.port1.onmessage=pd,Qr=function(){Yh.postMessage(null)}):Qr=function(){Jh(pd,0)};var hd,Yh;function xd(e,t){ki=Jh(function(){e(Oe.unstable_now())},t)}Oe.unstable_IdlePriority=5;Oe.unstable_ImmediatePriority=1;Oe.unstable_LowPriority=4;Oe.unstable_NormalPriority=3;Oe.unstable_Profiling=null;Oe.unstable_UserBlockingPriority=2;Oe.unstable_cancelCallback=function(e){e.callback=null};Oe.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):Zh=0<e?Math.floor(1e3/e):5};Oe.unstable_getCurrentPriorityLevel=function(){return $t};Oe.unstable_next=function(e){switch($t){case 1:case 2:case 3:var t=3;break;default:t=$t}var a=$t;$t=t;try{return e()}finally{$t=a}};Oe.unstable_requestPaint=function(){yd=!0};Oe.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=$t;$t=e;try{return t()}finally{$t=a}};Oe.unstable_scheduleCallback=function(e,t,a){var n=Oe.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:pk++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,vd(Nn,e),Da(Ga)===null&&e===Da(Nn)&&(_i?(Xh(ki),ki=-1):_i=!0,xd(bd,a-n))):(e.sortIndex=r,vd(Ga,e),Ni||gd||(Ni=!0,Vr||(Vr=!0,Qr()))),e};Oe.unstable_shouldYield=ev;Oe.unstable_wrapCallback=function(e){var t=$t;return function(){var a=$t;$t=t;try{return e.apply(this,arguments)}finally{$t=a}}}});var nv=$n((rO,av)=>{"use strict";av.exports=tv()});var sv=$n(Et=>{"use strict";var hk=Ie();function rv(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function _n(){}var Ct={d:{f:_n,r:function(){throw Error(rv(522))},D:_n,C:_n,L:_n,m:_n,X:_n,S:_n,M:_n},p:0,findDOMNode:null},vk=Symbol.for("react.portal");function gk(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:vk,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var Ri=hk.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function Sl(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}Et.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=Ct;Et.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(rv(299));return gk(e,t,null,a)};Et.flushSync=function(e){var t=Ri.T,a=Ct.p;try{if(Ri.T=null,Ct.p=2,e)return e()}finally{Ri.T=t,Ct.p=a,Ct.d.f()}};Et.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,Ct.d.C(e,t))};Et.prefetchDNS=function(e){typeof e=="string"&&Ct.d.D(e)};Et.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=Sl(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?Ct.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&Ct.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};Et.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=Sl(t.as,t.crossOrigin);Ct.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&Ct.d.M(e)};Et.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=Sl(a,t.crossOrigin);Ct.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};Et.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=Sl(t.as,t.crossOrigin);Ct.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else Ct.d.m(e)};Et.requestFormReset=function(e){Ct.d.r(e)};Et.unstable_batchedUpdates=function(e,t){return e(t)};Et.useFormState=function(e,t,a){return Ri.H.useFormState(e,t,a)};Et.useFormStatus=function(){return Ri.H.useHostTransitionStatus()};Et.version="19.1.0"});var lv=$n((iO,ov)=>{"use strict";function iv(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(iv)}catch(e){console.error(e)}}iv(),ov.exports=sv()});var c0=$n(Iu=>{"use strict";var st=nv(),Tg=Ie(),yk=lv();function L(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Ag(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function ho(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function Dg(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function uv(e){if(ho(e)!==e)throw Error(L(188))}function bk(e){var t=e.alternate;if(!t){if(t=ho(e),t===null)throw Error(L(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return uv(r),e;if(s===n)return uv(r),t;s=s.sibling}throw Error(L(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(L(189))}}if(a.alternate!==n)throw Error(L(190))}if(a.tag!==3)throw Error(L(188));return a.stateNode.current===a?e:t}function Mg(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=Mg(e),t!==null)return t;e=e.sibling}return null}var De=Object.assign,xk=Symbol.for("react.element"),Nl=Symbol.for("react.transitional.element"),Ui=Symbol.for("react.portal"),es=Symbol.for("react.fragment"),Og=Symbol.for("react.strict_mode"),Zd=Symbol.for("react.profiler"),$k=Symbol.for("react.provider"),Lg=Symbol.for("react.consumer"),Wa=Symbol.for("react.context"),Vm=Symbol.for("react.forward_ref"),Wd=Symbol.for("react.suspense"),em=Symbol.for("react.suspense_list"),Gm=Symbol.for("react.memo"),Cn=Symbol.for("react.lazy");Symbol.for("react.scope");var tm=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var wk=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var cv=Symbol.iterator;function Ci(e){return e===null||typeof e!="object"?null:(e=cv&&e[cv]||e["@@iterator"],typeof e=="function"?e:null)}var Sk=Symbol.for("react.client.reference");function am(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===Sk?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case es:return"Fragment";case Zd:return"Profiler";case Og:return"StrictMode";case Wd:return"Suspense";case em:return"SuspenseList";case tm:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case Ui:return"Portal";case Wa:return(e.displayName||"Context")+".Provider";case Lg:return(e._context.displayName||"Context")+".Consumer";case Vm:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case Gm:return t=e.displayName||null,t!==null?t:am(e.type)||"Memo";case Cn:t=e._payload,e=e._init;try{return am(e(t))}catch{}}return null}var ji=Array.isArray,ee=Tg.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,de=yk.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,fr={pending:!1,data:null,method:null,action:null},nm=[],ts=-1;function Fa(e){return{current:e}}function ft(e){0>ts||(e.current=nm[ts],nm[ts]=null,ts--)}function Ue(e,t){ts++,nm[ts]=e.current,e.current=t}var Ua=Fa(null),eo=Fa(null),Pn=Fa(null),eu=Fa(null);function tu(e,t){switch(Ue(Pn,t),Ue(eo,e),Ue(Ua,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?vg(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=vg(t),e=Zb(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}ft(Ua),Ue(Ua,e)}function xs(){ft(Ua),ft(eo),ft(Pn)}function rm(e){e.memoizedState!==null&&Ue(eu,e);var t=Ua.current,a=Zb(t,e.type);t!==a&&(Ue(eo,e),Ue(Ua,a))}function au(e){eo.current===e&&(ft(Ua),ft(eo)),eu.current===e&&(ft(eu),co._currentValue=fr)}var sm=Object.prototype.hasOwnProperty,Ym=st.unstable_scheduleCallback,$d=st.unstable_cancelCallback,Nk=st.unstable_shouldYield,_k=st.unstable_requestPaint,ja=st.unstable_now,kk=st.unstable_getCurrentPriorityLevel,Ug=st.unstable_ImmediatePriority,jg=st.unstable_UserBlockingPriority,nu=st.unstable_NormalPriority,Rk=st.unstable_LowPriority,Pg=st.unstable_IdlePriority,Ck=st.log,Ek=st.unstable_setDisableYieldValue,vo=null,Xt=null;function On(e){if(typeof Ck=="function"&&Ek(e),Xt&&typeof Xt.setStrictMode=="function")try{Xt.setStrictMode(vo,e)}catch{}}var Zt=Math.clz32?Math.clz32:Dk,Tk=Math.log,Ak=Math.LN2;function Dk(e){return e>>>=0,e===0?32:31-(Tk(e)/Ak|0)|0}var _l=256,kl=4194304;function cr(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Tu(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=cr(n):(i&=o,i!==0?r=cr(i):a||(a=o&~e,a!==0&&(r=cr(a))))):(o=n&~s,o!==0?r=cr(o):i!==0?r=cr(i):a||(a=n&~e,a!==0&&(r=cr(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function go(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function Mk(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function Fg(){var e=_l;return _l<<=1,(_l&4194048)===0&&(_l=256),e}function zg(){var e=kl;return kl<<=1,(kl&62914560)===0&&(kl=4194304),e}function wd(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function yo(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function Ok(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,u=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Zt(a),f=1<<d;o[d]=0,u[d]=-1;var m=c[d];if(m!==null)for(c[d]=null,d=0;d<m.length;d++){var p=m[d];p!==null&&(p.lane&=-536870913)}a&=~f}n!==0&&qg(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function qg(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Zt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function Bg(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Zt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function Jm(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function Xm(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function Hg(){var e=de.p;return e!==0?e:(e=window.event,e===void 0?32:l0(e.type))}function Lk(e,t){var a=de.p;try{return de.p=e,t()}finally{de.p=a}}var Yn=Math.random().toString(36).slice(2),wt="__reactFiber$"+Yn,Bt="__reactProps$"+Yn,As="__reactContainer$"+Yn,im="__reactEvents$"+Yn,Uk="__reactListeners$"+Yn,jk="__reactHandles$"+Yn,dv="__reactResources$"+Yn,bo="__reactMarker$"+Yn;function Zm(e){delete e[wt],delete e[Bt],delete e[im],delete e[Uk],delete e[jk]}function as(e){var t=e[wt];if(t)return t;for(var a=e.parentNode;a;){if(t=a[As]||a[wt]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=bg(e);e!==null;){if(a=e[wt])return a;e=bg(e)}return t}e=a,a=e.parentNode}return null}function Ds(e){if(e=e[wt]||e[As]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function Pi(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(L(33))}function ms(e){var t=e[dv];return t||(t=e[dv]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function dt(e){e[bo]=!0}var Kg=new Set,Ig={};function Nr(e,t){$s(e,t),$s(e+"Capture",t)}function $s(e,t){for(Ig[e]=t,e=0;e<t.length;e++)Kg.add(t[e])}var Pk=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),mv={},fv={};function Fk(e){return sm.call(fv,e)?!0:sm.call(mv,e)?!1:Pk.test(e)?fv[e]=!0:(mv[e]=!0,!1)}function ql(e,t,a){if(Fk(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function Rl(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function Ya(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var Sd,pv;function Xr(e){if(Sd===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);Sd=t&&t[1]||"",pv=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+Sd+e+pv}var Nd=!1;function _d(e,t){if(!e||Nd)return"";Nd=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var f=function(){throw Error()};if(Object.defineProperty(f.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(f,[])}catch(p){var m=p}Reflect.construct(e,[],f)}else{try{f.call()}catch(p){m=p}e.call(f.prototype)}}else{try{throw Error()}catch(p){m=p}(f=e())&&typeof f.catch=="function"&&f.catch(function(){})}}catch(p){if(p&&m&&typeof p.stack=="string")return[p.stack,m.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var u=i.split(`
`),c=o.split(`
`);for(r=n=0;n<u.length&&!u[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===u.length||r===c.length)for(n=u.length-1,r=c.length-1;1<=n&&0<=r&&u[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(u[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||u[n]!==c[r]){var d=`
`+u[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{Nd=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?Xr(a):""}function zk(e){switch(e.tag){case 26:case 27:case 5:return Xr(e.type);case 16:return Xr("Lazy");case 13:return Xr("Suspense");case 19:return Xr("SuspenseList");case 0:case 15:return _d(e.type,!1);case 11:return _d(e.type.render,!1);case 1:return _d(e.type,!0);case 31:return Xr("Activity");default:return""}}function hv(e){try{var t="";do t+=zk(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function ca(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function Qg(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function qk(e){var t=Qg(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function ru(e){e._valueTracker||(e._valueTracker=qk(e))}function Vg(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=Qg(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function su(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var Bk=/[\n"\\]/g;function fa(e){return e.replace(Bk,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function om(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+ca(t)):e.value!==""+ca(t)&&(e.value=""+ca(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?lm(e,i,ca(t)):a!=null?lm(e,i,ca(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+ca(o):e.removeAttribute("name")}function Gg(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+ca(a):"",t=t!=null?""+ca(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function lm(e,t,a){t==="number"&&su(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function fs(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+ca(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function Yg(e,t,a){if(t!=null&&(t=""+ca(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+ca(a):""}function Jg(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(L(92));if(ji(n)){if(1<n.length)throw Error(L(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=ca(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function ws(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var Hk=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function vv(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||Hk.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function Xg(e,t,a){if(t!=null&&typeof t!="object")throw Error(L(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&vv(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&vv(e,s,t[s])}function Wm(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var Kk=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),Ik=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function Bl(e){return Ik.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var um=null;function ef(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var ns=null,ps=null;function gv(e){var t=Ds(e);if(t&&(e=t.stateNode)){var a=e[Bt]||null;e:switch(e=t.stateNode,t.type){case"input":if(om(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+fa(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[Bt]||null;if(!r)throw Error(L(90));om(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&Vg(n)}break e;case"textarea":Yg(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&fs(e,!!a.multiple,t,!1)}}}var kd=!1;function Zg(e,t,a){if(kd)return e(t,a);kd=!0;try{var n=e(t);return n}finally{if(kd=!1,(ns!==null||ps!==null)&&(zu(),ns&&(t=ns,e=ps,ps=ns=null,gv(t),e)))for(t=0;t<e.length;t++)gv(e[t])}}function to(e,t){var a=e.stateNode;if(a===null)return null;var n=a[Bt]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(L(231,t,typeof a));return a}var on=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),cm=!1;if(on)try{Gr={},Object.defineProperty(Gr,"passive",{get:function(){cm=!0}}),window.addEventListener("test",Gr,Gr),window.removeEventListener("test",Gr,Gr)}catch{cm=!1}var Gr,Ln=null,tf=null,Hl=null;function Wg(){if(Hl)return Hl;var e,t=tf,a=t.length,n,r="value"in Ln?Ln.value:Ln.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return Hl=r.slice(e,1<n?1-n:void 0)}function Kl(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function Cl(){return!0}function yv(){return!1}function Ht(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?Cl:yv,this.isPropagationStopped=yv,this}return De(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=Cl)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=Cl)},persist:function(){},isPersistent:Cl}),t}var _r={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Au=Ht(_r),xo=De({},_r,{view:0,detail:0}),Qk=Ht(xo),Rd,Cd,Ei,Du=De({},xo,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:af,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==Ei&&(Ei&&e.type==="mousemove"?(Rd=e.screenX-Ei.screenX,Cd=e.screenY-Ei.screenY):Cd=Rd=0,Ei=e),Rd)},movementY:function(e){return"movementY"in e?e.movementY:Cd}}),bv=Ht(Du),Vk=De({},Du,{dataTransfer:0}),Gk=Ht(Vk),Yk=De({},xo,{relatedTarget:0}),Ed=Ht(Yk),Jk=De({},_r,{animationName:0,elapsedTime:0,pseudoElement:0}),Xk=Ht(Jk),Zk=De({},_r,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),Wk=Ht(Zk),eR=De({},_r,{data:0}),xv=Ht(eR),tR={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},aR={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},nR={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function rR(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=nR[e])?!!t[e]:!1}function af(){return rR}var sR=De({},xo,{key:function(e){if(e.key){var t=tR[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=Kl(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?aR[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:af,charCode:function(e){return e.type==="keypress"?Kl(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?Kl(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),iR=Ht(sR),oR=De({},Du,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),$v=Ht(oR),lR=De({},xo,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:af}),uR=Ht(lR),cR=De({},_r,{propertyName:0,elapsedTime:0,pseudoElement:0}),dR=Ht(cR),mR=De({},Du,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),fR=Ht(mR),pR=De({},_r,{newState:0,oldState:0}),hR=Ht(pR),vR=[9,13,27,32],nf=on&&"CompositionEvent"in window,zi=null;on&&"documentMode"in document&&(zi=document.documentMode);var gR=on&&"TextEvent"in window&&!zi,ey=on&&(!nf||zi&&8<zi&&11>=zi),wv=" ",Sv=!1;function ty(e,t){switch(e){case"keyup":return vR.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function ay(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var rs=!1;function yR(e,t){switch(e){case"compositionend":return ay(t);case"keypress":return t.which!==32?null:(Sv=!0,wv);case"textInput":return e=t.data,e===wv&&Sv?null:e;default:return null}}function bR(e,t){if(rs)return e==="compositionend"||!nf&&ty(e,t)?(e=Wg(),Hl=tf=Ln=null,rs=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return ey&&t.locale!=="ko"?null:t.data;default:return null}}var xR={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function Nv(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!xR[e.type]:t==="textarea"}function ny(e,t,a,n){ns?ps?ps.push(n):ps=[n]:ns=n,t=Su(t,"onChange"),0<t.length&&(a=new Au("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var qi=null,ao=null;function $R(e){Yb(e,0)}function Mu(e){var t=Pi(e);if(Vg(t))return e}function _v(e,t){if(e==="change")return t}var ry=!1;on&&(on?(Tl="oninput"in document,Tl||(Td=document.createElement("div"),Td.setAttribute("oninput","return;"),Tl=typeof Td.oninput=="function"),El=Tl):El=!1,ry=El&&(!document.documentMode||9<document.documentMode));var El,Tl,Td;function kv(){qi&&(qi.detachEvent("onpropertychange",sy),ao=qi=null)}function sy(e){if(e.propertyName==="value"&&Mu(ao)){var t=[];ny(t,ao,e,ef(e)),Zg($R,t)}}function wR(e,t,a){e==="focusin"?(kv(),qi=t,ao=a,qi.attachEvent("onpropertychange",sy)):e==="focusout"&&kv()}function SR(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return Mu(ao)}function NR(e,t){if(e==="click")return Mu(t)}function _R(e,t){if(e==="input"||e==="change")return Mu(t)}function kR(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var ta=typeof Object.is=="function"?Object.is:kR;function no(e,t){if(ta(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!sm.call(t,r)||!ta(e[r],t[r]))return!1}return!0}function Rv(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function Cv(e,t){var a=Rv(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=Rv(a)}}function iy(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?iy(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function oy(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=su(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=su(e.document)}return t}function rf(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var RR=on&&"documentMode"in document&&11>=document.documentMode,ss=null,dm=null,Bi=null,mm=!1;function Ev(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;mm||ss==null||ss!==su(n)||(n=ss,"selectionStart"in n&&rf(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),Bi&&no(Bi,n)||(Bi=n,n=Su(dm,"onSelect"),0<n.length&&(t=new Au("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=ss)))}function ur(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var is={animationend:ur("Animation","AnimationEnd"),animationiteration:ur("Animation","AnimationIteration"),animationstart:ur("Animation","AnimationStart"),transitionrun:ur("Transition","TransitionRun"),transitionstart:ur("Transition","TransitionStart"),transitioncancel:ur("Transition","TransitionCancel"),transitionend:ur("Transition","TransitionEnd")},Ad={},ly={};on&&(ly=document.createElement("div").style,"AnimationEvent"in window||(delete is.animationend.animation,delete is.animationiteration.animation,delete is.animationstart.animation),"TransitionEvent"in window||delete is.transitionend.transition);function kr(e){if(Ad[e])return Ad[e];if(!is[e])return e;var t=is[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in ly)return Ad[e]=t[a];return e}var uy=kr("animationend"),cy=kr("animationiteration"),dy=kr("animationstart"),CR=kr("transitionrun"),ER=kr("transitionstart"),TR=kr("transitioncancel"),my=kr("transitionend"),fy=new Map,fm="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");fm.push("scrollEnd");function _a(e,t){fy.set(e,t),Nr(t,[e])}var Tv=new WeakMap;function pa(e,t){if(typeof e=="object"&&e!==null){var a=Tv.get(e);return a!==void 0?a:(t={value:e,source:t,stack:hv(t)},Tv.set(e,t),t)}return{value:e,source:t,stack:hv(t)}}var ua=[],os=0,sf=0;function Ou(){for(var e=os,t=sf=os=0;t<e;){var a=ua[t];ua[t++]=null;var n=ua[t];ua[t++]=null;var r=ua[t];ua[t++]=null;var s=ua[t];if(ua[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&py(a,r,s)}}function Lu(e,t,a,n){ua[os++]=e,ua[os++]=t,ua[os++]=a,ua[os++]=n,sf|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function of(e,t,a,n){return Lu(e,t,a,n),iu(e)}function Ms(e,t){return Lu(e,null,null,t),iu(e)}function py(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Zt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function iu(e){if(50<Zi)throw Zi=0,Om=null,Error(L(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var ls={};function AR(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Jt(e,t,a,n){return new AR(e,t,a,n)}function lf(e){return e=e.prototype,!(!e||!e.isReactComponent)}function rn(e,t){var a=e.alternate;return a===null?(a=Jt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function hy(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function Il(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")lf(e)&&(i=1);else if(typeof e=="string")i=AC(e,a,Ua.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case tm:return e=Jt(31,a,t,r),e.elementType=tm,e.lanes=s,e;case es:return pr(a.children,r,s,t);case Og:i=8,r|=24;break;case Zd:return e=Jt(12,a,t,r|2),e.elementType=Zd,e.lanes=s,e;case Wd:return e=Jt(13,a,t,r),e.elementType=Wd,e.lanes=s,e;case em:return e=Jt(19,a,t,r),e.elementType=em,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case $k:case Wa:i=10;break e;case Lg:i=9;break e;case Vm:i=11;break e;case Gm:i=14;break e;case Cn:i=16,n=null;break e}i=29,a=Error(L(130,e===null?"null":typeof e,"")),n=null}return t=Jt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function pr(e,t,a,n){return e=Jt(7,e,n,t),e.lanes=a,e}function Dd(e,t,a){return e=Jt(6,e,null,t),e.lanes=a,e}function Md(e,t,a){return t=Jt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var us=[],cs=0,ou=null,lu=0,da=[],ma=0,hr=null,en=1,tn="";function dr(e,t){us[cs++]=lu,us[cs++]=ou,ou=e,lu=t}function vy(e,t,a){da[ma++]=en,da[ma++]=tn,da[ma++]=hr,hr=e;var n=en;e=tn;var r=32-Zt(n)-1;n&=~(1<<r),a+=1;var s=32-Zt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,en=1<<32-Zt(t)+r|a<<r|n,tn=s+e}else en=1<<s|a<<r|n,tn=e}function uf(e){e.return!==null&&(dr(e,1),vy(e,1,0))}function cf(e){for(;e===ou;)ou=us[--cs],us[cs]=null,lu=us[--cs],us[cs]=null;for(;e===hr;)hr=da[--ma],da[ma]=null,tn=da[--ma],da[ma]=null,en=da[--ma],da[ma]=null}var Tt=null,Be=null,ce=!1,vr=null,Oa=!1,pm=Error(L(519));function xr(e){var t=Error(L(418,""));throw ro(pa(t,e)),pm}function Av(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[wt]=e,t[Bt]=n,a){case"dialog":se("cancel",t),se("close",t);break;case"iframe":case"object":case"embed":se("load",t);break;case"video":case"audio":for(a=0;a<oo.length;a++)se(oo[a],t);break;case"source":se("error",t);break;case"img":case"image":case"link":se("error",t),se("load",t);break;case"details":se("toggle",t);break;case"input":se("invalid",t),Gg(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),ru(t);break;case"select":se("invalid",t);break;case"textarea":se("invalid",t),Jg(t,n.value,n.defaultValue,n.children),ru(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||Xb(t.textContent,a)?(n.popover!=null&&(se("beforetoggle",t),se("toggle",t)),n.onScroll!=null&&se("scroll",t),n.onScrollEnd!=null&&se("scrollend",t),n.onClick!=null&&(t.onclick=Hu),t=!0):t=!1,t||xr(e)}function Dv(e){for(Tt=e.return;Tt;)switch(Tt.tag){case 5:case 13:Oa=!1;return;case 27:case 3:Oa=!0;return;default:Tt=Tt.return}}function Ti(e){if(e!==Tt)return!1;if(!ce)return Dv(e),ce=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||zm(e.type,e.memoizedProps)),a=!a),a&&Be&&xr(e),Dv(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(L(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){Be=Na(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}Be=null}}else t===27?(t=Be,Jn(e.type)?(e=Hm,Hm=null,Be=e):Be=t):Be=Tt?Na(e.stateNode.nextSibling):null;return!0}function $o(){Be=Tt=null,ce=!1}function Mv(){var e=vr;return e!==null&&(qt===null?qt=e:qt.push.apply(qt,e),vr=null),e}function ro(e){vr===null?vr=[e]:vr.push(e)}var hm=Fa(null),Rr=null,an=null;function Tn(e,t,a){Ue(hm,t._currentValue),t._currentValue=a}function sn(e){e._currentValue=hm.current,ft(hm)}function vm(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function gm(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var u=0;u<t.length;u++)if(o.context===t[u]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),vm(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(L(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),vm(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function wo(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(L(387));if(i=i.memoizedProps,i!==null){var o=r.type;ta(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===eu.current){if(i=r.alternate,i===null)throw Error(L(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(co):e=[co])}r=r.return}e!==null&&gm(t,e,a,n),t.flags|=262144}function uu(e){for(e=e.firstContext;e!==null;){if(!ta(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function $r(e){Rr=e,an=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function St(e){return gy(Rr,e)}function Al(e,t){return Rr===null&&$r(e),gy(e,t)}function gy(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},an===null){if(e===null)throw Error(L(308));an=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else an=an.next=t;return a}var DR=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},MR=st.unstable_scheduleCallback,OR=st.unstable_NormalPriority,nt={$$typeof:Wa,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function df(){return{controller:new DR,data:new Map,refCount:0}}function So(e){e.refCount--,e.refCount===0&&MR(OR,function(){e.controller.abort()})}var Hi=null,ym=0,Ss=0,hs=null;function LR(e,t){if(Hi===null){var a=Hi=[];ym=0,Ss=Of(),hs={status:"pending",value:void 0,then:function(n){a.push(n)}}}return ym++,t.then(Ov,Ov),t}function Ov(){if(--ym===0&&Hi!==null){hs!==null&&(hs.status="fulfilled");var e=Hi;Hi=null,Ss=0,hs=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function UR(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var Lv=ee.S;ee.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&LR(e,t),Lv!==null&&Lv(e,t)};var gr=Fa(null);function mf(){var e=gr.current;return e!==null?e:Ce.pooledCache}function Ql(e,t){t===null?Ue(gr,gr.current):Ue(gr,t.pool)}function yy(){var e=mf();return e===null?null:{parent:nt._currentValue,pool:e}}var No=Error(L(460)),by=Error(L(474)),Uu=Error(L(542)),bm={then:function(){}};function Uv(e){return e=e.status,e==="fulfilled"||e==="rejected"}function Dl(){}function xy(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(Dl,Dl),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,Pv(e),e;default:if(typeof t.status=="string")t.then(Dl,Dl);else{if(e=Ce,e!==null&&100<e.shellSuspendCounter)throw Error(L(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,Pv(e),e}throw Ki=t,No}}var Ki=null;function jv(){if(Ki===null)throw Error(L(459));var e=Ki;return Ki=null,e}function Pv(e){if(e===No||e===Uu)throw Error(L(483))}var En=!1;function ff(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function xm(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function Fn(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function zn(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,($e&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=iu(e),py(e,null,a),t}return Lu(e,n,t,a),iu(e)}function Ii(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,Bg(e,a)}}function Od(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var $m=!1;function Qi(){if($m){var e=hs;if(e!==null)throw e}}function Vi(e,t,a,n){$m=!1;var r=e.updateQueue;En=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var u=o,c=u.next;u.next=null,i===null?s=c:i.next=c,i=u;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=u))}if(s!==null){var f=r.baseState;i=0,d=c=u=null,o=s;do{var m=o.lane&-536870913,p=m!==o.lane;if(p?(le&m)===m:(n&m)===m){m!==0&&m===Ss&&($m=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var y=e,b=o;m=t;var w=a;switch(b.tag){case 1:if(y=b.payload,typeof y=="function"){f=y.call(w,f,m);break e}f=y;break e;case 3:y.flags=y.flags&-65537|128;case 0:if(y=b.payload,m=typeof y=="function"?y.call(w,f,m):y,m==null)break e;f=De({},f,m);break e;case 2:En=!0}}m=o.callback,m!==null&&(e.flags|=64,p&&(e.flags|=8192),p=r.callbacks,p===null?r.callbacks=[m]:p.push(m))}else p={lane:m,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=p,u=f):d=d.next=p,i|=m;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;p=o,o=p.next,p.next=null,r.lastBaseUpdate=p,r.shared.pending=null}}while(!0);d===null&&(u=f),r.baseState=u,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),Gn|=i,e.lanes=i,e.memoizedState=f}}function $y(e,t){if(typeof e!="function")throw Error(L(191,e));e.call(t)}function wy(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)$y(a[e],t)}var Ns=Fa(null),cu=Fa(0);function Fv(e,t){e=cn,Ue(cu,e),Ue(Ns,t),cn=e|t.baseLanes}function wm(){Ue(cu,cn),Ue(Ns,Ns.current)}function pf(){cn=cu.current,ft(Ns),ft(cu)}var Qn=0,re=null,Se=null,Xe=null,du=!1,vs=!1,wr=!1,mu=0,so=0,gs=null,jR=0;function Qe(){throw Error(L(321))}function hf(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!ta(e[a],t[a]))return!1;return!0}function vf(e,t,a,n,r,s){return Qn=s,re=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,ee.H=e===null||e.memoizedState===null?Wy:eb,wr=!1,s=a(n,r),wr=!1,vs&&(s=Ny(t,a,n,r)),Sy(e),s}function Sy(e){ee.H=fu;var t=Se!==null&&Se.next!==null;if(Qn=0,Xe=Se=re=null,du=!1,so=0,gs=null,t)throw Error(L(300));e===null||mt||(e=e.dependencies,e!==null&&uu(e)&&(mt=!0))}function Ny(e,t,a,n){re=e;var r=0;do{if(vs&&(gs=null),so=0,vs=!1,25<=r)throw Error(L(301));if(r+=1,Xe=Se=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}ee.H=KR,s=t(a,n)}while(vs);return s}function PR(){var e=ee.H,t=e.useState()[0];return t=typeof t.then=="function"?_o(t):t,e=e.useState()[0],(Se!==null?Se.memoizedState:null)!==e&&(re.flags|=1024),t}function gf(){var e=mu!==0;return mu=0,e}function yf(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function bf(e){if(du){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}du=!1}Qn=0,Xe=Se=re=null,vs=!1,so=mu=0,gs=null}function Ft(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return Xe===null?re.memoizedState=Xe=e:Xe=Xe.next=e,Xe}function Ze(){if(Se===null){var e=re.alternate;e=e!==null?e.memoizedState:null}else e=Se.next;var t=Xe===null?re.memoizedState:Xe.next;if(t!==null)Xe=t,Se=e;else{if(e===null)throw re.alternate===null?Error(L(467)):Error(L(310));Se=e,e={memoizedState:Se.memoizedState,baseState:Se.baseState,baseQueue:Se.baseQueue,queue:Se.queue,next:null},Xe===null?re.memoizedState=Xe=e:Xe=Xe.next=e}return Xe}function xf(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function _o(e){var t=so;return so+=1,gs===null&&(gs=[]),e=xy(gs,e,t),t=re,(Xe===null?t.memoizedState:Xe.next)===null&&(t=t.alternate,ee.H=t===null||t.memoizedState===null?Wy:eb),e}function ju(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return _o(e);if(e.$$typeof===Wa)return St(e)}throw Error(L(438,String(e)))}function $f(e){var t=null,a=re.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=re.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=xf(),re.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=wk;return t.index++,a}function ln(e,t){return typeof t=="function"?t(e):t}function Vl(e){var t=Ze();return wf(t,Se,e)}function wf(e,t,a){var n=e.queue;if(n===null)throw Error(L(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,u=null,c=t,d=!1;do{var f=c.lane&-536870913;if(f!==c.lane?(le&f)===f:(Qn&f)===f){var m=c.revertLane;if(m===0)u!==null&&(u=u.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),f===Ss&&(d=!0);else if((Qn&m)===m){c=c.next,m===Ss&&(d=!0);continue}else f={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=f,i=s):u=u.next=f,re.lanes|=m,Gn|=m;f=c.action,wr&&a(s,f),s=c.hasEagerState?c.eagerState:a(s,f)}else m={lane:f,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=m,i=s):u=u.next=m,re.lanes|=f,Gn|=f;c=c.next}while(c!==null&&c!==t);if(u===null?i=s:u.next=o,!ta(s,e.memoizedState)&&(mt=!0,d&&(a=hs,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=u,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function Ld(e){var t=Ze(),a=t.queue;if(a===null)throw Error(L(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);ta(s,t.memoizedState)||(mt=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function _y(e,t,a){var n=re,r=Ze(),s=ce;if(s){if(a===void 0)throw Error(L(407));a=a()}else a=t();var i=!ta((Se||r).memoizedState,a);i&&(r.memoizedState=a,mt=!0),r=r.queue;var o=Cy.bind(null,n,r,e);if(ko(2048,8,o,[e]),r.getSnapshot!==t||i||Xe!==null&&Xe.memoizedState.tag&1){if(n.flags|=2048,_s(9,Pu(),Ry.bind(null,n,r,a,t),null),Ce===null)throw Error(L(349));s||(Qn&124)!==0||ky(n,t,a)}return a}function ky(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=re.updateQueue,t===null?(t=xf(),re.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function Ry(e,t,a,n){t.value=a,t.getSnapshot=n,Ey(t)&&Ty(e)}function Cy(e,t,a){return a(function(){Ey(t)&&Ty(e)})}function Ey(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!ta(e,a)}catch{return!0}}function Ty(e){var t=Ms(e,2);t!==null&&ea(t,e,2)}function Sm(e){var t=Ft();if(typeof e=="function"){var a=e;if(e=a(),wr){On(!0);try{a()}finally{On(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:ln,lastRenderedState:e},t}function Ay(e,t,a,n){return e.baseState=a,wf(e,Se,typeof n=="function"?n:ln)}function FR(e,t,a,n,r){if(Fu(e))throw Error(L(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};ee.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,Dy(t,s)):(s.next=a.next,t.pending=a.next=s)}}function Dy(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=ee.T,i={};ee.T=i;try{var o=a(r,n),u=ee.S;u!==null&&u(i,o),zv(e,t,o)}catch(c){Nm(e,t,c)}finally{ee.T=s}}else try{s=a(r,n),zv(e,t,s)}catch(c){Nm(e,t,c)}}function zv(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){qv(e,t,n)},function(n){return Nm(e,t,n)}):qv(e,t,a)}function qv(e,t,a){t.status="fulfilled",t.value=a,My(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,Dy(e,a)))}function Nm(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,My(t),t=t.next;while(t!==n)}e.action=null}function My(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function Oy(e,t){return t}function Bv(e,t){if(ce){var a=Ce.formState;if(a!==null){e:{var n=re;if(ce){if(Be){t:{for(var r=Be,s=Oa;r.nodeType!==8;){if(!s){r=null;break t}if(r=Na(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){Be=Na(r.nextSibling),n=r.data==="F!";break e}}xr(n)}n=!1}n&&(t=a[0])}}return a=Ft(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:Oy,lastRenderedState:t},a.queue=n,a=Jy.bind(null,re,n),n.dispatch=a,n=Sm(!1),s=kf.bind(null,re,!1,n.queue),n=Ft(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=FR.bind(null,re,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function Hv(e){var t=Ze();return Ly(t,Se,e)}function Ly(e,t,a){if(t=wf(e,t,Oy)[0],e=Vl(ln)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=_o(t)}catch(i){throw i===No?Uu:i}else n=t;t=Ze();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(re.flags|=2048,_s(9,Pu(),zR.bind(null,r,a),null)),[n,s,e]}function zR(e,t){e.action=t}function Kv(e){var t=Ze(),a=Se;if(a!==null)return Ly(t,a,e);Ze(),t=t.memoizedState,a=Ze();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function _s(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=re.updateQueue,t===null&&(t=xf(),re.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function Pu(){return{destroy:void 0,resource:void 0}}function Uy(){return Ze().memoizedState}function Gl(e,t,a,n){var r=Ft();n=n===void 0?null:n,re.flags|=e,r.memoizedState=_s(1|t,Pu(),a,n)}function ko(e,t,a,n){var r=Ze();n=n===void 0?null:n;var s=r.memoizedState.inst;Se!==null&&n!==null&&hf(n,Se.memoizedState.deps)?r.memoizedState=_s(t,s,a,n):(re.flags|=e,r.memoizedState=_s(1|t,s,a,n))}function Iv(e,t){Gl(8390656,8,e,t)}function jy(e,t){ko(2048,8,e,t)}function Py(e,t){return ko(4,2,e,t)}function Fy(e,t){return ko(4,4,e,t)}function zy(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function qy(e,t,a){a=a!=null?a.concat([e]):null,ko(4,4,zy.bind(null,t,e),a)}function Sf(){}function By(e,t){var a=Ze();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&hf(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function Hy(e,t){var a=Ze();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&hf(t,n[1]))return n[0];if(n=e(),wr){On(!0);try{e()}finally{On(!1)}}return a.memoizedState=[n,t],n}function Nf(e,t,a){return a===void 0||(Qn&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=Mb(),re.lanes|=e,Gn|=e,a)}function Ky(e,t,a,n){return ta(a,t)?a:Ns.current!==null?(e=Nf(e,a,n),ta(e,t)||(mt=!0),e):(Qn&42)===0?(mt=!0,e.memoizedState=a):(e=Mb(),re.lanes|=e,Gn|=e,t)}function Iy(e,t,a,n,r){var s=de.p;de.p=s!==0&&8>s?s:8;var i=ee.T,o={};ee.T=o,kf(e,!1,t,a);try{var u=r(),c=ee.S;if(c!==null&&c(o,u),u!==null&&typeof u=="object"&&typeof u.then=="function"){var d=UR(u,n);Gi(e,t,d,Wt(e))}else Gi(e,t,n,Wt(e))}catch(f){Gi(e,t,{then:function(){},status:"rejected",reason:f},Wt())}finally{de.p=s,ee.T=i}}function qR(){}function _m(e,t,a,n){if(e.tag!==5)throw Error(L(476));var r=Qy(e).queue;Iy(e,r,t,fr,a===null?qR:function(){return Vy(e),a(n)})}function Qy(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:fr,baseState:fr,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:ln,lastRenderedState:fr},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:ln,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function Vy(e){var t=Qy(e).next.queue;Gi(e,t,{},Wt())}function _f(){return St(co)}function Gy(){return Ze().memoizedState}function Yy(){return Ze().memoizedState}function BR(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=Wt();e=Fn(a);var n=zn(t,e,a);n!==null&&(ea(n,t,a),Ii(n,t,a)),t={cache:df()},e.payload=t;return}t=t.return}}function HR(e,t,a){var n=Wt();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},Fu(e)?Xy(t,a):(a=of(e,t,a,n),a!==null&&(ea(a,e,n),Zy(a,t,n)))}function Jy(e,t,a){var n=Wt();Gi(e,t,a,n)}function Gi(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(Fu(e))Xy(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,ta(o,i))return Lu(e,t,r,0),Ce===null&&Ou(),!1}catch{}finally{}if(a=of(e,t,r,n),a!==null)return ea(a,e,n),Zy(a,t,n),!0}return!1}function kf(e,t,a,n){if(n={lane:2,revertLane:Of(),action:n,hasEagerState:!1,eagerState:null,next:null},Fu(e)){if(t)throw Error(L(479))}else t=of(e,a,n,2),t!==null&&ea(t,e,2)}function Fu(e){var t=e.alternate;return e===re||t!==null&&t===re}function Xy(e,t){vs=du=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function Zy(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,Bg(e,a)}}var fu={readContext:St,use:ju,useCallback:Qe,useContext:Qe,useEffect:Qe,useImperativeHandle:Qe,useLayoutEffect:Qe,useInsertionEffect:Qe,useMemo:Qe,useReducer:Qe,useRef:Qe,useState:Qe,useDebugValue:Qe,useDeferredValue:Qe,useTransition:Qe,useSyncExternalStore:Qe,useId:Qe,useHostTransitionStatus:Qe,useFormState:Qe,useActionState:Qe,useOptimistic:Qe,useMemoCache:Qe,useCacheRefresh:Qe},Wy={readContext:St,use:ju,useCallback:function(e,t){return Ft().memoizedState=[e,t===void 0?null:t],e},useContext:St,useEffect:Iv,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,Gl(4194308,4,zy.bind(null,t,e),a)},useLayoutEffect:function(e,t){return Gl(4194308,4,e,t)},useInsertionEffect:function(e,t){Gl(4,2,e,t)},useMemo:function(e,t){var a=Ft();t=t===void 0?null:t;var n=e();if(wr){On(!0);try{e()}finally{On(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=Ft();if(a!==void 0){var r=a(t);if(wr){On(!0);try{a(t)}finally{On(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=HR.bind(null,re,e),[n.memoizedState,e]},useRef:function(e){var t=Ft();return e={current:e},t.memoizedState=e},useState:function(e){e=Sm(e);var t=e.queue,a=Jy.bind(null,re,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:Sf,useDeferredValue:function(e,t){var a=Ft();return Nf(a,e,t)},useTransition:function(){var e=Sm(!1);return e=Iy.bind(null,re,e.queue,!0,!1),Ft().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=re,r=Ft();if(ce){if(a===void 0)throw Error(L(407));a=a()}else{if(a=t(),Ce===null)throw Error(L(349));(le&124)!==0||ky(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,Iv(Cy.bind(null,n,s,e),[e]),n.flags|=2048,_s(9,Pu(),Ry.bind(null,n,s,a,t),null),a},useId:function(){var e=Ft(),t=Ce.identifierPrefix;if(ce){var a=tn,n=en;a=(n&~(1<<32-Zt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=mu++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=jR++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:_f,useFormState:Bv,useActionState:Bv,useOptimistic:function(e){var t=Ft();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=kf.bind(null,re,!0,a),a.dispatch=t,[e,t]},useMemoCache:$f,useCacheRefresh:function(){return Ft().memoizedState=BR.bind(null,re)}},eb={readContext:St,use:ju,useCallback:By,useContext:St,useEffect:jy,useImperativeHandle:qy,useInsertionEffect:Py,useLayoutEffect:Fy,useMemo:Hy,useReducer:Vl,useRef:Uy,useState:function(){return Vl(ln)},useDebugValue:Sf,useDeferredValue:function(e,t){var a=Ze();return Ky(a,Se.memoizedState,e,t)},useTransition:function(){var e=Vl(ln)[0],t=Ze().memoizedState;return[typeof e=="boolean"?e:_o(e),t]},useSyncExternalStore:_y,useId:Gy,useHostTransitionStatus:_f,useFormState:Hv,useActionState:Hv,useOptimistic:function(e,t){var a=Ze();return Ay(a,Se,e,t)},useMemoCache:$f,useCacheRefresh:Yy},KR={readContext:St,use:ju,useCallback:By,useContext:St,useEffect:jy,useImperativeHandle:qy,useInsertionEffect:Py,useLayoutEffect:Fy,useMemo:Hy,useReducer:Ld,useRef:Uy,useState:function(){return Ld(ln)},useDebugValue:Sf,useDeferredValue:function(e,t){var a=Ze();return Se===null?Nf(a,e,t):Ky(a,Se.memoizedState,e,t)},useTransition:function(){var e=Ld(ln)[0],t=Ze().memoizedState;return[typeof e=="boolean"?e:_o(e),t]},useSyncExternalStore:_y,useId:Gy,useHostTransitionStatus:_f,useFormState:Kv,useActionState:Kv,useOptimistic:function(e,t){var a=Ze();return Se!==null?Ay(a,Se,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:$f,useCacheRefresh:Yy},ys=null,io=0;function Ml(e){var t=io;return io+=1,ys===null&&(ys=[]),xy(ys,e,t)}function Ai(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function Ol(e,t){throw t.$$typeof===xk?Error(L(525)):(e=Object.prototype.toString.call(t),Error(L(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function Qv(e){var t=e._init;return t(e._payload)}function tb(e){function t(g,v){if(e){var x=g.deletions;x===null?(g.deletions=[v],g.flags|=16):x.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=rn(g,v),g.index=0,g.sibling=null,g}function s(g,v,x){return g.index=x,e?(x=g.alternate,x!==null?(x=x.index,x<v?(g.flags|=67108866,v):x):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,x,$){return v===null||v.tag!==6?(v=Dd(x,g.mode,$),v.return=g,v):(v=r(v,x),v.return=g,v)}function u(g,v,x,$){var S=x.type;return S===es?d(g,v,x.props.children,$,x.key):v!==null&&(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Cn&&Qv(S)===v.type)?(v=r(v,x.props),Ai(v,x),v.return=g,v):(v=Il(x.type,x.key,x.props,null,g.mode,$),Ai(v,x),v.return=g,v)}function c(g,v,x,$){return v===null||v.tag!==4||v.stateNode.containerInfo!==x.containerInfo||v.stateNode.implementation!==x.implementation?(v=Md(x,g.mode,$),v.return=g,v):(v=r(v,x.children||[]),v.return=g,v)}function d(g,v,x,$,S){return v===null||v.tag!==7?(v=pr(x,g.mode,$,S),v.return=g,v):(v=r(v,x),v.return=g,v)}function f(g,v,x){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=Dd(""+v,g.mode,x),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case Nl:return x=Il(v.type,v.key,v.props,null,g.mode,x),Ai(x,v),x.return=g,x;case Ui:return v=Md(v,g.mode,x),v.return=g,v;case Cn:var $=v._init;return v=$(v._payload),f(g,v,x)}if(ji(v)||Ci(v))return v=pr(v,g.mode,x,null),v.return=g,v;if(typeof v.then=="function")return f(g,Ml(v),x);if(v.$$typeof===Wa)return f(g,Al(g,v),x);Ol(g,v)}return null}function m(g,v,x,$){var S=v!==null?v.key:null;if(typeof x=="string"&&x!==""||typeof x=="number"||typeof x=="bigint")return S!==null?null:o(g,v,""+x,$);if(typeof x=="object"&&x!==null){switch(x.$$typeof){case Nl:return x.key===S?u(g,v,x,$):null;case Ui:return x.key===S?c(g,v,x,$):null;case Cn:return S=x._init,x=S(x._payload),m(g,v,x,$)}if(ji(x)||Ci(x))return S!==null?null:d(g,v,x,$,null);if(typeof x.then=="function")return m(g,v,Ml(x),$);if(x.$$typeof===Wa)return m(g,v,Al(g,x),$);Ol(g,x)}return null}function p(g,v,x,$,S){if(typeof $=="string"&&$!==""||typeof $=="number"||typeof $=="bigint")return g=g.get(x)||null,o(v,g,""+$,S);if(typeof $=="object"&&$!==null){switch($.$$typeof){case Nl:return g=g.get($.key===null?x:$.key)||null,u(v,g,$,S);case Ui:return g=g.get($.key===null?x:$.key)||null,c(v,g,$,S);case Cn:var R=$._init;return $=R($._payload),p(g,v,x,$,S)}if(ji($)||Ci($))return g=g.get(x)||null,d(v,g,$,S,null);if(typeof $.then=="function")return p(g,v,x,Ml($),S);if($.$$typeof===Wa)return p(g,v,x,Al(v,$),S);Ol(v,$)}return null}function y(g,v,x,$){for(var S=null,R=null,_=v,C=v=0,U=null;_!==null&&C<x.length;C++){_.index>C?(U=_,_=null):U=_.sibling;var O=m(g,_,x[C],$);if(O===null){_===null&&(_=U);break}e&&_&&O.alternate===null&&t(g,_),v=s(O,v,C),R===null?S=O:R.sibling=O,R=O,_=U}if(C===x.length)return a(g,_),ce&&dr(g,C),S;if(_===null){for(;C<x.length;C++)_=f(g,x[C],$),_!==null&&(v=s(_,v,C),R===null?S=_:R.sibling=_,R=_);return ce&&dr(g,C),S}for(_=n(_);C<x.length;C++)U=p(_,g,C,x[C],$),U!==null&&(e&&U.alternate!==null&&_.delete(U.key===null?C:U.key),v=s(U,v,C),R===null?S=U:R.sibling=U,R=U);return e&&_.forEach(function(B){return t(g,B)}),ce&&dr(g,C),S}function b(g,v,x,$){if(x==null)throw Error(L(151));for(var S=null,R=null,_=v,C=v=0,U=null,O=x.next();_!==null&&!O.done;C++,O=x.next()){_.index>C?(U=_,_=null):U=_.sibling;var B=m(g,_,O.value,$);if(B===null){_===null&&(_=U);break}e&&_&&B.alternate===null&&t(g,_),v=s(B,v,C),R===null?S=B:R.sibling=B,R=B,_=U}if(O.done)return a(g,_),ce&&dr(g,C),S;if(_===null){for(;!O.done;C++,O=x.next())O=f(g,O.value,$),O!==null&&(v=s(O,v,C),R===null?S=O:R.sibling=O,R=O);return ce&&dr(g,C),S}for(_=n(_);!O.done;C++,O=x.next())O=p(_,g,C,O.value,$),O!==null&&(e&&O.alternate!==null&&_.delete(O.key===null?C:O.key),v=s(O,v,C),R===null?S=O:R.sibling=O,R=O);return e&&_.forEach(function(A){return t(g,A)}),ce&&dr(g,C),S}function w(g,v,x,$){if(typeof x=="object"&&x!==null&&x.type===es&&x.key===null&&(x=x.props.children),typeof x=="object"&&x!==null){switch(x.$$typeof){case Nl:e:{for(var S=x.key;v!==null;){if(v.key===S){if(S=x.type,S===es){if(v.tag===7){a(g,v.sibling),$=r(v,x.props.children),$.return=g,g=$;break e}}else if(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Cn&&Qv(S)===v.type){a(g,v.sibling),$=r(v,x.props),Ai($,x),$.return=g,g=$;break e}a(g,v);break}else t(g,v);v=v.sibling}x.type===es?($=pr(x.props.children,g.mode,$,x.key),$.return=g,g=$):($=Il(x.type,x.key,x.props,null,g.mode,$),Ai($,x),$.return=g,g=$)}return i(g);case Ui:e:{for(S=x.key;v!==null;){if(v.key===S)if(v.tag===4&&v.stateNode.containerInfo===x.containerInfo&&v.stateNode.implementation===x.implementation){a(g,v.sibling),$=r(v,x.children||[]),$.return=g,g=$;break e}else{a(g,v);break}else t(g,v);v=v.sibling}$=Md(x,g.mode,$),$.return=g,g=$}return i(g);case Cn:return S=x._init,x=S(x._payload),w(g,v,x,$)}if(ji(x))return y(g,v,x,$);if(Ci(x)){if(S=Ci(x),typeof S!="function")throw Error(L(150));return x=S.call(x),b(g,v,x,$)}if(typeof x.then=="function")return w(g,v,Ml(x),$);if(x.$$typeof===Wa)return w(g,v,Al(g,x),$);Ol(g,x)}return typeof x=="string"&&x!==""||typeof x=="number"||typeof x=="bigint"?(x=""+x,v!==null&&v.tag===6?(a(g,v.sibling),$=r(v,x),$.return=g,g=$):(a(g,v),$=Dd(x,g.mode,$),$.return=g,g=$),i(g)):a(g,v)}return function(g,v,x,$){try{io=0;var S=w(g,v,x,$);return ys=null,S}catch(_){if(_===No||_===Uu)throw _;var R=Jt(29,_,null,g.mode);return R.lanes=$,R.return=g,R}finally{}}}var ks=tb(!0),ab=tb(!1),va=Fa(null),Pa=null;function An(e){var t=e.alternate;Ue(rt,rt.current&1),Ue(va,e),Pa===null&&(t===null||Ns.current!==null||t.memoizedState!==null)&&(Pa=e)}function nb(e){if(e.tag===22){if(Ue(rt,rt.current),Ue(va,e),Pa===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(Pa=e)}}else Dn(e)}function Dn(){Ue(rt,rt.current),Ue(va,va.current)}function nn(e){ft(va),Pa===e&&(Pa=null),ft(rt)}var rt=Fa(0);function pu(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||Bm(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function Ud(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:De({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var km={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=Wt(),r=Fn(n);r.payload=t,a!=null&&(r.callback=a),t=zn(e,r,n),t!==null&&(ea(t,e,n),Ii(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=Wt(),r=Fn(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=zn(e,r,n),t!==null&&(ea(t,e,n),Ii(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=Wt(),n=Fn(a);n.tag=2,t!=null&&(n.callback=t),t=zn(e,n,a),t!==null&&(ea(t,e,a),Ii(t,e,a))}};function Vv(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!no(a,n)||!no(r,s):!0}function Gv(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&km.enqueueReplaceState(t,t.state,null)}function Sr(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=De({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var hu=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function rb(e){hu(e)}function sb(e){console.error(e)}function ib(e){hu(e)}function vu(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function Yv(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function Rm(e,t,a){return a=Fn(a),a.tag=3,a.payload={element:null},a.callback=function(){vu(e,t)},a}function ob(e){return e=Fn(e),e.tag=3,e}function lb(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){Yv(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){Yv(t,a,n),typeof r!="function"&&(qn===null?qn=new Set([this]):qn.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function IR(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&wo(t,a,r,!0),a=va.current,a!==null){switch(a.tag){case 13:return Pa===null?Lm():a.alternate===null&&He===0&&(He=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===bm?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),Vd(e,n,r)),!1;case 22:return a.flags|=65536,n===bm?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),Vd(e,n,r)),!1}throw Error(L(435,a.tag))}return Vd(e,n,r),Lm(),!1}if(ce)return t=va.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==pm&&(e=Error(L(422),{cause:n}),ro(pa(e,a)))):(n!==pm&&(t=Error(L(423),{cause:n}),ro(pa(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=pa(n,a),r=Rm(e.stateNode,n,r),Od(e,r),He!==4&&(He=2)),!1;var s=Error(L(520),{cause:n});if(s=pa(s,a),Xi===null?Xi=[s]:Xi.push(s),He!==4&&(He=2),t===null)return!0;n=pa(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=Rm(a.stateNode,n,e),Od(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(qn===null||!qn.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=ob(r),lb(r,e,a,n),Od(a,r),!1}a=a.return}while(a!==null);return!1}var ub=Error(L(461)),mt=!1;function ht(e,t,a,n){t.child=e===null?ab(t,null,a,n):ks(t,e.child,a,n)}function Jv(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return $r(t),n=vf(e,t,a,i,s,r),o=gf(),e!==null&&!mt?(yf(e,t,r),un(e,t,r)):(ce&&o&&uf(t),t.flags|=1,ht(e,t,n,r),t.child)}function Xv(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!lf(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,cb(e,t,s,n,r)):(e=Il(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!Rf(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:no,a(i,n)&&e.ref===t.ref)return un(e,t,r)}return t.flags|=1,e=rn(s,n),e.ref=t.ref,e.return=t,t.child=e}function cb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if(no(s,n)&&e.ref===t.ref)if(mt=!1,t.pendingProps=n=s,Rf(e,r))(e.flags&131072)!==0&&(mt=!0);else return t.lanes=e.lanes,un(e,t,r)}return Cm(e,t,a,n,r)}function db(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return Zv(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&Ql(t,s!==null?s.cachePool:null),s!==null?Fv(t,s):wm(),nb(t);else return t.lanes=t.childLanes=536870912,Zv(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(Ql(t,s.cachePool),Fv(t,s),Dn(t),t.memoizedState=null):(e!==null&&Ql(t,null),wm(),Dn(t));return ht(e,t,r,a),t.child}function Zv(e,t,a,n){var r=mf();return r=r===null?null:{parent:nt._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&Ql(t,null),wm(),nb(t),e!==null&&wo(e,t,n,!0),null}function Yl(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(L(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function Cm(e,t,a,n,r){return $r(t),a=vf(e,t,a,n,void 0,r),n=gf(),e!==null&&!mt?(yf(e,t,r),un(e,t,r)):(ce&&n&&uf(t),t.flags|=1,ht(e,t,a,r),t.child)}function Wv(e,t,a,n,r,s){return $r(t),t.updateQueue=null,a=Ny(t,n,a,r),Sy(e),n=gf(),e!==null&&!mt?(yf(e,t,s),un(e,t,s)):(ce&&n&&uf(t),t.flags|=1,ht(e,t,a,s),t.child)}function eg(e,t,a,n,r){if($r(t),t.stateNode===null){var s=ls,i=a.contextType;typeof i=="object"&&i!==null&&(s=St(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=km,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},ff(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?St(i):ls,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(Ud(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&km.enqueueReplaceState(s,s.state,null),Vi(t,n,s,r),Qi(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,u=Sr(a,o);s.props=u;var c=s.context,d=a.contextType;i=ls,typeof d=="object"&&d!==null&&(i=St(d));var f=a.getDerivedStateFromProps;d=typeof f=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&Gv(t,s,n,i),En=!1;var m=t.memoizedState;s.state=m,Vi(t,n,s,r),Qi(),c=t.memoizedState,o||m!==c||En?(typeof f=="function"&&(Ud(t,a,f,n),c=t.memoizedState),(u=En||Vv(t,a,u,n,m,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=u):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,xm(e,t),i=t.memoizedProps,d=Sr(a,i),s.props=d,f=t.pendingProps,m=s.context,c=a.contextType,u=ls,typeof c=="object"&&c!==null&&(u=St(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==f||m!==u)&&Gv(t,s,n,u),En=!1,m=t.memoizedState,s.state=m,Vi(t,n,s,r),Qi();var p=t.memoizedState;i!==f||m!==p||En||e!==null&&e.dependencies!==null&&uu(e.dependencies)?(typeof o=="function"&&(Ud(t,a,o,n),p=t.memoizedState),(d=En||Vv(t,a,d,n,m,p,u)||e!==null&&e.dependencies!==null&&uu(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,p,u),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,p,u)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=p),s.props=n,s.state=p,s.context=u,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,Yl(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=ks(t,e.child,null,r),t.child=ks(t,null,a,r)):ht(e,t,a,r),t.memoizedState=s.state,e=t.child):e=un(e,t,r),e}function tg(e,t,a,n){return $o(),t.flags|=256,ht(e,t,a,n),t.child}var jd={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function Pd(e){return{baseLanes:e,cachePool:yy()}}function Fd(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=ha),e}function mb(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(rt.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(ce){if(r?An(t):Dn(t),ce){var o=Be,u;if(u=o){e:{for(u=o,o=Oa;u.nodeType!==8;){if(!o){o=null;break e}if(u=Na(u.nextSibling),u===null){o=null;break e}}o=u}o!==null?(t.memoizedState={dehydrated:o,treeContext:hr!==null?{id:en,overflow:tn}:null,retryLane:536870912,hydrationErrors:null},u=Jt(18,null,null,0),u.stateNode=o,u.return=t,t.child=u,Tt=t,Be=null,u=!0):u=!1}u||xr(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return Bm(o)?t.lanes=32:t.lanes=536870912,null;nn(t)}return o=n.children,n=n.fallback,r?(Dn(t),r=t.mode,o=gu({mode:"hidden",children:o},r),n=pr(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=Pd(a),r.childLanes=Fd(e,i,a),t.memoizedState=jd,n):(An(t),Em(t,o))}if(u=e.memoizedState,u!==null&&(o=u.dehydrated,o!==null)){if(s)t.flags&256?(An(t),t.flags&=-257,t=zd(e,t,a)):t.memoizedState!==null?(Dn(t),t.child=e.child,t.flags|=128,t=null):(Dn(t),r=n.fallback,o=t.mode,n=gu({mode:"visible",children:n.children},o),r=pr(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,ks(t,e.child,null,a),n=t.child,n.memoizedState=Pd(a),n.childLanes=Fd(e,i,a),t.memoizedState=jd,t=r);else if(An(t),Bm(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(L(419)),n.stack="",n.digest=i,ro({value:n,source:null,stack:null}),t=zd(e,t,a)}else if(mt||wo(e,t,a,!1),i=(a&e.childLanes)!==0,mt||i){if(i=Ce,i!==null&&(n=a&-a,n=(n&42)!==0?1:Jm(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==u.retryLane))throw u.retryLane=n,Ms(e,n),ea(i,e,n),ub;o.data==="$?"||Lm(),t=zd(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=u.treeContext,Be=Na(o.nextSibling),Tt=t,ce=!0,vr=null,Oa=!1,e!==null&&(da[ma++]=en,da[ma++]=tn,da[ma++]=hr,en=e.id,tn=e.overflow,hr=t),t=Em(t,n.children),t.flags|=4096);return t}return r?(Dn(t),r=n.fallback,o=t.mode,u=e.child,c=u.sibling,n=rn(u,{mode:"hidden",children:n.children}),n.subtreeFlags=u.subtreeFlags&65011712,c!==null?r=rn(c,r):(r=pr(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=Pd(a):(u=o.cachePool,u!==null?(c=nt._currentValue,u=u.parent!==c?{parent:c,pool:c}:u):u=yy(),o={baseLanes:o.baseLanes|a,cachePool:u}),r.memoizedState=o,r.childLanes=Fd(e,i,a),t.memoizedState=jd,n):(An(t),a=e.child,e=a.sibling,a=rn(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function Em(e,t){return t=gu({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function gu(e,t){return e=Jt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function zd(e,t,a){return ks(t,e.child,null,a),e=Em(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function ag(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),vm(e.return,t,a)}function qd(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function fb(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(ht(e,t,n.children,a),n=rt.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&ag(e,a,t);else if(e.tag===19)ag(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Ue(rt,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&pu(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),qd(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&pu(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}qd(t,!0,a,null,s);break;case"together":qd(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function un(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),Gn|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(wo(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(L(153));if(t.child!==null){for(e=t.child,a=rn(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=rn(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function Rf(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&uu(e)))}function QR(e,t,a){switch(t.tag){case 3:tu(t,t.stateNode.containerInfo),Tn(t,nt,e.memoizedState.cache),$o();break;case 27:case 5:rm(t);break;case 4:tu(t,t.stateNode.containerInfo);break;case 10:Tn(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(An(t),t.flags|=128,null):(a&t.child.childLanes)!==0?mb(e,t,a):(An(t),e=un(e,t,a),e!==null?e.sibling:null);An(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(wo(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return fb(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Ue(rt,rt.current),n)break;return null;case 22:case 23:return t.lanes=0,db(e,t,a);case 24:Tn(t,nt,e.memoizedState.cache)}return un(e,t,a)}function pb(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)mt=!0;else{if(!Rf(e,a)&&(t.flags&128)===0)return mt=!1,QR(e,t,a);mt=(e.flags&131072)!==0}else mt=!1,ce&&(t.flags&1048576)!==0&&vy(t,lu,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")lf(n)?(e=Sr(n,e),t.tag=1,t=eg(null,t,n,e,a)):(t.tag=0,t=Cm(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===Vm){t.tag=11,t=Jv(null,t,n,e,a);break e}else if(r===Gm){t.tag=14,t=Xv(null,t,n,e,a);break e}}throw t=am(n)||n,Error(L(306,t,""))}}return t;case 0:return Cm(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Sr(n,t.pendingProps),eg(e,t,n,r,a);case 3:e:{if(tu(t,t.stateNode.containerInfo),e===null)throw Error(L(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,xm(e,t),Vi(t,n,null,a);var i=t.memoizedState;if(n=i.cache,Tn(t,nt,n),n!==s.cache&&gm(t,[nt],a,!0),Qi(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=tg(e,t,n,a);break e}else if(n!==r){r=pa(Error(L(424)),t),ro(r),t=tg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(Be=Na(e.firstChild),Tt=t,ce=!0,vr=null,Oa=!0,a=ab(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if($o(),n===r){t=un(e,t,a);break e}ht(e,t,n,a)}t=t.child}return t;case 26:return Yl(e,t),e===null?(a=$g(t.type,null,t.pendingProps,null))?t.memoizedState=a:ce||(a=t.type,e=t.pendingProps,n=Nu(Pn.current).createElement(a),n[wt]=t,n[Bt]=e,gt(n,a,e),dt(n),t.stateNode=n):t.memoizedState=$g(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return rm(t),e===null&&ce&&(n=t.stateNode=e0(t.type,t.pendingProps,Pn.current),Tt=t,Oa=!0,r=Be,Jn(t.type)?(Hm=r,Be=Na(n.firstChild)):Be=r),ht(e,t,t.pendingProps.children,a),Yl(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&ce&&((r=n=Be)&&(n=yC(n,t.type,t.pendingProps,Oa),n!==null?(t.stateNode=n,Tt=t,Be=Na(n.firstChild),Oa=!1,r=!0):r=!1),r||xr(t)),rm(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,zm(r,s)?n=null:i!==null&&zm(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=vf(e,t,PR,null,null,a),co._currentValue=r),Yl(e,t),ht(e,t,n,a),t.child;case 6:return e===null&&ce&&((e=a=Be)&&(a=bC(a,t.pendingProps,Oa),a!==null?(t.stateNode=a,Tt=t,Be=null,e=!0):e=!1),e||xr(t)),null;case 13:return mb(e,t,a);case 4:return tu(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=ks(t,null,n,a):ht(e,t,n,a),t.child;case 11:return Jv(e,t,t.type,t.pendingProps,a);case 7:return ht(e,t,t.pendingProps,a),t.child;case 8:return ht(e,t,t.pendingProps.children,a),t.child;case 12:return ht(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,Tn(t,t.type,n.value),ht(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,$r(t),r=St(r),n=n(r),t.flags|=1,ht(e,t,n,a),t.child;case 14:return Xv(e,t,t.type,t.pendingProps,a);case 15:return cb(e,t,t.type,t.pendingProps,a);case 19:return fb(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=gu(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=rn(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return db(e,t,a);case 24:return $r(t),n=St(nt),e===null?(r=mf(),r===null&&(r=Ce,s=df(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},ff(t),Tn(t,nt,r)):((e.lanes&a)!==0&&(xm(e,t),Vi(t,null,null,a),Qi()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),Tn(t,nt,n)):(n=s.cache,Tn(t,nt,n),n!==r.cache&&gm(t,[nt],a,!0))),ht(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(L(156,t.tag))}function Ja(e){e.flags|=4}function ng(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!n0(t)){if(t=va.current,t!==null&&((le&4194048)===le?Pa!==null:(le&62914560)!==le&&(le&536870912)===0||t!==Pa))throw Ki=bm,by;e.flags|=8192}}function Ll(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?zg():536870912,e.lanes|=t,Rs|=t)}function Di(e,t){if(!ce)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function Fe(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function VR(e,t,a){var n=t.pendingProps;switch(cf(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return Fe(t),null;case 1:return Fe(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),sn(nt),xs(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Ti(t)?Ja(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,Mv())),Fe(t),null;case 26:return a=t.memoizedState,e===null?(Ja(t),a!==null?(Fe(t),ng(t,a)):(Fe(t),t.flags&=-16777217)):a?a!==e.memoizedState?(Ja(t),Fe(t),ng(t,a)):(Fe(t),t.flags&=-16777217):(e.memoizedProps!==n&&Ja(t),Fe(t),t.flags&=-16777217),null;case 27:au(t),a=Pn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&Ja(t);else{if(!n){if(t.stateNode===null)throw Error(L(166));return Fe(t),null}e=Ua.current,Ti(t)?Av(t,e):(e=e0(r,n,a),t.stateNode=e,Ja(t))}return Fe(t),null;case 5:if(au(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&Ja(t);else{if(!n){if(t.stateNode===null)throw Error(L(166));return Fe(t),null}if(e=Ua.current,Ti(t))Av(t,e);else{switch(r=Nu(Pn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[wt]=t,e[Bt]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(gt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&Ja(t)}}return Fe(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&Ja(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(L(166));if(e=Pn.current,Ti(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=Tt,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[wt]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||Xb(e.nodeValue,a)),e||xr(t)}else e=Nu(e).createTextNode(n),e[wt]=t,t.stateNode=e}return Fe(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Ti(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(L(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(L(317));r[wt]=t}else $o(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;Fe(t),r=!1}else r=Mv(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(nn(t),t):(nn(t),null)}if(nn(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),Ll(t,t.updateQueue),Fe(t),null;case 4:return xs(),e===null&&Lf(t.stateNode.containerInfo),Fe(t),null;case 10:return sn(t.type),Fe(t),null;case 19:if(ft(rt),r=t.memoizedState,r===null)return Fe(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)Di(r,!1);else{if(He!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=pu(e),s!==null){for(t.flags|=128,Di(r,!1),e=s.updateQueue,t.updateQueue=e,Ll(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)hy(a,e),a=a.sibling;return Ue(rt,rt.current&1|2),t.child}e=e.sibling}r.tail!==null&&ja()>bu&&(t.flags|=128,n=!0,Di(r,!1),t.lanes=4194304)}else{if(!n)if(e=pu(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,Ll(t,e),Di(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!ce)return Fe(t),null}else 2*ja()-r.renderingStartTime>bu&&a!==536870912&&(t.flags|=128,n=!0,Di(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=ja(),t.sibling=null,e=rt.current,Ue(rt,n?e&1|2:e&1),t):(Fe(t),null);case 22:case 23:return nn(t),pf(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(Fe(t),t.subtreeFlags&6&&(t.flags|=8192)):Fe(t),a=t.updateQueue,a!==null&&Ll(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&ft(gr),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),sn(nt),Fe(t),null;case 25:return null;case 30:return null}throw Error(L(156,t.tag))}function GR(e,t){switch(cf(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return sn(nt),xs(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return au(t),null;case 13:if(nn(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(L(340));$o()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return ft(rt),null;case 4:return xs(),null;case 10:return sn(t.type),null;case 22:case 23:return nn(t),pf(),e!==null&&ft(gr),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return sn(nt),null;case 25:return null;default:return null}}function hb(e,t){switch(cf(t),t.tag){case 3:sn(nt),xs();break;case 26:case 27:case 5:au(t);break;case 4:xs();break;case 13:nn(t);break;case 19:ft(rt);break;case 10:sn(t.type);break;case 22:case 23:nn(t),pf(),e!==null&&ft(gr);break;case 24:sn(nt)}}function Ro(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){_e(t,t.return,o)}}function Vn(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var u=a,c=o;try{c()}catch(d){_e(r,u,d)}}}n=n.next}while(n!==s)}}catch(d){_e(t,t.return,d)}}function vb(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{wy(t,a)}catch(n){_e(e,e.return,n)}}}function gb(e,t,a){a.props=Sr(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){_e(e,t,n)}}function Yi(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){_e(e,t,r)}}function La(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){_e(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){_e(e,t,r)}else a.current=null}function yb(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){_e(e,e.return,r)}}function Bd(e,t,a){try{var n=e.stateNode;fC(n,e.type,a,t),n[Bt]=t}catch(r){_e(e,e.return,r)}}function bb(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&Jn(e.type)||e.tag===4}function Hd(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||bb(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&Jn(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function Tm(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=Hu));else if(n!==4&&(n===27&&Jn(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(Tm(e,t,a),e=e.sibling;e!==null;)Tm(e,t,a),e=e.sibling}function yu(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&Jn(e.type)&&(a=e.stateNode),e=e.child,e!==null))for(yu(e,t,a),e=e.sibling;e!==null;)yu(e,t,a),e=e.sibling}function xb(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);gt(t,n,a),t[wt]=e,t[Bt]=a}catch(s){_e(e,e.return,s)}}var Za=!1,Ve=!1,Kd=!1,rg=typeof WeakSet=="function"?WeakSet:Set,ct=null;function YR(e,t){if(e=e.containerInfo,Pm=Cu,e=oy(e),rf(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,u=-1,c=0,d=0,f=e,m=null;t:for(;;){for(var p;f!==a||r!==0&&f.nodeType!==3||(o=i+r),f!==s||n!==0&&f.nodeType!==3||(u=i+n),f.nodeType===3&&(i+=f.nodeValue.length),(p=f.firstChild)!==null;)m=f,f=p;for(;;){if(f===e)break t;if(m===a&&++c===r&&(o=i),m===s&&++d===n&&(u=i),(p=f.nextSibling)!==null)break;f=m,m=f.parentNode}f=p}a=o===-1||u===-1?null:{start:o,end:u}}else a=null}a=a||{start:0,end:0}}else a=null;for(Fm={focusedElem:e,selectionRange:a},Cu=!1,ct=t;ct!==null;)if(t=ct,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,ct=e;else for(;ct!==null;){switch(t=ct,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var y=Sr(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(y,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(b){_e(a,a.return,b)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)qm(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":qm(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(L(163))}if(e=t.sibling,e!==null){e.return=t.return,ct=e;break}ct=t.return}}function $b(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:kn(e,a),n&4&&Ro(5,a);break;case 1:if(kn(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){_e(a,a.return,i)}else{var r=Sr(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){_e(a,a.return,i)}}n&64&&vb(a),n&512&&Yi(a,a.return);break;case 3:if(kn(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{wy(e,t)}catch(i){_e(a,a.return,i)}}break;case 27:t===null&&n&4&&xb(a);case 26:case 5:kn(e,a),t===null&&n&4&&yb(a),n&512&&Yi(a,a.return);break;case 12:kn(e,a);break;case 13:kn(e,a),n&4&&Nb(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=rC.bind(null,a),xC(e,a))));break;case 22:if(n=a.memoizedState!==null||Za,!n){t=t!==null&&t.memoizedState!==null||Ve,r=Za;var s=Ve;Za=n,(Ve=t)&&!s?Rn(e,a,(a.subtreeFlags&8772)!==0):kn(e,a),Za=r,Ve=s}break;case 30:break;default:kn(e,a)}}function wb(e){var t=e.alternate;t!==null&&(e.alternate=null,wb(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&Zm(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var Le=null,zt=!1;function Xa(e,t,a){for(a=a.child;a!==null;)Sb(e,t,a),a=a.sibling}function Sb(e,t,a){if(Xt&&typeof Xt.onCommitFiberUnmount=="function")try{Xt.onCommitFiberUnmount(vo,a)}catch{}switch(a.tag){case 26:Ve||La(a,t),Xa(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:Ve||La(a,t);var n=Le,r=zt;Jn(a.type)&&(Le=a.stateNode,zt=!1),Xa(e,t,a),Wi(a.stateNode),Le=n,zt=r;break;case 5:Ve||La(a,t);case 6:if(n=Le,r=zt,Le=null,Xa(e,t,a),Le=n,zt=r,Le!==null)if(zt)try{(Le.nodeType===9?Le.body:Le.nodeName==="HTML"?Le.ownerDocument.body:Le).removeChild(a.stateNode)}catch(s){_e(a,t,s)}else try{Le.removeChild(a.stateNode)}catch(s){_e(a,t,s)}break;case 18:Le!==null&&(zt?(e=Le,yg(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),po(e)):yg(Le,a.stateNode));break;case 4:n=Le,r=zt,Le=a.stateNode.containerInfo,zt=!0,Xa(e,t,a),Le=n,zt=r;break;case 0:case 11:case 14:case 15:Ve||Vn(2,a,t),Ve||Vn(4,a,t),Xa(e,t,a);break;case 1:Ve||(La(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&gb(a,t,n)),Xa(e,t,a);break;case 21:Xa(e,t,a);break;case 22:Ve=(n=Ve)||a.memoizedState!==null,Xa(e,t,a),Ve=n;break;default:Xa(e,t,a)}}function Nb(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{po(e)}catch(a){_e(t,t.return,a)}}function JR(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new rg),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new rg),t;default:throw Error(L(435,e.tag))}}function Id(e,t){var a=JR(e);t.forEach(function(n){var r=sC.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function Vt(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(Jn(o.type)){Le=o.stateNode,zt=!1;break e}break;case 5:Le=o.stateNode,zt=!1;break e;case 3:case 4:Le=o.stateNode.containerInfo,zt=!0;break e}o=o.return}if(Le===null)throw Error(L(160));Sb(s,i,r),Le=null,zt=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)_b(t,e),t=t.sibling}var Sa=null;function _b(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:Vt(t,e),Gt(e),n&4&&(Vn(3,e,e.return),Ro(3,e),Vn(5,e,e.return));break;case 1:Vt(t,e),Gt(e),n&512&&(Ve||a===null||La(a,a.return)),n&64&&Za&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=Sa;if(Vt(t,e),Gt(e),n&512&&(Ve||a===null||La(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[bo]||s[wt]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),gt(s,n,a),s[wt]=e,dt(s),n=s;break e;case"link":var i=Sg("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),gt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=Sg("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),gt(s,n,a),r.head.appendChild(s);break;default:throw Error(L(468,n))}s[wt]=e,dt(s),n=s}e.stateNode=n}else Ng(r,e.type,e.stateNode);else e.stateNode=wg(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?Ng(r,e.type,e.stateNode):wg(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&Bd(e,e.memoizedProps,a.memoizedProps)}break;case 27:Vt(t,e),Gt(e),n&512&&(Ve||a===null||La(a,a.return)),a!==null&&n&4&&Bd(e,e.memoizedProps,a.memoizedProps);break;case 5:if(Vt(t,e),Gt(e),n&512&&(Ve||a===null||La(a,a.return)),e.flags&32){r=e.stateNode;try{ws(r,"")}catch(p){_e(e,e.return,p)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,Bd(e,r,a!==null?a.memoizedProps:r)),n&1024&&(Kd=!0);break;case 6:if(Vt(t,e),Gt(e),n&4){if(e.stateNode===null)throw Error(L(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(p){_e(e,e.return,p)}}break;case 3:if(Zl=null,r=Sa,Sa=_u(t.containerInfo),Vt(t,e),Sa=r,Gt(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{po(t.containerInfo)}catch(p){_e(e,e.return,p)}Kd&&(Kd=!1,kb(e));break;case 4:n=Sa,Sa=_u(e.stateNode.containerInfo),Vt(t,e),Gt(e),Sa=n;break;case 12:Vt(t,e),Gt(e);break;case 13:Vt(t,e),Gt(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(Df=ja()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,Id(e,n)));break;case 22:r=e.memoizedState!==null;var u=a!==null&&a.memoizedState!==null,c=Za,d=Ve;if(Za=c||r,Ve=d||u,Vt(t,e),Ve=d,Za=c,Gt(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||u||Za||Ve||mr(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){u=a=t;try{if(s=u.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=u.stateNode;var f=u.memoizedProps.style,m=f!=null&&f.hasOwnProperty("display")?f.display:null;o.style.display=m==null||typeof m=="boolean"?"":(""+m).trim()}}catch(p){_e(u,u.return,p)}}}else if(t.tag===6){if(a===null){u=t;try{u.stateNode.nodeValue=r?"":u.memoizedProps}catch(p){_e(u,u.return,p)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,Id(e,a))));break;case 19:Vt(t,e),Gt(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,Id(e,n)));break;case 30:break;case 21:break;default:Vt(t,e),Gt(e)}}function Gt(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(bb(n)){a=n;break}n=n.return}if(a==null)throw Error(L(160));switch(a.tag){case 27:var r=a.stateNode,s=Hd(e);yu(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(ws(i,""),a.flags&=-33);var o=Hd(e);yu(e,o,i);break;case 3:case 4:var u=a.stateNode.containerInfo,c=Hd(e);Tm(e,c,u);break;default:throw Error(L(161))}}catch(d){_e(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function kb(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;kb(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function kn(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)$b(e,t.alternate,t),t=t.sibling}function mr(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:Vn(4,t,t.return),mr(t);break;case 1:La(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&gb(t,t.return,a),mr(t);break;case 27:Wi(t.stateNode);case 26:case 5:La(t,t.return),mr(t);break;case 22:t.memoizedState===null&&mr(t);break;case 30:mr(t);break;default:mr(t)}e=e.sibling}}function Rn(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:Rn(r,s,a),Ro(4,s);break;case 1:if(Rn(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){_e(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var u=r.shared.hiddenCallbacks;if(u!==null)for(r.shared.hiddenCallbacks=null,r=0;r<u.length;r++)$y(u[r],o)}catch(c){_e(n,n.return,c)}}a&&i&64&&vb(s),Yi(s,s.return);break;case 27:xb(s);case 26:case 5:Rn(r,s,a),a&&n===null&&i&4&&yb(s),Yi(s,s.return);break;case 12:Rn(r,s,a);break;case 13:Rn(r,s,a),a&&i&4&&Nb(r,s);break;case 22:s.memoizedState===null&&Rn(r,s,a),Yi(s,s.return);break;case 30:break;default:Rn(r,s,a)}t=t.sibling}}function Cf(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&So(a))}function Ef(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&So(e))}function Ma(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)Rb(e,t,a,n),t=t.sibling}function Rb(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:Ma(e,t,a,n),r&2048&&Ro(9,t);break;case 1:Ma(e,t,a,n);break;case 3:Ma(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&So(e)));break;case 12:if(r&2048){Ma(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(u){_e(t,t.return,u)}}else Ma(e,t,a,n);break;case 13:Ma(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?Ma(e,t,a,n):Ji(e,t):s._visibility&2?Ma(e,t,a,n):(s._visibility|=2,Zr(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&Cf(i,t);break;case 24:Ma(e,t,a,n),r&2048&&Ef(t.alternate,t);break;default:Ma(e,t,a,n)}}function Zr(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,u=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:Zr(s,i,o,u,r),Ro(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?Zr(s,i,o,u,r):Ji(s,i):(d._visibility|=2,Zr(s,i,o,u,r)),r&&c&2048&&Cf(i.alternate,i);break;case 24:Zr(s,i,o,u,r),r&&c&2048&&Ef(i.alternate,i);break;default:Zr(s,i,o,u,r)}t=t.sibling}}function Ji(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:Ji(a,n),r&2048&&Cf(n.alternate,n);break;case 24:Ji(a,n),r&2048&&Ef(n.alternate,n);break;default:Ji(a,n)}t=t.sibling}}var Fi=8192;function Yr(e){if(e.subtreeFlags&Fi)for(e=e.child;e!==null;)Cb(e),e=e.sibling}function Cb(e){switch(e.tag){case 26:Yr(e),e.flags&Fi&&e.memoizedState!==null&&MC(Sa,e.memoizedState,e.memoizedProps);break;case 5:Yr(e);break;case 3:case 4:var t=Sa;Sa=_u(e.stateNode.containerInfo),Yr(e),Sa=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=Fi,Fi=16777216,Yr(e),Fi=t):Yr(e));break;default:Yr(e)}}function Eb(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function Mi(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ct=n,Ab(n,e)}Eb(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)Tb(e),e=e.sibling}function Tb(e){switch(e.tag){case 0:case 11:case 15:Mi(e),e.flags&2048&&Vn(9,e,e.return);break;case 3:Mi(e);break;case 12:Mi(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,Jl(e)):Mi(e);break;default:Mi(e)}}function Jl(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ct=n,Ab(n,e)}Eb(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:Vn(8,t,t.return),Jl(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,Jl(t));break;default:Jl(t)}e=e.sibling}}function Ab(e,t){for(;ct!==null;){var a=ct;switch(a.tag){case 0:case 11:case 15:Vn(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:So(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,ct=n;else e:for(a=e;ct!==null;){n=ct;var r=n.sibling,s=n.return;if(wb(n),n===a){ct=null;break e}if(r!==null){r.return=s,ct=r;break e}ct=s}}}var XR={getCacheForType:function(e){var t=St(nt),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},ZR=typeof WeakMap=="function"?WeakMap:Map,$e=0,Ce=null,ie=null,le=0,xe=0,Yt=null,Un=!1,Os=!1,Tf=!1,cn=0,He=0,Gn=0,yr=0,Af=0,ha=0,Rs=0,Xi=null,qt=null,Am=!1,Df=0,bu=1/0,xu=null,qn=null,vt=0,Bn=null,Cs=null,bs=0,Dm=0,Mm=null,Db=null,Zi=0,Om=null;function Wt(){if(($e&2)!==0&&le!==0)return le&-le;if(ee.T!==null){var e=Ss;return e!==0?e:Of()}return Hg()}function Mb(){ha===0&&(ha=(le&536870912)===0||ce?Fg():536870912);var e=va.current;return e!==null&&(e.flags|=32),ha}function ea(e,t,a){(e===Ce&&(xe===2||xe===9)||e.cancelPendingCommit!==null)&&(Es(e,0),jn(e,le,ha,!1)),yo(e,a),(($e&2)===0||e!==Ce)&&(e===Ce&&(($e&2)===0&&(yr|=a),He===4&&jn(e,le,ha,!1)),za(e))}function Ob(e,t,a){if(($e&6)!==0)throw Error(L(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||go(e,t),r=n?tC(e,t):Qd(e,t,!0),s=n;do{if(r===0){Os&&!n&&jn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!WR(a)){r=Qd(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=Xi;var u=o.current.memoizedState.isDehydrated;if(u&&(Es(o,i).flags|=256),i=Qd(o,i,!1),i!==2){if(Tf&&!u){o.errorRecoveryDisabledLanes|=s,yr|=s,r=4;break e}s=qt,qt=r,s!==null&&(qt===null?qt=s:qt.push.apply(qt,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){Es(e,0),jn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(L(345));case 4:if((t&4194048)!==t)break;case 6:jn(n,t,ha,!Un);break e;case 2:qt=null;break;case 3:case 5:break;default:throw Error(L(329))}if((t&62914560)===t&&(r=Df+300-ja(),10<r)){if(jn(n,t,ha,!Un),Tu(n,0,!0)!==0)break e;n.timeoutHandle=Wb(sg.bind(null,n,a,qt,xu,Am,t,ha,yr,Rs,Un,s,2,-0,0),r);break e}sg(n,a,qt,xu,Am,t,ha,yr,Rs,Un,s,0,-0,0)}}break}while(!0);za(e)}function sg(e,t,a,n,r,s,i,o,u,c,d,f,m,p){if(e.timeoutHandle=-1,f=t.subtreeFlags,(f&8192||(f&16785408)===16785408)&&(uo={stylesheets:null,count:0,unsuspend:DC},Cb(t),f=OC(),f!==null)){e.cancelPendingCommit=f(og.bind(null,e,t,s,a,n,r,i,o,u,d,1,m,p)),jn(e,s,i,!c);return}og(e,t,s,a,n,r,i,o,u)}function WR(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!ta(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function jn(e,t,a,n){t&=~Af,t&=~yr,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Zt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&qg(e,a,t)}function zu(){return($e&6)===0?(Co(0,!1),!1):!0}function Mf(){if(ie!==null){if(xe===0)var e=ie.return;else e=ie,an=Rr=null,bf(e),ys=null,io=0,e=ie;for(;e!==null;)hb(e.alternate,e),e=e.return;ie=null}}function Es(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,hC(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),Mf(),Ce=e,ie=a=rn(e.current,null),le=t,xe=0,Yt=null,Un=!1,Os=go(e,t),Tf=!1,Rs=ha=Af=yr=Gn=He=0,qt=Xi=null,Am=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Zt(n),s=1<<r;t|=e[r],n&=~s}return cn=t,Ou(),a}function Lb(e,t){re=null,ee.H=fu,t===No||t===Uu?(t=jv(),xe=3):t===by?(t=jv(),xe=4):xe=t===ub?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Yt=t,ie===null&&(He=1,vu(e,pa(t,e.current)))}function Ub(){var e=ee.H;return ee.H=fu,e===null?fu:e}function jb(){var e=ee.A;return ee.A=XR,e}function Lm(){He=4,Un||(le&4194048)!==le&&va.current!==null||(Os=!0),(Gn&134217727)===0&&(yr&134217727)===0||Ce===null||jn(Ce,le,ha,!1)}function Qd(e,t,a){var n=$e;$e|=2;var r=Ub(),s=jb();(Ce!==e||le!==t)&&(xu=null,Es(e,t)),t=!1;var i=He;e:do try{if(xe!==0&&ie!==null){var o=ie,u=Yt;switch(xe){case 8:Mf(),i=6;break e;case 3:case 2:case 9:case 6:va.current===null&&(t=!0);var c=xe;if(xe=0,Yt=null,ds(e,o,u,c),a&&Os){i=0;break e}break;default:c=xe,xe=0,Yt=null,ds(e,o,u,c)}}eC(),i=He;break}catch(d){Lb(e,d)}while(!0);return t&&e.shellSuspendCounter++,an=Rr=null,$e=n,ee.H=r,ee.A=s,ie===null&&(Ce=null,le=0,Ou()),i}function eC(){for(;ie!==null;)Pb(ie)}function tC(e,t){var a=$e;$e|=2;var n=Ub(),r=jb();Ce!==e||le!==t?(xu=null,bu=ja()+500,Es(e,t)):Os=go(e,t);e:do try{if(xe!==0&&ie!==null){t=ie;var s=Yt;t:switch(xe){case 1:xe=0,Yt=null,ds(e,t,s,1);break;case 2:case 9:if(Uv(s)){xe=0,Yt=null,ig(t);break}t=function(){xe!==2&&xe!==9||Ce!==e||(xe=7),za(e)},s.then(t,t);break e;case 3:xe=7;break e;case 4:xe=5;break e;case 7:Uv(s)?(xe=0,Yt=null,ig(t)):(xe=0,Yt=null,ds(e,t,s,7));break;case 5:var i=null;switch(ie.tag){case 26:i=ie.memoizedState;case 5:case 27:var o=ie;if(!i||n0(i)){xe=0,Yt=null;var u=o.sibling;if(u!==null)ie=u;else{var c=o.return;c!==null?(ie=c,qu(c)):ie=null}break t}}xe=0,Yt=null,ds(e,t,s,5);break;case 6:xe=0,Yt=null,ds(e,t,s,6);break;case 8:Mf(),He=6;break e;default:throw Error(L(462))}}aC();break}catch(d){Lb(e,d)}while(!0);return an=Rr=null,ee.H=n,ee.A=r,$e=a,ie!==null?0:(Ce=null,le=0,Ou(),He)}function aC(){for(;ie!==null&&!Nk();)Pb(ie)}function Pb(e){var t=pb(e.alternate,e,cn);e.memoizedProps=e.pendingProps,t===null?qu(e):ie=t}function ig(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=Wv(a,t,t.pendingProps,t.type,void 0,le);break;case 11:t=Wv(a,t,t.pendingProps,t.type.render,t.ref,le);break;case 5:bf(t);default:hb(a,t),t=ie=hy(t,cn),t=pb(a,t,cn)}e.memoizedProps=e.pendingProps,t===null?qu(e):ie=t}function ds(e,t,a,n){an=Rr=null,bf(t),ys=null,io=0;var r=t.return;try{if(IR(e,r,t,a,le)){He=1,vu(e,pa(a,e.current)),ie=null;return}}catch(s){if(r!==null)throw ie=r,s;He=1,vu(e,pa(a,e.current)),ie=null;return}t.flags&32768?(ce||n===1?e=!0:Os||(le&536870912)!==0?e=!1:(Un=e=!0,(n===2||n===9||n===3||n===6)&&(n=va.current,n!==null&&n.tag===13&&(n.flags|=16384))),Fb(t,e)):qu(t)}function qu(e){var t=e;do{if((t.flags&32768)!==0){Fb(t,Un);return}e=t.return;var a=VR(t.alternate,t,cn);if(a!==null){ie=a;return}if(t=t.sibling,t!==null){ie=t;return}ie=t=e}while(t!==null);He===0&&(He=5)}function Fb(e,t){do{var a=GR(e.alternate,e);if(a!==null){a.flags&=32767,ie=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){ie=e;return}ie=e=a}while(e!==null);He=6,ie=null}function og(e,t,a,n,r,s,i,o,u){e.cancelPendingCommit=null;do Bu();while(vt!==0);if(($e&6)!==0)throw Error(L(327));if(t!==null){if(t===e.current)throw Error(L(177));if(s=t.lanes|t.childLanes,s|=sf,Ok(e,a,s,i,o,u),e===Ce&&(ie=Ce=null,le=0),Cs=t,Bn=e,bs=a,Dm=s,Mm=r,Db=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,iC(nu,function(){return Kb(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=ee.T,ee.T=null,r=de.p,de.p=2,i=$e,$e|=4;try{YR(e,t,a)}finally{$e=i,de.p=r,ee.T=n}}vt=1,zb(),qb(),Bb()}}function zb(){if(vt===1){vt=0;var e=Bn,t=Cs,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=ee.T,ee.T=null;var n=de.p;de.p=2;var r=$e;$e|=4;try{_b(t,e);var s=Fm,i=oy(e.containerInfo),o=s.focusedElem,u=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&iy(o.ownerDocument.documentElement,o)){if(u!==null&&rf(o)){var c=u.start,d=u.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var f=o.ownerDocument||document,m=f&&f.defaultView||window;if(m.getSelection){var p=m.getSelection(),y=o.textContent.length,b=Math.min(u.start,y),w=u.end===void 0?b:Math.min(u.end,y);!p.extend&&b>w&&(i=w,w=b,b=i);var g=Cv(o,b),v=Cv(o,w);if(g&&v&&(p.rangeCount!==1||p.anchorNode!==g.node||p.anchorOffset!==g.offset||p.focusNode!==v.node||p.focusOffset!==v.offset)){var x=f.createRange();x.setStart(g.node,g.offset),p.removeAllRanges(),b>w?(p.addRange(x),p.extend(v.node,v.offset)):(x.setEnd(v.node,v.offset),p.addRange(x))}}}}for(f=[],p=o;p=p.parentNode;)p.nodeType===1&&f.push({element:p,left:p.scrollLeft,top:p.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<f.length;o++){var $=f[o];$.element.scrollLeft=$.left,$.element.scrollTop=$.top}}Cu=!!Pm,Fm=Pm=null}finally{$e=r,de.p=n,ee.T=a}}e.current=t,vt=2}}function qb(){if(vt===2){vt=0;var e=Bn,t=Cs,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=ee.T,ee.T=null;var n=de.p;de.p=2;var r=$e;$e|=4;try{$b(e,t.alternate,t)}finally{$e=r,de.p=n,ee.T=a}}vt=3}}function Bb(){if(vt===4||vt===3){vt=0,_k();var e=Bn,t=Cs,a=bs,n=Db;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?vt=5:(vt=0,Cs=Bn=null,Hb(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(qn=null),Xm(a),t=t.stateNode,Xt&&typeof Xt.onCommitFiberRoot=="function")try{Xt.onCommitFiberRoot(vo,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=ee.T,r=de.p,de.p=2,ee.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{ee.T=t,de.p=r}}(bs&3)!==0&&Bu(),za(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===Om?Zi++:(Zi=0,Om=e):Zi=0,Co(0,!1)}}function Hb(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,So(t)))}function Bu(e){return zb(),qb(),Bb(),Kb(e)}function Kb(){if(vt!==5)return!1;var e=Bn,t=Dm;Dm=0;var a=Xm(bs),n=ee.T,r=de.p;try{de.p=32>a?32:a,ee.T=null,a=Mm,Mm=null;var s=Bn,i=bs;if(vt=0,Cs=Bn=null,bs=0,($e&6)!==0)throw Error(L(331));var o=$e;if($e|=4,Tb(s.current),Rb(s,s.current,i,a),$e=o,Co(0,!1),Xt&&typeof Xt.onPostCommitFiberRoot=="function")try{Xt.onPostCommitFiberRoot(vo,s)}catch{}return!0}finally{de.p=r,ee.T=n,Hb(e,t)}}function lg(e,t,a){t=pa(a,t),t=Rm(e.stateNode,t,2),e=zn(e,t,2),e!==null&&(yo(e,2),za(e))}function _e(e,t,a){if(e.tag===3)lg(e,e,a);else for(;t!==null;){if(t.tag===3){lg(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(qn===null||!qn.has(n))){e=pa(a,e),a=ob(2),n=zn(t,a,2),n!==null&&(lb(a,n,t,e),yo(n,2),za(n));break}}t=t.return}}function Vd(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new ZR;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(Tf=!0,r.add(a),e=nC.bind(null,e,t,a),t.then(e,e))}function nC(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,Ce===e&&(le&a)===a&&(He===4||He===3&&(le&62914560)===le&&300>ja()-Df?($e&2)===0&&Es(e,0):Af|=a,Rs===le&&(Rs=0)),za(e)}function Ib(e,t){t===0&&(t=zg()),e=Ms(e,t),e!==null&&(yo(e,t),za(e))}function rC(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),Ib(e,a)}function sC(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(L(314))}n!==null&&n.delete(t),Ib(e,a)}function iC(e,t){return Ym(e,t)}var $u=null,Wr=null,Um=!1,wu=!1,Gd=!1,br=0;function za(e){e!==Wr&&e.next===null&&(Wr===null?$u=Wr=e:Wr=Wr.next=e),wu=!0,Um||(Um=!0,lC())}function Co(e,t){if(!Gd&&wu){Gd=!0;do for(var a=!1,n=$u;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Zt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,ug(n,s))}else s=le,s=Tu(n,n===Ce?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||go(n,s)||(a=!0,ug(n,s));n=n.next}while(a);Gd=!1}}function oC(){Qb()}function Qb(){wu=Um=!1;var e=0;br!==0&&(pC()&&(e=br),br=0);for(var t=ja(),a=null,n=$u;n!==null;){var r=n.next,s=Vb(n,t);s===0?(n.next=null,a===null?$u=r:a.next=r,r===null&&(Wr=a)):(a=n,(e!==0||(s&3)!==0)&&(wu=!0)),n=r}Co(e,!1)}function Vb(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Zt(s),o=1<<i,u=r[i];u===-1?((o&a)===0||(o&n)!==0)&&(r[i]=Mk(o,t)):u<=t&&(e.expiredLanes|=o),s&=~o}if(t=Ce,a=le,a=Tu(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&(xe===2||xe===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&$d(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||go(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&$d(n),Xm(a)){case 2:case 8:a=jg;break;case 32:a=nu;break;case 268435456:a=Pg;break;default:a=nu}return n=Gb.bind(null,e),a=Ym(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&$d(n),e.callbackPriority=2,e.callbackNode=null,2}function Gb(e,t){if(vt!==0&&vt!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(Bu(!0)&&e.callbackNode!==a)return null;var n=le;return n=Tu(e,e===Ce?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:(Ob(e,n,t),Vb(e,ja()),e.callbackNode!=null&&e.callbackNode===a?Gb.bind(null,e):null)}function ug(e,t){if(Bu())return null;Ob(e,t,!0)}function lC(){vC(function(){($e&6)!==0?Ym(Ug,oC):Qb()})}function Of(){return br===0&&(br=Fg()),br}function cg(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:Bl(""+e)}function dg(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function uC(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=cg((r[Bt]||null).action),i=n.submitter;i&&(t=(t=i[Bt]||null)?cg(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Au("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(br!==0){var u=i?dg(r,i):new FormData(r);_m(a,{pending:!0,data:u,method:r.method,action:s},null,u)}}else typeof s=="function"&&(o.preventDefault(),u=i?dg(r,i):new FormData(r),_m(a,{pending:!0,data:u,method:r.method,action:s},s,u))},currentTarget:r}]})}}for(Ul=0;Ul<fm.length;Ul++)jl=fm[Ul],mg=jl.toLowerCase(),fg=jl[0].toUpperCase()+jl.slice(1),_a(mg,"on"+fg);var jl,mg,fg,Ul;_a(uy,"onAnimationEnd");_a(cy,"onAnimationIteration");_a(dy,"onAnimationStart");_a("dblclick","onDoubleClick");_a("focusin","onFocus");_a("focusout","onBlur");_a(CR,"onTransitionRun");_a(ER,"onTransitionStart");_a(TR,"onTransitionCancel");_a(my,"onTransitionEnd");$s("onMouseEnter",["mouseout","mouseover"]);$s("onMouseLeave",["mouseout","mouseover"]);$s("onPointerEnter",["pointerout","pointerover"]);$s("onPointerLeave",["pointerout","pointerover"]);Nr("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Nr("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Nr("onBeforeInput",["compositionend","keypress","textInput","paste"]);Nr("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Nr("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Nr("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var oo="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),cC=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(oo));function Yb(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],u=o.instance,c=o.currentTarget;if(o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){hu(d)}r.currentTarget=null,s=u}else for(i=0;i<n.length;i++){if(o=n[i],u=o.instance,c=o.currentTarget,o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){hu(d)}r.currentTarget=null,s=u}}}}function se(e,t){var a=t[im];a===void 0&&(a=t[im]=new Set);var n=e+"__bubble";a.has(n)||(Jb(t,e,2,!1),a.add(n))}function Yd(e,t,a){var n=0;t&&(n|=4),Jb(a,e,n,t)}var Pl="_reactListening"+Math.random().toString(36).slice(2);function Lf(e){if(!e[Pl]){e[Pl]=!0,Kg.forEach(function(a){a!=="selectionchange"&&(cC.has(a)||Yd(a,!1,e),Yd(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[Pl]||(t[Pl]=!0,Yd("selectionchange",!1,t))}}function Jb(e,t,a,n){switch(l0(t)){case 2:var r=jC;break;case 8:r=PC;break;default:r=Ff}a=r.bind(null,t,a,e),r=void 0,!cm||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function Jd(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var u=i.tag;if((u===3||u===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=as(o),i===null)return;if(u=i.tag,u===5||u===6||u===26||u===27){n=s=i;continue e}o=o.parentNode}}n=n.return}Zg(function(){var c=s,d=ef(a),f=[];e:{var m=fy.get(e);if(m!==void 0){var p=Au,y=e;switch(e){case"keypress":if(Kl(a)===0)break e;case"keydown":case"keyup":p=iR;break;case"focusin":y="focus",p=Ed;break;case"focusout":y="blur",p=Ed;break;case"beforeblur":case"afterblur":p=Ed;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":p=bv;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":p=Gk;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":p=uR;break;case uy:case cy:case dy:p=Xk;break;case my:p=dR;break;case"scroll":case"scrollend":p=Qk;break;case"wheel":p=fR;break;case"copy":case"cut":case"paste":p=Wk;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":p=$v;break;case"toggle":case"beforetoggle":p=hR}var b=(t&4)!==0,w=!b&&(e==="scroll"||e==="scrollend"),g=b?m!==null?m+"Capture":null:m;b=[];for(var v=c,x;v!==null;){var $=v;if(x=$.stateNode,$=$.tag,$!==5&&$!==26&&$!==27||x===null||g===null||($=to(v,g),$!=null&&b.push(lo(v,$,x))),w)break;v=v.return}0<b.length&&(m=new p(m,y,null,a,d),f.push({event:m,listeners:b}))}}if((t&7)===0){e:{if(m=e==="mouseover"||e==="pointerover",p=e==="mouseout"||e==="pointerout",m&&a!==um&&(y=a.relatedTarget||a.fromElement)&&(as(y)||y[As]))break e;if((p||m)&&(m=d.window===d?d:(m=d.ownerDocument)?m.defaultView||m.parentWindow:window,p?(y=a.relatedTarget||a.toElement,p=c,y=y?as(y):null,y!==null&&(w=ho(y),b=y.tag,y!==w||b!==5&&b!==27&&b!==6)&&(y=null)):(p=null,y=c),p!==y)){if(b=bv,$="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(b=$v,$="onPointerLeave",g="onPointerEnter",v="pointer"),w=p==null?m:Pi(p),x=y==null?m:Pi(y),m=new b($,v+"leave",p,a,d),m.target=w,m.relatedTarget=x,$=null,as(d)===c&&(b=new b(g,v+"enter",y,a,d),b.target=x,b.relatedTarget=w,$=b),w=$,p&&y)t:{for(b=p,g=y,v=0,x=b;x;x=Jr(x))v++;for(x=0,$=g;$;$=Jr($))x++;for(;0<v-x;)b=Jr(b),v--;for(;0<x-v;)g=Jr(g),x--;for(;v--;){if(b===g||g!==null&&b===g.alternate)break t;b=Jr(b),g=Jr(g)}b=null}else b=null;p!==null&&pg(f,m,p,b,!1),y!==null&&w!==null&&pg(f,w,y,b,!0)}}e:{if(m=c?Pi(c):window,p=m.nodeName&&m.nodeName.toLowerCase(),p==="select"||p==="input"&&m.type==="file")var S=_v;else if(Nv(m))if(ry)S=_R;else{S=SR;var R=wR}else p=m.nodeName,!p||p.toLowerCase()!=="input"||m.type!=="checkbox"&&m.type!=="radio"?c&&Wm(c.elementType)&&(S=_v):S=NR;if(S&&(S=S(e,c))){ny(f,S,a,d);break e}R&&R(e,m,c),e==="focusout"&&c&&m.type==="number"&&c.memoizedProps.value!=null&&lm(m,"number",m.value)}switch(R=c?Pi(c):window,e){case"focusin":(Nv(R)||R.contentEditable==="true")&&(ss=R,dm=c,Bi=null);break;case"focusout":Bi=dm=ss=null;break;case"mousedown":mm=!0;break;case"contextmenu":case"mouseup":case"dragend":mm=!1,Ev(f,a,d);break;case"selectionchange":if(RR)break;case"keydown":case"keyup":Ev(f,a,d)}var _;if(nf)e:{switch(e){case"compositionstart":var C="onCompositionStart";break e;case"compositionend":C="onCompositionEnd";break e;case"compositionupdate":C="onCompositionUpdate";break e}C=void 0}else rs?ty(e,a)&&(C="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(C="onCompositionStart");C&&(ey&&a.locale!=="ko"&&(rs||C!=="onCompositionStart"?C==="onCompositionEnd"&&rs&&(_=Wg()):(Ln=d,tf="value"in Ln?Ln.value:Ln.textContent,rs=!0)),R=Su(c,C),0<R.length&&(C=new xv(C,e,null,a,d),f.push({event:C,listeners:R}),_?C.data=_:(_=ay(a),_!==null&&(C.data=_)))),(_=gR?yR(e,a):bR(e,a))&&(C=Su(c,"onBeforeInput"),0<C.length&&(R=new xv("onBeforeInput","beforeinput",null,a,d),f.push({event:R,listeners:C}),R.data=_)),uC(f,e,c,a,d)}Yb(f,t)})}function lo(e,t,a){return{instance:e,listener:t,currentTarget:a}}function Su(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=to(e,a),r!=null&&n.unshift(lo(e,r,s)),r=to(e,t),r!=null&&n.push(lo(e,r,s))),e.tag===3)return n;e=e.return}return[]}function Jr(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function pg(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,u=o.alternate,c=o.stateNode;if(o=o.tag,u!==null&&u===n)break;o!==5&&o!==26&&o!==27||c===null||(u=c,r?(c=to(a,s),c!=null&&i.unshift(lo(a,c,u))):r||(c=to(a,s),c!=null&&i.push(lo(a,c,u)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var dC=/\r\n?/g,mC=/\u0000|\uFFFD/g;function hg(e){return(typeof e=="string"?e:""+e).replace(dC,`
`).replace(mC,"")}function Xb(e,t){return t=hg(t),hg(e)===t}function Hu(){}function we(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||ws(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&ws(e,""+n);break;case"className":Rl(e,"class",n);break;case"tabIndex":Rl(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":Rl(e,a,n);break;case"style":Xg(e,n,s);break;case"data":if(t!=="object"){Rl(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=Bl(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&we(e,t,"name",r.name,r,null),we(e,t,"formEncType",r.formEncType,r,null),we(e,t,"formMethod",r.formMethod,r,null),we(e,t,"formTarget",r.formTarget,r,null)):(we(e,t,"encType",r.encType,r,null),we(e,t,"method",r.method,r,null),we(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=Bl(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=Hu);break;case"onScroll":n!=null&&se("scroll",e);break;case"onScrollEnd":n!=null&&se("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(L(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(L(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=Bl(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":se("beforetoggle",e),se("toggle",e),ql(e,"popover",n);break;case"xlinkActuate":Ya(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":Ya(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":Ya(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":Ya(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":Ya(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":Ya(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":Ya(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":Ya(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":Ya(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":ql(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=Kk.get(a)||a,ql(e,a,n))}}function jm(e,t,a,n,r,s){switch(a){case"style":Xg(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(L(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(L(60));e.innerHTML=a}}break;case"children":typeof n=="string"?ws(e,n):(typeof n=="number"||typeof n=="bigint")&&ws(e,""+n);break;case"onScroll":n!=null&&se("scroll",e);break;case"onScrollEnd":n!=null&&se("scrollend",e);break;case"onClick":n!=null&&(e.onclick=Hu);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!Ig.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[Bt]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):ql(e,a,n)}}}function gt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":se("error",e),se("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(L(137,t));default:we(e,t,s,i,a,null)}}r&&we(e,t,"srcSet",a.srcSet,a,null),n&&we(e,t,"src",a.src,a,null);return;case"input":se("invalid",e);var o=s=i=r=null,u=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":u=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(L(137,t));break;default:we(e,t,n,d,a,null)}}Gg(e,s,o,u,c,i,r,!1),ru(e);return;case"select":se("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:we(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?fs(e,!!n,t,!1):a!=null&&fs(e,!!n,a,!0);return;case"textarea":se("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(L(91));break;default:we(e,t,i,o,a,null)}Jg(e,n,r,s),ru(e);return;case"option":for(u in a)if(a.hasOwnProperty(u)&&(n=a[u],n!=null))switch(u){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:we(e,t,u,n,a,null)}return;case"dialog":se("beforetoggle",e),se("toggle",e),se("cancel",e),se("close",e);break;case"iframe":case"object":se("load",e);break;case"video":case"audio":for(n=0;n<oo.length;n++)se(oo[n],e);break;case"image":se("error",e),se("load",e);break;case"details":se("toggle",e);break;case"embed":case"source":case"link":se("error",e),se("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(L(137,t));default:we(e,t,c,n,a,null)}return;default:if(Wm(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&jm(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&we(e,t,o,n,a,null))}function fC(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,u=null,c=null,d=null;for(p in a){var f=a[p];if(a.hasOwnProperty(p)&&f!=null)switch(p){case"checked":break;case"value":break;case"defaultValue":u=f;default:n.hasOwnProperty(p)||we(e,t,p,null,n,f)}}for(var m in n){var p=n[m];if(f=a[m],n.hasOwnProperty(m)&&(p!=null||f!=null))switch(m){case"type":s=p;break;case"name":r=p;break;case"checked":c=p;break;case"defaultChecked":d=p;break;case"value":i=p;break;case"defaultValue":o=p;break;case"children":case"dangerouslySetInnerHTML":if(p!=null)throw Error(L(137,t));break;default:p!==f&&we(e,t,m,p,n,f)}}om(e,i,o,u,c,d,s,r);return;case"select":p=i=o=m=null;for(s in a)if(u=a[s],a.hasOwnProperty(s)&&u!=null)switch(s){case"value":break;case"multiple":p=u;default:n.hasOwnProperty(s)||we(e,t,s,null,n,u)}for(r in n)if(s=n[r],u=a[r],n.hasOwnProperty(r)&&(s!=null||u!=null))switch(r){case"value":m=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==u&&we(e,t,r,s,n,u)}t=o,a=i,n=p,m!=null?fs(e,!!a,m,!1):!!n!=!!a&&(t!=null?fs(e,!!a,t,!0):fs(e,!!a,a?[]:"",!1));return;case"textarea":p=m=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:we(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":m=r;break;case"defaultValue":p=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(L(91));break;default:r!==s&&we(e,t,i,r,n,s)}Yg(e,m,p);return;case"option":for(var y in a)if(m=a[y],a.hasOwnProperty(y)&&m!=null&&!n.hasOwnProperty(y))switch(y){case"selected":e.selected=!1;break;default:we(e,t,y,null,n,m)}for(u in n)if(m=n[u],p=a[u],n.hasOwnProperty(u)&&m!==p&&(m!=null||p!=null))switch(u){case"selected":e.selected=m&&typeof m!="function"&&typeof m!="symbol";break;default:we(e,t,u,m,n,p)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var b in a)m=a[b],a.hasOwnProperty(b)&&m!=null&&!n.hasOwnProperty(b)&&we(e,t,b,null,n,m);for(c in n)if(m=n[c],p=a[c],n.hasOwnProperty(c)&&m!==p&&(m!=null||p!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(m!=null)throw Error(L(137,t));break;default:we(e,t,c,m,n,p)}return;default:if(Wm(t)){for(var w in a)m=a[w],a.hasOwnProperty(w)&&m!==void 0&&!n.hasOwnProperty(w)&&jm(e,t,w,void 0,n,m);for(d in n)m=n[d],p=a[d],!n.hasOwnProperty(d)||m===p||m===void 0&&p===void 0||jm(e,t,d,m,n,p);return}}for(var g in a)m=a[g],a.hasOwnProperty(g)&&m!=null&&!n.hasOwnProperty(g)&&we(e,t,g,null,n,m);for(f in n)m=n[f],p=a[f],!n.hasOwnProperty(f)||m===p||m==null&&p==null||we(e,t,f,m,n,p)}var Pm=null,Fm=null;function Nu(e){return e.nodeType===9?e:e.ownerDocument}function vg(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function Zb(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function zm(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var Xd=null;function pC(){var e=window.event;return e&&e.type==="popstate"?e===Xd?!1:(Xd=e,!0):(Xd=null,!1)}var Wb=typeof setTimeout=="function"?setTimeout:void 0,hC=typeof clearTimeout=="function"?clearTimeout:void 0,gg=typeof Promise=="function"?Promise:void 0,vC=typeof queueMicrotask=="function"?queueMicrotask:typeof gg<"u"?function(e){return gg.resolve(null).then(e).catch(gC)}:Wb;function gC(e){setTimeout(function(){throw e})}function Jn(e){return e==="head"}function yg(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&Wi(i.documentElement),a&2&&Wi(i.body),a&4)for(a=i.head,Wi(a),i=a.firstChild;i;){var o=i.nextSibling,u=i.nodeName;i[bo]||u==="SCRIPT"||u==="STYLE"||u==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),po(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);po(t)}function qm(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":qm(a),Zm(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function yC(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[bo])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=Na(e.nextSibling),e===null)break}return null}function bC(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=Na(e.nextSibling),e===null))return null;return e}function Bm(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function xC(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function Na(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var Hm=null;function bg(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function e0(e,t,a){switch(t=Nu(a),e){case"html":if(e=t.documentElement,!e)throw Error(L(452));return e;case"head":if(e=t.head,!e)throw Error(L(453));return e;case"body":if(e=t.body,!e)throw Error(L(454));return e;default:throw Error(L(451))}}function Wi(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);Zm(e)}var ga=new Map,xg=new Set;function _u(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var dn=de.d;de.d={f:$C,r:wC,D:SC,C:NC,L:_C,m:kC,X:CC,S:RC,M:EC};function $C(){var e=dn.f(),t=zu();return e||t}function wC(e){var t=Ds(e);t!==null&&t.tag===5&&t.type==="form"?Vy(t):dn.r(e)}var Ls=typeof document>"u"?null:document;function t0(e,t,a){var n=Ls;if(n&&typeof t=="string"&&t){var r=fa(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),xg.has(r)||(xg.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),gt(t,"link",e),dt(t),n.head.appendChild(t)))}}function SC(e){dn.D(e),t0("dns-prefetch",e,null)}function NC(e,t){dn.C(e,t),t0("preconnect",e,t)}function _C(e,t,a){dn.L(e,t,a);var n=Ls;if(n&&e&&t){var r='link[rel="preload"][as="'+fa(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+fa(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+fa(a.imageSizes)+'"]')):r+='[href="'+fa(e)+'"]';var s=r;switch(t){case"style":s=Ts(e);break;case"script":s=Us(e)}ga.has(s)||(e=De({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),ga.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(Eo(s))||t==="script"&&n.querySelector(To(s))||(t=n.createElement("link"),gt(t,"link",e),dt(t),n.head.appendChild(t)))}}function kC(e,t){dn.m(e,t);var a=Ls;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+fa(n)+'"][href="'+fa(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=Us(e)}if(!ga.has(s)&&(e=De({rel:"modulepreload",href:e},t),ga.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(To(s)))return}n=a.createElement("link"),gt(n,"link",e),dt(n),a.head.appendChild(n)}}}function RC(e,t,a){dn.S(e,t,a);var n=Ls;if(n&&e){var r=ms(n).hoistableStyles,s=Ts(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(Eo(s)))o.loading=5;else{e=De({rel:"stylesheet",href:e,"data-precedence":t},a),(a=ga.get(s))&&Uf(e,a);var u=i=n.createElement("link");dt(u),gt(u,"link",e),u._p=new Promise(function(c,d){u.onload=c,u.onerror=d}),u.addEventListener("load",function(){o.loading|=1}),u.addEventListener("error",function(){o.loading|=2}),o.loading|=4,Xl(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function CC(e,t){dn.X(e,t);var a=Ls;if(a&&e){var n=ms(a).hoistableScripts,r=Us(e),s=n.get(r);s||(s=a.querySelector(To(r)),s||(e=De({src:e,async:!0},t),(t=ga.get(r))&&jf(e,t),s=a.createElement("script"),dt(s),gt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function EC(e,t){dn.M(e,t);var a=Ls;if(a&&e){var n=ms(a).hoistableScripts,r=Us(e),s=n.get(r);s||(s=a.querySelector(To(r)),s||(e=De({src:e,async:!0,type:"module"},t),(t=ga.get(r))&&jf(e,t),s=a.createElement("script"),dt(s),gt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function $g(e,t,a,n){var r=(r=Pn.current)?_u(r):null;if(!r)throw Error(L(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=Ts(a.href),a=ms(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=Ts(a.href);var s=ms(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(Eo(e)))&&!s._p&&(i.instance=s,i.state.loading=5),ga.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},ga.set(e,a),s||TC(r,e,a,i.state))),t&&n===null)throw Error(L(528,""));return i}if(t&&n!==null)throw Error(L(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=Us(a),a=ms(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(L(444,e))}}function Ts(e){return'href="'+fa(e)+'"'}function Eo(e){return'link[rel="stylesheet"]['+e+"]"}function a0(e){return De({},e,{"data-precedence":e.precedence,precedence:null})}function TC(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),gt(t,"link",a),dt(t),e.head.appendChild(t))}function Us(e){return'[src="'+fa(e)+'"]'}function To(e){return"script[async]"+e}function wg(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+fa(a.href)+'"]');if(n)return t.instance=n,dt(n),n;var r=De({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),dt(n),gt(n,"style",r),Xl(n,a.precedence,e),t.instance=n;case"stylesheet":r=Ts(a.href);var s=e.querySelector(Eo(r));if(s)return t.state.loading|=4,t.instance=s,dt(s),s;n=a0(a),(r=ga.get(r))&&Uf(n,r),s=(e.ownerDocument||e).createElement("link"),dt(s);var i=s;return i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),gt(s,"link",n),t.state.loading|=4,Xl(s,a.precedence,e),t.instance=s;case"script":return s=Us(a.src),(r=e.querySelector(To(s)))?(t.instance=r,dt(r),r):(n=a,(r=ga.get(s))&&(n=De({},a),jf(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),dt(r),gt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(L(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,Xl(n,a.precedence,e));return t.instance}function Xl(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function Uf(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function jf(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var Zl=null;function Sg(e,t,a){if(Zl===null){var n=new Map,r=Zl=new Map;r.set(a,n)}else r=Zl,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[bo]||s[wt]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function Ng(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function AC(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function n0(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var uo=null;function DC(){}function MC(e,t,a){if(uo===null)throw Error(L(475));var n=uo;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=Ts(a.href),s=e.querySelector(Eo(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=ku.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,dt(s);return}s=e.ownerDocument||e,a=a0(a),(r=ga.get(r))&&Uf(a,r),s=s.createElement("link"),dt(s);var i=s;i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),gt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=ku.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function OC(){if(uo===null)throw Error(L(475));var e=uo;return e.stylesheets&&e.count===0&&Km(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&Km(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function ku(){if(this.count--,this.count===0){if(this.stylesheets)Km(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var Ru=null;function Km(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,Ru=new Map,t.forEach(LC,e),Ru=null,ku.call(e))}function LC(e,t){if(!(t.state.loading&4)){var a=Ru.get(e);if(a)var n=a.get(null);else{a=new Map,Ru.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=ku.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var co={$$typeof:Wa,Provider:null,Consumer:null,_currentValue:fr,_currentValue2:fr,_threadCount:0};function UC(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=wd(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=wd(0),this.hiddenUpdates=wd(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function r0(e,t,a,n,r,s,i,o,u,c,d,f){return e=new UC(e,t,a,i,o,u,c,f),t=1,s===!0&&(t|=24),s=Jt(3,null,null,t),e.current=s,s.stateNode=e,t=df(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},ff(s),e}function s0(e){return e?(e=ls,e):ls}function i0(e,t,a,n,r,s){r=s0(r),n.context===null?n.context=r:n.pendingContext=r,n=Fn(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=zn(e,n,t),a!==null&&(ea(a,e,t),Ii(a,e,t))}function _g(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function Pf(e,t){_g(e,t),(e=e.alternate)&&_g(e,t)}function o0(e){if(e.tag===13){var t=Ms(e,67108864);t!==null&&ea(t,e,67108864),Pf(e,67108864)}}var Cu=!0;function jC(e,t,a,n){var r=ee.T;ee.T=null;var s=de.p;try{de.p=2,Ff(e,t,a,n)}finally{de.p=s,ee.T=r}}function PC(e,t,a,n){var r=ee.T;ee.T=null;var s=de.p;try{de.p=8,Ff(e,t,a,n)}finally{de.p=s,ee.T=r}}function Ff(e,t,a,n){if(Cu){var r=Im(n);if(r===null)Jd(e,t,n,Eu,a),kg(e,n);else if(zC(r,e,t,a,n))n.stopPropagation();else if(kg(e,n),t&4&&-1<FC.indexOf(e)){for(;r!==null;){var s=Ds(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=cr(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var u=1<<31-Zt(i);o.entanglements[1]|=u,i&=~u}za(s),($e&6)===0&&(bu=ja()+500,Co(0,!1))}}break;case 13:o=Ms(s,2),o!==null&&ea(o,s,2),zu(),Pf(s,2)}if(s=Im(n),s===null&&Jd(e,t,n,Eu,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else Jd(e,t,n,null,a)}}function Im(e){return e=ef(e),zf(e)}var Eu=null;function zf(e){if(Eu=null,e=as(e),e!==null){var t=ho(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=Dg(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return Eu=e,null}function l0(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(kk()){case Ug:return 2;case jg:return 8;case nu:case Rk:return 32;case Pg:return 268435456;default:return 32}default:return 32}}var Qm=!1,Hn=null,Kn=null,In=null,mo=new Map,fo=new Map,Mn=[],FC="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function kg(e,t){switch(e){case"focusin":case"focusout":Hn=null;break;case"dragenter":case"dragleave":Kn=null;break;case"mouseover":case"mouseout":In=null;break;case"pointerover":case"pointerout":mo.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":fo.delete(t.pointerId)}}function Oi(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Ds(t),t!==null&&o0(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function zC(e,t,a,n,r){switch(t){case"focusin":return Hn=Oi(Hn,e,t,a,n,r),!0;case"dragenter":return Kn=Oi(Kn,e,t,a,n,r),!0;case"mouseover":return In=Oi(In,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return mo.set(s,Oi(mo.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,fo.set(s,Oi(fo.get(s)||null,e,t,a,n,r)),!0}return!1}function u0(e){var t=as(e.target);if(t!==null){var a=ho(t);if(a!==null){if(t=a.tag,t===13){if(t=Dg(a),t!==null){e.blockedOn=t,Lk(e.priority,function(){if(a.tag===13){var n=Wt();n=Jm(n);var r=Ms(a,n);r!==null&&ea(r,a,n),Pf(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function Wl(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=Im(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);um=n,a.target.dispatchEvent(n),um=null}else return t=Ds(a),t!==null&&o0(t),e.blockedOn=a,!1;t.shift()}return!0}function Rg(e,t,a){Wl(e)&&a.delete(t)}function qC(){Qm=!1,Hn!==null&&Wl(Hn)&&(Hn=null),Kn!==null&&Wl(Kn)&&(Kn=null),In!==null&&Wl(In)&&(In=null),mo.forEach(Rg),fo.forEach(Rg)}function Fl(e,t){e.blockedOn===t&&(e.blockedOn=null,Qm||(Qm=!0,st.unstable_scheduleCallback(st.unstable_NormalPriority,qC)))}var zl=null;function Cg(e){zl!==e&&(zl=e,st.unstable_scheduleCallback(st.unstable_NormalPriority,function(){zl===e&&(zl=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(zf(n||a)===null)continue;break}var s=Ds(a);s!==null&&(e.splice(t,3),t-=3,_m(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function po(e){function t(u){return Fl(u,e)}Hn!==null&&Fl(Hn,e),Kn!==null&&Fl(Kn,e),In!==null&&Fl(In,e),mo.forEach(t),fo.forEach(t);for(var a=0;a<Mn.length;a++){var n=Mn[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<Mn.length&&(a=Mn[0],a.blockedOn===null);)u0(a),a.blockedOn===null&&Mn.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[Bt]||null;if(typeof s=="function")i||Cg(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[Bt]||null)o=i.formAction;else if(zf(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),Cg(a)}}}function qf(e){this._internalRoot=e}Ku.prototype.render=qf.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(L(409));var a=t.current,n=Wt();i0(a,n,e,t,null,null)};Ku.prototype.unmount=qf.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;i0(e.current,2,null,e,null,null),zu(),t[As]=null}};function Ku(e){this._internalRoot=e}Ku.prototype.unstable_scheduleHydration=function(e){if(e){var t=Hg();e={blockedOn:null,target:e,priority:t};for(var a=0;a<Mn.length&&t!==0&&t<Mn[a].priority;a++);Mn.splice(a,0,e),a===0&&u0(e)}};var Eg=Tg.version;if(Eg!=="19.1.0")throw Error(L(527,Eg,"19.1.0"));de.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(L(188)):(e=Object.keys(e).join(","),Error(L(268,e)));return e=bk(t),e=e!==null?Mg(e):null,e=e===null?null:e.stateNode,e};var BC={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:ee,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(Li=__REACT_DEVTOOLS_GLOBAL_HOOK__,!Li.isDisabled&&Li.supportsFiber))try{vo=Li.inject(BC),Xt=Li}catch{}var Li;Iu.createRoot=function(e,t){if(!Ag(e))throw Error(L(299));var a=!1,n="",r=rb,s=sb,i=ib,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=r0(e,1,!1,null,null,a,n,r,s,i,o,null),e[As]=t.current,Lf(e),new qf(t)};Iu.hydrateRoot=function(e,t,a){if(!Ag(e))throw Error(L(299));var n=!1,r="",s=rb,i=sb,o=ib,u=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(u=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=r0(e,1,!0,t,a??null,n,r,s,i,o,u,c),t.context=s0(null),a=t.current,n=Wt(),n=Jm(n),r=Fn(n),r.callback=null,zn(a,r,n),a=n,t.current.lanes=a,yo(t,a),za(t),e[As]=t.current,Lf(e),new Ku(t)};Iu.version="19.1.0"});var f0=$n((lO,m0)=>{"use strict";function d0(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(d0)}catch(e){console.error(e)}}d0(),m0.exports=c0()});var Ut=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var Z_={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},W_=class{#t=Z_;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},Ea=new W_;function wh(e){setTimeout(e,0)}var jt=typeof window>"u"||"Deno"in globalThis;function Me(){}function _h(e,t){return typeof e=="function"?e(t):e}function vi(e){return typeof e=="number"&&e>=0&&e!==1/0}function nl(e,t){return Math.max(e+(t||0)-Date.now(),0)}function wa(e,t){return typeof e=="function"?e(t):e}function Pt(e,t){return typeof e=="function"?e(t):e}function rl(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==gi(i,t.options))return!1}else if(!or(t.queryKey,i))return!1}if(a!=="all"){let u=t.isActive();if(a==="active"&&!u||a==="inactive"&&u)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function sl(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(Ta(t.options.mutationKey)!==Ta(s))return!1}else if(!or(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function gi(e,t){return(t?.queryKeyHashFn||Ta)(e)}function Ta(e){return JSON.stringify(e,(t,a)=>Wc(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function or(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>or(e[a],t[a])):!1}var ek=Object.prototype.hasOwnProperty;function yi(e,t){if(e===t)return e;let a=Sh(e)&&Sh(t);if(!a&&!(Wc(e)&&Wc(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},u=0;for(let c=0;c<i;c++){let d=a?c:s[c],f=e[d],m=t[d];if(f===m){o[d]=f,(a?c<r:ek.call(e,d))&&u++;continue}if(f===null||m===null||typeof f!="object"||typeof m!="object"){o[d]=m;continue}let p=yi(f,m);o[d]=p,p===f&&u++}return r===i&&u===r?e:o}function wn(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function Sh(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function Wc(e){if(!Nh(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!Nh(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function Nh(e){return Object.prototype.toString.call(e)==="[object Object]"}function kh(e){return new Promise(t=>{Ea.setTimeout(t,e)})}function bi(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?yi(e,t):t}function Rh(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function Ch(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var qr=Symbol();function il(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===qr?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function xi(e,t){return typeof e=="function"?e(...t):!!e}var tk=class extends Ut{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!jt&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},Br=new tk;function $i(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var Eh=wh;function ak(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=Eh,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(u=>{a(u)})})})};return{batch:o=>{let u;t++;try{u=o()}finally{t--,t||i()}return u},batchCalls:o=>(...u)=>{s(()=>{o(...u)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var oe=ak();var nk=class extends Ut{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!jt&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},Hr=new nk;function rk(e){return Math.min(1e3*2**e,3e4)}function ed(e){return(e??"online")==="online"?Hr.isOnline():!0}var ol=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function ll(e){let t=!1,a=0,n,r=$i(),s=()=>r.status!=="pending",i=b=>{if(!s()){let w=new ol(b);m(w),e.onCancel?.(w)}},o=()=>{t=!0},u=()=>{t=!1},c=()=>Br.isFocused()&&(e.networkMode==="always"||Hr.isOnline())&&e.canRun(),d=()=>ed(e.networkMode)&&e.canRun(),f=b=>{s()||(n?.(),r.resolve(b))},m=b=>{s()||(n?.(),r.reject(b))},p=()=>new Promise(b=>{n=w=>{(s()||c())&&b(w)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),y=()=>{if(s())return;let b,w=a===0?e.initialPromise:void 0;try{b=w??e.fn()}catch(g){b=Promise.reject(g)}Promise.resolve(b).then(f).catch(g=>{if(s())return;let v=e.retry??(jt?0:3),x=e.retryDelay??rk,$=typeof x=="function"?x(a,g):x,S=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!S){m(g);return}a++,e.onFail?.(a,g),kh($).then(()=>c()?void 0:p()).then(()=>{t?m(g):y()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:u,canStart:d,start:()=>(d()?y():p().then(y),r)}}var ul=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),vi(this.gcTime)&&(this.#t=Ea.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(jt?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(Ea.clearTimeout(this.#t),this.#t=void 0)}};var Ah=class extends ul{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=Th(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=Th(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=bi(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(Me).catch(Me):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>Pt(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===qr||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>wa(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!nl(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(u=>u.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=il(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=ll({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof ol&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,u)=>{this.#i({type:"failed",failureCount:o,error:u})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof ol){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,...td(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),oe.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function td(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:ed(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function Th(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var lr=class extends Ut{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=$i(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),Dh(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return ad(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return ad(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof Pt(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!wn(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&Mh(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||Pt(this.options.enabled,this.#e)!==Pt(t.enabled,this.#e)||wa(this.options.staleTime,this.#e)!==wa(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||Pt(this.options.enabled,this.#e)!==Pt(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return ik(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(Me)),t}#v(){this.#x();let e=wa(this.options.staleTime,this.#e);if(jt||this.#n.isStale||!vi(e))return;let a=nl(this.#n.dataUpdatedAt,e)+1;this.#u=Ea.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(jt||Pt(this.options.enabled,this.#e)===!1||!vi(this.#l)||this.#l===0)&&(this.#c=Ea.setInterval(()=>{(this.options.refetchIntervalInBackground||Br.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(Ea.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(Ea.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,u=e!==a?e.state:this.#a,{state:c}=e,d={...c},f=!1,m;if(t._optimisticResults){let C=this.hasListeners(),U=!C&&Dh(e,t),O=C&&Mh(e,a,t,n);(U||O)&&(d={...d,...td(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:p,errorUpdatedAt:y,status:b}=d;m=d.data;let w=!1;if(t.placeholderData!==void 0&&m===void 0&&b==="pending"){let C;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(C=r.data,w=!0):C=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,C!==void 0&&(b="success",m=bi(r?.data,C,t),f=!0)}if(t.select&&m!==void 0&&!w)if(r&&m===s?.data&&t.select===this.#f)m=this.#d;else try{this.#f=t.select,m=t.select(m),m=bi(r?.data,m,t),this.#d=m,this.#i=null}catch(C){this.#i=C}this.#i&&(p=this.#i,m=this.#d,y=Date.now(),b="error");let g=d.fetchStatus==="fetching",v=b==="pending",x=b==="error",$=v&&g,S=m!==void 0,_={status:b,fetchStatus:d.fetchStatus,isPending:v,isSuccess:b==="success",isError:x,isInitialLoading:$,isLoading:$,data:m,dataUpdatedAt:d.dataUpdatedAt,error:p,errorUpdatedAt:y,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>u.dataUpdateCount||d.errorUpdateCount>u.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:x&&!S,isPaused:d.fetchStatus==="paused",isPlaceholderData:f,isRefetchError:x&&S,isStale:nd(e,t),refetch:this.refetch,promise:this.#o,isEnabled:Pt(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let C=B=>{_.status==="error"?B.reject(_.error):_.data!==void 0&&B.resolve(_.data)},U=()=>{let B=this.#o=_.promise=$i();C(B)},O=this.#o;switch(O.status){case"pending":e.queryHash===a.queryHash&&C(O);break;case"fulfilled":(_.status==="error"||_.data!==O.value)&&U();break;case"rejected":(_.status!=="error"||_.error!==O.reason)&&U();break}}return _}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),wn(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){oe.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function sk(e,t){return Pt(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function Dh(e,t){return sk(e,t)||e.state.data!==void 0&&ad(e,t,t.refetchOnMount)}function ad(e,t,a){if(Pt(t.enabled,e)!==!1&&wa(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&nd(e,t)}return!1}function Mh(e,t,a,n){return(e!==t||Pt(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&nd(e,a)}function nd(e,t){return Pt(t.enabled,e)!==!1&&e.isStaleByTime(wa(t.staleTime,e))}function ik(e,t){return!wn(e.getCurrentResult(),t)}function rd(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},u=0,c=async()=>{let d=!1,f=y=>{Object.defineProperty(y,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},m=il(t.options,t.fetchOptions),p=async(y,b,w)=>{if(d)return Promise.reject();if(b==null&&y.pages.length)return Promise.resolve(y);let v=(()=>{let R={client:t.client,queryKey:t.queryKey,pageParam:b,direction:w?"backward":"forward",meta:t.options.meta};return f(R),R})(),x=await m(v),{maxPages:$}=t.options,S=w?Ch:Rh;return{pages:S(y.pages,x,$),pageParams:S(y.pageParams,b,$)}};if(r&&s.length){let y=r==="backward",b=y?ok:Oh,w={pages:s,pageParams:i},g=b(n,w);o=await p(w,g,y)}else{let y=e??s.length;do{let b=u===0?i[0]??n.initialPageParam:Oh(n,o);if(u>0&&b==null)break;o=await p(o,b),u++}while(u<y)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function Oh(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function ok(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var Lh=class extends ul{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||sd(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=ll({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),oe.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function sd(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var Uh=class extends Ut{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new Lh({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=cl(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=cl(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=cl(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=cl(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){oe.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>sl(t,a))}findAll(e={}){return this.getAll().filter(t=>sl(e,t))}notify(e){oe.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return oe.batch(()=>Promise.all(e.map(t=>t.continue().catch(Me))))}};function cl(e){return e.options.scope?.id}var id=class extends Ut{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),wn(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&Ta(t.mutationKey)!==Ta(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??sd();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){oe.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function jh(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function lk(e,t,a){let n=e.slice(0);return n[t]=a,n}var od=class extends Ut{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,oe.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,f)=>d!==a[f]),u=i||o,c=u?!0:s.some((d,f)=>{let m=this.#e[f];return!m||!wn(d,m)});!u&&!c||(u&&(this.#r=r),this.#e=s,this.hasListeners()&&(u&&(jh(a,r).forEach(d=>{d.destroy()}),jh(r,a).forEach(d=>{d.subscribe(f=>{this.#c(d,f)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=yi(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new lr(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=lk(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&oe.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var Ph=class extends Ut{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??gi(n,t),s=this.get(r);return s||(s=new Ah({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){oe.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>rl(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>rl(e,a)):t}notify(e){oe.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){oe.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){oe.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var ld=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new Ph,this.#e=e.mutationCache||new Uh,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=Br.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=Hr.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime(wa(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=_h(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return oe.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;oe.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return oe.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=oe.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(Me).catch(Me)}invalidateQueries(e,t={}){return oe.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=oe.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(Me)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(Me)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime(wa(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(Me).catch(Me)}fetchInfiniteQuery(e){return e.behavior=rd(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(Me).catch(Me)}ensureInfiniteQueryData(e){return e.behavior=rd(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return Hr.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(Ta(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{or(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(Ta(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{or(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=gi(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===qr&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var Aa=qe(Ie(),1);var Kr=qe(Ie(),1),Bh=qe(ud(),1),cd=Kr.createContext(void 0),Y=e=>{let t=Kr.useContext(cd);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},dd=({client:e,children:t})=>(Kr.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,Bh.jsx)(cd.Provider,{value:e,children:t}));var ml=qe(Ie(),1),Hh=ml.createContext(!1),fl=()=>ml.useContext(Hh),NM=Hh.Provider;var wi=qe(Ie(),1),dk=qe(ud(),1);function mk(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var fk=wi.createContext(mk()),pl=()=>wi.useContext(fk);var Kh=qe(Ie(),1);var hl=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},vl=e=>{Kh.useEffect(()=>{e.clearReset()},[e])},gl=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||xi(a,[e.error,n]));var yl=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},bl=(e,t)=>e.isLoading&&e.isFetching&&!t,Si=(e,t)=>e?.suspense&&t.isPending,Ir=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function md({queries:e,...t},a){let n=Y(a),r=fl(),s=pl(),i=Aa.useMemo(()=>e.map(b=>{let w=n.defaultQueryOptions(b);return w._optimisticResults=r?"isRestoring":"optimistic",w}),[e,n,r]);i.forEach(b=>{yl(b),hl(b,s)}),vl(s);let[o]=Aa.useState(()=>new od(n,i,t)),[u,c,d]=o.getOptimisticResult(i,t.combine),f=!r&&t.subscribed!==!1;Aa.useSyncExternalStore(Aa.useCallback(b=>f?o.subscribe(oe.batchCalls(b)):Me,[o,f]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),Aa.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let p=u.some((b,w)=>Si(i[w],b))?u.flatMap((b,w)=>{let g=i[w];if(g){let v=new lr(n,g);if(Si(g,b))return Ir(g,v,s);bl(b,r)&&Ir(g,v,s)}return[]}):[];if(p.length>0)throw Promise.all(p);let y=u.find((b,w)=>{let g=i[w];return g&&gl({result:b,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(y?.error)throw y.error;return c(d())}var Sn=qe(Ie(),1);function Ih(e,t,a){let n=fl(),r=pl(),s=Y(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",yl(i),hl(i,r),vl(r);let o=!s.getQueryCache().get(i.queryHash),[u]=Sn.useState(()=>new t(s,i)),c=u.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(Sn.useSyncExternalStore(Sn.useCallback(f=>{let m=d?u.subscribe(oe.batchCalls(f)):Me;return u.updateResult(),m},[u,d]),()=>u.getCurrentResult(),()=>u.getCurrentResult()),Sn.useEffect(()=>{u.setOptions(i)},[i,u]),Si(i,c))throw Ir(i,u,r);if(gl({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!jt&&bl(c,n)&&(o?Ir(i,u,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(Me).finally(()=>{u.updateResult()}),i.notifyOnChangeProps?c:u.trackResult(c)}function z(e,t){return Ih(e,lr,t)}var Va=qe(Ie(),1);function I(e,t){let a=Y(t),[n]=Va.useState(()=>new id(a,e));Va.useEffect(()=>{n.setOptions(e)},[n,e]);let r=Va.useSyncExternalStore(Va.useCallback(i=>n.subscribe(oe.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=Va.useCallback((i,o)=>{n.mutate(i,o).catch(Me)},[n]);if(r.error&&xi(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var G_=qe(f0());var na=qe(Ie(),1),V=qe(Ie(),1),Te=qe(Ie(),1),ip=qe(Ie(),1),U0=qe(Ie(),1),me=qe(Ie(),1),H3=qe(Ie(),1),K3=qe(Ie(),1),I3=qe(Ie(),1),X=qe(Ie(),1),X0=qe(Ie(),1);var p0="popstate";function b0(e={}){function t(n,r){let{pathname:s,search:i,hash:o}=n.location;return Kf("",{pathname:s,search:i,hash:o},r.state&&r.state.usr||null,r.state&&r.state.key||"default")}function a(n,r){return typeof r=="string"?r:js(r)}return KC(t,a,null,e)}function Ee(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function aa(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function HC(){return Math.random().toString(36).substring(2,10)}function h0(e,t){return{usr:e.state,key:e.key,idx:t}}function Kf(e,t,a=null,n){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?Cr(t):t,state:a,key:t&&t.key||n||HC()}}function js({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function Cr(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function KC(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",u=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function f(){o="POP";let w=d(),g=w==null?null:w-c;c=w,u&&u({action:o,location:b.location,delta:g})}function m(w,g){o="PUSH";let v=Kf(b.location,w,g);a&&a(v,w),c=d()+1;let x=h0(v,c),$=b.createHref(v);try{i.pushState(x,"",$)}catch(S){if(S instanceof DOMException&&S.name==="DataCloneError")throw S;r.location.assign($)}s&&u&&u({action:o,location:b.location,delta:1})}function p(w,g){o="REPLACE";let v=Kf(b.location,w,g);a&&a(v,w),c=d();let x=h0(v,c),$=b.createHref(v);i.replaceState(x,"",$),s&&u&&u({action:o,location:b.location,delta:0})}function y(w){return IC(w)}let b={get action(){return o},get location(){return e(r,i)},listen(w){if(u)throw new Error("A history only accepts one active listener");return r.addEventListener(p0,f),u=w,()=>{r.removeEventListener(p0,f),u=null}},createHref(w){return t(r,w)},createURL:y,encodeLocation(w){let g=y(w);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:m,replace:p,go(w){return i.go(w)}};return b}function IC(e,t=!1){let a="http://localhost";typeof window<"u"&&(a=window.location.origin!=="null"?window.location.origin:window.location.href),Ee(a,"No window.location.(origin|href) available to create URL");let n=typeof e=="string"?e:js(e);return n=n.replace(/ $/,"%20"),!t&&n.startsWith("//")&&(n=a+n),new URL(n,a)}var QC;QC=new WeakMap;function Gf(e,t,a="/"){return VC(e,t,a,!1)}function VC(e,t,a,n){let r=typeof t=="string"?Cr(t):t,s=qa(r.pathname||"/",a);if(s==null)return null;let i=x0(e);YC(i);let o=null;for(let u=0;o==null&&u<i.length;++u){let c=i3(s);o=r3(i[u],c,n)}return o}function GC(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function x0(e,t=[],a=[],n="",r=!1){let s=(i,o,u=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&u)return;Ee(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let f=mn([n,d.relativePath]),m=a.concat(d);i.children&&i.children.length>0&&(Ee(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${f}".`),x0(i.children,t,m,f,u)),!(i.path==null&&!i.index)&&t.push({path:f,score:a3(f,i.index),routesMeta:m})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let u of $0(i.path))s(i,o,!0,u)}),t}function $0(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=$0(n.join("/")),o=[];return o.push(...i.map(u=>u===""?s:[s,u].join("/"))),r&&o.push(...i),o.map(u=>e.startsWith("/")&&u===""?"/":u)}function YC(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:n3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var JC=/^:[\w-]+$/,XC=3,ZC=2,WC=1,e3=10,t3=-2,v0=e=>e==="*";function a3(e,t){let a=e.split("/"),n=a.length;return a.some(v0)&&(n+=t3),t&&(n+=ZC),a.filter(r=>!v0(r)).reduce((r,s)=>r+(JC.test(s)?XC:s===""?WC:e3),n)}function n3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function r3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let u=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",f=Do({path:u.relativePath,caseSensitive:u.caseSensitive,end:c},d),m=u.route;if(!f&&c&&a&&!n[n.length-1].route.index&&(f=Do({path:u.relativePath,caseSensitive:u.caseSensitive,end:!1},d)),!f)return null;Object.assign(r,f.params),i.push({params:r,pathname:mn([s,f.pathname]),pathnameBase:u3(mn([s,f.pathnameBase])),route:m}),f.pathnameBase!=="/"&&(s=mn([s,f.pathnameBase]))}return i}function Do(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=s3(e.path,e.caseSensitive,e.end),r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:f},m)=>{if(d==="*"){let y=o[m]||"";i=s.slice(0,s.length-y.length).replace(/(.)\/+$/,"$1")}let p=o[m];return f&&!p?c[d]=void 0:c[d]=(p||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function s3(e,t=!1,a=!0){aa(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,u)=>(n.push({paramName:o,isOptional:u!=null}),u?"/?([^\\/]+)?":"/([^\\/]+)")).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function i3(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return aa(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function qa(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}function w0(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?Cr(e):e;return{pathname:a?a.startsWith("/")?a:o3(a,t):t,search:c3(n),hash:d3(r)}}function o3(e,t){let a=t.replace(/\/+$/,"").split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function Bf(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function l3(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function Yf(e){let t=l3(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function Jf(e,t,a,n=!1){let r;typeof e=="string"?r=Cr(e):(r={...e},Ee(!r.pathname||!r.pathname.includes("?"),Bf("?","pathname","search",r)),Ee(!r.pathname||!r.pathname.includes("#"),Bf("#","pathname","hash",r)),Ee(!r.search||!r.search.includes("#"),Bf("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let f=t.length-1;if(!n&&i.startsWith("..")){let m=i.split("/");for(;m[0]==="..";)m.shift(),f-=1;r.pathname=m.join("/")}o=f>=0?t[f]:"/"}let u=w0(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!u.pathname.endsWith("/")&&(c||d)&&(u.pathname+="/"),u}var mn=e=>e.join("/").replace(/\/\/+/g,"/"),u3=e=>e.replace(/\/+$/,"").replace(/^\/*/,"/"),c3=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,d3=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;function S0(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}var N0=["POST","PUT","PATCH","DELETE"],uO=new Set(N0),m3=["GET",...N0],cO=new Set(m3);var dO=Symbol("ResetLoaderData");var Er=na.createContext(null);Er.displayName="DataRouter";var Ps=na.createContext(null);Ps.displayName="DataRouterState";var mO=na.createContext(!1);var Xf=na.createContext({isTransitioning:!1});Xf.displayName="ViewTransition";var _0=na.createContext(new Map);_0.displayName="Fetchers";var f3=na.createContext(null);f3.displayName="Await";var Kt=na.createContext(null);Kt.displayName="Navigation";var Fs=na.createContext(null);Fs.displayName="Location";var ra=na.createContext({outlet:null,matches:[],isDataRoute:!1});ra.displayName="Route";var Zf=na.createContext(null);Zf.displayName="RouteError";var If=!0;function k0(e,{relative:t}={}){Ee(Tr(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=V.useContext(Kt),{hash:r,pathname:s,search:i}=zs(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:mn([a,s])),n.createHref({pathname:o,search:i,hash:r})}function Tr(){return V.useContext(Fs)!=null}function je(){return Ee(Tr(),"useLocation() may be used only in the context of a <Router> component."),V.useContext(Fs).location}var R0="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function C0(e){V.useContext(Kt).static||V.useLayoutEffect(e)}function fe(){let{isDataRoute:e}=V.useContext(ra);return e?S3():p3()}function p3(){Ee(Tr(),"useNavigate() may be used only in the context of a <Router> component.");let e=V.useContext(Er),{basename:t,navigator:a}=V.useContext(Kt),{matches:n}=V.useContext(ra),{pathname:r}=je(),s=JSON.stringify(Yf(n)),i=V.useRef(!1);return C0(()=>{i.current=!0}),V.useCallback((u,c={})=>{if(aa(i.current,R0),!i.current)return;if(typeof u=="number"){a.go(u);return}let d=Jf(u,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:mn([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var E0=V.createContext(null);function Ba(){return V.useContext(E0)}function T0(e){let t=V.useContext(ra).outlet;return t&&V.createElement(E0.Provider,{value:e},t)}function it(){let{matches:e}=V.useContext(ra),t=e[e.length-1];return t?t.params:{}}function zs(e,{relative:t}={}){let{matches:a}=V.useContext(ra),{pathname:n}=je(),r=JSON.stringify(Yf(a));return V.useMemo(()=>Jf(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function A0(e,t){return D0(e,t)}function D0(e,t,a,n,r){Ee(Tr(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:s}=V.useContext(Kt),{matches:i}=V.useContext(ra),o=i[i.length-1],u=o?o.params:{},c=o?o.pathname:"/",d=o?o.pathnameBase:"/",f=o&&o.route;if(If){let v=f&&f.path||"";L0(c,!f||v.endsWith("*")||v.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${c}" (under <Route path="${v}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${v}"> to <Route path="${v==="/"?"*":`${v}/*`}">.`)}let m=je(),p;if(t){let v=typeof t=="string"?Cr(t):t;Ee(d==="/"||v.pathname?.startsWith(d),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${d}" but pathname "${v.pathname}" was given in the \`location\` prop.`),p=v}else p=m;let y=p.pathname||"/",b=y;if(d!=="/"){let v=d.replace(/^\//,"").split("/");b="/"+y.replace(/^\//,"").split("/").slice(v.length).join("/")}let w=Gf(e,{pathname:b});If&&(aa(f||w!=null,`No routes matched location "${p.pathname}${p.search}${p.hash}" `),aa(w==null||w[w.length-1].route.element!==void 0||w[w.length-1].route.Component!==void 0||w[w.length-1].route.lazy!==void 0,`Matched leaf route at location "${p.pathname}${p.search}${p.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let g=b3(w&&w.map(v=>Object.assign({},v,{params:Object.assign({},u,v.params),pathname:mn([d,s.encodeLocation?s.encodeLocation(v.pathname).pathname:v.pathname]),pathnameBase:v.pathnameBase==="/"?d:mn([d,s.encodeLocation?s.encodeLocation(v.pathnameBase).pathname:v.pathnameBase])})),i,a,n,r);return t&&g?V.createElement(Fs.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",...p},navigationType:"POP"}},g):g}function h3(){let e=O0(),t=S0(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return If&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=V.createElement(V.Fragment,null,V.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),V.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",V.createElement("code",{style:s},"ErrorBoundary")," or"," ",V.createElement("code",{style:s},"errorElement")," prop on your route."))),V.createElement(V.Fragment,null,V.createElement("h2",null,"Unexpected Application Error!"),V.createElement("h3",{style:{fontStyle:"italic"}},t),a?V.createElement("pre",{style:r},a):null,i)}var v3=V.createElement(h3,null),g3=class extends V.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.unstable_onError?this.props.unstable_onError(e,t):console.error("React Router caught the following error during render",e)}render(){return this.state.error!==void 0?V.createElement(ra.Provider,{value:this.props.routeContext},V.createElement(Zf.Provider,{value:this.state.error,children:this.props.component})):this.props.children}};function y3({routeContext:e,match:t,children:a}){let n=V.useContext(Er);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),V.createElement(ra.Provider,{value:e},a)}function b3(e,t=[],a=null,n=null,r=null){if(e==null){if(!a)return null;if(a.errors)e=a.matches;else if(t.length===0&&!a.initialized&&a.matches.length>0)e=a.matches;else return null}let s=e,i=a?.errors;if(i!=null){let c=s.findIndex(d=>d.route.id&&i?.[d.route.id]!==void 0);Ee(c>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(i).join(",")}`),s=s.slice(0,Math.min(s.length,c+1))}let o=!1,u=-1;if(a)for(let c=0;c<s.length;c++){let d=s[c];if((d.route.HydrateFallback||d.route.hydrateFallbackElement)&&(u=c),d.route.id){let{loaderData:f,errors:m}=a,p=d.route.loader&&!f.hasOwnProperty(d.route.id)&&(!m||m[d.route.id]===void 0);if(d.route.lazy||p){o=!0,u>=0?s=s.slice(0,u+1):s=[s[0]];break}}}return s.reduceRight((c,d,f)=>{let m,p=!1,y=null,b=null;a&&(m=i&&d.route.id?i[d.route.id]:void 0,y=d.route.errorElement||v3,o&&(u<0&&f===0?(L0("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),p=!0,b=null):u===f&&(p=!0,b=d.route.hydrateFallbackElement||null)));let w=t.concat(s.slice(0,f+1)),g=()=>{let v;return m?v=y:p?v=b:d.route.Component?v=V.createElement(d.route.Component,null):d.route.element?v=d.route.element:v=c,V.createElement(y3,{match:d,routeContext:{outlet:c,matches:w,isDataRoute:a!=null},children:v})};return a&&(d.route.ErrorBoundary||d.route.errorElement||f===0)?V.createElement(g3,{location:a.location,revalidation:a.revalidation,component:y,error:m,children:g(),routeContext:{outlet:null,matches:w,isDataRoute:!0},unstable_onError:n}):g()},null)}function Wf(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function x3(e){let t=V.useContext(Er);return Ee(t,Wf(e)),t}function ep(e){let t=V.useContext(Ps);return Ee(t,Wf(e)),t}function $3(e){let t=V.useContext(ra);return Ee(t,Wf(e)),t}function tp(e){let t=$3(e),a=t.matches[t.matches.length-1];return Ee(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function w3(){return tp("useRouteId")}function M0(){return ep("useNavigation").navigation}function ap(){let{matches:e,loaderData:t}=ep("useMatches");return V.useMemo(()=>e.map(a=>GC(a,t)),[e,t])}function O0(){let e=V.useContext(Zf),t=ep("useRouteError"),a=tp("useRouteError");return e!==void 0?e:t.errors?.[a]}function S3(){let{router:e}=x3("useNavigate"),t=tp("useNavigate"),a=V.useRef(!1);return C0(()=>{a.current=!0}),V.useCallback(async(r,s={})=>{aa(a.current,R0),a.current&&(typeof r=="number"?e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var g0={};function L0(e,t,a){!t&&!g0[e]&&(g0[e]=!0,aa(!1,a))}var fO=Te.memo(N3);function N3({routes:e,future:t,state:a,unstable_onError:n}){return D0(e,void 0,a,n,t)}function ot({to:e,replace:t,state:a,relative:n}){Ee(Tr(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=Te.useContext(Kt);aa(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=Te.useContext(ra),{pathname:i}=je(),o=fe(),u=Jf(e,Yf(s),i,n==="path"),c=JSON.stringify(u);return Te.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function np(e){return T0(e.context)}function pe(e){Ee(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function rp({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1}){Ee(!Tr(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let i=e.replace(/^\/*/,"/"),o=Te.useMemo(()=>({basename:i,navigator:r,static:s,future:{}}),[i,r,s]);typeof a=="string"&&(a=Cr(a));let{pathname:u="/",search:c="",hash:d="",state:f=null,key:m="default"}=a,p=Te.useMemo(()=>{let y=qa(u,i);return y==null?null:{location:{pathname:y,search:c,hash:d,state:f,key:m},navigationType:n}},[i,u,c,d,f,m,n]);return aa(p!=null,`<Router basename="${i}"> is not able to match the URL "${u}${c}${d}" because it does not start with the basename, so the <Router> won't render anything.`),p==null?null:Te.createElement(Kt.Provider,{value:o},Te.createElement(Fs.Provider,{children:t,value:p}))}function sp({children:e,location:t}){return A0(Ju(e),t)}function Ju(e,t=[]){let a=[];return Te.Children.forEach(e,(n,r)=>{if(!Te.isValidElement(n))return;let s=[...t,r];if(n.type===Te.Fragment){a.push.apply(a,Ju(n.props.children,s));return}Ee(n.type===pe,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),Ee(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=Ju(n.props.children,s)),a.push(i)}),a}var Gu="get",Yu="application/x-www-form-urlencoded";function Xu(e){return e!=null&&typeof e.tagName=="string"}function _3(e){return Xu(e)&&e.tagName.toLowerCase()==="button"}function k3(e){return Xu(e)&&e.tagName.toLowerCase()==="form"}function R3(e){return Xu(e)&&e.tagName.toLowerCase()==="input"}function C3(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function E3(e,t){return e.button===0&&(!t||t==="_self")&&!C3(e)}var Qu=null;function T3(){if(Qu===null)try{new FormData(document.createElement("form"),0),Qu=!1}catch{Qu=!0}return Qu}var A3=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function Hf(e){return e!=null&&!A3.has(e)?(aa(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${Yu}"`),null):e}function D3(e,t){let a,n,r,s,i;if(k3(e)){let o=e.getAttribute("action");n=o?qa(o,t):null,a=e.getAttribute("method")||Gu,r=Hf(e.getAttribute("enctype"))||Yu,s=new FormData(e)}else if(_3(e)||R3(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let u=e.getAttribute("formaction")||o.getAttribute("action");if(n=u?qa(u,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||Gu,r=Hf(e.getAttribute("formenctype"))||Hf(o.getAttribute("enctype"))||Yu,s=new FormData(o,e),!T3()){let{name:c,type:d,value:f}=e;if(d==="image"){let m=c?`${c}.`:"";s.append(`${m}x`,"0"),s.append(`${m}y`,"0")}else c&&s.append(c,f)}}else{if(Xu(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=Gu,n=null,r=Yu,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var pO=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");function op(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var M3=Symbol("SingleFetchRedirect");function O3(e,t,a){let n=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return n.pathname==="/"?n.pathname=`_root.${a}`:t&&qa(n.pathname,t)==="/"?n.pathname=`${t.replace(/\/$/,"")}/_root.${a}`:n.pathname=`${n.pathname.replace(/\/$/,"")}.${a}`,n}async function L3(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function U3(e){return e!=null&&typeof e.page=="string"}function j3(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function P3(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await L3(s,a);return i.links?i.links():[]}return[]}));return B3(n.flat(1).filter(j3).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function y0(e,t,a,n,r,s){let i=(u,c)=>a[c]?u.route.id!==a[c].route.id:!0,o=(u,c)=>a[c].pathname!==u.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==u.params["*"];return s==="assets"?t.filter((u,c)=>i(u,c)||o(u,c)):s==="data"?t.filter((u,c)=>{let d=n.routes[u.route.id];if(!d||!d.hasLoader)return!1;if(i(u,c)||o(u,c))return!0;if(u.route.shouldRevalidate){let f=u.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:u.params,defaultShouldRevalidate:!0});if(typeof f=="boolean")return f}return!0}):[]}function F3(e,t,{includeHydrateFallback:a}={}){return z3(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function z3(e){return[...new Set(e)]}function q3(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function B3(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!U3(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(q3(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function j0(){let e=me.useContext(Er);return op(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function Q3(){let e=me.useContext(Ps);return op(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var Mo=me.createContext(void 0);Mo.displayName="FrameworkContext";function P0(){let e=me.useContext(Mo);return op(e,"You must render this element inside a <HydratedRouter> element"),e}function V3(e,t){let a=me.useContext(Mo),[n,r]=me.useState(!1),[s,i]=me.useState(!1),{onFocus:o,onBlur:u,onMouseEnter:c,onMouseLeave:d,onTouchStart:f}=t,m=me.useRef(null);me.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let b=g=>{g.forEach(v=>{i(v.isIntersecting)})},w=new IntersectionObserver(b,{threshold:.5});return m.current&&w.observe(m.current),()=>{w.disconnect()}}},[e]),me.useEffect(()=>{if(n){let b=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(b)}}},[n]);let p=()=>{r(!0)},y=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,m,{}]:[s,m,{onFocus:Ao(o,p),onBlur:Ao(u,y),onMouseEnter:Ao(c,p),onMouseLeave:Ao(d,y),onTouchStart:Ao(f,p)}]:[!1,m,{}]}function Ao(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function F0({page:e,...t}){let{router:a}=j0(),n=me.useMemo(()=>Gf(a.routes,e,a.basename),[a.routes,e,a.basename]);return n?me.createElement(Y3,{page:e,matches:n,...t}):null}function G3(e){let{manifest:t,routeModules:a}=P0(),[n,r]=me.useState([]);return me.useEffect(()=>{let s=!1;return P3(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function Y3({page:e,matches:t,...a}){let n=je(),{manifest:r,routeModules:s}=P0(),{basename:i}=j0(),{loaderData:o,matches:u}=Q3(),c=me.useMemo(()=>y0(e,t,u,r,n,"data"),[e,t,u,r,n]),d=me.useMemo(()=>y0(e,t,u,r,n,"assets"),[e,t,u,r,n]),f=me.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let y=new Set,b=!1;if(t.forEach(g=>{let v=r.routes[g.route.id];!v||!v.hasLoader||(!c.some(x=>x.route.id===g.route.id)&&g.route.id in o&&s[g.route.id]?.shouldRevalidate||v.hasClientLoader?b=!0:y.add(g.route.id))}),y.size===0)return[];let w=O3(e,i,"data");return b&&y.size>0&&w.searchParams.set("_routes",t.filter(g=>y.has(g.route.id)).map(g=>g.route.id).join(",")),[w.pathname+w.search]},[i,o,n,r,c,t,e,s]),m=me.useMemo(()=>F3(d,r),[d,r]),p=G3(d);return me.createElement(me.Fragment,null,f.map(y=>me.createElement("link",{key:y,rel:"prefetch",as:"fetch",href:y,...a})),m.map(y=>me.createElement("link",{key:y,rel:"modulepreload",href:y,...a})),p.map(({key:y,link:b})=>me.createElement("link",{key:y,nonce:a.nonce,...b})))}function J3(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var z0=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{z0&&(window.__reactRouterVersion="7.9.1")}catch{}function lp({basename:e,children:t,window:a}){let n=X.useRef();n.current==null&&(n.current=b0({window:a,v5Compat:!0}));let r=n.current,[s,i]=X.useState({action:r.action,location:r.location}),o=X.useCallback(u=>{X.startTransition(()=>i(u))},[i]);return X.useLayoutEffect(()=>r.listen(o),[r,o]),X.createElement(rp,{basename:e,children:t,location:s.location,navigationType:s.action,navigator:r})}function q0({basename:e,children:t,history:a}){let[n,r]=X.useState({action:a.action,location:a.location}),s=X.useCallback(i=>{X.startTransition(()=>r(i))},[r]);return X.useLayoutEffect(()=>a.listen(s),[a,s]),X.createElement(rp,{basename:e,children:t,location:n.location,navigationType:n.action,navigator:a})}q0.displayName="unstable_HistoryRouter";var B0=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i,Ar=X.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,state:o,target:u,to:c,preventScrollReset:d,viewTransition:f,...m},p){let{basename:y}=X.useContext(Kt),b=typeof c=="string"&&B0.test(c),w,g=!1;if(typeof c=="string"&&b&&(w=c,z0))try{let U=new URL(window.location.href),O=c.startsWith("//")?new URL(U.protocol+c):new URL(c),B=qa(O.pathname,y);O.origin===U.origin&&B!=null?c=B+O.search+O.hash:g=!0}catch{aa(!1,`<Link to="${c}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}let v=k0(c,{relative:r}),[x,$,S]=V3(n,m),R=Q0(c,{replace:i,state:o,target:u,preventScrollReset:d,relative:r,viewTransition:f});function _(U){t&&t(U),U.defaultPrevented||R(U)}let C=X.createElement("a",{...m,...S,href:w||v,onClick:g||s?t:_,ref:J3(p,$),target:u,"data-discover":!b&&a==="render"?"true":void 0});return x&&!b?X.createElement(X.Fragment,null,C,X.createElement(F0,{page:v})):C});Ar.displayName="Link";var Xn=X.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:u,...c},d){let f=zs(i,{relative:c.relative}),m=je(),p=X.useContext(Ps),{navigator:y,basename:b}=X.useContext(Kt),w=p!=null&&J0(f)&&o===!0,g=y.encodeLocation?y.encodeLocation(f).pathname:f.pathname,v=m.pathname,x=p&&p.navigation&&p.navigation.location?p.navigation.location.pathname:null;a||(v=v.toLowerCase(),x=x?x.toLowerCase():null,g=g.toLowerCase()),x&&b&&(x=qa(x,b)||x);let $=g!=="/"&&g.endsWith("/")?g.length-1:g.length,S=v===g||!r&&v.startsWith(g)&&v.charAt($)==="/",R=x!=null&&(x===g||!r&&x.startsWith(g)&&x.charAt(g.length)==="/"),_={isActive:S,isPending:R,isTransitioning:w},C=S?t:void 0,U;typeof n=="function"?U=n(_):U=[n,S?"active":null,R?"pending":null,w?"transitioning":null].filter(Boolean).join(" ");let O=typeof s=="function"?s(_):s;return X.createElement(Ar,{...c,"aria-current":C,className:U,ref:d,style:O,to:i,viewTransition:o},typeof u=="function"?u(_):u)});Xn.displayName="NavLink";var H0=X.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=Gu,action:o,onSubmit:u,relative:c,preventScrollReset:d,viewTransition:f,...m},p)=>{let y=V0(),b=G0(o,{relative:c}),w=i.toLowerCase()==="get"?"get":"post",g=typeof o=="string"&&B0.test(o);return X.createElement("form",{ref:p,method:w,action:b,onSubmit:n?u:x=>{if(u&&u(x),x.defaultPrevented)return;x.preventDefault();let $=x.nativeEvent.submitter,S=$?.getAttribute("formmethod")||i;y($||x.currentTarget,{fetcherKey:t,method:S,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:f})},...m,"data-discover":!g&&e==="render"?"true":void 0})});H0.displayName="Form";function K0({getKey:e,storageKey:t,...a}){let n=X.useContext(Mo),{basename:r}=X.useContext(Kt),s=je(),i=ap();Y0({getKey:e,storageKey:t});let o=X.useMemo(()=>{if(!n||!e)return null;let c=Vf(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let u=((c,d)=>{if(!window.history.state||!window.history.state.key){let f=Math.random().toString(32).slice(2);window.history.replaceState({key:f},"")}try{let m=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof m=="number"&&window.scrollTo(0,m)}catch(f){console.error(f),sessionStorage.removeItem(c)}}).toString();return X.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${u})(${JSON.stringify(t||Qf)}, ${JSON.stringify(o)})`}})}K0.displayName="ScrollRestoration";function I0(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function up(e){let t=X.useContext(Er);return Ee(t,I0(e)),t}function X3(e){let t=X.useContext(Ps);return Ee(t,I0(e)),t}function Q0(e,{target:t,replace:a,state:n,preventScrollReset:r,relative:s,viewTransition:i}={}){let o=fe(),u=je(),c=zs(e,{relative:s});return X.useCallback(d=>{if(E3(d,t)){d.preventDefault();let f=a!==void 0?a:js(u)===js(c);o(e,{replace:f,state:n,preventScrollReset:r,relative:s,viewTransition:i})}},[u,o,c,a,n,t,e,r,s,i])}var Z3=0,W3=()=>`__${String(++Z3)}__`;function V0(){let{router:e}=up("useSubmit"),{basename:t}=X.useContext(Kt),a=w3();return X.useCallback(async(n,r={})=>{let{action:s,method:i,encType:o,formData:u,body:c}=D3(n,t);if(r.navigate===!1){let d=r.fetcherKey||W3();await e.fetch(d,a,r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,flushSync:r.flushSync})}else await e.navigate(r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,replace:r.replace,state:r.state,fromRouteId:a,flushSync:r.flushSync,viewTransition:r.viewTransition})},[e,t,a])}function G0(e,{relative:t}={}){let{basename:a}=X.useContext(Kt),n=X.useContext(ra);Ee(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...zs(e||".",{relative:t})},i=je();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),u=o.getAll("index");if(u.some(d=>d==="")){o.delete("index"),u.filter(f=>f).forEach(f=>o.append("index",f));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:mn([a,s.pathname])),js(s)}var Qf="react-router-scroll-positions",Vu={};function Vf(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:qa(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function Y0({getKey:e,storageKey:t}={}){let{router:a}=up("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=X3("useScrollRestoration"),{basename:s}=X.useContext(Kt),i=je(),o=ap(),u=M0();X.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),e4(X.useCallback(()=>{if(u.state==="idle"){let c=Vf(i,o,s,e);Vu[c]=window.scrollY}try{sessionStorage.setItem(t||Qf,JSON.stringify(Vu))}catch(c){aa(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[u.state,e,s,i,o,t])),typeof document<"u"&&(X.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||Qf);c&&(Vu=JSON.parse(c))}catch{}},[t]),X.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(Vu,()=>window.scrollY,e?(d,f)=>Vf(d,f,s,e):void 0);return()=>c&&c()},[a,s,e]),X.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{aa(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function e4(e,t){let{capture:a}=t||{};X.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function J0(e,{relative:t}={}){let a=X.useContext(Xf);Ee(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=up("useViewTransitionState"),r=zs(e,{relative:t});if(!a.isTransitioning)return!1;let s=qa(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=qa(a.nextLocation.pathname,n)||a.nextLocation.pathname;return Do(r.pathname,i)!=null||Do(r.pathname,s)!=null}var At=new ld({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var cp="ironclaw_token",yt="/api/webchat/v2",Dr=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function ya(){return sessionStorage.getItem(cp)||""}function qs(e){e?sessionStorage.setItem(cp,e):sessionStorage.removeItem(cp)}function Zu(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function W0(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function Z0(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function ex({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=Z0(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=Z0(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function Z(e,t={}){let a=ya(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await W0(r);throw new Dr(ex({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function Wu(){return Z(`${yt}/session`)}function ec({clientActionId:e,requestedThreadId:t}={}){let a={client_action_id:e||Zu()};return t&&(a.requested_thread_id=t),Z(`${yt}/threads`,{method:"POST",body:JSON.stringify(a)})}function tx({limit:e,cursor:t}={}){let a=new URL(`${yt}/threads`,window.location.origin);return e!=null&&a.searchParams.set("limit",String(e)),t&&a.searchParams.set("cursor",t),Z(a.pathname+a.search)}function ax({threadId:e}={}){return e?Z(`${yt}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function nx(e){return`${yt}/threads/${encodeURIComponent(e)}/files`}function rx({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${nx(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),Z(a.pathname+a.search)}function sx({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${nx(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function ix({limit:e,runLimit:t}={}){let a=new URLSearchParams;e!=null&&a.set("limit",String(e)),t!=null&&a.set("run_limit",String(t));let n=a.toString();return Z(`${yt}/automations${n?`?${n}`:""}`)}function ox(){return Z(`${yt}/outbound/preferences`)}function lx(){return Z(`${yt}/outbound/targets`)}function ux({finalReplyTargetId:e}={}){return Z(`${yt}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function cx({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:u,source:c}={}){let d=new URL(`${yt}/operator/logs`,window.location.origin);return e!=null&&d.searchParams.set("limit",String(e)),t&&d.searchParams.set("cursor",t),a&&d.searchParams.set("level",a),n&&d.searchParams.set("target",n),r&&d.searchParams.set("thread_id",r),s&&d.searchParams.set("run_id",s),i&&d.searchParams.set("turn_id",i),o&&d.searchParams.set("tool_call_id",o),u&&d.searchParams.set("tool_name",u),c&&d.searchParams.set("source",c),Z(d.pathname+d.search)}function dx({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||Zu(),content:t};return a.length>0&&(r.attachments=a),Z(`${yt}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function mx({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${yt}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),Z(n.pathname+n.search)}function fx({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${yt}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function Oo(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new Dr("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=ya(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await W0(r);throw new Dr(ex({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function dp(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function px(e){return dp(await Oo(e))}function hx({threadId:e,afterCursor:t}={}){let a=new URL(`${yt}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=ya();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function vx({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||Zu()};return a&&(r.reason=a),Z(`${yt}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function mp({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let u={client_action_id:i||Zu(),resolution:n};return r!=null&&(u.always=r),s&&(u.credential_ref=s),Z(`${yt}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(u)})}function gx({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return Z("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function yx(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),Z(`${yt}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function Bs(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function bx(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function xx(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new Dr("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new Dr("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function $x(){let e=ya();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var tc="anon",wx=tc;function Sx(e){wx=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:tc}function Nt(){return wx}var Nx="ironclaw:v2-thread-pins:",fp=new Set,fn=new Set,pp=null;function hp(){return`${Nx}${Nt()}`}function t4(){try{let e=window.localStorage.getItem(hp());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function a4(){try{fn.size===0?window.localStorage.removeItem(hp()):window.localStorage.setItem(hp(),JSON.stringify([...fn]))}catch{}}function _x(){let e=Nt();if(e!==pp){fn.clear();for(let t of t4())fn.add(t);pp=e}}function kx(){return new Set(fn)}function Rx(){let e=kx();for(let t of fp)try{t(e)}catch{}}function Cx(e){e&&(_x(),fn.has(e)?fn.delete(e):fn.add(e),a4(),Rx())}function Ex(){return _x(),kx()}function Tx(e){return fp.add(e),()=>{fp.delete(e)}}function Ax(){fn.clear(),pp=Nt();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(Nx)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}Rx()}var n4=0,Mr={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function vp(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function Dx(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":r4(t)?"text":"download"}function r4(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function Lo(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function s4(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function i4(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function o4(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function Mx(e,{limits:t,existing:a=[],t:n}){let r=t||Mr,s=[],i=[],o=a.length,u=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!s4(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:Lo(r.maxFileBytes)}));continue}if(u+c.size>r.maxTotalBytes){let b=n("chat.attachmentTotalTooLarge",{max:Lo(r.maxTotalBytes)});i.includes(b)||i.push(b);continue}let d;try{d=await i4(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:f,base64:m}=o4(d,c.type),p=f||"application/octet-stream",y=vp(p);s.push({id:`staged-${n4++}`,filename:c.name||"attachment",mimeType:p,kind:y,sizeBytes:c.size,sizeLabel:Lo(c.size),dataBase64:m,previewUrl:y==="image"?d:null}),o+=1,u+=c.size}return{staged:s,errors:i}}function Ox(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function Lx(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}function l4(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||vp(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?fx({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?Lo(n.size_bytes):"",preview_url:null,fetch_url:s}})}function jx(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let c=m4(s);if(!c)continue;let d=`tool-${c.invocationId}`;if(n.has(d))continue;n.add(d),r.push({id:d,role:"tool_activity",...c,timestamp:Ux(s)||c.updatedAt||null,sequence:s.sequence,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=d4(s),u=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy");r.push({id:i,role:o,content:s.content||"",attachments:l4(s,a),timestamp:Ux(s),kind:s.kind,status:u?"error":s.status,...u&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:c4(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=u4(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function u4(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function c4(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function d4(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function Ux(e){return e.received_at||e.created_at||null}function m4(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:gp(t)}function gp(e){let t=e.status==="failed"||e.status==="killed";return{invocationId:e.invocation_id,callId:e.invocation_id,toolName:e.title||e.capability_id||"tool",toolStatus:Fx(e.status),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(e.output_summary||e.output_preview||e.result_ref)||null,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null}}function yp(e){return{invocationId:e.invocation_id,callId:e.invocation_id,toolName:e.capability_id||"tool",toolStatus:Fx(e.status),toolDetail:null,toolParameters:null,toolResultPreview:null,toolError:e.error_kind||null,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null}}function Px(e){return e==="success"||e==="error"}function Fx(e){switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}var f4=50,pn=new Map,p4=30;function bp(e,t){for(pn.delete(e),pn.set(e,t);pn.size>p4;){let a=pn.keys().next().value;pn.delete(a)}}function ac(e){return`${Nt()}:${e}`}function qx(){pn.clear()}function Bx(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?pn.get(ac(e)):null,[s,i]=h.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),o=h.default.useRef(new Set),u=h.default.useRef(e);u.current=e;let c=h.default.useCallback(async(d,f={})=>{let{preserveClientOnly:m=!1}=f;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(o.current.has(e))return;o.current.add(e);let p=Nt(),y=ac(e);i(b=>({...b,isLoading:!0}));try{let b=await mx({threadId:e,limit:f4,cursor:d});if(Nt()!==p)return;let w=d?[]:a?.()||[],g=jx(b.messages||[],w,e),v=b.next_cursor||null;if(d||n?.([]),!d){let x=pn.get(y)?.messages||[],$=m?zx(g,x):g;bp(y,{messages:$,nextCursor:v})}i(x=>{if(u.current!==e)return x;let $;return d?$=h4(g,x.messages):m?$=zx(g,x.messages):$=g,d&&bp(y,{messages:$,nextCursor:v}),{messages:$,nextCursor:v,isLoading:!1,loadError:null}})}catch(b){if(console.error("Failed to load timeline:",b),Nt()!==p)return;i(w=>u.current===e?{...w,isLoading:!1,loadError:"Failed to load conversation history."}:w)}finally{o.current.delete(e)}},[e,a,n]);return h.default.useEffect(()=>{let d=e?pn.get(ac(e)):null;i({messages:d?.messages||[],nextCursor:d?.nextCursor||null,isLoading:!!e&&!d,loadError:null}),e&&c()},[e,c]),{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:c,setMessages:d=>i(f=>{let m=typeof d=="function"?d(f.messages):d;return e&&bp(ac(e),{messages:m,nextCursor:f.nextCursor}),{...f,messages:m}})}}function h4(e,t){let a=new Set(t.map(n=>n.id));return[...e.filter(n=>!a.has(n.id)),...t]}function zx(e,t){let a=new Set(e.map(r=>r?.id).filter(Boolean)),n=t.filter(r=>r&&typeof r.id=="string"&&!a.has(r.id)&&r.id.startsWith("err-"));return[...e,...n]}var jo="__new__",Hx="ironclaw:v2-draft:";function Hs(e){return`${Hx}${Nt()}:${e||jo}`}function xp(e){try{return window.localStorage.getItem(Hs(e))||""}catch{return""}}function $p(e,t){try{t?window.localStorage.setItem(Hs(e),t):window.localStorage.removeItem(Hs(e))}catch{}}function Kx(e){$p(e,"")}var Uo=new Map;function wp(e){return Uo.get(Hs(e))||[]}function Ix(e,t){let a=Hs(e);t&&t.length>0?Uo.set(a,t):Uo.delete(a)}function Qx(e){Uo.delete(Hs(e))}function Vx(){Uo.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(Hx)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function v4(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function g4(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function y4(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=v4(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?g4(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),ya()?"":(qs(n),n)}function b4(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var x4={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function $4(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),x4[t]||"Could not complete sign-in. Please try again."):""}function Gx(){let[e,t]=h.default.useState(()=>y4()||ya()),[a,n]=h.default.useState(()=>$4()),[r]=h.default.useState(()=>b4()),[s,i]=h.default.useState(null),[o,u]=h.default.useState(()=>!!(r&&!ya())),[c,d]=h.default.useState(()=>!!ya());h.default.useEffect(()=>{if(!r||ya()){u(!1);return}let y=!1;return xx(r).then(b=>{y||(qs(b),d(!0),t(b),i(null),n(""),u(!1),At.clear())}).catch(()=>{y||(n("Could not complete sign-in. Please try again."),u(!1))}),()=>{y=!0}},[r]),h.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let y=!1;return d(!0),Wu().then(b=>{y||(i(b),d(!1))}).catch(b=>{y||(i(null),d(!1),(b?.status===401||b?.status===403)&&(qs(""),t(""),n("Your session expired. Please sign in again."),At.clear()))}),()=>{y=!0}},[e,o]),Sx(s);let f=h.default.useRef(null);h.default.useEffect(()=>{let y=Nt();f.current&&f.current!==tc&&f.current!==y&&(qx(),Vx(),Ax()),f.current=y},[s]);let m=h.default.useCallback(y=>{qs(y),d(!!y),t(y),i(null),n(""),At.clear()},[]),p=h.default.useCallback(()=>{$x().catch(()=>{}),qs(""),d(!1),t(""),i(null),n(""),At.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,signIn:m,signOut:p}}var Or="/chat",Po=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace",hidden:!0},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var w4=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],S4=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],N4=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],nc={settings:w4,extensions:S4,admin:N4};var Yx="ironclaw:v2-theme";function _4(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(Yx);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function rc(){let[e,t]=h.default.useState(_4);h.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(Yx,e)}catch{}},[e]);let a=h.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function Jx(e){return z({enabled:!!e,queryKey:["gateway-status",e],queryFn:Bs,refetchInterval:3e4})}function Xx(){return Promise.resolve({settings:{},todo:!0})}function Zx(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 settings endpoint"})}function Wx(e){return Promise.resolve({success:!1,message:"TODO: requires v2 settings endpoint"})}function sc(){return Z("/api/webchat/v2/llm/providers")}function e$(e){return Z("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function t$(e){return Z(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function Fo(e){return Z("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function a$(e){return Z("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function n$(e){return Z("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function r$(e){return Z("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function s$(e){return Z("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function i$(){return Z("/api/webchat/v2/llm/codex/login",{method:"POST"})}function o$(){return Promise.resolve({tools:[],todo:!0})}function l$(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 tools endpoint"})}function u$(){return Z("/api/webchat/v2/extensions")}function c$(){return Z("/api/webchat/v2/extensions/registry")}function d$(){return Z("/api/webchat/v2/skills")}function m$(e){return Z(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function f$(e){return Z("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function p$(e,t){return Z(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function h$(e){return Z(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function v$(){return Z("/api/webchat/v2/traces/credit")}function g$(e){return Z(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function y$(){return Promise.resolve({users:[],todo:!0})}function b$(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function x$(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var Sp="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",Np=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function zo(e){return Np.find(t=>t.value===e)?.label||e}function Ks(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function $$(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function ic(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function w$(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function Lr(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Sp||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?Ks(e,t).trim().length>0:!0:!1}function k4(e,t,a){return e.id===a?"active":Lr(e,t)?"ready":"setup"}function S$(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=k4(r,t,a);n[s]&&n[s].push(r)}return n}function oc(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Sp||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!Ks(e,t).trim()?"base_url":"ok"}function _p(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===Sp&&(i.api_key=void 0),i}function N$(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function _$(e){return/^[a-z0-9_-]+$/.test(e)}function k$(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var R4=Object.freeze({});function Is({settings:e,gatewayStatus:t,enabled:a=!0}){let n=Y(),r=z({queryKey:["llm-providers"],queryFn:sc,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=R4,u=(s.providers||[]).map($=>({...$,name:$.description,has_api_key:$.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,f=d||"nearai",m=s.active?.model||t?.llm_model||"",p=u.filter($=>$.builtin),y=u.filter($=>!$.builtin),b=[...u].sort(($,S)=>$.id===d?-1:S.id===d?1:($.name||$.id).localeCompare(S.name||S.id)),w=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=I({mutationFn:async $=>{if(!Lr($,o)){let R=oc($,o);throw new Error(R==="base_url"?"base_url":"api_key")}let S=ic($,o);if(!S)throw new Error("model");return await Fo({provider_id:$.id,model:S}),$},onSuccess:w}),v=I({mutationFn:async({provider:$,form:S,apiKey:R,editingProvider:_})=>{let C=!!$?.builtin,O={id:(C?$.id:S.id.trim()).trim(),name:C?$.name||$.id:S.name.trim(),adapter:C?$.adapter:S.adapter,base_url:S.baseUrl.trim()||$?.base_url||"",default_model:S.model.trim()||void 0};return R.trim()&&(O.api_key=R.trim()),(_||$)?.id===f&&O.default_model&&(O.set_active=!0,O.model=O.default_model),await e$(O),O},onSuccess:w}),x=I({mutationFn:async $=>(await t$($.id),$),onSuccess:w});return{providers:b,builtinProviders:p,customProviders:y,builtinOverrides:o,activeProviderId:d,selectedModel:m,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:$=>g.mutateAsync($),saveCustomProvider:$=>v.mutateAsync($),saveBuiltinProvider:$=>v.mutateAsync($),deleteCustomProvider:$=>x.mutateAsync($),testConnection:a$,listModels:n$,isBusy:g.isPending||v.isPending||x.isPending}}function R$({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}function C$({onNewChat:e}={}){let t=fe(),[a,n]=h.default.useState(!1),r=h.default.useCallback(()=>n(!1),[]),s=h.default.useCallback(()=>n(u=>!u),[]),i=h.default.useCallback(async()=>{let u=await e?.(),c=typeof u=="string"&&u.length>0?u:null;t(c?`/chat/${c}`:"/chat"),r()},[t,r,e]),o=h.default.useCallback(u=>{t(`/chat/${u}`),r()},[t,r]);return{open:a,close:r,toggle:s,newChat:i,selectThread:o}}var kp=new Set,C4=0;function Qs(e,t={}){let a={id:++C4,message:e,tone:t.tone||"info",duration:t.duration??2600};return kp.forEach(n=>n(a)),a.id}function E$(e){return kp.add(e),()=>kp.delete(e)}function E4(e){return e?.status===409&&e?.payload?.kind==="busy"}function T$(e,t){return E4(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function A$(){let e=z({queryKey:["threads"],queryFn:()=>tx({})}),[t,a]=h.default.useState(null),[n,r]=h.default.useState(!1),s=h.default.useRef(null),i=h.default.useCallback(async()=>{if(s.current)return s.current;r(!0);let c=(async()=>{try{let d=await ec();At.invalidateQueries({queryKey:["threads"]});let f=d?.thread?.thread_id;return f&&a(f),f}finally{r(!1),s.current=null}})();return s.current=c,c},[]),o=h.default.useCallback(async c=>{await ax({threadId:c}),t===c&&a(null),At.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:h.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var D$={attach:l`<path
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
      ${D$[e]||D$.spark}
    </svg>
  `}function G(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=G(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function M$(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function T4(e){return M$(e).trim().charAt(0).toUpperCase()||"I"}function A4(){let[e,t]=h.default.useState(!1),a=h.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function O$({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=k(),s=A4(),i=M$(a),o=a?.email||a?.role||r("common.gatewaySession");return l`
    <div
      className="relative flex items-center gap-2 border-t border-[var(--v2-panel-border)] px-3 py-3"
    >
      ${s.open&&l`
        <div
          className=${G("absolute bottom-full left-3 right-3 mb-2 rounded-[10px] border p-3 shadow-lg","border-[var(--v2-panel-border)] bg-[var(--v2-surface)]")}
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
            />`:l`<span className="place-self-center">${T4(a)}</span>`}
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
  `}var L$={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",settings:"settings",admin:"shield"},D4=Po.filter(e=>e.id!=="chat"&&!e.hidden);function M4({route:e,label:t,onNavigate:a}){return l`
    <${Xn}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>G("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${D} name=${L$[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function O4({route:e,label:t,subRoutes:a,onNavigate:n}){let r=k(),s=je(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return l`
    <div className="flex flex-col">
      <${Xn}
        to=${o}
        onClick=${n}
        className=${()=>G("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${D}
          name=${L$[e.id]||"bolt"}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1 truncate">${t}</span>
        <${D}
          name="chevron"
          className=${G("h-3.5 w-3.5 shrink-0 transition-transform duration-150",i&&"rotate-180")}
        />
      <//>

      ${i&&l`
        <div className="mt-0.5 flex flex-col gap-0.5 pl-3">
          ${a.map(u=>l`
              <${Xn}
                key=${u.id}
                to=${e.path+"/"+u.id}
                onClick=${n}
                className=${({isActive:c})=>G("flex items-center gap-2.5 rounded-[8px] py-1.5 pl-7 pr-3 text-[12px] font-medium",c?"text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
              >
                <${D} name=${u.icon} className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate">${r(u.labelKey)}</span>
              <//>
            `)}
        </div>
      `}
    </div>
  `}function U$({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=k(),s=h.default.useMemo(()=>D4.filter(i=>a||i.id!=="admin"),[a]);return l`
    <div className="flex flex-col px-3 py-2">
      <button
        onClick=${e}
        disabled=${t}
        className=${G("flex items-center gap-2.5 rounded-[10px] px-3 py-2","border border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]","bg-[var(--v2-accent-soft)] text-[13px] font-medium text-[var(--v2-accent-text)]","hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)] disabled:opacity-50")}
      >
        <${D} name="plus" className="h-4 w-4 shrink-0" />
        <span>${r(t?"chat.creating":"chat.newThread")}</span>
      </button>

      <nav className="mt-2 flex flex-col gap-1">
        ${s.map(i=>{let o=(nc[i.id]||[]).filter(u=>a||!(i.id==="settings"&&["users","inference"].includes(u.id)));return o.length>0?l`
              <${O4}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:l`
            <${M4}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var hn=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),qo=new Set([hn.NEEDS_ATTENTION,hn.FAILED]),Rp="ironclaw:v2-thread-attention",Cp=new Set,Vs=new Map;function L4(){try{let e=window.localStorage.getItem(Rp);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&qo.has(a[1])):[]}catch{return[]}}function j$(){let e=[];for(let[t,a]of Vs)qo.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(Rp):window.localStorage.setItem(Rp,JSON.stringify(e))}catch{}}for(let[e,t]of L4())Vs.set(e,t);function F$(){return new Map(Vs)}function P$(){let e=F$();for(let t of Cp)try{t(e)}catch{}}function lc(e,t){if(!e)return;let a=Vs.get(e);if(t==null){if(!Vs.delete(e))return;qo.has(a)&&j$(),P$();return}a!==t&&(Vs.set(e,t),(qo.has(t)||qo.has(a))&&j$(),P$())}function z$(e){lc(e,null)}function U4(){return F$()}function j4(e){return Cp.add(e),()=>{Cp.delete(e)}}function q$(){let[e,t]=h.default.useState(U4);return h.default.useEffect(()=>j4(t),[]),e}function uc(e){return e.updated_at||e.created_at||null}function Ep(e,t){let a=uc(e)||"",n=uc(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function B$(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function H$(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function P4(){let[e,t]=h.default.useState(Ex);return h.default.useEffect(()=>Tx(t),[]),e}var F4=Object.freeze({[hn.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-[var(--v2-warning-text)]"},[hn.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[hn.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function z4(e){return e&&F4[e]||null}function q4({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=k(),o=uc(e),u=B$(o),c=H$(o),d=h.default.useCallback(m=>{m.preventDefault(),m.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(p=>{window.alert(p?.message||"Unable to delete chat")})},[s,e.id]),f=h.default.useCallback(m=>{m.preventDefault(),m.stopPropagation(),Cx(e.id)},[e.id]);return l`
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
          ${n&&l`<span
            aria-label=${n.label}
            className=${G("h-1.5 w-1.5 shrink-0 rounded-full",n.dotClass)}
          />`}
        </div>
        ${(n||u)&&l`<span
          className=${G("block truncate text-[11px]",n?n.textClass:"text-[var(--v2-text-faint)]")}
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
        className=${G("my-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] transition",a?"text-[var(--v2-accent-text)]":"opacity-0 text-[var(--v2-text-faint)] group-hover:opacity-100 focus:opacity-100","hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-accent-text)]")}
      >
        <${D} name="pin" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>
      ${s&&l`<button
        type="button"
        onClick=${d}
        title=${i("common.deleteChat")}
        aria-label=${i("common.deleteChat")}
        className=${G("my-1 mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px]","opacity-0 transition group-hover:opacity-100 focus:opacity-100","text-[var(--v2-text-faint)] hover:bg-[var(--v2-danger-soft)] hover:text-[var(--v2-danger-text)]")}
      >
        <${D} name="trash" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>`}
    </div>
  `}function K$({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:l`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>l`
          <${q4}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${z4(n.get(o.id))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function I$({threads:e,activeThreadId:t,onSelect:a,onDelete:n}){let[r,s]=h.default.useState(!1),[i,o]=h.default.useState(""),u=q$(),c=P4(),d=k(),{pinned:f,recent:m,totalMatches:p}=h.default.useMemo(()=>{let y=i.trim().toLowerCase(),b=y?e.filter(v=>(v.title||v.id||"").toLowerCase().includes(y)):e,w=[],g=[];for(let v of b)c.has(v.id)?w.push(v):g.push(v);return w.sort(Ep),g.sort(Ep),{pinned:w,recent:g,totalMatches:w.length+g.length}},[e,i,c]);return l`
    <div className="flex min-h-0 flex-1 flex-col px-2">
      <button
        onClick=${()=>s(y=>!y)}
        className="flex w-full items-center gap-1 rounded-[6px] px-2 py-1.5 hover:bg-[var(--v2-surface-muted)]"
      >
        <span
          className="flex-1 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]"
        >
          ${d("chat.conversations")}
        </span>
        <${D}
          name="chevron"
          className=${G("h-3.5 w-3.5 text-[var(--v2-text-faint)]",r?"-rotate-90":"")}
          strokeWidth=${2.2}
        />
      </button>

      ${!r&&l`
        ${e.length>0&&l`<div className="relative mb-1 mt-1 px-1">
          <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--v2-text-faint)]">
            <${D} name="search" className="h-3.5 w-3.5" />
          </span>
          <input
            type="text"
            value=${i}
            onInput=${y=>o(y.currentTarget.value)}
            placeholder=${d("common.searchChats")}
            className="h-8 w-full rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] pl-8 pr-2 text-[12px] text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]"
          />
        </div>`}
        <div
          className="mt-1 flex flex-col gap-2 overflow-y-auto [scrollbar-width:thin]"
        >
          ${e.length===0&&l`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            ${d("chat.noConversations")}
          </div>`}
          ${e.length>0&&p===0&&l`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            ${d("common.noChatsMatch").replace("{query}",i)}
          </div>`}

          <${K$}
            label=${d("common.pinned")}
            items=${f}
            activeThreadId=${t}
            states=${u}
            pinnedIds=${c}
            onSelect=${a}
            onDelete=${n}
          />
          <${K$}
            label=${d("common.recent")}
            items=${m}
            activeThreadId=${t}
            states=${u}
            pinnedIds=${c}
            onSelect=${a}
            onDelete=${n}
          />
        </div>
      `}
    </div>
  `}function cc(){let e=Y(),t=z({queryKey:["trace-credits"],queryFn:v$,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=I({mutationFn:g$,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function B4(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function Q$(){let e=k(),{credits:t}=cc();if(!t||!t.enrolled)return null;let a=B4(t.final_credit),n=t.submissions_accepted||0,r=t.submissions_submitted||0,s=t.manual_review_hold_count||0;return l`
    <div className="px-3 pb-1">
      <${Ar}
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
  `}function V$({threadsState:e,theme:t,toggleTheme:a,profile:n,isAdmin:r,onSignOut:s,onClose:i,onNewChat:o,onSelectThread:u,onDeleteThread:c}){return l`
    <aside
      className="flex h-full w-[260px] shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
    >
      <div className="flex items-center gap-2.5 px-4 py-5">
        <${Ar}
          to="/chat"
          onClick=${i}
          className="flex items-center gap-2.5 opacity-90 hover:opacity-100"
          aria-label="IronClaw"
        >
          <img src="/v2/assets/logo.jpg" alt="IronClaw" className="h-7 w-auto" />
        <//>
      </div>

      <${U$}
        onNewChat=${o}
        isCreating=${e.isCreating}
        isAdmin=${r}
        onNavigate=${i}
      />

      <${Q$} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${I$}
          threads=${e.threads}
          activeThreadId=${e.activeThreadId}
          onSelect=${u}
          onDelete=${c}
        />
      </div>

      <${O$}
        theme=${t}
        toggleTheme=${a}
        profile=${n}
        onSignOut=${s}
      />
    </aside>
  `}var H4="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",K4="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",G$="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",Y$={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},J$={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function T({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=Y$[n]??Y$.md,u=r?"w-full":"";if(a==="primary")return l`
      <${s}
        style=${{background:H4,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${G(G$,o,u,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:K4}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=J$[a]??J$.outline;return l`
    <${s}
      className=${G(G$,o,u,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function X$(){let e=h.default.useMemo(()=>I4(window.location),[]),[t,a]=h.default.useState(null),[n,r]=h.default.useState(null),[s,i]=h.default.useState(!1),[o,u]=h.default.useState(""),[c,d]=h.default.useState(!1);h.default.useEffect(()=>{if(!e)return;let p=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:p.signal}).then(y=>{if(!y.ok)throw new Error(String(y.status));return y.json()}).then(a).catch(()=>{p.signal.aborted||a(null)}),()=>p.abort()},[e]);let f=h.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),u("");try{let p=await fetch(`${e.base}/attestation/report`);if(!p.ok)throw new Error(String(p.status));let y=await p.json();return r(y),y}catch(p){return u(p.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),m=h.default.useCallback(async()=>{let p=n||await f();return!p||!navigator.clipboard?!1:(await navigator.clipboard.writeText(JSON.stringify({...p,instance_attestation:t},null,2)),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[f,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:f,copyReport:m}}function I4(e){let t=e.hostname;if(!t||t==="localhost"||Q4(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function Q4(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}var V4=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function Z$(){let e=k(),t=X$(),[a,n]=h.default.useState(!1),r=h.default.useCallback(()=>{n(o=>{let u=!o;return u&&t.loadReport(),u})},[t]),s=h.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=G4({teeInfo:t.teeInfo,report:t.report,t:e});return l`
    <div className="relative">
      <button
        type="button"
        onClick=${r}
        aria-expanded=${a}
        title=${e("tee.title")}
        className=${G("grid h-8 w-8 place-items-center rounded-[8px]","border border-[color-mix(in_srgb,var(--v2-positive-text)_28%,transparent)]","bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]","hover:border-[color-mix(in_srgb,var(--v2-positive-text)_52%,transparent)]")}
      >
        <${D} name="shield" className="h-4 w-4" />
      </button>

      ${a&&l`
        <div
          className=${G("absolute right-0 top-full z-40 mt-2 w-[min(22rem,calc(100vw-2rem))]","rounded-[14px] border border-[var(--v2-panel-border)]","bg-[var(--v2-surface)] p-3 shadow-[0_18px_48px_rgba(0,0,0,0.35)]")}
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
  `}function G4({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return V4.map(([r,s])=>({label:a(s),value:Y4(n[r])||a("common.unknown")}))}function Y4(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var J4="https://docs.ironclaw.com";function W$({threadsState:e,onToggleSidebar:t}){let a=k(),n=je(),r=h.default.useMemo(()=>{for(let i of Po){let o=nc[i.id];if(!o)continue;let u=i.path+"/";if(n.pathname.startsWith(u)){let c=n.pathname.slice(u.length).split("/")[0],d=o.find(f=>f.id===c);if(d)return{parent:a(i.labelKey),current:a(d.labelKey)}}}return null},[n.pathname,a]),s=h.default.useMemo(()=>{if(r)return null;if(n.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(u=>u.id===e.activeThreadId)?.title||a("nav.chat");let i=Po.find(o=>n.pathname.startsWith(o.path));return i?a(i.labelKey):""},[n.pathname,e.activeThreadId,e.threads,a,r]);return l`
    <header
      className=${G("flex h-14 shrink-0 items-center gap-3 px-4","border-b border-[var(--v2-panel-border)]","bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] backdrop-blur-xl")}
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
        <${Z$} />
        <${Xn}
          to="/logs"
          className=${({isActive:i})=>G("grid h-8 w-8 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",i&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
          title=${a("nav.logs")}
        >
          <${D} name="list" className="h-4 w-4" />
        <//>
        <a
          href=${J4}
          target="_blank"
          rel="noopener noreferrer"
          className="grid h-8 w-8 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${a("nav.docs")}
        >
          <${D} name="file" className="h-4 w-4" />
        </a>
      </div>
    </header>
  `}function e1({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=fe(),i=k(),[o,u]=h.default.useState(""),[c,d]=h.default.useState(0),f=h.default.useRef(null),m=h.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(x=>({id:`thread-${x.id}`,label:x.title||`Thread ${x.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${x.id}`)}));return[...g,...v]},[a,s,n,r]),p=h.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?m.filter(v=>v.label.toLowerCase().includes(g)):m},[m,o]);h.default.useEffect(()=>{if(!e)return;u(""),d(0);let g=window.requestAnimationFrame(()=>f.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),h.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,p.length-1)))},[p.length]);let y=h.default.useCallback(g=>{g&&(t(),g.run())},[t]),b=h.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,p.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),y(p[c])):g.key==="Escape"&&(g.preventDefault(),t())},[p,c,y,t]);if(!e)return null;let w=null;return l`
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh]" role="dialog" aria-modal="true" aria-label="Command palette">
      <button type="button" aria-label="Close" onClick=${t} className="absolute inset-0 bg-black/50"></button>
      <div className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_30px_60px_-20px_rgba(0,0,0,0.8)]">
        <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-3">
          <${D} name="search" className="h-4 w-4 text-[var(--v2-text-faint)]" />
          <input
            ref=${f}
            value=${o}
            onInput=${g=>u(g.currentTarget.value)}
            onKeyDown=${b}
            placeholder=${i("command.placeholder")}
            className="h-12 w-full border-0 bg-transparent text-sm text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)]"
          />
          <kbd className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]">esc</kbd>
        </div>
        <ul className="max-h-[50vh] overflow-y-auto p-1.5">
          ${p.length===0&&l`<li className="px-3 py-6 text-center text-sm text-[var(--v2-text-faint)]">No matches</li>`}
          ${p.map((g,v)=>{let x=g.group!==w;return w=g.group,l`
              ${x&&l`<li key=${`g-${g.group}`} className="px-2 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">${g.group}</li>`}
              <li key=${g.id}>
                <button
                  type="button"
                  onMouseEnter=${()=>d(v)}
                  onClick=${()=>y(g)}
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
  `}var t1={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},X4={info:"bolt",success:"check",error:"close"};function a1(){let[e,t]=h.default.useState([]);return h.default.useEffect(()=>E$(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:l`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${e.map(a=>l`
          <div
            key=${a.id}
            role="status"
            className=${["pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",t1[a.tone]||t1.info].join(" ")}
          >
            <${D} name=${X4[a.tone]||"bolt"} className="h-4 w-4 shrink-0" />
            <span>${a.message}</span>
          </div>
        `)}
    </div>
  `}function n1({token:e,profile:t,isChecking:a=!1,isAdmin:n,onSignOut:r}){let s=k(),{theme:i,toggleTheme:o}=rc(),u=Jx(e),c=A$(),d=C$({onNewChat:()=>c.setActiveThreadId(null)}),f=u.data,m=je(),p=fe(),y=Is({settings:{},gatewayStatus:f,enabled:n}),b=n&&R$({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),w=m.pathname==="/welcome"||m.pathname.startsWith("/settings"),[g,v]=h.default.useState(!1);h.default.useEffect(()=>{let $=S=>{(S.metaKey||S.ctrlKey)&&S.key.toLowerCase()==="k"&&(S.preventDefault(),v(R=>!R))};return window.addEventListener("keydown",$),()=>window.removeEventListener("keydown",$)},[]);let x=h.default.useCallback(async $=>{let S=c.activeThreadId===$;try{await c.deleteThread($),S&&p("/chat",{replace:!0})}catch(R){console.error("Failed to delete thread:",R),Qs(T$(R,s),{tone:"error"})}},[p,c,s]);return b&&!w?l`<${ot} to="/welcome" replace />`:l`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${d.open&&l`<button
        type="button"
        aria-label=${s("nav.close")}
        onClick=${d.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${G("fixed inset-y-0 left-0 z-50 md:relative md:z-auto",d.open?"flex":"hidden md:flex")}
      >
        <${V$}
          threadsState=${c}
          theme=${i}
          toggleTheme=${o}
          profile=${t}
          isAdmin=${n}
          onSignOut=${r}
          onClose=${d.close}
          onNewChat=${d.newChat}
          onSelectThread=${d.selectThread}
          onDeleteThread=${x}
        />
      </div>

      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <${W$}
          threadsState=${c}
          onToggleSidebar=${d.toggle}
        />
        <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
          ${u.error&&l`
            <div
              className=${G("m-4 rounded-[14px] border px-4 py-3 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >
              ${u.error.message||s("error.gatewayConnection")}
            </div>
          `}
          <${np}
            context=${{gatewayStatus:f,gatewayStatusQuery:u,currentUser:t,isChecking:a,isAdmin:n,threadsState:c}}
          />
        </main>
      </div>
      <${e1}
        open=${g}
        onClose=${()=>v(!1)}
        threadsState=${c}
        onNewChat=${d.newChat}
        onToggleTheme=${o}
      />
      <${a1} />
    </div>
  `}var It=qe(Ie(),1),Qo=e=>e.type==="checkbox",Ur=e=>e instanceof Date,Dt=e=>e==null,v1=e=>typeof e=="object",Ge=e=>!Dt(e)&&!Array.isArray(e)&&v1(e)&&!Ur(e),Z4=e=>Ge(e)&&e.target?Qo(e.target)?e.target.checked:e.target.value:e,W4=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,eE=(e,t)=>e.has(W4(t)),tE=e=>{let t=e.constructor&&e.constructor.prototype;return Ge(t)&&t.hasOwnProperty("isPrototypeOf")},Dp=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function pt(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(Dp&&(e instanceof Blob||n))&&(a||Ge(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!tE(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=pt(e[r]));else return e;return t}var hc=e=>/^\w*$/.test(e),et=e=>e===void 0,Mp=e=>Array.isArray(e)?e.filter(Boolean):[],Op=e=>Mp(e.replace(/["|']|\]/g,"").split(/\.|\[/)),Q=(e,t,a)=>{if(!t||!Ge(e))return a;let n=(hc(t)?[t]:Op(t)).reduce((r,s)=>Dt(r)?r:r[s],e);return et(n)||n===e?et(e[t])?a:e[t]:n},Ha=e=>typeof e=="boolean",Pe=(e,t,a)=>{let n=-1,r=hc(t)?[t]:Op(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],u=a;if(n!==i){let c=e[o];u=Ge(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=u,e=e[o]}},r1={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},ka={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},vn={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},aE=It.default.createContext(null);aE.displayName="HookFormContext";var nE=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==ka.all&&(t._proxyFormState[i]=!n||ka.all),a&&(a[i]=!0),e[i]}});return r},rE=typeof window<"u"?It.default.useLayoutEffect:It.default.useEffect;var Ka=e=>typeof e=="string",sE=(e,t,a,n,r)=>Ka(e)?(n&&t.watch.add(e),Q(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),Q(a,s))):(n&&(t.watchAll=!0),a),Ap=e=>Dt(e)||!v1(e);function Zn(e,t,a=new WeakSet){if(Ap(e)||Ap(t))return e===t;if(Ur(e)&&Ur(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(Ur(i)&&Ur(o)||Ge(i)&&Ge(o)||Array.isArray(i)&&Array.isArray(o)?!Zn(i,o,a):i!==o)return!1}}return!0}var iE=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},Ko=e=>Array.isArray(e)?e:[e],s1=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},Qt=e=>Ge(e)&&!Object.keys(e).length,Lp=e=>e.type==="file",Ra=e=>typeof e=="function",mc=e=>{if(!Dp)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},g1=e=>e.type==="select-multiple",Up=e=>e.type==="radio",oE=e=>Up(e)||Qo(e),Tp=e=>mc(e)&&e.isConnected;function lE(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=et(e)?n++:e[t[n++]];return e}function uE(e){for(let t in e)if(e.hasOwnProperty(t)&&!et(e[t]))return!1;return!0}function We(e,t){let a=Array.isArray(t)?t:hc(t)?[t]:Op(t),n=a.length===1?e:lE(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Ge(n)&&Qt(n)||Array.isArray(n)&&uE(n))&&We(e,a.slice(0,-1)),e}var y1=e=>{for(let t in e)if(Ra(e[t]))return!0;return!1};function fc(e,t={}){let a=Array.isArray(e);if(Ge(e)||a)for(let n in e)Array.isArray(e[n])||Ge(e[n])&&!y1(e[n])?(t[n]=Array.isArray(e[n])?[]:{},fc(e[n],t[n])):Dt(e[n])||(t[n]=!0);return t}function b1(e,t,a){let n=Array.isArray(e);if(Ge(e)||n)for(let r in e)Array.isArray(e[r])||Ge(e[r])&&!y1(e[r])?et(t)||Ap(a[r])?a[r]=Array.isArray(e[r])?fc(e[r],[]):{...fc(e[r])}:b1(e[r],Dt(t)?{}:t[r],a[r]):a[r]=!Zn(e[r],t[r]);return a}var Bo=(e,t)=>b1(e,t,fc(t)),i1={value:!1,isValid:!1},o1={value:!0,isValid:!0},x1=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!et(e[0].attributes.value)?et(e[0].value)||e[0].value===""?o1:{value:e[0].value,isValid:!0}:o1:i1}return i1},$1=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>et(e)?e:t?e===""?NaN:e&&+e:a&&Ka(e)?new Date(e):n?n(e):e,l1={isValid:!1,value:null},w1=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,l1):l1;function u1(e){let t=e.ref;return Lp(t)?t.files:Up(t)?w1(e.refs).value:g1(t)?[...t.selectedOptions].map(({value:a})=>a):Qo(t)?x1(e.refs).value:$1(et(t.value)?e.ref.value:t.value,e)}var cE=(e,t,a,n)=>{let r={};for(let s of e){let i=Q(t,s);i&&Pe(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},pc=e=>e instanceof RegExp,Ho=e=>et(e)?e:pc(e)?e.source:Ge(e)?pc(e.value)?e.value.source:e.value:e,c1=e=>({isOnSubmit:!e||e===ka.onSubmit,isOnBlur:e===ka.onBlur,isOnChange:e===ka.onChange,isOnAll:e===ka.all,isOnTouch:e===ka.onTouched}),d1="AsyncFunction",dE=e=>!!e&&!!e.validate&&!!(Ra(e.validate)&&e.validate.constructor.name===d1||Ge(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===d1)),mE=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),m1=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),Io=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=Q(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(Io(o,t))break}else if(Ge(o)&&Io(o,t))break}}};function f1(e,t,a){let n=Q(e,a);if(n||hc(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=Q(t,s),o=Q(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var fE=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return Qt(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||ka.all))},pE=(e,t,a)=>!e||!t||e===t||Ko(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),hE=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,vE=(e,t)=>!Mp(Q(e,t)).length&&We(e,t),gE=(e,t,a)=>{let n=Ko(Q(e,a));return Pe(n,"root",t[a]),Pe(e,a,n),e},dc=e=>Ka(e);function p1(e,t,a="validate"){if(dc(e)||Array.isArray(e)&&e.every(dc)||Ha(e)&&!e)return{type:a,message:dc(e)?e:"",ref:t}}var Gs=e=>Ge(e)&&!pc(e)?e:{value:e,message:""},h1=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:u,maxLength:c,minLength:d,min:f,max:m,pattern:p,validate:y,name:b,valueAsNumber:w,mount:g}=e._f,v=Q(a,b);if(!g||t.has(b))return{};let x=o?o[0]:i,$=A=>{r&&x.reportValidity&&(x.setCustomValidity(Ha(A)?"":A||""),x.reportValidity())},S={},R=Up(i),_=Qo(i),C=R||_,U=(w||Lp(i))&&et(i.value)&&et(v)||mc(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,O=iE.bind(null,b,n,S),B=(A,K,te,ye=vn.maxLength,ke=vn.minLength)=>{let Je=A?K:te;S[b]={type:A?ye:ke,message:Je,ref:i,...O(A?ye:ke,Je)}};if(s?!Array.isArray(v)||!v.length:u&&(!C&&(U||Dt(v))||Ha(v)&&!v||_&&!x1(o).isValid||R&&!w1(o).isValid)){let{value:A,message:K}=dc(u)?{value:!!u,message:u}:Gs(u);if(A&&(S[b]={type:vn.required,message:K,ref:x,...O(vn.required,K)},!n))return $(K),S}if(!U&&(!Dt(f)||!Dt(m))){let A,K,te=Gs(m),ye=Gs(f);if(!Dt(v)&&!isNaN(v)){let ke=i.valueAsNumber||v&&+v;Dt(te.value)||(A=ke>te.value),Dt(ye.value)||(K=ke<ye.value)}else{let ke=i.valueAsDate||new Date(v),Je=xa=>new Date(new Date().toDateString()+" "+xa),kt=i.type=="time",lt=i.type=="week";Ka(te.value)&&v&&(A=kt?Je(v)>Je(te.value):lt?v>te.value:ke>new Date(te.value)),Ka(ye.value)&&v&&(K=kt?Je(v)<Je(ye.value):lt?v<ye.value:ke<new Date(ye.value))}if((A||K)&&(B(!!A,te.message,ye.message,vn.max,vn.min),!n))return $(S[b].message),S}if((c||d)&&!U&&(Ka(v)||s&&Array.isArray(v))){let A=Gs(c),K=Gs(d),te=!Dt(A.value)&&v.length>+A.value,ye=!Dt(K.value)&&v.length<+K.value;if((te||ye)&&(B(te,A.message,K.message),!n))return $(S[b].message),S}if(p&&!U&&Ka(v)){let{value:A,message:K}=Gs(p);if(pc(A)&&!v.match(A)&&(S[b]={type:vn.pattern,message:K,ref:i,...O(vn.pattern,K)},!n))return $(K),S}if(y){if(Ra(y)){let A=await y(v,a),K=p1(A,x);if(K&&(S[b]={...K,...O(vn.validate,K.message)},!n))return $(K.message),S}else if(Ge(y)){let A={};for(let K in y){if(!Qt(A)&&!n)break;let te=p1(await y[K](v,a),x,K);te&&(A={...te,...O(K,te.message)},$(te.message),n&&(S[b]=A))}if(!Qt(A)&&(S[b]={ref:x,...A},!n))return S}}return $(!0),S},yE={mode:ka.onSubmit,reValidateMode:ka.onChange,shouldFocusError:!0};function bE(e={}){let t={...yE,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:Ra(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Ge(t.defaultValues)||Ge(t.values)?pt(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:pt(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},u,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},f={...d},m={array:s1(),state:s1()},p=t.criteriaMode===ka.all,y=N=>E=>{clearTimeout(c),c=setTimeout(N,E)},b=async N=>{if(!t.disabled&&(d.isValid||f.isValid||N)){let E=t.resolver?Qt((await _()).errors):await U(n,!0);E!==a.isValid&&m.state.next({isValid:E})}},w=(N,E)=>{!t.disabled&&(d.isValidating||d.validatingFields||f.isValidating||f.validatingFields)&&((N||Array.from(o.mount)).forEach(M=>{M&&(E?Pe(a.validatingFields,M,E):We(a.validatingFields,M))}),m.state.next({validatingFields:a.validatingFields,isValidating:!Qt(a.validatingFields)}))},g=(N,E=[],M,H,q=!0,F=!0)=>{if(H&&M&&!t.disabled){if(i.action=!0,F&&Array.isArray(Q(n,N))){let J=M(Q(n,N),H.argA,H.argB);q&&Pe(n,N,J)}if(F&&Array.isArray(Q(a.errors,N))){let J=M(Q(a.errors,N),H.argA,H.argB);q&&Pe(a.errors,N,J),vE(a.errors,N)}if((d.touchedFields||f.touchedFields)&&F&&Array.isArray(Q(a.touchedFields,N))){let J=M(Q(a.touchedFields,N),H.argA,H.argB);q&&Pe(a.touchedFields,N,J)}(d.dirtyFields||f.dirtyFields)&&(a.dirtyFields=Bo(r,s)),m.state.next({name:N,isDirty:B(N,E),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else Pe(s,N,E)},v=(N,E)=>{Pe(a.errors,N,E),m.state.next({errors:a.errors})},x=N=>{a.errors=N,m.state.next({errors:a.errors,isValid:!1})},$=(N,E,M,H)=>{let q=Q(n,N);if(q){let F=Q(s,N,et(M)?Q(r,N):M);et(F)||H&&H.defaultChecked||E?Pe(s,N,E?F:u1(q._f)):te(N,F),i.mount&&b()}},S=(N,E,M,H,q)=>{let F=!1,J=!1,be={name:N};if(!t.disabled){if(!M||H){(d.isDirty||f.isDirty)&&(J=a.isDirty,a.isDirty=be.isDirty=B(),F=J!==be.isDirty);let Re=Zn(Q(r,N),E);J=!!Q(a.dirtyFields,N),Re?We(a.dirtyFields,N):Pe(a.dirtyFields,N,!0),be.dirtyFields=a.dirtyFields,F=F||(d.dirtyFields||f.dirtyFields)&&J!==!Re}if(M){let Re=Q(a.touchedFields,N);Re||(Pe(a.touchedFields,N,M),be.touchedFields=a.touchedFields,F=F||(d.touchedFields||f.touchedFields)&&Re!==M)}F&&q&&m.state.next(be)}return F?be:{}},R=(N,E,M,H)=>{let q=Q(a.errors,N),F=(d.isValid||f.isValid)&&Ha(E)&&a.isValid!==E;if(t.delayError&&M?(u=y(()=>v(N,M)),u(t.delayError)):(clearTimeout(c),u=null,M?Pe(a.errors,N,M):We(a.errors,N)),(M?!Zn(q,M):q)||!Qt(H)||F){let J={...H,...F&&Ha(E)?{isValid:E}:{},errors:a.errors,name:N};a={...a,...J},m.state.next(J)}},_=async N=>{w(N,!0);let E=await t.resolver(s,t.context,cE(N||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return w(N),E},C=async N=>{let{errors:E}=await _(N);if(N)for(let M of N){let H=Q(E,M);H?Pe(a.errors,M,H):We(a.errors,M)}else a.errors=E;return E},U=async(N,E,M={valid:!0})=>{for(let H in N){let q=N[H];if(q){let{_f:F,...J}=q;if(F){let be=o.array.has(F.name),Re=q._f&&dE(q._f);Re&&d.validatingFields&&w([H],!0);let oa=await h1(q,o.disabled,s,p,t.shouldUseNativeValidation&&!E,be);if(Re&&d.validatingFields&&w([H]),oa[F.name]&&(M.valid=!1,E))break;!E&&(Q(oa,F.name)?be?gE(a.errors,oa,F.name):Pe(a.errors,F.name,oa[F.name]):We(a.errors,F.name))}!Qt(J)&&await U(J,E,M)}}return M.valid},O=()=>{for(let N of o.unMount){let E=Q(n,N);E&&(E._f.refs?E._f.refs.every(M=>!Tp(M)):!Tp(E._f.ref))&&ve(N)}o.unMount=new Set},B=(N,E)=>!t.disabled&&(N&&E&&Pe(s,N,E),!Zn(xa(),r)),A=(N,E,M)=>sE(N,o,{...i.mount?s:et(E)?r:Ka(N)?{[N]:E}:E},M,E),K=N=>Mp(Q(i.mount?s:r,N,t.shouldUnregister?Q(r,N,[]):[])),te=(N,E,M={})=>{let H=Q(n,N),q=E;if(H){let F=H._f;F&&(!F.disabled&&Pe(s,N,$1(E,F)),q=mc(F.ref)&&Dt(E)?"":E,g1(F.ref)?[...F.ref.options].forEach(J=>J.selected=q.includes(J.value)):F.refs?Qo(F.ref)?F.refs.forEach(J=>{(!J.defaultChecked||!J.disabled)&&(Array.isArray(q)?J.checked=!!q.find(be=>be===J.value):J.checked=q===J.value||!!q)}):F.refs.forEach(J=>J.checked=J.value===q):Lp(F.ref)?F.ref.value="":(F.ref.value=q,F.ref.type||m.state.next({name:N,values:pt(s)})))}(M.shouldDirty||M.shouldTouch)&&S(N,q,M.shouldTouch,M.shouldDirty,!0),M.shouldValidate&&lt(N)},ye=(N,E,M)=>{for(let H in E){if(!E.hasOwnProperty(H))return;let q=E[H],F=N+"."+H,J=Q(n,F);(o.array.has(N)||Ge(q)||J&&!J._f)&&!Ur(q)?ye(F,q,M):te(F,q,M)}},ke=(N,E,M={})=>{let H=Q(n,N),q=o.array.has(N),F=pt(E);Pe(s,N,F),q?(m.array.next({name:N,values:pt(s)}),(d.isDirty||d.dirtyFields||f.isDirty||f.dirtyFields)&&M.shouldDirty&&m.state.next({name:N,dirtyFields:Bo(r,s),isDirty:B(N,F)})):H&&!H._f&&!Dt(F)?ye(N,F,M):te(N,F,M),m1(N,o)&&m.state.next({...a,name:N}),m.state.next({name:i.mount?N:void 0,values:pt(s)})},Je=async N=>{i.mount=!0;let E=N.target,M=E.name,H=!0,q=Q(n,M),F=Re=>{H=Number.isNaN(Re)||Ur(Re)&&isNaN(Re.getTime())||Zn(Re,Q(s,M,Re))},J=c1(t.mode),be=c1(t.reValidateMode);if(q){let Re,oa,el=E.type?u1(q._f):Z4(N),xn=N.type===r1.BLUR||N.type===r1.FOCUS_OUT,Y_=!mE(q._f)&&!t.resolver&&!Q(a.errors,M)&&!q._f.deps||hE(xn,Q(a.touchedFields,M),a.isSubmitted,be,J),Xc=m1(M,o,xn);Pe(s,M,el),xn?(!E||!E.readOnly)&&(q._f.onBlur&&q._f.onBlur(N),u&&u(0)):q._f.onChange&&q._f.onChange(N);let Zc=S(M,el,xn),J_=!Qt(Zc)||Xc;if(!xn&&m.state.next({name:M,type:N.type,values:pt(s)}),Y_)return(d.isValid||f.isValid)&&(t.mode==="onBlur"?xn&&b():xn||b()),J_&&m.state.next({name:M,...Xc?{}:Zc});if(!xn&&Xc&&m.state.next({...a}),t.resolver){let{errors:yh}=await _([M]);if(F(el),H){let X_=f1(a.errors,n,M),bh=f1(yh,n,X_.name||M);Re=bh.error,M=bh.name,oa=Qt(yh)}}else w([M],!0),Re=(await h1(q,o.disabled,s,p,t.shouldUseNativeValidation))[M],w([M]),F(el),H&&(Re?oa=!1:(d.isValid||f.isValid)&&(oa=await U(n,!0)));H&&(q._f.deps&&lt(q._f.deps),R(M,oa,Re,Zc))}},kt=(N,E)=>{if(Q(a.errors,E)&&N.focus)return N.focus(),1},lt=async(N,E={})=>{let M,H,q=Ko(N);if(t.resolver){let F=await C(et(N)?N:q);M=Qt(F),H=N?!q.some(J=>Q(F,J)):M}else N?(H=(await Promise.all(q.map(async F=>{let J=Q(n,F);return await U(J&&J._f?{[F]:J}:J)}))).every(Boolean),!(!H&&!a.isValid)&&b()):H=M=await U(n);return m.state.next({...!Ka(N)||(d.isValid||f.isValid)&&M!==a.isValid?{}:{name:N},...t.resolver||!N?{isValid:M}:{},errors:a.errors}),E.shouldFocus&&!H&&Io(n,kt,N?q:o.mount),H},xa=N=>{let E={...i.mount?s:r};return et(N)?E:Ka(N)?Q(E,N):N.map(M=>Q(E,M))},$a=(N,E)=>({invalid:!!Q((E||a).errors,N),isDirty:!!Q((E||a).dirtyFields,N),error:Q((E||a).errors,N),isValidating:!!Q(a.validatingFields,N),isTouched:!!Q((E||a).touchedFields,N)}),ue=N=>{N&&Ko(N).forEach(E=>We(a.errors,E)),m.state.next({errors:N?a.errors:{}})},ne=(N,E,M)=>{let H=(Q(n,N,{_f:{}})._f||{}).ref,q=Q(a.errors,N)||{},{ref:F,message:J,type:be,...Re}=q;Pe(a.errors,N,{...Re,...E,ref:H}),m.state.next({name:N,errors:a.errors,isValid:!1}),M&&M.shouldFocus&&H&&H.focus&&H.focus()},ze=(N,E)=>Ra(N)?m.state.subscribe({next:M=>"values"in M&&N(A(void 0,E),M)}):A(N,E,!0),Ae=N=>m.state.subscribe({next:E=>{pE(N.name,E.name,N.exact)&&fE(E,N.formState||d,ae,N.reRenderRoot)&&N.callback({values:{...s},...a,...E,defaultValues:r})}}).unsubscribe,bt=N=>(i.mount=!0,f={...f,...N.formState},Ae({...N,formState:f})),ve=(N,E={})=>{for(let M of N?Ko(N):o.mount)o.mount.delete(M),o.array.delete(M),E.keepValue||(We(n,M),We(s,M)),!E.keepError&&We(a.errors,M),!E.keepDirty&&We(a.dirtyFields,M),!E.keepTouched&&We(a.touchedFields,M),!E.keepIsValidating&&We(a.validatingFields,M),!t.shouldUnregister&&!E.keepDefaultValue&&We(r,M);m.state.next({values:pt(s)}),m.state.next({...a,...E.keepDirty?{isDirty:B()}:{}}),!E.keepIsValid&&b()},ge=({disabled:N,name:E})=>{(Ha(N)&&i.mount||N||o.disabled.has(E))&&(N?o.disabled.add(E):o.disabled.delete(E))},ut=(N,E={})=>{let M=Q(n,N),H=Ha(E.disabled)||Ha(t.disabled);return Pe(n,N,{...M||{},_f:{...M&&M._f?M._f:{ref:{name:N}},name:N,mount:!0,...E}}),o.mount.add(N),M?ge({disabled:Ha(E.disabled)?E.disabled:t.disabled,name:N}):$(N,!0,E.value),{...H?{disabled:E.disabled||t.disabled}:{},...t.progressive?{required:!!E.required,min:Ho(E.min),max:Ho(E.max),minLength:Ho(E.minLength),maxLength:Ho(E.maxLength),pattern:Ho(E.pattern)}:{},name:N,onChange:Je,onBlur:Je,ref:q=>{if(q){ut(N,E),M=Q(n,N);let F=et(q.value)&&q.querySelectorAll&&q.querySelectorAll("input,select,textarea")[0]||q,J=oE(F),be=M._f.refs||[];if(J?be.find(Re=>Re===F):F===M._f.ref)return;Pe(n,N,{_f:{...M._f,...J?{refs:[...be.filter(Tp),F,...Array.isArray(Q(r,N))?[{}]:[]],ref:{type:F.type,name:N}}:{ref:F}}}),$(N,!1,void 0,F)}else M=Q(n,N,{}),M._f&&(M._f.mount=!1),(t.shouldUnregister||E.shouldUnregister)&&!(eE(o.array,N)&&i.action)&&o.unMount.add(N)}}},Ke=()=>t.shouldFocusError&&Io(n,kt,o.mount),Rt=N=>{Ha(N)&&(m.state.next({disabled:N}),Io(n,(E,M)=>{let H=Q(n,M);H&&(E.disabled=H._f.disabled||N,Array.isArray(H._f.refs)&&H._f.refs.forEach(q=>{q.disabled=H._f.disabled||N}))},0,!1))},Ne=(N,E)=>async M=>{let H;M&&(M.preventDefault&&M.preventDefault(),M.persist&&M.persist());let q=pt(s);if(m.state.next({isSubmitting:!0}),t.resolver){let{errors:F,values:J}=await _();a.errors=F,q=pt(J)}else await U(n);if(o.disabled.size)for(let F of o.disabled)We(q,F);if(We(a.errors,"root"),Qt(a.errors)){m.state.next({errors:{}});try{await N(q,M)}catch(F){H=F}}else E&&await E({...a.errors},M),Ke(),setTimeout(Ke);if(m.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:Qt(a.errors)&&!H,submitCount:a.submitCount+1,errors:a.errors}),H)throw H},yn=(N,E={})=>{Q(n,N)&&(et(E.defaultValue)?ke(N,pt(Q(r,N))):(ke(N,E.defaultValue),Pe(r,N,pt(E.defaultValue))),E.keepTouched||We(a.touchedFields,N),E.keepDirty||(We(a.dirtyFields,N),a.isDirty=E.defaultValue?B(N,pt(Q(r,N))):B()),E.keepError||(We(a.errors,N),d.isValid&&b()),m.state.next({...a}))},Lt=(N,E={})=>{let M=N?pt(N):r,H=pt(M),q=Qt(N),F=q?r:H;if(E.keepDefaultValues||(r=M),!E.keepValues){if(E.keepDirtyValues){let J=new Set([...o.mount,...Object.keys(Bo(r,s))]);for(let be of Array.from(J))Q(a.dirtyFields,be)?Pe(F,be,Q(s,be)):ke(be,Q(F,be))}else{if(Dp&&et(N))for(let J of o.mount){let be=Q(n,J);if(be&&be._f){let Re=Array.isArray(be._f.refs)?be._f.refs[0]:be._f.ref;if(mc(Re)){let oa=Re.closest("form");if(oa){oa.reset();break}}}}if(E.keepFieldsRef)for(let J of o.mount)ke(J,Q(F,J));else n={}}s=t.shouldUnregister?E.keepDefaultValues?pt(r):{}:pt(F),m.array.next({values:{...F}}),m.state.next({values:{...F}})}o={mount:E.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!E.keepIsValid||!!E.keepDirtyValues,i.watch=!!t.shouldUnregister,m.state.next({submitCount:E.keepSubmitCount?a.submitCount:0,isDirty:q?!1:E.keepDirty?a.isDirty:!!(E.keepDefaultValues&&!Zn(N,r)),isSubmitted:E.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:q?{}:E.keepDirtyValues?E.keepDefaultValues&&s?Bo(r,s):a.dirtyFields:E.keepDefaultValues&&N?Bo(r,N):E.keepDirty?a.dirtyFields:{},touchedFields:E.keepTouched?a.touchedFields:{},errors:E.keepErrors?a.errors:{},isSubmitSuccessful:E.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},ia=(N,E)=>Lt(Ra(N)?N(s):N,E),Jc=(N,E={})=>{let M=Q(n,N),H=M&&M._f;if(H){let q=H.refs?H.refs[0]:H.ref;q.focus&&(q.focus(),E.shouldSelect&&Ra(q.select)&&q.select())}},ae=N=>{a={...a,...N}},bn={control:{register:ut,unregister:ve,getFieldState:$a,handleSubmit:Ne,setError:ne,_subscribe:Ae,_runSchema:_,_focusError:Ke,_getWatch:A,_getDirty:B,_setValid:b,_setFieldArray:g,_setDisabledField:ge,_setErrors:x,_getFieldArray:K,_reset:Lt,_resetDefaultValues:()=>Ra(t.defaultValues)&&t.defaultValues().then(N=>{ia(N,t.resetOptions),m.state.next({isLoading:!1})}),_removeUnmounted:O,_disableForm:Rt,_subjects:m,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(N){i=N},get _defaultValues(){return r},get _names(){return o},set _names(N){o=N},get _formState(){return a},get _options(){return t},set _options(N){t={...t,...N}}},subscribe:bt,trigger:lt,register:ut,handleSubmit:Ne,watch:ze,setValue:ke,getValues:xa,reset:ia,resetField:yn,clearErrors:ue,unregister:ve,setError:ne,setFocus:Jc,getFieldState:$a};return{...bn,formControl:bn}}function S1(e={}){let t=It.default.useRef(void 0),a=It.default.useRef(void 0),[n,r]=It.default.useState({isDirty:!1,isValidating:!1,isLoading:Ra(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:Ra(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!Ra(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=bE(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,rE(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),It.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),It.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),It.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),It.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),It.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),It.default.useEffect(()=>{e.values&&!Zn(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),It.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=nE(n,s),t.current}var N1={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},_1={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},xE={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function W({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return l`
    <${s}
      className=${G(N1[a]??N1.default,_1[n]??_1.md,xE[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var jp="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",vc={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function Mt({className:e="",size:t="md",error:a=!1,...n}){return l`
    <input
      className=${G(jp,vc[t]??vc.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function gc({className:e="",error:t=!1,rows:a=4,...n}){return l`
    <textarea
      rows=${a}
      className=${G(jp,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Pp({children:e,className:t="",size:a="md",error:n=!1,...r}){return l`
    <div className="relative w-full">
      <select
        className=${G(jp,vc[a]??vc.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
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
  `}function $E({children:e,className:t="",required:a=!1,...n}){return l`
    <label
      className=${G("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&l`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function gn({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return l`
    <div className=${G("flex flex-col gap-2",s)}>
      ${e&&l`<${$E} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&l`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&l`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var wE={google:"Google",github:"GitHub",apple:"Apple"};function SE(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function k1({providers:e,redirectAfter:t}){let a=k();return e.length?l`
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
              href=${SE(n,t)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <${D} name="shield" className="h-4 w-4" />
              ${a("login.oauthProvider",{provider:wE[n]||n})}
            <//>
          `)}
      </div>
    </div>
  `:null}var NE=["google","github","apple"];function R1(){let[e,t]=h.default.useState([]);return h.default.useEffect(()=>{let a=!1;return bx().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(NE.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function C1({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=k(),{theme:s,toggleTheme:i}=rc(),o=R1(),{formState:{errors:u,isSubmitting:c},handleSubmit:d,register:f}=S1({defaultValues:{token:e||""}});return l`
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
      <${W}
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
          <${gn}
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
              className=${G("rounded-[10px] border px-3 py-2 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
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

        <${k1}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var E1={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},T1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function P({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return l`
    <span
      className=${G("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",T1[n]??T1.md,E1[e]??E1.muted,r)}
    >
      ${a&&l`<span
          className=${G("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var _E=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,A1=/(bash|shell|exec|run|command|terminal|spawn|process)/,D1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function M1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return _E.test(n)?{tone:"danger",key:"tool.riskWrite"}:A1.test(n)?{tone:"warning",key:"tool.riskExec"}:D1.test(n)?{tone:"info",key:"tool.riskNetwork"}:A1.test(r)?{tone:"warning",key:"tool.riskExec"}:D1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}function O1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=k(),{toolName:s,description:i,parameters:o,allowAlways:u,approvalDetails:c=[]}=e,[d,f]=h.default.useState(!1),m=h.default.useMemo(()=>M1(s,i,o),[s,i,o]),p=s||r("approval.thisTool"),y=h.default.useCallback(()=>{d&&u?n?.():t?.()},[d,u,n,t]);return l`
    <div className="mx-auto max-w-lg rounded-xl border border-copper/30 bg-copper/10 p-4">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-copper/25 bg-copper/10 text-copper">
          <${D} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">${r("approval.title")}</span>
        <${P}
          tone=${m.tone}
          label=${r(m.key)}
          dot=${!1}
          size="sm"
          className="ml-auto"
        />
      </div>
      ${s&&l`<div className="mb-1 break-all font-mono text-sm font-medium text-iron-100">${s}</div>`}
      ${i&&l`<div className="mb-3 break-words text-sm text-iron-200">${i}</div>`}
      ${c.length>0?l`
            <dl className="mb-3 max-h-56 overflow-y-auto rounded-md border border-iron-800 bg-iron-950/80 text-xs">
              ${c.map(b=>l`
                  <div className="grid gap-1 border-b border-iron-800/70 px-3 py-2 last:border-b-0 sm:grid-cols-[7rem_1fr]">
                    <dt className="font-medium text-iron-400">${b.label}</dt>
                    <dd className="min-w-0 break-all font-mono text-iron-100">${b.value}</dd>
                  </div>
                `)}
            </dl>
          `:o&&l`<pre className="mb-3 max-h-56 overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100">${o}</pre>`}

      ${u&&l`
        <label className="mb-3 flex items-center gap-2 text-xs text-iron-200">
          <input
            type="checkbox"
            checked=${d}
            onChange=${b=>f(b.currentTarget.checked)}
            className="h-3.5 w-3.5 accent-[var(--v2-accent)]"
          />
          ${r("approval.alwaysAllowToolLabel",{tool:p})}
        </label>
      `}

      <div className="flex flex-wrap gap-2">
        <${T} variant="primary" onClick=${y}>
          ${r(d&&u?"approval.approveAndAlways":"approval.approve")}
        <//>
        <${T} variant="secondary" onClick=${()=>a?.()}>
          ${r("approval.deny")}
        <//>
      </div>
    </div>
  `}function Ys({icon:e="lock",headline:t,provider:a,accountLabel:n,body:r,expiresAt:s,pillHint:i,defaultExpanded:o=!0,children:u}){let c=k(),[d,f]=h.default.useState(o),m=h.default.useId(),p=n||a||"";return l`
    <div className="mx-auto w-full max-w-lg rounded-xl border border-[rgba(76,167,230,0.34)] bg-[rgba(76,167,230,0.08)]">
      <button
        type="button"
        onClick=${()=>f(y=>!y)}
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
  `}function L1({gate:e,onCancel:t}){let a=k();return l`
    <${Ys}
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
  `}function U1({gate:e,onCancel:t}){let a=k(),[n,r]=h.default.useState(!1),s=h.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]),i=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),o=h.default.useCallback(()=>{s&&(window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0))},[e.authorizationUrl,s]),u=n?a("authGate.reopenAuthorization",{provider:i}):a("authGate.openAuthorization",{provider:i});return l`
    <${Ys}
      icon="link"
      headline=${e?.headline||a("authGate.oauthTitle")}
      provider=${e?.provider?i:""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      expiresAt=${e?.expiresAt||""}
      pillHint=${a("authGate.pillAuthorize")}
    >
      <div className="flex flex-wrap gap-2">
        <${T}
          as="a"
          href=${s?e.authorizationUrl:void 0}
          target="_blank"
          rel="noopener noreferrer"
          className="auth-oauth"
          variant="primary"
          disabled=${!s}
          onClick=${c=>{c.preventDefault(),o()}}
        >
          <${D} name="link" className="h-4 w-4" />
          ${u}
        <//>
        <${T}
          type="button"
          variant="secondary"
          onClick=${()=>t?.()}
        >
          ${a("authGate.cancel")}
        <//>
      </div>

      ${n&&l`
        <p className="mt-2 text-xs text-iron-300">${a("authGate.oauthWaiting")}</p>
      `}
    <//>
  `}function j1({gate:e,onSubmit:t,onCancel:a}){let n=k(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState(!1),d=h.default.useCallback(async f=>{f.preventDefault();let m=r.trim();if(!m){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(m),s("")}catch(p){o(p?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return l`
    <${Ys}
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
  `}var kE="/api/webchat/v2/extensions/pairing/redeem";function P1(e){return Z(kE,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function yc({action:e}){let t=k(),a=Y(),n=I({mutationFn:({code:u})=>P1(u),onSuccess:()=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=h.default.useState(""),i=RE(e,t),o=()=>{let u=r.trim();u&&(n.mutate({code:u}),s(""))};return l`
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
        ${CE(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function RE(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function CE(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function EE(e,t){return e?.channel==="slack"&&e.strategy===t}function F1({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return l`
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

      ${EE(e,"inbound_proof_code")?l`<${yc} action=${e.action} />`:l`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function TE(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):Mr.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:Mr.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:Mr.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:Mr.maxTotalBytes}:Mr}function z1(){let e=ya(),t=z({enabled:!!e,queryKey:["session"],queryFn:Wu,staleTime:5*6e4});return TE(t.data)}function bc({onSend:e,onCancel:t,disabled:a,canCancel:n=!1,initialText:r="",resetKey:s="",draftKey:i=jo,variant:o="dock",context:u={},statusText:c=""}){let d=k(),f=o==="hero",m=z1(),[p,y]=h.default.useState(()=>xp(i)),[b,w]=h.default.useState(()=>wp(i)),[g,v]=h.default.useState(""),[x,$]=h.default.useState(!1),[S,R]=h.default.useState(!1),[_,C]=h.default.useState(!1),U=h.default.useRef(null),O=h.default.useRef(null),B=h.default.useRef([]),A=h.default.useRef(Promise.resolve());h.default.useEffect(()=>{B.current=b},[b]);let K=h.default.useRef(null),te=h.default.useRef(null),ye=h.default.useCallback(()=>{te.current&&(window.clearTimeout(te.current),te.current=null);let ae=K.current;K.current=null,ae&&ae.scope===Nt()&&$p(ae.key,ae.text)},[]),ke=h.default.useCallback(()=>{te.current&&(window.clearTimeout(te.current),te.current=null),K.current=null},[]),Je=h.default.useCallback(()=>{let ae=U.current;ae&&(ae.style.height="auto",ae.style.height=`${Math.min(ae.scrollHeight,200)}px`)},[]);h.default.useEffect(()=>{Je()},[p,Je]),h.default.useEffect(()=>(y(xp(i)),()=>ye()),[i,ye]);let kt=h.default.useRef(i);h.default.useEffect(()=>{if(kt.current!==i){kt.current=i,w(wp(i)),v("");return}Ix(i,b)},[i,b]),h.default.useEffect(()=>{r&&(y(r),window.requestAnimationFrame(()=>{U.current&&(U.current.focus(),U.current.setSelectionRange(r.length,r.length))}))},[r,s]);let lt=h.default.useCallback(ae=>{a||!ae||ae.length===0||(A.current=A.current.then(async()=>{let{staged:xt,errors:bn}=await Mx(ae,{limits:m,existing:B.current,t:d});xt.length>0&&w(N=>{let E=[...N,...xt];return B.current=E,E}),v(bn.length>0?bn.join(" "):"")}).catch(()=>{v(d("chat.attachmentStagingFailed"))}))},[a,m,d]),xa=h.default.useCallback(ae=>{w(xt=>{let bn=xt.filter(N=>N.id!==ae);return B.current=bn,bn}),v("")},[]),$a=h.default.useCallback(()=>{a||O.current?.click()},[a]),ue=h.default.useCallback(ae=>{let xt=Array.from(ae.target.files||[]);lt(xt),ae.target.value=""},[lt]),ne=h.default.useCallback(async()=>{if(!(!p.trim()||a||x)){$(!0);try{await e(p.trim(),{attachments:b}),y(""),w([]),B.current=[],v(""),ke(),Kx(i),Qx(i),U.current&&(U.current.style.height="auto")}catch{}finally{$(!1)}}},[p,b,a,x,e,i,ke]),ze=h.default.useCallback(ae=>{let xt=ae.target.value;y(xt),K.current={key:i,text:xt,scope:Nt()},te.current&&window.clearTimeout(te.current),te.current=window.setTimeout(ye,300)},[i,ye]),Ae=h.default.useCallback(async()=>{if(!(!n||S||!t)){R(!0);try{await t()}finally{R(!1)}}},[n,S,t]),bt=h.default.useCallback(ae=>{ae.key==="Enter"&&!ae.shiftKey&&(ae.preventDefault(),ne())},[ne]),ve=h.default.useCallback(ae=>{let xt=Array.from(ae.clipboardData?.files||[]);xt.length>0&&(ae.preventDefault(),lt(xt))},[lt]),ge=h.default.useCallback(ae=>{ae.preventDefault(),C(!1);let xt=Array.from(ae.dataTransfer?.files||[]);xt.length>0&&lt(xt)},[lt]),ut=h.default.useCallback(ae=>{ae.preventDefault(),!a&&C(!0)},[a]),Ke=h.default.useCallback(ae=>{ae.currentTarget.contains(ae.relatedTarget)||C(!1)},[]),Rt=p.trim(),Ne=d(f?"chat.heroPlaceholder":"chat.followUpPlaceholder"),yn=m.accept.length>0?m.accept.join(","):void 0,Lt=f?"w-full":"px-4 py-3 sm:px-5 lg:px-8",ia=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",f?"min-h-[120px]":"",a?"opacity-70":""].join(" "),Jc=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",f?"min-h-[72px]":"min-h-[40px]"].join(" ");return l`
    <div className=${Lt}>
      <div
        className=${ia}
        onDrop=${ge}
        onDragOver=${ut}
        onDragLeave=${Ke}
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
              <${D} name="close" className="h-3.5 w-3.5" strokeWidth=${2} />
            </button>
          </div>
        `}

        ${b.length>0&&l`
          <div className="mb-2 flex flex-wrap gap-2 px-1">
            ${b.map(ae=>l`
                <div
                  key=${ae.id}
                  className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
                >
                  ${ae.previewUrl?l`<img
                        src=${ae.previewUrl}
                        alt=${ae.filename}
                        className="h-9 w-9 shrink-0 rounded object-cover"
                      />`:l`<span
                        className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                      >
                        <${D} name="file" className="h-4 w-4" />
                      </span>`}
                  <span className="flex min-w-0 flex-col">
                    <span className="max-w-[12rem] truncate font-medium">
                      ${ae.filename}
                    </span>
                    <span className="text-[10px] text-iron-400">${ae.sizeLabel}</span>
                  </span>
                  <button
                    type="button"
                    onClick=${()=>xa(ae.id)}
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
          ref=${U}
          data-testid="chat-composer"
          value=${p}
          onChange=${ze}
          onKeyDown=${bt}
          onPaste=${ve}
          placeholder=${Ne}
          rows=${1}
          disabled=${a}
          className=${Jc}
        />

        <input
          ref=${O}
          type="file"
          multiple
          accept=${yn}
          className="hidden"
          onChange=${ue}
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
              onClick=${$a}
              disabled=${a}
              aria-label=${d("chat.attachFiles")}
              title=${d("chat.attachFiles")}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <${D} name="plus" className="h-5 w-5" />
            </button>
            ${n?l`
                <${T}
                  type="button"
                  variant="danger"
                  size="icon-sm"
                  onClick=${Ae}
                  disabled=${S}
                  aria-label=${d("common.cancel")}
                  title=${d("common.cancel")}
                  className="rounded-full"
                >
                  <${D} name="close" className="h-5 w-5" />
                <//>
              `:l`
                <${T}
                  type="button"
                  variant="primary"
                  size="icon-sm"
                  onClick=${ne}
                  disabled=${a||x||!Rt}
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
  `}var q1={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function B1({status:e}){let t=k();if(e==="idle"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return l`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",q1[e]||q1.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function H1({onSuggestion:e,onSend:t,disabled:a,initialText:n,resetKey:r,draftKey:s,context:i,statusText:o,canCancel:u,onCancel:c}){let d=k(),f=[{icon:"tool",title:d("chat.suggestion1"),detail:d("chat.suggestion1Desc")},{icon:"shield",title:d("chat.suggestion2"),detail:d("chat.suggestion2Desc")},{icon:"plug",title:d("chat.suggestion3"),detail:d("chat.suggestion3Desc")}];return l`
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
        <${bc}
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
  `}var AE=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function K1({open:e,onClose:t}){let a=k();return e?l`
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
          ${AE.map((n,r)=>l`
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
  `:null}function Q1(e){let t=0,a=0,n=0,r=0;for(let i of e){if(i.role==="thinking"&&(t+=1),i.role==="tool_activity"){let o=I1([i]);a+=o.tools,n+=o.failed,r+=o.running}if(DE(i)){let o=I1(i.toolCalls);a+=o.tools,n+=o.failed,r+=o.running}}let s=[];return t&&s.push(`${t} reasoning`),a&&s.push(`${a} ${a===1?"tool":"tools"}`),n&&s.push(`${n} failed`),!n&&r&&s.push("running"),{hasError:n>0,label:`Activity${s.length?` - ${s.join(", ")}`:""}`}}function I1(e){let t=0,a=0;for(let n of e)n.toolStatus==="error"&&(t+=1),n.toolStatus==="running"&&(a+=1);return{tools:e.length,failed:t,running:a}}function DE(e){return e.toolCalls&&e.toolCalls.length>0}var V1=!1;function ME(){V1||!window.DOMPurify||(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),V1=!0)}function G1(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}ME();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var Fp=360;function OE(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let u=s("Copy");if(u.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),u.textContent="Copied",Qs("Code copied",{tone:"success"}),setTimeout(()=>u.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(u),n.appendChild(r),t.scrollHeight>Fp){t.style.maxHeight=`${Fp}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${Fp}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function LE({content:e,className:t=""}){let a=h.default.useRef(null),n=h.default.useMemo(()=>G1(e),[e]);return h.default.useEffect(()=>{OE(a.current)},[n]),l`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var Ye=h.default.memo(LE);var Y1={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",error:"bg-[var(--v2-danger-text)]"},UE={success:"ok",error:"err",running:"run"},jE=2;function Js({activity:e}){return e.toolCalls&&e.toolCalls.length>0?l`<${FE} tools=${e.toolCalls} />`:l`<${zE} activity=${e} />`}function PE(e,t){let a=0,n=0,r=0,s=0;for(let u of t){let c=String(u.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function FE({tools:e}){let t=k(),a=e.some(i=>i.toolStatus==="error"),[n,r]=h.default.useState(a);if(h.default.useEffect(()=>{a&&r(!0)},[a]),e.length<=jE)return l`
      <div className="flex flex-col gap-3">
        ${e.map((i,o)=>l`<${Js}
            key=${i.id||i.callId||`${i.toolName}-${o}`}
            activity=${i}
          />`)}
      </div>
    `;let s=PE(t,e);return l`
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
          ${e.map((i,o)=>l`<${Js}
              key=${i.id||i.callId||`${i.toolName}-${o}`}
              activity=${i}
            />`)}
        </div>
      `}
    </div>
  `}function zE({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:u}=e,[c,d]=h.default.useState(n==="error");h.default.useEffect(()=>{n==="error"&&d(!0)},[n]);let f=Y1[n]||Y1.running,m=i!=null,p=h.default.useId(),y=l`
    <button
      type="button"
      onClick=${()=>d(b=>!b)}
      aria-expanded=${c?"true":"false"}
      aria-controls=${p}
      className="v2-button flex w-full items-center gap-2.5 border-0 border-b border-iron-700/40 bg-transparent px-1 py-2 text-left text-sm"
    >
      <span className=${["h-2 w-2 shrink-0 rounded-full",f].join(" ")} />
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-wide text-iron-300"
        >${UE[n]||"run"}</span
      >
      <span className="shrink-0 truncate font-mono text-[13px] font-medium text-iron-100"
        >${a}</span
      >
      ${r&&l`<span className="min-w-0 truncate font-mono text-xs text-iron-400"
        >${r}</span
      >`}
      <span className="ml-auto flex shrink-0 items-center gap-2">
        ${n==="running"&&!m&&l`<span className="font-mono text-[11px] text-iron-300">…</span>`}
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
        ${y}
        ${c&&l`<${qE}
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
  `}function qE({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=k(),u=h.default.useMemo(()=>{let m=[];return r&&m.push({id:"error",label:o("tool.tabError")}),t&&m.push({id:"details",label:o("tool.tabDetails")}),a&&m.push({id:"params",label:o("tool.tabParameters")}),n&&m.push({id:"result",label:o("tool.tabResult")}),m},[o,r,t,a,n]),[c,d]=h.default.useState(null),f=c&&u.some(m=>m.id===c)?c:u[0]?.id;return h.default.useEffect(()=>{r&&d("error")},[r]),u.length===0?l`
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
        ${f==="result"&&l`<${BE} text=${n} />`}
        ${f==="error"&&l`<pre className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-danger-text)]">${r}</pre>`}
      </div>
    </div>
  `}function BE({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return l`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(HE)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return l`
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
                  >${KE(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?l`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:l`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function HE(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function KE(e){return e==null?"":String(e)}function X1({activity:e}){let t=Q1(e),[a,n]=h.default.useState(!1);return l`
    <div className="mr-auto flex w-full max-w-[85%] flex-col">
      <button
        type="button"
        onClick=${()=>n(r=>!r)}
        aria-expanded=${a?"true":"false"}
        className=${["v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",t.hasError?"text-[var(--v2-danger-text)]":"text-iron-400 hover:text-iron-200"].join(" ")}
      >
        <${D} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${t.label}</span>
        <${D}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0",a?"rotate-180":""].join(" ")}
        />
      </button>

      ${a&&l`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((r,s)=>l`
            <${IE}
              key=${r.id||`${r.role||"activity"}-${s}`}
              item=${r}
            />
          `)}
        </div>
      `}
    </div>
  `}function IE({item:e}){if(e.role==="thinking")return l`<${QE} content=${e.content} />`;if(e.role==="tool_activity"||J1(e)){let t=J1(e)?{id:e.id,toolCalls:e.toolCalls}:e;return l`<${Js} activity=${t} />`}return null}function QE({content:e}){return e?l`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <${D} name="spark" className="h-4 w-4" />
      </div>
      <div className="min-w-0 max-w-[85%] flex-1 border-l-2 border-white/10 pl-3 text-iron-300">
        <${Ye} content=${e} className="text-[13px]" />
      </div>
    </div>
  `:null}function J1(e){return e.toolCalls&&e.toolCalls.length>0}var VE={user:"U",assistant:"IC",system:"S"},Z1={user:"border border-signal/30 bg-signal text-iron-950",assistant:"border border-white/10 bg-iron-700 text-iron-100",system:"bg-copper text-iron-950"};function xc({role:e,className:t=""}){return l`
    <div
      className=${["flex h-7 w-7 shrink-0 items-center justify-center rounded-full font-mono text-[10px] font-semibold",Z1[e]||Z1.assistant,t].join(" ")}
    >
      ${VE[e]||"IC"}
    </div>
  `}function W1(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function GE({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=h.default.useState(()=>t&&e.preview_url||null);return h.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return px(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?l`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:l`<${D} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var ew="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",tw="px-3 py-2";function $c({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=h.default.useState(!1),o=h.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await Oo(e.fetch_url);W1(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),u=l`
    <${GE} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?l`<div
      className=${`${ew} ${tw} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${u}
    </div>`:l`<div className=${`${ew} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${tw} text-left transition-colors hover:bg-iron-900/80`}
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
  </div>`}var aw={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function Xs({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return h.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),h.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?l`
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
        className=${G("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",aw[n]??aw.md,r)}
      >
        ${a?l`<${zp} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function zp({children:e,onClose:t,className:a=""}){return l`
    <div
      className=${G("flex shrink-0 items-center justify-between gap-4","px-5 py-4 md:px-7 md:py-5","border-b border-[var(--v2-panel-border)]",a)}
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
  `}function Zs({children:e,className:t=""}){return l`
    <div className=${G("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function Ws({children:e,className:t=""}){return l`
    <div
      className=${G("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var nw=1e5;function wc({attachment:e,onClose:t}){let a=!!e,[n,r]=h.default.useState("loading"),[s,i]=h.default.useState({}),o=e?Dx(e.mime_type):"download";if(h.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return Oo(e.fetch_url).then(async f=>{d=URL.createObjectURL(f);let m={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")m.dataUrl=await dp(f);else if(o==="pdf")m.frameUrl=d;else if(o==="text"){let p=await f.text();m.truncated=p.length>nw,m.text=m.truncated?p.slice(0,nw):p}if(c){URL.revokeObjectURL(d);return}i(m),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let u=e.filename||"attachment";return l`
    <${Xs} open=${a} onClose=${t} size="xl">
      <${zp} onClose=${t}>
        <span className="block truncate">${u}</span>
      <//>
      <${Zs} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&l`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&l`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&l`<${YE} mode=${o} view=${s} filename=${u} />`}
      <//>
      <${Ws}>
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
  `}function YE({mode:e,view:t,filename:a}){switch(e){case"image":return l`<img
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
      </div>`}}var JE=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function XE(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function rw(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of XE(e).matchAll(JE)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function sw(e){return e.split("/").filter(Boolean).pop()||e}function iw(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function ZE({threadId:e,path:t,onPreview:a}){let[n,r]=h.default.useState({mime_type:"",size_label:""});h.default.useEffect(()=>{let i=!0;return rx({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:iw(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:sw(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:sx({threadId:e,path:t})};return l`<${$c}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function ow({threadId:e,content:t}){let a=h.default.useMemo(()=>rw(t),[t]),[n,r]=h.default.useState(null);return!e||a.length===0?null:l`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>l`<${ZE}
          key=${s}
          threadId=${e}
          path=${s}
          onPreview=${r}
        />`)}
      <${wc}
        attachment=${n}
        onClose=${()=>r(null)}
      />
    </div>
  `}var lw={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function WE(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function eT({content:e}){let[t,a]=h.default.useState(!1);return e?l`
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
          <${Ye} content=${e} className="text-[13px]" />
        </div>
      `}
    </div>
  `:null}function tT({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:u,status:c,error:d,toolCalls:f,timestamp:m}=e,p=n==="user",y=k(),[b,w]=h.default.useState(!1),[g,v]=h.default.useState(null),x=h.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),w(!0),Qs("Copied to clipboard",{tone:"success"}),setTimeout(()=>w(!1),1400)}catch{}},[r]);if(n==="tool_activity"||f&&f.length>0){let B=f&&f.length>0?{id:e.id,toolCalls:f}:e;return l`<${Js} activity=${B} />`}if(n==="thinking")return l`<${eT} content=${r} />`;if(n==="image")return l`
      <div className="flex">
        <div className="flex flex-wrap gap-2">
          ${(o||[]).map((A,K)=>A.data_url?l`<img key=${K} src=${A.data_url} className="max-h-64 rounded-lg border border-iron-700 object-cover" alt="Generated result" />`:l`
                  <div key=${K} className="rounded-lg border border-iron-700 bg-iron-900/70 px-4 py-3 text-sm text-iron-200">
                    <div>Generated image unavailable in history payload</div>
                    ${A.path&&l`<div className="mt-1 font-mono text-xs text-iron-300">${A.path}</div>`}
                  </div>
                `)}
        </div>
      </div>
    `;let $=WE(m),S=(n==="assistant"||n==="user")&&!u,_=p?"max-w-[85%]":n==="system"||n==="error"?"mx-auto max-w-[85%]":"w-full max-w-[85%]",C=p?"":"w-full min-w-0 max-w-full",U=n==="user"||n==="assistant",O=y(p?"chat.identityUser":"chat.identityAssistant");return l`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",p?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col gap-2",_].join(" ")}>
        ${U&&l`
          <div
            className=${["flex items-center gap-2 px-1",p?"flex-row-reverse":""].join(" ")}
          >
            <${xc} role=${n} />
            <span className="text-xs font-medium text-[var(--v2-text-muted)]">
              ${O}
            </span>
          </div>
        `}
        <div
          className=${["text-base leading-7",C,lw[n]||lw.assistant,u?"opacity-70":""].join(" ")}
        >
          ${n==="assistant"||n==="system"||n==="error"?l`<${Ye} content=${r} />`:l`<div className="whitespace-pre-wrap">${r}</div>`}

          ${c==="error"&&l`
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-300">
              <span>${d}</span>
            </div>
          `}

          ${s&&s.length>0&&l`
            <div className="mt-2 flex flex-wrap gap-2">
              ${s.map((B,A)=>l`<img key=${A} src=${B} className="max-h-48 rounded-lg border border-iron-700 object-cover" alt="Message attachment" />`)}
            </div>
          `}

          ${i&&i.length>0&&l`
            <div className="mt-2 flex flex-col gap-1.5">
              ${i.map((B,A)=>l`<${$c}
                key=${B.id||A}
                att=${B}
                onPreview=${v}
              />`)}
            </div>
            <${wc}
              attachment=${g}
              onClose=${()=>v(null)}
            />
          `}

          ${n==="assistant"&&l`<${ow}
            threadId=${a}
            content=${typeof r=="string"?r:""}
          />`}
        </div>

        ${(S||c==="error"||$)&&l`
          <div
            className=${["flex items-center gap-1.5 px-1 text-iron-400 opacity-0 group-hover:opacity-100 focus-within:opacity-100",p?"justify-end":"justify-start"].join(" ")}
          >
            ${S&&l`
              <button
                type="button"
                onClick=${x}
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
            ${$&&l`<span className="font-mono text-[10px] text-iron-500">${$}</span>`}
          </div>
        `}
      </div>
    </div>
  `}var uw=h.default.memo(tT);function fw(e){let t=aT(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(pw(r)){let s=cw(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){dw(a,s),mw(a,r),n+=s.length;continue}}if(qp(r)){let s=cw(t,n);dw(a,s),n+=s.length-1;continue}mw(a,r)}return a}function aT(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=Sc(i);o&&pw(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!qp(i))continue;let o=Sc(i),u=o?t.get(o):void 0;if(u===void 0||u>=s)continue;let c=a.get(u)||[];c.push(i),a.set(u,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function cw(e,t){let a=t,n=Sc(e[t]);for(;a<e.length&&qp(e[a])&&nT(n,e[a]);)a+=1;return e.slice(t,a)}function nT(e,t){let a=Sc(t);return!e||!a||a===e}function dw(e,t){t.length!==0&&e.push({type:"activity-run",id:`activity-run-${t[0].id}`,activity:t})}function mw(e,t){e.push({type:"message",id:t.id,message:t})}function pw(e){return e.role==="assistant"&&!hw(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function qp(e){return e.role==="thinking"||e.role==="tool_activity"||hw(e)}function hw(e){return e?.toolCalls&&e.toolCalls.length>0}function Sc(e){return e?.turnRunId||null}function vw({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let u=k(),c=h.default.useRef(null),d=h.default.useRef(!0),[f,m]=h.default.useState(!0);h.default.useEffect(()=>{if(!c.current||!d.current)return;let g=window.requestAnimationFrame(()=>{let v=c.current;v&&(v.scrollTop=v.scrollHeight)});return()=>window.cancelAnimationFrame(g)},[e,i]);let p=h.default.useCallback(()=>{let w=c.current;if(!w)return;let g=100,v=w.scrollHeight-w.scrollTop-w.clientHeight;d.current=v<g,m(v<g),a&&w.scrollTop<g&&n&&!t&&n()},[a,n,t]),y=h.default.useCallback(()=>{let w=c.current;w&&(w.scrollTop=w.scrollHeight,d.current=!0,m(!0))},[]),b=h.default.useMemo(()=>fw(e),[e]);return l`
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
        ${b.map(w=>w.type==="activity-run"?l`<${X1} key=${w.id} activity=${w.activity} />`:l`<${uw}
                key=${w.id}
                message=${w.message}
                onRetry=${r}
                threadId=${s}
              />`)}
        ${o}
      </div>
    </div>
    ${!f&&l`
      <button
        type="button"
        onClick=${y}
        aria-label=${u("chat.jumpToLatest")}
        className="absolute bottom-4 left-1/2 inline-flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-3 py-1.5 text-xs font-medium text-[var(--v2-text-strong)] shadow-[0_10px_30px_-12px_rgba(0,0,0,0.7)] hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]"
      >
        <${D} name="arrowDown" className="h-3.5 w-3.5" />
        ${u("chat.jumpToLatest")}
      </button>
    `}
    </div>
  `}function gw({notice:e,onRecover:t}){return l`
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
  `}function yw({suggestions:e,onSelect:t}){return!e||e.length===0?null:l`
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
  `}function bw(){let e=k();return l`
    <div className="flex flex-col items-start">
      <div className="flex min-w-0 max-w-[85%] flex-col gap-2">
        <div className="flex items-center gap-2 px-1">
          <${xc} role="assistant" />
          <span className="text-xs font-medium text-[var(--v2-text-muted)]">
            ${e("chat.identityAssistant")}
          </span>
        </div>
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
  `}function Nc(){return Z("/api/webchat/v2/channels/connectable")}function xw(e,t){if(!Bp(e))return null;let a=_c(e),n=oT(a),r=null;for(let s of t||[]){if(!iT(s))continue;let i=lT(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function Bp(e){let t=_c(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function rT(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function sT(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>$w(_c(n))):a}function iT(e){return e?.strategy!=="admin_managed_channels"}function oT(e){return ww(e,"slack")&&$w(e)}function $w(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function _c(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function lT(e,t,a={}){return(a.commandAliasesOnly?sT(t,{channelManagementOnly:!0}):rT(t)).reduce((r,s)=>{let i=_c(s);return ww(e,i)?Math.max(r,i.length):r},0)}function ww(e,t){return t?` ${e} `.includes(` ${t} `):!1}function Sw(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n=a?uT(a):[],r={kind:"gate",runId:t.turn_run_id,gateRef:t.gate_ref,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return a?{...r,toolName:a.tool_name||null,description:a.reason||t.body,actionLabel:a.action?.label||null,destination:a.destination||null,approvalScope:a.scope||null,approvalDetails:n,parameters:n.length?n.map(s=>`${s.label}: ${s.value}`).join(`
`):null}:r}return e==="auth_required"?{kind:"auth_required",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function uT(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function Nw({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function kw({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,onRunSettled:i}){let o=h.default.useRef(new Set),u=h.default.useRef(null),c=h.default.useRef(null);return h.default.useCallback(d=>{let{type:f,frame:m}=d||{};if(!(!f||!m))switch(f){case"accepted":{let p=m.ack||{};p.run_id&&(u.current=p.run_id),r?.({runId:p.run_id||null,threadId:p.thread_id||e,status:p.status||null}),a(!0);return}case"running":case"capability_progress":{let p=m.progress||{};p.turn_run_id&&(u.current=p.turn_run_id,r?.(y=>y&&y.runId===p.turn_run_id?y:{runId:p.turn_run_id,threadId:e,status:"running"}),fT(n,p.turn_run_id,c)),a(!0);return}case"capability_activity":{let p=m.activity;if(!p||!p.invocation_id)return;let y=yp(p);Cw(t,p.invocation_id,y);return}case"capability_display_preview":{let p=m.preview;if(!p||!p.invocation_id)return;let y=gp(p);vT(t,p.invocation_id,y);return}case"gate":case"auth_required":{let p=Sw(f,m.prompt);p&&(n(p),r?.({runId:p.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let p=m.reply||{};t(y=>[...y,{id:`reply-${p.turn_run_id||Date.now()}`,role:"assistant",content:p.text||"",timestamp:p.generated_at||new Date().toISOString(),turnRunId:p.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let p=m.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Hp(o,i,p,!1);return}case"failed":{let p=m.run_state||{},y=p.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Rw(t,{runId:y,status:p.status||"failed",failureCategory:hT(p),failureSummary:null}),Hp(o,i,y,!1);return}case"projection_snapshot":case"projection_update":{let p=m.state?.items||[];pT({items:p,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,activeRunRef:s});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i])}function Hp(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var cT=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),dT=new Set(["completed","succeeded"]),_w=new Set(["blocked_auth","blocked_approval","blocked_resource"]);function mT(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function fT(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function pT({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,activeRunRef:d}){let f=u?.current??null;for(let m of e){if(m.run_status){let{run_id:p,status:y,failure_category:b,failure_summary:w}=m.run_status,g=cT.has(y),v=d?.current?.source==="local"?d.current.runId:null,x=!!(p&&v&&v!==p),$=f??u?.current??null;if(x||!!(g&&p&&$&&$!==p))continue;p&&(f=p,!g&&u&&(u.current=p),s?.(R=>R&&R.runId===p?{...R,status:y}:{runId:p,threadId:t,status:y})),p&&_w.has(y)?c&&(c.current=p):p&&c?.current===p&&(c.current=null),g?(n(!1),r(null),s?.(null),f=null,u&&(u.current=null),p&&c?.current===p&&(c.current=null),Hp(o,i,p,dT.has(y)),(y==="failed"||y==="recovery_required")&&Rw(a,{runId:p,status:y,failureCategory:b,failureSummary:w})):_w.has(y)||(mT(r,p,c),n(!0))}if(m.text){let p=`text-${m.text.id}`;a(y=>{let b=y.findIndex(g=>g.id===p),w={id:p,role:"assistant",content:m.text.body||"",timestamp:new Date().toISOString(),isFinalReply:!0};if(b>=0){let g=[...y];return g[b]=w,g}return[...y,w]}),n(!1)}if(m.thinking){let p=`thinking-${m.thinking.id}`;a(y=>{let b=y.findIndex(g=>g.id===p),w={id:p,role:"thinking",content:m.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:m.thinking.run_id||null};if(b>=0){let g=[...y];return g[b]=w,g}return[...y,w]})}if(m.capability_activity){let p=m.capability_activity;if(p.invocation_id){let y=yp(p);Cw(a,p.invocation_id,y)}}if(m.gate&&f&&c?.current===f&&(r(p=>p||{kind:"gate",runId:f,gateRef:m.gate.gate_ref,headline:m.gate.headline,body:"",allowAlways:m.gate.allow_always===!0}),n(!1)),m.skill_activation){let{id:p,skill_names:y=[],feedback:b=[]}=m.skill_activation;if(y.length||b.length){let w=`skill-${p||y.join("-")||"activation"}`,g=[y.length?`Skill activated: ${y.join(", ")}`:"",...b].filter(Boolean).join(`
`);a(v=>v.some(x=>x.id===w)?v:[...v,{id:w,role:"system",content:g,timestamp:new Date().toISOString()}])}}}u&&f&&(u.current=f)}function hT(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function Rw(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`;e(i=>{let o=i.findIndex(c=>c.id===s),u=Nw({status:a,failureCategory:n,failureSummary:r});if(o>=0){if(!r||i[o].content===u)return i;let c=[...i];return c[o]={...c[o],content:u},c}return[...i,{id:s,role:"error",content:u,timestamp:new Date().toISOString()}]})}function vT(e,t,a){let n=`tool-${t}`,r={id:n,role:"tool_activity",...a};e(s=>{let i=s.findIndex(o=>o.id===n);if(i>=0){let o=[...s];return o[i]=r,o}return[...s,r]})}function Cw(e,t,a){let n=`tool-${t}`;e(r=>{let s=r.findIndex(i=>i.id===n);if(s>=0){let i=r[s],o=Px(i.toolStatus)&&a.toolStatus==="running"?i.toolStatus:a.toolStatus,u=[...r];return u[s]={...i,toolStatus:o,toolError:a.toolError||i.toolError,updatedAt:a.updatedAt||i.updatedAt,turnRunId:a.turnRunId||i.turnRunId||null},u}return[...r,{id:n,role:"tool_activity",...a}]})}function Ew(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function Tw(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function Aw(e,t,a,n){let r=yT(n);return r?(gT(e,t,a,{timelineMessageId:r}),r):null}function gT(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function yT(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var bT=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function Dw({threadId:e,onEvent:t,enabled:a}){let[n,r]=h.default.useState("idle"),s=h.default.useRef(t);s.current=t;let i=h.default.useRef(null);return h.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,u=null,c=0,d=3e4;function f(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=hx({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let b=Math.min(1e3*2**c,d);u=setTimeout(f,b)};let y=(b,w)=>{let g=null;try{g=JSON.parse(b.data)}catch{return}!g||typeof g!="object"||(b.lastEventId&&(i.current=b.lastEventId),s.current?.({type:g.type||w,frame:g,lastEventId:b.lastEventId||null}))};o.onmessage=b=>y(b,"message");for(let b of bT)o.addEventListener(b,w=>y(w,b))}function m(){u&&(clearTimeout(u),u=null),o&&(o.close(),o=null),r("paused")}function p(){document.visibilityState==="hidden"?m():o||f()}return f(),document.addEventListener("visibilitychange",p),()=>{document.removeEventListener("visibilitychange",p),u&&clearTimeout(u),o&&o.close()}},[a,e]),{status:n}}var xT=3e4,$T="credential_stored_gate_resolution_failed",wT="ironclaw-product-auth",Kp="ironclaw:product-auth:oauth-complete",ST="ironclaw:product-auth:oauth-complete";async function Mw(e){let t=new AbortController,a=setTimeout(()=>t.abort(),xT);try{return await e(t.signal)}finally{clearTimeout(a)}}function NT(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=$T,t.cause=e,t}function _T(e){let a=At.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function kT(e){return e?.continuation?.type==="turn_gate_resume"}function Ow(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function RT(e){return e?.type===ST&&e?.status==="completed"}function CT(e,t,a){if(!RT(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function Ip(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function ET(e){if(!Bp(e))return null;try{let a=(await At.fetchQuery({queryKey:["connectable-channels"],queryFn:Nc}))?.channels||[];return xw(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function Lw(e){let t=h.default.useRef(new Map),a=h.default.useRef(1),[n,r]=h.default.useState(0),[s,i]=h.default.useState(Date.now()),[o,u]=h.default.useState(null),c=h.default.useRef(o),d=h.default.useCallback(ue=>{let ne=typeof ue=="function"?ue(c.current):ue;c.current=ne,u(ne)},[]),[f,m]=h.default.useState(null),p=h.default.useCallback(()=>t.current.get(e||"__new__")||[],[e]),y=h.default.useCallback(ue=>{let ne=e||"__new__";ue.length>0?t.current.set(ne,ue):t.current.delete(ne)},[e]),{messages:b,hasMore:w,nextCursor:g,isLoading:v,loadError:x,loadHistory:$,setMessages:S}=Bx(e,{getPendingMessages:p,setPendingMessages:y}),[R,_]=h.default.useState(!1),[C,U]=h.default.useState(null),O=h.default.useRef({gateKey:null,credentialRef:null,inFlight:!1});h.default.useEffect(()=>{_(!1),U(null),d(null),m(null)},[e]);let B=Math.max(0,Math.ceil((n-s)/1e3)),A=C?.runId&&C?.gateRef?`${C.runId}
${C.gateRef}`:null;h.default.useEffect(()=>{if(!n)return;let ue=setInterval(()=>i(Date.now()),250);return()=>clearInterval(ue)},[n]),h.default.useEffect(()=>{O.current.gateKey!==A&&(O.current={gateKey:A,credentialRef:null,inFlight:!1})},[A]),h.default.useEffect(()=>{if(!Ow(C))return;let ue=Date.now(),ne=ve=>{CT(ve,C,ue)&&(U(ge=>Ow(ge)?null:ge),_(!0))},ze=null;typeof window.BroadcastChannel=="function"&&(ze=new window.BroadcastChannel(wT),ze.onmessage=ve=>ne(ve.data));let Ae=ve=>{ve.key===Kp&&ne(Ip(ve.newValue))};window.addEventListener("storage",Ae),ne(Ip(window.localStorage?.getItem?.(Kp)));let bt=window.setInterval(()=>{ne(Ip(window.localStorage?.getItem?.(Kp)))},500);return()=>{window.clearInterval(bt),ze&&ze.close(),window.removeEventListener("storage",Ae)}},[C]);let K=kw({threadId:e,setMessages:S,setIsProcessing:_,setPendingGate:U,setActiveRun:d,activeRunRef:c,onRunSettled:(ue,{success:ne})=>{ne&&y([]),$(void 0,{preserveClientOnly:!0})}}),{status:te}=Dw({threadId:e,onEvent:K,enabled:!!e}),ye=h.default.useCallback(async(ue,ne={})=>{let{threadId:ze,attachments:Ae=[]}=ne,bt=Ae.map(Ox),ve=Ae.map(Lx);if(Ae.length===0){let Ne=await ET(ue);if(Ne)return m(Ne),{channel_connect_action:Ne}}m(null);let ge=ze||e;if(!ge){let Ne=await ec();if(At.invalidateQueries({queryKey:["threads"]}),ge=Ne?.thread?.thread_id,!ge)throw new Error("createThread returned no thread_id")}let ut=ge,Ke={id:`pending-${a.current++}`,role:"user",content:ue,attachments:ve,timestamp:new Date().toISOString(),isOptimistic:!0};Ew(t.current,ut,Ke);let Rt=Ke.id;S(Ne=>[...Ne,{id:Rt,role:"user",content:ue,attachments:ve,timestamp:Ke.timestamp,isOptimistic:!0}]),_(!0),U(null);try{let Ne=await dx({threadId:ge,content:ue,attachments:bt});_T(ge)&&At.invalidateQueries({queryKey:["threads"]}),Ne?.run_id&&d({runId:Ne.run_id,threadId:Ne.thread_id||ge,status:Ne.status||null,source:"local"});let yn=Aw(t.current,ut,Rt,Ne?.accepted_message_ref);return yn&&S(Lt=>Lt.map(ia=>ia.id===Rt?{...ia,timelineMessageId:yn}:ia)),Ne?.outcome==="rejected_busy"&&(S(Lt=>Lt.map(ia=>ia.id===Rt?{...ia,isOptimistic:!1,status:"error"}:ia)),Ne?.notice&&S(Lt=>[...Lt,{id:`system-rejected-${a.current++}`,role:"system",content:Ne.notice,timestamp:new Date().toISOString(),isOptimistic:!1}]),_(!1)),Ne}catch(Ne){throw Ne.status===429&&r(Date.now()+TT(Ne)),S(yn=>yn.map(Lt=>Lt.id===Rt?{...Lt,isOptimistic:!1,status:"error",error:Ne.message}:Lt)),_(!1),Ne}finally{Tw(t.current,ut,Rt)}},[e,S]),ke=h.default.useCallback(async(ue,ne={})=>{if(!C)return;let{runId:ze,gateRef:Ae}=C;if(!ze||!Ae)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");await mp({threadId:e,runId:ze,gateRef:Ae,resolution:ue,always:ne.always,credentialRef:ne.credentialRef}),U(null),_(!0)},[C,e]),Je=h.default.useCallback(async ue=>{if(!C)throw new Error("auth gate is no longer pending");let{runId:ne,gateRef:ze,provider:Ae}=C;if(!ne||!ze||!Ae)throw new Error("auth gate is missing required credential metadata");let bt=C.accountLabel||`${Ae} credential`,ve=`${ne}
${ze}`;if(O.current.gateKey!==ve&&(O.current={gateKey:ve,credentialRef:null,inFlight:!1}),O.current.inFlight)throw new Error("auth token submission already in progress");O.current.inFlight=!0;try{let ge=O.current.credentialRef,ut=null;if(!ge){if(ut=await Mw(Ke=>gx({provider:Ae,accountLabel:bt,token:ue,threadId:e,runId:ne,gateRef:ze,signal:Ke})),ge=ut?.credential_ref,!ge)throw new Error("manual token submit returned no credential_ref");O.current.credentialRef=ge}if(!kT(ut))try{await Mw(Ke=>mp({threadId:e,runId:ne,gateRef:ze,resolution:"credential_provided",credentialRef:ge,signal:Ke}))}catch(Ke){throw NT(Ke)}O.current={gateKey:null,credentialRef:null,inFlight:!1},U(null),_(!0)}catch(ge){throw O.current.gateKey===ve&&(O.current.inFlight=!1),ge}},[C,e]),kt=h.default.useCallback(async ue=>{let ne=o?.runId;!ne||!e||(U(null),_(!1),d(null),await vx({threadId:e,runId:ne,reason:ue}))},[o,e]),lt=h.default.useCallback(()=>{w&&g&&$(g)},[w,g,$]),xa=h.default.useCallback(async(ue,ne,ze)=>{let Ae="approved",bt=!1;ne==="deny"?Ae="denied":ne==="cancel"?Ae="cancelled":ne==="always"&&(Ae="approved",bt=!0),await ke(Ae,{always:bt})},[ke]),$a=h.default.useCallback(()=>{},[]);return{messages:b,isProcessing:R,pendingGate:C,channelConnectAction:f,activeRun:o,sseStatus:te,historyLoading:v,historyLoadError:x,hasMore:w,cooldownSeconds:B,send:ye,resolveGate:ke,submitAuthToken:Je,cancelRun:kt,loadMore:lt,dismissChannelConnectAction:()=>m(null),suggestions:[],setSuggestions:$a,retryMessage:$a,approve:xa,recoverHistory:$a,recoveryNotice:null}}function TT(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function Uw({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}function AT(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function kc({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let u=o.toString(),c=`/logs${u?`?${u}`:""}`;return i?`/v2${c}`:c}function jw(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(AT),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}function Pw({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let o=k(),{messages:u,isProcessing:c,pendingGate:d,channelConnectAction:f,suggestions:m,sseStatus:p,historyLoading:y,historyLoadError:b,hasMore:w,cooldownSeconds:g,recoveryNotice:v,activeRun:x,send:$,cancelRun:S,retryMessage:R,approve:_,recoverHistory:C,loadMore:U,setSuggestions:O,submitAuthToken:B,dismissChannelConnectAction:A}=Lw(t),K=h.default.useMemo(()=>e.find(ve=>ve.id===t)||null,[e,t]),te=h.default.useMemo(()=>Uw({gatewayStatus:i,activeThread:K}),[i,K]),ye=u.length>0||c||!!d||!!f,ke=!y&&!ye&&!b,Je=c&&!d||g>0,kt=g>0?`Retry in ${g}s`:void 0,lt=t||jo,xa=!!(t&&x?.runId&&x.threadId===t&&c&&!d),$a=h.default.useMemo(()=>{if(!t)return null;let ve=x?.threadId===t?x.runId:null;return kc({threadId:t,runId:ve},{absolute:!0})},[x,t]),ue=h.default.useCallback(async(ve,{images:ge=[],attachments:ut=[]}={})=>{let Ke=await $(ve,{images:ge,attachments:ut,threadId:t}),Rt=Ke?.thread_id||t;return!t&&Rt&&a&&a(Rt,{replace:!0}),Ke},[t,a,$]),ne=h.default.useCallback(async ve=>{O([]),await ue(ve)},[ue,O]),ze=h.default.useCallback(()=>S("user_requested"),[S]);h.default.useEffect(()=>{t&&(d?lc(t,hn.NEEDS_ATTENTION):c?lc(t,hn.RUNNING):z$(t))},[t,d,c]);let[Ae,bt]=h.default.useState(!1);return h.default.useEffect(()=>{let ve=ge=>{if(ge.key==="Escape"){bt(!1);return}if(ge.key!=="?")return;let ut=ge.target,Ke=ut?.tagName;Ke==="INPUT"||Ke==="TEXTAREA"||ut?.isContentEditable||(ge.preventDefault(),bt(Rt=>!Rt))};return window.addEventListener("keydown",ve),()=>window.removeEventListener("keydown",ve)},[]),l`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${B1} status=${p} />

        ${$a&&l`
          <div className="flex justify-end border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-1.5">
            <a
              href=${$a}
              className="rounded-[6px] px-2 py-1 text-xs font-medium text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
            >
              ${o("nav.logs")}
            </a>
          </div>
        `}

        ${b&&l`
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            ${b}
          </div>
        `}

        ${ke&&l`
          <${H1}
            onSuggestion=${ne}
            onSend=${ue}
            disabled=${Je}
            initialText=${r}
            resetKey=${s}
            draftKey=${lt}
            context=${te}
            statusText=${kt}
            canCancel=${xa}
            onCancel=${ze}
          />
        `}
        ${!ke&&l`
          <${vw}
            messages=${u}
            isLoading=${y}
            hasMore=${w}
            onLoadMore=${U}
            onRetryMessage=${R}
            threadId=${t}
            pending=${c}
          >
            ${v&&l`
              <${gw}
                notice=${v}
                onRecover=${C}
              />
            `}
            ${c&&!d&&l`<${bw} />`}
            ${f&&l`
              <${F1}
                connectAction=${f}
                onDismiss=${A}
              />
            `}
            ${d&&(d.kind==="auth_required"?d.challengeKind==="oauth_url"?l`
                  <${U1}
                    gate=${d}
                    onCancel=${()=>_(d.requestId,"cancel",d.kind)}
                  />
                `:d.challengeKind==="manual_token"?l`
                  <${j1}
                    gate=${d}
                    onSubmit=${B}
                    onCancel=${()=>_(d.requestId,"cancel",d.kind)}
                  />
                `:l`
                  <${L1}
                    gate=${d}
                    onCancel=${()=>_(d.requestId,"cancel",d.kind)}
                  />
                `:l`
              <${O1}
                gate=${d}
                onApprove=${()=>_(d.requestId,"approve",d.kind)}
                onDeny=${()=>_(d.requestId,"deny",d.kind)}
                onAlways=${()=>_(d.requestId,"always",d.kind)}
              />
            `)}
          <//>

          <${yw}
            suggestions=${m}
            onSelect=${ne}
          />

          <${bc}
            onSend=${ue}
            disabled=${Je}
            initialText=${r}
            resetKey=${s}
            draftKey=${lt}
            context=${te}
            statusText=${kt}
            canCancel=${xa}
            onCancel=${ze}
          />
        `}
      </div>
      <${K1}
        open=${Ae}
        onClose=${()=>bt(!1)}
      />
    </div>
  `}function Qp(){let{threadsState:e,gatewayStatus:t}=Ba(),{threadId:a}=it(),n=fe(),r=je(),s=r.state?.composerDraft||"";h.default.useEffect(()=>{a&&a!==e.activeThreadId?e.setActiveThreadId(a):a||e.setActiveThreadId(null)},[a]);let i=h.default.useCallback((o,u={})=>{if(!o){e.setActiveThreadId(null),n("/chat",u);return}e.setActiveThreadId(o),n(`/chat/${o}`,u)},[e,n]);return l`
    <${Pw}
      threads=${e.threads}
      activeThreadId=${e.activeThreadId}
      onSelectThread=${i}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function Fw(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?Ks(e,t):"",model:e?ic(e,t):""}}function zw({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u}){let[c,d]=h.default.useState(()=>Fw(e,a)),[f,m]=h.default.useState(""),[p,y]=h.default.useState([]),[b,w]=h.default.useState(null),[g,v]=h.default.useState(""),x=h.default.useRef(!!e);h.default.useEffect(()=>{n&&(d(Fw(e,a)),m(""),y([]),w(null),v(""),x.current=!!e)},[n,e,a]);let $=e?.builtin===!0,S=e&&!e.builtin,R=h.default.useCallback((B,A)=>{d(K=>{let te={...K,[B]:A};return B==="name"&&!x.current&&(te.id=N$(A)),te})},[]),_=h.default.useCallback(()=>!$&&(!c.name.trim()||!c.id.trim())?u("llm.fieldsRequired"):!$&&!_$(c.id.trim())?u("llm.invalidId"):!S&&!$&&t.includes(c.id.trim())?u("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,$,S,u]),C=h.default.useCallback(async()=>{let B=_();if(B){w({tone:"error",text:B});return}v("save");try{await s({form:c,apiKey:f,provider:e}),r()}catch(A){w({tone:"error",text:A.message})}finally{v("")}},[f,c,r,s,e,_]),U=h.default.useCallback(async()=>{if(!c.model.trim()){w({tone:"error",text:u("llm.modelRequired")});return}v("test");try{let B=await i(_p(e,c,f,a));w({tone:B.ok?"success":"error",text:B.message})}catch(B){w({tone:"error",text:B.message})}finally{v("")}},[f,a,c,i,e,u]),O=h.default.useCallback(async()=>{if(($?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){w({tone:"error",text:u("llm.baseUrlRequired")});return}v("models");try{let A=await o(_p(e,c,f,a));if(!A.ok||!Array.isArray(A.models)||!A.models.length)w({tone:"error",text:A.message||u("llm.modelsFetchFailed")});else{y(A.models);let K=k$(c.model,A.models);K!==null&&R("model",K),w({tone:"success",text:u("llm.modelsFetched",{count:A.models.length})})}}catch(A){w({tone:"error",text:A.message})}finally{v("")}},[f,a,c,$,o,e,u,R]);return{form:c,apiKey:f,models:p,message:b,busy:g,isBuiltin:$,isEditing:S,setApiKey:m,update:R,submit:C,runTest:U,fetchModels:O,markIdEdited:()=>{x.current=!0}}}function Rc({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let u=k(),c=zw({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u});if(!n)return null;let{form:d,apiKey:f,models:m,message:p,busy:y,isBuiltin:b,isEditing:w}=c,g=b?u("llm.configureProvider",{name:e.name||e.id}):u(w?"llm.editProvider":"llm.newProvider");return l`
    <${Xs} open=${n} onClose=${r} title=${g} size="lg">
      <${Zs} className="space-y-4">
        ${!b&&l`
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${u("llm.providerName")}
              <${Mt} value=${d.name} onChange=${v=>c.update("name",v.target.value)} />
            </label>
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${u("llm.providerId")}
              <${Mt}
                value=${d.id}
                disabled=${w}
                onChange=${v=>{c.markIdEdited(),c.update("id",v.target.value)}}
              />
            </label>
          </div>
          <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
            ${u("llm.adapter")}
            <${Pp} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${Np.map(v=>l`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
            <//>
          </label>
        `}

        ${b&&l`
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            ${zo(e.adapter)}
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
            <${T} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${y!==""} onClick=${c.fetchModels}>
              ${u(y==="models"?"llm.fetchingModels":"llm.fetchModels")}
            <//>
          </div>
        </label>

        ${m.length>0&&l`
          <${Pp} value=${d.model} onChange=${v=>c.update("model",v.target.value)}>
            ${m.map(v=>l`<option key=${v} value=${v}>${v}</option>`)}
          <//>
        `}

        ${p&&l`
          <div className=${p.tone==="error"?"text-sm text-red-200":"text-sm text-mint"} role="status">
            ${p.text}
          </div>
        `}
      <//>
      <${Ws}>
        <${T} type="button" variant="secondary" disabled=${y!==""} onClick=${c.runTest}>
          ${u(y==="test"?"llm.testing":"llm.testConnection")}
        <//>
        <${T} type="button" variant="ghost" disabled=${y!==""} onClick=${r}>${u("common.cancel")}<//>
        <${T} type="button" disabled=${y!==""} onClick=${c.submit}>
          ${u(y==="save"?"common.saving":"common.save")}
        <//>
      <//>
    <//>
  `}function Cc({login:e}){let t=k(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return l`
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
  `}function DT(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function Ec({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=Is({settings:e,gatewayStatus:t}),[s,i]=h.default.useState(null),[o,u]=h.default.useState(!1),[c,d]=h.default.useState(null),f=h.default.useRef(null),m=h.default.useCallback((g,v)=>{f.current&&window.clearTimeout(f.current),d({tone:g,text:v}),f.current=window.setTimeout(()=>d(null),3500)},[]);h.default.useEffect(()=>()=>{f.current&&window.clearTimeout(f.current)},[]);let p=h.default.useCallback((g=null)=>{i(g),u(!0)},[]),y=h.default.useCallback(async g=>{try{await r.setActiveProvider(g),m("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(p(g),m("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):m("error",v.message)}},[p,r,m,n]),b=h.default.useCallback(async({form:g,apiKey:v,provider:x})=>{if(x?.builtin){await r.saveBuiltinProvider({provider:x,form:g,apiKey:v}),m("success",n("llm.providerConfigured",{name:x.name||x.id}));return}let $=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:x});m("success",n(x?"llm.providerUpdated":"llm.providerAdded",{name:$.name||$.id}))},[r,m,n]),w=h.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),m("success",n("llm.providerDeleted"))}catch(v){m("error",v.message)}},[r,m,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>DT(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:p,closeDialog:()=>u(!1),handleUse:y,handleSave:b,handleDelete:w}}var MT=3e5;function OT(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function LT(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function UT(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=u=>{let c=u.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},MT);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var jT=3e5,PT=9e5,FT=2e3;async function qw(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,FT)),(await sc().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function Tc({onSuccess:e}={}){let t=k(),a=Y(),[n,r]=h.default.useState(!1),[s,i]=h.default.useState(""),[o,u]=h.default.useState(!1),[c,d]=h.default.useState(""),[f,m]=h.default.useState(null),p=h.default.useCallback(()=>{i(""),d(""),m(null)},[]),y=h.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),b=h.default.useCallback(async v=>{if(p(),OT()){i(t("onboarding.nearaiLocalSso"));return}let x=window.open("about:blank","_blank");if(!x){i(t("onboarding.nearaiFailed"));return}try{x.opener=null}catch{}r(!0);try{let{auth_url:$}=await r$({provider:v,origin:window.location.origin});x.location.href=$;let S=await qw("nearai",jT,x);if(S==="active"){await y();return}x.close(),i(t(S==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{x.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[y,p,t]),w=h.default.useCallback(async()=>{p(),r(!0);try{let v=LT(),x=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!x){i(t("onboarding.nearaiFailed"));return}x.opener=null;let $=await UT(x,v);if(!$){i(t("onboarding.nearaiFailed"));return}await s$({account_id:$.accountId,public_key:$.publicKey,signature:$.signature,message:$.message,recipient:$.recipient,nonce:$.nonce}),await y()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[y,p,t]),g=h.default.useCallback(async()=>{p();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}u(!0);try{let{user_code:x,verification_uri:$}=await i$();m({userCode:x,verificationUri:$}),v&&(v.location.href=$);let S=await qw("openai_codex",PT,v);if(S==="active"){await y();return}v&&v.close(),d(t(S==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{u(!1)}},[y,p,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:f,startNearai:b,startNearaiWallet:w,startCodex:g}}var Bw="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",zT="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",qT="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",BT="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",HT={nearai:{color:"#00ec97",path:zT},openai_codex:{color:"#10a37f",path:Bw},openai:{color:"#10a37f",path:Bw},anthropic:{color:"#d97757",path:qT},ollama:{color:null,path:BT}};function Hw({id:e,name:t}){let a=HT[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return l`
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
  `}var KT=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function IT({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=h.default.useState(!1),o=h.default.useRef(null),u=t||a.nearaiBusy;h.default.useEffect(()=>{if(!s)return;let d=m=>{o.current&&!o.current.contains(m.target)&&i(!1)},f=m=>{m.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",f),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",f)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return l`
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
  `}function QT({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let u=s(e.nameKey),c;return e.auth==="nearai"?c=l`<${IT} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=l`
      <${T} type="button" variant="secondary" size="sm" disabled=${r.codexBusy} onClick=${r.startCodex}>
        ${s("onboarding.signIn")}
      <//>
    `:a?c=l`<${T} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>i(t)}>
      ${s("llm.use")}
    <//>`:c=l`<${T} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>o(t)}>
      ${s("onboarding.setUp")}
    <//>`,l`
    <${W} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-4">
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <${Hw} id=${e.id} name=${u} />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">${u}</span>
            ${a&&l`<${P} tone="positive" label=${s("onboarding.ready")} size="sm" />`}
          </div>
          <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">${s(e.descKey)}</div>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap gap-2 sm:justify-end">${c}</div>
    <//>
  `}function Kw(){let{isAdmin:e=!1,isChecking:t=!1}=Ba();return t?null:e?l`<${VT} />`:l`<${ot} to="/chat" replace />`}function VT(){let e=k(),t=fe(),a=Y(),{gatewayStatus:n}=Ba(),r=Ec({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=KT.map(f=>({entry:f,provider:s.providers.find(m=>m.id===f.id)})).filter(f=>f.provider),o=h.default.useCallback(()=>t("/chat"),[t]),u=Tc({onSuccess:o}),c=h.default.useCallback(async f=>{let m=f.active_model||f.default_model||"";await Fo({provider_id:f.id,model:m}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=h.default.useCallback(async({form:f,apiKey:m,provider:p})=>{await r.handleSave({form:f,apiKey:m,provider:p});let y=p?.id||f.id.trim(),b=f.model?.trim()||p?.default_model||"";await Fo({provider_id:y,model:b}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?l`
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
              <${QT}
                key=${f.id}
                entry=${f}
                provider=${m}
                configured=${Lr(m,s.builtinOverrides)}
                isBusy=${s.isBusy}
                login=${u}
                t=${e}
                onUse=${c}
                onSetUp=${r.openDialog}
              />
            `)}
        </div>

        <${Cc} login=${u} />

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

      <${Rc}
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
  `}var Iw={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function Ia({result:e,onDismiss:t}){return e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",Iw[e.type]||Iw.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}function j({children:e,className:t="",...a}){return l`<${W} className=${t} ...${a}>${e}<//>`}function tt({label:e,value:t,tone:a="muted",badgeLabel:n,detail:r,showDivider:s=!0,className:i="",valueClassName:o="text-[1.75rem] md:text-[2rem]"}){return l`
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
          ${r&&l`<div className="mt-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            ${r}
          </div>`}
        </div>
        <${P} tone=${a} label=${n??a} />
      </div>
    </div>
  `}function Qw({items:e}){return l`
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
  `;return n?l`<${W} padding="lg">${r}<//>`:l`<div className="py-8">${r}</div>`}function Vo(e=""){return Promise.resolve({entries:[],todo:!0})}function Vw(e){return Promise.resolve({content:"",todo:!0})}function Gw(e){return Promise.resolve({success:!1,message:"TODO: requires v2 workspace endpoint"})}function Yw(e,t=20){return Promise.resolve({matches:[],todo:!0})}var Jw="README.md";function Ac(e){return e?e.split("/").filter(Boolean):[]}function Dc(e){return e?`/workspace/${Ac(e).map(encodeURIComponent).join("/")}`:"/workspace"}function Vp(e){let t=Ac(e);return t.pop(),t.join("/")}function Xw(e){return/\.mdx?$/i.test(e||"")}function Zw(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not indexed"}function Ww(e,t,a=140){let n=String(e||""),r=String(t||"").trim().toLowerCase();if(!r)return n.slice(0,a);let s=n.toLowerCase().indexOf(r);if(s<0)return n.slice(0,a);let i=Math.max(0,s-Math.floor(a/2)),o=Math.min(n.length,i+a);return`${i>0?"...":""}${n.slice(i,o)}${o<n.length?"...":""}`}function Gp(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function e2({entry:e,depth:t,selectedPath:a,expandedPaths:n,onToggleDirectory:r,onSelectFile:s}){let i=k(),o=n.has(e.path),u=z({queryKey:["workspace-list",e.path],queryFn:()=>Vo(e.path),enabled:e.is_dir&&o});return e.is_dir?l`
      <div>
        <button
          type="button"
          onClick=${()=>r(e.path)}
          className="flex min-h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm text-iron-200 hover:bg-white/[0.05] hover:text-white"
          style=${{paddingLeft:`${8+t*16}px`}}
          aria-expanded=${o}
        >
          <span className=${["w-3 text-[10px]",o?"rotate-90":""].join(" ")}>></span>
          <span className="min-w-0 truncate font-semibold">${e.name}</span>
        </button>
        ${o&&l`
          <div className="space-y-1">
            ${u.isLoading?l`<div className="px-4 py-2 text-xs text-iron-400">${i("workspace.loading")}</div>`:(u.data?.entries||[]).filter(c=>!Gp(c.path)).map(c=>l`
                  <${e2}
                    key=${c.path}
                    entry=${c}
                    depth=${t+1}
                    selectedPath=${a}
                    expandedPaths=${n}
                    onToggleDirectory=${r}
                    onSelectFile=${s}
                  />
                `)}
          </div>
        `}
      </div>
    `:l`
    <button
      type="button"
      onClick=${()=>s(e.path)}
      className=${["flex min-h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm",a===e.path?"bg-signal/10 text-signal":"text-iron-300 hover:bg-white/[0.05] hover:text-white"].join(" ")}
      style=${{paddingLeft:`${24+t*16}px`}}
    >
      <span className="min-w-0 truncate">${e.name}</span>
    </button>
  `}function t2({entries:e,selectedPath:t,expandedPaths:a,onToggleDirectory:n,onSelectFile:r,isLoading:s}){let i=k();if(s)return l`<div className="space-y-2 p-3">${[1,2,3,4].map(u=>l`<div key=${u} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let o=e.filter(u=>!Gp(u.path));return o.length?l`
    <div className="space-y-1 p-2">
      ${o.map(u=>l`
        <${e2}
          key=${u.path}
          entry=${u}
          depth=${0}
          selectedPath=${t}
          expandedPaths=${a}
          onToggleDirectory=${n}
          onSelectFile=${r}
        />
      `)}
    </div>
  `:l`<div className="px-4 py-8 text-sm text-iron-300">${i("workspace.noFiles")}</div>`}function a2({results:e,query:t,onSelectFile:a,isSearching:n}){let r=k();if(n)return l`<div className="p-4 text-sm text-iron-300">${r("workspace.searching")}</div>`;let s=e.filter(i=>!Gp(i.path));return s.length?l`
    <div className="space-y-2 p-2">
      ${s.map(i=>l`
        <button
          key=${i.path}
          type="button"
          onClick=${()=>a(i.path)}
          className="w-full rounded-md border border-white/8 bg-white/[0.025] p-3 text-left hover:border-signal/25 hover:bg-white/[0.05]"
        >
          <div className="flex items-center justify-between gap-2">
            <div className="min-w-0 truncate font-mono text-xs text-signal">${i.path}</div>
            <${P} tone="muted" label=${Number(i.score||0).toFixed(2)} />
          </div>
          <div className="mt-2 line-clamp-2 text-xs leading-5 text-iron-300">${Ww(i.content,t)}</div>
        </button>
      `)}
    </div>
  `:l`<div className="p-4 text-sm text-iron-300">${r("workspace.noResults")}</div>`}function n2({search:e,onSearchChange:t,rootEntries:a,selectedPath:n,expandedPaths:r,searchResults:s,isLoadingTree:i,isSearching:o,onToggleDirectory:u,onSelectFile:c}){let d=k(),f=e.trim().length>0;return l`
    <${j} className="flex min-h-[420px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="border-b border-white/10 p-3">
        <input
          value=${e}
          onInput=${m=>t(m.target.value)}
          placeholder=${d("workspace.searchPlaceholder")}
          className="h-11 w-full rounded-md border border-white/10 bg-iron-950/80 px-3 text-sm text-white outline-none placeholder:text-iron-400 focus:border-signal/45"
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        ${f?l`
              <${a2}
                results=${s}
                query=${e}
                onSelectFile=${c}
                isSearching=${o}
              />
            `:l`
              <${t2}
                entries=${a}
                selectedPath=${n}
                expandedPaths=${r}
                onToggleDirectory=${u}
                onSelectFile=${c}
                isLoading=${i}
              />
            `}
      </div>
    <//>
  `}function GT({path:e,onNavigate:t}){let a=k(),n=Ac(e),r="";return l`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button type="button" onClick=${()=>t("/workspace")} className="text-signal hover:underline">${a("workspace.breadcrumbRoot")}</button>
      ${n.map(s=>{r=r?`${r}/${s}`:s;let i=r;return l`
          <span key=${i} className="text-iron-400">/</span>
          <button
            key=${`${i}-button`}
            type="button"
            onClick=${()=>t(Dc(i))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${s}
          </button>
        `})}
    </div>
  `}function r2({path:e,file:t,draft:a,onDraftChange:n,editing:r,onStartEdit:s,onCancelEdit:i,onSave:o,isLoading:u,isSaving:c,onNavigate:d}){let f=k();return u?l`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `:t?l`
    <${j} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${GT} path=${e} onNavigate=${d} />
        <div className="flex items-center gap-2">
          <${P} tone="muted" label=${Zw(t.updated_at)} />
          ${r?l`
                <${T} variant="ghost" size="sm" onClick=${i} disabled=${c}>${f("workspace.cancel")}<//>
                <${T} size="sm" onClick=${o} disabled=${c}>${f(c?"workspace.saving":"workspace.save")}<//>
              `:l`<${T} variant="secondary" size="sm" onClick=${s}>${f("workspace.edit")}<//>`}
        </div>
      </div>

      ${r?l`
            <div className="min-h-0 flex-1 p-4">
              <textarea
                value=${a}
                onInput=${m=>n(m.target.value)}
                className="h-full min-h-[460px] w-full resize-none rounded-xl border border-white/10 bg-iron-950/80 p-4 font-mono text-sm leading-6 text-white outline-none focus:border-signal/45"
                spellCheck=${!1}
              />
            </div>
          `:l`
            <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
              ${Xw(e)?l`<${Ye} content=${t.content} className="max-w-4xl text-base leading-7" />`:l`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
            </div>
          `}

      ${Vp(e)&&l`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${f("workspace.parent",{path:Vp(e)})}
        </div>
      `}
    <//>
  `:l`
      <${he}
        title=${f("workspace.pickFileTitle")}
        description=${f("workspace.pickFileDesc")}
      />
    `}function s2(e){let t=k(),a=Y(),[n,r]=h.default.useState(new Set),[s,i]=h.default.useState(""),[o,u]=h.default.useState(!1),[c,d]=h.default.useState(""),[f,m]=h.default.useState(null),p=z({queryKey:["workspace-list",""],queryFn:()=>Vo("")}),y=z({queryKey:["workspace-file",e],queryFn:()=>Vw(e),enabled:!!e}),b=z({queryKey:["workspace-search",s.trim()],queryFn:()=>Yw(s.trim(),20),enabled:s.trim().length>0});h.default.useEffect(()=>{y.data?.content!=null&&!o&&d(y.data.content)},[o,y.data?.content]),h.default.useEffect(()=>{u(!1),m(null)},[e]);let w=h.default.useCallback(x=>a.fetchQuery({queryKey:["workspace-list",x],queryFn:()=>Vo(x)}),[a]),g=h.default.useCallback(async x=>{let $=new Set(n);if($.has(x)){$.delete(x),r($);return}$.add(x),r($);try{await w(x)}catch(S){m({type:"error",message:S.message||t("workspace.unableOpenDirectory")})}},[n,w]),v=I({mutationFn:()=>Gw({path:e,content:c}),onSuccess:()=>{u(!1),m({type:"success",message:t("workspace.savedPath",{path:e})}),a.invalidateQueries({queryKey:["workspace-file",e]}),a.invalidateQueries({queryKey:["workspace-list"]})},onError:x=>{m({type:"error",message:x.message||t("workspace.unableSaveFile")})}});return{rootEntries:p.data?.entries||[],file:y.data||null,searchResults:b.data?.results||[],expandedPaths:n,search:s,setSearch:i,editing:o,setEditing:u,draft:c,setDraft:d,result:f,clearResult:()=>m(null),isLoadingTree:p.isLoading,isLoadingFile:y.isLoading,isSearching:b.isFetching,isSaving:v.isPending,error:p.error||y.error||b.error||null,loadDirectory:w,toggleDirectory:g,save:v.mutateAsync,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function Yp(){let e=k(),t=fe(),n=it()["*"]||Jw,r=s2(n),s=h.default.useCallback(o=>{t(Dc(o))},[t]),i=h.default.useCallback(async()=>{try{await r.save()}catch{}},[r]);return l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="flex h-full min-h-0 flex-col space-y-5">
          ${r.error&&l`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${r.error.message}
            </div>
          `}
          <${Ia}
            result=${r.result}
            onDismiss=${r.clearResult}
          />

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <${n2}
              search=${r.search}
              onSearchChange=${r.setSearch}
              rootEntries=${r.rootEntries}
              selectedPath=${n}
              expandedPaths=${r.expandedPaths}
              searchResults=${r.searchResults}
              isLoadingTree=${r.isLoadingTree}
              isSearching=${r.isSearching}
              onToggleDirectory=${r.toggleDirectory}
              onSelectFile=${s}
            />
            <${r2}
              path=${n}
              file=${r.file}
              draft=${r.draft}
              onDraftChange=${r.setDraft}
              editing=${r.editing}
              onStartEdit=${()=>r.setEditing(!0)}
              onCancelEdit=${()=>r.setEditing(!1)}
              onSave=${i}
              isLoading=${r.isLoadingFile}
              isSaving=${r.isSaving}
              onNavigate=${t}
            />
          </div>
        </div>
      </div>
    </div>
  `}function i2(){return Promise.resolve({projects:[],todo:!0})}function o2(e){return Promise.resolve(null)}function l2(e){return Promise.resolve({missions:[],todo:!0})}function u2(e){return Promise.resolve({threads:[],todo:!0})}function c2(e){return Promise.resolve({widgets:[],todo:!0})}function d2(e){return Promise.resolve(null)}function m2(e){return Promise.resolve(null)}function f2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function p2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function h2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function v2(){let e=Y(),t=z({queryKey:["projects-overview"],queryFn:i2,refetchInterval:5e3}),a=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function g2(e){let t=Y(),a=!!e,n=z({queryKey:["project-detail",e],queryFn:()=>o2(e),enabled:a,refetchInterval:a?7e3:!1}),r=z({queryKey:["project-missions",e],queryFn:()=>l2(e),enabled:a,refetchInterval:a?5e3:!1}),s=z({queryKey:["project-threads",e],queryFn:()=>u2(e),enabled:a,refetchInterval:a?4e3:!1}),i=z({queryKey:["project-widgets",e],queryFn:()=>c2(e),enabled:a,refetchInterval:a?15e3:!1}),o=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data?.project||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function y2({projectId:e,missionId:t,threadId:a}){let n=Y(),[r,s]=h.default.useState(null),i=z({queryKey:["project-mission-detail",t],queryFn:()=>d2(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=z({queryKey:["project-thread-detail",a],queryFn:()=>m2(a),enabled:!!a,refetchInterval:a?4e3:!1}),u=h.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=I({mutationFn:({targetMissionId:m})=>f2(m),onSuccess:m=>{s({type:"success",message:m?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to fire mission"})}}),d=I({mutationFn:({targetMissionId:m})=>p2(m),onSuccess:()=>{s({type:"success",message:"Mission paused."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to pause mission"})}}),f=I({mutationFn:({targetMissionId:m})=>h2(m),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:f.mutateAsync,isBusy:c.isPending||d.isPending||f.isPending}}function ba(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function ei(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function Wn(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function ti(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function Go(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function Mc(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function YT(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function Oc(e){let t=YT(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function b2(e){let t=e?.projects||[],a=t.reduce((o,u)=>o+Number(u.cost_today_usd||0),0),n=t.reduce((o,u)=>o+Number(u.active_missions||0),0),r=t.reduce((o,u)=>o+Number(u.threads_today||0),0),s=t.reduce((o,u)=>o+Number(u.pending_gates||0),0),i=t.reduce((o,u)=>o+Number(u.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function Lc(e=[]){return e.reduce((t,a)=>(a?.status==="Active"?t.active+=1:a?.status==="Paused"?t.paused+=1:a?.status==="Completed"?t.completed+=1:a?.status==="Failed"&&(t.failed+=1),t),{active:0,paused:0,completed:0,failed:0})}function Ot(e,t){return`${e} ${t}${e===1?"":"s"}`}function x2(e){if(!e)return"";if(typeof e.content=="string")return e.content;if(e.content==null)return"";try{return JSON.stringify(e.content,null,2)}catch{return String(e.content)}}function $2(e){if(!e)return"Not set";let t=e.unit?` ${e.unit}`:"",a=e.current!=null?`${e.current}${t}`:"Not set",n=e.target!=null?`${e.target}${t}`:null;return n?`${a} / ${n}`:a}var JT={projects:"muted",missions:"signal",attention:"warning",spend:"success"};function w2({overview:e}){let t=b2(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"missions",label:"Active missions",value:t.activeMissions,detail:`${t.pendingGates} gates across the workspace`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:Wn(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return l`
    <${j} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${P} tone=${JT[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function XT(e){return e?.type==="failure"?"danger":"warning"}function ZT(e){return e?.type==="failure"?"failure":"gate"}function S2({items:e,onOpenItem:t}){return e?.length?l`
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
              <${P} tone=${XT(a)} label=${ZT(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open workspace
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function WT({project:e,onOpen:t,t:a}){return l`
    <article className="group rounded-xl border border-iron-700 bg-iron-800/60 p-5 hover:border-signal/30 hover:bg-iron-800/80">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="truncate font-serif text-2xl font-semibold tracking-[-0.03em] text-iron-100">${e.name}</h3>
          <p className="mt-2 line-clamp-3 text-sm leading-6 text-iron-300">
            ${e.description||a("projects.noDescription")}
          </p>
        </div>
        <${P} tone=${ti(e.health)} label=${e.health||"unknown"} />
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
          <div className="mt-2 text-sm text-iron-100">${Ot(e.active_missions||0,"mission")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${a("projects.card.threadsToday",{count:Ot(e.threads_today||0,"thread")})}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${a("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">${Ot(e.pending_gates||0,"gate")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${a("projects.card.failures24h",{count:Ot(e.failures_24h||0,"failure")})}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>${a("projects.card.spendToday",{value:Wn(e.cost_today_usd||0)})}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${ei(e.last_activity)}</div>
        </div>
        <${T} variant="secondary" onClick=${()=>t(e.id)}>${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function eA({project:e,onOpen:t,t:a}){return l`
    <${j} className="overflow-hidden p-5 sm:p-6">
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
            ${Ot(e.active_missions||0,"active mission")}
          </div>
          <div className="rounded-2xl border border-iron-700 bg-iron-950/55 px-4 py-3 text-sm text-iron-200">
            ${Ot(e.threads_today||0,"thread")} today
          </div>
          <${T} variant="secondary" onClick=${()=>t(e.id)}>${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function N2({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=k(),u=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return!e.length&&t>0?l`
      <${he}
        title=${o("projects.empty.noMatchTitle")}
        description=${o("projects.empty.noMatchDesc")}
      />
    `:e.length?l`
    <div className="space-y-5">
      ${u&&l`<${eA} project=${u} onOpen=${r} t=${o} />`}

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
            <${T} onClick=${s}>${o(i?"projects.preparingChat":"projects.newProject")}<//>
          </div>
        </div>
      <//>

      ${c.length?l`<div className="grid gap-4 xl:grid-cols-2 2xl:grid-cols-3">
            ${c.map(d=>l`<${WT} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
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
    `}function tA({widget:e,projectId:t}){let a=h.default.useRef(null),[n,r]=h.default.useState("");return h.default.useEffect(()=>{let s=a.current;if(!s||!e)return;let i=null;try{s.innerHTML="",e.css&&(i=document.createElement("style"),i.textContent=e.css,document.head.appendChild(i));let o=window.IronClaw?.api||null;new Function("container","api","projectId",e.js)(s,o,t),r("")}catch(o){console.error("[v2-projects] failed to mount widget",e?.manifest?.id,o),r(`Unable to mount ${e?.manifest?.name||"project widget"}.`)}return()=>{s.innerHTML="",i&&i.remove()}},[t,e]),l`
    <div className="rounded-[20px] border border-white/10 bg-white/[0.03] p-4">
      <div className="mb-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${e.manifest?.slot||"project widget"}</div>
        <div className="mt-1 text-lg font-semibold tracking-tight text-white">${e.manifest?.name||e.manifest?.id}</div>
      </div>
      ${n?l`<p className="rounded-xl border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${n}</p>`:l`<div ref=${a} />`}
    </div>
  `}function _2({widgets:e,projectId:t}){return e?.length?l`
    <${j} className="p-4 sm:p-5">
      <div className="mb-4">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Widgets</div>
        <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">Project instrumentation</h2>
      </div>
      <div className="grid gap-4 xl:grid-cols-2">
        ${e.map(a=>l`<${tA} key=${a.manifest?.id} widget=${a} projectId=${t} />`)}
      </div>
    <//>
  `:null}function k2({missions:e,selectedMissionId:t,onSelectMission:a}){let n=Lc(e);return l`
    <${j} className="p-4 sm:p-5">
      <div className="flex items-end justify-between gap-4">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Missions</div>
          <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">Project execution plan</h2>
        </div>
        <div className="text-right text-xs uppercase tracking-[0.16em] text-iron-400">
          <div>${n.active} active / ${n.paused} paused</div>
          <div className="mt-1">${n.completed} completed / ${n.failed} failed</div>
        </div>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(r=>l`
              <button
                key=${r.id}
                onClick=${()=>a(r.id)}
                className=${["w-full rounded-[20px] border p-4 text-left",t===r.id?"border-signal/35 bg-signal/10":"border-white/10 bg-white/[0.025] hover:border-signal/25 hover:bg-white/[0.045]"].join(" ")}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="truncate text-lg font-semibold text-white">${r.name}</div>
                    <p className="mt-2 line-clamp-2 text-sm leading-6 text-iron-300">${r.goal}</p>
                  </div>
                  <${P} tone=${Go(r.status)} label=${r.status} />
                </div>
                <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                  <span>${r.cadence_description||r.cadence_type||"manual"}</span>
                  <span>${r.thread_count} threads</span>
                  <span>Updated ${ba(r.updated_at)}</span>
                </div>
              </button>
            `):l`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                This project does not have any missions yet. Use the chat workspace to describe the operating loop you want IronClaw to run.
              </div>
            `}
      </div>
    <//>
  `}function R2({threads:e,selectedThreadId:t,onSelectThread:a}){let n=[...e].sort((r,s)=>new Date(s.updated_at||s.created_at)-new Date(r.updated_at||r.created_at));return l`
    <${j} className="p-4 sm:p-5">
      <div>
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Activity</div>
        <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">Recent project runs</h2>
      </div>

      <div className="mt-5 space-y-3">
        ${n.length?n.slice(0,18).map(r=>{let s=Oc(r);return l`
                <button
                  key=${r.id}
                  onClick=${()=>a(r.id)}
                  className=${["w-full rounded-[20px] border p-4 text-left",t===r.id?"border-signal/35 bg-signal/10":"border-white/10 bg-white/[0.025] hover:border-signal/25 hover:bg-white/[0.045]"].join(" ")}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="truncate text-base font-semibold text-white">${s.title}</div>
                      <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-400">${s.subtitle}</div>
                      ${s.brief?l`<p className="mt-3 line-clamp-2 text-sm leading-6 text-iron-300">${s.brief}</p>`:null}
                    </div>
                    <${P} tone=${Mc(r.state)} label=${r.state} />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                    <span>${r.step_count||0} steps</span>
                    <span>${r.total_tokens||0} tokens</span>
                    <span>${ei(r.updated_at||r.created_at)}</span>
                  </div>
                </button>
              `}):l`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                No project threads yet. When a mission runs or you open scoped chat work inside this project, activity will appear here.
              </div>
            `}
      </div>
    <//>
  `}function Uc({label:e,value:t}){return l`
    <div className="rounded-2xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function C2({mission:e,onFire:t,onPause:a,onResume:n,onOpenThread:r,isBusy:s}){let i=[];return e.status==="Active"?(i.push(l`<${T} key="fire" onClick=${()=>t(e.id)} disabled=${s}>Fire now<//>`),i.push(l`<${T} key="pause" variant="secondary" onClick=${()=>a(e.id)} disabled=${s}>Pause<//>`)):e.status==="Paused"?(i.push(l`<${T} key="resume" onClick=${()=>n(e.id)} disabled=${s}>Resume<//>`),i.push(l`<${T} key="fire" variant="secondary" onClick=${()=>t(e.id)} disabled=${s}>Run once<//>`)):i.push(l`<${T} key="retry" onClick=${()=>t(e.id)} disabled=${s}>Run again<//>`),l`
    <div className="space-y-4">
      <${j} className="p-4 sm:p-5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Mission dossier</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
          </div>
          <${P} tone=${Go(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${Uc} label="Cadence" value=${e.cadence_description||e.cadence_type||"manual"} />
          <${Uc} label="Threads today" value=${`${e.threads_today||0} / ${e.max_threads_per_day||"\u221E"}`} />
          <${Uc} label="Next fire" value=${e.next_fire_at?ba(e.next_fire_at):"Not scheduled"} />
          <${Uc} label="Created" value=${ba(e.created_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">${i}</div>
      <//>

      <${j} className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Mission brief</div>
        <div className="mt-4 text-sm leading-6 text-iron-200">
          <${Ye} content=${e.goal||"No mission goal set."} />
        </div>
      <//>

      ${e.current_focus?l`
            <${j} className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Current focus</div>
              <div className="mt-4 text-sm leading-6 text-iron-200">
                <${Ye} content=${e.current_focus} />
              </div>
            <//>
          `:null}

      ${e.success_criteria?l`
            <${j} className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Success criteria</div>
              <div className="mt-4 text-sm leading-6 text-iron-200">
                <${Ye} content=${e.success_criteria} />
              </div>
            <//>
          `:null}

      ${e.approach_history?.length?l`
            <${j} className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Approach history</div>
              <div className="mt-4 space-y-3">
                ${e.approach_history.map((o,u)=>l`
                  <div key=${u} className="rounded-2xl border border-white/8 bg-iron-950/60 p-4">
                    <div className="mb-3 text-xs uppercase tracking-[0.16em] text-iron-400">Run ${u+1}</div>
                    <${Ye} content=${o} />
                  </div>
                `)}
              </div>
            <//>
          `:null}

      ${e.threads?.length?l`
            <${j} className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Spawned threads</div>
              <div className="mt-4 space-y-3">
                ${e.threads.map(o=>l`
                  <button
                    key=${o.id}
                    onClick=${()=>r(o.id)}
                    className="w-full rounded-2xl border border-white/8 bg-iron-950/60 p-4 text-left hover:border-signal/30 hover:bg-white/[0.05]"
                  >
                    <div className="flex items-center justify-between gap-3">
                      <div className="min-w-0 truncate text-sm font-semibold text-white">${o.goal}</div>
                      <${P} tone=${Go(o.state==="Running"?"Active":o.state==="Failed"?"Failed":"Completed")} label=${o.state} />
                    </div>
                  </button>
                `)}
              </div>
            <//>
          `:null}
    </div>
  `}function ai({label:e,value:t}){return l`
    <div className="rounded-2xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function E2({thread:e}){let t=Oc(e);return l`
    <div className="space-y-4">
      <${j} className="p-4 sm:p-5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${t.subtitle}</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">${t.title}</h2>
          </div>
          <${P} tone=${Mc(e.state)} label=${e.state} />
        </div>

        ${t.brief?l`
              <div className="mt-4 rounded-2xl border border-mint/15 bg-mint/10 p-4">
                <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-mint">Mission brief</div>
                <div className="mt-3 text-sm leading-6 text-iron-100">
                  <${Ye} content=${t.brief} />
                </div>
              </div>
            `:null}

        <div className="mt-5 grid gap-3 sm:grid-cols-2">
          <${ai} label="Thread type" value=${e.thread_type||"mission_run"} />
          <${ai} label="Steps" value=${e.step_count||0} />
          <${ai} label="Tokens" value=${(e.total_tokens||0).toLocaleString()} />
          <${ai} label="Spend" value=${e.total_cost_usd?Wn(e.total_cost_usd):"Not measured"} />
          <${ai} label="Created" value=${ba(e.created_at)} />
          <${ai} label="Completed" value=${e.completed_at?ba(e.completed_at):"Still running"} />
        </div>
      <//>

      <${j} className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Timeline</div>
        <div className="mt-4 space-y-3">
          ${e.messages?.length?e.messages.map((a,n)=>l`
                <article key=${n} className="rounded-2xl border border-white/8 bg-iron-950/60 p-4">
                  <div className="text-xs uppercase tracking-[0.16em] text-iron-400">${a.role||"System"}</div>
                  <div className="mt-3 text-sm leading-6 text-iron-100">
                    <${Ye} content=${x2(a)} />
                  </div>
                </article>
              `):l`
                <div className="rounded-2xl border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                  No messages were captured for this thread.
                </div>
              `}
        </div>
      <//>
    </div>
  `}function aA({project:e,missions:t,threads:a,overview:n}){let r=Lc(t);return l`
    <div className="space-y-4">
      <${j} className="p-4 sm:p-5">
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Project snapshot</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
          </div>
          <${P} tone=${ti(n?.health)} label=${n?.health||"steady"} />
        </div>
        <p className="mt-4 text-sm leading-6 text-iron-200">${e.description||"No project description yet."}</p>

        <div className="mt-5 grid gap-3 sm:grid-cols-2">
          <div className="rounded-2xl border border-white/8 bg-iron-950/60 p-3 text-sm text-iron-100">
            ${Ot(r.active,"active mission")} / ${Ot(r.paused,"paused mission")}
          </div>
          <div className="rounded-2xl border border-white/8 bg-iron-950/60 p-3 text-sm text-iron-100">
            ${Ot(a.length,"thread")} / ${Ot(n?.pending_gates||0,"gate")}
          </div>
        </div>
      <//>

      ${e.goals?.length?l`
            <${j} className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Goals</div>
              <div className="mt-4 space-y-2 text-sm leading-6 text-iron-200">
                ${e.goals.map((s,i)=>l`<div key=${i} className="rounded-2xl border border-white/8 bg-iron-950/60 px-3 py-2">${s}</div>`)}
              </div>
            <//>
          `:null}

      ${e.metrics?.length?l`
            <${j} className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Metrics</div>
              <div className="mt-4 space-y-3">
                ${e.metrics.map((s,i)=>l`
                  <div key=${i} className="rounded-2xl border border-white/8 bg-iron-950/60 p-3">
                    <div className="text-sm font-semibold text-white">${s.name}</div>
                    <div className="mt-2 text-sm text-iron-200">${$2(s)}</div>
                    ${s.updated_at&&l`
                      <div className="mt-2 font-mono text-[10px] uppercase tracking-[0.16em] text-iron-400">
                        Updated ${ba(s.updated_at)}
                      </div>
                    `}
                  </div>
                `)}
              </div>
            <//>
          `:null}
    </div>
  `}function T2({project:e,overview:t,missions:a,threads:n,inspector:r,isLoading:s,error:i,onClear:o,onOpenThread:u,onFireMission:c,onPauseMission:d,onResumeMission:f,isBusy:m}){return l`
    <aside className="space-y-4">
      <div className="flex items-center justify-between gap-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Inspector</div>
        ${r?.type&&l`<${T} variant="ghost" className="h-8 px-3 text-xs" onClick=${o}>Clear focus<//>`}
      </div>

      ${s?l`<div className="space-y-4">${[1,2].map(p=>l`<div key=${p} className="v2-skeleton h-48 rounded-[20px]" />`)}</div>`:i?l`<div className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">${i.message}</div>`:r?.type==="mission"?l`
                <${C2}
                  mission=${r.mission}
                  onFire=${c}
                  onPause=${d}
                  onResume=${f}
                  onOpenThread=${u}
                  isBusy=${m}
                />
              `:r?.type==="thread"?l`<${E2} thread=${r.thread} />`:l`<${aA} project=${e} missions=${a} threads=${n} overview=${t} />`}
    </aside>
  `}function A2({project:e,overview:t,missions:a,threads:n,widgets:r,selectedMissionId:s,selectedThreadId:i,inspector:o,inspectorState:u,onSelectMission:c,onSelectThread:d,onClearInspector:f}){return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]">
      <div className="space-y-5">
        <${j} className="overflow-hidden p-5 sm:p-6">
          <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
            <div className="min-w-0 max-w-3xl">
              <div className="flex flex-wrap items-center gap-3">
                <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-signal">Project workspace</div>
                <${P} tone=${ti(t?.health)} label=${t?.health||"steady"} />
              </div>
              <h2 className="mt-3 text-3xl font-semibold tracking-tight text-white">${e.name}</h2>
              <p className="mt-3 text-sm leading-6 text-iron-200">
                ${e.description||"This project is active, but it does not have a human-authored description yet."}
              </p>
            </div>

            <div className="grid gap-3 sm:grid-cols-2 xl:w-[320px] xl:grid-cols-1">
              <div className="rounded-2xl border border-white/10 bg-iron-950/60 px-4 py-3 text-sm text-iron-100">
                ${Ot(t?.active_missions||a.length,"active mission")}
              </div>
              <div className="rounded-2xl border border-white/10 bg-iron-950/60 px-4 py-3 text-sm text-iron-100">
                ${Ot(t?.threads_today||0,"thread")} today
              </div>
              <div className="rounded-2xl border border-white/10 bg-iron-950/60 px-4 py-3 text-sm text-iron-100">
                ${Wn(t?.cost_today_usd||0)} spend today
              </div>
              <div className="rounded-2xl border border-white/10 bg-iron-950/60 px-4 py-3 text-sm text-iron-100">
                ${ei(t?.last_activity)}
              </div>
            </div>
          </div>

          <div className="mt-5 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
            <div className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
              <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">Created</div>
              <div className="mt-2 text-sm leading-6 text-white">${ba(e.created_at)}</div>
            </div>
            <div className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
              <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">Pending gates</div>
              <div className="mt-2 text-sm leading-6 text-white">${t?.pending_gates||0}</div>
            </div>
            <div className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
              <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">Failures 24h</div>
              <div className="mt-2 text-sm leading-6 text-white">${t?.failures_24h||0}</div>
            </div>
            <div className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
              <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">Total missions</div>
              <div className="mt-2 text-sm leading-6 text-white">${t?.total_missions||a.length}</div>
            </div>
          </div>
        <//>

        <${_2} widgets=${r} projectId=${e.id} />

        <div className="grid gap-5 2xl:grid-cols-2">
          <${k2}
            missions=${a}
            selectedMissionId=${s}
            onSelectMission=${c}
          />
          <${R2}
            threads=${n}
            selectedThreadId=${i}
            onSelectThread=${d}
          />
        </div>
      </div>

      <${T2}
        project=${e}
        overview=${t}
        missions=${a}
        threads=${n}
        inspector=${o}
        isLoading=${u.isLoading}
        error=${u.error}
        onClear=${f}
        onOpenThread=${d}
        onFireMission=${u.fireMission}
        onPauseMission=${u.pauseMission}
        onResumeMission=${u.resumeMission}
        isBusy=${u.isBusy}
      />
    </div>
  `}function Yo(){let e=k(),t=fe(),{threadsState:a}=Ba(),{projectId:n=null,missionId:r=null,threadId:s=null}=it(),[i,o]=h.default.useState(""),[u,c]=h.default.useState(null),d=v2(),f=g2(n),m=y2({projectId:n,missionId:r,threadId:s}),p=h.default.useMemo(()=>{let C=i.trim().toLowerCase();return C?d.overview.projects.filter(U=>[U.name,U.description,...U.goals||[]].some(O=>String(O||"").toLowerCase().includes(C))):d.overview.projects},[d.overview.projects,i]),y=h.default.useMemo(()=>d.overview.projects.find(C=>C.id===n)||null,[d.overview.projects,n]),b=h.default.useCallback(()=>{d.invalidate(),f.invalidate()},[d,f]),w=h.default.useCallback(C=>{t(`/projects/${C}`)},[t]),g=h.default.useCallback(C=>{if(C.thread_id){t(`/projects/${C.project_id}/threads/${C.thread_id}`);return}t(`/projects/${C.project_id}`)},[t]),v=h.default.useCallback(async()=>{let C=null;c(null);try{C=await a.createThread()}catch(U){c({type:"error",message:U.message||e("projects.chatAutoFail")})}t("/chat",{state:{composerDraft:e("projects.creationDraft"),threadId:C}})},[t,a]),x=h.default.useCallback(C=>{t(`/projects/${n}/missions/${C}`)},[t,n]),$=h.default.useCallback(C=>{t(`/projects/${n}/threads/${C}`)},[t,n]),S=h.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),R=l`
    ${n&&l`<${T} variant="ghost" onClick=${()=>t("/projects")}>${e("projects.allProjects")}<//>`}
    <${T} onClick=${v}>
      ${a.isCreating?e("projects.preparingChat"):e("projects.newProject")}
    <//>
  `,_=null;return n?f.isLoading?_=l`
        <div className="space-y-4">
          ${[1,2,3].map(C=>l`<div key=${C} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `:f.error||!f.project&&!y?_=l`
        <${he}
          title=${e("projects.unavailable")}
          description=${f.error?.message||e("projects.unavailableDesc")}
        >
          <${T} variant="secondary" onClick=${()=>t("/projects")}>${e("projects.returnToProjects")}<//>
        <//>
      `:_=l`
        <${A2}
          project=${f.project||y}
          overview=${y||f.project}
          missions=${f.missions}
          threads=${f.threads}
          widgets=${f.widgets}
          selectedMissionId=${r}
          selectedThreadId=${s}
          inspector=${{type:m.inspectorType,mission:m.mission,thread:m.thread}}
          inspectorState=${m}
          onSelectMission=${x}
          onSelectThread=${$}
          onClearInspector=${S}
        />
      `:_=d.isLoading?l`
          <div className="space-y-4">
            ${[1,2,3].map(C=>l`<div key=${C} className="v2-skeleton h-40 rounded-[20px]" />`)}
          </div>
        `:l`
          <${N2}
            projects=${p}
            totalProjects=${d.overview.projects.length}
            search=${i}
            onSearchChange=${o}
            onOpenProject=${w}
            onCreateProject=${v}
            isPreparingChat=${a.isCreating}
          />
        `,l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <div className="flex flex-wrap justify-end gap-2">
            ${R}
          </div>
          ${d.error&&l`
            <div className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
              ${d.error.message}
            </div>
          `}
          <${Ia} result=${u} onDismiss=${()=>c(null)} />
          <${Ia} result=${m.actionResult} onDismiss=${m.clearActionResult} />
          <${w2} overview=${d.overview} />
          <${S2} items=${d.overview.attention} onOpenItem=${g} />
          ${_}
        </div>
      </div>
    </div>
  `}function Jo(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not scheduled"}function Xo(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function D2(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function M2(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);return r!==0?r:new Date(n.updated_at||0).getTime()-new Date(a.updated_at||0).getTime()})}function jc({label:e,value:t}){return l`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function nA({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=k();return e.status==="Active"?l`
      <${T} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.fireNow")}<//>
      <${T} variant="secondary" onClick=${()=>n(e.id)} disabled=${t}>${s("missions.action.pause")}<//>
    `:e.status==="Paused"?l`
      <${T} onClick=${()=>r(e.id)} disabled=${t}>${s("missions.action.resume")}<//>
      <${T} variant="secondary" onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runOnce")}<//>
    `:l`<${T} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runAgain")}<//>`}function O2({mission:e,isLoading:t,error:a,isBusy:n,onFire:r,onPause:s,onResume:i,onOpenProject:o,onOpenThread:u}){let c=k();return t?l`
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
          <${P} tone=${Xo(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${jc} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${jc} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${jc} label=${c("missions.meta.nextFire")} value=${Jo(e.next_fire_at)} />
          <${jc} label=${c("missions.meta.updated")} value=${Jo(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${nA}
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
          <${Ye} content=${e.goal||c("missions.noGoal")} />
        </div>
      <//>

      ${e.current_focus&&l`
        <${j} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${Ye} content=${e.current_focus} />
          </div>
        <//>
      `}

      ${e.success_criteria&&l`
        <${j} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${Ye} content=${e.success_criteria} />
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
                  <${P} tone=${Xo(d.state==="Running"?"Active":d.state==="Failed"?"Failed":"Completed")} label=${d.state} />
                </div>
              </button>
            `)}
          </div>
        <//>
      `:null}
    </div>
  `}function rA(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function L2({value:e,onChange:t,children:a,label:n}){return l`
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
  `}function sA({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=k(),s=t===e.id;return l`
    <div
      className=${["w-full rounded-xl border p-4 text-left",s?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80"].join(" ")}
    >
      <button type="button" onClick=${()=>a(e.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${e.name}</div>
              <${P} tone=${Xo(e.status)} label=${e.status} />
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
  `}function Jp({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:u,projectOptions:c,onSelectMission:d,onOpenProject:f}){let m=k(),p=rA(m);return l`
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
          onChange=${y=>r(y.target.value)}
          placeholder=${m("missions.searchPlaceholder")}
          className="h-11 min-w-[220px] flex-1 rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/40"
        />
        <${L2} value=${s} onChange=${i} label=${m("missions.filter.status")}>
          ${p.map(y=>l`<option key=${y.value} value=${y.value}>${y.label}<//>`)}
        <//>
        <${L2} value=${o} onChange=${u} label=${m("missions.filter.project")}>
          <option value="all">${m("missions.filter.allProjects")}</option>
          ${c.map(y=>l`<option key=${y.id} value=${y.id}>${y.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(y=>l`
              <${sA}
                key=${y.id}
                mission=${y}
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
  `}function iA(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function U2({summary:e}){let t=k(),a=iA(t);return l`
    <${j} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${P} tone=${n.tone} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${e[n.key]||0}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">
              ${n.key==="total"?t("missions.summary.completedFailed",{completed:e.completed||0,failed:e.failed||0}):t("missions.summary.acrossProjects")}
            </p>
          </div>
        `)}
      </div>
    <//>
  `}function j2(){return Promise.resolve({projects:[],todo:!0})}function P2({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function F2(e){return Promise.resolve(null)}function z2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function q2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function B2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function H2(e){let t=z({queryKey:["mission-detail",e],queryFn:()=>F2(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function oA(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function K2(){let e=Y(),[t,a]=h.default.useState(null),n=z({queryKey:["projects-overview"],queryFn:j2,refetchInterval:7e3}),r=n.data?.projects||[],s=md({queries:r.map(m=>({queryKey:["missions","project",m.id],queryFn:()=>P2({projectId:m.id}),refetchInterval:5e3,select:p=>p?.missions||[]}))}),i=s.flatMap((m,p)=>{let y=r[p];return(m.data||[]).map(b=>oA(b,y))}),o=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),u=(m,p)=>({mutationFn:({missionId:y})=>m(y),onSuccess:()=>{a({type:"success",message:p}),o()},onError:y=>{a({type:"error",message:y.message||"Unable to update mission"})}}),c=I(u(z2,"Mission fired and a run was queued.")),d=I(u(q2,"Mission paused.")),f=I(u(B2,"Mission resumed."));return{projects:r,missions:i,summary:D2(i),isLoading:n.isLoading||s.some(m=>m.isLoading),isRefreshing:n.isFetching||s.some(m=>m.isFetching),error:n.error||s.find(m=>m.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:f.mutateAsync,isBusy:c.isPending||d.isPending||f.isPending,invalidate:o}}function Xp(){let e=k(),t=fe(),{missionId:a=null}=it(),[n,r]=h.default.useState(""),[s,i]=h.default.useState("all"),[o,u]=h.default.useState("all"),c=K2(),d=H2(a),f=h.default.useMemo(()=>{let g=n.trim().toLowerCase();return M2(c.missions).filter(v=>{let x=!g||[v.name,v.goal,v.project?.name].some(R=>String(R||"").toLowerCase().includes(g)),$=s==="all"||v.status===s,S=o==="all"||v.project?.id===o;return x&&$&&S})},[c.missions,o,n,s]),m=h.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),p=d.mission?{...m,...d.mission,project:m?.project||null}:m,y=h.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),b=h.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),w=a?l`
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <${Jp}
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
          <${O2}
            mission=${p}
            isLoading=${d.isLoading}
            error=${d.error}
            isBusy=${c.isBusy}
            onFire=${g=>b(c.fireMission,g)}
            onPause=${g=>b(c.pauseMission,g)}
            onResume=${g=>b(c.resumeMission,g)}
            onOpenProject=${g=>t(`/projects/${g}`)}
            onOpenThread=${y}
          />
        </div>
      `:l`
        <${Jp}
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

          <${Ia}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${U2} summary=${c.summary} />

          ${c.isLoading?l`
                <div className="space-y-4">
                  ${[1,2,3].map(g=>l`<div
                        key=${g}
                        className="v2-skeleton h-32 rounded-xl"
                      />`)}
                </div>
              `:w}
        </div>
      </div>
    </div>
  `}var I2=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],lA=new Set(["pending","in_progress"]),Q2=new Set(["failed","interrupted","stuck","cancelled"]);function er(e){return e?String(e).replace(/_/g," "):"unknown"}function ni(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":Q2.has(e)?"danger":"muted":"muted"}function uA(e){return lA.has(e)}function Pc(e){return uA(e?.state)}function V2(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":Q2.has(e.state):!1}function jr(e,t=8){return e?String(e).slice(0,t):"unknown"}function sa(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function G2(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function Zp(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${sa(e.started_at)}`:null].filter(Boolean).join(" / ")}var cA=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function Y2(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function dA({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?l`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${t==="tool_use"?a.tool_name||"Tool call":a.tool_name||"Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${Y2(t==="tool_use"?a.input:a.output||a.error||a)}</pre>
      </details>
    `:t==="message"?l`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a.role||"assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${a.content||""}</div>
      </div>
    `:l`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t.replace(/_/g," ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${a.message||a.status||Y2(a)}</div>
    </div>
  `}function J2({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=k(),[s,i]=h.default.useState("all"),[o,u]=h.default.useState(""),[c,d]=h.default.useState(!0),f=h.default.useRef(null),m=h.default.useMemo(()=>s==="all"?t:t.filter(y=>y.event_type===s),[t,s]);h.default.useEffect(()=>{c&&f.current&&(f.current.scrollTop=f.current.scrollHeight)},[c,m.length]);let p=h.default.useCallback(async(y=!1)=>{let b=o.trim();if(!(!b&&!y))try{await a({content:b||"(done)",done:y}),u("")}catch{}},[o,a]);return l`
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
            onChange=${y=>i(y.target.value)}
            className="v2-select h-10 rounded-md border border-white/10 bg-iron-950/90 px-3 text-sm text-white outline-none focus:border-signal/45"
          >
            ${cA.map(y=>l`<option key=${y.value} value=${y.value}>${y.label}</option>`)}
          </select>
          <label className="flex items-center gap-2 text-sm text-iron-300">
            <input type="checkbox" checked=${c} onChange=${y=>d(y.target.checked)} />
            Auto-scroll
          </label>
        </div>
      </div>

      <div ref=${f} className="mt-5 max-h-[56vh] space-y-3 overflow-y-auto rounded-[18px] border border-white/10 bg-iron-950/78 p-4">
        ${m.length?m.map(y=>l`
              <div key=${y.id||`${y.event_type}-${y.created_at}`}>
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${sa(y.created_at)}</div>
                <${dA} event=${y} />
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
            onInput=${y=>u(y.target.value)}
            onKeyDown=${y=>{y.key==="Enter"&&!y.shiftKey&&(y.preventDefault(),p(!1))}}
            placeholder=${r("job.followupPlaceholder")}
            className="h-11 rounded-md border border-white/10 bg-iron-950/90 px-3 text-sm text-white outline-none focus:border-signal/45"
          />
          <${T} variant="secondary" disabled=${n} onClick=${()=>p(!0)}>${r("common.done")}<//>
          <${T} variant="primary" disabled=${n} onClick=${()=>p(!1)}>${r("common.send")}<//>
        </div>
      `}
    <//>
  `}function X2({job:e,activeTab:t,onTabChange:a,onBack:n,onCancel:r,onRestart:s,isBusy:i,children:o}){return l`
    <div className="space-y-5">
      <${j} className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick=${n} className="text-sm text-signal hover:text-white">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-semibold tracking-tight text-white">${e.title||"Untitled job"}</h2>
              <${P} tone=${ni(e.state)} label=${er(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${jr(e.id)}</span>
              <span>created ${sa(e.created_at)}</span>
              ${Zp(e)&&l`<span>${Zp(e)}</span>`}
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
            ${Pc(e)&&l`
              <${T} variant="secondary" disabled=${i} onClick=${()=>r(e.id)}>Cancel<//>
            `}
            ${V2(e)&&l`
              <${T} variant="primary" disabled=${i} onClick=${()=>s(e.id)}>Restart<//>
            `}
          </div>
        </div>
      <//>

      <div className="flex flex-wrap gap-2">
        ${I2.map(u=>l`
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
  `}function Z2({nodes:e,depth:t=0,selectedPath:a,expandingPath:n,onToggleDirectory:r,onSelectPath:s}){return l`
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
        ${i.isDir&&i.expanded&&i.children?.length?l`<${Z2}
              nodes=${i.children}
              depth=${t+1}
              selectedPath=${a}
              expandingPath=${n}
              onToggleDirectory=${r}
              onSelectPath=${s}
            />`:null}
      </div>
    `)}
  `}function W2({canBrowse:e,tree:t,selectedPath:a,selectedFile:n,fileError:r,isLoadingTree:s,isLoadingFile:i,expandingPath:o,treeError:u,onToggleDirectory:c,onSelectPath:d}){return e?l`
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <${j} className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          ${u&&l`<div className="mx-2 mb-3 rounded-md border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${u}</div>`}
          ${s?l`<div className="space-y-2 px-2">${[1,2,3,4].map(f=>l`<div key=${f} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?l`
                  <${Z2}
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
    `}function ri({label:e,value:t}){return l`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function eS({job:e}){let t=(e.transitions||[]).map(a=>({title:`${er(a.from)} -> ${er(a.to)}`,description:[sa(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${j} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${P} tone=${ni(e.state)} label=${er(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${ri} label="Created" value=${sa(e.created_at)} />
          <${ri} label="Started" value=${sa(e.started_at)} />
          <${ri} label="Completed" value=${sa(e.completed_at)} />
          <${ri} label="Duration" value=${G2(e.elapsed_secs)} />
          <${ri} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${ri} label="Mode" value=${e.job_mode||"Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${j} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${e.description?l`<${Ye} content=${e.description} className="mt-4 text-sm leading-7 text-iron-200" />`:l`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
        <//>

        ${t.length?l`
              <${j} className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Transitions</div>
                <h3 className="mt-2 text-xl font-semibold text-white">State timeline</h3>
                <div className="mt-3">
                  <${Qw} items=${t} />
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
  `}function tS({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:u,isBusy:c,isRefreshing:d}){let f=k(),m=[{value:"all",label:f("jobs.list.filter.all")},{value:"pending",label:f("jobs.list.filter.pending")},{value:"in_progress",label:f("jobs.list.filter.inProgress")},{value:"completed",label:f("jobs.list.filter.completed")},{value:"failed",label:f("jobs.list.filter.failed")},{value:"stuck",label:f("jobs.list.filter.stuck")}];if(!e.length){let p=!!n.trim()||s!=="all";return l`
      <${he}
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
                  <${P} tone=${ni(p.state)} label=${er(p.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${jr(p.id)}</span>
                  <span>${f("jobs.list.created",{value:sa(p.created_at)})}</span>
                  ${p.started_at&&l`<span>${f("jobs.list.started",{value:sa(p.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${Pc(p)&&l`
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
  `}var mA=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function aS({summary:e}){return l`
    <${j} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${mA.map(t=>l`
          <div
            key=${t.key}
            className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
          >
            <${tt}
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
  `}function nS(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function rS(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function sS(e){return Promise.resolve(null)}function iS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function oS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function lS(e){return Promise.resolve({events:[],todo:!0})}function uS(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function Wp(e,t=""){return Promise.resolve({entries:[],todo:!0})}function cS(e,t){return Promise.resolve({content:"",todo:!0})}function dS(e){let t=Y(),[a,n]=h.default.useState(null),r=z({queryKey:["job-detail",e],queryFn:()=>sS(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=z({queryKey:["job-events",e],queryFn:()=>lS(e),enabled:!!e,refetchInterval:e?2500:!1}),i=I({mutationFn:({content:o,done:u})=>uS(e,{content:o,done:u}),onSuccess:(o,{done:u})=>{n({type:"success",message:u?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return h.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function mS(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function fS(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=fS(a.children,t);if(n)return n}}return null}function Fc(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:Fc(n.children,t,a)}:n)}function pS(e){let[t,a]=h.default.useState([]),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState(""),c=!!(e?.project_dir&&e?.id),d=z({queryKey:["job-files-root",e?.id],queryFn:()=>Wp(e.id,""),enabled:c}),f=z({queryKey:["job-file",e?.id,n],queryFn:()=>cS(e.id,n),enabled:!!(c&&n)});h.default.useEffect(()=>{a([]),r(""),i(""),u("")},[e?.id]),h.default.useEffect(()=>{d.data?.entries?(a(mS(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let m=h.default.useCallback(async p=>{let y=fS(t,p);if(!(!y||!e?.id)){if(y.expanded){a(b=>Fc(b,p,w=>({...w,expanded:!1})));return}if(y.loaded){a(b=>Fc(b,p,w=>({...w,expanded:!0})));return}u(p);try{let b=await Wp(e.id,p);a(w=>Fc(w,p,g=>({...g,expanded:!0,loaded:!0,children:mS(b.entries)}))),i("")}catch(b){i(b.message||"Unable to open folder")}finally{u("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:f.data||null,fileError:f.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:f.isLoading||f.isFetching,expandingPath:o,treeError:s,toggleDirectory:m}}function hS(){let e=Y(),[t,a]=h.default.useState(null),n=z({queryKey:["jobs-summary"],queryFn:rS,refetchInterval:5e3}),r=z({queryKey:["jobs"],queryFn:nS,refetchInterval:5e3}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=I({mutationFn:({jobId:u})=>iS(u),onSuccess:(u,{jobId:c})=>{a({type:"success",message:`Job ${jr(c)} cancelled`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to cancel job"})}}),o=I({mutationFn:({jobId:u})=>oS(u),onSuccess:u=>{a({type:"success",message:`Restart queued as ${jr(u?.new_job_id)}`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function vS({result:e,onDismiss:t}){let a=k();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return l`
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
  `}function eh(){let e=k(),t=fe(),{jobId:a=null}=it(),[n,r]=h.default.useState(""),[s,i]=h.default.useState("all"),[o,u]=h.default.useState(a?"activity":"overview"),c=hS(),d=dS(a),f=pS(d.job);h.default.useEffect(()=>{u(a?"activity":"overview")},[a]);let m=h.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(x=>{let $=!v||x.title.toLowerCase().includes(v)||x.id.toLowerCase().includes(v),S=s==="all"||x.state===s;return $&&S})},[c.jobs,n,s]),p=h.default.useCallback(v=>t(`/jobs/${v}`),[t]),y=h.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),b=h.default.useCallback(async v=>{try{let x=await c.restartJob({jobId:v});x?.new_job_id&&t(`/jobs/${x.new_job_id}`)}catch{}},[c,t]),w=l`
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
      `;else{let v={overview:l`<${eS} job=${d.job} />`,activity:l`
          <${J2}
            job=${d.job}
            events=${d.events}
            onSendPrompt=${d.sendPrompt}
            isSendingPrompt=${d.isSendingPrompt}
          />
        `,files:l`
          <${W2}
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
        <${X2}
          job=${d.job}
          activeTab=${o}
          onTabChange=${u}
          onBack=${()=>t("/jobs")}
          onCancel=${y}
          onRestart=${b}
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
          <${tS}
            jobs=${m}
            totalJobs=${c.jobs.length}
            selectedJobId=${a}
            search=${n}
            onSearchChange=${r}
            stateFilter=${s}
            onStateFilterChange=${i}
            onSelectJob=${p}
            onCancelJob=${y}
            isBusy=${c.isBusy}
            isRefreshing=${c.isRefreshing}
          />
        `;return l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          ${a&&l`<div className="flex flex-wrap justify-end gap-2">
            ${w}
          </div>`}
          ${c.error&&l`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${c.error.message}
            </div>
          `}
          <${vS}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${vS}
            result=${d.promptResult}
            onDismiss=${d.clearPromptResult}
          />
          <${aS} summary=${c.summary} />
          ${g}
        </div>
      </div>
    </div>
  `}function tr(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not scheduled"}function zc(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function qc(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function gS(e=[]){return[...e].sort((t,a)=>t.enabled!==a.enabled?t.enabled?-1:1:new Date(a.next_fire_at||a.last_run_at||0).getTime()-new Date(t.next_fire_at||t.last_run_at||0).getTime())}function yS(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function fA(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function bS({runs:e}){return e?.length?l`
    <div className="space-y-3">
      ${e.map(t=>l`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${P} tone=${fA(t.status)} label=${t.status} />
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
  `}function xS({title:e,value:t}){return l`
    <div>
      <h3 className="text-sm font-semibold text-iron-100">${e}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >${JSON.stringify(t||{},null,2)}</pre>
    </div>
  `}function $S({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=fe(),u=k();return t?l`
      <div className="space-y-4">
        ${[1,2,3].map(c=>l`<div key=${c} className="v2-skeleton h-32 rounded-xl" />`)}
      </div>
    `:a||!e?l`
      <${he}
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
            <${P}
              tone=${zc(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${P}
              tone=${qc(e.verification_status)}
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
        <${ar} label="Action" value=${yS(e.action)} />
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
        <${xS} title=${u("routine.triggerPayload")} value=${e.trigger} />
        <${xS} title=${u("routine.actionPayload")} value=${e.action} />
      </div>

      <div className="mt-6">
        <h3 className="mb-3 text-sm font-semibold text-iron-100">Recent runs</h3>
        <${bS} runs=${e.recent_runs} />
      </div>
    <//>
  `}function wS({routine:e,selectedRoutineId:t,onSelectRoutine:a,onTriggerRoutine:n,onToggleRoutine:r,isBusy:s}){let i=t===e.id;return l`
    <article
      className=${["group flex flex-col gap-4 rounded-[18px] border p-5",i?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick=${()=>a(e.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">${e.name}</h3>
            <${P}
              tone=${zc(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${P}
              tone=${qc(e.verification_status)}
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
  `}var pA=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function th({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:u,onToggleRoutine:c,isBusy:d,isRefreshing:f}){let m=k();if(!e.length){let p=!!n.trim()||s!=="all";return l`
      <${he}
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
            ${pA.map(p=>l`<option key=${p.value} value=${p.value}>${p.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(p=>l`
            <${wS}
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
  `}var hA=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function SS({summary:e}){return l`
    <${j} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${hA.map(t=>l`
            <div
              key=${t.key}
              className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
            >
              <${tt}
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
  `}function NS(e){let[t,a]=h.default.useState(""),[n,r]=h.default.useState("all");return{filteredRoutines:h.default.useMemo(()=>{let i=t.trim().toLowerCase();return gS(e).filter(o=>{let u=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||u.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function _S(){return Promise.resolve({routines:[],todo:!0})}function kS(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function RS(e){return Promise.resolve(null)}function Bc(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function Hc(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function CS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function ES(e){let t=Y(),[a,n]=h.default.useState(null),r=z({queryKey:["routine-detail",e],queryFn:()=>RS(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:f=>{n({type:"error",message:f.message||"Unable to update routine"})}}),o=I(i(Bc,"Routine run queued.")),u=I(i(Hc,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,isBusy:o.isPending||u.isPending}}function TS(){let e=Y(),[t,a]=h.default.useState(null),n=z({queryKey:["routines-summary"],queryFn:kS,refetchInterval:5e3}),r=z({queryKey:["routines"],queryFn:_S,refetchInterval:5e3}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,f)=>({mutationFn:({routineId:m})=>d(m),onSuccess:()=>{a({type:"success",message:f}),s()},onError:m=>{a({type:"error",message:m.message||"Unable to update routine"})}}),o=I(i(Bc,"Routine run queued.")),u=I(i(Hc,"Routine status updated.")),c=I(i(CS,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||u.isPending||c.isPending,invalidate:s}}function ah(){let e=fe(),{routineId:t=null}=it(),a=TS(),n=ES(t),r=NS(a.routines),s=h.default.useCallback(async(u,c)=>{try{await u({routineId:c})}catch{}},[]),i=h.default.useCallback(async(u,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:u}),e("/routines")}catch{}},[e,a]),o=t?l`
        <div className="grid gap-5 xl:grid-cols-[minmax(0,0.9fr)_minmax(440px,1.1fr)]">
          <${th}
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
          <${$S}
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
        <${th}
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

          <${Ia}
            result=${a.actionResult}
            onDismiss=${a.clearActionResult}
          />
          <${Ia}
            result=${n.actionResult}
            onDismiss=${n.clearActionResult}
          />
          <${SS} summary=${a.summary} />

          ${a.isLoading?l`
                <div className="space-y-4">
                  ${[1,2,3].map(u=>l`<div key=${u} className="v2-skeleton h-32 rounded-xl" />`)}
                </div>
              `:o}
        </div>
      </div>
    </div>
  `}function vA(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function gA(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function AS({deliveryState:e}){let t=k(),a=e.currentTarget?.target_id||"",[n,r]=h.default.useState(a),[s,i]=h.default.useState(!1),o=h.default.useRef(null);h.default.useEffect(()=>{r(a)},[a]),h.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let u=n!==a,c=e.isLoading||e.isSaving,d=u&&!c,f=!!a&&!c,m=e.finalReplyTargets.length>0,p=e.targets.some(U=>U?.capabilities?.final_replies&&U?.target?.status==="unavailable"),y=m||p,b=U=>(o.current&&clearTimeout(o.current),i(!1),U.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),w=()=>{d&&b(e.saveFinalReplyTarget(n||null))},g=()=>{f&&(r(""),b(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),x=e.currentStatus,$=x==="available"?"success":x==="unavailable"?"warning":"muted",S=t(x==="available"?"automations.delivery.pill.ready":x==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),R=!!e.currentTarget,_=t(R?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),C=gA(t("automations.delivery.footnote"),{command:l`<code
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
              <${P} tone=${$} label=${S} />
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
            ${e.finalReplyTargets.map(U=>{let O=U?.target?.target_id??"",B=U?.target?.display_name||U?.target?.target_id||"",A=U?.target?.description||"",K=U?.target?.status??"available",te=n===O;return l`
                <label
                  key=${O}
                  className=${G("flex items-start gap-3.5 rounded-xl border px-4 py-3.5 cursor-pointer","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]","hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",te&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
                >
                  <input
                    type="radio"
                    name="delivery-target"
                    value=${O}
                    checked=${te}
                    disabled=${c}
                    onChange=${()=>r(O)}
                    className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-semibold text-[var(--v2-text-strong)] leading-snug">
                      ${B}
                    </div>
                    ${A&&l`<div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                      ${A}
                    </div>`}
                  </div>
                  <${P}
                    tone=${vA(K)}
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
                <${P}
                  tone="warning"
                  label=${t("automations.delivery.pill.notPaired")}
                  className="shrink-0"
                />
              </div>
            `}

            <!-- Web app only / fallback row -->
            <label
              className=${G("flex items-start gap-3.5 rounded-xl border px-4 py-3.5","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]",m?"cursor-pointer hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]":"cursor-default",n===""&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
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
              <${P}
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
            onClick=${w}
          >
            <${D} name="check" className="h-3.5 w-3.5" />
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
        ${y&&l`
          <div
            className="rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-faint)]"
          >
            ${C}
          </div>
        `}

      </div>
    <//>
  `}var MS={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},OS={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},LS={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function ii(e){return typeof e=="function"?e:t=>t}var rh=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:Zo},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:EA}];function US(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>r?.source?.type==="schedule").map(r=>_A(r,t,a)).sort(CA)}function jS(e,t){let a=rh.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function PS(e){let t=e.filter(s=>Zo(s)).length,a=e.filter(s=>s.has_running_run).length,n=e.filter(s=>s.has_failed_runs).length,r=e.filter(s=>Zo(s)&&nh(s)!=null).sort((s,i)=>(s.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(i.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:e.length,active:t,running:a,failures:n,nextRun:r?.next_run_label||null}}function yA(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=MA(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:u,month:c,dayOfWeek:d,year:f}=s,m=t&&typeof t=="string"?t:null,p=m?` (${m})`:"",y=f==="*"&&u==="*"&&c==="*"&&d==="*";if(y&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=OA(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(nr(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let b=TA(o,i,n);if(!b)return r("automations.schedule.custom");if(y)return r("automations.schedule.everyDayAt",{time:b})+p;let w=LA(d);if(f==="*"&&u==="*"&&c==="*"&&w==="1-5")return r("automations.schedule.weekdaysAt",{time:b})+p;if(f==="*"&&u==="*"&&c==="*"&&nr(w,0,7)){let g=AA(Number(w)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:b})+p}if(f==="*"&&nr(u,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(u),time:b})+p;if(nr(u,1,31)&&nr(c,1,12)&&d==="*"&&(f==="*"||nr(f,1970,9999))){let g=DA(Number(c),Number(u),f==="*"?null:Number(f),n);return r("automations.schedule.dateAt",{date:g,time:b})+p}return r("automations.schedule.custom")}function si(e,t="Unknown",a){if(!e)return t;let n=new Date(e);if(Number.isNaN(n.getTime()))return t;try{return n.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}catch{return n.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function bA(e,t){let a=MS[e]?.labelKey||"automations.state.unknown";return ii(t)(a)}function xA(e){return MS[e]?.tone||"muted"}function $A(e,t){let a=OS[e]?.labelKey||"automations.lastStatus.none";return ii(t)(a)}function wA(e){return OS[e]?.tone||"muted"}function SA(e,t){let a=LS[sh(e)]?.labelKey||"automations.runStatus.unknown";return ii(t)(a)}function NA(e){return LS[sh(e)]?.tone||"muted"}function _A(e,t,a){let n=ii(t),r=kA(e.recent_runs,t,a),s=r[0]||null,i=r.find(d=>d.status==="running")||null,o=r.find(d=>d.status==="ok"||d.status==="error")||null,u=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null;return{...e,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:yA(e.source?.cron,e.source?.timezone||"UTC",t,a),state_label:bA(e.state,t),state_tone:xA(e.state),next_run_timestamp:ih(e.next_run_at),next_run_label:si(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:si(c,n("automations.date.noRuns"),a),last_status_label:$A(u,t),last_status_tone:wA(u),created_label:si(e.created_at,n("automations.date.unknown"),a),recent_runs:r,latest_run:s,current_run:i,has_running_run:r.some(d=>d.status==="running"),has_failed_runs:r.some(d=>d.status==="error"),success_rate_label:RA(r,t)}}function kA(e,t,a){let n=ii(t);return Array.isArray(e)?e.map(r=>{let s=sh(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=ih(i);return{...r,status:s,status_label:SA(s,t),status_tone:NA(s),timestamp:o,timestamp_source:i,fired_label:si(i,n("automations.date.unscheduled"),a),submitted_label:si(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:si(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function sh(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function RA(e,t){let a=ii(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function CA(e,t){let a=Zo(e),n=Zo(t);return a!==n?a?-1:1:(nh(e)??Number.MAX_SAFE_INTEGER)-(nh(t)??Number.MAX_SAFE_INTEGER)}function ih(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function Zo(e){return e?.state==="active"||e?.state==="scheduled"}function EA(e){return["paused","disabled","inactive"].includes(e?.state)}function nh(e){return e?.next_run_timestamp??ih(e?.next_run_at)}function oh(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function TA(e,t,a){return!nr(e,0,23)||!nr(t,0,59)?null:oh(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function AA(e,t){return oh(t,{weekday:"long"},new Date(2001,0,7+e))}function DA(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return oh(n,r,new Date(a??2e3,e-1,t))}function MA(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&DS(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&DS(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function DS(e){return/^0+$/.test(e)}function nr(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function OA(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function LA(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}function lh(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function Kc({runs:e}){let t=k(),a=e.slice(0,8);return a.length?l`
    <div className="flex items-center gap-1.5" aria-label=${t("automations.table.recentRuns")}>
      ${a.map(n=>l`
        <span
          key=${lh(n)}
          title=${`${n.status_label} \xB7 ${n.fired_label}`}
          className=${G("h-3 w-3 rounded-full border",n.status==="ok"&&"border-emerald-300/50 bg-emerald-400",n.status==="error"&&"border-red-300/50 bg-red-400",n.status==="running"&&"border-sky-300/60 bg-sky-400",n.status==="unknown"&&"border-iron-500 bg-iron-600")}
        />
      `)}
    </div>
  `:l`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`}function FS({run:e,onOpenRun:t,onOpenLogs:a}){let n=k(),r=!!e.chat_path,s=kc({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return l`
    <div className="grid gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto] sm:items-center">
      <div>
        <${P} tone=${e.status_tone} label=${e.status_label} />
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
  `}function Ic({label:e,value:t,tone:a}){return l`
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
  `}function zS({automation:e}){let t=k(),a=fe();if(!e)return l`
      <${j} className="p-4 sm:p-5">
        <${he}
          boxed=${!1}
          title=${t("automations.detail.emptyTitle")}
          description=${t("automations.detail.emptyDescription")}
        />
      <//>
    `;let n=e.current_run;return l`
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
          <${P}
            tone=${e.has_running_run?"info":e.state_tone}
            label=${e.has_running_run?t("automations.status.running"):e.state_label}
          />
        </div>
      </div>

      <div className="space-y-5 p-4 sm:p-5">
        <div className="grid gap-3 sm:grid-cols-2">
          <${Ic} label=${t("automations.detail.schedule")} value=${e.schedule_label} />
          <${Ic}
            label=${t("automations.detail.successRate")}
            value=${e.success_rate_label}
            tone=${e.has_failed_runs?"danger":"success"}
          />
          <${Ic} label=${t("automations.detail.lastCompleted")} value=${e.last_run_label} />
          <${Ic}
            label=${t("automations.detail.currentRun")}
            value=${n?.run_id||n?.thread_id||t("automations.detail.noCurrentRun")}
            tone=${e.has_running_run?"info":null}
          />
        </div>

        <div>
          <div className="mb-2 flex items-center justify-between gap-3">
            <h4 className="text-sm font-semibold text-iron-100">
              ${t("automations.detail.recentRuns")}
            </h4>
            <${Kc} runs=${e.recent_runs} />
          </div>

          ${e.recent_runs.length?l`
                <div>
                  ${e.recent_runs.map(r=>l`
                    <${FS}
                      key=${lh(r)}
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
  `}function qS({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,selectedAutomationId:s,onSelectAutomation:i}){let o=k(),u=jS(e,t),c=e.length>0,d=u.find(f=>f.automation_id===s)||u[0]||null;return l`
    <div className="space-y-5">
      <${j} className="p-4 sm:p-5">
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
              ${rh.map(f=>l`
                <button
                  key=${f.value}
                  type="button"
                  aria-pressed=${t===f.value}
                  onClick=${()=>a(f.value)}
                  className=${G("h-9 px-3 text-xs font-semibold",t===f.value?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
                >
                  ${o(f.labelKey)}
                </button>
              `)}
            </div>
            <${T}
              variant="secondary"
              size="icon-sm"
              aria-label=${o("automations.refresh")}
              disabled=${r}
              onClick=${n}
            >
              <${D} name="retry" className="h-4 w-4" />
            <//>
          </div>
        </div>
      <//>

      ${u.length?l`
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1.12fr)_minmax(22rem,0.88fr)]">
              <${j} className="overflow-hidden">
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
                            className=${G("border-b border-[var(--v2-panel-border)] last:border-0 hover:bg-white/[0.03]",m&&"bg-[var(--v2-accent-soft)]/30")}
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
                              <${Kc} runs=${f.recent_runs} />
                            </td>
                            <td className="px-5 py-4 align-top">
                              <${P}
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

              <${zS} automation=${d} />
            </div>
          `:l`
            <${he}
              title=${o(c?"automations.empty.matchingTitle":"automations.empty.noneTitle")}
              description=${o(c?"automations.empty.matchingDescription":"automations.empty.noneDescription")}
            />
          `}
    </div>
  `}function BS({summary:e}){let t=k(),a=[{key:"scheduled",label:t("automations.summary.scheduled"),value:e?.scheduled??0,tone:"muted",detail:t("automations.summary.scheduledDetail")},{key:"active",label:t("automations.summary.active"),value:e?.active??0,tone:"signal",detail:t("automations.summary.activeDetail")},{key:"running",label:t("automations.summary.running"),value:e?.running??0,tone:"info",detail:t("automations.summary.runningDetail")},{key:"failures",label:t("automations.summary.failures"),value:e?.failures??0,tone:(e?.failures??0)>0?"danger":"success",detail:t("automations.summary.failuresDetail")},{key:"nextRun",label:t("automations.summary.nextRun"),value:e?.nextRun||t("automations.summary.none"),tone:"info",detail:t("automations.summary.nextRunDetail"),valueClassName:"text-lg md:text-xl"}];return l`
    <${j} className="p-4 sm:p-5">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        ${a.map(n=>l`
          <div
            key=${n.key}
            className="rounded-[14px] border border-white/8 bg-white/[0.03] p-4"
          >
            <${tt}
              label=${n.label}
              value=${n.value}
              tone=${n.tone}
              badgeLabel=${t(`automations.badge.${n.tone}`)}
              detail=${n.detail}
              valueClassName=${n.valueClassName}
              showDivider=${!1}
              className="px-0 py-0"
            />
          </div>
        `)}
      </div>
    <//>
  `}var UA=50,jA=25;function HS(){let{t:e,lang:t}=tl(),a=z({queryKey:["automations"],queryFn:()=>ix({limit:UA,runLimit:jA}),refetchInterval:3e4,refetchIntervalInBackground:!1}),n=h.default.useMemo(()=>US(a.data,e,t),[a.data,e,t]),r=h.default.useMemo(()=>PS(n),[n]),s=a.data?.scheduler_enabled!==!1;return{automations:n,summary:r,schedulerEnabled:s,isLoading:a.isLoading,isRefreshing:a.isFetching,error:a.error||null,refetch:a.refetch}}var KS=["outbound-delivery","preferences"],IS=["outbound-delivery","targets"];function QS(){let e=Y(),t=z({queryKey:KS,queryFn:ox}),a=z({queryKey:IS,queryFn:lx}),n=I({mutationFn:({finalReplyTargetId:i})=>ux({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(KS,i),e.invalidateQueries({queryKey:IS})}}),r=h.default.useMemo(()=>a.data?.targets??[],[a.data]),s=h.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function VS(){let e=k(),[t,a]=h.default.useState("all"),[n,r]=h.default.useState(null),s=HS(),i=QS(),o=s.error&&!s.isLoading&&s.automations.length===0;return h.default.useEffect(()=>{if(!s.automations.length){r(null);return}s.automations.some(c=>c.automation_id===n)||r(s.automations[0].automation_id)},[s.automations,n]),l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          ${s.error&&l`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${e("automations.error.loadFailed")}
            </div>
          `}

          ${o?null:l`
                ${!s.isLoading&&!s.schedulerEnabled&&l`
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
                <${BS} summary=${s.summary} />
                <${AS} deliveryState=${i} />

                ${s.isLoading?l`
                      <div className="space-y-4">
                        ${[1,2,3].map(u=>l`<div
                              key=${u}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`)}
                      </div>
                    `:l`
                      <${qS}
                        automations=${s.automations}
                        filter=${t}
                        onFilterChange=${a}
                        onRefresh=${s.refetch}
                        isRefreshing=${s.isRefreshing}
                        selectedAutomationId=${n}
                        onSelectAutomation=${r}
                      />
                    `}
              `}
        </div>
      </div>
    </div>
  `}var GS={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function YS({result:e,onDismiss:t}){return h.default.useEffect(()=>{if(!e)return;let a=setTimeout(t,4e3);return()=>clearTimeout(a)},[e,t]),e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",GS[e.type]||GS.info].join(" ")}>
      <${D}
        name=${e.type==="success"?"check":e.type==="error"?"close":"bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">
        <${D} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `:null}var JS="/api/webchat/v2/channels/slack/allowed",PA="/api/webchat/v2/channels/slack/subjects";function XS(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function ZS(){return Z(JS)}function WS(){return Z(PA)}function eN(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return Z(JS,{method:"PUT",body:JSON.stringify(n)})}function tN(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var aN=["slack-allowed-channels"];function rN({action:e}){let t=k(),a=Y(),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState([]),c=zA(e,t),d=z({queryKey:aN,queryFn:ZS}),f=z({queryKey:["slack-routable-subjects"],queryFn:WS}),m=f.data?.subjects||[],p=nN(m),y=f.isSuccess||f.isError,b=m.length>0;h.default.useEffect(()=>{d.data&&u(uh(d.data.channels||[]))},[d.data]);let w=I({mutationFn:({channels:R})=>eN(R),onSuccess:R=>{u(uh(R.channels||[])),a.invalidateQueries({queryKey:aN}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let R=n.trim();!R||!f.isSuccess||(u(_=>uh([..._,{channel_id:R,subject_user_id:s}])),r(""))},v=R=>{u(_=>_.filter(C=>C.channel_id!==R))},x=(R,_)=>{u(C=>C.map(U=>U.channel_id===R?{...U,subject_user_id:_}:U))},$=()=>{w.mutate({channels:FA(o)})},S=f.isError&&o.some(R=>!R.subject_user_id);return l`
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
          disabled=${!b}
          className="h-9 min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
        >
          ${!b&&l`<option value="">${c.noSubjectsLabel}</option>`}
          ${b&&l`<option value="">${c.autoSubjectLabel}</option>`}
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
                ${b?l`
                    <select
                      value=${R.subject_user_id}
                      onChange=${_=>x(R.channel_id,_.target.value)}
                      className="h-8 rounded-md border border-white/10 bg-white/[0.04] px-2 text-xs text-iron-100 outline-none focus:border-signal/45"
                    >
                      <option value="">${c.autoSubjectLabel}</option>
                      ${nN(m,R).map(_=>l`
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
          onClick=${$}
          disabled=${!d.isSuccess||!y||w.isPending||S}
        >
          ${w.isPending?c.savingLabel:c.submitLabel}
        <//>
        ${w.isSuccess&&l`<p className="text-xs text-emerald-300">
          ${c.successMessage}
        </p>`}
        ${(d.isError||f.isError||w.isError)&&l`<p className="text-xs text-red-300">
          ${tN(w.error||d.error||f.error,c.errorMessage)}
        </p>`}
      </div>
    </div>
  `}function nN(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function uh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return XS(Array.from(t.keys())).map(a=>t.get(a))}function FA(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function zA(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var ch={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function Pr(e){return e==="wasm_channel"||e==="channel"}var sN={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},iN={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function oN(e){let t=lN(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||Pr(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function lN(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function dh(e){let t=lN(e);return t==="active"||t==="ready"}function uN({extension:e,secrets:t=[],fields:a=[]}={}){return dh(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var cN="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",dN="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",mN="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",fN="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",pN="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",qA="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function hN(e){return e.package_ref?.id||""}function BA({actions:e,isBusy:t}){let a=k(),[n,r]=h.default.useState(!1),s=h.default.useRef(null);return h.default.useEffect(()=>{if(!n)return;let i=o=>{s.current&&!s.current.contains(o.target)&&r(!1)};return document.addEventListener("mousedown",i),()=>document.removeEventListener("mousedown",i)},[n]),l`
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
  `}function vN({items:e}){return!e||e.length===0?null:l`
    <div className="mt-3 flex flex-wrap gap-1">
      ${e.map(t=>l`<span key=${t} className=${qA}>${t}</span>`)}
    </div>
  `}function oi({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=k(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=sN[i]||"muted",u=s(`extensions.state.${i}`)||iN[i]||i,c=s(`extensions.kind.${e.kind}`)||ch[e.kind]||e.kind,d=e.display_name||hN(e),f=!!e.package_ref,m=e.tools||[],[p,y]=h.default.useState(!1),w=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],x=[],$=oN(e);$==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):$==="activate"&&v.push({id:"activate",label:"Activate",run:()=>t(g)}),f&&(e.needs_setup||e.has_auth)&&$!=="configure"&&x.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)}),f&&Pr(e.kind)&&(i==="setup_required"||i==="failed")&&x.push({id:"setup",label:"Setup",icon:"settings",run:()=>a(g)}),f&&Pr(e.kind)&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&x.push({id:"reconfigure",label:"Reconfigure",icon:"settings",run:()=>a(g)}),f&&x.push({id:"remove",label:s("common.remove")||"Remove",icon:"trash",danger:!0,run:()=>n(g)});let S=v[0];return l`
    <div className=${cN}>
      <div className="flex items-start gap-2">
        <${P} tone=${o} label=${u} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${x.length>0&&l`<${BA} actions=${x} isBusy=${r} />`}
      </div>

      <div className=${dN}>
        <span>${c}</span>
        ${e.version&&l`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&l`<p className=${mN}>${e.description}</p>`}

      ${e.activation_error&&l`
        <div
          className="mt-2 rounded-[10px] border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-1.5 text-xs text-[var(--v2-danger-text)]"
        >
          ${e.activation_error}
        </div>
      `}

      ${w&&l`
        <div className="mt-2 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2 text-xs leading-5 text-[var(--v2-text-muted)]">
          ${w}
        </div>
      `}

      <div className=${fN}>
        ${m.length>0?l`
              <button
                type="button"
                aria-expanded=${p?"true":"false"}
                onClick=${()=>y(R=>!R)}
                className=${pN}
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
          <${T} variant="secondary" size="sm" onClick=${S.run} disabled=${r}>
            ${S.label}
          <//>
        `}
      </div>

      ${p&&l`<${vN} items=${m} />`}
    </div>
  `}function Fr({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=k(),s=r(`extensions.kind.${e.kind}`)||ch[e.kind]||e.kind,i=e.display_name||hN(e),o=!!(e.package_ref&&t),u=e.keywords||[],[c,d]=h.default.useState(!1);return l`
    <div className=${cN}>
      <div className="flex items-start gap-2">
        <${P}
          tone="muted"
          label=${n||r("extensions.state.available")||"available"}
          size="sm"
        />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${i}
        </span>
      </div>

      <div className=${dN}>
        <span>${s}</span>
        ${e.version&&l`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&l`<p className=${mN}>${e.description}</p>`}

      <div className=${fN}>
        ${u.length>0?l`
              <button
                type="button"
                aria-expanded=${c?"true":"false"}
                onClick=${()=>d(f=>!f)}
                className=${pN}
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
          <${T}
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

      ${c&&l`<${vN} items=${u} />`}
    </div>
  `}function gN(){return Z("/api/webchat/v2/extensions")}function yN(){return Z("/api/webchat/v2/extensions/registry")}function bN(e){return Z("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function xN(e){return Z(`/api/webchat/v2/extensions/${encodeURIComponent(Wo(e))}/activate`,{method:"POST"})}function $N(e){return Z(`/api/webchat/v2/extensions/${encodeURIComponent(Wo(e))}/remove`,{method:"POST"})}function wN(e){return Z(`/api/webchat/v2/extensions/${encodeURIComponent(Wo(e))}/setup`)}function SN(e,t,a){return yx(Wo(e),{action:"submit",payload:{secrets:t,fields:a}})}function NN(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return Z(`/api/webchat/v2/extensions/${encodeURIComponent(Wo(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function _N(){return Promise.resolve({requests:[]})}function kN(){return Promise.resolve({success:!1,message:"Pairing requires a v2 pairing endpoint."})}function Wo(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var HA=2e3,KA=10*60*1e3;function li(e){return e?.package_ref?.id||null}function mh(e){return e?.display_name||li(e)||""}function RN(e,t,a){return li(t)||`${e}:${mh(t)||"unknown"}:${a}`}function IA(e,t){return e.installed!==t.installed?e.installed?-1:1:mh(e.entry||e.extension).localeCompare(mh(t.entry||t.extension))}function CN(){let e=Y(),t=z({queryKey:["gateway-status-extensions"],queryFn:Bs,staleTime:1e4}),a=z({queryKey:["extensions"],queryFn:gN}),n=z({queryKey:["extension-registry"],queryFn:yN}),r=z({queryKey:["connectable-channels"],queryFn:Nc}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["extensions"]}),e.invalidateQueries({queryKey:["extension-registry"]}),e.invalidateQueries({queryKey:["gateway-status-extensions"]}),e.invalidateQueries({queryKey:["connectable-channels"]})},[e]),[i,o]=h.default.useState(null),u=h.default.useCallback(()=>o(null),[]),c=I({mutationFn:({packageRef:A})=>bN(A),onSuccess:(A,{displayName:K})=>{A.success?(o({type:"success",message:A.message||A.instructions||`${K||"Extension"} installed`}),A.auth_url&&window.open(A.auth_url,"_blank","noopener,noreferrer")):o({type:"error",message:A.message||"Install failed"}),s()},onError:A=>{o({type:"error",message:A.message}),s()}}),d=I({mutationFn:({packageRef:A})=>xN(A),onSuccess:(A,{displayName:K})=>{A.success?(o({type:"success",message:A.message||A.instructions||`${K||"Extension"} activated`}),A.auth_url&&window.open(A.auth_url,"_blank","noopener,noreferrer")):A.auth_url?(window.open(A.auth_url,"_blank","noopener,noreferrer"),o({type:"info",message:"Opening authentication\u2026"})):A.awaiting_token?o({type:"info",message:"Configuration required"}):o({type:"error",message:A.message||"Activation failed"}),s()},onError:A=>{o({type:"error",message:A.message})}}),f=I({mutationFn:({packageRef:A})=>$N(A),onSuccess:(A,{displayName:K})=>{A.success?o({type:"success",message:`${K||"Extension"} removed`}):o({type:"error",message:A.message||"Remove failed"}),s()},onError:A=>{o({type:"error",message:A.message})}}),m=t.data||{},p=a.data?.extensions||[],y=n.data?.entries||[],b=r.data?.channels||[],w=new Map(p.map(A=>[li(A),A]).filter(([A])=>!!A)),g=new Set(y.map(A=>li(A)).filter(Boolean)),v=[...y.map((A,K)=>{let te=li(A),ye=te&&w.get(te)||null;return{id:RN("registry",A,K),installed:!!(ye||A.installed),entry:A,extension:ye}}),...p.filter(A=>{let K=li(A);return!K||!g.has(K)}).map((A,K)=>({id:RN("installed",A,K),installed:!0,entry:null,extension:A}))].sort(IA),x=A=>Pr(A.kind),$=p.filter(x),S=p.filter(A=>A.kind==="mcp_server"),R=p.filter(A=>!x(A)&&A.kind!=="mcp_server"),_=y.filter(A=>x(A)&&!A.installed),C=y.filter(A=>A.kind==="mcp_server"&&!A.installed),U=y.filter(A=>A.kind!=="mcp_server"&&!x(A)&&!A.installed),O=a.isLoading||n.isLoading,B=c.isPending||d.isPending||f.isPending;return{status:m,extensions:p,channels:$,mcpServers:S,tools:R,channelRegistry:_,mcpRegistry:C,toolRegistry:U,registry:y,catalogEntries:v,connectableChannels:b,isLoading:O,isBusy:B,actionResult:i,clearResult:u,install:c.mutate,activate:d.mutate,remove:f.mutate,invalidate:s}}function EN(e){let t=z({queryKey:["extension-setup",e?.id||e],queryFn:()=>wN(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function TN(e,t){let a=Y(),n=e?.id||e;return I({mutationFn:({secrets:r,fields:s})=>SN(e,r,s),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function AN(e){let t=Y(),a=e?.id||e,n=h.default.useRef(null),r=h.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=h.default.useCallback(()=>{let u=t.getQueryData(["extension-setup",a]);if(u?.secrets?.length>0&&u.secrets.every(m=>m.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(m=>m.package_ref?.id===a),f=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return f==="active"||f==="ready"},[a,t]),o=h.default.useCallback(u=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||u&&u.closed||Date.now()-c>KA)&&(r(),s())},HA)},[r,s,i]);return h.default.useEffect(()=>r,[r]),I({mutationFn:({secret:u,popup:c})=>NN(e,u).then(d=>({res:d,popup:c})),onSuccess:({res:u,popup:c})=>{let d=c;u.authorization_url&&c&&!c.closed?c.location.href=u.authorization_url:u.authorization_url?d=window.open(u.authorization_url,"_blank","noopener,noreferrer"):c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(u,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function DN(e,t={}){let a=z({queryKey:["pairing",e],queryFn:()=>_N(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=Y(),r=I({mutationFn:({code:s})=>kN(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function MN(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var QA={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function ON({channel:e,redeemFn:t,i18nKeys:a=QA,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=k(),o=typeof t=="function",u=DN(e,{enabled:!o}),c=Y(),[d,f]=h.default.useState(""),m=VA(i,a,r),p=I({mutationFn:({code:S})=>t(e,S),onSuccess:()=>{f("");for(let S of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:S})}}),y=h.default.useCallback(S=>u.approve({code:S}),[u.approve]),b=h.default.useCallback(()=>{let S=d.trim();S&&(o?p.mutate({code:S}):(u.approve({code:S}),f("")))},[o,d,u.approve,p]),w=o?[]:u.requests,g=o?!1:u.isLoading,v=o?p.isPending:u.isApproving,x=o?p.isSuccess?p.data:null:u.result,$=o?p.isError?p.error:null:u.error;return g?l`
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
          onKeyDown=${S=>S.key==="Enter"&&b()}
          placeholder=${m.placeholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${T}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${b}
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
      ${$&&l`<p className="mb-3 text-xs text-red-300">
        ${MN($,m.error)}
      </p>`}

      ${s&&w.length>0?l`
            <div className="space-y-2">
              ${w.map(S=>l`
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
                    onClick=${()=>y(S.code||S.id)}
                    disabled=${v}
                  >
                    ${m.action}
                  <//>
                </div>
              `)}
            </div>
          `:s&&l`<p className="text-xs text-iron-300">${i(a.empty)}</p>`}
    </div>
  `}function VA(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function Qc(e){return e.package_ref?.id||""}function LN(e){return Qc(e)==="slack"}function jN(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function PN(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function GA(e){let t=e||[],a=[t.find(jN),t.find(PN)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function UN({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>jN(r)?l`<${rN} action=${r.action} />`:PN(r)?l`<${yc} action=${r.action} />`:null).filter(Boolean);return n.length>0?l`<div className="space-y-3">${n}</div>`:null}function FN({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:u}){let c=k(),d=t||[],f=e.enabled_channels||[],m=GA(a),p=d.some(LN),y=m.length>0&&!p;return l`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          ${c("channels.builtIn")}
        </h3>
        <${ui}
          name="Web Gateway"
          description=${c("channels.webGatewayDesc")||"Browser-based chat with SSE streaming"}
          enabled=${!0}
          detail=${"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)}
        />
        <${ui}
          name="HTTP Webhook"
          description=${c("channels.httpWebhookDesc")||"Inbound webhook endpoint for external integrations"}
          enabled=${f.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${ui}
          name="CLI"
          description=${c("channels.cliDesc")||"Terminal interface with TUI or simple REPL"}
          enabled=${f.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${ui}
          name="REPL"
          description=${c("channels.replDesc")||"Minimal read-eval-print loop for testing"}
          enabled=${f.includes("repl")}
          detail="ironclaw run --repl"
        />
        ${y&&l`
          <${ui}
            name=${c("channels.slack")||"Slack"}
            description=${c("channels.slackDesc")||"Tenant app channel for DMs and app mentions"}
            enabled=${!1}
            statusLabel="legacy"
            statusTone="muted"
            detail=${c("channels.slackDetail")||"Tenant Slack app install"}
          >
            <${UN}
              slackConnectActions=${m}
            />
          </${ui}>
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
            ${d.map(b=>l`
                <div key=${Qc(b)} className="flex flex-col gap-3">
                  <${oi}
                    ext=${b}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${u}
                  />
                  ${LN(b)&&l`<${UN}
                    slackConnectActions=${m}
                  />`}
                  ${(b.onboarding_state==="pairing_required"||b.onboarding_state==="pairing")&&l` <${ON} channel=${Qc(b)} /> `}
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
            ${n.map(b=>l`
                <${Fr}
                  key=${Qc(b)}
                  entry=${b}
                  onInstall=${o}
                  isBusy=${u}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function ui({name:e,description:t,enabled:a,detail:n,children:r,statusLabel:s=a?"on":"off",statusTone:i=a?"success":"muted"}){return l`
    <div
      className="border-t border-white/[0.06] py-4 first:border-0 first:pt-0"
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-iron-200">${e}</span>
            <${P}
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
  `}function zN({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=k(),s=e?.displayName||e?.packageRef?.id||"Extension",{secrets:i=[],fields:o=[],onboarding:u,isLoading:c,error:d}=EN(e?.packageRef),[f,m]=h.default.useState({}),[p,y]=h.default.useState({}),b=AN(e?.packageRef),w=TN(e?.packageRef,_=>{_.success!==!1&&(n&&n(_),a())}),g=h.default.useCallback(()=>{let _={};for(let[C,U]of Object.entries(f)){let O=(U||"").trim();O&&(_[C]=O)}w.mutate({secrets:_,fields:p})},[f,p,w]),v=h.default.useCallback(_=>{let C=window.open("about:blank","_blank","width=600,height=600");C&&(C.opener=null),b.mutate({secret:_,popup:C})},[b]),$=i.filter(_=>(_.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,S=dh(e),R=uN({extension:e,secrets:i,fields:o});return c?l`
      <${Vc} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(_=>l`<div
                key=${_}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?l`
      <${Vc} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")||"Failed to load setup:"} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?l`
      <${Vc} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")||"No configuration required for this extension."}
        </p>
      <//>
    `:l`
    <${Vc} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
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
                        disabled=${b.isPending}
                      >
                        ${b.isPending?r("extensions.opening"):_.provided?r("extensions.reconnect"):r("extensions.authorize")}
                      <//>
                    </div>
                  `:l`
              <input
                type="password"
                placeholder=${_.provided?"\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (leave blank to keep)":""}
                value=${f[_.name]||""}
                onChange=${C=>m(U=>({...U,[_.name]:C.target.value}))}
                onKeyDown=${C=>C.key==="Enter"&&g()}
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
                onChange=${C=>y(U=>({...U,[_.name]:C.target.value}))}
                onKeyDown=${C=>C.key==="Enter"&&g()}
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
      ${w.error&&l`
        <div
          className="mt-4 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
        >
          ${w.error.message}
        </div>
      `}
      ${b.error&&l`
        <div
          className="mt-4 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
        >
          ${b.error.message}
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
        ${$&&l`
        <${T}
          variant=${R?"secondary":"primary"}
          onClick=${g}
          disabled=${w.isPending}
        >
          ${w.isPending?"Saving\u2026":r("common.save")||"Save"}
        <//>
        `}
      </div>
    <//>
  `}function Vc({onClose:e,title:t,children:a}){return h.default.useEffect(()=>{let n=r=>{r.key==="Escape"&&e()};return window.addEventListener("keydown",n),()=>window.removeEventListener("keydown",n)},[e]),l`
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
  `}function qN(e){return e.package_ref?.id||""}function BN({mcpServers:e,mcpRegistry:t,onActivate:a,onConfigure:n,onRemove:r,onInstall:s,isBusy:i}){let o=k();return e.length===0&&t.length===0?l`
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
                <${oi}
                  key=${qN(u)}
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
                <${Fr}
                  key=${qN(u)}
                  entry=${u}
                  onInstall=${s}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function YA(e){return e?.package_ref?.id||""}function JA(e){return e.entry||e.extension||{}}function HN({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=k(),[o,u]=h.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(b=>{let w=JA(b);return(w.display_name||YA(w)).toLowerCase().includes(c)||(w.description||"").toLowerCase().includes(c)||(w.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,f=d.filter(b=>b.installed&&b.extension),m=d.filter(b=>b.installed&&!b.extension&&b.entry),p=f.length+m.length,y=d.filter(b=>!b.installed&&b.entry);return e.length===0?l`
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
          onChange=${b=>u(b.target.value)}
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
                  ${f.map(b=>l`
                      <${oi}
                        key=${b.id}
                        ext=${b.extension||b.entry}
                        onActivate=${a}
                        onConfigure=${n}
                        onRemove=${r}
                        isBusy=${s}
                      />
                    `)}
                  ${m.map(b=>l`
                      <${Fr}
                        key=${b.id}
                        entry=${b.entry}
                        statusLabel=${i("extensions.installed")}
                        isBusy=${s}
                      />
                    `)}
                </div>
              `}

              ${y.length>0&&l`
                <h3
                  className=${["mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal",p>0?"mt-6":""].join(" ")}
                >
                  ${i("ext.registry.availableTitle")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${y.map(b=>l`
                      <${Fr}
                        key=${b.id}
                        entry=${b.entry}
                        onInstall=${t}
                        isBusy=${s}
                      />
                    `)}
                </div>
              `}
            `}
      </div>
    </div>
  `}function fh(){let{tab:e="registry"}=it(),[t,a]=h.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:u,connectableChannels:c,isLoading:d,isBusy:f,actionResult:m,clearResult:p,install:y,activate:b,remove:w,invalidate:g}=CN(),v=h.default.useCallback(_=>a(_),[]),x=h.default.useCallback(()=>a(null),[]),$=h.default.useCallback(()=>g(),[g]),S=h.default.useCallback(_=>{_&&(b(_),a(null))},[b]);if(d)return l`
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
    `;if(e==="installed")return l`<${ot} to="/extensions/registry" replace />`;let R={channels:l`<${FN}
      status=${n}
      channels=${r}
      connectableChannels=${c}
      channelRegistry=${i}
      onActivate=${b}
      onConfigure=${v}
      onRemove=${w}
      onInstall=${y}
      isBusy=${f}
    />`,mcp:l`<${BN}
      mcpServers=${s}
      mcpRegistry=${o}
      onActivate=${b}
      onConfigure=${v}
      onRemove=${w}
      onInstall=${y}
      isBusy=${f}
    />`,registry:l`<${HN}
      catalogEntries=${u}
      onInstall=${y}
      onActivate=${b}
      onConfigure=${v}
      onRemove=${w}
      isBusy=${f}
    />`};return R[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <${YS} result=${m} onDismiss=${p} />
          ${R[e]}
        </div>
      </div>

      ${t&&l`
        <${zN}
          extension=${t}
          onActivate=${S}
          onClose=${x}
          onSaved=${$}
        />
      `}
    </div>
  `:l`<${ot} to="/extensions/registry" replace />`}var KN=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],IN=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.auto_approve_tools",labelKey:"settings.field.autoApproveTools",descKey:"settings.field.autoApproveToolsDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],QN=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],ph=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","agent.auto_approve_tools","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function VN(e){return String(e||"").trim().toLowerCase()}function GN(e){if(e==null)return"";if(Array.isArray(e))return e.map(GN).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function at(e,t){let a=VN(e);return a?t.map(GN).join(" ").toLowerCase().includes(a):!0}function ci(e,t,a,n){let r=VN(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(u=>at(r,[i,u.key,u.labelKey?n(u.labelKey):u.label,u.descKey?n(u.descKey):u.description,t[u.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function XA({visible:e}){let t=k();return e?l`
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      ${t("tools.saved")}
    </span>
  `:null}function ZA({checked:e,onChange:t,label:a}){return l`
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
  `}function WA({field:e,value:t,onSave:a,isSaved:n}){let r=k(),[s,i]=h.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",u=e.descKey?r(e.descKey):e.description||"";h.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=h.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let f=parseInt(d,10);isNaN(f)||a(e.key,f)}else if(e.type==="float"){let f=parseFloat(d);isNaN(f)||a(e.key,f)}else a(e.key,d)},[e.key,e.type,a]);return l`
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">${o}</div>
        ${u&&l`<div className="mt-1 text-xs leading-5 text-iron-300">${u}</div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${e.type==="boolean"?l`
              <${ZA}
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
        <${XA} visible=${n} />
      </div>
    </div>
  `}function di({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=k(),o=t?i(t):e||"";return l`
    <${W} className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${o}</h3>
      <div>
        ${a.map(u=>l`
              <${WA}
                key=${u.key}
                field=${u}
                value=${n[u.key]}
                onSave=${r}
                isSaved=${s[u.key]}
              />
            `)}
      </div>
    <//>
  `}function _t({query:e}){let t=k();return l`
    <${W} padding="lg">
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
  `}function YN({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=k();if(n)return l`<${e5} />`;let i=ci(IN,e,r,s);return i.length===0?l`<${_t} query=${r} />`:l`
    <div className="space-y-5">
      ${i.map(o=>l`
            <${di}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function e5(){return l`
    <div className="space-y-5">
      ${[1,2,3].map(e=>l`
            <${W} key=${e} padding="md">
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
  `}function JN(){let e=z({queryKey:["gateway-status-settings"],queryFn:Bs,staleTime:1e4}),t=z({queryKey:["extensions"],queryFn:u$}),a=z({queryKey:["extension-registry"],queryFn:c$}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(f=>f.kind==="wasm_channel"||f.kind==="channel"),o=s.filter(f=>(f.kind==="wasm_channel"||f.kind==="channel")&&!f.installed),u=r.filter(f=>f.kind==="mcp_server"),c=s.filter(f=>f.kind==="mcp_server"&&!f.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:u,mcpRegistry:c,extensions:r,isLoading:d}}function t5({name:e,description:t,enabled:a,detail:n}){let r=k();return l`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${e}</span>
          <${P}
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
  `}function XN({channel:e,registryEntry:t}){let a=k(),n=t?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},u={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return l`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${n}</span>
          ${s?l`<${P}
                tone=${o[i]||"muted"}
                label=${u[i]||i}
                size="sm"
              />`:l`<${P}
                tone="muted"
                label=${a("channels.available")}
                size="sm"
              />`}
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${r}</div>
      </div>
    </div>
  `}function a5(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function n5({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=a5(e,i).filter(y=>at(s,[i("channels.builtIn"),y.id,y.name,y.description,y.detail])),u=new Set(t.map(y=>y.name)),c=t.filter(y=>at(s,[i("channels.messaging"),y.name,y.display_name,y.description,y.onboarding_state])),d=a.filter(y=>!u.has(y.name)).filter(y=>at(s,[i("channels.messaging"),y.name,y.display_name,y.description])),f=new Set(n.map(y=>y.name)),m=n.filter(y=>at(s,[i("channels.mcpServers"),y.name,y.display_name,y.description,y.active?i("channels.active"):i("channels.inactive")])),p=r.filter(y=>!f.has(y.name)).filter(y=>at(s,[i("channels.mcpServers"),y.name,y.display_name,y.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:p}}function ZN({searchQuery:e=""}){let t=k(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=JN();if(o)return l`
      <div className="space-y-5">
        <${W} padding="md">
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
    `;let{builtInChannels:u,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:m}=n5({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return u.length===0&&c.length===0&&d.length===0&&f.length===0&&m.length===0?l`<${_t} query=${e} />`:l`
    <div className="space-y-5">
      ${u.length>0&&l`
      <${W} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${u.map(p=>l`
            <${t5}
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
        <${W} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.messaging")}
          </h3>
          ${c.map(p=>l`
              <${XN}
                key=${p.name}
                channel=${p}
                registryEntry=${r.find(y=>y.name===p.name)}
              />
            `)}
          ${d.map(p=>l`
              <${XN} key=${p.name} registryEntry=${p} />
            `)}
        <//>
      `}
      ${(f.length>0||m.length>0)&&l`
        <${W} padding="md">
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
                      <${P}
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
                      <${P}
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
  `}function WN({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:u,onNearaiWallet:c,onCodexLogin:d,loginBusy:f}){let m=k(),p=e.id===t,y=Lr(e,n),b=Ks(e,n),w=$$(e,n,t,a),g=oc(e,n),v=w$(e),x=m(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[$,S]=h.default.useState(p),R=h.default.useCallback(()=>S(kt=>!kt),[]);h.default.useEffect(()=>{S(p)},[p]);let _=y?l`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${zo(e.adapter)} · ${w||e.default_model||m("llm.none")}
      </span>`:l`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${x}
      </span>`,C=e.id==="nearai"||e.id==="openai_codex",U=e.api_key_set===!0||e.has_api_key===!0,O=e.builtin?e.id==="nearai"&&v&&!U?m("llm.addApiKey"):m("llm.configure"):m("common.edit"),B=v&&e.builtin?l`
          <${T}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${r}
            onClick=${()=>i(e)}
          >
            ${O}
          <//>
        `:null,A=!p&&e.id==="nearai"?l`
          ${B}
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
        `:null,te=!p&&y&&(!C||e.id==="nearai"&&e.has_api_key===!0)?l`
        <${T}
          type="button"
          variant="primary"
          size="sm"
          disabled=${r}
          onClick=${()=>s(e)}
        >
          ${m("llm.use")}
        <//>
      `:null,ye=y?null:l`
        <${T}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${r}
          onClick=${()=>i(e)}
        >
          ${m(g==="api_key"?"llm.addApiKey":"llm.configure")}
        <//>
      `,ke=p?null:te||(C?A:ye),Je=!C&&(e.builtin&&e.id!=="bedrock"||!e.builtin)||e.id==="nearai"&&v;return l`
    <${W}
      padding="none"
      data-testid="llm-provider-card"
      data-provider-id=${e.id}
      className=${["transition-colors",p?"border-[color-mix(in_srgb,var(--v2-positive-text)_36%,var(--v2-panel-border))]":$?"border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))]":""].join(" ")}
    >
      <div className="flex w-full items-stretch hover:bg-[var(--v2-surface-soft)]">
        <button
          type="button"
          aria-expanded=${$?"true":"false"}
          aria-label=${m($?"llm.collapseDetails":"llm.expandDetails")}
          data-testid="llm-provider-disclosure"
          onClick=${R}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 px-4 py-3 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)] sm:pl-5 sm:pr-3"
        >
          <span
            className=${["h-2 w-2 shrink-0 rounded-full",p?"bg-[var(--v2-positive-text)]":y?"bg-[var(--v2-accent)]":"bg-[var(--v2-warning-text)]"].join(" ")}
          />
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
            <span className="min-w-0 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
              ${e.name||e.id}
            </span>
            <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">${e.id}</span>
            ${p&&l`<${P} tone="positive" label=${m("llm.active")} size="sm" />`}
            ${e.builtin&&!p&&l`<${P} tone="muted" label=${m("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${_}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 py-3 pr-4 sm:pr-5">
          ${ke}
          <button
            type="button"
            onClick=${R}
            data-testid="llm-provider-chevron"
            aria-label=${m($?"llm.collapseDetails":"llm.expandDetails")}
            className=${["grid h-7 w-7 place-items-center rounded-md text-[var(--v2-text-faint)] transition-transform hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]",$?"rotate-180":""].join(" ")}
          >
            <${D} name="chevron" className="h-4 w-4" />
          </button>
        </div>
      </div>

      ${$&&l`
        <div data-testid="llm-provider-details" className="border-t border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-4 sm:px-5">
          <div className="grid gap-3 text-xs text-[var(--v2-text-muted)] sm:grid-cols-3">
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${m("llm.adapter")}</div>
              <div className="mt-1 truncate">${zo(e.adapter)}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${m("llm.baseUrl")}</div>
              <div className="mt-1 truncate font-mono">${b||m("llm.none")}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${m("llm.model")}</div>
              <div className="mt-1 truncate font-mono">${w||m("llm.none")}</div>
            </div>
          </div>

          <div className="mt-4 flex flex-wrap justify-end gap-2 border-t border-[var(--v2-panel-border)] pt-3">
            ${Je&&l`
              <${T}
                type="button"
                variant="secondary"
                size="sm"
                disabled=${r}
                onClick=${()=>i(e)}
              >
                ${O}
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
  `}var r5=[{key:"active",labelKey:"llm.groupActive",dotClass:"bg-[var(--v2-positive-text)]"},{key:"ready",labelKey:"llm.groupReady",dotClass:"bg-[var(--v2-accent)]"},{key:"setup",labelKey:"llm.groupSetup",dotClass:"bg-[var(--v2-warning-text)]"}];function s5({label:e,count:t,dotClass:a}){return l`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full "+a} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${t}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `}function e_({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=k(),r=Ec({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=Tc(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return l`<${_t} query=${a} />`;let u=S$(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return l`
    <${W} className="p-4 sm:p-6">
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

      ${r.message&&l`
        <div
          className=${["mb-4 rounded-md border px-3 py-2 text-sm",r.message.tone==="error"?"border-red-400/30 bg-red-500/10 text-red-200":"border-mint/30 bg-mint/10 text-mint"].join(" ")}
          role="status"
        >
          ${r.message.text}
        </div>
      `}

      <${Cc} login=${i} />

      ${s.isLoading?l`<div className="text-sm text-[var(--v2-text-muted)]">${n("common.loading")}</div>`:s.error?l`<div className="text-sm text-red-200">${n("error.loadFailed",{what:n("llm.providers"),message:s.error.message})}</div>`:l`
            <div className="space-y-1">
              ${r5.flatMap(c=>{let d=u[c.key];return d.length?[l`
                    <section
                      key=${c.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${c.key}
                      className="mb-3"
                    >
                      <${s5}
                        label=${n(c.labelKey)}
                        count=${d.length}
                        dotClass=${c.dotClass}
                      />
                      <div className="space-y-2">
                      ${d.map(f=>l`
                          <${WN}
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

      <${Rc}
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
  `}function t_({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=k(),{activeProviderId:o,selectedModel:u,providers:c,hasActiveProvider:d}=Is({settings:e,gatewayStatus:t});if(r)return l`<${i5} />`;let f=d?o:"",m=c.find(g=>g.id===o),p=d&&(u||m?.default_model||e.selected_model)||"",y=ci(KN,e,s,i),b=at(s,[i("inference.provider"),i("inference.backend"),f,i("inference.model"),p]),w=at(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!b&&!w&&y.length===0?l`<${_t} query=${s} />`:l`
    <div className="space-y-5">
      ${b&&l`
      <${W} padding="none" className="p-4 sm:p-5">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${i("inference.provider")}</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${i("inference.backend")}</div>
            <div className="mt-1 flex items-center gap-2">
              <span className="font-mono text-lg font-semibold text-[var(--v2-text-strong)]">${f||i("inference.none")}</span>
              ${d?l`<${P} tone="positive" label=${i("inference.active")} size="sm" />`:l`<${P} tone="muted" label=${i("llm.notConfigured")} size="sm" />`}
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

      ${w&&l`
        <${e_}
          settings=${e}
          gatewayStatus=${t}
          searchQuery=${s}
        />
      `}

      ${y.map(g=>l`
            <${di}
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
  `}function i5(){return l`
    <div className="space-y-5">
      <${W} padding="md">
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
            <${W} key=${e} padding="md">
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
  `}function a_({searchQuery:e=""}){let t=k(),{lang:a,setLang:n}=tl(),r=al.find(i=>i.code===a)||al[0],s=al.filter(i=>at(e,[i.code,i.name,i.native]));return s.length===0?l`<${_t} query=${e} />`:l`
    <${W} padding="md">
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
  `}function n_({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=k();if(n)return l`
      <div className="space-y-5">
        ${[1,2].map(o=>l`
              <${W} key=${o} padding="md">
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
    `;let i=ci(QN,e,r,s);return i.length===0?l`<${_t} query=${r} />`:l`
    <div className="space-y-5">
      ${i.map(o=>l`
            <${di}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function r_(){let e=k(),[t,a]=h.default.useState(!1),n=h.default.useCallback(()=>a(!0),[]),r=h.default.useCallback(()=>a(!1),[]),s=h.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function s_({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=k(),r=r_({gatewayStatus:t,gatewayStatusQuery:a});return e?l`
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

    <${Xs}
      open=${r.confirmOpen}
      onClose=${r.closeConfirm}
      title=${n("restart.title")}
      size="sm"
    >
      <${Zs} className="space-y-3">
        <p className="text-sm text-[var(--v2-text)]">
          ${n("restart.description")}
        </p>
        <div className="rounded-xl border border-copper/25 bg-copper/10 px-3 py-2 text-xs text-copper">
          ${n("restart.warning")}
        </div>
      <//>
      <${Ws}>
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
  `:null}function i_(){let e=Y(),t=z({queryKey:["skills"],queryFn:d$}),a=I({mutationFn:f$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),n=I({mutationFn:h$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),r=I({mutationFn:({name:i,content:o})=>p$(i,{content:o}),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}});return{skills:t.data?.skills||[],query:t,fetchSkillContent:m$,installSkill:a.mutateAsync,removeSkill:n.mutateAsync,updateSkill:r.mutateAsync,isInstalling:a.isPending,isRemoving:n.isPending,isUpdating:r.isPending}}function o_({skill:e,onEdit:t,onRemove:a,onUpdate:n,isRemoving:r,isUpdating:s}){let i=k(),o=e.name||e.id,u=e.trust||e.trust_level||"installed",c=e.source_kind||"installed",d=!!e.can_edit,f=!!e.can_delete,[m,p]=h.default.useState(!1),[y,b]=h.default.useState(""),[w,g]=h.default.useState(""),[v,x]=h.default.useState(!1);h.default.useEffect(()=>{m||(b(""),g(""))},[m]);let $=h.default.useCallback(async()=>{x(!0),g("");try{let R=await t(o);b(R?.content||""),p(!0)}catch(R){g(R.message||i("skills.contentLoadFailed"))}finally{x(!1)}},[o,t,i]),S=h.default.useCallback(async()=>{(await n(o,y))?.success&&p(!1)},[y,o,n]);return l`
    <div className="ext-card border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text)]">${o}</span>
            <${P}
              tone=${String(u).toLowerCase()==="trusted"?"positive":"muted"}
              label=${u}
              size="sm"
            />
            <${P}
              tone=${c==="system"?"positive":"muted"}
              label=${i(`skills.source.${c}`)}
              size="sm"
            />
            ${e.version&&l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v${e.version}</span>`}
          </div>

          ${e.description&&l`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${e.description}</div>`}

          ${m?l`
                <div className="mt-3">
                  <${gc}
                    rows=${12}
                    value=${y}
                    className="font-mono text-xs leading-5"
                    onInput=${R=>b(R.currentTarget.value)}
                  />
                </div>
              `:l`<${o5} skill=${e} />`}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          ${d&&!m&&l`
            <${T}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${s||v}
              title=${i("skills.edit")}
              onClick=${$}
            >
              <${D} name="file" className="h-4 w-4" />
              ${i(v?"skills.loading":"skills.edit")}
            <//>
          `}
          ${m&&l`
            <${T}
              type="button"
              variant="ghost"
              size="sm"
              disabled=${s}
              onClick=${()=>{b(""),p(!1)}}
            >
              <${D} name="close" className="h-4 w-4" />
              ${i("skills.cancel")}
            <//>
            <${T}
              type="button"
              variant="primary"
              size="sm"
              disabled=${s}
              onClick=${S}
            >
              <${D} name="check" className="h-4 w-4" />
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
              <${D} name="trash" className="h-4 w-4" />
              ${i("skills.delete")}
            <//>
          `}
        </div>
      </div>
      ${w&&l`<p className="mt-2 text-xs text-[var(--v2-danger-text)]">${w}</p>`}
    </div>
  `}function o5({skill:e}){let t=k();return l`
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
        ${e.has_requirements&&l`<${hh}>requirements.txt<//>`}
        ${e.has_scripts&&l`<${hh}>scripts/<//>`}
        ${e.install_source_url&&l`<${hh}>${t("skills.imported")}<//>`}
      </div>
    `}
  `}function hh({children:e}){return l`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${e}
    </span>
  `}function l_({onInstall:e,isInstalling:t}){let a=k(),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState(""),[c,d]=h.default.useState(""),f=h.default.useCallback(async()=>{let m=l5({name:n,content:s});if(!m.name){u(a("skills.nameRequired"));return}if(!m.content){u(a("skills.contentRequired"));return}u(""),d("");try{let p=await e(m);if(!p?.success){u(p?.message||a("skills.installFailed"));return}r(""),i(""),d(p.message||a("skills.installedSuccess",{name:m.name}))}catch(p){u(p.message||a("skills.installFailed"))}},[s,n,e,a]);return l`
    <${W} padding="md">
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

      <${gn} label=${a("skills.name")} error=${o&&!n.trim()?o:""}>
        <${Mt}
          size="sm"
          value=${n}
          placeholder=${a("skills.namePlaceholder")}
          onInput=${m=>r(m.currentTarget.value)}
        />
      <//>

      <${gn} className="mt-3" label=${a("skills.content")} hint=${a("skills.contentHint")}>
        <${gc}
          rows=${5}
          value=${s}
          placeholder=${a("skills.contentPlaceholder")}
          onInput=${m=>i(m.currentTarget.value)}
        />
      <//>

      ${o&&l`<p className="mt-3 text-sm text-[var(--v2-danger-text)]">${o}</p>`}
      ${c&&l`<p className="mt-3 text-sm text-[var(--v2-positive-text)]">${c}</p>`}

      <div className="mt-4 flex justify-end">
        <${T} type="button" size="sm" disabled=${t} onClick=${f}>
          <${D} name="upload" className="h-4 w-4" />
          ${a(t?"skills.installing":"skills.install")}
        <//>
      </div>
    <//>
  `}function l5({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function u_({searchQuery:e=""}){let t=k(),{skills:a,query:n,fetchSkillContent:r,installSkill:s,removeSkill:i,updateSkill:o,isInstalling:u,isRemoving:c,isUpdating:d}=i_(),[f,m]=h.default.useState(""),[p,y]=h.default.useState(""),b=h.default.useCallback(async v=>{if(window.confirm(t("skills.confirmDelete",{name:v}))){m(""),y("");try{let x=await i(v);if(!x?.success){m(x?.message||t("skills.removeFailed"));return}y(x.message||t("skills.removed",{name:v}))}catch(x){m(x.message||t("skills.removeFailed"))}}},[i,t]),w=h.default.useCallback(async(v,x)=>{if(!x.trim())return m(t("skills.contentRequired")),y(""),{success:!1,message:t("skills.contentRequired")};m(""),y("");try{let $=await o({name:v,content:x});return $?.success?(y($.message||t("skills.updated",{name:v})),$):(m($?.message||t("skills.updateFailed")),$)}catch($){let S=$.message||t("skills.updateFailed");return m(S),{success:!1,message:S}}},[t,o]),g;if(n.isLoading)g=l`
      <${W} padding="md">
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
      <${W} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">${t("skills.failedLoad",{message:n.error.message})}</p>
        <//>
    `;else{let v=a.filter($=>at(e,[$.name,$.id,$.description,$.keywords,$.trust_level,$.source_kind,$.version])),x=c5(v);a.length===0?g=l`
        <${W} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:v.length===0?g=l`<${_t} query=${e} />`:g=l`
        <div id="skills-list">
          ${x.map($=>l`
              <${u5}
                key=${$.id}
                title=${t($.labelKey)}
                skills=${$.skills}
                onEdit=${r}
                onRemove=${b}
                onUpdate=${w}
                isRemoving=${c}
                isUpdating=${d}
              />
            `)}
        </div>
      `}return l`
    <div className="space-y-4">
      <${l_} onInstall=${s} isInstalling=${u} />
      <${d5} error=${f} result=${p} />
      ${g}
    </div>
  `}function u5({title:e,skills:t,onEdit:a,onRemove:n,onUpdate:r,isRemoving:s,isUpdating:i}){return t.length===0?null:l`
    <${W} padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${e}
      </h3>
      ${t.map(o=>l`
          <${o_}
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
  `}function c5(e){let t=[{id:"user",labelKey:"skills.group.user",skills:[]},{id:"system",labelKey:"skills.group.system",skills:[]},{id:"workspace",labelKey:"skills.group.workspace",skills:[]}],a=t[0];for(let n of e){let r=n.source_kind||"";(r==="system"?t[1]:r==="workspace"?t[2]:a).skills.push(n)}return t.filter(n=>n.skills.length>0)}function d5({error:e,result:t}){return!e&&!t?null:l`
    <div
      className=${e?"rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200":"rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200"}
    >
      ${e||t}
    </div>
  `}function Gc(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function c_(){let e=Y(),t=z({queryKey:["settings-tools"],queryFn:o$}),a=t.data?.tools||[],[n,r]=h.default.useState({}),s=I({mutationFn:async({name:o,state:u})=>Gc(await l$(o,u),"Save failed"),onSuccess:(o,{name:u,state:c})=>{e.setQueryData(["settings-tools"],d=>d&&{...d,tools:d.tools.map(f=>f.name===u?{...f,state:c}:f)}),r(d=>({...d,[u]:!0})),setTimeout(()=>r(d=>({...d,[u]:!1})),2e3)}}),i=h.default.useCallback((o,u)=>s.mutate({name:o,state:u}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}function m5({tool:e,onPermissionChange:t,isSaved:a}){let n=k(),r=[{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],s=e.locked,i=r.find(u=>u.value===e.state)||r[1],o=e.state===e.default_state;return l`
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
        ${s?l`<${P} tone=${i.tone} label=${i.label} size="sm" />`:l`
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
  `}function d_({searchQuery:e=""}){let t=k(),{tools:a,query:n,setPermission:r,savedTools:s}=c_();if(n.isLoading)return l`
      <${W} padding="md">
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
      <${W} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("tools.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let i=a.filter(o=>at(e,[o.name,o.description,o.state,o.default_state,o.locked?t("tools.disabled"):""]));return l`
    <div className="space-y-4">
      ${e&&l`
        <div className="flex justify-end">
          <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">
            ${i.length} / ${a.length}
          </span>
        </div>
      `}

      <${W} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("tools.permissions")}
        </h3>
        ${i.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("tools.noMatch")}
            </p>`:i.map(o=>l`
                  <${m5}
                    key=${o.name}
                    tool=${o}
                    onPermissionChange=${r}
                    isSaved=${s[o.name]}
                  />
                `)}
      <//>
    </div>
  `}function m_(e){return(Number(e)||0).toFixed(2)}function f5(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function f_(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function zr({label:e,value:t,description:a}){return l`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&l`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function p_({searchQuery:e=""}){let t=k(),{credits:a,query:n,authorize:r}=cc();if(!at(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return l`<${_t} query=${e} />`;let s;if(n.isLoading)s=l`
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
        <${zr}
          label=${t("traceCommons.enrollment")}
          value=${a.enrolled?t("traceCommons.enrolled"):t("traceCommons.notEnrolled")}
        />
        <${zr}
          label=${t("traceCommons.pendingCredit")}
          description=${t("traceCommons.pendingCreditDesc")}
          value=${m_(a.pending_credit)}
        />
        <${zr}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${m_(a.final_credit)}
        />
        <${zr}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${f5(a.delayed_credit_delta)}
        />
        <${zr}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${zr}
          label=${t("traceCommons.lastSubmission")}
          value=${f_(a.last_submission_at,t)}
        />
        <${zr}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${f_(a.last_credit_sync_at,t)}
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
    <${W} padding="md">
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
  `}function h_(){let e=Y(),t=z({queryKey:["admin-users"],queryFn:y$,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=I({mutationFn:b$,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=I({mutationFn:({id:i,payload:o})=>x$(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function p5({onCreate:e,isCreating:t,error:a}){let n=k(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState("member"),[d,f]=h.default.useState(!1),m=p=>{p.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:u},{onSuccess:()=>{s(""),o(""),f(!1)}})};return d?l`
    <${W} padding="md">
      <h3
        className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${n("users.newUser")}
      </h3>
      <form onSubmit=${m} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <${gn} label=${n("users.displayName")} htmlFor="user-name">
            <${Mt}
              id="user-name"
              type="text"
              value=${r}
              onChange=${p=>s(p.target.value)}
              required
            />
          <//>
          <${gn} label=${n("users.email")} htmlFor="user-email">
            <${Mt}
              id="user-email"
              type="email"
              value=${i}
              onChange=${p=>o(p.target.value)}
            />
          <//>
        </div>
        <${gn} label=${n("users.role")} htmlFor="user-role">
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
        <${D} name="plus" className="mr-2 h-4 w-4" />
        ${n("users.addUser")}
      <//>
    `}function h5({user:e}){let t=k(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return l`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]"
            >${e.display_name||e.id}</span
          >
          <${P}
            tone=${n}
            label=${e.role==="admin"?t("users.admin"):t("users.member")}
            size="sm"
          />
          <${P} tone=${a} label=${e.status||"active"} size="sm" />
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
  `}function v_({searchQuery:e=""}){let t=k(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=h_();if(n.isLoading)return l`
      <${W} padding="md">
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
      <${W} padding="lg">
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
      <${W} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("users.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let u=a.filter(c=>at(e,[c.id,c.display_name,c.email,c.role,c.status,c.last_active]));return l`
    <div className="space-y-5">
      <${p5}
        onCreate=${s}
        isCreating=${o}
        error=${i}
      />

      <${W} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("users.title",{count:u.length})}
        </h3>
        ${a.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("users.noUsers")}
            </p>`:u.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("settings.noMatchingSettings",{query:e})}
            </p>`:u.map(c=>l`<${h5} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function g_(){let e=Y(),t=z({queryKey:["settings-export"],queryFn:Xx,staleTime:3e4}),a=t.data?.settings||{},[n,r]=h.default.useState({}),[s,i]=h.default.useState(!1),o=I({mutationFn:async({key:f,value:m})=>Gc(await Zx(f,m),"Save failed"),onSuccess:(f,{key:m,value:p})=>{e.setQueryData(["settings-export"],y=>{if(!y)return y;let b={...y,settings:{...y.settings}};return p==null?delete b.settings[m]:b.settings[m]=p,b}),r(y=>({...y,[m]:!0})),setTimeout(()=>r(y=>({...y,[m]:!1})),2e3),ph.has(m)&&i(!0)}}),u=h.default.useCallback((f,m)=>o.mutate({key:f,value:m}),[o]),c=I({mutationFn:Wx,onSuccess:(f,m)=>{e.invalidateQueries({queryKey:["settings-export"]}),Object.keys(m?.settings||{}).some(y=>ph.has(y))&&i(!0)}}),d=h.default.useCallback(f=>c.mutateAsync(f),[c]);return{settings:a,query:t,save:u,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error||c.error}}function vh(){let e=k(),{tab:t}=it(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=Ba(),s=r?"inference":"language",i=t||s,{settings:o,query:u,save:c,savedKeys:d,needsRestart:f,saveError:m}=g_(),[p,y]=h.default.useState("");h.default.useEffect(()=>{y("")},[i]);let b=u.isLoading,w={inference:l`<${t_}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${b}
      searchQuery=${p}
    />`,agent:l`<${YN}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${b}
      searchQuery=${p}
    />`,channels:l`<${ZN} searchQuery=${p} />`,networking:l`<${n_}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${b}
      searchQuery=${p}
    />`,tools:l`<${d_} searchQuery=${p} />`,skills:l`<${u_} searchQuery=${p} />`,traces:l`<${p_} searchQuery=${p} />`,users:l`<${v_} searchQuery=${p} />`,language:l`<${a_} searchQuery=${p} />`},g=R=>R==="users"||R==="inference",v=R=>Object.prototype.hasOwnProperty.call(w,R),x=Object.keys(w).filter(R=>r||!g(R)),S=v(s)&&x.includes(s)?s:x[0]||"language";return!v(i)||!r&&g(i)?l`<${ot} to=${`/settings/${S}`} replace />`:l`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${f&&l`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${s_}
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

            ${w[i]}
          </div>
        </div>
      </div>
    </div>
  `}var gh=Object.freeze({todo:!0});function y_(){return Promise.resolve({users:[],total:0,...gh})}function b_(e){return Promise.resolve(null)}function x_(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function $_(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function w_(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function S_(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function N_(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function __(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function k_(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...gh})}function R_(e="day",t){return Promise.resolve({entries:[],...gh})}function C_(){return z({queryKey:["admin","usage-summary"],queryFn:k_,refetchInterval:3e4})}function Yc(e="day",t){return z({queryKey:["admin","usage",e,t],queryFn:()=>R_(e,t),refetchInterval:3e4})}function mi(){let e=Y(),t=z({queryKey:["admin","users"],queryFn:y_,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=I({mutationFn:x_,onSuccess:s}),o=I({mutationFn:({id:m,payload:p})=>$_(m,p),onSuccess:s}),u=I({mutationFn:m=>w_(m),onSuccess:s}),c=I({mutationFn:m=>S_(m),onSuccess:s}),d=I({mutationFn:m=>N_(m),onSuccess:s}),f=I({mutationFn:({userId:m,name:p})=>__(m,p)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(m,p)=>o.mutateAsync({id:m,payload:p}),deleteUser:u.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(m,p)=>f.mutateAsync({userId:m,name:p}),newToken:f.data,clearToken:()=>f.reset()}}function E_(e){return z({queryKey:["admin","user",e],queryFn:()=>b_(e),enabled:!!e,refetchInterval:1e4})}function Qa(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function Ca(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function T_(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function sr(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function fi(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function pi(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function hi(e){return e==="admin"?"signal":"muted"}function A_(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function D_(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function M_(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function O_(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function L_(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function v5({users:e,onSelectUser:t}){let a=k(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?l`
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
                <td className="py-3 pr-4"><${P} tone=${hi(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${P} tone=${pi(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${sr(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:l`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function U_({onSelectUser:e,onNavigateTab:t}){let a=k(),n=C_(),{users:r,query:s}=mi(),i=n.data||{},o=A_(r),u=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?l`
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
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:T_(i.uptime_seconds)})}</span>
          `}
        </div>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <${tt}
            label=${a("admin.dashboard.totalUsers")}
            value=${String(o.total)}
            tone=${o.total>0?"success":"muted"}
          />
          <${tt}
            label=${a("admin.dashboard.activeUsers")}
            value=${String(o.active)}
            tone="success"
          />
          <${tt}
            label=${a("admin.dashboard.suspended")}
            value=${String(o.suspended)}
            tone=${o.suspended>0?"danger":"muted"}
          />
          <${tt}
            label=${a("admin.dashboard.admins")}
            value=${String(o.admins)}
            tone="signal"
          />
        </div>
      <//>

      <${j} className="p-5 sm:p-6">
        <h3 className="mb-5 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.usage30d")}</h3>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <${tt}
            label=${a("admin.dashboard.totalJobs")}
            value=${String(c.total||0)}
            tone="muted"
          />
          <${tt}
            label=${a("admin.dashboard.llmCalls")}
            value=${String(u.llm_calls||0)}
            tone="muted"
          />
          <${tt}
            label=${a("admin.dashboard.totalCost")}
            value=${Ca(u.total_cost)}
            tone="signal"
          />
          <${tt}
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
        <${v5} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var g5=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function y5({value:e,max:t}){let a=t>0?e/t*100:0;return l`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function j_({onSelectUser:e}){let t=k(),[a,n]=h.default.useState("day"),r=Yc(a),s=r.data?.usage||[],i=M_(s),o=O_(s),u=L_(i),c=i.length>0?i[0].cost:0;return r.isLoading?l`
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
            ${g5.map(d=>l`
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
                <${tt} label=${t("admin.usage.totalCalls")} value=${u.calls.toLocaleString()} tone="muted" />
                <${tt} label=${t("admin.usage.inputTokens")} value=${Qa(u.input_tokens)} tone="muted" />
                <${tt} label=${t("admin.usage.outputTokens")} value=${Qa(u.output_tokens)} tone="muted" />
                <${tt} label=${t("admin.usage.totalCost")} value=${Ca(u.cost.toFixed(2))} tone="signal" />
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
                          ${fi(d.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Qa(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Qa(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${Ca(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${y5} value=${d.cost} max=${c} />
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
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Qa(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Qa(d.output_tokens)}</td>
                      <td className="py-3 font-mono text-xs text-iron-100">${Ca(d.cost.toFixed(2))}</td>
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
  `}function P_({userId:e,onBack:t}){let a=k(),n=E_(e),r=Yc("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:u,createToken:c,newToken:d,clearToken:f}=mi(),[m,p]=h.default.useState(null),[y,b]=h.default.useState(!1),w=n.data,g=r.data?.usage||[];if(h.default.useEffect(()=>{w&&m===null&&p(w.role)},[w]),n.isLoading)return l`
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
    `;if(!w)return null;let v=async()=>{m&&m!==w.role&&await o(w.id,{role:m})},x=async()=>{await u(w.id),t()},$=async()=>{let S=window.prompt(a("admin.users.tokenNamePrompt",{name:w.display_name||a("admin.users.userFallback")}));S&&await c(w.id,S)};return l`
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
            <h2 className="text-2xl font-semibold tracking-tight text-white">${w.display_name||w.id}</h2>
            <div className="mt-2 flex items-center gap-2">
              <${P} tone=${hi(w.role)} label=${w.role||"member"} />
              <${P} tone=${pi(w.status)} label=${w.status||"active"} />
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            ${w.status==="active"?l`<${T} variant="secondary" onClick=${()=>s(w.id)}>${a("admin.users.suspend")}<//>`:l`<${T} variant="secondary" onClick=${()=>i(w.id)}>${a("admin.users.activate")}<//>`}
            <${T} variant="secondary" onClick=${$}>${a("admin.users.createToken")}<//>
            <button
              onClick=${()=>b(!0)}
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
          <${ir} label=${a("admin.user.id")}>
            <span className="font-mono text-xs">${w.id}</span>
          <//>
          <${ir} label=${a("admin.user.email")}>${w.email||a("admin.user.notSet")}<//>
          <${ir} label=${a("admin.user.created")}>${sr(w.created_at)}<//>
          <${ir} label=${a("admin.user.lastLogin")}>${sr(w.last_login_at)}<//>
          ${w.created_by&&l`
            <${ir} label=${a("admin.user.createdBy")}>
              <span className="font-mono text-xs">${fi(w.created_by)}</span>
            <//>
          `}
        <//>

        <${j} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${ir} label=${a("admin.user.jobs")}>${w.job_count??0}<//>
          <${ir} label=${a("admin.user.totalCost")}>${Ca(w.total_cost)}<//>
          <${ir} label=${a("admin.user.lastActive")}>${sr(w.last_active_at)}<//>
        <//>
      </div>

      <${j} className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.roleManagement")}</h3>
        <div className="flex items-end gap-3">
          <div>
            <label className="mb-1 block text-xs text-iron-300">${a("admin.user.currentRole")}</label>
            <select
              value=${m||w.role}
              onChange=${S=>p(S.target.value)}
              className="v2-select h-9 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            >
              <option value="member">${a("admin.users.member")}</option>
              <option value="admin">${a("admin.users.admin")}</option>
            </select>
          </div>
          <${T} onClick=${v} disabled=${!m||m===w.role}>
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
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Qa(S.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Qa(S.output_tokens)}</td>
                          <td className="py-3 font-mono text-xs text-iron-100">${Ca(S.total_cost)}</td>
                        </tr>
                      `)}
                  </tbody>
                </table>
              </div>
            `}
      <//>

      ${y&&l`
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick=${()=>b(!1)}>
          <div className="w-full max-w-md rounded-xl border border-white/10 bg-iron-900 p-6" onClick=${S=>S.stopPropagation()}>
            <h3 className="text-lg font-semibold text-white">${a("admin.users.deleteUserTitle")}</h3>
            <p className="mt-2 text-sm text-iron-300">
              ${a("admin.users.deleteUserDesc",{name:w.display_name})}
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <${T} variant="ghost" onClick=${()=>b(!1)}>${a("admin.users.cancel")}<//>
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
  `}function b5(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function x5({token:e,onDismiss:t}){let a=k(),[n,r]=h.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return l`
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
  `}function $5({onCreate:e,isCreating:t,error:a}){let n=k(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState("member"),[d,f]=h.default.useState(!1),m=async p=>{p.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:u}),s(""),o(""),f(!1))};return d?l`
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
          <${T} type="submit" disabled=${t}>
            ${n(t?"admin.users.creating":"admin.users.createUser")}
          <//>
          <${T} variant="ghost" type="button" onClick=${()=>f(!1)}>${n("admin.users.cancel")}<//>
        </div>
      </form>
    <//>
  `:l`
      <${T} variant="secondary" onClick=${()=>f(!0)}>
        <${D} name="plus" className="mr-2 h-4 w-4" />
        ${n("admin.users.newUser")}
      <//>
    `}function w5({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=k();return l`
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
  `}function S5({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=k();return l`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${P} tone=${hi(e.role)} label=${e.role||"member"} />
          <${P} tone=${pi(e.status)} label=${e.status||"active"} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          ${e.email&&l`<span className="font-mono text-xs text-iron-300">${e.email}</span>`}
          <span className="font-mono text-xs text-iron-700">${fi(e.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-iron-300 sm:inline">
          ${e.job_count!=null?i("admin.users.jobsCount",{count:e.job_count}):""}
          ${e.total_cost!=null?` \xB7 ${Ca(e.total_cost)}`:""}
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
  `}function F_({selectedUserId:e,onSelectUser:t}){let a=k(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:u,updateUser:c,deleteUser:d,suspendUser:f,activateUser:m,createToken:p,newToken:y,clearToken:b}=mi(),[w,g]=h.default.useState(""),[v,x]=h.default.useState("all"),[$,S]=h.default.useState(null),R=D_(n,{search:w,filter:v}),_=b5(a),C=O=>{S({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{f(O),S(null)}})},U=async(O,B)=>{let A=window.prompt(a("admin.users.tokenNamePrompt",{name:B||a("admin.users.userFallback")}));A&&await p(O,A)};return r.isLoading?l`
      <${j} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-3 w-24 rounded" />
        ${[1,2,3].map(O=>l`
          <div key=${O} className="flex items-center justify-between border-t border-iron-700 py-3.5 first:border-0">
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
      ${y&&l`
        <${x5}
          token=${y.token||y.plaintext_token}
          onDismiss=${b}
        />
      `}

      <${$5} onCreate=${i} isCreating=${o} error=${u} />

      <${j} className="p-5 sm:p-6">
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${a("admin.users.title",{count:R.length,total:n.length})}
          </h3>
          <div className="flex items-center gap-2">
            <input
              type="text"
              placeholder=${a("admin.users.searchPlaceholder")}
              value=${w}
              onChange=${O=>g(O.target.value)}
              className="h-8 w-48 rounded-md border border-iron-700 bg-iron-800/70 px-3 text-xs text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
            />
            <div className="flex gap-1">
              ${_.map(O=>l`
                  <button
                    key=${O.value}
                    onClick=${()=>x(O.value)}
                    className=${["rounded-md px-2.5 py-1.5 text-[11px] font-medium",v===O.value?"border border-signal/35 bg-signal/10 text-iron-100":"border border-transparent text-iron-300 hover:text-iron-100"].join(" ")}
                  >
                    ${O.label}
                  </button>
                `)}
            </div>
          </div>
        </div>

        ${R.length===0?l`<p className="py-4 text-sm text-iron-300">${a("admin.users.noMatch")}</p>`:R.map(O=>l`
                <${S5}
                  key=${O.id}
                  user=${O}
                  onSelect=${t}
                  onSuspend=${C}
                  onActivate=${m}
                  onChangeRole=${(B,A)=>c(B,{role:A})}
                  onCreateToken=${U}
                />
              `)}
      <//>

      ${$&&l`
        <${w5}
          title=${$.title}
          message=${$.message}
          confirmLabel=${$.confirmLabel}
          onConfirm=${$.onConfirm}
          onCancel=${()=>S(null)}
        />
      `}
    </div>
  `}function z_(){let{tab:e="dashboard"}=it(),t=fe(),[a,n]=h.default.useState(null),r=h.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=h.default.useCallback(()=>{n(null)},[]),i={dashboard:l`<${U_}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?l`<${P_} userId=${a} onBack=${s} />`:l`<${F_}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:l`<${j_} onSelectUser=${r} />`};return i[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:l`<${ot} to="/admin/dashboard" replace />`}var N5=2e3,_5=500,k5=2e3,R5=new Set([403,404]),C5=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function E5(e=globalThis.location){let t=new URLSearchParams(e?.search||"");return C5.reduce((a,[n,r,s])=>{let i=t.get(r)?.trim();return i?(a[n]=i,a.active.push({key:n,param:r,labelKey:s,value:i})):a[n]=null,a},{active:[]})}function q_(){let e=je(),t=h.default.useMemo(()=>E5(e),[e.search]),[a,n]=h.default.useState([]),[r,s]=h.default.useState("all"),[i,o]=h.default.useState(""),[u,c]=h.default.useState(!1),[d,f]=h.default.useState(!0),[m,p]=h.default.useState(!0),[y,b]=h.default.useState(null),[w,g]=h.default.useState(!1),v=h.default.useRef(new Set),x=h.default.useRef(0),$=h.default.useCallback(async()=>{if(w)return;let _=++x.current;p(!0);try{let C=await cx({limit:_5,level:r==="all"?null:r,target:i.trim()||null,threadId:t.threadId,runId:t.runId,turnId:t.turnId,toolCallId:t.toolCallId,toolName:t.toolName,source:t.source});if(_!==x.current)return;let U=v.current,B=jw(C).entries.filter(A=>!U.has(A.id));n(B),b(null)}catch(C){if(_!==x.current)return;if(R5.has(C?.status)){n([]),b(null),g(!0);return}b(C)}finally{_===x.current&&p(!1)}},[w,r,t,i]);h.default.useEffect(()=>{$()},[$]),h.default.useEffect(()=>{if(u||w)return;let _=setInterval($,N5);return()=>clearInterval(_)},[w,$,u]);let S=h.default.useCallback(()=>{c(_=>!_)},[]),R=h.default.useCallback(()=>{let _=[...v.current,...a.map(C=>C.id)].slice(-k5);v.current=new Set(_),n([])},[a]);return{entries:a,totalCount:a.length,paused:u,togglePause:S,clearEntries:R,levelFilter:r,setLevelFilter:s,targetFilter:i,setTargetFilter:o,autoScroll:d,setAutoScroll:f,serverLevel:null,changeServerLevel:async()=>{},scope:t,status:y?"error":m?"loading":"ready",isLoading:m,error:y}}var T5=["all","trace","debug","info","warn","error"],A5=["trace","debug","info","warn","error"],B_={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},D5={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function M5({entry:e}){let t=k(),[a,n]=h.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=B_[e.level]||B_.info,i=D5[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(u=>!!u.value);return l`
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
  `}function H_({value:e,onChange:t,options:a,labelKey:n,t:r}){return l`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>l`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function O5({label:e,value:t,scopeKey:a}){return l`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function K_(){let e=k(),{entries:t,totalCount:a,paused:n,togglePause:r,clearEntries:s,levelFilter:i,setLevelFilter:o,targetFilter:u,setTargetFilter:c,autoScroll:d,setAutoScroll:f,serverLevel:m,changeServerLevel:p,scope:y,isLoading:b,error:w}=q_(),g=h.default.useRef(null),v=h.default.useRef(!0);h.default.useEffect(()=>{d&&v.current&&g.current&&(g.current.scrollTop=0)},[t,d]);let x=h.default.useCallback(R=>{v.current=R.currentTarget.scrollTop<=48},[]),$=t.length>0,S=y?.active||[];return l`
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${H_}
          value=${i}
          onChange=${o}
          options=${T5}
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
            ${S.map(R=>l`<${O5} key=${R.param} scopeKey=${R.param} label=${e(R.labelKey)} value=${R.value} />`)}
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
            <${H_}
              value=${m}
              onChange=${p}
              options=${A5}
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
        ${w&&$?l`
              <div
                className="sticky top-0 z-10 border-b border-red-500/25 bg-red-950/70 px-4 py-2 text-xs text-red-100 backdrop-blur"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:w.message||w.statusText||"Request failed"})}
              </div>
            `:null}
        ${w&&!$?l`
              <div
                className="flex h-full items-center justify-center px-6 text-center text-sm text-red-300"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:w.message||w.statusText||"Request failed"})}
              </div>
            `:b&&!$?l`
                <div
                  className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
                >
                  ${e("common.loading")}
                </div>
              `:$?t.map(R=>l`<${M5} key=${R.id} entry=${R} />`):l`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function Q_(){return l`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function L5({auth:e}){let t=fe(),n=je().state?.from,r=n?`${n.pathname||Or}${n.search||""}${n.hash||""}`:Or,s=`/v2${r==="/"?"":r}`,i=h.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?l`<${Q_} />`:e.isAuthenticated?l`<${ot} to=${r} replace />`:l`<${C1}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function U5({auth:e,children:t}){let a=je();return e.isChecking?l`<${Q_} />`:e.isAuthenticated?t:l`<${ot} to="/login" replace state=${{from:a}} />`}function j5({auth:e}){return l`
    <${U5} auth=${e}>
      <${n1}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        onSignOut=${e.signOut}
      />
    <//>
  `}function I_({auth:e}){return e.isAdmin?l`<${z_} />`:l`<${ot} to=${Or} replace />`}function V_(){let e=Gx();return l`
    <${lp} basename="/v2">
      <${sp}>
        <${pe} path="/login" element=${l`<${L5} auth=${e} />`} />
        <${pe} path="/" element=${l`<${j5} auth=${e} />`}>
          <${pe} index element=${l`<${ot} to=${Or} replace />`} />
          <${pe} path="overview" element=${l`<${ot} to=${Or} replace />`} />
          <${pe} path="welcome" element=${l`<${Kw} />`} />
          <${pe} path="chat" element=${l`<${Qp} />`} />
          <${pe} path="chat/:threadId" element=${l`<${Qp} />`} />
          <${pe} path="workspace" element=${l`<${Yp} />`} />
          <${pe} path="workspace/*" element=${l`<${Yp} />`} />
          <${pe} path="projects" element=${l`<${Yo} />`} />
          <${pe} path="projects/:projectId" element=${l`<${Yo} />`} />
          <${pe} path="projects/:projectId/missions/:missionId" element=${l`<${Yo} />`} />
          <${pe} path="projects/:projectId/threads/:threadId" element=${l`<${Yo} />`} />
          <${pe} path="missions" element=${l`<${Xp} />`} />
          <${pe} path="missions/:missionId" element=${l`<${Xp} />`} />
          <${pe} path="jobs" element=${l`<${eh} />`} />
          <${pe} path="jobs/:jobId" element=${l`<${eh} />`} />
          <${pe} path="routines" element=${l`<${ah} />`} />
          <${pe} path="routines/:routineId" element=${l`<${ah} />`} />
          <${pe} path="automations" element=${l`<${VS} />`} />
          <${pe} path="extensions" element=${l`<${fh} />`} />
          <${pe} path="extensions/:tab" element=${l`<${fh} />`} />
          <${pe} path="logs" element=${l`<${K_} />`} />
          <${pe} path="settings" element=${l`<${vh} />`} />
          <${pe} path="settings/:tab" element=${l`<${vh} />`} />
          <${pe} path="admin" element=${l`<${I_} auth=${e} />`} />
          <${pe} path="admin/:tab" element=${l`<${I_} auth=${e} />`} />
        <//>
        <${pe} path="*" element=${l`<${ot} to=${Or} replace />`} />
      <//>
    <//>
  `}xh("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveTools":"Auto-approve tools","settings.field.autoApproveToolsDesc":"Skip approval for all tool calls","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Persistent memory","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.searching":"Searching...","workspace.noResults":"No results.","workspace.noFiles":"No files in workspace.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a workspace file","workspace.pickFileDesc":"Choose a memory document from the tree or search results to inspect and edit it.","workspace.edit":"Edit","workspace.cancel":"Cancel","workspace.save":"Save","workspace.saving":"Saving","workspace.parent":"Parent: {path}","workspace.searchPlaceholder":"Search memory...","workspace.unableOpenDirectory":"Unable to open directory","workspace.unableSaveFile":"Unable to save file","workspace.savedPath":"Saved {path}","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up an autonomous workspace for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open workspace","projects.openGeneralWorkspace":"Open general workspace","projects.noDescription":"No project description yet. The workspace is still being shaped by active missions and thread history.","projects.general.label":"General workspace","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,G_.createRoot)(document.getElementById("v2-root")).render(l`
  <${$h}>
    <${dd} client=${At}>
      <${V_} />
    <//>
  <//>
`);
