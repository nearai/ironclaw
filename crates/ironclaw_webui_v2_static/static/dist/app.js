import{a as En,b as ze,c as Qe,d as h,e as l,f as Hh,g as Qh,h as cl,i as C,j as dl}from"./chunks/chunk-IGTNS7XG.js";var dv=En(xl=>{"use strict";var vR=Symbol.for("react.transitional.element"),gR=Symbol.for("react.fragment");function cv(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:vR,type:e,key:n,ref:t!==void 0?t:null,props:a}}xl.Fragment=gR;xl.jsx=cv;xl.jsxs=cv});var kd=En((PL,mv)=>{"use strict";mv.exports=dv()});var kv=En(De=>{"use strict";function Md(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<El(r,t))e[n]=t,e[a]=r,a=n;else break e}}function Fa(e){return e.length===0?null:e[0]}function Al(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],u=i+1,c=e[u];if(0>El(o,a))u<r&&0>El(c,o)?(e[n]=c,e[u]=a,n=u):(e[n]=o,e[i]=a,n=i);else if(u<r&&0>El(c,a))e[n]=c,e[u]=a,n=u;else break e}}return t}function El(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}De.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(gv=performance,De.unstable_now=function(){return gv.now()}):(Td=Date,yv=Td.now(),De.unstable_now=function(){return Td.now()-yv});var gv,Td,yv,rn=[],Dn=[],$R=1,la=null,$t=3,Od=!1,Ai=!1,Di=!1,Ld=!1,$v=typeof setTimeout=="function"?setTimeout:null,wv=typeof clearTimeout=="function"?clearTimeout:null,bv=typeof setImmediate<"u"?setImmediate:null;function Tl(e){for(var t=Fa(Dn);t!==null;){if(t.callback===null)Al(Dn);else if(t.startTime<=e)Al(Dn),t.sortIndex=t.expirationTime,Md(rn,t);else break;t=Fa(Dn)}}function Pd(e){if(Di=!1,Tl(e),!Ai)if(Fa(rn)!==null)Ai=!0,es||(es=!0,Wr());else{var t=Fa(Dn);t!==null&&Ud(Pd,t.startTime-e)}}var es=!1,Mi=-1,Sv=5,Nv=-1;function _v(){return Ld?!0:!(De.unstable_now()-Nv<Sv)}function Ad(){if(Ld=!1,es){var e=De.unstable_now();Nv=e;var t=!0;try{e:{Ai=!1,Di&&(Di=!1,wv(Mi),Mi=-1),Od=!0;var a=$t;try{t:{for(Tl(e),la=Fa(rn);la!==null&&!(la.expirationTime>e&&_v());){var n=la.callback;if(typeof n=="function"){la.callback=null,$t=la.priorityLevel;var r=n(la.expirationTime<=e);if(e=De.unstable_now(),typeof r=="function"){la.callback=r,Tl(e),t=!0;break t}la===Fa(rn)&&Al(rn),Tl(e)}else Al(rn);la=Fa(rn)}if(la!==null)t=!0;else{var s=Fa(Dn);s!==null&&Ud(Pd,s.startTime-e),t=!1}}break e}finally{la=null,$t=a,Od=!1}t=void 0}}finally{t?Wr():es=!1}}}var Wr;typeof bv=="function"?Wr=function(){bv(Ad)}:typeof MessageChannel<"u"?(Dd=new MessageChannel,xv=Dd.port2,Dd.port1.onmessage=Ad,Wr=function(){xv.postMessage(null)}):Wr=function(){$v(Ad,0)};var Dd,xv;function Ud(e,t){Mi=$v(function(){e(De.unstable_now())},t)}De.unstable_IdlePriority=5;De.unstable_ImmediatePriority=1;De.unstable_LowPriority=4;De.unstable_NormalPriority=3;De.unstable_Profiling=null;De.unstable_UserBlockingPriority=2;De.unstable_cancelCallback=function(e){e.callback=null};De.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):Sv=0<e?Math.floor(1e3/e):5};De.unstable_getCurrentPriorityLevel=function(){return $t};De.unstable_next=function(e){switch($t){case 1:case 2:case 3:var t=3;break;default:t=$t}var a=$t;$t=t;try{return e()}finally{$t=a}};De.unstable_requestPaint=function(){Ld=!0};De.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=$t;$t=e;try{return t()}finally{$t=a}};De.unstable_scheduleCallback=function(e,t,a){var n=De.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:$R++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,Md(Dn,e),Fa(rn)===null&&e===Fa(Dn)&&(Di?(wv(Mi),Mi=-1):Di=!0,Ud(Pd,a-n))):(e.sortIndex=r,Md(rn,e),Ai||Od||(Ai=!0,es||(es=!0,Wr()))),e};De.unstable_shouldYield=_v;De.unstable_wrapCallback=function(e){var t=$t;return function(){var a=$t;$t=t;try{return e.apply(this,arguments)}finally{$t=a}}}});var Cv=En((y6,Rv)=>{"use strict";Rv.exports=kv()});var Tv=En(Rt=>{"use strict";var wR=Qe();function Ev(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Mn(){}var kt={d:{f:Mn,r:function(){throw Error(Ev(522))},D:Mn,C:Mn,L:Mn,m:Mn,X:Mn,S:Mn,M:Mn},p:0,findDOMNode:null},SR=Symbol.for("react.portal");function NR(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:SR,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var Oi=wR.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function Dl(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}Rt.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=kt;Rt.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(Ev(299));return NR(e,t,null,a)};Rt.flushSync=function(e){var t=Oi.T,a=kt.p;try{if(Oi.T=null,kt.p=2,e)return e()}finally{Oi.T=t,kt.p=a,kt.d.f()}};Rt.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,kt.d.C(e,t))};Rt.prefetchDNS=function(e){typeof e=="string"&&kt.d.D(e)};Rt.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=Dl(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?kt.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&kt.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};Rt.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=Dl(t.as,t.crossOrigin);kt.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&kt.d.M(e)};Rt.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=Dl(a,t.crossOrigin);kt.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};Rt.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=Dl(t.as,t.crossOrigin);kt.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else kt.d.m(e)};Rt.requestFormReset=function(e){kt.d.r(e)};Rt.unstable_batchedUpdates=function(e,t){return e(t)};Rt.useFormState=function(e,t,a){return Oi.H.useFormState(e,t,a)};Rt.useFormStatus=function(){return Oi.H.useHostTransitionStatus()};Rt.version="19.1.0"});var Mv=En((x6,Dv)=>{"use strict";function Av(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(Av)}catch(e){console.error(e)}}Av(),Dv.exports=Tv()});var L0=En(ec=>{"use strict";var st=Cv(),ty=Qe(),_R=Mv();function U(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function ay(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function wo(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function ny(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function Ov(e){if(wo(e)!==e)throw Error(U(188))}function kR(e){var t=e.alternate;if(!t){if(t=wo(e),t===null)throw Error(U(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return Ov(r),e;if(s===n)return Ov(r),t;s=s.sibling}throw Error(U(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(U(189))}}if(a.alternate!==n)throw Error(U(190))}if(a.tag!==3)throw Error(U(188));return a.stateNode.current===a?e:t}function ry(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=ry(e),t!==null)return t;e=e.sibling}return null}var Te=Object.assign,RR=Symbol.for("react.element"),Ml=Symbol.for("react.transitional.element"),Ii=Symbol.for("react.portal"),os=Symbol.for("react.fragment"),sy=Symbol.for("react.strict_mode"),hm=Symbol.for("react.profiler"),CR=Symbol.for("react.provider"),iy=Symbol.for("react.consumer"),cn=Symbol.for("react.context"),df=Symbol.for("react.forward_ref"),vm=Symbol.for("react.suspense"),gm=Symbol.for("react.suspense_list"),mf=Symbol.for("react.memo"),Pn=Symbol.for("react.lazy");Symbol.for("react.scope");var ym=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var ER=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var Lv=Symbol.iterator;function Li(e){return e===null||typeof e!="object"?null:(e=Lv&&e[Lv]||e["@@iterator"],typeof e=="function"?e:null)}var TR=Symbol.for("react.client.reference");function bm(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===TR?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case os:return"Fragment";case hm:return"Profiler";case sy:return"StrictMode";case vm:return"Suspense";case gm:return"SuspenseList";case ym:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case Ii:return"Portal";case cn:return(e.displayName||"Context")+".Provider";case iy:return(e._context.displayName||"Context")+".Consumer";case df:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case mf:return t=e.displayName||null,t!==null?t:bm(e.type)||"Memo";case Pn:t=e._payload,e=e._init;try{return bm(e(t))}catch{}}return null}var Ki=Array.isArray,te=ty.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,ve=_R.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,br={pending:!1,data:null,method:null,action:null},xm=[],ls=-1;function Qa(e){return{current:e}}function mt(e){0>ls||(e.current=xm[ls],xm[ls]=null,ls--)}function Oe(e,t){ls++,xm[ls]=e.current,e.current=t}var Ia=Qa(null),oo=Qa(null),Qn=Qa(null),uu=Qa(null);function cu(e,t){switch(Oe(Qn,t),Oe(oo,e),Oe(Ia,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?zg(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=zg(t),e=S0(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}mt(Ia),Oe(Ia,e)}function Rs(){mt(Ia),mt(oo),mt(Qn)}function $m(e){e.memoizedState!==null&&Oe(uu,e);var t=Ia.current,a=S0(t,e.type);t!==a&&(Oe(oo,e),Oe(Ia,a))}function du(e){oo.current===e&&(mt(Ia),mt(oo)),uu.current===e&&(mt(uu),yo._currentValue=br)}var wm=Object.prototype.hasOwnProperty,ff=st.unstable_scheduleCallback,jd=st.unstable_cancelCallback,AR=st.unstable_shouldYield,DR=st.unstable_requestPaint,Ka=st.unstable_now,MR=st.unstable_getCurrentPriorityLevel,oy=st.unstable_ImmediatePriority,ly=st.unstable_UserBlockingPriority,mu=st.unstable_NormalPriority,OR=st.unstable_LowPriority,uy=st.unstable_IdlePriority,LR=st.log,PR=st.unstable_setDisableYieldValue,So=null,Gt=null;function qn(e){if(typeof LR=="function"&&PR(e),Gt&&typeof Gt.setStrictMode=="function")try{Gt.setStrictMode(So,e)}catch{}}var Yt=Math.clz32?Math.clz32:FR,UR=Math.log,jR=Math.LN2;function FR(e){return e>>>=0,e===0?32:31-(UR(e)/jR|0)|0}var Ol=256,Ll=4194304;function vr(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Fu(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=vr(n):(i&=o,i!==0?r=vr(i):a||(a=o&~e,a!==0&&(r=vr(a))))):(o=n&~s,o!==0?r=vr(o):i!==0?r=vr(i):a||(a=n&~e,a!==0&&(r=vr(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function No(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function BR(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function cy(){var e=Ol;return Ol<<=1,(Ol&4194048)===0&&(Ol=256),e}function dy(){var e=Ll;return Ll<<=1,(Ll&62914560)===0&&(Ll=4194304),e}function Fd(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function _o(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function zR(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,u=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Yt(a),m=1<<d;o[d]=0,u[d]=-1;var f=c[d];if(f!==null)for(c[d]=null,d=0;d<f.length;d++){var p=f[d];p!==null&&(p.lane&=-536870913)}a&=~m}n!==0&&my(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function my(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Yt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function fy(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Yt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function pf(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function hf(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function py(){var e=ve.p;return e!==0?e:(e=window.event,e===void 0?32:M0(e.type))}function qR(e,t){var a=ve.p;try{return ve.p=e,t()}finally{ve.p=a}}var nr=Math.random().toString(36).slice(2),wt="__reactFiber$"+nr,jt="__reactProps$"+nr,js="__reactContainer$"+nr,Sm="__reactEvents$"+nr,IR="__reactListeners$"+nr,KR="__reactHandles$"+nr,Pv="__reactResources$"+nr,ko="__reactMarker$"+nr;function vf(e){delete e[wt],delete e[jt],delete e[Sm],delete e[IR],delete e[KR]}function us(e){var t=e[wt];if(t)return t;for(var a=e.parentNode;a;){if(t=a[js]||a[wt]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=Kg(e);e!==null;){if(a=e[wt])return a;e=Kg(e)}return t}e=a,a=e.parentNode}return null}function Fs(e){if(e=e[wt]||e[js]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function Hi(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(U(33))}function bs(e){var t=e[Pv];return t||(t=e[Pv]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function ct(e){e[ko]=!0}var hy=new Set,vy={};function Tr(e,t){Cs(e,t),Cs(e+"Capture",t)}function Cs(e,t){for(vy[e]=t,e=0;e<t.length;e++)hy.add(t[e])}var HR=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),Uv={},jv={};function QR(e){return wm.call(jv,e)?!0:wm.call(Uv,e)?!1:HR.test(e)?jv[e]=!0:(Uv[e]=!0,!1)}function Jl(e,t,a){if(QR(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function Pl(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function sn(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var Bd,Fv;function rs(e){if(Bd===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);Bd=t&&t[1]||"",Fv=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+Bd+e+Fv}var zd=!1;function qd(e,t){if(!e||zd)return"";zd=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var m=function(){throw Error()};if(Object.defineProperty(m.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(m,[])}catch(p){var f=p}Reflect.construct(e,[],m)}else{try{m.call()}catch(p){f=p}e.call(m.prototype)}}else{try{throw Error()}catch(p){f=p}(m=e())&&typeof m.catch=="function"&&m.catch(function(){})}}catch(p){if(p&&f&&typeof p.stack=="string")return[p.stack,f.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var u=i.split(`
`),c=o.split(`
`);for(r=n=0;n<u.length&&!u[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===u.length||r===c.length)for(n=u.length-1,r=c.length-1;1<=n&&0<=r&&u[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(u[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||u[n]!==c[r]){var d=`
`+u[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{zd=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?rs(a):""}function VR(e){switch(e.tag){case 26:case 27:case 5:return rs(e.type);case 16:return rs("Lazy");case 13:return rs("Suspense");case 19:return rs("SuspenseList");case 0:case 15:return qd(e.type,!1);case 11:return qd(e.type.render,!1);case 1:return qd(e.type,!0);case 31:return rs("Activity");default:return""}}function Bv(e){try{var t="";do t+=VR(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function ca(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function gy(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function GR(e){var t=gy(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function fu(e){e._valueTracker||(e._valueTracker=GR(e))}function yy(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=gy(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function pu(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var YR=/[\n"\\]/g;function fa(e){return e.replace(YR,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function Nm(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+ca(t)):e.value!==""+ca(t)&&(e.value=""+ca(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?_m(e,i,ca(t)):a!=null?_m(e,i,ca(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+ca(o):e.removeAttribute("name")}function by(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+ca(a):"",t=t!=null?""+ca(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function _m(e,t,a){t==="number"&&pu(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function xs(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+ca(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function xy(e,t,a){if(t!=null&&(t=""+ca(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+ca(a):""}function $y(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(U(92));if(Ki(n)){if(1<n.length)throw Error(U(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=ca(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function Es(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var JR=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function zv(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||JR.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function wy(e,t,a){if(t!=null&&typeof t!="object")throw Error(U(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&zv(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&zv(e,s,t[s])}function gf(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var XR=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),ZR=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function Xl(e){return ZR.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var km=null;function yf(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var cs=null,$s=null;function qv(e){var t=Fs(e);if(t&&(e=t.stateNode)){var a=e[jt]||null;e:switch(e=t.stateNode,t.type){case"input":if(Nm(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+fa(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[jt]||null;if(!r)throw Error(U(90));Nm(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&yy(n)}break e;case"textarea":xy(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&xs(e,!!a.multiple,t,!1)}}}var Id=!1;function Sy(e,t,a){if(Id)return e(t,a);Id=!0;try{var n=e(t);return n}finally{if(Id=!1,(cs!==null||$s!==null)&&(Yu(),cs&&(t=cs,e=$s,$s=cs=null,qv(t),e)))for(t=0;t<e.length;t++)qv(e[t])}}function lo(e,t){var a=e.stateNode;if(a===null)return null;var n=a[jt]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(U(231,t,typeof a));return a}var gn=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),Rm=!1;if(gn)try{ts={},Object.defineProperty(ts,"passive",{get:function(){Rm=!0}}),window.addEventListener("test",ts,ts),window.removeEventListener("test",ts,ts)}catch{Rm=!1}var ts,In=null,bf=null,Zl=null;function Ny(){if(Zl)return Zl;var e,t=bf,a=t.length,n,r="value"in In?In.value:In.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return Zl=r.slice(e,1<n?1-n:void 0)}function Wl(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function Ul(){return!0}function Iv(){return!1}function Ft(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?Ul:Iv,this.isPropagationStopped=Iv,this}return Te(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=Ul)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=Ul)},persist:function(){},isPersistent:Ul}),t}var Ar={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Bu=Ft(Ar),Ro=Te({},Ar,{view:0,detail:0}),WR=Ft(Ro),Kd,Hd,Pi,zu=Te({},Ro,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:xf,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==Pi&&(Pi&&e.type==="mousemove"?(Kd=e.screenX-Pi.screenX,Hd=e.screenY-Pi.screenY):Hd=Kd=0,Pi=e),Kd)},movementY:function(e){return"movementY"in e?e.movementY:Hd}}),Kv=Ft(zu),eC=Te({},zu,{dataTransfer:0}),tC=Ft(eC),aC=Te({},Ro,{relatedTarget:0}),Qd=Ft(aC),nC=Te({},Ar,{animationName:0,elapsedTime:0,pseudoElement:0}),rC=Ft(nC),sC=Te({},Ar,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),iC=Ft(sC),oC=Te({},Ar,{data:0}),Hv=Ft(oC),lC={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},uC={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},cC={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function dC(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=cC[e])?!!t[e]:!1}function xf(){return dC}var mC=Te({},Ro,{key:function(e){if(e.key){var t=lC[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=Wl(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?uC[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:xf,charCode:function(e){return e.type==="keypress"?Wl(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?Wl(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),fC=Ft(mC),pC=Te({},zu,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),Qv=Ft(pC),hC=Te({},Ro,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:xf}),vC=Ft(hC),gC=Te({},Ar,{propertyName:0,elapsedTime:0,pseudoElement:0}),yC=Ft(gC),bC=Te({},zu,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),xC=Ft(bC),$C=Te({},Ar,{newState:0,oldState:0}),wC=Ft($C),SC=[9,13,27,32],$f=gn&&"CompositionEvent"in window,Vi=null;gn&&"documentMode"in document&&(Vi=document.documentMode);var NC=gn&&"TextEvent"in window&&!Vi,_y=gn&&(!$f||Vi&&8<Vi&&11>=Vi),Vv=" ",Gv=!1;function ky(e,t){switch(e){case"keyup":return SC.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function Ry(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var ds=!1;function _C(e,t){switch(e){case"compositionend":return Ry(t);case"keypress":return t.which!==32?null:(Gv=!0,Vv);case"textInput":return e=t.data,e===Vv&&Gv?null:e;default:return null}}function kC(e,t){if(ds)return e==="compositionend"||!$f&&ky(e,t)?(e=Ny(),Zl=bf=In=null,ds=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return _y&&t.locale!=="ko"?null:t.data;default:return null}}var RC={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function Yv(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!RC[e.type]:t==="textarea"}function Cy(e,t,a,n){cs?$s?$s.push(n):$s=[n]:cs=n,t=Du(t,"onChange"),0<t.length&&(a=new Bu("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var Gi=null,uo=null;function CC(e){x0(e,0)}function qu(e){var t=Hi(e);if(yy(t))return e}function Jv(e,t){if(e==="change")return t}var Ey=!1;gn&&(gn?(Fl="oninput"in document,Fl||(Vd=document.createElement("div"),Vd.setAttribute("oninput","return;"),Fl=typeof Vd.oninput=="function"),jl=Fl):jl=!1,Ey=jl&&(!document.documentMode||9<document.documentMode));var jl,Fl,Vd;function Xv(){Gi&&(Gi.detachEvent("onpropertychange",Ty),uo=Gi=null)}function Ty(e){if(e.propertyName==="value"&&qu(uo)){var t=[];Cy(t,uo,e,yf(e)),Sy(CC,t)}}function EC(e,t,a){e==="focusin"?(Xv(),Gi=t,uo=a,Gi.attachEvent("onpropertychange",Ty)):e==="focusout"&&Xv()}function TC(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return qu(uo)}function AC(e,t){if(e==="click")return qu(t)}function DC(e,t){if(e==="input"||e==="change")return qu(t)}function MC(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var Zt=typeof Object.is=="function"?Object.is:MC;function co(e,t){if(Zt(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!wm.call(t,r)||!Zt(e[r],t[r]))return!1}return!0}function Zv(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function Wv(e,t){var a=Zv(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=Zv(a)}}function Ay(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?Ay(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function Dy(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=pu(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=pu(e.document)}return t}function wf(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var OC=gn&&"documentMode"in document&&11>=document.documentMode,ms=null,Cm=null,Yi=null,Em=!1;function eg(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;Em||ms==null||ms!==pu(n)||(n=ms,"selectionStart"in n&&wf(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),Yi&&co(Yi,n)||(Yi=n,n=Du(Cm,"onSelect"),0<n.length&&(t=new Bu("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=ms)))}function hr(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var fs={animationend:hr("Animation","AnimationEnd"),animationiteration:hr("Animation","AnimationIteration"),animationstart:hr("Animation","AnimationStart"),transitionrun:hr("Transition","TransitionRun"),transitionstart:hr("Transition","TransitionStart"),transitioncancel:hr("Transition","TransitionCancel"),transitionend:hr("Transition","TransitionEnd")},Gd={},My={};gn&&(My=document.createElement("div").style,"AnimationEvent"in window||(delete fs.animationend.animation,delete fs.animationiteration.animation,delete fs.animationstart.animation),"TransitionEvent"in window||delete fs.transitionend.transition);function Dr(e){if(Gd[e])return Gd[e];if(!fs[e])return e;var t=fs[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in My)return Gd[e]=t[a];return e}var Oy=Dr("animationend"),Ly=Dr("animationiteration"),Py=Dr("animationstart"),LC=Dr("transitionrun"),PC=Dr("transitionstart"),UC=Dr("transitioncancel"),Uy=Dr("transitionend"),jy=new Map,Tm="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");Tm.push("scrollEnd");function ka(e,t){jy.set(e,t),Tr(t,[e])}var tg=new WeakMap;function pa(e,t){if(typeof e=="object"&&e!==null){var a=tg.get(e);return a!==void 0?a:(t={value:e,source:t,stack:Bv(t)},tg.set(e,t),t)}return{value:e,source:t,stack:Bv(t)}}var ua=[],ps=0,Sf=0;function Iu(){for(var e=ps,t=Sf=ps=0;t<e;){var a=ua[t];ua[t++]=null;var n=ua[t];ua[t++]=null;var r=ua[t];ua[t++]=null;var s=ua[t];if(ua[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&Fy(a,r,s)}}function Ku(e,t,a,n){ua[ps++]=e,ua[ps++]=t,ua[ps++]=a,ua[ps++]=n,Sf|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function Nf(e,t,a,n){return Ku(e,t,a,n),hu(e)}function Bs(e,t){return Ku(e,null,null,t),hu(e)}function Fy(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Yt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function hu(e){if(50<so)throw so=0,Xm=null,Error(U(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var hs={};function jC(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Vt(e,t,a,n){return new jC(e,t,a,n)}function _f(e){return e=e.prototype,!(!e||!e.isReactComponent)}function hn(e,t){var a=e.alternate;return a===null?(a=Vt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function By(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function eu(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")_f(e)&&(i=1);else if(typeof e=="string")i=jE(e,a,Ia.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case ym:return e=Vt(31,a,t,r),e.elementType=ym,e.lanes=s,e;case os:return xr(a.children,r,s,t);case sy:i=8,r|=24;break;case hm:return e=Vt(12,a,t,r|2),e.elementType=hm,e.lanes=s,e;case vm:return e=Vt(13,a,t,r),e.elementType=vm,e.lanes=s,e;case gm:return e=Vt(19,a,t,r),e.elementType=gm,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case CR:case cn:i=10;break e;case iy:i=9;break e;case df:i=11;break e;case mf:i=14;break e;case Pn:i=16,n=null;break e}i=29,a=Error(U(130,e===null?"null":typeof e,"")),n=null}return t=Vt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function xr(e,t,a,n){return e=Vt(7,e,n,t),e.lanes=a,e}function Yd(e,t,a){return e=Vt(6,e,null,t),e.lanes=a,e}function Jd(e,t,a){return t=Vt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var vs=[],gs=0,vu=null,gu=0,da=[],ma=0,$r=null,dn=1,mn="";function gr(e,t){vs[gs++]=gu,vs[gs++]=vu,vu=e,gu=t}function zy(e,t,a){da[ma++]=dn,da[ma++]=mn,da[ma++]=$r,$r=e;var n=dn;e=mn;var r=32-Yt(n)-1;n&=~(1<<r),a+=1;var s=32-Yt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,dn=1<<32-Yt(t)+r|a<<r|n,mn=s+e}else dn=1<<s|a<<r|n,mn=e}function kf(e){e.return!==null&&(gr(e,1),zy(e,1,0))}function Rf(e){for(;e===vu;)vu=vs[--gs],vs[gs]=null,gu=vs[--gs],vs[gs]=null;for(;e===$r;)$r=da[--ma],da[ma]=null,mn=da[--ma],da[ma]=null,dn=da[--ma],da[ma]=null}var Ct=null,qe=null,he=!1,wr=null,za=!1,Am=Error(U(519));function kr(e){var t=Error(U(418,""));throw mo(pa(t,e)),Am}function ag(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[wt]=e,t[jt]=n,a){case"dialog":le("cancel",t),le("close",t);break;case"iframe":case"object":case"embed":le("load",t);break;case"video":case"audio":for(a=0;a<ho.length;a++)le(ho[a],t);break;case"source":le("error",t);break;case"img":case"image":case"link":le("error",t),le("load",t);break;case"details":le("toggle",t);break;case"input":le("invalid",t),by(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),fu(t);break;case"select":le("invalid",t);break;case"textarea":le("invalid",t),$y(t,n.value,n.defaultValue,n.children),fu(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||w0(t.textContent,a)?(n.popover!=null&&(le("beforetoggle",t),le("toggle",t)),n.onScroll!=null&&le("scroll",t),n.onScrollEnd!=null&&le("scrollend",t),n.onClick!=null&&(t.onclick=Zu),t=!0):t=!1,t||kr(e)}function ng(e){for(Ct=e.return;Ct;)switch(Ct.tag){case 5:case 13:za=!1;return;case 27:case 3:za=!0;return;default:Ct=Ct.return}}function Ui(e){if(e!==Ct)return!1;if(!he)return ng(e),he=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||nf(e.type,e.memoizedProps)),a=!a),a&&qe&&kr(e),ng(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(U(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){qe=_a(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}qe=null}}else t===27?(t=qe,rr(e.type)?(e=of,of=null,qe=e):qe=t):qe=Ct?_a(e.stateNode.nextSibling):null;return!0}function Co(){qe=Ct=null,he=!1}function rg(){var e=wr;return e!==null&&(Ut===null?Ut=e:Ut.push.apply(Ut,e),wr=null),e}function mo(e){wr===null?wr=[e]:wr.push(e)}var Dm=Qa(null),Mr=null,fn=null;function jn(e,t,a){Oe(Dm,t._currentValue),t._currentValue=a}function vn(e){e._currentValue=Dm.current,mt(Dm)}function Mm(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function Om(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var u=0;u<t.length;u++)if(o.context===t[u]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),Mm(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(U(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),Mm(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function Eo(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(U(387));if(i=i.memoizedProps,i!==null){var o=r.type;Zt(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===uu.current){if(i=r.alternate,i===null)throw Error(U(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(yo):e=[yo])}r=r.return}e!==null&&Om(t,e,a,n),t.flags|=262144}function yu(e){for(e=e.firstContext;e!==null;){if(!Zt(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function Rr(e){Mr=e,fn=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function St(e){return qy(Mr,e)}function Bl(e,t){return Mr===null&&Rr(e),qy(e,t)}function qy(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},fn===null){if(e===null)throw Error(U(308));fn=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else fn=fn.next=t;return a}var FC=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},BC=st.unstable_scheduleCallback,zC=st.unstable_NormalPriority,nt={$$typeof:cn,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function Cf(){return{controller:new FC,data:new Map,refCount:0}}function To(e){e.refCount--,e.refCount===0&&BC(zC,function(){e.controller.abort()})}var Ji=null,Lm=0,Ts=0,ws=null;function qC(e,t){if(Ji===null){var a=Ji=[];Lm=0,Ts=Xf(),ws={status:"pending",value:void 0,then:function(n){a.push(n)}}}return Lm++,t.then(sg,sg),t}function sg(){if(--Lm===0&&Ji!==null){ws!==null&&(ws.status="fulfilled");var e=Ji;Ji=null,Ts=0,ws=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function IC(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var ig=te.S;te.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&qC(e,t),ig!==null&&ig(e,t)};var Sr=Qa(null);function Ef(){var e=Sr.current;return e!==null?e:ke.pooledCache}function tu(e,t){t===null?Oe(Sr,Sr.current):Oe(Sr,t.pool)}function Iy(){var e=Ef();return e===null?null:{parent:nt._currentValue,pool:e}}var Ao=Error(U(460)),Ky=Error(U(474)),Hu=Error(U(542)),Pm={then:function(){}};function og(e){return e=e.status,e==="fulfilled"||e==="rejected"}function zl(){}function Hy(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(zl,zl),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,ug(e),e;default:if(typeof t.status=="string")t.then(zl,zl);else{if(e=ke,e!==null&&100<e.shellSuspendCounter)throw Error(U(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,ug(e),e}throw Xi=t,Ao}}var Xi=null;function lg(){if(Xi===null)throw Error(U(459));var e=Xi;return Xi=null,e}function ug(e){if(e===Ao||e===Hu)throw Error(U(483))}var Un=!1;function Tf(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function Um(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function Vn(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function Gn(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,(we&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=hu(e),Fy(e,null,a),t}return Ku(e,n,t,a),hu(e)}function Zi(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,fy(e,a)}}function Xd(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var jm=!1;function Wi(){if(jm){var e=ws;if(e!==null)throw e}}function eo(e,t,a,n){jm=!1;var r=e.updateQueue;Un=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var u=o,c=u.next;u.next=null,i===null?s=c:i.next=c,i=u;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=u))}if(s!==null){var m=r.baseState;i=0,d=c=u=null,o=s;do{var f=o.lane&-536870913,p=f!==o.lane;if(p?(fe&f)===f:(n&f)===f){f!==0&&f===Ts&&(jm=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var x=e,y=o;f=t;var w=a;switch(y.tag){case 1:if(x=y.payload,typeof x=="function"){m=x.call(w,m,f);break e}m=x;break e;case 3:x.flags=x.flags&-65537|128;case 0:if(x=y.payload,f=typeof x=="function"?x.call(w,m,f):x,f==null)break e;m=Te({},m,f);break e;case 2:Un=!0}}f=o.callback,f!==null&&(e.flags|=64,p&&(e.flags|=8192),p=r.callbacks,p===null?r.callbacks=[f]:p.push(f))}else p={lane:f,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=p,u=m):d=d.next=p,i|=f;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;p=o,o=p.next,p.next=null,r.lastBaseUpdate=p,r.shared.pending=null}}while(!0);d===null&&(u=m),r.baseState=u,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),ar|=i,e.lanes=i,e.memoizedState=m}}function Qy(e,t){if(typeof e!="function")throw Error(U(191,e));e.call(t)}function Vy(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)Qy(a[e],t)}var As=Qa(null),bu=Qa(0);function cg(e,t){e=xn,Oe(bu,e),Oe(As,t),xn=e|t.baseLanes}function Fm(){Oe(bu,xn),Oe(As,As.current)}function Af(){xn=bu.current,mt(As),mt(bu)}var er=0,ie=null,Ne=null,Xe=null,xu=!1,Ss=!1,Cr=!1,$u=0,fo=0,Ns=null,KC=0;function Ve(){throw Error(U(321))}function Df(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!Zt(e[a],t[a]))return!1;return!0}function Mf(e,t,a,n,r,s){return er=s,ie=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,te.H=e===null||e.memoizedState===null?Nb:_b,Cr=!1,s=a(n,r),Cr=!1,Ss&&(s=Yy(t,a,n,r)),Gy(e),s}function Gy(e){te.H=wu;var t=Ne!==null&&Ne.next!==null;if(er=0,Xe=Ne=ie=null,xu=!1,fo=0,Ns=null,t)throw Error(U(300));e===null||dt||(e=e.dependencies,e!==null&&yu(e)&&(dt=!0))}function Yy(e,t,a,n){ie=e;var r=0;do{if(Ss&&(Ns=null),fo=0,Ss=!1,25<=r)throw Error(U(301));if(r+=1,Xe=Ne=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}te.H=XC,s=t(a,n)}while(Ss);return s}function HC(){var e=te.H,t=e.useState()[0];return t=typeof t.then=="function"?Do(t):t,e=e.useState()[0],(Ne!==null?Ne.memoizedState:null)!==e&&(ie.flags|=1024),t}function Of(){var e=$u!==0;return $u=0,e}function Lf(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function Pf(e){if(xu){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}xu=!1}er=0,Xe=Ne=ie=null,Ss=!1,fo=$u=0,Ns=null}function Lt(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return Xe===null?ie.memoizedState=Xe=e:Xe=Xe.next=e,Xe}function Ze(){if(Ne===null){var e=ie.alternate;e=e!==null?e.memoizedState:null}else e=Ne.next;var t=Xe===null?ie.memoizedState:Xe.next;if(t!==null)Xe=t,Ne=e;else{if(e===null)throw ie.alternate===null?Error(U(467)):Error(U(310));Ne=e,e={memoizedState:Ne.memoizedState,baseState:Ne.baseState,baseQueue:Ne.baseQueue,queue:Ne.queue,next:null},Xe===null?ie.memoizedState=Xe=e:Xe=Xe.next=e}return Xe}function Uf(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function Do(e){var t=fo;return fo+=1,Ns===null&&(Ns=[]),e=Hy(Ns,e,t),t=ie,(Xe===null?t.memoizedState:Xe.next)===null&&(t=t.alternate,te.H=t===null||t.memoizedState===null?Nb:_b),e}function Qu(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return Do(e);if(e.$$typeof===cn)return St(e)}throw Error(U(438,String(e)))}function jf(e){var t=null,a=ie.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=ie.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=Uf(),ie.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=ER;return t.index++,a}function yn(e,t){return typeof t=="function"?t(e):t}function au(e){var t=Ze();return Ff(t,Ne,e)}function Ff(e,t,a){var n=e.queue;if(n===null)throw Error(U(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,u=null,c=t,d=!1;do{var m=c.lane&-536870913;if(m!==c.lane?(fe&m)===m:(er&m)===m){var f=c.revertLane;if(f===0)u!==null&&(u=u.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),m===Ts&&(d=!0);else if((er&f)===f){c=c.next,f===Ts&&(d=!0);continue}else m={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=m,i=s):u=u.next=m,ie.lanes|=f,ar|=f;m=c.action,Cr&&a(s,m),s=c.hasEagerState?c.eagerState:a(s,m)}else f={lane:m,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=f,i=s):u=u.next=f,ie.lanes|=m,ar|=m;c=c.next}while(c!==null&&c!==t);if(u===null?i=s:u.next=o,!Zt(s,e.memoizedState)&&(dt=!0,d&&(a=ws,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=u,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function Zd(e){var t=Ze(),a=t.queue;if(a===null)throw Error(U(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);Zt(s,t.memoizedState)||(dt=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function Jy(e,t,a){var n=ie,r=Ze(),s=he;if(s){if(a===void 0)throw Error(U(407));a=a()}else a=t();var i=!Zt((Ne||r).memoizedState,a);i&&(r.memoizedState=a,dt=!0),r=r.queue;var o=Wy.bind(null,n,r,e);if(Mo(2048,8,o,[e]),r.getSnapshot!==t||i||Xe!==null&&Xe.memoizedState.tag&1){if(n.flags|=2048,Ds(9,Vu(),Zy.bind(null,n,r,a,t),null),ke===null)throw Error(U(349));s||(er&124)!==0||Xy(n,t,a)}return a}function Xy(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=ie.updateQueue,t===null?(t=Uf(),ie.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function Zy(e,t,a,n){t.value=a,t.getSnapshot=n,eb(t)&&tb(e)}function Wy(e,t,a){return a(function(){eb(t)&&tb(e)})}function eb(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!Zt(e,a)}catch{return!0}}function tb(e){var t=Bs(e,2);t!==null&&Xt(t,e,2)}function Bm(e){var t=Lt();if(typeof e=="function"){var a=e;if(e=a(),Cr){qn(!0);try{a()}finally{qn(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:yn,lastRenderedState:e},t}function ab(e,t,a,n){return e.baseState=a,Ff(e,Ne,typeof n=="function"?n:yn)}function QC(e,t,a,n,r){if(Gu(e))throw Error(U(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};te.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,nb(t,s)):(s.next=a.next,t.pending=a.next=s)}}function nb(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=te.T,i={};te.T=i;try{var o=a(r,n),u=te.S;u!==null&&u(i,o),dg(e,t,o)}catch(c){zm(e,t,c)}finally{te.T=s}}else try{s=a(r,n),dg(e,t,s)}catch(c){zm(e,t,c)}}function dg(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){mg(e,t,n)},function(n){return zm(e,t,n)}):mg(e,t,a)}function mg(e,t,a){t.status="fulfilled",t.value=a,rb(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,nb(e,a)))}function zm(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,rb(t),t=t.next;while(t!==n)}e.action=null}function rb(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function sb(e,t){return t}function fg(e,t){if(he){var a=ke.formState;if(a!==null){e:{var n=ie;if(he){if(qe){t:{for(var r=qe,s=za;r.nodeType!==8;){if(!s){r=null;break t}if(r=_a(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){qe=_a(r.nextSibling),n=r.data==="F!";break e}}kr(n)}n=!1}n&&(t=a[0])}}return a=Lt(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:sb,lastRenderedState:t},a.queue=n,a=$b.bind(null,ie,n),n.dispatch=a,n=Bm(!1),s=If.bind(null,ie,!1,n.queue),n=Lt(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=QC.bind(null,ie,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function pg(e){var t=Ze();return ib(t,Ne,e)}function ib(e,t,a){if(t=Ff(e,t,sb)[0],e=au(yn)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=Do(t)}catch(i){throw i===Ao?Hu:i}else n=t;t=Ze();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(ie.flags|=2048,Ds(9,Vu(),VC.bind(null,r,a),null)),[n,s,e]}function VC(e,t){e.action=t}function hg(e){var t=Ze(),a=Ne;if(a!==null)return ib(t,a,e);Ze(),t=t.memoizedState,a=Ze();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function Ds(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=ie.updateQueue,t===null&&(t=Uf(),ie.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function Vu(){return{destroy:void 0,resource:void 0}}function ob(){return Ze().memoizedState}function nu(e,t,a,n){var r=Lt();n=n===void 0?null:n,ie.flags|=e,r.memoizedState=Ds(1|t,Vu(),a,n)}function Mo(e,t,a,n){var r=Ze();n=n===void 0?null:n;var s=r.memoizedState.inst;Ne!==null&&n!==null&&Df(n,Ne.memoizedState.deps)?r.memoizedState=Ds(t,s,a,n):(ie.flags|=e,r.memoizedState=Ds(1|t,s,a,n))}function vg(e,t){nu(8390656,8,e,t)}function lb(e,t){Mo(2048,8,e,t)}function ub(e,t){return Mo(4,2,e,t)}function cb(e,t){return Mo(4,4,e,t)}function db(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function mb(e,t,a){a=a!=null?a.concat([e]):null,Mo(4,4,db.bind(null,t,e),a)}function Bf(){}function fb(e,t){var a=Ze();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&Df(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function pb(e,t){var a=Ze();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&Df(t,n[1]))return n[0];if(n=e(),Cr){qn(!0);try{e()}finally{qn(!1)}}return a.memoizedState=[n,t],n}function zf(e,t,a){return a===void 0||(er&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=r0(),ie.lanes|=e,ar|=e,a)}function hb(e,t,a,n){return Zt(a,t)?a:As.current!==null?(e=zf(e,a,n),Zt(e,t)||(dt=!0),e):(er&42)===0?(dt=!0,e.memoizedState=a):(e=r0(),ie.lanes|=e,ar|=e,t)}function vb(e,t,a,n,r){var s=ve.p;ve.p=s!==0&&8>s?s:8;var i=te.T,o={};te.T=o,If(e,!1,t,a);try{var u=r(),c=te.S;if(c!==null&&c(o,u),u!==null&&typeof u=="object"&&typeof u.then=="function"){var d=IC(u,n);to(e,t,d,Jt(e))}else to(e,t,n,Jt(e))}catch(m){to(e,t,{then:function(){},status:"rejected",reason:m},Jt())}finally{ve.p=s,te.T=i}}function GC(){}function qm(e,t,a,n){if(e.tag!==5)throw Error(U(476));var r=gb(e).queue;vb(e,r,t,br,a===null?GC:function(){return yb(e),a(n)})}function gb(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:br,baseState:br,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:yn,lastRenderedState:br},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:yn,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function yb(e){var t=gb(e).next.queue;to(e,t,{},Jt())}function qf(){return St(yo)}function bb(){return Ze().memoizedState}function xb(){return Ze().memoizedState}function YC(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=Jt();e=Vn(a);var n=Gn(t,e,a);n!==null&&(Xt(n,t,a),Zi(n,t,a)),t={cache:Cf()},e.payload=t;return}t=t.return}}function JC(e,t,a){var n=Jt();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},Gu(e)?wb(t,a):(a=Nf(e,t,a,n),a!==null&&(Xt(a,e,n),Sb(a,t,n)))}function $b(e,t,a){var n=Jt();to(e,t,a,n)}function to(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(Gu(e))wb(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,Zt(o,i))return Ku(e,t,r,0),ke===null&&Iu(),!1}catch{}finally{}if(a=Nf(e,t,r,n),a!==null)return Xt(a,e,n),Sb(a,t,n),!0}return!1}function If(e,t,a,n){if(n={lane:2,revertLane:Xf(),action:n,hasEagerState:!1,eagerState:null,next:null},Gu(e)){if(t)throw Error(U(479))}else t=Nf(e,a,n,2),t!==null&&Xt(t,e,2)}function Gu(e){var t=e.alternate;return e===ie||t!==null&&t===ie}function wb(e,t){Ss=xu=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function Sb(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,fy(e,a)}}var wu={readContext:St,use:Qu,useCallback:Ve,useContext:Ve,useEffect:Ve,useImperativeHandle:Ve,useLayoutEffect:Ve,useInsertionEffect:Ve,useMemo:Ve,useReducer:Ve,useRef:Ve,useState:Ve,useDebugValue:Ve,useDeferredValue:Ve,useTransition:Ve,useSyncExternalStore:Ve,useId:Ve,useHostTransitionStatus:Ve,useFormState:Ve,useActionState:Ve,useOptimistic:Ve,useMemoCache:Ve,useCacheRefresh:Ve},Nb={readContext:St,use:Qu,useCallback:function(e,t){return Lt().memoizedState=[e,t===void 0?null:t],e},useContext:St,useEffect:vg,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,nu(4194308,4,db.bind(null,t,e),a)},useLayoutEffect:function(e,t){return nu(4194308,4,e,t)},useInsertionEffect:function(e,t){nu(4,2,e,t)},useMemo:function(e,t){var a=Lt();t=t===void 0?null:t;var n=e();if(Cr){qn(!0);try{e()}finally{qn(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=Lt();if(a!==void 0){var r=a(t);if(Cr){qn(!0);try{a(t)}finally{qn(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=JC.bind(null,ie,e),[n.memoizedState,e]},useRef:function(e){var t=Lt();return e={current:e},t.memoizedState=e},useState:function(e){e=Bm(e);var t=e.queue,a=$b.bind(null,ie,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:Bf,useDeferredValue:function(e,t){var a=Lt();return zf(a,e,t)},useTransition:function(){var e=Bm(!1);return e=vb.bind(null,ie,e.queue,!0,!1),Lt().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=ie,r=Lt();if(he){if(a===void 0)throw Error(U(407));a=a()}else{if(a=t(),ke===null)throw Error(U(349));(fe&124)!==0||Xy(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,vg(Wy.bind(null,n,s,e),[e]),n.flags|=2048,Ds(9,Vu(),Zy.bind(null,n,s,a,t),null),a},useId:function(){var e=Lt(),t=ke.identifierPrefix;if(he){var a=mn,n=dn;a=(n&~(1<<32-Yt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=$u++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=KC++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:qf,useFormState:fg,useActionState:fg,useOptimistic:function(e){var t=Lt();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=If.bind(null,ie,!0,a),a.dispatch=t,[e,t]},useMemoCache:jf,useCacheRefresh:function(){return Lt().memoizedState=YC.bind(null,ie)}},_b={readContext:St,use:Qu,useCallback:fb,useContext:St,useEffect:lb,useImperativeHandle:mb,useInsertionEffect:ub,useLayoutEffect:cb,useMemo:pb,useReducer:au,useRef:ob,useState:function(){return au(yn)},useDebugValue:Bf,useDeferredValue:function(e,t){var a=Ze();return hb(a,Ne.memoizedState,e,t)},useTransition:function(){var e=au(yn)[0],t=Ze().memoizedState;return[typeof e=="boolean"?e:Do(e),t]},useSyncExternalStore:Jy,useId:bb,useHostTransitionStatus:qf,useFormState:pg,useActionState:pg,useOptimistic:function(e,t){var a=Ze();return ab(a,Ne,e,t)},useMemoCache:jf,useCacheRefresh:xb},XC={readContext:St,use:Qu,useCallback:fb,useContext:St,useEffect:lb,useImperativeHandle:mb,useInsertionEffect:ub,useLayoutEffect:cb,useMemo:pb,useReducer:Zd,useRef:ob,useState:function(){return Zd(yn)},useDebugValue:Bf,useDeferredValue:function(e,t){var a=Ze();return Ne===null?zf(a,e,t):hb(a,Ne.memoizedState,e,t)},useTransition:function(){var e=Zd(yn)[0],t=Ze().memoizedState;return[typeof e=="boolean"?e:Do(e),t]},useSyncExternalStore:Jy,useId:bb,useHostTransitionStatus:qf,useFormState:hg,useActionState:hg,useOptimistic:function(e,t){var a=Ze();return Ne!==null?ab(a,Ne,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:jf,useCacheRefresh:xb},_s=null,po=0;function ql(e){var t=po;return po+=1,_s===null&&(_s=[]),Hy(_s,e,t)}function ji(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function Il(e,t){throw t.$$typeof===RR?Error(U(525)):(e=Object.prototype.toString.call(t),Error(U(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function gg(e){var t=e._init;return t(e._payload)}function kb(e){function t(g,v){if(e){var b=g.deletions;b===null?(g.deletions=[v],g.flags|=16):b.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=hn(g,v),g.index=0,g.sibling=null,g}function s(g,v,b){return g.index=b,e?(b=g.alternate,b!==null?(b=b.index,b<v?(g.flags|=67108866,v):b):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,b,$){return v===null||v.tag!==6?(v=Yd(b,g.mode,$),v.return=g,v):(v=r(v,b),v.return=g,v)}function u(g,v,b,$){var S=b.type;return S===os?d(g,v,b.props.children,$,b.key):v!==null&&(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Pn&&gg(S)===v.type)?(v=r(v,b.props),ji(v,b),v.return=g,v):(v=eu(b.type,b.key,b.props,null,g.mode,$),ji(v,b),v.return=g,v)}function c(g,v,b,$){return v===null||v.tag!==4||v.stateNode.containerInfo!==b.containerInfo||v.stateNode.implementation!==b.implementation?(v=Jd(b,g.mode,$),v.return=g,v):(v=r(v,b.children||[]),v.return=g,v)}function d(g,v,b,$,S){return v===null||v.tag!==7?(v=xr(b,g.mode,$,S),v.return=g,v):(v=r(v,b),v.return=g,v)}function m(g,v,b){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=Yd(""+v,g.mode,b),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case Ml:return b=eu(v.type,v.key,v.props,null,g.mode,b),ji(b,v),b.return=g,b;case Ii:return v=Jd(v,g.mode,b),v.return=g,v;case Pn:var $=v._init;return v=$(v._payload),m(g,v,b)}if(Ki(v)||Li(v))return v=xr(v,g.mode,b,null),v.return=g,v;if(typeof v.then=="function")return m(g,ql(v),b);if(v.$$typeof===cn)return m(g,Bl(g,v),b);Il(g,v)}return null}function f(g,v,b,$){var S=v!==null?v.key:null;if(typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint")return S!==null?null:o(g,v,""+b,$);if(typeof b=="object"&&b!==null){switch(b.$$typeof){case Ml:return b.key===S?u(g,v,b,$):null;case Ii:return b.key===S?c(g,v,b,$):null;case Pn:return S=b._init,b=S(b._payload),f(g,v,b,$)}if(Ki(b)||Li(b))return S!==null?null:d(g,v,b,$,null);if(typeof b.then=="function")return f(g,v,ql(b),$);if(b.$$typeof===cn)return f(g,v,Bl(g,b),$);Il(g,b)}return null}function p(g,v,b,$,S){if(typeof $=="string"&&$!==""||typeof $=="number"||typeof $=="bigint")return g=g.get(b)||null,o(v,g,""+$,S);if(typeof $=="object"&&$!==null){switch($.$$typeof){case Ml:return g=g.get($.key===null?b:$.key)||null,u(v,g,$,S);case Ii:return g=g.get($.key===null?b:$.key)||null,c(v,g,$,S);case Pn:var E=$._init;return $=E($._payload),p(g,v,b,$,S)}if(Ki($)||Li($))return g=g.get(b)||null,d(v,g,$,S,null);if(typeof $.then=="function")return p(g,v,b,ql($),S);if($.$$typeof===cn)return p(g,v,b,Bl(v,$),S);Il(v,$)}return null}function x(g,v,b,$){for(var S=null,E=null,_=v,T=v=0,L=null;_!==null&&T<b.length;T++){_.index>T?(L=_,_=null):L=_.sibling;var D=f(g,_,b[T],$);if(D===null){_===null&&(_=L);break}e&&_&&D.alternate===null&&t(g,_),v=s(D,v,T),E===null?S=D:E.sibling=D,E=D,_=L}if(T===b.length)return a(g,_),he&&gr(g,T),S;if(_===null){for(;T<b.length;T++)_=m(g,b[T],$),_!==null&&(v=s(_,v,T),E===null?S=_:E.sibling=_,E=_);return he&&gr(g,T),S}for(_=n(_);T<b.length;T++)L=p(_,g,T,b[T],$),L!==null&&(e&&L.alternate!==null&&_.delete(L.key===null?T:L.key),v=s(L,v,T),E===null?S=L:E.sibling=L,E=L);return e&&_.forEach(function(P){return t(g,P)}),he&&gr(g,T),S}function y(g,v,b,$){if(b==null)throw Error(U(151));for(var S=null,E=null,_=v,T=v=0,L=null,D=b.next();_!==null&&!D.done;T++,D=b.next()){_.index>T?(L=_,_=null):L=_.sibling;var P=f(g,_,D.value,$);if(P===null){_===null&&(_=L);break}e&&_&&P.alternate===null&&t(g,_),v=s(P,v,T),E===null?S=P:E.sibling=P,E=P,_=L}if(D.done)return a(g,_),he&&gr(g,T),S;if(_===null){for(;!D.done;T++,D=b.next())D=m(g,D.value,$),D!==null&&(v=s(D,v,T),E===null?S=D:E.sibling=D,E=D);return he&&gr(g,T),S}for(_=n(_);!D.done;T++,D=b.next())D=p(_,g,T,D.value,$),D!==null&&(e&&D.alternate!==null&&_.delete(D.key===null?T:D.key),v=s(D,v,T),E===null?S=D:E.sibling=D,E=D);return e&&_.forEach(function(R){return t(g,R)}),he&&gr(g,T),S}function w(g,v,b,$){if(typeof b=="object"&&b!==null&&b.type===os&&b.key===null&&(b=b.props.children),typeof b=="object"&&b!==null){switch(b.$$typeof){case Ml:e:{for(var S=b.key;v!==null;){if(v.key===S){if(S=b.type,S===os){if(v.tag===7){a(g,v.sibling),$=r(v,b.props.children),$.return=g,g=$;break e}}else if(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Pn&&gg(S)===v.type){a(g,v.sibling),$=r(v,b.props),ji($,b),$.return=g,g=$;break e}a(g,v);break}else t(g,v);v=v.sibling}b.type===os?($=xr(b.props.children,g.mode,$,b.key),$.return=g,g=$):($=eu(b.type,b.key,b.props,null,g.mode,$),ji($,b),$.return=g,g=$)}return i(g);case Ii:e:{for(S=b.key;v!==null;){if(v.key===S)if(v.tag===4&&v.stateNode.containerInfo===b.containerInfo&&v.stateNode.implementation===b.implementation){a(g,v.sibling),$=r(v,b.children||[]),$.return=g,g=$;break e}else{a(g,v);break}else t(g,v);v=v.sibling}$=Jd(b,g.mode,$),$.return=g,g=$}return i(g);case Pn:return S=b._init,b=S(b._payload),w(g,v,b,$)}if(Ki(b))return x(g,v,b,$);if(Li(b)){if(S=Li(b),typeof S!="function")throw Error(U(150));return b=S.call(b),y(g,v,b,$)}if(typeof b.then=="function")return w(g,v,ql(b),$);if(b.$$typeof===cn)return w(g,v,Bl(g,b),$);Il(g,b)}return typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint"?(b=""+b,v!==null&&v.tag===6?(a(g,v.sibling),$=r(v,b),$.return=g,g=$):(a(g,v),$=Yd(b,g.mode,$),$.return=g,g=$),i(g)):a(g,v)}return function(g,v,b,$){try{po=0;var S=w(g,v,b,$);return _s=null,S}catch(_){if(_===Ao||_===Hu)throw _;var E=Vt(29,_,null,g.mode);return E.lanes=$,E.return=g,E}finally{}}}var Ms=kb(!0),Rb=kb(!1),va=Qa(null),Ha=null;function Fn(e){var t=e.alternate;Oe(rt,rt.current&1),Oe(va,e),Ha===null&&(t===null||As.current!==null||t.memoizedState!==null)&&(Ha=e)}function Cb(e){if(e.tag===22){if(Oe(rt,rt.current),Oe(va,e),Ha===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(Ha=e)}}else Bn(e)}function Bn(){Oe(rt,rt.current),Oe(va,va.current)}function pn(e){mt(va),Ha===e&&(Ha=null),mt(rt)}var rt=Qa(0);function Su(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||sf(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function Wd(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:Te({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var Im={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=Jt(),r=Vn(n);r.payload=t,a!=null&&(r.callback=a),t=Gn(e,r,n),t!==null&&(Xt(t,e,n),Zi(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=Jt(),r=Vn(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=Gn(e,r,n),t!==null&&(Xt(t,e,n),Zi(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=Jt(),n=Vn(a);n.tag=2,t!=null&&(n.callback=t),t=Gn(e,n,a),t!==null&&(Xt(t,e,a),Zi(t,e,a))}};function yg(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!co(a,n)||!co(r,s):!0}function bg(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&Im.enqueueReplaceState(t,t.state,null)}function Er(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=Te({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var Nu=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function Eb(e){Nu(e)}function Tb(e){console.error(e)}function Ab(e){Nu(e)}function _u(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function xg(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function Km(e,t,a){return a=Vn(a),a.tag=3,a.payload={element:null},a.callback=function(){_u(e,t)},a}function Db(e){return e=Vn(e),e.tag=3,e}function Mb(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){xg(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){xg(t,a,n),typeof r!="function"&&(Yn===null?Yn=new Set([this]):Yn.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function ZC(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&Eo(t,a,r,!0),a=va.current,a!==null){switch(a.tag){case 13:return Ha===null?Zm():a.alternate===null&&Ie===0&&(Ie=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===Pm?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),cm(e,n,r)),!1;case 22:return a.flags|=65536,n===Pm?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),cm(e,n,r)),!1}throw Error(U(435,a.tag))}return cm(e,n,r),Zm(),!1}if(he)return t=va.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==Am&&(e=Error(U(422),{cause:n}),mo(pa(e,a)))):(n!==Am&&(t=Error(U(423),{cause:n}),mo(pa(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=pa(n,a),r=Km(e.stateNode,n,r),Xd(e,r),Ie!==4&&(Ie=2)),!1;var s=Error(U(520),{cause:n});if(s=pa(s,a),ro===null?ro=[s]:ro.push(s),Ie!==4&&(Ie=2),t===null)return!0;n=pa(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=Km(a.stateNode,n,e),Xd(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(Yn===null||!Yn.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=Db(r),Mb(r,e,a,n),Xd(a,r),!1}a=a.return}while(a!==null);return!1}var Ob=Error(U(461)),dt=!1;function ht(e,t,a,n){t.child=e===null?Rb(t,null,a,n):Ms(t,e.child,a,n)}function $g(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return Rr(t),n=Mf(e,t,a,i,s,r),o=Of(),e!==null&&!dt?(Lf(e,t,r),bn(e,t,r)):(he&&o&&kf(t),t.flags|=1,ht(e,t,n,r),t.child)}function wg(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!_f(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,Lb(e,t,s,n,r)):(e=eu(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!Kf(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:co,a(i,n)&&e.ref===t.ref)return bn(e,t,r)}return t.flags|=1,e=hn(s,n),e.ref=t.ref,e.return=t,t.child=e}function Lb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if(co(s,n)&&e.ref===t.ref)if(dt=!1,t.pendingProps=n=s,Kf(e,r))(e.flags&131072)!==0&&(dt=!0);else return t.lanes=e.lanes,bn(e,t,r)}return Hm(e,t,a,n,r)}function Pb(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return Sg(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&tu(t,s!==null?s.cachePool:null),s!==null?cg(t,s):Fm(),Cb(t);else return t.lanes=t.childLanes=536870912,Sg(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(tu(t,s.cachePool),cg(t,s),Bn(t),t.memoizedState=null):(e!==null&&tu(t,null),Fm(),Bn(t));return ht(e,t,r,a),t.child}function Sg(e,t,a,n){var r=Ef();return r=r===null?null:{parent:nt._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&tu(t,null),Fm(),Cb(t),e!==null&&Eo(e,t,n,!0),null}function ru(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(U(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function Hm(e,t,a,n,r){return Rr(t),a=Mf(e,t,a,n,void 0,r),n=Of(),e!==null&&!dt?(Lf(e,t,r),bn(e,t,r)):(he&&n&&kf(t),t.flags|=1,ht(e,t,a,r),t.child)}function Ng(e,t,a,n,r,s){return Rr(t),t.updateQueue=null,a=Yy(t,n,a,r),Gy(e),n=Of(),e!==null&&!dt?(Lf(e,t,s),bn(e,t,s)):(he&&n&&kf(t),t.flags|=1,ht(e,t,a,s),t.child)}function _g(e,t,a,n,r){if(Rr(t),t.stateNode===null){var s=hs,i=a.contextType;typeof i=="object"&&i!==null&&(s=St(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=Im,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},Tf(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?St(i):hs,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(Wd(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&Im.enqueueReplaceState(s,s.state,null),eo(t,n,s,r),Wi(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,u=Er(a,o);s.props=u;var c=s.context,d=a.contextType;i=hs,typeof d=="object"&&d!==null&&(i=St(d));var m=a.getDerivedStateFromProps;d=typeof m=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&bg(t,s,n,i),Un=!1;var f=t.memoizedState;s.state=f,eo(t,n,s,r),Wi(),c=t.memoizedState,o||f!==c||Un?(typeof m=="function"&&(Wd(t,a,m,n),c=t.memoizedState),(u=Un||yg(t,a,u,n,f,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=u):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,Um(e,t),i=t.memoizedProps,d=Er(a,i),s.props=d,m=t.pendingProps,f=s.context,c=a.contextType,u=hs,typeof c=="object"&&c!==null&&(u=St(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==m||f!==u)&&bg(t,s,n,u),Un=!1,f=t.memoizedState,s.state=f,eo(t,n,s,r),Wi();var p=t.memoizedState;i!==m||f!==p||Un||e!==null&&e.dependencies!==null&&yu(e.dependencies)?(typeof o=="function"&&(Wd(t,a,o,n),p=t.memoizedState),(d=Un||yg(t,a,d,n,f,p,u)||e!==null&&e.dependencies!==null&&yu(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,p,u),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,p,u)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=p),s.props=n,s.state=p,s.context=u,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,ru(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=Ms(t,e.child,null,r),t.child=Ms(t,null,a,r)):ht(e,t,a,r),t.memoizedState=s.state,e=t.child):e=bn(e,t,r),e}function kg(e,t,a,n){return Co(),t.flags|=256,ht(e,t,a,n),t.child}var em={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function tm(e){return{baseLanes:e,cachePool:Iy()}}function am(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=ha),e}function Ub(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(rt.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(he){if(r?Fn(t):Bn(t),he){var o=qe,u;if(u=o){e:{for(u=o,o=za;u.nodeType!==8;){if(!o){o=null;break e}if(u=_a(u.nextSibling),u===null){o=null;break e}}o=u}o!==null?(t.memoizedState={dehydrated:o,treeContext:$r!==null?{id:dn,overflow:mn}:null,retryLane:536870912,hydrationErrors:null},u=Vt(18,null,null,0),u.stateNode=o,u.return=t,t.child=u,Ct=t,qe=null,u=!0):u=!1}u||kr(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return sf(o)?t.lanes=32:t.lanes=536870912,null;pn(t)}return o=n.children,n=n.fallback,r?(Bn(t),r=t.mode,o=ku({mode:"hidden",children:o},r),n=xr(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=tm(a),r.childLanes=am(e,i,a),t.memoizedState=em,n):(Fn(t),Qm(t,o))}if(u=e.memoizedState,u!==null&&(o=u.dehydrated,o!==null)){if(s)t.flags&256?(Fn(t),t.flags&=-257,t=nm(e,t,a)):t.memoizedState!==null?(Bn(t),t.child=e.child,t.flags|=128,t=null):(Bn(t),r=n.fallback,o=t.mode,n=ku({mode:"visible",children:n.children},o),r=xr(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,Ms(t,e.child,null,a),n=t.child,n.memoizedState=tm(a),n.childLanes=am(e,i,a),t.memoizedState=em,t=r);else if(Fn(t),sf(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(U(419)),n.stack="",n.digest=i,mo({value:n,source:null,stack:null}),t=nm(e,t,a)}else if(dt||Eo(e,t,a,!1),i=(a&e.childLanes)!==0,dt||i){if(i=ke,i!==null&&(n=a&-a,n=(n&42)!==0?1:pf(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==u.retryLane))throw u.retryLane=n,Bs(e,n),Xt(i,e,n),Ob;o.data==="$?"||Zm(),t=nm(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=u.treeContext,qe=_a(o.nextSibling),Ct=t,he=!0,wr=null,za=!1,e!==null&&(da[ma++]=dn,da[ma++]=mn,da[ma++]=$r,dn=e.id,mn=e.overflow,$r=t),t=Qm(t,n.children),t.flags|=4096);return t}return r?(Bn(t),r=n.fallback,o=t.mode,u=e.child,c=u.sibling,n=hn(u,{mode:"hidden",children:n.children}),n.subtreeFlags=u.subtreeFlags&65011712,c!==null?r=hn(c,r):(r=xr(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=tm(a):(u=o.cachePool,u!==null?(c=nt._currentValue,u=u.parent!==c?{parent:c,pool:c}:u):u=Iy(),o={baseLanes:o.baseLanes|a,cachePool:u}),r.memoizedState=o,r.childLanes=am(e,i,a),t.memoizedState=em,n):(Fn(t),a=e.child,e=a.sibling,a=hn(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function Qm(e,t){return t=ku({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function ku(e,t){return e=Vt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function nm(e,t,a){return Ms(t,e.child,null,a),e=Qm(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function Rg(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),Mm(e.return,t,a)}function rm(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function jb(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(ht(e,t,n.children,a),n=rt.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&Rg(e,a,t);else if(e.tag===19)Rg(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Oe(rt,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&Su(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),rm(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&Su(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}rm(t,!0,a,null,s);break;case"together":rm(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function bn(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),ar|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(Eo(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(U(153));if(t.child!==null){for(e=t.child,a=hn(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=hn(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function Kf(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&yu(e)))}function WC(e,t,a){switch(t.tag){case 3:cu(t,t.stateNode.containerInfo),jn(t,nt,e.memoizedState.cache),Co();break;case 27:case 5:$m(t);break;case 4:cu(t,t.stateNode.containerInfo);break;case 10:jn(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(Fn(t),t.flags|=128,null):(a&t.child.childLanes)!==0?Ub(e,t,a):(Fn(t),e=bn(e,t,a),e!==null?e.sibling:null);Fn(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(Eo(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return jb(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Oe(rt,rt.current),n)break;return null;case 22:case 23:return t.lanes=0,Pb(e,t,a);case 24:jn(t,nt,e.memoizedState.cache)}return bn(e,t,a)}function Fb(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)dt=!0;else{if(!Kf(e,a)&&(t.flags&128)===0)return dt=!1,WC(e,t,a);dt=(e.flags&131072)!==0}else dt=!1,he&&(t.flags&1048576)!==0&&zy(t,gu,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")_f(n)?(e=Er(n,e),t.tag=1,t=_g(null,t,n,e,a)):(t.tag=0,t=Hm(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===df){t.tag=11,t=$g(null,t,n,e,a);break e}else if(r===mf){t.tag=14,t=wg(null,t,n,e,a);break e}}throw t=bm(n)||n,Error(U(306,t,""))}}return t;case 0:return Hm(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Er(n,t.pendingProps),_g(e,t,n,r,a);case 3:e:{if(cu(t,t.stateNode.containerInfo),e===null)throw Error(U(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,Um(e,t),eo(t,n,null,a);var i=t.memoizedState;if(n=i.cache,jn(t,nt,n),n!==s.cache&&Om(t,[nt],a,!0),Wi(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=kg(e,t,n,a);break e}else if(n!==r){r=pa(Error(U(424)),t),mo(r),t=kg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(qe=_a(e.firstChild),Ct=t,he=!0,wr=null,za=!0,a=Rb(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if(Co(),n===r){t=bn(e,t,a);break e}ht(e,t,n,a)}t=t.child}return t;case 26:return ru(e,t),e===null?(a=Qg(t.type,null,t.pendingProps,null))?t.memoizedState=a:he||(a=t.type,e=t.pendingProps,n=Mu(Qn.current).createElement(a),n[wt]=t,n[jt]=e,gt(n,a,e),ct(n),t.stateNode=n):t.memoizedState=Qg(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return $m(t),e===null&&he&&(n=t.stateNode=_0(t.type,t.pendingProps,Qn.current),Ct=t,za=!0,r=qe,rr(t.type)?(of=r,qe=_a(n.firstChild)):qe=r),ht(e,t,t.pendingProps.children,a),ru(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&he&&((r=n=qe)&&(n=_E(n,t.type,t.pendingProps,za),n!==null?(t.stateNode=n,Ct=t,qe=_a(n.firstChild),za=!1,r=!0):r=!1),r||kr(t)),$m(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,nf(r,s)?n=null:i!==null&&nf(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=Mf(e,t,HC,null,null,a),yo._currentValue=r),ru(e,t),ht(e,t,n,a),t.child;case 6:return e===null&&he&&((e=a=qe)&&(a=kE(a,t.pendingProps,za),a!==null?(t.stateNode=a,Ct=t,qe=null,e=!0):e=!1),e||kr(t)),null;case 13:return Ub(e,t,a);case 4:return cu(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=Ms(t,null,n,a):ht(e,t,n,a),t.child;case 11:return $g(e,t,t.type,t.pendingProps,a);case 7:return ht(e,t,t.pendingProps,a),t.child;case 8:return ht(e,t,t.pendingProps.children,a),t.child;case 12:return ht(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,jn(t,t.type,n.value),ht(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,Rr(t),r=St(r),n=n(r),t.flags|=1,ht(e,t,n,a),t.child;case 14:return wg(e,t,t.type,t.pendingProps,a);case 15:return Lb(e,t,t.type,t.pendingProps,a);case 19:return jb(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=ku(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=hn(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return Pb(e,t,a);case 24:return Rr(t),n=St(nt),e===null?(r=Ef(),r===null&&(r=ke,s=Cf(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},Tf(t),jn(t,nt,r)):((e.lanes&a)!==0&&(Um(e,t),eo(t,null,null,a),Wi()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),jn(t,nt,n)):(n=s.cache,jn(t,nt,n),n!==r.cache&&Om(t,[nt],a,!0))),ht(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(U(156,t.tag))}function on(e){e.flags|=4}function Cg(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!C0(t)){if(t=va.current,t!==null&&((fe&4194048)===fe?Ha!==null:(fe&62914560)!==fe&&(fe&536870912)===0||t!==Ha))throw Xi=Pm,Ky;e.flags|=8192}}function Kl(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?dy():536870912,e.lanes|=t,Os|=t)}function Fi(e,t){if(!he)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function Fe(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function eE(e,t,a){var n=t.pendingProps;switch(Rf(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return Fe(t),null;case 1:return Fe(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),vn(nt),Rs(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Ui(t)?on(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,rg())),Fe(t),null;case 26:return a=t.memoizedState,e===null?(on(t),a!==null?(Fe(t),Cg(t,a)):(Fe(t),t.flags&=-16777217)):a?a!==e.memoizedState?(on(t),Fe(t),Cg(t,a)):(Fe(t),t.flags&=-16777217):(e.memoizedProps!==n&&on(t),Fe(t),t.flags&=-16777217),null;case 27:du(t),a=Qn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&on(t);else{if(!n){if(t.stateNode===null)throw Error(U(166));return Fe(t),null}e=Ia.current,Ui(t)?ag(t,e):(e=_0(r,n,a),t.stateNode=e,on(t))}return Fe(t),null;case 5:if(du(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&on(t);else{if(!n){if(t.stateNode===null)throw Error(U(166));return Fe(t),null}if(e=Ia.current,Ui(t))ag(t,e);else{switch(r=Mu(Qn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[wt]=t,e[jt]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(gt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&on(t)}}return Fe(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&on(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(U(166));if(e=Qn.current,Ui(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=Ct,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[wt]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||w0(e.nodeValue,a)),e||kr(t)}else e=Mu(e).createTextNode(n),e[wt]=t,t.stateNode=e}return Fe(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Ui(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(U(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(U(317));r[wt]=t}else Co(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;Fe(t),r=!1}else r=rg(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(pn(t),t):(pn(t),null)}if(pn(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),Kl(t,t.updateQueue),Fe(t),null;case 4:return Rs(),e===null&&Zf(t.stateNode.containerInfo),Fe(t),null;case 10:return vn(t.type),Fe(t),null;case 19:if(mt(rt),r=t.memoizedState,r===null)return Fe(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)Fi(r,!1);else{if(Ie!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=Su(e),s!==null){for(t.flags|=128,Fi(r,!1),e=s.updateQueue,t.updateQueue=e,Kl(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)By(a,e),a=a.sibling;return Oe(rt,rt.current&1|2),t.child}e=e.sibling}r.tail!==null&&Ka()>Cu&&(t.flags|=128,n=!0,Fi(r,!1),t.lanes=4194304)}else{if(!n)if(e=Su(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,Kl(t,e),Fi(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!he)return Fe(t),null}else 2*Ka()-r.renderingStartTime>Cu&&a!==536870912&&(t.flags|=128,n=!0,Fi(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=Ka(),t.sibling=null,e=rt.current,Oe(rt,n?e&1|2:e&1),t):(Fe(t),null);case 22:case 23:return pn(t),Af(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(Fe(t),t.subtreeFlags&6&&(t.flags|=8192)):Fe(t),a=t.updateQueue,a!==null&&Kl(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&mt(Sr),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),vn(nt),Fe(t),null;case 25:return null;case 30:return null}throw Error(U(156,t.tag))}function tE(e,t){switch(Rf(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return vn(nt),Rs(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return du(t),null;case 13:if(pn(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(U(340));Co()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return mt(rt),null;case 4:return Rs(),null;case 10:return vn(t.type),null;case 22:case 23:return pn(t),Af(),e!==null&&mt(Sr),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return vn(nt),null;case 25:return null;default:return null}}function Bb(e,t){switch(Rf(t),t.tag){case 3:vn(nt),Rs();break;case 26:case 27:case 5:du(t);break;case 4:Rs();break;case 13:pn(t);break;case 19:mt(rt);break;case 10:vn(t.type);break;case 22:case 23:pn(t),Af(),e!==null&&mt(Sr);break;case 24:vn(nt)}}function Oo(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){_e(t,t.return,o)}}function tr(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var u=a,c=o;try{c()}catch(d){_e(r,u,d)}}}n=n.next}while(n!==s)}}catch(d){_e(t,t.return,d)}}function zb(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{Vy(t,a)}catch(n){_e(e,e.return,n)}}}function qb(e,t,a){a.props=Er(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){_e(e,t,n)}}function ao(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){_e(e,t,r)}}function qa(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){_e(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){_e(e,t,r)}else a.current=null}function Ib(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){_e(e,e.return,r)}}function sm(e,t,a){try{var n=e.stateNode;xE(n,e.type,a,t),n[jt]=t}catch(r){_e(e,e.return,r)}}function Kb(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&rr(e.type)||e.tag===4}function im(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||Kb(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&rr(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function Vm(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=Zu));else if(n!==4&&(n===27&&rr(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(Vm(e,t,a),e=e.sibling;e!==null;)Vm(e,t,a),e=e.sibling}function Ru(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&rr(e.type)&&(a=e.stateNode),e=e.child,e!==null))for(Ru(e,t,a),e=e.sibling;e!==null;)Ru(e,t,a),e=e.sibling}function Hb(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);gt(t,n,a),t[wt]=e,t[jt]=a}catch(s){_e(e,e.return,s)}}var un=!1,Ge=!1,om=!1,Eg=typeof WeakSet=="function"?WeakSet:Set,ut=null;function aE(e,t){if(e=e.containerInfo,tf=Uu,e=Dy(e),wf(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,u=-1,c=0,d=0,m=e,f=null;t:for(;;){for(var p;m!==a||r!==0&&m.nodeType!==3||(o=i+r),m!==s||n!==0&&m.nodeType!==3||(u=i+n),m.nodeType===3&&(i+=m.nodeValue.length),(p=m.firstChild)!==null;)f=m,m=p;for(;;){if(m===e)break t;if(f===a&&++c===r&&(o=i),f===s&&++d===n&&(u=i),(p=m.nextSibling)!==null)break;m=f,f=m.parentNode}m=p}a=o===-1||u===-1?null:{start:o,end:u}}else a=null}a=a||{start:0,end:0}}else a=null;for(af={focusedElem:e,selectionRange:a},Uu=!1,ut=t;ut!==null;)if(t=ut,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,ut=e;else for(;ut!==null;){switch(t=ut,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var x=Er(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(x,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(y){_e(a,a.return,y)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)rf(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":rf(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(U(163))}if(e=t.sibling,e!==null){e.return=t.return,ut=e;break}ut=t.return}}function Qb(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:On(e,a),n&4&&Oo(5,a);break;case 1:if(On(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){_e(a,a.return,i)}else{var r=Er(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){_e(a,a.return,i)}}n&64&&zb(a),n&512&&ao(a,a.return);break;case 3:if(On(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{Vy(e,t)}catch(i){_e(a,a.return,i)}}break;case 27:t===null&&n&4&&Hb(a);case 26:case 5:On(e,a),t===null&&n&4&&Ib(a),n&512&&ao(a,a.return);break;case 12:On(e,a);break;case 13:On(e,a),n&4&&Yb(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=dE.bind(null,a),RE(e,a))));break;case 22:if(n=a.memoizedState!==null||un,!n){t=t!==null&&t.memoizedState!==null||Ge,r=un;var s=Ge;un=n,(Ge=t)&&!s?Ln(e,a,(a.subtreeFlags&8772)!==0):On(e,a),un=r,Ge=s}break;case 30:break;default:On(e,a)}}function Vb(e){var t=e.alternate;t!==null&&(e.alternate=null,Vb(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&vf(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var Me=null,Pt=!1;function ln(e,t,a){for(a=a.child;a!==null;)Gb(e,t,a),a=a.sibling}function Gb(e,t,a){if(Gt&&typeof Gt.onCommitFiberUnmount=="function")try{Gt.onCommitFiberUnmount(So,a)}catch{}switch(a.tag){case 26:Ge||qa(a,t),ln(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:Ge||qa(a,t);var n=Me,r=Pt;rr(a.type)&&(Me=a.stateNode,Pt=!1),ln(e,t,a),io(a.stateNode),Me=n,Pt=r;break;case 5:Ge||qa(a,t);case 6:if(n=Me,r=Pt,Me=null,ln(e,t,a),Me=n,Pt=r,Me!==null)if(Pt)try{(Me.nodeType===9?Me.body:Me.nodeName==="HTML"?Me.ownerDocument.body:Me).removeChild(a.stateNode)}catch(s){_e(a,t,s)}else try{Me.removeChild(a.stateNode)}catch(s){_e(a,t,s)}break;case 18:Me!==null&&(Pt?(e=Me,Ig(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),$o(e)):Ig(Me,a.stateNode));break;case 4:n=Me,r=Pt,Me=a.stateNode.containerInfo,Pt=!0,ln(e,t,a),Me=n,Pt=r;break;case 0:case 11:case 14:case 15:Ge||tr(2,a,t),Ge||tr(4,a,t),ln(e,t,a);break;case 1:Ge||(qa(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&qb(a,t,n)),ln(e,t,a);break;case 21:ln(e,t,a);break;case 22:Ge=(n=Ge)||a.memoizedState!==null,ln(e,t,a),Ge=n;break;default:ln(e,t,a)}}function Yb(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{$o(e)}catch(a){_e(t,t.return,a)}}function nE(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new Eg),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new Eg),t;default:throw Error(U(435,e.tag))}}function lm(e,t){var a=nE(e);t.forEach(function(n){var r=mE.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function Kt(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(rr(o.type)){Me=o.stateNode,Pt=!1;break e}break;case 5:Me=o.stateNode,Pt=!1;break e;case 3:case 4:Me=o.stateNode.containerInfo,Pt=!0;break e}o=o.return}if(Me===null)throw Error(U(160));Gb(s,i,r),Me=null,Pt=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)Jb(t,e),t=t.sibling}var Na=null;function Jb(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:Kt(t,e),Ht(e),n&4&&(tr(3,e,e.return),Oo(3,e),tr(5,e,e.return));break;case 1:Kt(t,e),Ht(e),n&512&&(Ge||a===null||qa(a,a.return)),n&64&&un&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=Na;if(Kt(t,e),Ht(e),n&512&&(Ge||a===null||qa(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[ko]||s[wt]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),gt(s,n,a),s[wt]=e,ct(s),n=s;break e;case"link":var i=Gg("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),gt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=Gg("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),gt(s,n,a),r.head.appendChild(s);break;default:throw Error(U(468,n))}s[wt]=e,ct(s),n=s}e.stateNode=n}else Yg(r,e.type,e.stateNode);else e.stateNode=Vg(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?Yg(r,e.type,e.stateNode):Vg(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&sm(e,e.memoizedProps,a.memoizedProps)}break;case 27:Kt(t,e),Ht(e),n&512&&(Ge||a===null||qa(a,a.return)),a!==null&&n&4&&sm(e,e.memoizedProps,a.memoizedProps);break;case 5:if(Kt(t,e),Ht(e),n&512&&(Ge||a===null||qa(a,a.return)),e.flags&32){r=e.stateNode;try{Es(r,"")}catch(p){_e(e,e.return,p)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,sm(e,r,a!==null?a.memoizedProps:r)),n&1024&&(om=!0);break;case 6:if(Kt(t,e),Ht(e),n&4){if(e.stateNode===null)throw Error(U(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(p){_e(e,e.return,p)}}break;case 3:if(ou=null,r=Na,Na=Ou(t.containerInfo),Kt(t,e),Na=r,Ht(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{$o(t.containerInfo)}catch(p){_e(e,e.return,p)}om&&(om=!1,Xb(e));break;case 4:n=Na,Na=Ou(e.stateNode.containerInfo),Kt(t,e),Ht(e),Na=n;break;case 12:Kt(t,e),Ht(e);break;case 13:Kt(t,e),Ht(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(Yf=Ka()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,lm(e,n)));break;case 22:r=e.memoizedState!==null;var u=a!==null&&a.memoizedState!==null,c=un,d=Ge;if(un=c||r,Ge=d||u,Kt(t,e),Ge=d,un=c,Ht(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||u||un||Ge||yr(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){u=a=t;try{if(s=u.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=u.stateNode;var m=u.memoizedProps.style,f=m!=null&&m.hasOwnProperty("display")?m.display:null;o.style.display=f==null||typeof f=="boolean"?"":(""+f).trim()}}catch(p){_e(u,u.return,p)}}}else if(t.tag===6){if(a===null){u=t;try{u.stateNode.nodeValue=r?"":u.memoizedProps}catch(p){_e(u,u.return,p)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,lm(e,a))));break;case 19:Kt(t,e),Ht(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,lm(e,n)));break;case 30:break;case 21:break;default:Kt(t,e),Ht(e)}}function Ht(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(Kb(n)){a=n;break}n=n.return}if(a==null)throw Error(U(160));switch(a.tag){case 27:var r=a.stateNode,s=im(e);Ru(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(Es(i,""),a.flags&=-33);var o=im(e);Ru(e,o,i);break;case 3:case 4:var u=a.stateNode.containerInfo,c=im(e);Vm(e,c,u);break;default:throw Error(U(161))}}catch(d){_e(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function Xb(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;Xb(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function On(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)Qb(e,t.alternate,t),t=t.sibling}function yr(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:tr(4,t,t.return),yr(t);break;case 1:qa(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&qb(t,t.return,a),yr(t);break;case 27:io(t.stateNode);case 26:case 5:qa(t,t.return),yr(t);break;case 22:t.memoizedState===null&&yr(t);break;case 30:yr(t);break;default:yr(t)}e=e.sibling}}function Ln(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:Ln(r,s,a),Oo(4,s);break;case 1:if(Ln(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){_e(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var u=r.shared.hiddenCallbacks;if(u!==null)for(r.shared.hiddenCallbacks=null,r=0;r<u.length;r++)Qy(u[r],o)}catch(c){_e(n,n.return,c)}}a&&i&64&&zb(s),ao(s,s.return);break;case 27:Hb(s);case 26:case 5:Ln(r,s,a),a&&n===null&&i&4&&Ib(s),ao(s,s.return);break;case 12:Ln(r,s,a);break;case 13:Ln(r,s,a),a&&i&4&&Yb(r,s);break;case 22:s.memoizedState===null&&Ln(r,s,a),ao(s,s.return);break;case 30:break;default:Ln(r,s,a)}t=t.sibling}}function Hf(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&To(a))}function Qf(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&To(e))}function Ba(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)Zb(e,t,a,n),t=t.sibling}function Zb(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:Ba(e,t,a,n),r&2048&&Oo(9,t);break;case 1:Ba(e,t,a,n);break;case 3:Ba(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&To(e)));break;case 12:if(r&2048){Ba(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(u){_e(t,t.return,u)}}else Ba(e,t,a,n);break;case 13:Ba(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?Ba(e,t,a,n):no(e,t):s._visibility&2?Ba(e,t,a,n):(s._visibility|=2,ss(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&Hf(i,t);break;case 24:Ba(e,t,a,n),r&2048&&Qf(t.alternate,t);break;default:Ba(e,t,a,n)}}function ss(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,u=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:ss(s,i,o,u,r),Oo(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?ss(s,i,o,u,r):no(s,i):(d._visibility|=2,ss(s,i,o,u,r)),r&&c&2048&&Hf(i.alternate,i);break;case 24:ss(s,i,o,u,r),r&&c&2048&&Qf(i.alternate,i);break;default:ss(s,i,o,u,r)}t=t.sibling}}function no(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:no(a,n),r&2048&&Hf(n.alternate,n);break;case 24:no(a,n),r&2048&&Qf(n.alternate,n);break;default:no(a,n)}t=t.sibling}}var Qi=8192;function as(e){if(e.subtreeFlags&Qi)for(e=e.child;e!==null;)Wb(e),e=e.sibling}function Wb(e){switch(e.tag){case 26:as(e),e.flags&Qi&&e.memoizedState!==null&&BE(Na,e.memoizedState,e.memoizedProps);break;case 5:as(e);break;case 3:case 4:var t=Na;Na=Ou(e.stateNode.containerInfo),as(e),Na=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=Qi,Qi=16777216,as(e),Qi=t):as(e));break;default:as(e)}}function e0(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function Bi(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ut=n,a0(n,e)}e0(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)t0(e),e=e.sibling}function t0(e){switch(e.tag){case 0:case 11:case 15:Bi(e),e.flags&2048&&tr(9,e,e.return);break;case 3:Bi(e);break;case 12:Bi(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,su(e)):Bi(e);break;default:Bi(e)}}function su(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ut=n,a0(n,e)}e0(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:tr(8,t,t.return),su(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,su(t));break;default:su(t)}e=e.sibling}}function a0(e,t){for(;ut!==null;){var a=ut;switch(a.tag){case 0:case 11:case 15:tr(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:To(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,ut=n;else e:for(a=e;ut!==null;){n=ut;var r=n.sibling,s=n.return;if(Vb(n),n===a){ut=null;break e}if(r!==null){r.return=s,ut=r;break e}ut=s}}}var rE={getCacheForType:function(e){var t=St(nt),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},sE=typeof WeakMap=="function"?WeakMap:Map,we=0,ke=null,ue=null,fe=0,$e=0,Qt=null,Kn=!1,zs=!1,Vf=!1,xn=0,Ie=0,ar=0,Nr=0,Gf=0,ha=0,Os=0,ro=null,Ut=null,Gm=!1,Yf=0,Cu=1/0,Eu=null,Yn=null,vt=0,Jn=null,Ls=null,ks=0,Ym=0,Jm=null,n0=null,so=0,Xm=null;function Jt(){if((we&2)!==0&&fe!==0)return fe&-fe;if(te.T!==null){var e=Ts;return e!==0?e:Xf()}return py()}function r0(){ha===0&&(ha=(fe&536870912)===0||he?cy():536870912);var e=va.current;return e!==null&&(e.flags|=32),ha}function Xt(e,t,a){(e===ke&&($e===2||$e===9)||e.cancelPendingCommit!==null)&&(Ps(e,0),Hn(e,fe,ha,!1)),_o(e,a),((we&2)===0||e!==ke)&&(e===ke&&((we&2)===0&&(Nr|=a),Ie===4&&Hn(e,fe,ha,!1)),Va(e))}function s0(e,t,a){if((we&6)!==0)throw Error(U(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||No(e,t),r=n?lE(e,t):um(e,t,!0),s=n;do{if(r===0){zs&&!n&&Hn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!iE(a)){r=um(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=ro;var u=o.current.memoizedState.isDehydrated;if(u&&(Ps(o,i).flags|=256),i=um(o,i,!1),i!==2){if(Vf&&!u){o.errorRecoveryDisabledLanes|=s,Nr|=s,r=4;break e}s=Ut,Ut=r,s!==null&&(Ut===null?Ut=s:Ut.push.apply(Ut,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){Ps(e,0),Hn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(U(345));case 4:if((t&4194048)!==t)break;case 6:Hn(n,t,ha,!Kn);break e;case 2:Ut=null;break;case 3:case 5:break;default:throw Error(U(329))}if((t&62914560)===t&&(r=Yf+300-Ka(),10<r)){if(Hn(n,t,ha,!Kn),Fu(n,0,!0)!==0)break e;n.timeoutHandle=N0(Tg.bind(null,n,a,Ut,Eu,Gm,t,ha,Nr,Os,Kn,s,2,-0,0),r);break e}Tg(n,a,Ut,Eu,Gm,t,ha,Nr,Os,Kn,s,0,-0,0)}}break}while(!0);Va(e)}function Tg(e,t,a,n,r,s,i,o,u,c,d,m,f,p){if(e.timeoutHandle=-1,m=t.subtreeFlags,(m&8192||(m&16785408)===16785408)&&(go={stylesheets:null,count:0,unsuspend:FE},Wb(t),m=zE(),m!==null)){e.cancelPendingCommit=m(Dg.bind(null,e,t,s,a,n,r,i,o,u,d,1,f,p)),Hn(e,s,i,!c);return}Dg(e,t,s,a,n,r,i,o,u)}function iE(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!Zt(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function Hn(e,t,a,n){t&=~Gf,t&=~Nr,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Yt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&my(e,a,t)}function Yu(){return(we&6)===0?(Lo(0,!1),!1):!0}function Jf(){if(ue!==null){if($e===0)var e=ue.return;else e=ue,fn=Mr=null,Pf(e),_s=null,po=0,e=ue;for(;e!==null;)Bb(e.alternate,e),e=e.return;ue=null}}function Ps(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,wE(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),Jf(),ke=e,ue=a=hn(e.current,null),fe=t,$e=0,Qt=null,Kn=!1,zs=No(e,t),Vf=!1,Os=ha=Gf=Nr=ar=Ie=0,Ut=ro=null,Gm=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Yt(n),s=1<<r;t|=e[r],n&=~s}return xn=t,Iu(),a}function i0(e,t){ie=null,te.H=wu,t===Ao||t===Hu?(t=lg(),$e=3):t===Ky?(t=lg(),$e=4):$e=t===Ob?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Qt=t,ue===null&&(Ie=1,_u(e,pa(t,e.current)))}function o0(){var e=te.H;return te.H=wu,e===null?wu:e}function l0(){var e=te.A;return te.A=rE,e}function Zm(){Ie=4,Kn||(fe&4194048)!==fe&&va.current!==null||(zs=!0),(ar&134217727)===0&&(Nr&134217727)===0||ke===null||Hn(ke,fe,ha,!1)}function um(e,t,a){var n=we;we|=2;var r=o0(),s=l0();(ke!==e||fe!==t)&&(Eu=null,Ps(e,t)),t=!1;var i=Ie;e:do try{if($e!==0&&ue!==null){var o=ue,u=Qt;switch($e){case 8:Jf(),i=6;break e;case 3:case 2:case 9:case 6:va.current===null&&(t=!0);var c=$e;if($e=0,Qt=null,ys(e,o,u,c),a&&zs){i=0;break e}break;default:c=$e,$e=0,Qt=null,ys(e,o,u,c)}}oE(),i=Ie;break}catch(d){i0(e,d)}while(!0);return t&&e.shellSuspendCounter++,fn=Mr=null,we=n,te.H=r,te.A=s,ue===null&&(ke=null,fe=0,Iu()),i}function oE(){for(;ue!==null;)u0(ue)}function lE(e,t){var a=we;we|=2;var n=o0(),r=l0();ke!==e||fe!==t?(Eu=null,Cu=Ka()+500,Ps(e,t)):zs=No(e,t);e:do try{if($e!==0&&ue!==null){t=ue;var s=Qt;t:switch($e){case 1:$e=0,Qt=null,ys(e,t,s,1);break;case 2:case 9:if(og(s)){$e=0,Qt=null,Ag(t);break}t=function(){$e!==2&&$e!==9||ke!==e||($e=7),Va(e)},s.then(t,t);break e;case 3:$e=7;break e;case 4:$e=5;break e;case 7:og(s)?($e=0,Qt=null,Ag(t)):($e=0,Qt=null,ys(e,t,s,7));break;case 5:var i=null;switch(ue.tag){case 26:i=ue.memoizedState;case 5:case 27:var o=ue;if(!i||C0(i)){$e=0,Qt=null;var u=o.sibling;if(u!==null)ue=u;else{var c=o.return;c!==null?(ue=c,Ju(c)):ue=null}break t}}$e=0,Qt=null,ys(e,t,s,5);break;case 6:$e=0,Qt=null,ys(e,t,s,6);break;case 8:Jf(),Ie=6;break e;default:throw Error(U(462))}}uE();break}catch(d){i0(e,d)}while(!0);return fn=Mr=null,te.H=n,te.A=r,we=a,ue!==null?0:(ke=null,fe=0,Iu(),Ie)}function uE(){for(;ue!==null&&!AR();)u0(ue)}function u0(e){var t=Fb(e.alternate,e,xn);e.memoizedProps=e.pendingProps,t===null?Ju(e):ue=t}function Ag(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=Ng(a,t,t.pendingProps,t.type,void 0,fe);break;case 11:t=Ng(a,t,t.pendingProps,t.type.render,t.ref,fe);break;case 5:Pf(t);default:Bb(a,t),t=ue=By(t,xn),t=Fb(a,t,xn)}e.memoizedProps=e.pendingProps,t===null?Ju(e):ue=t}function ys(e,t,a,n){fn=Mr=null,Pf(t),_s=null,po=0;var r=t.return;try{if(ZC(e,r,t,a,fe)){Ie=1,_u(e,pa(a,e.current)),ue=null;return}}catch(s){if(r!==null)throw ue=r,s;Ie=1,_u(e,pa(a,e.current)),ue=null;return}t.flags&32768?(he||n===1?e=!0:zs||(fe&536870912)!==0?e=!1:(Kn=e=!0,(n===2||n===9||n===3||n===6)&&(n=va.current,n!==null&&n.tag===13&&(n.flags|=16384))),c0(t,e)):Ju(t)}function Ju(e){var t=e;do{if((t.flags&32768)!==0){c0(t,Kn);return}e=t.return;var a=eE(t.alternate,t,xn);if(a!==null){ue=a;return}if(t=t.sibling,t!==null){ue=t;return}ue=t=e}while(t!==null);Ie===0&&(Ie=5)}function c0(e,t){do{var a=tE(e.alternate,e);if(a!==null){a.flags&=32767,ue=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){ue=e;return}ue=e=a}while(e!==null);Ie=6,ue=null}function Dg(e,t,a,n,r,s,i,o,u){e.cancelPendingCommit=null;do Xu();while(vt!==0);if((we&6)!==0)throw Error(U(327));if(t!==null){if(t===e.current)throw Error(U(177));if(s=t.lanes|t.childLanes,s|=Sf,zR(e,a,s,i,o,u),e===ke&&(ue=ke=null,fe=0),Ls=t,Jn=e,ks=a,Ym=s,Jm=r,n0=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,fE(mu,function(){return h0(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=te.T,te.T=null,r=ve.p,ve.p=2,i=we,we|=4;try{aE(e,t,a)}finally{we=i,ve.p=r,te.T=n}}vt=1,d0(),m0(),f0()}}function d0(){if(vt===1){vt=0;var e=Jn,t=Ls,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=te.T,te.T=null;var n=ve.p;ve.p=2;var r=we;we|=4;try{Jb(t,e);var s=af,i=Dy(e.containerInfo),o=s.focusedElem,u=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&Ay(o.ownerDocument.documentElement,o)){if(u!==null&&wf(o)){var c=u.start,d=u.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var m=o.ownerDocument||document,f=m&&m.defaultView||window;if(f.getSelection){var p=f.getSelection(),x=o.textContent.length,y=Math.min(u.start,x),w=u.end===void 0?y:Math.min(u.end,x);!p.extend&&y>w&&(i=w,w=y,y=i);var g=Wv(o,y),v=Wv(o,w);if(g&&v&&(p.rangeCount!==1||p.anchorNode!==g.node||p.anchorOffset!==g.offset||p.focusNode!==v.node||p.focusOffset!==v.offset)){var b=m.createRange();b.setStart(g.node,g.offset),p.removeAllRanges(),y>w?(p.addRange(b),p.extend(v.node,v.offset)):(b.setEnd(v.node,v.offset),p.addRange(b))}}}}for(m=[],p=o;p=p.parentNode;)p.nodeType===1&&m.push({element:p,left:p.scrollLeft,top:p.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<m.length;o++){var $=m[o];$.element.scrollLeft=$.left,$.element.scrollTop=$.top}}Uu=!!tf,af=tf=null}finally{we=r,ve.p=n,te.T=a}}e.current=t,vt=2}}function m0(){if(vt===2){vt=0;var e=Jn,t=Ls,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=te.T,te.T=null;var n=ve.p;ve.p=2;var r=we;we|=4;try{Qb(e,t.alternate,t)}finally{we=r,ve.p=n,te.T=a}}vt=3}}function f0(){if(vt===4||vt===3){vt=0,DR();var e=Jn,t=Ls,a=ks,n=n0;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?vt=5:(vt=0,Ls=Jn=null,p0(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(Yn=null),hf(a),t=t.stateNode,Gt&&typeof Gt.onCommitFiberRoot=="function")try{Gt.onCommitFiberRoot(So,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=te.T,r=ve.p,ve.p=2,te.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{te.T=t,ve.p=r}}(ks&3)!==0&&Xu(),Va(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===Xm?so++:(so=0,Xm=e):so=0,Lo(0,!1)}}function p0(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,To(t)))}function Xu(e){return d0(),m0(),f0(),h0(e)}function h0(){if(vt!==5)return!1;var e=Jn,t=Ym;Ym=0;var a=hf(ks),n=te.T,r=ve.p;try{ve.p=32>a?32:a,te.T=null,a=Jm,Jm=null;var s=Jn,i=ks;if(vt=0,Ls=Jn=null,ks=0,(we&6)!==0)throw Error(U(331));var o=we;if(we|=4,t0(s.current),Zb(s,s.current,i,a),we=o,Lo(0,!1),Gt&&typeof Gt.onPostCommitFiberRoot=="function")try{Gt.onPostCommitFiberRoot(So,s)}catch{}return!0}finally{ve.p=r,te.T=n,p0(e,t)}}function Mg(e,t,a){t=pa(a,t),t=Km(e.stateNode,t,2),e=Gn(e,t,2),e!==null&&(_o(e,2),Va(e))}function _e(e,t,a){if(e.tag===3)Mg(e,e,a);else for(;t!==null;){if(t.tag===3){Mg(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(Yn===null||!Yn.has(n))){e=pa(a,e),a=Db(2),n=Gn(t,a,2),n!==null&&(Mb(a,n,t,e),_o(n,2),Va(n));break}}t=t.return}}function cm(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new sE;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(Vf=!0,r.add(a),e=cE.bind(null,e,t,a),t.then(e,e))}function cE(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,ke===e&&(fe&a)===a&&(Ie===4||Ie===3&&(fe&62914560)===fe&&300>Ka()-Yf?(we&2)===0&&Ps(e,0):Gf|=a,Os===fe&&(Os=0)),Va(e)}function v0(e,t){t===0&&(t=dy()),e=Bs(e,t),e!==null&&(_o(e,t),Va(e))}function dE(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),v0(e,a)}function mE(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(U(314))}n!==null&&n.delete(t),v0(e,a)}function fE(e,t){return ff(e,t)}var Tu=null,is=null,Wm=!1,Au=!1,dm=!1,_r=0;function Va(e){e!==is&&e.next===null&&(is===null?Tu=is=e:is=is.next=e),Au=!0,Wm||(Wm=!0,hE())}function Lo(e,t){if(!dm&&Au){dm=!0;do for(var a=!1,n=Tu;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Yt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,Og(n,s))}else s=fe,s=Fu(n,n===ke?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||No(n,s)||(a=!0,Og(n,s));n=n.next}while(a);dm=!1}}function pE(){g0()}function g0(){Au=Wm=!1;var e=0;_r!==0&&($E()&&(e=_r),_r=0);for(var t=Ka(),a=null,n=Tu;n!==null;){var r=n.next,s=y0(n,t);s===0?(n.next=null,a===null?Tu=r:a.next=r,r===null&&(is=a)):(a=n,(e!==0||(s&3)!==0)&&(Au=!0)),n=r}Lo(e,!1)}function y0(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Yt(s),o=1<<i,u=r[i];u===-1?((o&a)===0||(o&n)!==0)&&(r[i]=BR(o,t)):u<=t&&(e.expiredLanes|=o),s&=~o}if(t=ke,a=fe,a=Fu(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&($e===2||$e===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&jd(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||No(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&jd(n),hf(a)){case 2:case 8:a=ly;break;case 32:a=mu;break;case 268435456:a=uy;break;default:a=mu}return n=b0.bind(null,e),a=ff(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&jd(n),e.callbackPriority=2,e.callbackNode=null,2}function b0(e,t){if(vt!==0&&vt!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(Xu(!0)&&e.callbackNode!==a)return null;var n=fe;return n=Fu(e,e===ke?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:(s0(e,n,t),y0(e,Ka()),e.callbackNode!=null&&e.callbackNode===a?b0.bind(null,e):null)}function Og(e,t){if(Xu())return null;s0(e,t,!0)}function hE(){SE(function(){(we&6)!==0?ff(oy,pE):g0()})}function Xf(){return _r===0&&(_r=cy()),_r}function Lg(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:Xl(""+e)}function Pg(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function vE(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=Lg((r[jt]||null).action),i=n.submitter;i&&(t=(t=i[jt]||null)?Lg(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Bu("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(_r!==0){var u=i?Pg(r,i):new FormData(r);qm(a,{pending:!0,data:u,method:r.method,action:s},null,u)}}else typeof s=="function"&&(o.preventDefault(),u=i?Pg(r,i):new FormData(r),qm(a,{pending:!0,data:u,method:r.method,action:s},s,u))},currentTarget:r}]})}}for(Hl=0;Hl<Tm.length;Hl++)Ql=Tm[Hl],Ug=Ql.toLowerCase(),jg=Ql[0].toUpperCase()+Ql.slice(1),ka(Ug,"on"+jg);var Ql,Ug,jg,Hl;ka(Oy,"onAnimationEnd");ka(Ly,"onAnimationIteration");ka(Py,"onAnimationStart");ka("dblclick","onDoubleClick");ka("focusin","onFocus");ka("focusout","onBlur");ka(LC,"onTransitionRun");ka(PC,"onTransitionStart");ka(UC,"onTransitionCancel");ka(Uy,"onTransitionEnd");Cs("onMouseEnter",["mouseout","mouseover"]);Cs("onMouseLeave",["mouseout","mouseover"]);Cs("onPointerEnter",["pointerout","pointerover"]);Cs("onPointerLeave",["pointerout","pointerover"]);Tr("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Tr("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Tr("onBeforeInput",["compositionend","keypress","textInput","paste"]);Tr("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Tr("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Tr("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var ho="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),gE=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(ho));function x0(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],u=o.instance,c=o.currentTarget;if(o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Nu(d)}r.currentTarget=null,s=u}else for(i=0;i<n.length;i++){if(o=n[i],u=o.instance,c=o.currentTarget,o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Nu(d)}r.currentTarget=null,s=u}}}}function le(e,t){var a=t[Sm];a===void 0&&(a=t[Sm]=new Set);var n=e+"__bubble";a.has(n)||($0(t,e,2,!1),a.add(n))}function mm(e,t,a){var n=0;t&&(n|=4),$0(a,e,n,t)}var Vl="_reactListening"+Math.random().toString(36).slice(2);function Zf(e){if(!e[Vl]){e[Vl]=!0,hy.forEach(function(a){a!=="selectionchange"&&(gE.has(a)||mm(a,!1,e),mm(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[Vl]||(t[Vl]=!0,mm("selectionchange",!1,t))}}function $0(e,t,a,n){switch(M0(t)){case 2:var r=KE;break;case 8:r=HE;break;default:r=ap}a=r.bind(null,t,a,e),r=void 0,!Rm||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function fm(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var u=i.tag;if((u===3||u===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=us(o),i===null)return;if(u=i.tag,u===5||u===6||u===26||u===27){n=s=i;continue e}o=o.parentNode}}n=n.return}Sy(function(){var c=s,d=yf(a),m=[];e:{var f=jy.get(e);if(f!==void 0){var p=Bu,x=e;switch(e){case"keypress":if(Wl(a)===0)break e;case"keydown":case"keyup":p=fC;break;case"focusin":x="focus",p=Qd;break;case"focusout":x="blur",p=Qd;break;case"beforeblur":case"afterblur":p=Qd;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":p=Kv;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":p=tC;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":p=vC;break;case Oy:case Ly:case Py:p=rC;break;case Uy:p=yC;break;case"scroll":case"scrollend":p=WR;break;case"wheel":p=xC;break;case"copy":case"cut":case"paste":p=iC;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":p=Qv;break;case"toggle":case"beforetoggle":p=wC}var y=(t&4)!==0,w=!y&&(e==="scroll"||e==="scrollend"),g=y?f!==null?f+"Capture":null:f;y=[];for(var v=c,b;v!==null;){var $=v;if(b=$.stateNode,$=$.tag,$!==5&&$!==26&&$!==27||b===null||g===null||($=lo(v,g),$!=null&&y.push(vo(v,$,b))),w)break;v=v.return}0<y.length&&(f=new p(f,x,null,a,d),m.push({event:f,listeners:y}))}}if((t&7)===0){e:{if(f=e==="mouseover"||e==="pointerover",p=e==="mouseout"||e==="pointerout",f&&a!==km&&(x=a.relatedTarget||a.fromElement)&&(us(x)||x[js]))break e;if((p||f)&&(f=d.window===d?d:(f=d.ownerDocument)?f.defaultView||f.parentWindow:window,p?(x=a.relatedTarget||a.toElement,p=c,x=x?us(x):null,x!==null&&(w=wo(x),y=x.tag,x!==w||y!==5&&y!==27&&y!==6)&&(x=null)):(p=null,x=c),p!==x)){if(y=Kv,$="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(y=Qv,$="onPointerLeave",g="onPointerEnter",v="pointer"),w=p==null?f:Hi(p),b=x==null?f:Hi(x),f=new y($,v+"leave",p,a,d),f.target=w,f.relatedTarget=b,$=null,us(d)===c&&(y=new y(g,v+"enter",x,a,d),y.target=b,y.relatedTarget=w,$=y),w=$,p&&x)t:{for(y=p,g=x,v=0,b=y;b;b=ns(b))v++;for(b=0,$=g;$;$=ns($))b++;for(;0<v-b;)y=ns(y),v--;for(;0<b-v;)g=ns(g),b--;for(;v--;){if(y===g||g!==null&&y===g.alternate)break t;y=ns(y),g=ns(g)}y=null}else y=null;p!==null&&Fg(m,f,p,y,!1),x!==null&&w!==null&&Fg(m,w,x,y,!0)}}e:{if(f=c?Hi(c):window,p=f.nodeName&&f.nodeName.toLowerCase(),p==="select"||p==="input"&&f.type==="file")var S=Jv;else if(Yv(f))if(Ey)S=DC;else{S=TC;var E=EC}else p=f.nodeName,!p||p.toLowerCase()!=="input"||f.type!=="checkbox"&&f.type!=="radio"?c&&gf(c.elementType)&&(S=Jv):S=AC;if(S&&(S=S(e,c))){Cy(m,S,a,d);break e}E&&E(e,f,c),e==="focusout"&&c&&f.type==="number"&&c.memoizedProps.value!=null&&_m(f,"number",f.value)}switch(E=c?Hi(c):window,e){case"focusin":(Yv(E)||E.contentEditable==="true")&&(ms=E,Cm=c,Yi=null);break;case"focusout":Yi=Cm=ms=null;break;case"mousedown":Em=!0;break;case"contextmenu":case"mouseup":case"dragend":Em=!1,eg(m,a,d);break;case"selectionchange":if(OC)break;case"keydown":case"keyup":eg(m,a,d)}var _;if($f)e:{switch(e){case"compositionstart":var T="onCompositionStart";break e;case"compositionend":T="onCompositionEnd";break e;case"compositionupdate":T="onCompositionUpdate";break e}T=void 0}else ds?ky(e,a)&&(T="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(T="onCompositionStart");T&&(_y&&a.locale!=="ko"&&(ds||T!=="onCompositionStart"?T==="onCompositionEnd"&&ds&&(_=Ny()):(In=d,bf="value"in In?In.value:In.textContent,ds=!0)),E=Du(c,T),0<E.length&&(T=new Hv(T,e,null,a,d),m.push({event:T,listeners:E}),_?T.data=_:(_=Ry(a),_!==null&&(T.data=_)))),(_=NC?_C(e,a):kC(e,a))&&(T=Du(c,"onBeforeInput"),0<T.length&&(E=new Hv("onBeforeInput","beforeinput",null,a,d),m.push({event:E,listeners:T}),E.data=_)),vE(m,e,c,a,d)}x0(m,t)})}function vo(e,t,a){return{instance:e,listener:t,currentTarget:a}}function Du(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=lo(e,a),r!=null&&n.unshift(vo(e,r,s)),r=lo(e,t),r!=null&&n.push(vo(e,r,s))),e.tag===3)return n;e=e.return}return[]}function ns(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function Fg(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,u=o.alternate,c=o.stateNode;if(o=o.tag,u!==null&&u===n)break;o!==5&&o!==26&&o!==27||c===null||(u=c,r?(c=lo(a,s),c!=null&&i.unshift(vo(a,c,u))):r||(c=lo(a,s),c!=null&&i.push(vo(a,c,u)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var yE=/\r\n?/g,bE=/\u0000|\uFFFD/g;function Bg(e){return(typeof e=="string"?e:""+e).replace(yE,`
`).replace(bE,"")}function w0(e,t){return t=Bg(t),Bg(e)===t}function Zu(){}function Se(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||Es(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&Es(e,""+n);break;case"className":Pl(e,"class",n);break;case"tabIndex":Pl(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":Pl(e,a,n);break;case"style":wy(e,n,s);break;case"data":if(t!=="object"){Pl(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=Xl(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&Se(e,t,"name",r.name,r,null),Se(e,t,"formEncType",r.formEncType,r,null),Se(e,t,"formMethod",r.formMethod,r,null),Se(e,t,"formTarget",r.formTarget,r,null)):(Se(e,t,"encType",r.encType,r,null),Se(e,t,"method",r.method,r,null),Se(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=Xl(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=Zu);break;case"onScroll":n!=null&&le("scroll",e);break;case"onScrollEnd":n!=null&&le("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(U(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(U(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=Xl(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":le("beforetoggle",e),le("toggle",e),Jl(e,"popover",n);break;case"xlinkActuate":sn(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":sn(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":sn(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":sn(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":sn(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":sn(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":sn(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":sn(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":sn(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":Jl(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=XR.get(a)||a,Jl(e,a,n))}}function ef(e,t,a,n,r,s){switch(a){case"style":wy(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(U(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(U(60));e.innerHTML=a}}break;case"children":typeof n=="string"?Es(e,n):(typeof n=="number"||typeof n=="bigint")&&Es(e,""+n);break;case"onScroll":n!=null&&le("scroll",e);break;case"onScrollEnd":n!=null&&le("scrollend",e);break;case"onClick":n!=null&&(e.onclick=Zu);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!vy.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[jt]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):Jl(e,a,n)}}}function gt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":le("error",e),le("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(U(137,t));default:Se(e,t,s,i,a,null)}}r&&Se(e,t,"srcSet",a.srcSet,a,null),n&&Se(e,t,"src",a.src,a,null);return;case"input":le("invalid",e);var o=s=i=r=null,u=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":u=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(U(137,t));break;default:Se(e,t,n,d,a,null)}}by(e,s,o,u,c,i,r,!1),fu(e);return;case"select":le("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:Se(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?xs(e,!!n,t,!1):a!=null&&xs(e,!!n,a,!0);return;case"textarea":le("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(U(91));break;default:Se(e,t,i,o,a,null)}$y(e,n,r,s),fu(e);return;case"option":for(u in a)if(a.hasOwnProperty(u)&&(n=a[u],n!=null))switch(u){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:Se(e,t,u,n,a,null)}return;case"dialog":le("beforetoggle",e),le("toggle",e),le("cancel",e),le("close",e);break;case"iframe":case"object":le("load",e);break;case"video":case"audio":for(n=0;n<ho.length;n++)le(ho[n],e);break;case"image":le("error",e),le("load",e);break;case"details":le("toggle",e);break;case"embed":case"source":case"link":le("error",e),le("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(U(137,t));default:Se(e,t,c,n,a,null)}return;default:if(gf(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&ef(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&Se(e,t,o,n,a,null))}function xE(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,u=null,c=null,d=null;for(p in a){var m=a[p];if(a.hasOwnProperty(p)&&m!=null)switch(p){case"checked":break;case"value":break;case"defaultValue":u=m;default:n.hasOwnProperty(p)||Se(e,t,p,null,n,m)}}for(var f in n){var p=n[f];if(m=a[f],n.hasOwnProperty(f)&&(p!=null||m!=null))switch(f){case"type":s=p;break;case"name":r=p;break;case"checked":c=p;break;case"defaultChecked":d=p;break;case"value":i=p;break;case"defaultValue":o=p;break;case"children":case"dangerouslySetInnerHTML":if(p!=null)throw Error(U(137,t));break;default:p!==m&&Se(e,t,f,p,n,m)}}Nm(e,i,o,u,c,d,s,r);return;case"select":p=i=o=f=null;for(s in a)if(u=a[s],a.hasOwnProperty(s)&&u!=null)switch(s){case"value":break;case"multiple":p=u;default:n.hasOwnProperty(s)||Se(e,t,s,null,n,u)}for(r in n)if(s=n[r],u=a[r],n.hasOwnProperty(r)&&(s!=null||u!=null))switch(r){case"value":f=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==u&&Se(e,t,r,s,n,u)}t=o,a=i,n=p,f!=null?xs(e,!!a,f,!1):!!n!=!!a&&(t!=null?xs(e,!!a,t,!0):xs(e,!!a,a?[]:"",!1));return;case"textarea":p=f=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:Se(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":f=r;break;case"defaultValue":p=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(U(91));break;default:r!==s&&Se(e,t,i,r,n,s)}xy(e,f,p);return;case"option":for(var x in a)if(f=a[x],a.hasOwnProperty(x)&&f!=null&&!n.hasOwnProperty(x))switch(x){case"selected":e.selected=!1;break;default:Se(e,t,x,null,n,f)}for(u in n)if(f=n[u],p=a[u],n.hasOwnProperty(u)&&f!==p&&(f!=null||p!=null))switch(u){case"selected":e.selected=f&&typeof f!="function"&&typeof f!="symbol";break;default:Se(e,t,u,f,n,p)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var y in a)f=a[y],a.hasOwnProperty(y)&&f!=null&&!n.hasOwnProperty(y)&&Se(e,t,y,null,n,f);for(c in n)if(f=n[c],p=a[c],n.hasOwnProperty(c)&&f!==p&&(f!=null||p!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(f!=null)throw Error(U(137,t));break;default:Se(e,t,c,f,n,p)}return;default:if(gf(t)){for(var w in a)f=a[w],a.hasOwnProperty(w)&&f!==void 0&&!n.hasOwnProperty(w)&&ef(e,t,w,void 0,n,f);for(d in n)f=n[d],p=a[d],!n.hasOwnProperty(d)||f===p||f===void 0&&p===void 0||ef(e,t,d,f,n,p);return}}for(var g in a)f=a[g],a.hasOwnProperty(g)&&f!=null&&!n.hasOwnProperty(g)&&Se(e,t,g,null,n,f);for(m in n)f=n[m],p=a[m],!n.hasOwnProperty(m)||f===p||f==null&&p==null||Se(e,t,m,f,n,p)}var tf=null,af=null;function Mu(e){return e.nodeType===9?e:e.ownerDocument}function zg(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function S0(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function nf(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var pm=null;function $E(){var e=window.event;return e&&e.type==="popstate"?e===pm?!1:(pm=e,!0):(pm=null,!1)}var N0=typeof setTimeout=="function"?setTimeout:void 0,wE=typeof clearTimeout=="function"?clearTimeout:void 0,qg=typeof Promise=="function"?Promise:void 0,SE=typeof queueMicrotask=="function"?queueMicrotask:typeof qg<"u"?function(e){return qg.resolve(null).then(e).catch(NE)}:N0;function NE(e){setTimeout(function(){throw e})}function rr(e){return e==="head"}function Ig(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&io(i.documentElement),a&2&&io(i.body),a&4)for(a=i.head,io(a),i=a.firstChild;i;){var o=i.nextSibling,u=i.nodeName;i[ko]||u==="SCRIPT"||u==="STYLE"||u==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),$o(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);$o(t)}function rf(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":rf(a),vf(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function _E(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[ko])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=_a(e.nextSibling),e===null)break}return null}function kE(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=_a(e.nextSibling),e===null))return null;return e}function sf(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function RE(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function _a(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var of=null;function Kg(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function _0(e,t,a){switch(t=Mu(a),e){case"html":if(e=t.documentElement,!e)throw Error(U(452));return e;case"head":if(e=t.head,!e)throw Error(U(453));return e;case"body":if(e=t.body,!e)throw Error(U(454));return e;default:throw Error(U(451))}}function io(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);vf(e)}var ga=new Map,Hg=new Set;function Ou(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var $n=ve.d;ve.d={f:CE,r:EE,D:TE,C:AE,L:DE,m:ME,X:LE,S:OE,M:PE};function CE(){var e=$n.f(),t=Yu();return e||t}function EE(e){var t=Fs(e);t!==null&&t.tag===5&&t.type==="form"?yb(t):$n.r(e)}var qs=typeof document>"u"?null:document;function k0(e,t,a){var n=qs;if(n&&typeof t=="string"&&t){var r=fa(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),Hg.has(r)||(Hg.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),gt(t,"link",e),ct(t),n.head.appendChild(t)))}}function TE(e){$n.D(e),k0("dns-prefetch",e,null)}function AE(e,t){$n.C(e,t),k0("preconnect",e,t)}function DE(e,t,a){$n.L(e,t,a);var n=qs;if(n&&e&&t){var r='link[rel="preload"][as="'+fa(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+fa(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+fa(a.imageSizes)+'"]')):r+='[href="'+fa(e)+'"]';var s=r;switch(t){case"style":s=Us(e);break;case"script":s=Is(e)}ga.has(s)||(e=Te({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),ga.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(Po(s))||t==="script"&&n.querySelector(Uo(s))||(t=n.createElement("link"),gt(t,"link",e),ct(t),n.head.appendChild(t)))}}function ME(e,t){$n.m(e,t);var a=qs;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+fa(n)+'"][href="'+fa(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=Is(e)}if(!ga.has(s)&&(e=Te({rel:"modulepreload",href:e},t),ga.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(Uo(s)))return}n=a.createElement("link"),gt(n,"link",e),ct(n),a.head.appendChild(n)}}}function OE(e,t,a){$n.S(e,t,a);var n=qs;if(n&&e){var r=bs(n).hoistableStyles,s=Us(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(Po(s)))o.loading=5;else{e=Te({rel:"stylesheet",href:e,"data-precedence":t},a),(a=ga.get(s))&&Wf(e,a);var u=i=n.createElement("link");ct(u),gt(u,"link",e),u._p=new Promise(function(c,d){u.onload=c,u.onerror=d}),u.addEventListener("load",function(){o.loading|=1}),u.addEventListener("error",function(){o.loading|=2}),o.loading|=4,iu(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function LE(e,t){$n.X(e,t);var a=qs;if(a&&e){var n=bs(a).hoistableScripts,r=Is(e),s=n.get(r);s||(s=a.querySelector(Uo(r)),s||(e=Te({src:e,async:!0},t),(t=ga.get(r))&&ep(e,t),s=a.createElement("script"),ct(s),gt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function PE(e,t){$n.M(e,t);var a=qs;if(a&&e){var n=bs(a).hoistableScripts,r=Is(e),s=n.get(r);s||(s=a.querySelector(Uo(r)),s||(e=Te({src:e,async:!0,type:"module"},t),(t=ga.get(r))&&ep(e,t),s=a.createElement("script"),ct(s),gt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function Qg(e,t,a,n){var r=(r=Qn.current)?Ou(r):null;if(!r)throw Error(U(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=Us(a.href),a=bs(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=Us(a.href);var s=bs(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(Po(e)))&&!s._p&&(i.instance=s,i.state.loading=5),ga.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},ga.set(e,a),s||UE(r,e,a,i.state))),t&&n===null)throw Error(U(528,""));return i}if(t&&n!==null)throw Error(U(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=Is(a),a=bs(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(U(444,e))}}function Us(e){return'href="'+fa(e)+'"'}function Po(e){return'link[rel="stylesheet"]['+e+"]"}function R0(e){return Te({},e,{"data-precedence":e.precedence,precedence:null})}function UE(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),gt(t,"link",a),ct(t),e.head.appendChild(t))}function Is(e){return'[src="'+fa(e)+'"]'}function Uo(e){return"script[async]"+e}function Vg(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+fa(a.href)+'"]');if(n)return t.instance=n,ct(n),n;var r=Te({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),ct(n),gt(n,"style",r),iu(n,a.precedence,e),t.instance=n;case"stylesheet":r=Us(a.href);var s=e.querySelector(Po(r));if(s)return t.state.loading|=4,t.instance=s,ct(s),s;n=R0(a),(r=ga.get(r))&&Wf(n,r),s=(e.ownerDocument||e).createElement("link"),ct(s);var i=s;return i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),gt(s,"link",n),t.state.loading|=4,iu(s,a.precedence,e),t.instance=s;case"script":return s=Is(a.src),(r=e.querySelector(Uo(s)))?(t.instance=r,ct(r),r):(n=a,(r=ga.get(s))&&(n=Te({},a),ep(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),ct(r),gt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(U(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,iu(n,a.precedence,e));return t.instance}function iu(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function Wf(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function ep(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var ou=null;function Gg(e,t,a){if(ou===null){var n=new Map,r=ou=new Map;r.set(a,n)}else r=ou,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[ko]||s[wt]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function Yg(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function jE(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function C0(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var go=null;function FE(){}function BE(e,t,a){if(go===null)throw Error(U(475));var n=go;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=Us(a.href),s=e.querySelector(Po(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=Lu.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,ct(s);return}s=e.ownerDocument||e,a=R0(a),(r=ga.get(r))&&Wf(a,r),s=s.createElement("link"),ct(s);var i=s;i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),gt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=Lu.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function zE(){if(go===null)throw Error(U(475));var e=go;return e.stylesheets&&e.count===0&&lf(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&lf(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function Lu(){if(this.count--,this.count===0){if(this.stylesheets)lf(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var Pu=null;function lf(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,Pu=new Map,t.forEach(qE,e),Pu=null,Lu.call(e))}function qE(e,t){if(!(t.state.loading&4)){var a=Pu.get(e);if(a)var n=a.get(null);else{a=new Map,Pu.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=Lu.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var yo={$$typeof:cn,Provider:null,Consumer:null,_currentValue:br,_currentValue2:br,_threadCount:0};function IE(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=Fd(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=Fd(0),this.hiddenUpdates=Fd(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function E0(e,t,a,n,r,s,i,o,u,c,d,m){return e=new IE(e,t,a,i,o,u,c,m),t=1,s===!0&&(t|=24),s=Vt(3,null,null,t),e.current=s,s.stateNode=e,t=Cf(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},Tf(s),e}function T0(e){return e?(e=hs,e):hs}function A0(e,t,a,n,r,s){r=T0(r),n.context===null?n.context=r:n.pendingContext=r,n=Vn(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=Gn(e,n,t),a!==null&&(Xt(a,e,t),Zi(a,e,t))}function Jg(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function tp(e,t){Jg(e,t),(e=e.alternate)&&Jg(e,t)}function D0(e){if(e.tag===13){var t=Bs(e,67108864);t!==null&&Xt(t,e,67108864),tp(e,67108864)}}var Uu=!0;function KE(e,t,a,n){var r=te.T;te.T=null;var s=ve.p;try{ve.p=2,ap(e,t,a,n)}finally{ve.p=s,te.T=r}}function HE(e,t,a,n){var r=te.T;te.T=null;var s=ve.p;try{ve.p=8,ap(e,t,a,n)}finally{ve.p=s,te.T=r}}function ap(e,t,a,n){if(Uu){var r=uf(n);if(r===null)fm(e,t,n,ju,a),Xg(e,n);else if(VE(r,e,t,a,n))n.stopPropagation();else if(Xg(e,n),t&4&&-1<QE.indexOf(e)){for(;r!==null;){var s=Fs(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=vr(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var u=1<<31-Yt(i);o.entanglements[1]|=u,i&=~u}Va(s),(we&6)===0&&(Cu=Ka()+500,Lo(0,!1))}}break;case 13:o=Bs(s,2),o!==null&&Xt(o,s,2),Yu(),tp(s,2)}if(s=uf(n),s===null&&fm(e,t,n,ju,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else fm(e,t,n,null,a)}}function uf(e){return e=yf(e),np(e)}var ju=null;function np(e){if(ju=null,e=us(e),e!==null){var t=wo(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=ny(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return ju=e,null}function M0(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(MR()){case oy:return 2;case ly:return 8;case mu:case OR:return 32;case uy:return 268435456;default:return 32}default:return 32}}var cf=!1,Xn=null,Zn=null,Wn=null,bo=new Map,xo=new Map,zn=[],QE="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function Xg(e,t){switch(e){case"focusin":case"focusout":Xn=null;break;case"dragenter":case"dragleave":Zn=null;break;case"mouseover":case"mouseout":Wn=null;break;case"pointerover":case"pointerout":bo.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":xo.delete(t.pointerId)}}function zi(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Fs(t),t!==null&&D0(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function VE(e,t,a,n,r){switch(t){case"focusin":return Xn=zi(Xn,e,t,a,n,r),!0;case"dragenter":return Zn=zi(Zn,e,t,a,n,r),!0;case"mouseover":return Wn=zi(Wn,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return bo.set(s,zi(bo.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,xo.set(s,zi(xo.get(s)||null,e,t,a,n,r)),!0}return!1}function O0(e){var t=us(e.target);if(t!==null){var a=wo(t);if(a!==null){if(t=a.tag,t===13){if(t=ny(a),t!==null){e.blockedOn=t,qR(e.priority,function(){if(a.tag===13){var n=Jt();n=pf(n);var r=Bs(a,n);r!==null&&Xt(r,a,n),tp(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function lu(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=uf(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);km=n,a.target.dispatchEvent(n),km=null}else return t=Fs(a),t!==null&&D0(t),e.blockedOn=a,!1;t.shift()}return!0}function Zg(e,t,a){lu(e)&&a.delete(t)}function GE(){cf=!1,Xn!==null&&lu(Xn)&&(Xn=null),Zn!==null&&lu(Zn)&&(Zn=null),Wn!==null&&lu(Wn)&&(Wn=null),bo.forEach(Zg),xo.forEach(Zg)}function Gl(e,t){e.blockedOn===t&&(e.blockedOn=null,cf||(cf=!0,st.unstable_scheduleCallback(st.unstable_NormalPriority,GE)))}var Yl=null;function Wg(e){Yl!==e&&(Yl=e,st.unstable_scheduleCallback(st.unstable_NormalPriority,function(){Yl===e&&(Yl=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(np(n||a)===null)continue;break}var s=Fs(a);s!==null&&(e.splice(t,3),t-=3,qm(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function $o(e){function t(u){return Gl(u,e)}Xn!==null&&Gl(Xn,e),Zn!==null&&Gl(Zn,e),Wn!==null&&Gl(Wn,e),bo.forEach(t),xo.forEach(t);for(var a=0;a<zn.length;a++){var n=zn[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<zn.length&&(a=zn[0],a.blockedOn===null);)O0(a),a.blockedOn===null&&zn.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[jt]||null;if(typeof s=="function")i||Wg(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[jt]||null)o=i.formAction;else if(np(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),Wg(a)}}}function rp(e){this._internalRoot=e}Wu.prototype.render=rp.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(U(409));var a=t.current,n=Jt();A0(a,n,e,t,null,null)};Wu.prototype.unmount=rp.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;A0(e.current,2,null,e,null,null),Yu(),t[js]=null}};function Wu(e){this._internalRoot=e}Wu.prototype.unstable_scheduleHydration=function(e){if(e){var t=py();e={blockedOn:null,target:e,priority:t};for(var a=0;a<zn.length&&t!==0&&t<zn[a].priority;a++);zn.splice(a,0,e),a===0&&O0(e)}};var ey=ty.version;if(ey!=="19.1.0")throw Error(U(527,ey,"19.1.0"));ve.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(U(188)):(e=Object.keys(e).join(","),Error(U(268,e)));return e=kR(t),e=e!==null?ry(e):null,e=e===null?null:e.stateNode,e};var YE={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:te,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(qi=__REACT_DEVTOOLS_GLOBAL_HOOK__,!qi.isDisabled&&qi.supportsFiber))try{So=qi.inject(YE),Gt=qi}catch{}var qi;ec.createRoot=function(e,t){if(!ay(e))throw Error(U(299));var a=!1,n="",r=Eb,s=Tb,i=Ab,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=E0(e,1,!1,null,null,a,n,r,s,i,o,null),e[js]=t.current,Zf(e),new rp(t)};ec.hydrateRoot=function(e,t,a){if(!ay(e))throw Error(U(299));var n=!1,r="",s=Eb,i=Tb,o=Ab,u=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(u=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=E0(e,1,!0,t,a??null,n,r,s,i,o,u,c),t.context=T0(null),a=t.current,n=Jt(),n=pf(n),r=Vn(n),r.callback=null,Gn(a,r,n),a=n,t.current.lanes=a,_o(t,a),Va(t),e[js]=t.current,Zf(e),new Wu(t)};ec.version="19.1.0"});var j0=En((w6,U0)=>{"use strict";function P0(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(P0)}catch(e){console.error(e)}}P0(),U0.exports=L0()});var Dt=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var sR={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},iR=class{#t=sR;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},Pa=new iR;function Vh(e){setTimeout(e,0)}var Mt=typeof window>"u"||"Deno"in globalThis;function Ae(){}function Jh(e,t){return typeof e=="function"?e(t):e}function Si(e){return typeof e=="number"&&e>=0&&e!==1/0}function ml(e,t){return Math.max(e+(t||0)-Date.now(),0)}function Sa(e,t){return typeof e=="function"?e(t):e}function Ot(e,t){return typeof e=="function"?e(t):e}function fl(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==Ni(i,t.options))return!1}else if(!fr(t.queryKey,i))return!1}if(a!=="all"){let u=t.isActive();if(a==="active"&&!u||a==="inactive"&&u)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function pl(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(Ua(t.options.mutationKey)!==Ua(s))return!1}else if(!fr(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function Ni(e,t){return(t?.queryKeyHashFn||Ua)(e)}function Ua(e){return JSON.stringify(e,(t,a)=>vd(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function fr(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>fr(e[a],t[a])):!1}var oR=Object.prototype.hasOwnProperty;function _i(e,t){if(e===t)return e;let a=Gh(e)&&Gh(t);if(!a&&!(vd(e)&&vd(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},u=0;for(let c=0;c<i;c++){let d=a?c:s[c],m=e[d],f=t[d];if(m===f){o[d]=m,(a?c<r:oR.call(e,d))&&u++;continue}if(m===null||f===null||typeof m!="object"||typeof f!="object"){o[d]=f;continue}let p=_i(m,f);o[d]=p,p===m&&u++}return r===i&&u===r?e:o}function Tn(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function Gh(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function vd(e){if(!Yh(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!Yh(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function Yh(e){return Object.prototype.toString.call(e)==="[object Object]"}function Xh(e){return new Promise(t=>{Pa.setTimeout(t,e)})}function ki(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?_i(e,t):t}function Zh(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function Wh(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var Gr=Symbol();function hl(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===Gr?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function Ri(e,t){return typeof e=="function"?e(...t):!!e}var lR=class extends Dt{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!Mt&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},Yr=new lR;function Ci(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var ev=Vh;function uR(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=ev,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(u=>{a(u)})})})};return{batch:o=>{let u;t++;try{u=o()}finally{t--,t||i()}return u},batchCalls:o=>(...u)=>{s(()=>{o(...u)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var me=uR();var cR=class extends Dt{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!Mt&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},Jr=new cR;function dR(e){return Math.min(1e3*2**e,3e4)}function gd(e){return(e??"online")==="online"?Jr.isOnline():!0}var vl=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function gl(e){let t=!1,a=0,n,r=Ci(),s=()=>r.status!=="pending",i=y=>{if(!s()){let w=new vl(y);f(w),e.onCancel?.(w)}},o=()=>{t=!0},u=()=>{t=!1},c=()=>Yr.isFocused()&&(e.networkMode==="always"||Jr.isOnline())&&e.canRun(),d=()=>gd(e.networkMode)&&e.canRun(),m=y=>{s()||(n?.(),r.resolve(y))},f=y=>{s()||(n?.(),r.reject(y))},p=()=>new Promise(y=>{n=w=>{(s()||c())&&y(w)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),x=()=>{if(s())return;let y,w=a===0?e.initialPromise:void 0;try{y=w??e.fn()}catch(g){y=Promise.reject(g)}Promise.resolve(y).then(m).catch(g=>{if(s())return;let v=e.retry??(Mt?0:3),b=e.retryDelay??dR,$=typeof b=="function"?b(a,g):b,S=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!S){f(g);return}a++,e.onFail?.(a,g),Xh($).then(()=>c()?void 0:p()).then(()=>{t?f(g):x()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:u,canStart:d,start:()=>(d()?x():p().then(x),r)}}var yl=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),Si(this.gcTime)&&(this.#t=Pa.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(Mt?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(Pa.clearTimeout(this.#t),this.#t=void 0)}};var av=class extends yl{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=tv(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=tv(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=ki(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(Ae).catch(Ae):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>Ot(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===Gr||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>Sa(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!ml(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(u=>u.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=hl(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=gl({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof vl&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,u)=>{this.#i({type:"failed",failureCount:o,error:u})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof vl){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,...yd(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),me.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function yd(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:gd(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function tv(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var pr=class extends Dt{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=Ci(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),nv(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return bd(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return bd(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof Ot(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!Tn(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&rv(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||Ot(this.options.enabled,this.#e)!==Ot(t.enabled,this.#e)||Sa(this.options.staleTime,this.#e)!==Sa(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||Ot(this.options.enabled,this.#e)!==Ot(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return fR(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(Ae)),t}#v(){this.#x();let e=Sa(this.options.staleTime,this.#e);if(Mt||this.#n.isStale||!Si(e))return;let a=ml(this.#n.dataUpdatedAt,e)+1;this.#u=Pa.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(Mt||Ot(this.options.enabled,this.#e)===!1||!Si(this.#l)||this.#l===0)&&(this.#c=Pa.setInterval(()=>{(this.options.refetchIntervalInBackground||Yr.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(Pa.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(Pa.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,u=e!==a?e.state:this.#a,{state:c}=e,d={...c},m=!1,f;if(t._optimisticResults){let T=this.hasListeners(),L=!T&&nv(e,t),D=T&&rv(e,a,t,n);(L||D)&&(d={...d,...yd(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:p,errorUpdatedAt:x,status:y}=d;f=d.data;let w=!1;if(t.placeholderData!==void 0&&f===void 0&&y==="pending"){let T;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(T=r.data,w=!0):T=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,T!==void 0&&(y="success",f=ki(r?.data,T,t),m=!0)}if(t.select&&f!==void 0&&!w)if(r&&f===s?.data&&t.select===this.#f)f=this.#d;else try{this.#f=t.select,f=t.select(f),f=ki(r?.data,f,t),this.#d=f,this.#i=null}catch(T){this.#i=T}this.#i&&(p=this.#i,f=this.#d,x=Date.now(),y="error");let g=d.fetchStatus==="fetching",v=y==="pending",b=y==="error",$=v&&g,S=f!==void 0,_={status:y,fetchStatus:d.fetchStatus,isPending:v,isSuccess:y==="success",isError:b,isInitialLoading:$,isLoading:$,data:f,dataUpdatedAt:d.dataUpdatedAt,error:p,errorUpdatedAt:x,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>u.dataUpdateCount||d.errorUpdateCount>u.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:b&&!S,isPaused:d.fetchStatus==="paused",isPlaceholderData:m,isRefetchError:b&&S,isStale:xd(e,t),refetch:this.refetch,promise:this.#o,isEnabled:Ot(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let T=P=>{_.status==="error"?P.reject(_.error):_.data!==void 0&&P.resolve(_.data)},L=()=>{let P=this.#o=_.promise=Ci();T(P)},D=this.#o;switch(D.status){case"pending":e.queryHash===a.queryHash&&T(D);break;case"fulfilled":(_.status==="error"||_.data!==D.value)&&L();break;case"rejected":(_.status!=="error"||_.error!==D.reason)&&L();break}}return _}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),Tn(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){me.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function mR(e,t){return Ot(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function nv(e,t){return mR(e,t)||e.state.data!==void 0&&bd(e,t,t.refetchOnMount)}function bd(e,t,a){if(Ot(t.enabled,e)!==!1&&Sa(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&xd(e,t)}return!1}function rv(e,t,a,n){return(e!==t||Ot(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&xd(e,a)}function xd(e,t){return Ot(t.enabled,e)!==!1&&e.isStaleByTime(Sa(t.staleTime,e))}function fR(e,t){return!Tn(e.getCurrentResult(),t)}function $d(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},u=0,c=async()=>{let d=!1,m=x=>{Object.defineProperty(x,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},f=hl(t.options,t.fetchOptions),p=async(x,y,w)=>{if(d)return Promise.reject();if(y==null&&x.pages.length)return Promise.resolve(x);let v=(()=>{let E={client:t.client,queryKey:t.queryKey,pageParam:y,direction:w?"backward":"forward",meta:t.options.meta};return m(E),E})(),b=await f(v),{maxPages:$}=t.options,S=w?Wh:Zh;return{pages:S(x.pages,b,$),pageParams:S(x.pageParams,y,$)}};if(r&&s.length){let x=r==="backward",y=x?pR:sv,w={pages:s,pageParams:i},g=y(n,w);o=await p(w,g,x)}else{let x=e??s.length;do{let y=u===0?i[0]??n.initialPageParam:sv(n,o);if(u>0&&y==null)break;o=await p(o,y),u++}while(u<x)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function sv(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function pR(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var iv=class extends yl{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||wd(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=gl({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),me.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function wd(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var ov=class extends Dt{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new iv({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=bl(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=bl(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=bl(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=bl(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){me.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>pl(t,a))}findAll(e={}){return this.getAll().filter(t=>pl(e,t))}notify(e){me.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return me.batch(()=>Promise.all(e.map(t=>t.continue().catch(Ae))))}};function bl(e){return e.options.scope?.id}var Sd=class extends Dt{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),Tn(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&Ua(t.mutationKey)!==Ua(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??wd();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){me.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function lv(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function hR(e,t,a){let n=e.slice(0);return n[t]=a,n}var Nd=class extends Dt{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,me.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,m)=>d!==a[m]),u=i||o,c=u?!0:s.some((d,m)=>{let f=this.#e[m];return!f||!Tn(d,f)});!u&&!c||(u&&(this.#r=r),this.#e=s,this.hasListeners()&&(u&&(lv(a,r).forEach(d=>{d.destroy()}),lv(r,a).forEach(d=>{d.subscribe(m=>{this.#c(d,m)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=_i(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new pr(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=hR(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&me.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var uv=class extends Dt{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??Ni(n,t),s=this.get(r);return s||(s=new av({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){me.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>fl(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>fl(e,a)):t}notify(e){me.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){me.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){me.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var _d=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new uv,this.#e=e.mutationCache||new ov,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=Yr.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=Jr.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime(Sa(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=Jh(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return me.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;me.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return me.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=me.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(Ae).catch(Ae)}invalidateQueries(e,t={}){return me.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=me.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(Ae)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(Ae)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime(Sa(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(Ae).catch(Ae)}fetchInfiniteQuery(e){return e.behavior=$d(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(Ae).catch(Ae)}ensureInfiniteQueryData(e){return e.behavior=$d(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return Jr.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(Ua(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{fr(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(Ua(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{fr(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=Ni(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===Gr&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var ja=ze(Qe(),1);var Xr=ze(Qe(),1),fv=ze(kd(),1),Rd=Xr.createContext(void 0),J=e=>{let t=Xr.useContext(Rd);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},Cd=({client:e,children:t})=>(Xr.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,fv.jsx)(Rd.Provider,{value:e,children:t}));var $l=ze(Qe(),1),pv=$l.createContext(!1),wl=()=>$l.useContext(pv),jL=pv.Provider;var Ei=ze(Qe(),1),yR=ze(kd(),1);function bR(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var xR=Ei.createContext(bR()),Sl=()=>Ei.useContext(xR);var hv=ze(Qe(),1);var Nl=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},_l=e=>{hv.useEffect(()=>{e.clearReset()},[e])},kl=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||Ri(a,[e.error,n]));var Rl=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},Cl=(e,t)=>e.isLoading&&e.isFetching&&!t,Ti=(e,t)=>e?.suspense&&t.isPending,Zr=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function Ed({queries:e,...t},a){let n=J(a),r=wl(),s=Sl(),i=ja.useMemo(()=>e.map(y=>{let w=n.defaultQueryOptions(y);return w._optimisticResults=r?"isRestoring":"optimistic",w}),[e,n,r]);i.forEach(y=>{Rl(y),Nl(y,s)}),_l(s);let[o]=ja.useState(()=>new Nd(n,i,t)),[u,c,d]=o.getOptimisticResult(i,t.combine),m=!r&&t.subscribed!==!1;ja.useSyncExternalStore(ja.useCallback(y=>m?o.subscribe(me.batchCalls(y)):Ae,[o,m]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),ja.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let p=u.some((y,w)=>Ti(i[w],y))?u.flatMap((y,w)=>{let g=i[w];if(g){let v=new pr(n,g);if(Ti(g,y))return Zr(g,v,s);Cl(y,r)&&Zr(g,v,s)}return[]}):[];if(p.length>0)throw Promise.all(p);let x=u.find((y,w)=>{let g=i[w];return g&&kl({result:y,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(x?.error)throw x.error;return c(d())}var An=ze(Qe(),1);function vv(e,t,a){let n=wl(),r=Sl(),s=J(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",Rl(i),Nl(i,r),_l(r);let o=!s.getQueryCache().get(i.queryHash),[u]=An.useState(()=>new t(s,i)),c=u.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(An.useSyncExternalStore(An.useCallback(m=>{let f=d?u.subscribe(me.batchCalls(m)):Ae;return u.updateResult(),f},[u,d]),()=>u.getCurrentResult(),()=>u.getCurrentResult()),An.useEffect(()=>{u.setOptions(i)},[i,u]),Ti(i,c))throw Zr(i,u,r);if(kl({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!Mt&&Cl(c,n)&&(o?Zr(i,u,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(Ae).finally(()=>{u.updateResult()}),i.notifyOnChangeProps?c:u.trackResult(c)}function I(e,t){return vv(e,pr,t)}var nn=ze(Qe(),1);function Q(e,t){let a=J(t),[n]=nn.useState(()=>new Sd(a,e));nn.useEffect(()=>{n.setOptions(e)},[n,e]);let r=nn.useSyncExternalStore(nn.useCallback(i=>n.subscribe(me.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=nn.useCallback((i,o)=>{n.mutate(i,o).catch(Ae)},[n]);if(r.error&&Ri(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var rR=ze(j0());var ea=ze(Qe(),1),Z=ze(Qe(),1),Ce=ze(Qe(),1),Sp=ze(Qe(),1),ox=ze(Qe(),1),ge=ze(Qe(),1),J3=ze(Qe(),1),X3=ze(Qe(),1),Z3=ze(Qe(),1),W=ze(Qe(),1),wx=ze(Qe(),1);var F0="popstate";function K0(e={}){function t(n,r){let{pathname:s,search:i,hash:o}=n.location;return op("",{pathname:s,search:i,hash:o},r.state&&r.state.usr||null,r.state&&r.state.key||"default")}function a(n,r){return typeof r=="string"?r:Ks(r)}return XE(t,a,null,e)}function Re(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function Wt(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function JE(){return Math.random().toString(36).substring(2,10)}function B0(e,t){return{usr:e.state,key:e.key,idx:t}}function op(e,t,a=null,n){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?Or(t):t,state:a,key:t&&t.key||n||JE()}}function Ks({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function Or(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function XE(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",u=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function m(){o="POP";let w=d(),g=w==null?null:w-c;c=w,u&&u({action:o,location:y.location,delta:g})}function f(w,g){o="PUSH";let v=op(y.location,w,g);a&&a(v,w),c=d()+1;let b=B0(v,c),$=y.createHref(v);try{i.pushState(b,"",$)}catch(S){if(S instanceof DOMException&&S.name==="DataCloneError")throw S;r.location.assign($)}s&&u&&u({action:o,location:y.location,delta:1})}function p(w,g){o="REPLACE";let v=op(y.location,w,g);a&&a(v,w),c=d();let b=B0(v,c),$=y.createHref(v);i.replaceState(b,"",$),s&&u&&u({action:o,location:y.location,delta:0})}function x(w){return ZE(w)}let y={get action(){return o},get location(){return e(r,i)},listen(w){if(u)throw new Error("A history only accepts one active listener");return r.addEventListener(F0,m),u=w,()=>{r.removeEventListener(F0,m),u=null}},createHref(w){return t(r,w)},createURL:x,encodeLocation(w){let g=x(w);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:f,replace:p,go(w){return i.go(w)}};return y}function ZE(e,t=!1){let a="http://localhost";typeof window<"u"&&(a=window.location.origin!=="null"?window.location.origin:window.location.href),Re(a,"No window.location.(origin|href) available to create URL");let n=typeof e=="string"?e:Ks(e);return n=n.replace(/ $/,"%20"),!t&&n.startsWith("//")&&(n=a+n),new URL(n,a)}var WE;WE=new WeakMap;function dp(e,t,a="/"){return e3(e,t,a,!1)}function e3(e,t,a,n){let r=typeof t=="string"?Or(t):t,s=Ga(r.pathname||"/",a);if(s==null)return null;let i=H0(e);a3(i);let o=null;for(let u=0;o==null&&u<i.length;++u){let c=f3(s);o=d3(i[u],c,n)}return o}function t3(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function H0(e,t=[],a=[],n="",r=!1){let s=(i,o,u=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&u)return;Re(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let m=wn([n,d.relativePath]),f=a.concat(d);i.children&&i.children.length>0&&(Re(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${m}".`),H0(i.children,t,f,m,u)),!(i.path==null&&!i.index)&&t.push({path:m,score:u3(m,i.index),routesMeta:f})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let u of Q0(i.path))s(i,o,!0,u)}),t}function Q0(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=Q0(n.join("/")),o=[];return o.push(...i.map(u=>u===""?s:[s,u].join("/"))),r&&o.push(...i),o.map(u=>e.startsWith("/")&&u===""?"/":u)}function a3(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:c3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var n3=/^:[\w-]+$/,r3=3,s3=2,i3=1,o3=10,l3=-2,z0=e=>e==="*";function u3(e,t){let a=e.split("/"),n=a.length;return a.some(z0)&&(n+=l3),t&&(n+=s3),a.filter(r=>!z0(r)).reduce((r,s)=>r+(n3.test(s)?r3:s===""?i3:o3),n)}function c3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function d3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let u=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",m=Fo({path:u.relativePath,caseSensitive:u.caseSensitive,end:c},d),f=u.route;if(!m&&c&&a&&!n[n.length-1].route.index&&(m=Fo({path:u.relativePath,caseSensitive:u.caseSensitive,end:!1},d)),!m)return null;Object.assign(r,m.params),i.push({params:r,pathname:wn([s,m.pathname]),pathnameBase:v3(wn([s,m.pathnameBase])),route:f}),m.pathnameBase!=="/"&&(s=wn([s,m.pathnameBase]))}return i}function Fo(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=m3(e.path,e.caseSensitive,e.end),r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:m},f)=>{if(d==="*"){let x=o[f]||"";i=s.slice(0,s.length-x.length).replace(/(.)\/+$/,"$1")}let p=o[f];return m&&!p?c[d]=void 0:c[d]=(p||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function m3(e,t=!1,a=!0){Wt(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,u)=>(n.push({paramName:o,isOptional:u!=null}),u?"/?([^\\/]+)?":"/([^\\/]+)")).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function f3(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return Wt(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function Ga(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}function V0(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?Or(e):e;return{pathname:a?a.startsWith("/")?a:p3(a,t):t,search:g3(n),hash:y3(r)}}function p3(e,t){let a=t.replace(/\/+$/,"").split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function sp(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function h3(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function mp(e){let t=h3(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function fp(e,t,a,n=!1){let r;typeof e=="string"?r=Or(e):(r={...e},Re(!r.pathname||!r.pathname.includes("?"),sp("?","pathname","search",r)),Re(!r.pathname||!r.pathname.includes("#"),sp("#","pathname","hash",r)),Re(!r.search||!r.search.includes("#"),sp("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let m=t.length-1;if(!n&&i.startsWith("..")){let f=i.split("/");for(;f[0]==="..";)f.shift(),m-=1;r.pathname=f.join("/")}o=m>=0?t[m]:"/"}let u=V0(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!u.pathname.endsWith("/")&&(c||d)&&(u.pathname+="/"),u}var wn=e=>e.join("/").replace(/\/\/+/g,"/"),v3=e=>e.replace(/\/+$/,"").replace(/^\/*/,"/"),g3=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,y3=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;function G0(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}var Y0=["POST","PUT","PATCH","DELETE"],S6=new Set(Y0),b3=["GET",...Y0],N6=new Set(b3);var _6=Symbol("ResetLoaderData");var Lr=ea.createContext(null);Lr.displayName="DataRouter";var Hs=ea.createContext(null);Hs.displayName="DataRouterState";var k6=ea.createContext(!1);var pp=ea.createContext({isTransitioning:!1});pp.displayName="ViewTransition";var J0=ea.createContext(new Map);J0.displayName="Fetchers";var x3=ea.createContext(null);x3.displayName="Await";var Bt=ea.createContext(null);Bt.displayName="Navigation";var Qs=ea.createContext(null);Qs.displayName="Location";var ta=ea.createContext({outlet:null,matches:[],isDataRoute:!1});ta.displayName="Route";var hp=ea.createContext(null);hp.displayName="RouteError";var lp=!0;function X0(e,{relative:t}={}){Re(Pr(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=Z.useContext(Bt),{hash:r,pathname:s,search:i}=Vs(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:wn([a,s])),n.createHref({pathname:o,search:i,hash:r})}function Pr(){return Z.useContext(Qs)!=null}function Le(){return Re(Pr(),"useLocation() may be used only in the context of a <Router> component."),Z.useContext(Qs).location}var Z0="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function W0(e){Z.useContext(Bt).static||Z.useLayoutEffect(e)}function pe(){let{isDataRoute:e}=Z.useContext(ta);return e?T3():$3()}function $3(){Re(Pr(),"useNavigate() may be used only in the context of a <Router> component.");let e=Z.useContext(Lr),{basename:t,navigator:a}=Z.useContext(Bt),{matches:n}=Z.useContext(ta),{pathname:r}=Le(),s=JSON.stringify(mp(n)),i=Z.useRef(!1);return W0(()=>{i.current=!0}),Z.useCallback((u,c={})=>{if(Wt(i.current,Z0),!i.current)return;if(typeof u=="number"){a.go(u);return}let d=fp(u,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:wn([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var ex=Z.createContext(null);function ya(){return Z.useContext(ex)}function tx(e){let t=Z.useContext(ta).outlet;return t&&Z.createElement(ex.Provider,{value:e},t)}function it(){let{matches:e}=Z.useContext(ta),t=e[e.length-1];return t?t.params:{}}function Vs(e,{relative:t}={}){let{matches:a}=Z.useContext(ta),{pathname:n}=Le(),r=JSON.stringify(mp(a));return Z.useMemo(()=>fp(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function ax(e,t){return nx(e,t)}function nx(e,t,a,n,r){Re(Pr(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:s}=Z.useContext(Bt),{matches:i}=Z.useContext(ta),o=i[i.length-1],u=o?o.params:{},c=o?o.pathname:"/",d=o?o.pathnameBase:"/",m=o&&o.route;if(lp){let v=m&&m.path||"";ix(c,!m||v.endsWith("*")||v.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${c}" (under <Route path="${v}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${v}"> to <Route path="${v==="/"?"*":`${v}/*`}">.`)}let f=Le(),p;if(t){let v=typeof t=="string"?Or(t):t;Re(d==="/"||v.pathname?.startsWith(d),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${d}" but pathname "${v.pathname}" was given in the \`location\` prop.`),p=v}else p=f;let x=p.pathname||"/",y=x;if(d!=="/"){let v=d.replace(/^\//,"").split("/");y="/"+x.replace(/^\//,"").split("/").slice(v.length).join("/")}let w=dp(e,{pathname:y});lp&&(Wt(m||w!=null,`No routes matched location "${p.pathname}${p.search}${p.hash}" `),Wt(w==null||w[w.length-1].route.element!==void 0||w[w.length-1].route.Component!==void 0||w[w.length-1].route.lazy!==void 0,`Matched leaf route at location "${p.pathname}${p.search}${p.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let g=k3(w&&w.map(v=>Object.assign({},v,{params:Object.assign({},u,v.params),pathname:wn([d,s.encodeLocation?s.encodeLocation(v.pathname).pathname:v.pathname]),pathnameBase:v.pathnameBase==="/"?d:wn([d,s.encodeLocation?s.encodeLocation(v.pathnameBase).pathname:v.pathnameBase])})),i,a,n,r);return t&&g?Z.createElement(Qs.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",...p},navigationType:"POP"}},g):g}function w3(){let e=sx(),t=G0(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return lp&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=Z.createElement(Z.Fragment,null,Z.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),Z.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",Z.createElement("code",{style:s},"ErrorBoundary")," or"," ",Z.createElement("code",{style:s},"errorElement")," prop on your route."))),Z.createElement(Z.Fragment,null,Z.createElement("h2",null,"Unexpected Application Error!"),Z.createElement("h3",{style:{fontStyle:"italic"}},t),a?Z.createElement("pre",{style:r},a):null,i)}var S3=Z.createElement(w3,null),N3=class extends Z.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.unstable_onError?this.props.unstable_onError(e,t):console.error("React Router caught the following error during render",e)}render(){return this.state.error!==void 0?Z.createElement(ta.Provider,{value:this.props.routeContext},Z.createElement(hp.Provider,{value:this.state.error,children:this.props.component})):this.props.children}};function _3({routeContext:e,match:t,children:a}){let n=Z.useContext(Lr);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),Z.createElement(ta.Provider,{value:e},a)}function k3(e,t=[],a=null,n=null,r=null){if(e==null){if(!a)return null;if(a.errors)e=a.matches;else if(t.length===0&&!a.initialized&&a.matches.length>0)e=a.matches;else return null}let s=e,i=a?.errors;if(i!=null){let c=s.findIndex(d=>d.route.id&&i?.[d.route.id]!==void 0);Re(c>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(i).join(",")}`),s=s.slice(0,Math.min(s.length,c+1))}let o=!1,u=-1;if(a)for(let c=0;c<s.length;c++){let d=s[c];if((d.route.HydrateFallback||d.route.hydrateFallbackElement)&&(u=c),d.route.id){let{loaderData:m,errors:f}=a,p=d.route.loader&&!m.hasOwnProperty(d.route.id)&&(!f||f[d.route.id]===void 0);if(d.route.lazy||p){o=!0,u>=0?s=s.slice(0,u+1):s=[s[0]];break}}}return s.reduceRight((c,d,m)=>{let f,p=!1,x=null,y=null;a&&(f=i&&d.route.id?i[d.route.id]:void 0,x=d.route.errorElement||S3,o&&(u<0&&m===0?(ix("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),p=!0,y=null):u===m&&(p=!0,y=d.route.hydrateFallbackElement||null)));let w=t.concat(s.slice(0,m+1)),g=()=>{let v;return f?v=x:p?v=y:d.route.Component?v=Z.createElement(d.route.Component,null):d.route.element?v=d.route.element:v=c,Z.createElement(_3,{match:d,routeContext:{outlet:c,matches:w,isDataRoute:a!=null},children:v})};return a&&(d.route.ErrorBoundary||d.route.errorElement||m===0)?Z.createElement(N3,{location:a.location,revalidation:a.revalidation,component:x,error:f,children:g(),routeContext:{outlet:null,matches:w,isDataRoute:!0},unstable_onError:n}):g()},null)}function vp(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function R3(e){let t=Z.useContext(Lr);return Re(t,vp(e)),t}function gp(e){let t=Z.useContext(Hs);return Re(t,vp(e)),t}function C3(e){let t=Z.useContext(ta);return Re(t,vp(e)),t}function yp(e){let t=C3(e),a=t.matches[t.matches.length-1];return Re(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function E3(){return yp("useRouteId")}function rx(){return gp("useNavigation").navigation}function bp(){let{matches:e,loaderData:t}=gp("useMatches");return Z.useMemo(()=>e.map(a=>t3(a,t)),[e,t])}function sx(){let e=Z.useContext(hp),t=gp("useRouteError"),a=yp("useRouteError");return e!==void 0?e:t.errors?.[a]}function T3(){let{router:e}=R3("useNavigate"),t=yp("useNavigate"),a=Z.useRef(!1);return W0(()=>{a.current=!0}),Z.useCallback(async(r,s={})=>{Wt(a.current,Z0),a.current&&(typeof r=="number"?e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var q0={};function ix(e,t,a){!t&&!q0[e]&&(q0[e]=!0,Wt(!1,a))}var R6=Ce.memo(A3);function A3({routes:e,future:t,state:a,unstable_onError:n}){return nx(e,void 0,a,n,t)}function ot({to:e,replace:t,state:a,relative:n}){Re(Pr(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=Ce.useContext(Bt);Wt(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=Ce.useContext(ta),{pathname:i}=Le(),o=pe(),u=fp(e,mp(s),i,n==="path"),c=JSON.stringify(u);return Ce.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function xp(e){return tx(e.context)}function ye(e){Re(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function $p({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1}){Re(!Pr(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let i=e.replace(/^\/*/,"/"),o=Ce.useMemo(()=>({basename:i,navigator:r,static:s,future:{}}),[i,r,s]);typeof a=="string"&&(a=Or(a));let{pathname:u="/",search:c="",hash:d="",state:m=null,key:f="default"}=a,p=Ce.useMemo(()=>{let x=Ga(u,i);return x==null?null:{location:{pathname:x,search:c,hash:d,state:m,key:f},navigationType:n}},[i,u,c,d,m,f,n]);return Wt(p!=null,`<Router basename="${i}"> is not able to match the URL "${u}${c}${d}" because it does not start with the basename, so the <Router> won't render anything.`),p==null?null:Ce.createElement(Bt.Provider,{value:o},Ce.createElement(Qs.Provider,{children:t,value:p}))}function wp({children:e,location:t}){return ax(sc(e),t)}function sc(e,t=[]){let a=[];return Ce.Children.forEach(e,(n,r)=>{if(!Ce.isValidElement(n))return;let s=[...t,r];if(n.type===Ce.Fragment){a.push.apply(a,sc(n.props.children,s));return}Re(n.type===ye,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),Re(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=sc(n.props.children,s)),a.push(i)}),a}var nc="get",rc="application/x-www-form-urlencoded";function ic(e){return e!=null&&typeof e.tagName=="string"}function D3(e){return ic(e)&&e.tagName.toLowerCase()==="button"}function M3(e){return ic(e)&&e.tagName.toLowerCase()==="form"}function O3(e){return ic(e)&&e.tagName.toLowerCase()==="input"}function L3(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function P3(e,t){return e.button===0&&(!t||t==="_self")&&!L3(e)}var tc=null;function U3(){if(tc===null)try{new FormData(document.createElement("form"),0),tc=!1}catch{tc=!0}return tc}var j3=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function ip(e){return e!=null&&!j3.has(e)?(Wt(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${rc}"`),null):e}function F3(e,t){let a,n,r,s,i;if(M3(e)){let o=e.getAttribute("action");n=o?Ga(o,t):null,a=e.getAttribute("method")||nc,r=ip(e.getAttribute("enctype"))||rc,s=new FormData(e)}else if(D3(e)||O3(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let u=e.getAttribute("formaction")||o.getAttribute("action");if(n=u?Ga(u,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||nc,r=ip(e.getAttribute("formenctype"))||ip(o.getAttribute("enctype"))||rc,s=new FormData(o,e),!U3()){let{name:c,type:d,value:m}=e;if(d==="image"){let f=c?`${c}.`:"";s.append(`${f}x`,"0"),s.append(`${f}y`,"0")}else c&&s.append(c,m)}}else{if(ic(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=nc,n=null,r=rc,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var C6=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");function Np(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var B3=Symbol("SingleFetchRedirect");function z3(e,t,a){let n=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return n.pathname==="/"?n.pathname=`_root.${a}`:t&&Ga(n.pathname,t)==="/"?n.pathname=`${t.replace(/\/$/,"")}/_root.${a}`:n.pathname=`${n.pathname.replace(/\/$/,"")}.${a}`,n}async function q3(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function I3(e){return e!=null&&typeof e.page=="string"}function K3(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function H3(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await q3(s,a);return i.links?i.links():[]}return[]}));return Y3(n.flat(1).filter(K3).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function I0(e,t,a,n,r,s){let i=(u,c)=>a[c]?u.route.id!==a[c].route.id:!0,o=(u,c)=>a[c].pathname!==u.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==u.params["*"];return s==="assets"?t.filter((u,c)=>i(u,c)||o(u,c)):s==="data"?t.filter((u,c)=>{let d=n.routes[u.route.id];if(!d||!d.hasLoader)return!1;if(i(u,c)||o(u,c))return!0;if(u.route.shouldRevalidate){let m=u.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:u.params,defaultShouldRevalidate:!0});if(typeof m=="boolean")return m}return!0}):[]}function Q3(e,t,{includeHydrateFallback:a}={}){return V3(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function V3(e){return[...new Set(e)]}function G3(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function Y3(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!I3(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(G3(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function lx(){let e=ge.useContext(Lr);return Np(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function W3(){let e=ge.useContext(Hs);return Np(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var Bo=ge.createContext(void 0);Bo.displayName="FrameworkContext";function ux(){let e=ge.useContext(Bo);return Np(e,"You must render this element inside a <HydratedRouter> element"),e}function eT(e,t){let a=ge.useContext(Bo),[n,r]=ge.useState(!1),[s,i]=ge.useState(!1),{onFocus:o,onBlur:u,onMouseEnter:c,onMouseLeave:d,onTouchStart:m}=t,f=ge.useRef(null);ge.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let y=g=>{g.forEach(v=>{i(v.isIntersecting)})},w=new IntersectionObserver(y,{threshold:.5});return f.current&&w.observe(f.current),()=>{w.disconnect()}}},[e]),ge.useEffect(()=>{if(n){let y=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(y)}}},[n]);let p=()=>{r(!0)},x=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,f,{}]:[s,f,{onFocus:jo(o,p),onBlur:jo(u,x),onMouseEnter:jo(c,p),onMouseLeave:jo(d,x),onTouchStart:jo(m,p)}]:[!1,f,{}]}function jo(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function cx({page:e,...t}){let{router:a}=lx(),n=ge.useMemo(()=>dp(a.routes,e,a.basename),[a.routes,e,a.basename]);return n?ge.createElement(aT,{page:e,matches:n,...t}):null}function tT(e){let{manifest:t,routeModules:a}=ux(),[n,r]=ge.useState([]);return ge.useEffect(()=>{let s=!1;return H3(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function aT({page:e,matches:t,...a}){let n=Le(),{manifest:r,routeModules:s}=ux(),{basename:i}=lx(),{loaderData:o,matches:u}=W3(),c=ge.useMemo(()=>I0(e,t,u,r,n,"data"),[e,t,u,r,n]),d=ge.useMemo(()=>I0(e,t,u,r,n,"assets"),[e,t,u,r,n]),m=ge.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let x=new Set,y=!1;if(t.forEach(g=>{let v=r.routes[g.route.id];!v||!v.hasLoader||(!c.some(b=>b.route.id===g.route.id)&&g.route.id in o&&s[g.route.id]?.shouldRevalidate||v.hasClientLoader?y=!0:x.add(g.route.id))}),x.size===0)return[];let w=z3(e,i,"data");return y&&x.size>0&&w.searchParams.set("_routes",t.filter(g=>x.has(g.route.id)).map(g=>g.route.id).join(",")),[w.pathname+w.search]},[i,o,n,r,c,t,e,s]),f=ge.useMemo(()=>Q3(d,r),[d,r]),p=tT(d);return ge.createElement(ge.Fragment,null,m.map(x=>ge.createElement("link",{key:x,rel:"prefetch",as:"fetch",href:x,...a})),f.map(x=>ge.createElement("link",{key:x,rel:"modulepreload",href:x,...a})),p.map(({key:x,link:y})=>ge.createElement("link",{key:x,nonce:a.nonce,...y})))}function nT(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var dx=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{dx&&(window.__reactRouterVersion="7.9.1")}catch{}function _p({basename:e,children:t,window:a}){let n=W.useRef();n.current==null&&(n.current=K0({window:a,v5Compat:!0}));let r=n.current,[s,i]=W.useState({action:r.action,location:r.location}),o=W.useCallback(u=>{W.startTransition(()=>i(u))},[i]);return W.useLayoutEffect(()=>r.listen(o),[r,o]),W.createElement($p,{basename:e,children:t,location:s.location,navigationType:s.action,navigator:r})}function mx({basename:e,children:t,history:a}){let[n,r]=W.useState({action:a.action,location:a.location}),s=W.useCallback(i=>{W.startTransition(()=>r(i))},[r]);return W.useLayoutEffect(()=>a.listen(s),[a,s]),W.createElement($p,{basename:e,children:t,location:n.location,navigationType:n.action,navigator:a})}mx.displayName="unstable_HistoryRouter";var fx=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i,Sn=W.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,state:o,target:u,to:c,preventScrollReset:d,viewTransition:m,...f},p){let{basename:x}=W.useContext(Bt),y=typeof c=="string"&&fx.test(c),w,g=!1;if(typeof c=="string"&&y&&(w=c,dx))try{let L=new URL(window.location.href),D=c.startsWith("//")?new URL(L.protocol+c):new URL(c),P=Ga(D.pathname,x);D.origin===L.origin&&P!=null?c=P+D.search+D.hash:g=!0}catch{Wt(!1,`<Link to="${c}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}let v=X0(c,{relative:r}),[b,$,S]=eT(n,f),E=gx(c,{replace:i,state:o,target:u,preventScrollReset:d,relative:r,viewTransition:m});function _(L){t&&t(L),L.defaultPrevented||E(L)}let T=W.createElement("a",{...f,...S,href:w||v,onClick:g||s?t:_,ref:nT(p,$),target:u,"data-discover":!y&&a==="render"?"true":void 0});return b&&!y?W.createElement(W.Fragment,null,T,W.createElement(cx,{page:v})):T});Sn.displayName="Link";var Ya=W.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:u,...c},d){let m=Vs(i,{relative:c.relative}),f=Le(),p=W.useContext(Hs),{navigator:x,basename:y}=W.useContext(Bt),w=p!=null&&$x(m)&&o===!0,g=x.encodeLocation?x.encodeLocation(m).pathname:m.pathname,v=f.pathname,b=p&&p.navigation&&p.navigation.location?p.navigation.location.pathname:null;a||(v=v.toLowerCase(),b=b?b.toLowerCase():null,g=g.toLowerCase()),b&&y&&(b=Ga(b,y)||b);let $=g!=="/"&&g.endsWith("/")?g.length-1:g.length,S=v===g||!r&&v.startsWith(g)&&v.charAt($)==="/",E=b!=null&&(b===g||!r&&b.startsWith(g)&&b.charAt(g.length)==="/"),_={isActive:S,isPending:E,isTransitioning:w},T=S?t:void 0,L;typeof n=="function"?L=n(_):L=[n,S?"active":null,E?"pending":null,w?"transitioning":null].filter(Boolean).join(" ");let D=typeof s=="function"?s(_):s;return W.createElement(Sn,{...c,"aria-current":T,className:L,ref:d,style:D,to:i,viewTransition:o},typeof u=="function"?u(_):u)});Ya.displayName="NavLink";var px=W.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=nc,action:o,onSubmit:u,relative:c,preventScrollReset:d,viewTransition:m,...f},p)=>{let x=yx(),y=bx(o,{relative:c}),w=i.toLowerCase()==="get"?"get":"post",g=typeof o=="string"&&fx.test(o);return W.createElement("form",{ref:p,method:w,action:y,onSubmit:n?u:b=>{if(u&&u(b),b.defaultPrevented)return;b.preventDefault();let $=b.nativeEvent.submitter,S=$?.getAttribute("formmethod")||i;x($||b.currentTarget,{fetcherKey:t,method:S,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:m})},...f,"data-discover":!g&&e==="render"?"true":void 0})});px.displayName="Form";function hx({getKey:e,storageKey:t,...a}){let n=W.useContext(Bo),{basename:r}=W.useContext(Bt),s=Le(),i=bp();xx({getKey:e,storageKey:t});let o=W.useMemo(()=>{if(!n||!e)return null;let c=cp(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let u=((c,d)=>{if(!window.history.state||!window.history.state.key){let m=Math.random().toString(32).slice(2);window.history.replaceState({key:m},"")}try{let f=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof f=="number"&&window.scrollTo(0,f)}catch(m){console.error(m),sessionStorage.removeItem(c)}}).toString();return W.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${u})(${JSON.stringify(t||up)}, ${JSON.stringify(o)})`}})}hx.displayName="ScrollRestoration";function vx(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function kp(e){let t=W.useContext(Lr);return Re(t,vx(e)),t}function rT(e){let t=W.useContext(Hs);return Re(t,vx(e)),t}function gx(e,{target:t,replace:a,state:n,preventScrollReset:r,relative:s,viewTransition:i}={}){let o=pe(),u=Le(),c=Vs(e,{relative:s});return W.useCallback(d=>{if(P3(d,t)){d.preventDefault();let m=a!==void 0?a:Ks(u)===Ks(c);o(e,{replace:m,state:n,preventScrollReset:r,relative:s,viewTransition:i})}},[u,o,c,a,n,t,e,r,s,i])}var sT=0,iT=()=>`__${String(++sT)}__`;function yx(){let{router:e}=kp("useSubmit"),{basename:t}=W.useContext(Bt),a=E3();return W.useCallback(async(n,r={})=>{let{action:s,method:i,encType:o,formData:u,body:c}=F3(n,t);if(r.navigate===!1){let d=r.fetcherKey||iT();await e.fetch(d,a,r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,flushSync:r.flushSync})}else await e.navigate(r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,replace:r.replace,state:r.state,fromRouteId:a,flushSync:r.flushSync,viewTransition:r.viewTransition})},[e,t,a])}function bx(e,{relative:t}={}){let{basename:a}=W.useContext(Bt),n=W.useContext(ta);Re(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...Vs(e||".",{relative:t})},i=Le();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),u=o.getAll("index");if(u.some(d=>d==="")){o.delete("index"),u.filter(m=>m).forEach(m=>o.append("index",m));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:wn([a,s.pathname])),Ks(s)}var up="react-router-scroll-positions",ac={};function cp(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:Ga(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function xx({getKey:e,storageKey:t}={}){let{router:a}=kp("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=rT("useScrollRestoration"),{basename:s}=W.useContext(Bt),i=Le(),o=bp(),u=rx();W.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),oT(W.useCallback(()=>{if(u.state==="idle"){let c=cp(i,o,s,e);ac[c]=window.scrollY}try{sessionStorage.setItem(t||up,JSON.stringify(ac))}catch(c){Wt(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[u.state,e,s,i,o,t])),typeof document<"u"&&(W.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||up);c&&(ac=JSON.parse(c))}catch{}},[t]),W.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(ac,()=>window.scrollY,e?(d,m)=>cp(d,m,s,e):void 0);return()=>c&&c()},[a,s,e]),W.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{Wt(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function oT(e,t){let{capture:a}=t||{};W.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function $x(e,{relative:t}={}){let a=W.useContext(pp);Re(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=kp("useViewTransitionState"),r=Vs(e,{relative:t});if(!a.isTransitioning)return!1;let s=Ga(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=Ga(a.nextLocation.pathname,n)||a.nextLocation.pathname;return Fo(r.pathname,i)!=null||Fo(r.pathname,s)!=null}var Et=new _d({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var Rp="ironclaw_token",Ke="/api/webchat/v2",Ur=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function ba(){return sessionStorage.getItem(Rp)||""}function Gs(e){e?sessionStorage.setItem(Rp,e):sessionStorage.removeItem(Rp)}function oc(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function Nx(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function Sx(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function _x({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=Sx(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=Sx(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function H(e,t={}){let a=ba(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await Nx(r);throw new Ur(_x({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function lc(){return H(`${Ke}/session`)}function uc({clientActionId:e,requestedThreadId:t,projectId:a}={}){let n={client_action_id:e||oc()};return t&&(n.requested_thread_id=t),a&&(n.project_id=a),H(`${Ke}/threads`,{method:"POST",body:JSON.stringify(n)})}function kx({limit:e,cursor:t}={}){let a=new URL(`${Ke}/threads`,window.location.origin);return e!=null&&a.searchParams.set("limit",String(e)),t&&a.searchParams.set("cursor",t),H(a.pathname+a.search)}function Rx({threadId:e}={}){return e?H(`${Ke}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function Cp(e){return`${Ke}/threads/${encodeURIComponent(e)}/files`}function Cx({threadId:e,path:t}={}){if(!e)return Promise.reject(new Error("threadId is required"));let a=new URL(Cp(e),window.location.origin);return t&&a.searchParams.set("path",t),H(a.pathname+a.search)}function Ex({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${Cp(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),H(a.pathname+a.search)}function cc({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${Cp(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function Tx({limit:e,runLimit:t,includeCompleted:a}={}){let n=new URLSearchParams;e!=null&&n.set("limit",String(e)),t!=null&&n.set("run_limit",String(t)),a===!0&&n.set("include_completed","true");let r=n.toString();return H(`${Ke}/automations${r?`?${r}`:""}`)}function Ax({automationId:e}={}){return e?H(`${Ke}/automations/${encodeURIComponent(e)}/pause`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function Dx({automationId:e}={}){return e?H(`${Ke}/automations/${encodeURIComponent(e)}/resume`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function Mx({automationId:e}={}){return e?H(`${Ke}/automations/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("automationId is required"))}var Ox=`${Ke}/projects`;function lT(e){return`${Ox}/${encodeURIComponent(e)}`}function Lx({limit:e}={}){let t=new URL(Ox,window.location.origin);return e!=null&&t.searchParams.set("limit",String(e)),H(t.pathname+t.search)}function Px({projectId:e}={}){return e?H(lT(e)):Promise.reject(new Error("projectId is required"))}function Ux(){return H(`${Ke}/outbound/preferences`)}function jx(){return H(`${Ke}/outbound/targets`)}function Fx({finalReplyTargetId:e}={}){return H(`${Ke}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function Ep({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:u,source:c,tail:d,follow:m}={}){let f=new URL(`${Ke}/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),u&&f.searchParams.set("tool_name",u),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),H(f.pathname+f.search)}function Bx({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:u,source:c,tail:d,follow:m}={}){let f=new URL(`${Ke}/operator/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),u&&f.searchParams.set("tool_name",u),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),H(f.pathname+f.search)}function zx({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||oc(),content:t};return a.length>0&&(r.attachments=a),H(`${Ke}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function qx({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${Ke}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),H(n.pathname+n.search)}function Ix({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${Ke}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function Ra(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new Ur("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=ba(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await Nx(r);throw new Ur(_x({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function Tp(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function dc(e){return Tp(await Ra(e))}function Kx({threadId:e,afterCursor:t}={}){let a=new URL(`${Ke}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=ba();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function Hx({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||oc()};return a&&(r.reason=a),H(`${Ke}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function Ap({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let u={client_action_id:i||oc(),resolution:n};return r!=null&&(u.always=r),s&&(u.credential_ref=s),H(`${Ke}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(u)})}function Qx({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return H("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function Vx(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),H(`${Ke}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function Ys(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function Gx(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function Yx(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new Ur("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new Ur("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function Jx(){let e=ba();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var mc="anon",Xx=mc;function Zx(e){Xx=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:mc}function Nt(){return Xx}var Wx="ironclaw:v2-thread-pins:",Dp=new Set,Nn=new Set,Mp=null;function Op(){return`${Wx}${Nt()}`}function uT(){try{let e=window.localStorage.getItem(Op());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function cT(){try{Nn.size===0?window.localStorage.removeItem(Op()):window.localStorage.setItem(Op(),JSON.stringify([...Nn]))}catch{}}function e$(){let e=Nt();if(e!==Mp){Nn.clear();for(let t of uT())Nn.add(t);Mp=e}}function t$(){return new Set(Nn)}function a$(){let e=t$();for(let t of Dp)try{t(e)}catch{}}function n$(e){e&&(e$(),Nn.has(e)?Nn.delete(e):Nn.add(e),cT(),a$())}function r$(){return e$(),t$()}function s$(e){return Dp.add(e),()=>{Dp.delete(e)}}function i$(){Nn.clear(),Mp=Nt();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(Wx)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}a$()}var dT=0,jr={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function Lp(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function o$(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":mT(t)?"text":"download"}function mT(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function zo(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function fT(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function pT(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function hT(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function l$(e,{limits:t,existing:a=[],t:n}){let r=t||jr,s=[],i=[],o=a.length,u=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!fT(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:zo(r.maxFileBytes)}));continue}if(u+c.size>r.maxTotalBytes){let y=n("chat.attachmentTotalTooLarge",{max:zo(r.maxTotalBytes)});i.includes(y)||i.push(y);continue}let d;try{d=await pT(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:m,base64:f}=hT(d,c.type),p=m||"application/octet-stream",x=Lp(p);s.push({id:`staged-${dT++}`,filename:c.name||"attachment",mimeType:p,kind:x,sizeBytes:c.size,sizeLabel:zo(c.size),dataBase64:f,previewUrl:x==="image"?d:null}),o+=1,u+=c.size}return{staged:s,errors:i}}function u$(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function c$(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}function vT(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||Lp(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?Ix({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?zo(n.size_bytes):"",preview_url:null,fetch_url:s}})}function m$(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let c=xT(s);if(!c)continue;let d=`tool-${c.invocationId}`;if(n.has(d))continue;n.add(d),r.push({id:d,role:"tool_activity",...c,timestamp:d$(s)||c.updatedAt||null,sequence:s.sequence,activityOrder:c.activityOrder,activityOrderSource:c.activityOrderSource,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=bT(s),u=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy");r.push({id:i,role:o,content:s.content||"",attachments:vT(s,a),timestamp:d$(s),kind:s.kind,status:u?"error":s.status,...u&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:yT(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=gT(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function gT(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function yT(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function bT(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function d$(e){return e.received_at||e.created_at||null}function xT(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:Pp(t)}var $T="gate_declined";function Pp(e){let t=e.status==="failed"||e.status==="killed",a=e.error_kind||null,n=h$(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Io(e.title||e.capability_id)||"tool",toolStatus:p$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(f$(a)||e.output_summary||e.output_preview||e.result_ref)||null,toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null,activityOrder:n,activityOrderSource:Number.isFinite(n)?"projection":null}}function Up(e){let t=h$(e.activity_order),a=e.error_kind||null;return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Io(e.capability_id)||"tool",toolStatus:p$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:null,toolError:f$(a),toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null,activityOrder:t,activityOrderSource:Number.isFinite(t)?"projection":null}}function f$(e){return e||null}function qo(e){return e==="success"||e==="error"||e==="declined"}function Io(e){let t=typeof e=="string"?e.trim():"";if(!t)return"";let a=t.split(".");return a[a.length-1]||t}function p$(e,t=null){if(t===$T)return"declined";switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}function h$(e){let t=Number(e);return Number.isFinite(t)?t:null}var wT=50,Ja=new Map,ST=30;function Ko(e,t){for(Ja.delete(e),Ja.set(e,t);Ja.size>ST;){let a=Ja.keys().next().value;Ja.delete(a)}}function Ho(e){return`${Nt()}:${e}`}function g$(){Ja.clear()}function y$(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?Ja.get(Ho(e)):null,[s,i]=h.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),o=h.default.useRef(new Set),u=h.default.useRef(e);u.current=e;let c=h.default.useCallback(async(m,f={})=>{let{preserveClientOnly:p=!1,finalReplyTimestampByRun:x=null}=f;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(o.current.has(e))return;o.current.add(e);let y=Nt(),w=Ho(e);i(g=>({...g,isLoading:!0}));try{let g=await qx({threadId:e,limit:wT,cursor:m});if(Nt()!==y)return;let v=m?[]:a?.()||[],b=m$(g.messages||[],v,e),$=g.next_cursor||null;if(m||n?.([]),!m){let S=Ja.get(w)?.messages||[],E=v$(b,S,{preserveClientOnly:p,finalReplyTimestampByRun:x});Ko(w,{messages:E,nextCursor:$})}i(S=>{if(u.current!==e)return S;let E;return m?E=NT(b,S.messages):E=v$(b,S.messages,{preserveClientOnly:p,finalReplyTimestampByRun:x}),Ko(w,{messages:E,nextCursor:$}),{messages:E,nextCursor:$,isLoading:!1,loadError:null}})}catch(g){if(console.error("Failed to load timeline:",g),Nt()!==y)return;i(v=>u.current===e?{...v,isLoading:!1,loadError:"Failed to load conversation history."}:v)}finally{o.current.delete(e)}},[e,a,n]);h.default.useEffect(()=>{let m=e?Ja.get(Ho(e)):null;i({messages:m?.messages||[],nextCursor:m?.nextCursor||null,isLoading:!!e&&!m,loadError:null}),e&&c()},[e,c]);let d=h.default.useCallback((m,f)=>{if(!m)return;let p=Ho(m),x=g=>typeof f=="function"?f(g||[]):f;if(u.current===m){i(g=>{let v=x(g.messages||[]);return Ko(p,{messages:v,nextCursor:g.nextCursor||null}),{...g,messages:v}});return}let y=Ja.get(p)||{messages:[],nextCursor:null},w=x(y.messages||[]);Ko(p,{messages:w,nextCursor:y.nextCursor||null})},[]);return{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:c,seedThreadMessages:d,setMessages:m=>i(f=>{let p=typeof m=="function"?m(f.messages):m;return e&&Ko(Ho(e),{messages:p,nextCursor:f.nextCursor}),{...f,messages:p}})}}function NT(e,t){let a=new Set(t.map(n=>n?.id).filter(Boolean));return[...e.filter(n=>!a.has(n?.id)),...t]}function v$(e,t,a={}){let{preserveClientOnly:n=!1,finalReplyTimestampByRun:r=null}=a,s=kT(e,t,{finalReplyTimestampByRun:r}),i=new Set(s.map(u=>u?.id).filter(Boolean)),o=t.filter(u=>!u||typeof u.id!="string"||i.has(u.id)?!1:RT(u)?!0:typeof u.timelineMessageId=="string"&&i.has(`msg-${u.timelineMessageId}`)?!1:_T(u)?!0:n&&u.id.startsWith("err-"));return o.length>0?[...s,...o]:s}function _T(e){return e?.isOptimistic===!0&&typeof e.id=="string"&&e.id.startsWith("pending-")&&(e.role==="user"||e.role==="assistant")}function kT(e,t,a={}){let{finalReplyTimestampByRun:n=null}=a,r=new Map,s=new Map;for(let i of t||[])!i||!i.timestamp||(typeof i.id=="string"&&r.set(i.id,i),typeof i.timelineMessageId=="string"&&r.set(`msg-${i.timelineMessageId}`,i),jp(i)&&typeof i.turnRunId=="string"&&s.set(i.turnRunId,i));return r.size===0&&s.size===0&&!n?e:e.map(i=>{if(!i||i.timestamp||typeof i.id!="string")return i;let o=typeof i.turnRunId=="string"?i.turnRunId:null,u=r.get(i.id)||(jp(i)&&o?s.get(o):null),c=jp(i)&&o?n?.[o]:null,d=u?.timestamp||c;return d?{...i,timestamp:d}:i})}function jp(e){return e?.role==="assistant"&&e?.isFinalReply===!0}function RT(e){return e?.role==="tool_activity"||e?.role==="thinking"}var Vo="__new__",b$="ironclaw:v2-draft:";function Js(e){return`${b$}${Nt()}:${e||Vo}`}function Fp(e){try{return window.localStorage.getItem(Js(e))||""}catch{return""}}function Bp(e,t){try{t?window.localStorage.setItem(Js(e),t):window.localStorage.removeItem(Js(e))}catch{}}function x$(e){Bp(e,"")}var Qo=new Map;function zp(e){return Qo.get(Js(e))||[]}function $$(e,t){let a=Js(e);t&&t.length>0?Qo.set(a,t):Qo.delete(a)}function w$(e){Qo.delete(Js(e))}function S$(){Qo.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(b$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function CT(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function ET(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function TT(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=CT(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?ET(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),ba()?"":(Gs(n),n)}function AT(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var DT={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function MT(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),DT[t]||"Could not complete sign-in. Please try again."):""}function N$(){let[e,t]=h.default.useState(()=>TT()||ba()),[a,n]=h.default.useState(()=>MT()),[r]=h.default.useState(()=>AT()),[s,i]=h.default.useState(null),[o,u]=h.default.useState(()=>!!(r&&!ba())),[c,d]=h.default.useState(()=>!!ba());h.default.useEffect(()=>{if(!r||ba()){u(!1);return}let x=!1;return Yx(r).then(y=>{x||(Gs(y),d(!0),t(y),i(null),n(""),u(!1),Et.clear())}).catch(()=>{x||(n("Could not complete sign-in. Please try again."),u(!1))}),()=>{x=!0}},[r]),h.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let x=!1;return d(!0),lc().then(y=>{x||(i(y),d(!1))}).catch(y=>{x||(i(null),d(!1),(y?.status===401||y?.status===403)&&(Gs(""),t(""),n("Your session expired. Please sign in again."),Et.clear()))}),()=>{x=!0}},[e,o]),Zx(s);let m=h.default.useRef(null);h.default.useEffect(()=>{let x=Nt();m.current&&m.current!==mc&&m.current!==x&&(g$(),S$(),i$()),m.current=x},[s]);let f=h.default.useCallback(x=>{Gs(x),d(!!x),t(x),i(null),n(""),Et.clear()},[]),p=h.default.useCallback(()=>{Jx().catch(()=>{}),Gs(""),d(!1),t(""),i(null),n(""),Et.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,rebornProjectsEnabled:!!s?.features?.reborn_projects,signIn:f,signOut:p}}var Fr="/chat",Go=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace"},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"logs",path:"/logs",labelKey:"nav.logs",hidden:!0},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var OT=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"tools",labelKey:"settings.tools",icon:"tool"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],LT=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],PT=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],fc={settings:OT,extensions:LT,admin:PT};var _$="ironclaw:v2-theme";function UT(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(_$);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function pc(){let[e,t]=h.default.useState(UT);h.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(_$,e)}catch{}},[e]);let a=h.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function k$(e){return I({enabled:!!e,queryKey:["gateway-status",e],queryFn:Ys,refetchInterval:3e4})}var jT="/api/webchat/v2/operator/config",hc="/api/webchat/v2/settings/tools",Xs="agent.auto_approve_tools",R$="tool.",FT=new Set(["always_allow","ask_each_time","disabled"]),BT=new Set(["default","always_allow","ask_each_time","disabled"]);function C$(e){return e==="ask"?"ask_each_time":FT.has(e)?e:"ask_each_time"}function zT(e){return e==="ask"?"ask_each_time":BT.has(e)?e:"default"}function qT(e){return["default","global","override"].includes(e)?e:"default"}function E$(e){if(!e?.key?.startsWith(R$))return null;let t=e.value||{};return{name:t.name||e.key.slice(R$.length),description:t.description||"",state:C$(t.state),default_state:C$(t.default_state),locked:!!(t.locked||e.mutable===!1),effective_source:qT(t.effective_source||e.source)}}function IT(e){let t={};for(let a of e.entries||[])a?.key===Xs&&(t[Xs]=!!a.value);return t}async function T$(){let e=await H(hc);return{settings:IT(e),diagnostics:e.diagnostics||[],precedence:e.precedence||[]}}async function qp(e,t){if(e===Xs){let n=await H(hc,{method:"POST",body:JSON.stringify({enabled:!!t})});return{success:!0,entry:n.entry,value:n.entry?.value}}let a=await H(`${jT}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({value:t})});return{success:!0,entry:a.entry,value:a.entry?.value}}async function A$(e){let t=e?.settings||{},a=[];return Object.prototype.hasOwnProperty.call(t,Xs)&&a.push(await qp(Xs,!!t[Xs])),{success:!0,imported:a.length,results:a}}function vc(){return H("/api/webchat/v2/llm/providers")}function D$(e){return H("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function M$(e){return H(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function Yo(e){return H("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function O$(e){return H("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function L$(e){return H("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function P$(e){return H("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function U$(e){return H("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function j$(){return H("/api/webchat/v2/llm/codex/login",{method:"POST"})}async function F$(){let e=await H(hc);return{tools:(e.entries||[]).map(E$).filter(Boolean),diagnostics:e.diagnostics||[]}}async function B$(e,t){let a=zT(t),n=await H(`${hc}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({state:a})});return{success:!0,tool:E$(n.entry),entry:n.entry}}function z$(){return H("/api/webchat/v2/extensions")}function q$(){return H("/api/webchat/v2/extensions/registry")}function I$(){return H("/api/webchat/v2/skills")}function K$(e){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function H$(e){return H("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function Q$(e,t){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function V$(e){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function G$(e,t){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}/auto-activate`,{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:t})})}function Y$(e){return H("/api/webchat/v2/skills/auto-activate-learned",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function J$(){return H("/api/webchat/v2/traces/credit")}function X$(e){return H(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function Z$(){return Promise.resolve({users:[],todo:!0})}function W$(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function ew(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var Ip="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",Kp=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function Jo(e){return Kp.find(t=>t.value===e)?.label||e}function Zs(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function tw(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function gc(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function aw(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function Br(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Ip||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?Zs(e,t).trim().length>0:!0:!1}function KT(e,t,a){return e.id===a?"active":Br(e,t)?"ready":"setup"}function nw(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=KT(r,t,a);n[s]&&n[s].push(r)}return n}function yc(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Ip||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!Zs(e,t).trim()?"base_url":"ok"}function Hp(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===Ip&&(i.api_key=void 0),i}function rw(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function sw(e){return/^[a-z0-9_-]+$/.test(e)}function iw(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var HT=Object.freeze({});function Ws({settings:e,gatewayStatus:t,enabled:a=!0}){let n=J(),r=I({queryKey:["llm-providers"],queryFn:vc,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=HT,u=(s.providers||[]).map($=>({...$,name:$.description,has_api_key:$.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,m=d||"nearai",f=s.active?.model||t?.llm_model||"",p=u.filter($=>$.builtin),x=u.filter($=>!$.builtin),y=[...u].sort(($,S)=>$.id===d?-1:S.id===d?1:($.name||$.id).localeCompare(S.name||S.id)),w=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=Q({mutationFn:async $=>{if(!Br($,o)){let E=yc($,o);throw new Error(E==="base_url"?"base_url":"api_key")}let S=gc($,o);if(!S)throw new Error("model");return await Yo({provider_id:$.id,model:S}),$},onSuccess:w}),v=Q({mutationFn:async({provider:$,form:S,apiKey:E,editingProvider:_})=>{let T=!!$?.builtin,D={id:(T?$.id:S.id.trim()).trim(),name:T?$.name||$.id:S.name.trim(),adapter:T?$.adapter:S.adapter,base_url:S.baseUrl.trim()||$?.base_url||"",default_model:S.model.trim()||void 0};return E.trim()&&(D.api_key=E.trim()),(_||$)?.id===m&&D.default_model&&(D.set_active=!0,D.model=D.default_model),await D$(D),D},onSuccess:w}),b=Q({mutationFn:async $=>(await M$($.id),$),onSuccess:w});return{providers:y,builtinProviders:p,customProviders:x,builtinOverrides:o,activeProviderId:d,selectedModel:f,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:$=>g.mutateAsync($),saveCustomProvider:$=>v.mutateAsync($),saveBuiltinProvider:$=>v.mutateAsync($),deleteCustomProvider:$=>b.mutateAsync($),testConnection:O$,listModels:L$,isBusy:g.isPending||v.isPending||b.isPending}}function ow({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}var lw="ironclaw:v2-sidebar-open";function uw(){return typeof window>"u"?null:window}function cw(){try{return uw()?.localStorage||null}catch{return null}}function dw(e=cw()){try{return e?.getItem(lw)!=="false"}catch{return!0}}function mw(e,t=cw()){try{t?.setItem(lw,e?"true":"false")}catch{}}function fw(e=uw()){try{return e?.matchMedia?.("(min-width: 768px)").matches===!0}catch{return!1}}function pw(e,t){return t?{...e,desktopOpen:!e.desktopOpen}:{...e,mobileOpen:!e.mobileOpen}}function hw(e,t){return t?e.desktopOpen:e.mobileOpen}function vw({onNewChat:e}={}){let t=pe(),[a,n]=h.default.useState(()=>({mobileOpen:!1,desktopOpen:dw()})),[r,s]=h.default.useState(()=>fw());h.default.useEffect(()=>{let d=window.matchMedia("(min-width: 768px)"),m=()=>s(d.matches);return m(),d.addEventListener?.("change",m),()=>d.removeEventListener?.("change",m)},[]),h.default.useEffect(()=>{mw(a.desktopOpen)},[a.desktopOpen]);let i=h.default.useCallback(()=>{n(d=>({...d,mobileOpen:!1}))},[]),o=h.default.useCallback(()=>{n(d=>pw(d,r))},[r]),u=h.default.useCallback(async()=>{let d=await e?.(),m=typeof d=="string"&&d.length>0?d:null;t(m?`/chat/${m}`:"/chat"),i()},[t,i,e]),c=h.default.useCallback(d=>{t(`/chat/${d}`),i()},[t,i]);return{mobileOpen:a.mobileOpen,desktopOpen:a.desktopOpen,currentOpen:hw(a,r),close:i,toggle:o,newChat:u,selectThread:c}}var Qp=new Set,QT=0;function ei(e,t={}){let a={id:++QT,message:e,tone:t.tone||"info",duration:t.duration??2600};return Qp.forEach(n=>n(a)),a.id}function gw(e){return Qp.add(e),()=>Qp.delete(e)}function VT(e){return e?.status===409&&e?.payload?.kind==="busy"}function yw(e,t){return VT(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function bw(){let e=I({queryKey:["threads"],queryFn:()=>kx({})}),[t,a]=h.default.useState(null),[n,r]=h.default.useState(!1),s=h.default.useRef(new Map),i=h.default.useCallback(async c=>{let d=c||"__global__",m=s.current.get(d);if(m)return m;r(!0);let f=(async()=>{try{let p=await uc(c?{projectId:c}:void 0);Et.invalidateQueries({queryKey:["threads"]});let x=p?.thread?.thread_id;return x&&a(x),x}finally{s.current.delete(d),r(s.current.size>0)}})();return s.current.set(d,f),f},[]),o=h.default.useCallback(async c=>{await Rx({threadId:c}),t===c&&a(null),Et.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:h.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var xw={attach:l`<path
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
    />`,arrowDown:l`<path d="M12 5v14" /><path d="m6 13 6 6 6-6" />`,retry:l`<path d="M3.5 12a8.5 8.5 0 1 1 2.6 6.1" /><path d="M3.2 18.5v-5h5" />`};function O({name:e,className:t="",strokeWidth:a=1.7}){return l`
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
      ${xw[e]||xw.spark}
    </svg>
  `}function V(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=V(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function $w(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function GT(e){return $w(e).trim().charAt(0).toUpperCase()||"I"}function YT(){let[e,t]=h.default.useState(!1),a=h.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function ww({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=C(),s=YT(),i=$w(a),o=a?.email||a?.role||r("common.gatewaySession");return l`
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
            />`:l`<span className="place-self-center">${GT(a)}</span>`}
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
  `}var Sw={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",logs:"list",settings:"settings",admin:"shield"},JT=Go.filter(e=>e.id!=="chat"&&!e.hidden);function XT({route:e,label:t,onNavigate:a}){return l`
    <${Ya}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${O} name=${Sw[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function ZT({route:e,label:t,subRoutes:a,onNavigate:n}){let r=C(),s=Le(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return l`
    <div className="flex flex-col">
      <${Ya}
        to=${o}
        onClick=${n}
        className=${()=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${O}
          name=${Sw[e.id]||"bolt"}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1 truncate">${t}</span>
        <${O}
          name="chevron"
          className=${V("h-3.5 w-3.5 shrink-0 transition-transform duration-150",i&&"rotate-180")}
        />
      <//>

      ${i&&l`
        <div className="mt-0.5 flex flex-col gap-0.5 pl-3">
          ${a.map(u=>l`
              <${Ya}
                key=${u.id}
                to=${e.path+"/"+u.id}
                onClick=${n}
                className=${({isActive:c})=>V("flex items-center gap-2.5 rounded-[8px] py-1.5 pl-7 pr-3 text-[12px] font-medium",c?"text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
              >
                <${O} name=${u.icon} className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate">${r(u.labelKey)}</span>
              <//>
            `)}
        </div>
      `}
    </div>
  `}function Nw({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=C(),s=h.default.useMemo(()=>JT.filter(i=>a||i.id!=="admin"),[a]);return l`
    <div className="flex flex-col px-3 py-2">
      <button
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
        ${s.map(i=>{let o=(fc[i.id]||[]).filter(u=>a||!(i.id==="settings"&&["users","inference"].includes(u.id)));return o.length>0?l`
              <${ZT}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:l`
            <${XT}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var _n=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),Xo=new Set([_n.NEEDS_ATTENTION,_n.FAILED]),Vp="ironclaw:v2-thread-attention",Gp=new Set,ti=new Map;function WT(){try{let e=window.localStorage.getItem(Vp);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&Xo.has(a[1])):[]}catch{return[]}}function _w(){let e=[];for(let[t,a]of ti)Xo.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(Vp):window.localStorage.setItem(Vp,JSON.stringify(e))}catch{}}for(let[e,t]of WT())ti.set(e,t);function Rw(){return new Map(ti)}function kw(){let e=Rw();for(let t of Gp)try{t(e)}catch{}}function bc(e,t){if(!e)return;let a=ti.get(e);if(t==null){if(!ti.delete(e))return;Xo.has(a)&&_w(),kw();return}a!==t&&(ti.set(e,t),(Xo.has(t)||Xo.has(a))&&_w(),kw())}function Cw(e){bc(e,null)}function eA(){return Rw()}function tA(e){return Gp.add(e),()=>{Gp.delete(e)}}function Ew(){let[e,t]=h.default.useState(eA);return h.default.useEffect(()=>tA(t),[]),e}function xc(e){return e.updated_at||e.created_at||null}function Yp(e,t){let a=xc(e)||"",n=xc(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function Tw(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function Aw(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function aA(){let[e,t]=h.default.useState(r$);return h.default.useEffect(()=>s$(t),[]),e}var nA=Object.freeze({[_n.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-transparent"},[_n.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[_n.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function rA(e){return e&&nA[e]||null}function sA({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=C(),o=xc(e),u=Tw(o),c=Aw(o),d=h.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(p=>{window.alert(p?.message||"Unable to delete chat")})},[s,e.id]),m=h.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),n$(e.id)},[e.id]);return l`
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
        onClick=${m}
        title=${i(a?"common.unpin":"common.pin")}
        aria-label=${i(a?"common.unpin":"common.pin")}
        aria-pressed=${a?"true":"false"}
        className=${V("my-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] transition",a?"text-[var(--v2-accent-text)]":"opacity-0 text-[var(--v2-text-faint)] group-hover:opacity-100 focus:opacity-100","hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-accent-text)]")}
      >
        <${O} name="pin" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>
      ${s&&l`<button
        type="button"
        onClick=${d}
        title=${i("common.deleteChat")}
        aria-label=${i("common.deleteChat")}
        className=${V("my-1 mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px]","opacity-0 transition group-hover:opacity-100 focus:opacity-100","text-[var(--v2-text-faint)] hover:bg-[var(--v2-danger-soft)] hover:text-[var(--v2-danger-text)]")}
      >
        <${O} name="trash" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>`}
    </div>
  `}function Dw({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:l`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>l`
          <${sA}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${rA(n.get(o.id))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function Mw({threads:e,activeThreadId:t,rebornProjectsEnabled:a=!1,onSelect:n,onDelete:r,onNavigate:s}){let[i,o]=h.default.useState(!1),[u,c]=h.default.useState(""),d=Ew(),m=aA(),f=C(),{pinned:p,recent:x,totalMatches:y}=h.default.useMemo(()=>{let w=u.trim().toLowerCase(),g=w?e.filter($=>($.title||$.id||"").toLowerCase().includes(w)):e,v=[],b=[];for(let $ of g)m.has($.id)?v.push($):b.push($);return v.sort(Yp),b.sort(Yp),{pinned:v,recent:b,totalMatches:v.length+b.length}},[e,u,m]);return l`
    <div className="flex min-h-0 flex-1 flex-col px-2">
      <button
        onClick=${()=>o(w=>!w)}
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

      ${!i&&l`
        ${e.length>0&&l`<div className="relative mb-1 mt-1 px-1">
          <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--v2-text-faint)]">
            <${O} name="search" className="h-3.5 w-3.5" />
          </span>
          <input
            type="text"
            value=${u}
            onInput=${w=>c(w.currentTarget.value)}
            placeholder=${f("common.searchChats")}
            className="h-8 w-full rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] pl-8 pr-2 text-[12px] text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]"
          />
        </div>`}
        ${a&&l`<div className="mb-1 px-1">
          <${Ya}
            to="/projects"
            onClick=${s}
            className=${({isActive:w})=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",w?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
          >
            <${O} name="folder" className="h-4 w-4 shrink-0" />
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

          <${Dw}
            label=${f("common.pinned")}
            items=${p}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${m}
            onSelect=${n}
            onDelete=${r}
          />
          <${Dw}
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
  `}function $c(){let e=J(),t=I({queryKey:["trace-credits"],queryFn:J$,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=Q({mutationFn:X$,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function iA(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function Ow(){let e=C(),{credits:t}=$c();if(!t||!t.enrolled)return null;let a=iA(t.final_credit),n=t.submissions_accepted||0,r=t.submissions_submitted||0,s=t.manual_review_hold_count||0;return l`
    <div className="px-3 pb-1">
      <${Sn}
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
        ${s>0&&l`
          <div className="mt-1 text-[11px] font-medium text-[var(--v2-accent-text)]">
            ${e("traceCommons.cardHeld",{count:s})}
          </div>
        `}
      <//>
    </div>
  `}function Lw({id:e,threadsState:t,theme:a,toggleTheme:n,profile:r,isAdmin:s,rebornProjectsEnabled:i=!1,onSignOut:o,onClose:u,onNewChat:c,onSelectThread:d,onDeleteThread:m}){return l`
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

      <${Nw}
        onNewChat=${c}
        isCreating=${t.isCreating}
        isAdmin=${s}
        onNavigate=${u}
      />

      <${Ow} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${Mw}
          threads=${t.threads}
          activeThreadId=${t.activeThreadId}
          rebornProjectsEnabled=${i}
          onSelect=${d}
          onDelete=${m}
          onNavigate=${u}
        />
      </div>

      <${ww}
        theme=${a}
        toggleTheme=${n}
        profile=${r}
        onSignOut=${o}
      />
    </aside>
  `}var oA="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",lA="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",Pw="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",Uw={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},jw={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function A({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=Uw[n]??Uw.md,u=r?"w-full":"";if(a==="primary")return l`
      <${s}
        style=${{background:oA,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${V(Pw,o,u,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:lA}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=jw[a]??jw.outline;return l`
    <${s}
      className=${V(Pw,o,u,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function Fw(){let e=h.default.useMemo(()=>uA(window.location),[]),[t,a]=h.default.useState(null),[n,r]=h.default.useState(null),[s,i]=h.default.useState(!1),[o,u]=h.default.useState(""),[c,d]=h.default.useState(!1);h.default.useEffect(()=>{if(!e)return;let p=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:p.signal}).then(x=>{if(!x.ok)throw new Error(String(x.status));return x.json()}).then(a).catch(()=>{p.signal.aborted||a(null)}),()=>p.abort()},[e]);let m=h.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),u("");try{let p=await fetch(`${e.base}/attestation/report`);if(!p.ok)throw new Error(String(p.status));let x=await p.json();return r(x),x}catch(p){return u(p.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),f=h.default.useCallback(async()=>{let p=n||await m();return!p||!navigator.clipboard?!1:(await navigator.clipboard.writeText(JSON.stringify({...p,instance_attestation:t},null,2)),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[m,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:m,copyReport:f}}function uA(e){let t=e.hostname;if(!t||t==="localhost"||cA(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function cA(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}var dA=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function Bw(){let e=C(),t=Fw(),[a,n]=h.default.useState(!1),r=h.default.useCallback(()=>{n(o=>{let u=!o;return u&&t.loadReport(),u})},[t]),s=h.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=mA({teeInfo:t.teeInfo,report:t.report,t:e});return l`
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

      ${a&&l`
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
              <${O} name="check" className="h-4 w-4" />
              ${t.copied?e("tee.copied"):e("tee.copyReport")}
            <//>
          </div>
        </div>
      `}
    </div>
  `}function mA({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return dA.map(([r,s])=>({label:a(s),value:fA(n[r])||a("common.unknown")}))}function fA(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var pA="https://docs.ironclaw.com";function zw({threadsState:e,onToggleSidebar:t,sidebarOpen:a=!0}){let n=C(),r=Le(),s=h.default.useMemo(()=>{for(let o of Go){let u=fc[o.id];if(!u)continue;let c=o.path+"/";if(r.pathname.startsWith(c)){let d=r.pathname.slice(c.length).split("/")[0],m=u.find(f=>f.id===d);if(m)return{parent:n(o.labelKey),current:n(m.labelKey)}}}return null},[r.pathname,n]),i=h.default.useMemo(()=>{if(s)return null;if(r.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(c=>c.id===e.activeThreadId)?.title||n("nav.chat");let o=Go.find(u=>r.pathname.startsWith(u.path));return o?n(o.labelKey):""},[r.pathname,e.activeThreadId,e.threads,n,s]);return l`
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

      ${s?l`
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
          `:l`
            <span
              className="truncate text-[14px] font-semibold text-[var(--v2-text-strong)]"
            >
              ${i}
            </span>
          `}

      <div className="ml-auto flex shrink-0 items-center gap-1">
        <${Bw} />
        <${Ya}
          to="/logs"
          className=${({isActive:o})=>V("inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",o&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
          title=${n("nav.logs")}
        >
          ${n("nav.logs")}
        <//>
        <a
          href=${pA}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${n("nav.docs")}
        >
          ${n("nav.docs")}
        </a>
      </div>
    </header>
  `}function qw({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=pe(),i=C(),[o,u]=h.default.useState(""),[c,d]=h.default.useState(0),m=h.default.useRef(null),f=h.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(b=>({id:`thread-${b.id}`,label:b.title||`Thread ${b.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${b.id}`)}));return[...g,...v]},[a,s,n,r]),p=h.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?f.filter(v=>v.label.toLowerCase().includes(g)):f},[f,o]);h.default.useEffect(()=>{if(!e)return;u(""),d(0);let g=window.requestAnimationFrame(()=>m.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),h.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,p.length-1)))},[p.length]);let x=h.default.useCallback(g=>{g&&(t(),g.run())},[t]),y=h.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,p.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),x(p[c])):g.key==="Escape"&&(g.preventDefault(),t())},[p,c,x,t]);if(!e)return null;let w=null;return l`
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh]" role="dialog" aria-modal="true" aria-label="Command palette">
      <button type="button" aria-label="Close" onClick=${t} className="absolute inset-0 bg-black/50"></button>
      <div className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_30px_60px_-20px_rgba(0,0,0,0.8)]">
        <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-3">
          <${O} name="search" className="h-4 w-4 text-[var(--v2-text-faint)]" />
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
          ${p.length===0&&l`<li className="px-3 py-6 text-center text-sm text-[var(--v2-text-faint)]">No matches</li>`}
          ${p.map((g,v)=>{let b=g.group!==w;return w=g.group,l`
              ${b&&l`<li key=${`g-${g.group}`} className="px-2 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">${g.group}</li>`}
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
  `}var Iw={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},hA={info:"bolt",success:"check",error:"close"};function Kw(){let[e,t]=h.default.useState([]);return h.default.useEffect(()=>gw(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:l`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${e.map(a=>l`
          <div
            key=${a.id}
            role="status"
            className=${["pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",Iw[a.tone]||Iw.info].join(" ")}
          >
            <${O} name=${hA[a.tone]||"bolt"} className="h-4 w-4 shrink-0" />
            <span>${a.message}</span>
          </div>
        `)}
    </div>
  `}function Hw({token:e,profile:t,isChecking:a=!1,isAdmin:n,rebornProjectsEnabled:r=!1,onSignOut:s}){let i=C(),{theme:o,toggleTheme:u}=pc(),c=k$(e),d=bw(),m=vw({onNewChat:()=>d.setActiveThreadId(null)}),f=c.data,p=Le(),x=pe(),y=Ws({settings:{},gatewayStatus:f,enabled:n}),w=n&&ow({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),g=p.pathname==="/welcome"||p.pathname.startsWith("/settings"),[v,b]=h.default.useState(!1);h.default.useEffect(()=>{let S=E=>{(E.metaKey||E.ctrlKey)&&E.key.toLowerCase()==="k"&&(E.preventDefault(),b(_=>!_))};return window.addEventListener("keydown",S),()=>window.removeEventListener("keydown",S)},[]);let $=h.default.useCallback(async S=>{let E=d.activeThreadId===S;try{await d.deleteThread(S),E&&x("/chat",{replace:!0})}catch(_){console.error("Failed to delete thread:",_),ei(yw(_,i),{tone:"error"})}},[x,d,i]);return w&&!g?l`<${ot} to="/welcome" replace />`:l`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${m.mobileOpen&&l`<button
        type="button"
        aria-label=${i("nav.close")}
        onClick=${m.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${V("fixed inset-y-0 left-0 z-50 md:relative md:z-auto",m.mobileOpen?"flex":"hidden",m.desktopOpen?"md:flex":"md:hidden")}
      >
        <${Lw}
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
          onDeleteThread=${$}
        />
      </div>

      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <${zw}
          threadsState=${d}
          onToggleSidebar=${m.toggle}
          sidebarOpen=${m.currentOpen}
        />
        <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
          ${c.error&&l`
            <div
              className=${V("m-4 rounded-[14px] border px-4 py-3 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >
              ${c.error.message||i("error.gatewayConnection")}
            </div>
          `}
          <${xp}
            context=${{gatewayStatus:f,gatewayStatusQuery:c,currentUser:t,isChecking:a,isAdmin:n,threadsState:d}}
          />
        </main>
      </div>
      <${qw}
        open=${v}
        onClose=${()=>b(!1)}
        threadsState=${d}
        onNewChat=${m.newChat}
        onToggleTheme=${u}
      />
      <${Kw} />
    </div>
  `}var zt=ze(Qe(),1),al=e=>e.type==="checkbox",zr=e=>e instanceof Date,Tt=e=>e==null,r1=e=>typeof e=="object",Ye=e=>!Tt(e)&&!Array.isArray(e)&&r1(e)&&!zr(e),vA=e=>Ye(e)&&e.target?al(e.target)?e.target.checked:e.target.value:e,gA=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,yA=(e,t)=>e.has(gA(t)),bA=e=>{let t=e.constructor&&e.constructor.prototype;return Ye(t)&&t.hasOwnProperty("isPrototypeOf")},Zp=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function ft(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(Zp&&(e instanceof Blob||n))&&(a||Ye(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!bA(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=ft(e[r]));else return e;return t}var kc=e=>/^\w*$/.test(e),et=e=>e===void 0,Wp=e=>Array.isArray(e)?e.filter(Boolean):[],eh=e=>Wp(e.replace(/["|']|\]/g,"").split(/\.|\[/)),Y=(e,t,a)=>{if(!t||!Ye(e))return a;let n=(kc(t)?[t]:eh(t)).reduce((r,s)=>Tt(r)?r:r[s],e);return et(n)||n===e?et(e[t])?a:e[t]:n},Xa=e=>typeof e=="boolean",Pe=(e,t,a)=>{let n=-1,r=kc(t)?[t]:eh(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],u=a;if(n!==i){let c=e[o];u=Ye(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=u,e=e[o]}},Qw={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},Ca={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},kn={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},xA=zt.default.createContext(null);xA.displayName="HookFormContext";var $A=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==Ca.all&&(t._proxyFormState[i]=!n||Ca.all),a&&(a[i]=!0),e[i]}});return r},wA=typeof window<"u"?zt.default.useLayoutEffect:zt.default.useEffect;var Za=e=>typeof e=="string",SA=(e,t,a,n,r)=>Za(e)?(n&&t.watch.add(e),Y(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),Y(a,s))):(n&&(t.watchAll=!0),a),Xp=e=>Tt(e)||!r1(e);function sr(e,t,a=new WeakSet){if(Xp(e)||Xp(t))return e===t;if(zr(e)&&zr(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(zr(i)&&zr(o)||Ye(i)&&Ye(o)||Array.isArray(i)&&Array.isArray(o)?!sr(i,o,a):i!==o)return!1}}return!0}var NA=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},el=e=>Array.isArray(e)?e:[e],Vw=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},qt=e=>Ye(e)&&!Object.keys(e).length,th=e=>e.type==="file",Ea=e=>typeof e=="function",Sc=e=>{if(!Zp)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},s1=e=>e.type==="select-multiple",ah=e=>e.type==="radio",_A=e=>ah(e)||al(e),Jp=e=>Sc(e)&&e.isConnected;function kA(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=et(e)?n++:e[t[n++]];return e}function RA(e){for(let t in e)if(e.hasOwnProperty(t)&&!et(e[t]))return!1;return!0}function We(e,t){let a=Array.isArray(t)?t:kc(t)?[t]:eh(t),n=a.length===1?e:kA(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Ye(n)&&qt(n)||Array.isArray(n)&&RA(n))&&We(e,a.slice(0,-1)),e}var i1=e=>{for(let t in e)if(Ea(e[t]))return!0;return!1};function Nc(e,t={}){let a=Array.isArray(e);if(Ye(e)||a)for(let n in e)Array.isArray(e[n])||Ye(e[n])&&!i1(e[n])?(t[n]=Array.isArray(e[n])?[]:{},Nc(e[n],t[n])):Tt(e[n])||(t[n]=!0);return t}function o1(e,t,a){let n=Array.isArray(e);if(Ye(e)||n)for(let r in e)Array.isArray(e[r])||Ye(e[r])&&!i1(e[r])?et(t)||Xp(a[r])?a[r]=Array.isArray(e[r])?Nc(e[r],[]):{...Nc(e[r])}:o1(e[r],Tt(t)?{}:t[r],a[r]):a[r]=!sr(e[r],t[r]);return a}var Zo=(e,t)=>o1(e,t,Nc(t)),Gw={value:!1,isValid:!1},Yw={value:!0,isValid:!0},l1=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!et(e[0].attributes.value)?et(e[0].value)||e[0].value===""?Yw:{value:e[0].value,isValid:!0}:Yw:Gw}return Gw},u1=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>et(e)?e:t?e===""?NaN:e&&+e:a&&Za(e)?new Date(e):n?n(e):e,Jw={isValid:!1,value:null},c1=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,Jw):Jw;function Xw(e){let t=e.ref;return th(t)?t.files:ah(t)?c1(e.refs).value:s1(t)?[...t.selectedOptions].map(({value:a})=>a):al(t)?l1(e.refs).value:u1(et(t.value)?e.ref.value:t.value,e)}var CA=(e,t,a,n)=>{let r={};for(let s of e){let i=Y(t,s);i&&Pe(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},_c=e=>e instanceof RegExp,Wo=e=>et(e)?e:_c(e)?e.source:Ye(e)?_c(e.value)?e.value.source:e.value:e,Zw=e=>({isOnSubmit:!e||e===Ca.onSubmit,isOnBlur:e===Ca.onBlur,isOnChange:e===Ca.onChange,isOnAll:e===Ca.all,isOnTouch:e===Ca.onTouched}),Ww="AsyncFunction",EA=e=>!!e&&!!e.validate&&!!(Ea(e.validate)&&e.validate.constructor.name===Ww||Ye(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===Ww)),TA=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),e1=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),tl=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=Y(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(tl(o,t))break}else if(Ye(o)&&tl(o,t))break}}};function t1(e,t,a){let n=Y(e,a);if(n||kc(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=Y(t,s),o=Y(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var AA=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return qt(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||Ca.all))},DA=(e,t,a)=>!e||!t||e===t||el(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),MA=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,OA=(e,t)=>!Wp(Y(e,t)).length&&We(e,t),LA=(e,t,a)=>{let n=el(Y(e,a));return Pe(n,"root",t[a]),Pe(e,a,n),e},wc=e=>Za(e);function a1(e,t,a="validate"){if(wc(e)||Array.isArray(e)&&e.every(wc)||Xa(e)&&!e)return{type:a,message:wc(e)?e:"",ref:t}}var ai=e=>Ye(e)&&!_c(e)?e:{value:e,message:""},n1=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:u,maxLength:c,minLength:d,min:m,max:f,pattern:p,validate:x,name:y,valueAsNumber:w,mount:g}=e._f,v=Y(a,y);if(!g||t.has(y))return{};let b=o?o[0]:i,$=R=>{r&&b.reportValidity&&(b.setCustomValidity(Xa(R)?"":R||""),b.reportValidity())},S={},E=ah(i),_=al(i),T=E||_,L=(w||th(i))&&et(i.value)&&et(v)||Sc(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,D=NA.bind(null,y,n,S),P=(R,F,G,se=kn.maxLength,ce=kn.minLength)=>{let oe=R?F:G;S[y]={type:R?se:ce,message:oe,ref:i,...D(R?se:ce,oe)}};if(s?!Array.isArray(v)||!v.length:u&&(!T&&(L||Tt(v))||Xa(v)&&!v||_&&!l1(o).isValid||E&&!c1(o).isValid)){let{value:R,message:F}=wc(u)?{value:!!u,message:u}:ai(u);if(R&&(S[y]={type:kn.required,message:F,ref:b,...D(kn.required,F)},!n))return $(F),S}if(!L&&(!Tt(m)||!Tt(f))){let R,F,G=ai(f),se=ai(m);if(!Tt(v)&&!isNaN(v)){let ce=i.valueAsNumber||v&&+v;Tt(G.value)||(R=ce>G.value),Tt(se.value)||(F=ce<se.value)}else{let ce=i.valueAsDate||new Date(v),oe=yt=>new Date(new Date().toDateString()+" "+yt),Je=i.type=="time",He=i.type=="week";Za(G.value)&&v&&(R=Je?oe(v)>oe(G.value):He?v>G.value:ce>new Date(G.value)),Za(se.value)&&v&&(F=Je?oe(v)<oe(se.value):He?v<se.value:ce<new Date(se.value))}if((R||F)&&(P(!!R,G.message,se.message,kn.max,kn.min),!n))return $(S[y].message),S}if((c||d)&&!L&&(Za(v)||s&&Array.isArray(v))){let R=ai(c),F=ai(d),G=!Tt(R.value)&&v.length>+R.value,se=!Tt(F.value)&&v.length<+F.value;if((G||se)&&(P(G,R.message,F.message),!n))return $(S[y].message),S}if(p&&!L&&Za(v)){let{value:R,message:F}=ai(p);if(_c(R)&&!v.match(R)&&(S[y]={type:kn.pattern,message:F,ref:i,...D(kn.pattern,F)},!n))return $(F),S}if(x){if(Ea(x)){let R=await x(v,a),F=a1(R,b);if(F&&(S[y]={...F,...D(kn.validate,F.message)},!n))return $(F.message),S}else if(Ye(x)){let R={};for(let F in x){if(!qt(R)&&!n)break;let G=a1(await x[F](v,a),b,F);G&&(R={...G,...D(F,G.message)},$(G.message),n&&(S[y]=R))}if(!qt(R)&&(S[y]={ref:b,...R},!n))return S}}return $(!0),S},PA={mode:Ca.onSubmit,reValidateMode:Ca.onChange,shouldFocusError:!0};function UA(e={}){let t={...PA,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:Ea(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Ye(t.defaultValues)||Ye(t.values)?ft(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:ft(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},u,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},m={...d},f={array:Vw(),state:Vw()},p=t.criteriaMode===Ca.all,x=N=>k=>{clearTimeout(c),c=setTimeout(N,k)},y=async N=>{if(!t.disabled&&(d.isValid||m.isValid||N)){let k=t.resolver?qt((await _()).errors):await L(n,!0);k!==a.isValid&&f.state.next({isValid:k})}},w=(N,k)=>{!t.disabled&&(d.isValidating||d.validatingFields||m.isValidating||m.validatingFields)&&((N||Array.from(o.mount)).forEach(M=>{M&&(k?Pe(a.validatingFields,M,k):We(a.validatingFields,M))}),f.state.next({validatingFields:a.validatingFields,isValidating:!qt(a.validatingFields)}))},g=(N,k=[],M,K,B=!0,j=!0)=>{if(K&&M&&!t.disabled){if(i.action=!0,j&&Array.isArray(Y(n,N))){let X=M(Y(n,N),K.argA,K.argB);B&&Pe(n,N,X)}if(j&&Array.isArray(Y(a.errors,N))){let X=M(Y(a.errors,N),K.argA,K.argB);B&&Pe(a.errors,N,X),OA(a.errors,N)}if((d.touchedFields||m.touchedFields)&&j&&Array.isArray(Y(a.touchedFields,N))){let X=M(Y(a.touchedFields,N),K.argA,K.argB);B&&Pe(a.touchedFields,N,X)}(d.dirtyFields||m.dirtyFields)&&(a.dirtyFields=Zo(r,s)),f.state.next({name:N,isDirty:P(N,k),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else Pe(s,N,k)},v=(N,k)=>{Pe(a.errors,N,k),f.state.next({errors:a.errors})},b=N=>{a.errors=N,f.state.next({errors:a.errors,isValid:!1})},$=(N,k,M,K)=>{let B=Y(n,N);if(B){let j=Y(s,N,et(M)?Y(r,N):M);et(j)||K&&K.defaultChecked||k?Pe(s,N,k?j:Xw(B._f)):G(N,j),i.mount&&y()}},S=(N,k,M,K,B)=>{let j=!1,X=!1,de={name:N};if(!t.disabled){if(!M||K){(d.isDirty||m.isDirty)&&(X=a.isDirty,a.isDirty=de.isDirty=P(),j=X!==de.isDirty);let xe=sr(Y(r,N),k);X=!!Y(a.dirtyFields,N),xe?We(a.dirtyFields,N):Pe(a.dirtyFields,N,!0),de.dirtyFields=a.dirtyFields,j=j||(d.dirtyFields||m.dirtyFields)&&X!==!xe}if(M){let xe=Y(a.touchedFields,N);xe||(Pe(a.touchedFields,N,M),de.touchedFields=a.touchedFields,j=j||(d.touchedFields||m.touchedFields)&&xe!==M)}j&&B&&f.state.next(de)}return j?de:{}},E=(N,k,M,K)=>{let B=Y(a.errors,N),j=(d.isValid||m.isValid)&&Xa(k)&&a.isValid!==k;if(t.delayError&&M?(u=x(()=>v(N,M)),u(t.delayError)):(clearTimeout(c),u=null,M?Pe(a.errors,N,M):We(a.errors,N)),(M?!sr(B,M):B)||!qt(K)||j){let X={...K,...j&&Xa(k)?{isValid:k}:{},errors:a.errors,name:N};a={...a,...X},f.state.next(X)}},_=async N=>{w(N,!0);let k=await t.resolver(s,t.context,CA(N||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return w(N),k},T=async N=>{let{errors:k}=await _(N);if(N)for(let M of N){let K=Y(k,M);K?Pe(a.errors,M,K):We(a.errors,M)}else a.errors=k;return k},L=async(N,k,M={valid:!0})=>{for(let K in N){let B=N[K];if(B){let{_f:j,...X}=B;if(j){let de=o.array.has(j.name),xe=B._f&&EA(B._f);xe&&d.validatingFields&&w([K],!0);let re=await n1(B,o.disabled,s,p,t.shouldUseNativeValidation&&!k,de);if(xe&&d.validatingFields&&w([K]),re[j.name]&&(M.valid=!1,k))break;!k&&(Y(re,j.name)?de?LA(a.errors,re,j.name):Pe(a.errors,j.name,re[j.name]):We(a.errors,j.name))}!qt(X)&&await L(X,k,M)}}return M.valid},D=()=>{for(let N of o.unMount){let k=Y(n,N);k&&(k._f.refs?k._f.refs.every(M=>!Jp(M)):!Jp(k._f.ref))&&Ma(N)}o.unMount=new Set},P=(N,k)=>!t.disabled&&(N&&k&&Pe(s,N,k),!sr(yt(),r)),R=(N,k,M)=>SA(N,o,{...i.mount?s:et(k)?r:Za(N)?{[N]:k}:k},M,k),F=N=>Wp(Y(i.mount?s:r,N,t.shouldUnregister?Y(r,N,[]):[])),G=(N,k,M={})=>{let K=Y(n,N),B=k;if(K){let j=K._f;j&&(!j.disabled&&Pe(s,N,u1(k,j)),B=Sc(j.ref)&&Tt(k)?"":k,s1(j.ref)?[...j.ref.options].forEach(X=>X.selected=B.includes(X.value)):j.refs?al(j.ref)?j.refs.forEach(X=>{(!X.defaultChecked||!X.disabled)&&(Array.isArray(B)?X.checked=!!B.find(de=>de===X.value):X.checked=B===X.value||!!B)}):j.refs.forEach(X=>X.checked=X.value===B):th(j.ref)?j.ref.value="":(j.ref.value=B,j.ref.type||f.state.next({name:N,values:ft(s)})))}(M.shouldDirty||M.shouldTouch)&&S(N,B,M.shouldTouch,M.shouldDirty,!0),M.shouldValidate&&He(N)},se=(N,k,M)=>{for(let K in k){if(!k.hasOwnProperty(K))return;let B=k[K],j=N+"."+K,X=Y(n,j);(o.array.has(N)||Ye(B)||X&&!X._f)&&!zr(B)?se(j,B,M):G(j,B,M)}},ce=(N,k,M={})=>{let K=Y(n,N),B=o.array.has(N),j=ft(k);Pe(s,N,j),B?(f.array.next({name:N,values:ft(s)}),(d.isDirty||d.dirtyFields||m.isDirty||m.dirtyFields)&&M.shouldDirty&&f.state.next({name:N,dirtyFields:Zo(r,s),isDirty:P(N,j)})):K&&!K._f&&!Tt(j)?se(N,j,M):G(N,j,M),e1(N,o)&&f.state.next({...a,name:N}),f.state.next({name:i.mount?N:void 0,values:ft(s)})},oe=async N=>{i.mount=!0;let k=N.target,M=k.name,K=!0,B=Y(n,M),j=xe=>{K=Number.isNaN(xe)||zr(xe)&&isNaN(xe.getTime())||sr(xe,Y(s,M,xe))},X=Zw(t.mode),de=Zw(t.reValidateMode);if(B){let xe,re,La=k.type?Xw(B._f):vA(N),xt=N.type===Qw.BLUR||N.type===Qw.FOCUS_OUT,ia=!TA(B._f)&&!t.resolver&&!Y(a.errors,M)&&!B._f.deps||MA(xt,Y(a.touchedFields,M),a.isSubmitted,de,X),oa=e1(M,o,xt);Pe(s,M,La),xt?(!k||!k.readOnly)&&(B._f.onBlur&&B._f.onBlur(N),u&&u(0)):B._f.onChange&&B._f.onChange(N);let an=S(M,La,xt),pd=!qt(an)||oa;if(!xt&&f.state.next({name:M,type:N.type,values:ft(s)}),ia)return(d.isValid||m.isValid)&&(t.mode==="onBlur"?xt&&y():xt||y()),pd&&f.state.next({name:M,...oa?{}:an});if(!xt&&oa&&f.state.next({...a}),t.resolver){let{errors:wi}=await _([M]);if(j(La),K){let hd=t1(a.errors,n,M),Kh=t1(wi,n,hd.name||M);xe=Kh.error,M=Kh.name,re=qt(wi)}}else w([M],!0),xe=(await n1(B,o.disabled,s,p,t.shouldUseNativeValidation))[M],w([M]),j(La),K&&(xe?re=!1:(d.isValid||m.isValid)&&(re=await L(n,!0)));K&&(B._f.deps&&He(B._f.deps),E(M,re,xe,an))}},Je=(N,k)=>{if(Y(a.errors,k)&&N.focus)return N.focus(),1},He=async(N,k={})=>{let M,K,B=el(N);if(t.resolver){let j=await T(et(N)?N:B);M=qt(j),K=N?!B.some(X=>Y(j,X)):M}else N?(K=(await Promise.all(B.map(async j=>{let X=Y(n,j);return await L(X&&X._f?{[j]:X}:X)}))).every(Boolean),!(!K&&!a.isValid)&&y()):K=M=await L(n);return f.state.next({...!Za(N)||(d.isValid||m.isValid)&&M!==a.isValid?{}:{name:N},...t.resolver||!N?{isValid:M}:{},errors:a.errors}),k.shouldFocus&&!K&&tl(n,Je,N?B:o.mount),K},yt=N=>{let k={...i.mount?s:r};return et(N)?k:Za(N)?Y(k,N):N.map(M=>Y(k,M))},pt=(N,k)=>({invalid:!!Y((k||a).errors,N),isDirty:!!Y((k||a).dirtyFields,N),error:Y((k||a).errors,N),isValidating:!!Y(a.validatingFields,N),isTouched:!!Y((k||a).touchedFields,N)}),Ue=N=>{N&&el(N).forEach(k=>We(a.errors,k)),f.state.next({errors:N?a.errors:{}})},tn=(N,k,M)=>{let K=(Y(n,N,{_f:{}})._f||{}).ref,B=Y(a.errors,N)||{},{ref:j,message:X,type:de,...xe}=B;Pe(a.errors,N,{...xe,...k,ref:K}),f.state.next({name:N,errors:a.errors,isValid:!1}),M&&M.shouldFocus&&K&&K.focus&&K.focus()},Aa=(N,k)=>Ea(N)?f.state.subscribe({next:M=>"values"in M&&N(R(void 0,k),M)}):R(N,k,!0),xa=N=>f.state.subscribe({next:k=>{DA(N.name,k.name,N.exact)&&AA(k,N.formState||d,je,N.reRenderRoot)&&N.callback({values:{...s},...a,...k,defaultValues:r})}}).unsubscribe,Da=N=>(i.mount=!0,m={...m,...N.formState},xa({...N,formState:m})),Ma=(N,k={})=>{for(let M of N?el(N):o.mount)o.mount.delete(M),o.array.delete(M),k.keepValue||(We(n,M),We(s,M)),!k.keepError&&We(a.errors,M),!k.keepDirty&&We(a.dirtyFields,M),!k.keepTouched&&We(a.touchedFields,M),!k.keepIsValidating&&We(a.validatingFields,M),!t.shouldUnregister&&!k.keepDefaultValue&&We(r,M);f.state.next({values:ft(s)}),f.state.next({...a,...k.keepDirty?{isDirty:P()}:{}}),!k.keepIsValid&&y()},Oa=({disabled:N,name:k})=>{(Xa(N)&&i.mount||N||o.disabled.has(k))&&(N?o.disabled.add(k):o.disabled.delete(k))},$a=(N,k={})=>{let M=Y(n,N),K=Xa(k.disabled)||Xa(t.disabled);return Pe(n,N,{...M||{},_f:{...M&&M._f?M._f:{ref:{name:N}},name:N,mount:!0,...k}}),o.mount.add(N),M?Oa({disabled:Xa(k.disabled)?k.disabled:t.disabled,name:N}):$(N,!0,k.value),{...K?{disabled:k.disabled||t.disabled}:{},...t.progressive?{required:!!k.required,min:Wo(k.min),max:Wo(k.max),minLength:Wo(k.minLength),maxLength:Wo(k.maxLength),pattern:Wo(k.pattern)}:{},name:N,onChange:oe,onBlur:oe,ref:B=>{if(B){$a(N,k),M=Y(n,N);let j=et(B.value)&&B.querySelectorAll&&B.querySelectorAll("input,select,textarea")[0]||B,X=_A(j),de=M._f.refs||[];if(X?de.find(xe=>xe===j):j===M._f.ref)return;Pe(n,N,{_f:{...M._f,...X?{refs:[...de.filter(Jp),j,...Array.isArray(Y(r,N))?[{}]:[]],ref:{type:j.type,name:N}}:{ref:j}}}),$(N,!1,void 0,j)}else M=Y(n,N,{}),M._f&&(M._f.mount=!1),(t.shouldUnregister||k.shouldUnregister)&&!(yA(o.array,N)&&i.action)&&o.unMount.add(N)}}},lt=()=>t.shouldFocusError&&tl(n,Je,o.mount),sa=N=>{Xa(N)&&(f.state.next({disabled:N}),tl(n,(k,M)=>{let K=Y(n,M);K&&(k.disabled=K._f.disabled||N,Array.isArray(K._f.refs)&&K._f.refs.forEach(B=>{B.disabled=K._f.disabled||N}))},0,!1))},wa=(N,k)=>async M=>{let K;M&&(M.preventDefault&&M.preventDefault(),M.persist&&M.persist());let B=ft(s);if(f.state.next({isSubmitting:!0}),t.resolver){let{errors:j,values:X}=await _();a.errors=j,B=ft(X)}else await L(n);if(o.disabled.size)for(let j of o.disabled)We(B,j);if(We(a.errors,"root"),qt(a.errors)){f.state.next({errors:{}});try{await N(B,M)}catch(j){K=j}}else k&&await k({...a.errors},M),lt(),setTimeout(lt);if(f.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:qt(a.errors)&&!K,submitCount:a.submitCount+1,errors:a.errors}),K)throw K},It=(N,k={})=>{Y(n,N)&&(et(k.defaultValue)?ce(N,ft(Y(r,N))):(ce(N,k.defaultValue),Pe(r,N,ft(k.defaultValue))),k.keepTouched||We(a.touchedFields,N),k.keepDirty||(We(a.dirtyFields,N),a.isDirty=k.defaultValue?P(N,ft(Y(r,N))):P()),k.keepError||(We(a.errors,N),d.isValid&&y()),f.state.next({...a}))},ae=(N,k={})=>{let M=N?ft(N):r,K=ft(M),B=qt(N),j=B?r:K;if(k.keepDefaultValues||(r=M),!k.keepValues){if(k.keepDirtyValues){let X=new Set([...o.mount,...Object.keys(Zo(r,s))]);for(let de of Array.from(X))Y(a.dirtyFields,de)?Pe(j,de,Y(s,de)):ce(de,Y(j,de))}else{if(Zp&&et(N))for(let X of o.mount){let de=Y(n,X);if(de&&de._f){let xe=Array.isArray(de._f.refs)?de._f.refs[0]:de._f.ref;if(Sc(xe)){let re=xe.closest("form");if(re){re.reset();break}}}}if(k.keepFieldsRef)for(let X of o.mount)ce(X,Y(j,X));else n={}}s=t.shouldUnregister?k.keepDefaultValues?ft(r):{}:ft(j),f.array.next({values:{...j}}),f.state.next({values:{...j}})}o={mount:k.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!k.keepIsValid||!!k.keepDirtyValues,i.watch=!!t.shouldUnregister,f.state.next({submitCount:k.keepSubmitCount?a.submitCount:0,isDirty:B?!1:k.keepDirty?a.isDirty:!!(k.keepDefaultValues&&!sr(N,r)),isSubmitted:k.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:B?{}:k.keepDirtyValues?k.keepDefaultValues&&s?Zo(r,s):a.dirtyFields:k.keepDefaultValues&&N?Zo(r,N):k.keepDirty?a.dirtyFields:{},touchedFields:k.keepTouched?a.touchedFields:{},errors:k.keepErrors?a.errors:{},isSubmitSuccessful:k.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},ne=(N,k)=>ae(Ea(N)?N(s):N,k),Ee=(N,k={})=>{let M=Y(n,N),K=M&&M._f;if(K){let B=K.refs?K.refs[0]:K.ref;B.focus&&(B.focus(),k.shouldSelect&&Ea(B.select)&&B.select())}},je=N=>{a={...a,...N}},Be={control:{register:$a,unregister:Ma,getFieldState:pt,handleSubmit:wa,setError:tn,_subscribe:xa,_runSchema:_,_focusError:lt,_getWatch:R,_getDirty:P,_setValid:y,_setFieldArray:g,_setDisabledField:Oa,_setErrors:b,_getFieldArray:F,_reset:ae,_resetDefaultValues:()=>Ea(t.defaultValues)&&t.defaultValues().then(N=>{ne(N,t.resetOptions),f.state.next({isLoading:!1})}),_removeUnmounted:D,_disableForm:sa,_subjects:f,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(N){i=N},get _defaultValues(){return r},get _names(){return o},set _names(N){o=N},get _formState(){return a},get _options(){return t},set _options(N){t={...t,...N}}},subscribe:Da,trigger:He,register:$a,handleSubmit:wa,watch:Aa,setValue:ce,getValues:yt,reset:ne,resetField:It,clearErrors:Ue,unregister:Ma,setError:tn,setFocus:Ee,getFieldState:pt};return{...Be,formControl:Be}}function d1(e={}){let t=zt.default.useRef(void 0),a=zt.default.useRef(void 0),[n,r]=zt.default.useState({isDirty:!1,isValidating:!1,isLoading:Ea(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:Ea(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!Ea(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=UA(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,wA(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),zt.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),zt.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),zt.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),zt.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),zt.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),zt.default.useEffect(()=>{e.values&&!sr(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),zt.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=$A(n,s),t.current}var m1={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},f1={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},jA={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function ee({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return l`
    <${s}
      className=${V(m1[a]??m1.default,f1[n]??f1.md,jA[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var nh="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",Rc={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function At({className:e="",size:t="md",error:a=!1,...n}){return l`
    <input
      className=${V(nh,Rc[t]??Rc.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Cc({className:e="",error:t=!1,rows:a=4,...n}){return l`
    <textarea
      rows=${a}
      className=${V(nh,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function rh({children:e,className:t="",size:a="md",error:n=!1,...r}){return l`
    <div className="relative w-full">
      <select
        className=${V(nh,Rc[a]??Rc.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
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
  `}function FA({children:e,className:t="",required:a=!1,...n}){return l`
    <label
      className=${V("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&l`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function Rn({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return l`
    <div className=${V("flex flex-col gap-2",s)}>
      ${e&&l`<${FA} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&l`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&l`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var BA={google:"Google",github:"GitHub",apple:"Apple"};function zA(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function p1({providers:e,redirectAfter:t}){let a=C();return e.length?l`
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
              href=${zA(n,t)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <${O} name="shield" className="h-4 w-4" />
              ${a("login.oauthProvider",{provider:BA[n]||n})}
            <//>
          `)}
      </div>
    </div>
  `:null}var qA=["google","github","apple"];function h1(){let[e,t]=h.default.useState([]);return h.default.useEffect(()=>{let a=!1;return Gx().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(qA.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function v1({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=C(),{theme:s,toggleTheme:i}=pc(),o=h1(),{formState:{errors:u,isSubmitting:c},handleSubmit:d,register:m}=d1({defaultValues:{token:e||""}});return l`
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
          <${Rn}
            label=${r("login.tokenLabel")}
            htmlFor="v2-token"
            error=${u.token?.message??""}
            hint=${r("login.tokenHint")}
          >
            <${At}
              id="v2-token"
              type="password"
              error=${!!u.token}
              ...${m("token",{required:r("login.tokenRequired"),setValueAs:f=>f.trim()})}
              placeholder=${r("login.tokenPlaceholder")}
              autocomplete="current-password"
            />
          <//>

          ${t&&l`<p
              className=${V("rounded-[10px] border px-3 py-2 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
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

        <${p1}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var g1={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},y1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function z({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return l`
    <span
      className=${V("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",y1[n]??y1.md,g1[e]??g1.muted,r)}
    >
      ${a&&l`<span
          className=${V("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var IA=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,b1=/(bash|shell|exec|run|command|terminal|spawn|process)/,x1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function $1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return IA.test(n)?{tone:"danger",key:"tool.riskWrite"}:b1.test(n)?{tone:"warning",key:"tool.riskExec"}:x1.test(n)?{tone:"info",key:"tool.riskNetwork"}:b1.test(r)?{tone:"warning",key:"tool.riskExec"}:x1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}var Ec=480;function KA(e,t){return t&&t.length>0?t.some(a=>typeof a?.value=="string"&&a.value.length>Ec):typeof e=="string"&&e.length>Ec}function w1(e,t){return typeof e!="string"||t||e.length<=Ec?e:`${e.slice(0,Ec).trimEnd()}
...`}function S1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=C(),{toolName:s,description:i,parameters:o,allowAlways:u,approvalDetails:c=[]}=e,[d,m]=h.default.useState(!1),[f,p]=h.default.useState(!1);h.default.useEffect(()=>{p(!1)},[e]);let x=h.default.useMemo(()=>$1(s,i,o),[s,i,o]),y=s||r("approval.thisTool"),w=KA(o,c),g=f?"max-h-72":"max-h-36",v=h.default.useCallback(()=>{d&&u?n?.():t?.()},[d,u,n,t]);return l`
    <div
      data-testid="approval-card"
      className="mx-auto max-w-lg rounded-xl border border-copper/30 bg-copper/10 p-4"
    >
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-copper/25 bg-copper/10 text-copper">
          <${O} name="lock" className="h-4 w-4" />
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
      ${s&&l`<div className="mb-1 break-all font-mono text-sm font-medium text-iron-100">${s}</div>`}
      ${i&&l`<div className="mb-3 break-words text-sm text-iron-200">${i}</div>`}
      ${c.length>0?l`
            <dl className=${`mb-2 ${g} overflow-y-auto rounded-md border border-iron-800 bg-iron-950/80 text-xs`}>
              ${c.map(b=>l`
                  <div className="grid gap-1 border-b border-iron-800/70 px-3 py-2 last:border-b-0 sm:grid-cols-[7rem_1fr]">
                    <dt className="font-medium text-iron-400">${b.label}</dt>
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${w1(b.value,f)}</dd>
                  </div>
                `)}
            </dl>
          `:o&&l`<pre className=${`mb-2 ${g} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${w1(o,f)}</pre>`}

      ${w&&l`
        <${A}
          variant="ghost"
          size="sm"
          className="mb-3 px-0 text-[var(--v2-accent)] hover:bg-transparent"
          onClick=${()=>p(b=>!b)}
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
            onChange=${b=>m(b.currentTarget.checked)}
            className="h-3.5 w-3.5 accent-[var(--v2-accent)]"
          />
          ${r("approval.alwaysAllowToolLabel",{tool:y})}
        </label>
      `}

      <div className="flex flex-wrap gap-2">
        <${A} variant="primary" onClick=${v}>
          ${r(d&&u?"approval.approveAndAlways":"approval.approve")}
        <//>
        <${A} variant="secondary" onClick=${()=>a?.()}>
          ${r("approval.deny")}
        <//>
      </div>
    </div>
  `}function ni({icon:e="lock",headline:t,provider:a,accountLabel:n,body:r,expiresAt:s,pillHint:i,defaultExpanded:o=!0,children:u}){let c=C(),[d,m]=h.default.useState(o),f=h.default.useId(),p=n||a||"";return l`
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
          ${p&&l`<span className="block truncate text-xs text-iron-300">${p}</span>`}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 text-xs font-medium text-[#8fc8f2]">
          ${i&&l`<span className="hidden sm:inline">${i}</span>`}
          <${O}
            name="chevron"
            className=${["h-4 w-4",d?"rotate-180":""].join(" ")}
          />
        </span>
      </button>

      ${d&&l`
        <div
          id=${f}
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
  `}function N1({gate:e,onCancel:t}){let a=C();return l`
    <${ni}
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
  `}function _1({gate:e,onCancel:t}){let a=C(),[n,r]=h.default.useState(!1),[s,i]=h.default.useState(""),o=h.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]);h.default.useEffect(()=>{i("")},[e.authorizationUrl,e.gateRef,e.runId]);let u=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),c=h.default.useCallback(()=>{if(!o){i(a("authGate.serviceUnavailable"));return}i(""),window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0)},[e.authorizationUrl,o]),d=n?a("authGate.reopenAuthorization",{provider:u}):a("authGate.openAuthorization",{provider:u});return l`
    <${ni}
      icon="link"
      headline=${e?.headline||a("authGate.oauthTitle")}
      provider=${e?.provider?u:""}
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
          <${O} name="link" className="h-4 w-4" />
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
  `}function k1({gate:e,onSubmit:t,onCancel:a}){let n=C(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState(!1),d=h.default.useCallback(async m=>{m.preventDefault();let f=r.trim();if(!f){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(f),s("")}catch(p){o(p?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return l`
    <${ni}
      icon="lock"
      headline=${e?.headline||n("authGate.title")}
      provider=${e?.provider||""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      pillHint=${n("authGate.pillEnterToken")}
    >
      <form onSubmit=${d}>
        <div className="mb-3">
          <${At}
            type="password"
            autoComplete="off"
            spellCheck=${!1}
            value=${r}
            disabled=${u}
            placeholder=${n("authGate.tokenPlaceholder")}
            aria-label=${n("authGate.tokenLabel")}
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
  `}var HA="/api/webchat/v2/extensions/pairing/redeem";function R1(e){return H(HA,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function Tc({action:e}){let t=C(),a=J(),n=Q({mutationFn:({code:u})=>R1(u),onSuccess:()=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=h.default.useState(""),i=QA(e,t),o=()=>{let u=r.trim();u&&(n.mutate({code:u}),s(""))};return l`
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
        <${A}
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
        ${VA(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function QA(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function VA(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function GA(e,t){return e?.channel==="slack"&&e.strategy===t}function C1({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return l`
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
            <${O} name="close" className="h-4 w-4" />
          </button>
        `}
      </div>

      ${GA(e,"inbound_proof_code")?l`<${Tc} action=${e.action} />`:l`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function YA(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):jr.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:jr.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:jr.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:jr.maxTotalBytes}:jr}function E1(){let e=ba(),t=I({enabled:!!e,queryKey:["session"],queryFn:lc,staleTime:5*6e4});return YA(t.data)}function Ac({onSend:e,onCancel:t,disabled:a,sendDisabled:n=a,canCancel:r=!1,initialText:s="",resetKey:i="",draftKey:o=Vo,variant:u="dock",context:c={},statusText:d=""}){let m=C(),f=u==="hero",p=E1(),[x,y]=h.default.useState(()=>Fp(o)),[w,g]=h.default.useState(()=>zp(o)),[v,b]=h.default.useState(""),[$,S]=h.default.useState(!1),[E,_]=h.default.useState(!1),[T,L]=h.default.useState(!1),D=h.default.useRef(null),P=h.default.useRef(null),R=h.default.useRef(!1),F=a||n||$;R.current=F;let G=h.default.useRef([]),se=h.default.useRef(Promise.resolve());h.default.useEffect(()=>{G.current=w},[w]);let ce=h.default.useRef(null),oe=h.default.useRef(null),Je=h.default.useCallback(()=>{oe.current&&(window.clearTimeout(oe.current),oe.current=null);let k=ce.current;ce.current=null,k&&k.scope===Nt()&&Bp(k.key,k.text)},[]),He=h.default.useCallback(()=>{oe.current&&(window.clearTimeout(oe.current),oe.current=null),ce.current=null},[]),yt=h.default.useCallback(()=>{let k=D.current;k&&(k.style.height="auto",k.style.height=`${Math.min(k.scrollHeight,200)}px`)},[]);h.default.useEffect(()=>{yt()},[x,yt]),h.default.useEffect(()=>(y(Fp(o)),()=>Je()),[o,Je]);let pt=h.default.useRef(o);h.default.useEffect(()=>{if(pt.current!==o){pt.current=o,g(zp(o)),b("");return}$$(o,w)},[o,w]),h.default.useEffect(()=>{s&&(y(s),window.requestAnimationFrame(()=>{D.current&&(D.current.focus(),D.current.setSelectionRange(s.length,s.length))}))},[s,i]);let Ue=h.default.useCallback(k=>{a||!k||k.length===0||(se.current=se.current.then(async()=>{let{staged:M,errors:K}=await l$(k,{limits:p,existing:G.current,t:m});M.length>0&&g(B=>{let j=[...B,...M];return G.current=j,j}),b(K.length>0?K.join(" "):"")}).catch(()=>{b(m("chat.attachmentStagingFailed"))}))},[a,p,m]),tn=h.default.useCallback(k=>{g(M=>{let K=M.filter(B=>B.id!==k);return G.current=K,K}),b("")},[]),Aa=h.default.useCallback(()=>{a||P.current?.click()},[a]),xa=h.default.useCallback(k=>{let M=Array.from(k.target.files||[]);Ue(M),k.target.value=""},[Ue]),Da=h.default.useCallback(async()=>{if(!(!x.trim()||R.current)){R.current=!0,S(!0);try{if(await e(x.trim(),{attachments:w})===null)return;y(""),g([]),G.current=[],b(""),He(),x$(o),w$(o),D.current&&(D.current.style.height="auto")}catch{}finally{R.current=a||n,S(!1)}}},[x,w,e,o,He,a,n]),Ma=h.default.useCallback(k=>{let M=k.target.value;y(M),ce.current={key:o,text:M,scope:Nt()},oe.current&&window.clearTimeout(oe.current),oe.current=window.setTimeout(Je,300)},[o,Je]),Oa=h.default.useCallback(async()=>{if(!(!r||E||!t)){_(!0);try{await t()}finally{_(!1)}}},[r,E,t]),$a=h.default.useCallback(k=>{if(k.key==="Enter"&&!k.shiftKey){if(k.preventDefault(),D.current?.dataset?.sendDisabled==="true"||R.current)return;Da()}},[Da]),lt=h.default.useCallback(k=>{let M=Array.from(k.clipboardData?.files||[]);M.length>0&&(k.preventDefault(),Ue(M))},[Ue]),sa=h.default.useCallback(k=>{k.preventDefault(),L(!1);let M=Array.from(k.dataTransfer?.files||[]);M.length>0&&Ue(M)},[Ue]),wa=h.default.useCallback(k=>{k.preventDefault(),!a&&L(!0)},[a]),It=h.default.useCallback(k=>{k.currentTarget.contains(k.relatedTarget)||L(!1)},[]),ae=x.trim(),ne=a||n,Ee=m(f?"chat.heroPlaceholder":"chat.followUpPlaceholder"),je=p.accept.length>0?p.accept.join(","):void 0,bt=f?"w-full":"px-4 py-3 sm:px-5 lg:px-8",Be=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",f?"min-h-[120px]":"",a?"opacity-70":""].join(" "),N=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",f?"min-h-[72px]":"min-h-[40px]"].join(" ");return l`
    <div className=${bt}>
      <div
        className=${Be}
        onDrop=${sa}
        onDragOver=${wa}
        onDragLeave=${It}
      >
        ${T&&l`
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
              <${O} name="close" className="h-3.5 w-3.5" strokeWidth=${2} />
            </button>
          </div>
        `}

        ${w.length>0&&l`
          <div className="mb-2 flex flex-wrap gap-2 px-1">
            ${w.map(k=>l`
                <div
                  key=${k.id}
                  className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
                >
                  ${k.previewUrl?l`<img
                        src=${k.previewUrl}
                        alt=${k.filename}
                        className="h-9 w-9 shrink-0 rounded object-cover"
                      />`:l`<span
                        className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                      >
                        <${O} name="file" className="h-4 w-4" />
                      </span>`}
                  <span className="flex min-w-0 flex-col">
                    <span className="max-w-[12rem] truncate font-medium">
                      ${k.filename}
                    </span>
                    <span className="text-[10px] text-iron-400">${k.sizeLabel}</span>
                  </span>
                  <button
                    type="button"
                    onClick=${()=>tn(k.id)}
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
          ref=${D}
          data-testid="chat-composer"
          value=${x}
          onChange=${Ma}
          onKeyDown=${$a}
          onPaste=${lt}
          data-send-disabled=${ne?"true":"false"}
          placeholder=${Ee}
          rows=${1}
          disabled=${a}
          className=${N}
        />

        <input
          ref=${P}
          type="file"
          multiple
          accept=${je}
          className="hidden"
          onChange=${xa}
        />

        <div className="mt-2 flex items-center gap-2">
          ${ne&&l`
            <span className="inline-flex items-center gap-2 text-xs text-[var(--v2-text-muted)]">
              <span className="h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
              ${d||m("chat.statusWorking")}
            </span>
          `}
          <div className="ml-auto flex items-center gap-1.5">
            <button
              type="button"
              onClick=${Aa}
              disabled=${a}
              aria-label=${m("chat.attachFiles")}
              title=${m("chat.attachFiles")}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <${O} name="plus" className="h-5 w-5" />
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
                  <${O} name="close" className="h-5 w-5" />
                <//>
              `:l`
                <${A}
                  type="button"
                  variant="primary"
                  size="icon-sm"
                  onClick=${Da}
                  disabled=${ne||$||!ae}
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
  `}var T1={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function A1({status:e}){let t=C();if(e==="idle"||e==="connecting"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return l`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",T1[e]||T1.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function D1({onSuggestion:e,onSend:t,disabled:a,sendDisabled:n,initialText:r,resetKey:s,draftKey:i,context:o,statusText:u,canCancel:c,onCancel:d}){let m=C(),f=[{icon:"tool",title:m("chat.suggestion1"),detail:m("chat.suggestion1Desc")},{icon:"shield",title:m("chat.suggestion2"),detail:m("chat.suggestion2Desc")},{icon:"plug",title:m("chat.suggestion3"),detail:m("chat.suggestion3Desc")}];return l`
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
        <${Ac}
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
        ${f.map(p=>l`
            <button
              type="button"
              key=${p.title}
              onClick=${()=>e(p.title)}
              className="v2-button group grid grid-cols-[auto_1fr_auto] items-center gap-3 border-t border-white/10 px-2 py-4 text-left hover:border-signal/35"
            >
              <span
                className="grid h-8 w-8 place-items-center rounded-full border border-white/10 bg-white/[0.035] text-iron-300 group-hover:border-signal/35 group-hover:text-signal"
              >
                <${O} name=${p.icon} className="h-4 w-4" />
              </span>
              <span className="min-w-0">
                <span className="block text-sm font-semibold text-iron-100">
                  ${p.title}
                </span>
                <span className="mt-0.5 block text-sm text-iron-300">
                  ${p.detail}
                </span>
              </span>
            </button>
          `)}
      </div>
    </div>
  `}var JA=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function M1({open:e,onClose:t}){let a=C();return e?l`
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
          ${JA.map((n,r)=>l`
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
  `:null}function L1(e){let t=0,a=0,n=0,r=0,s=0;for(let o of e){if(o.role==="thinking"&&(t+=1),o.role==="tool_activity"){let u=O1([o]);a+=u.tools,n+=u.failed,r+=u.declined,s+=u.running}if(XA(o)){let u=O1(o.toolCalls);a+=u.tools,n+=u.failed,r+=u.declined,s+=u.running}}let i=[];return t&&i.push(`${t} reasoning`),a&&i.push(`${a} ${a===1?"tool":"tools"}`),n&&i.push(`${n} failed`),r&&i.push(`${r} declined`),!n&&!r&&s&&i.push("running"),{hasError:n>0,hasDeclined:r>0,label:`Activity${i.length?` - ${i.join(", ")}`:""}`}}function O1(e){let t=0,a=0,n=0;for(let r of e)r.toolStatus==="error"&&(t+=1),r.toolStatus==="declined"&&(a+=1),r.toolStatus==="running"&&(n+=1);return{tools:e.length,failed:t,declined:a,running:n}}function XA(e){return e.toolCalls&&e.toolCalls.length>0}var P1=!1;function ZA(){P1||!window.DOMPurify||(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),P1=!0)}function U1(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}ZA();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var sh=360;function WA(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let u=s("Copy");if(u.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),u.textContent="Copied",ei("Code copied",{tone:"success"}),setTimeout(()=>u.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(u),n.appendChild(r),t.scrollHeight>sh){t.style.maxHeight=`${sh}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${sh}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function e4({content:e,className:t=""}){let a=h.default.useRef(null),n=h.default.useMemo(()=>U1(e),[e]);return h.default.useEffect(()=>{WA(a.current)},[n]),l`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var aa=h.default.memo(e4);var j1={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",declined:"bg-iron-400",error:"bg-[var(--v2-danger-text)]"},t4={success:"ok",declined:"declined",error:"err",running:"run"},a4=2;function ri({activity:e}){return e.toolCalls&&e.toolCalls.length>0?l`<${r4} tools=${e.toolCalls} />`:l`<${s4} activity=${e} />`}function n4(e,t){let a=0,n=0,r=0,s=0;for(let u of t){let c=String(u.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function r4({tools:e}){let t=C(),a=e.some(o=>o.toolStatus==="error"),n=e.some(o=>o.toolStatus==="error"||o.toolStatus==="declined"),[r,s]=h.default.useState(n);if(h.default.useEffect(()=>{n&&s(!0)},[n]),e.length<=a4)return l`
      <div className="flex flex-col gap-3">
        ${e.map((o,u)=>l`<${ri}
            key=${o.id||o.callId||`${o.toolName}-${u}`}
            activity=${o}
          />`)}
      </div>
    `;let i=n4(t,e);return l`
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

      ${r&&l`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((o,u)=>l`<${ri}
              key=${o.id||o.callId||`${o.toolName}-${u}`}
              activity=${o}
            />`)}
        </div>
      `}
    </div>
  `}function s4({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:u}=e,[c,d]=h.default.useState(n==="error"||n==="declined");h.default.useEffect(()=>{(n==="error"||n==="declined")&&d(!0)},[n]);let m=j1[n]||j1.running,f=i!=null,p=h.default.useId(),x=l`
    <button
      type="button"
      onClick=${()=>d(y=>!y)}
      aria-expanded=${c?"true":"false"}
      aria-controls=${p}
      className="v2-button flex w-full items-center gap-2.5 border-0 border-b border-iron-700/40 bg-transparent px-1 py-2 text-left text-sm"
    >
      <span className=${["h-2 w-2 shrink-0 rounded-full",m].join(" ")} />
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-wide text-iron-300"
        >${t4[n]||"run"}</span
      >
      <span className="shrink-0 truncate font-mono text-[13px] font-medium text-iron-100"
        >${a}</span
      >
      ${r&&l`<span className="min-w-0 truncate font-mono text-xs text-iron-400"
        >${r}</span
      >`}
      <span className="ml-auto flex shrink-0 items-center gap-2">
        ${f&&l`<span className="font-mono text-[11px] text-iron-300">${i}ms</span>`}
        <${O}
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
          <${O} name="tool" className="h-4 w-4" />
        </div>
      `}
      <div className=${t?"min-w-0 flex-1":"min-w-0 max-w-[85%] flex-1"}>
        ${x}
        ${c&&l`<${i4}
          controlsId=${p}
          toolDetail=${r}
          toolParameters=${o}
          toolResultPreview=${u}
          toolError=${s}
          toolStatus=${n}
          toolDurationMs=${f?i:null}
        />`}
      </div>
    </div>
  `}function i4({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=C(),u=h.default.useMemo(()=>{let f=[];return r&&f.push({id:s==="declined"?"declined":"error",label:o(s==="declined"?"tool.tabDeclined":"tool.tabError")}),t&&f.push({id:"details",label:o("tool.tabDetails")}),a&&f.push({id:"params",label:o("tool.tabParameters")}),n&&f.push({id:"result",label:o("tool.tabResult")}),f},[o,r,t,a,n,s]),[c,d]=h.default.useState(null),m=c&&u.some(f=>f.id===c)?c:u[0]?.id;return h.default.useEffect(()=>{r&&d(s==="declined"?"declined":"error")},[r,s]),u.length===0?l`
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
        ${m==="result"&&l`<${o4} text=${n} />`}
        ${(m==="error"||m==="declined")&&l`<pre
          className=${["overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono",m==="declined"?"text-iron-300":"text-[var(--v2-danger-text)]"].join(" ")}
        >${r}</pre>`}
      </div>
    </div>
  `}function o4({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return l`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(l4)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return l`
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
                  >${u4(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?l`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:l`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function l4(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function u4(e){return e==null?"":String(e)}function F1({activity:e}){let t=L1(e),a=m4(e),[n,r]=h.default.useState(a);return h.default.useEffect(()=>{a&&r(!0)},[a]),l`
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

      ${n&&l`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((s,i)=>l`
            <${c4}
              key=${s.id||`${s.role||"activity"}-${i}`}
              item=${s}
            />
          `)}
        </div>
      `}
    </div>
  `}function c4({item:e}){if(e.role==="thinking")return l`<${d4} content=${e.content} />`;if(e.role==="tool_activity"||ih(e)){let t=ih(e)?{id:e.id,toolCalls:e.toolCalls}:e;return l`<${ri} activity=${t} />`}return null}function d4({content:e}){return e?l`
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
  `:null}function ih(e){return e?.toolCalls&&e.toolCalls.length>0}function m4(e){return(e||[]).some(t=>t?.role==="thinking"||t?.toolStatus==="running"||t?.toolStatus==="error"||t?.toolStatus==="declined"?!0:ih(t)?t.toolCalls.some(a=>a?.toolStatus==="running"||a?.toolStatus==="error"||a?.toolStatus==="declined"):!1)}function Dc(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function f4({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=h.default.useState(()=>t&&e.preview_url||null);return h.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return dc(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?l`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:l`<${O} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var B1="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",z1="px-3 py-2";function Mc({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=h.default.useState(!1),o=h.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await Ra(e.fetch_url);Dc(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),u=l`
    <${f4} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?l`<div
      className=${`${B1} ${z1} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${u}
    </div>`:l`<div className=${`${B1} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${z1} text-left transition-colors hover:bg-iron-900/80`}
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
      <${O} name="download" className="h-3.5 w-3.5" />
    </button>`}
  </div>`}var q1={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function si({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return h.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),h.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?l`
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
        className=${V("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",q1[n]??q1.md,r)}
      >
        ${a?l`<${oh} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function oh({children:e,onClose:t,className:a=""}){return l`
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
            <${O} name="close" className="h-4 w-4" />
          </button>
        `}
    </div>
  `}function ii({children:e,className:t=""}){return l`
    <div className=${V("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function oi({children:e,className:t=""}){return l`
    <div
      className=${V("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var I1=1e5;function Oc({attachment:e,onClose:t}){let a=!!e,[n,r]=h.default.useState("loading"),[s,i]=h.default.useState({}),o=e?o$(e.mime_type):"download";if(h.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return Ra(e.fetch_url).then(async m=>{d=URL.createObjectURL(m);let f={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")f.dataUrl=await Tp(m);else if(o==="pdf")f.frameUrl=d;else if(o==="text"){let p=await m.text();f.truncated=p.length>I1,f.text=f.truncated?p.slice(0,I1):p}if(c){URL.revokeObjectURL(d);return}i(f),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let u=e.filename||"attachment";return l`
    <${si} open=${a} onClose=${t} size="xl">
      <${oh} onClose=${t}>
        <span className="block truncate">${u}</span>
      <//>
      <${ii} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&l`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&l`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&l`<${p4} mode=${o} view=${s} filename=${u} />`}
      <//>
      <${oi}>
        ${s.downloadUrl&&l`<a
          href=${s.downloadUrl}
          download=${u}
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
  `}function p4({mode:e,view:t,filename:a}){switch(e){case"image":return l`<img
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
        <${O} name="file" className="h-10 w-10 text-signal" />
        <div className="text-sm">This file type can't be previewed.</div>
      </div>`}}var h4=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function v4(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function K1(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of v4(e).matchAll(h4)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function H1(e){return e.split("/").filter(Boolean).pop()||e}function Q1(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function g4({threadId:e,path:t,onPreview:a}){let[n,r]=h.default.useState({mime_type:"",size_label:""});h.default.useEffect(()=>{let i=!0;return Ex({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:Q1(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:H1(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:cc({threadId:e,path:t})};return l`<${Mc}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function V1({threadId:e,content:t}){let a=h.default.useMemo(()=>K1(t),[t]),[n,r]=h.default.useState(null);return!e||a.length===0?null:l`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>l`<${g4}
          key=${s}
          threadId=${e}
          path=${s}
          onPreview=${r}
        />`)}
      <${Oc}
        attachment=${n}
        onClose=${()=>r(null)}
      />
    </div>
  `}var G1={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function y4(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function b4({content:e}){let[t,a]=h.default.useState(!1);return e?l`
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
      ${t&&l`
        <div className="mt-1 border-l-2 border-white/10 pl-3 text-iron-300">
          <${aa} content=${e} className="text-[13px]" />
        </div>
      `}
    </div>
  `:null}function x4({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:u,status:c,error:d,toolCalls:m,timestamp:f}=e,p=n==="user",[x,y]=h.default.useState(!1),[w,g]=h.default.useState(null),v=h.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),y(!0),ei("Copied to clipboard",{tone:"success"}),setTimeout(()=>y(!1),1400)}catch{}},[r]);if(n==="tool_activity"||m&&m.length>0){let D=m&&m.length>0?{id:e.id,toolCalls:m}:e;return l`<${ri} activity=${D} />`}if(n==="thinking")return l`<${b4} content=${r} />`;if(n==="image")return l`
      <div className="flex">
        <div className="flex flex-wrap gap-2">
          ${(o||[]).map((P,R)=>P.data_url?l`<img key=${R} src=${P.data_url} className="max-h-64 rounded-lg border border-iron-700 object-cover" alt="Generated result" />`:l`
                  <div key=${R} className="rounded-lg border border-iron-700 bg-iron-900/70 px-4 py-3 text-sm text-iron-200">
                    <div>Generated image unavailable in history payload</div>
                    ${P.path&&l`<div className="mt-1 font-mono text-xs text-iron-300">${P.path}</div>`}
                  </div>
                `)}
        </div>
      </div>
    `;let b=y4(f),$=n==="user"||n==="assistant"&&!u,S=n==="system"||n==="error",E=p?"max-w-[85%]":S?"mx-auto max-w-[85%]":"w-full max-w-[85%]",_=p?"":"w-full min-w-0 max-w-full",T=c==="error"&&t,L=$||T||b;return l`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",p?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col",E].join(" ")}>
        <div
          className=${["text-base leading-7",_,G1[n]||G1.assistant,u?"opacity-70":""].join(" ")}
        >
          ${n==="assistant"||n==="system"||n==="error"?l`<${aa} content=${r} />`:l`<div className="whitespace-pre-wrap">${r}</div>`}

          ${c==="error"&&l`
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-300">
              <span>${d}</span>
            </div>
          `}

          ${s&&s.length>0&&l`
            <div className="mt-2 flex flex-wrap gap-2">
              ${s.map((D,P)=>l`<img key=${P} src=${D} className="max-h-48 rounded-lg border border-iron-700 object-cover" alt="Message attachment" />`)}
            </div>
          `}

          ${i&&i.length>0&&l`
            <div className="mt-2 flex flex-col gap-1.5">
              ${i.map((D,P)=>l`<${Mc}
                key=${D.id||P}
                att=${D}
                onPreview=${g}
              />`)}
            </div>
            <${Oc}
              attachment=${w}
              onClose=${()=>g(null)}
            />
          `}

          ${n==="assistant"&&l`<${V1}
            threadId=${a}
            content=${typeof r=="string"?r:""}
          />`}
        </div>
      </div>

      ${L&&l`
        <div
          className=${["mt-1 flex min-h-7 w-max max-w-[85%] flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100",p?"self-end justify-end":S?"self-center justify-center":"self-start justify-start"].join(" ")}
        >
          ${b&&l`<time dateTime=${f} className="shrink-0 font-mono text-[11px] text-iron-500">${b}</time>`}
          ${($||T)&&l`
            <div className="flex shrink-0 items-center gap-1">
            ${$&&l`
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
            ${T&&l`
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
  `}var Y1=h.default.memo(x4);function t2(e){let t=$4(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(a2(r)){let s=J1(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){X1(a,s),Z1(a,r),n+=s.length;continue}}if(lh(r)){let s=J1(t,n);X1(a,s),n+=s.length-1;continue}Z1(a,r)}return a}function $4(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=Lc(i);o&&a2(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!lh(i))continue;let o=Lc(i),u=o?t.get(o):void 0;if(u===void 0||u>=s)continue;let c=a.get(u)||[];c.push(i),a.set(u,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function J1(e,t){let a=t,n=Lc(e[t]);for(;a<e.length&&lh(e[a])&&w4(n,e[a]);)a+=1;return e.slice(t,a)}function w4(e,t){let a=Lc(t);return!e||!a||a===e}function X1(e,t){if(t.length===0)return;let a=S4(t);e.push({type:"activity-run",id:`activity-run-${a[0].id}`,activity:a})}function Z1(e,t){e.push({type:"message",id:t.id,message:t})}function a2(e){return e.role==="assistant"&&!n2(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function lh(e){return e.role==="thinking"||e.role==="tool_activity"||n2(e)}function n2(e){return e?.toolCalls&&e.toolCalls.length>0}function Lc(e){return e?.turnRunId||null}function S4(e){return[...e].sort((t,a)=>t?.role!=="tool_activity"||a?.role!=="tool_activity"?0:N4(t,a))}function N4(e,t){if(Number.isFinite(e.activityOrder)&&Number.isFinite(t.activityOrder)){let n=e.activityOrder-t.activityOrder;if(n!==0)return n}let a=W1(e2(e.updatedAt||e.timestamp),e2(t.updatedAt||t.timestamp));return a!==0?a:W1(e.sequence,t.sequence)}function W1(e,t){let a=Number.isFinite(e)?e:null,n=Number.isFinite(t)?t:null;return a===null&&n===null?0:a===null?1:n===null?-1:a-n}function e2(e){if(!e)return null;let t=Date.parse(e);return Number.isFinite(t)?t:null}var _4=100,k4=100;function R4(e){return e?e.scrollHeight-e.scrollTop-e.clientHeight:Number.POSITIVE_INFINITY}function r2(e,t=_4){return R4(e)<=t}function s2(e){e&&(e.scrollTop=Math.max(0,e.scrollHeight-e.clientHeight))}function i2(e){return e?.id?`${e.role||""}:${e.id}`:null}function C4(e,t){let a=i2(t);return!!(a&&t?.role==="user"&&a!==e)}function o2({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let u=C(),c=h.default.useRef(null),d=h.default.useRef(null),m=h.default.useRef(!0),f=h.default.useRef(null),p=h.default.useRef(null),x=h.default.useRef(null),y=h.default.useRef(0),w=h.default.useRef(!1),[g,v]=h.default.useState(!0),b=h.default.useCallback(()=>{p.current!==null&&(window.cancelAnimationFrame(p.current),p.current=null)},[]),$=h.default.useCallback((R=!1)=>{c.current&&(R&&(m.current=!0,w.current=!1),m.current&&(b(),p.current=window.requestAnimationFrame(()=>{p.current=null;let G=c.current;!G||!R&&!m.current||(s2(G),y.current=G.scrollTop,w.current=!1,v(!0))})))},[b]),S=h.default.useCallback(()=>{x.current!==null&&(window.cancelAnimationFrame(x.current),x.current=null)},[]);h.default.useLayoutEffect(()=>{let R=e.length>0?e[e.length-1]:null,F=i2(R),G=C4(f.current,R);return f.current=F,$(G),b},[e,i,$,b]),h.default.useLayoutEffect(()=>{let R=d.current;if(!R||typeof ResizeObserver!="function")return;let F=new ResizeObserver(()=>{$()});return F.observe(R),()=>{F.disconnect(),b()}},[$,b]);let E=h.default.useCallback(()=>{x.current=null;let R=c.current;if(!R)return;let F=r2(R);y.current=R.scrollTop,F?(m.current=!0,w.current=!1,v(!0)):w.current?(m.current=!1,v(!1)):(m.current=!0,v(!0),$()),a&&R.scrollTop<k4&&n&&!t&&n()},[a,n,t,$]),_=h.default.useCallback(()=>{w.current=!0},[]),T=h.default.useCallback(R=>{let F=c.current;if(!F||typeof R?.clientX!="number")return;let G=F.offsetWidth-F.clientWidth;if(G<=0)return;let se=F.getBoundingClientRect().right;R.clientX>=se-G-2&&(w.current=!0)},[]),L=h.default.useCallback(()=>{let R=c.current;if(!R)return;let F=r2(R),G=R.scrollTop<y.current;y.current=R.scrollTop,!F&&G&&(w.current=!0),F?(m.current=!0,w.current=!1):w.current?(m.current=!1,b()):m.current=!0,x.current===null&&(x.current=window.requestAnimationFrame(E))},[b,E]),D=h.default.useCallback(()=>{let R=c.current;R&&(s2(R),y.current=R.scrollTop,m.current=!0,w.current=!1,v(!0))},[]);h.default.useEffect(()=>S,[S]);let P=h.default.useMemo(()=>t2(e),[e]);return l`
    <div className="relative flex min-h-0 min-w-0 flex-1">
    <div
      ref=${c}
      onScroll=${L}
      onWheel=${_}
      onTouchMove=${_}
      onPointerDown=${T}
      className="flex min-w-0 flex-1 overflow-y-auto px-4 pt-6 pb-14 sm:px-5 lg:px-8"
    >
      <div ref=${d} className="mx-auto flex w-full min-w-0 max-w-5xl flex-col gap-5">
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
        ${P.map(R=>R.type==="activity-run"?l`<${F1} key=${R.id} activity=${R.activity} />`:l`<${Y1}
                key=${R.id}
                message=${R.message}
                onRetry=${r}
                threadId=${s}
              />`)}
        ${o}
      </div>
    </div>
    ${!g&&l`
      <button
        type="button"
        onClick=${D}
        aria-label=${u("chat.jumpToLatest")}
        className="absolute bottom-4 left-1/2 inline-flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-3 py-1.5 text-xs font-medium text-[var(--v2-text-strong)] shadow-[0_10px_30px_-12px_rgba(0,0,0,0.7)] hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]"
      >
        <${O} name="arrowDown" className="h-3.5 w-3.5" />
        ${u("chat.jumpToLatest")}
      </button>
    `}
    </div>
  `}function l2({notice:e,onRecover:t}){return l`
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
  `}function u2({suggestions:e,onSelect:t,disabled:a=!1}){return!e||e.length===0?null:l`
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
  `}function c2(){return l`
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
  `}function Pc(){return H("/api/webchat/v2/channels/connectable")}function d2(e,t){if(!uh(e))return null;let a=Uc(e),n=D4(a),r=null;for(let s of t||[]){if(!A4(s))continue;let i=M4(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function uh(e){let t=Uc(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function E4(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function T4(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>m2(Uc(n))):a}function A4(e){return e?.strategy!=="admin_managed_channels"}function D4(e){return f2(e,"slack")&&m2(e)}function m2(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function Uc(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function M4(e,t,a={}){return(a.commandAliasesOnly?T4(t,{channelManagementOnly:!0}):E4(t)).reduce((r,s)=>{let i=Uc(s);return f2(e,i)?Math.max(r,i.length):r},0)}function f2(e,t){return t?` ${e} `.includes(` ${t} `):!1}function p2(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n={kind:"gate",gateKind:"approval",runId:t.turn_run_id,gateRef:t.gate_ref,invocationId:t.invocation_id||null,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return O4(n,a,t.body)}return e==="auth_required"?{kind:"auth_required",gateKind:"auth",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,invocationId:t.invocation_id||null,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function h2(e){if(!e?.run_id||!e.gate_ref)return null;let t=e.gate_kind||"generic",a={gateKind:t,runId:e.run_id,gateRef:e.gate_ref,invocationId:e.invocation_id||null,headline:e.headline,body:e.body||"",allowAlways:e.allow_always===!0};if(t==="auth"){let n=e.auth_context||{};return{...a,kind:"auth_required",challengeKind:n.challenge_kind||"other",provider:n.provider||null,accountLabel:n.account_label||"",authorizationUrl:n.authorization_url||null,expiresAt:n.expires_at||null}}return{...a,kind:"gate"}}function O4(e,t,a){if(!t)return e;let n=L4(t);return{...e,toolName:t.tool_name||null,description:t.reason||a,actionLabel:t.action?.label||null,destination:t.destination||null,approvalScope:t.scope||null,approvalDetails:n,parameters:n.length?n.map(r=>`${r.label}: ${r.value}`).join(`
`):null}}function L4(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function v2({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function g2(){return{terminalByInvocation:new Map}}function y2(e){e?.current?.terminalByInvocation?.clear()}function dh(e,t,a){let n=x2(t,{toolStatus:"running"});n&&li(e,n,a)}function b2(e,t,a,n="gate_declined"){let r=x2(t,{toolStatus:"declined",toolError:n,toolErrorKind:"gate_declined"});r&&li(e,r,a)}function li(e,t,a){if(!t)return;let n=z4(t);n=B4(n,a),e(r=>{let s=$2(n),i=U4(r,n,s);if(i>=0){let u=[...r];return u[i]=j4(u[i],n),ch(u[i],a),u}let o={id:s,role:"tool_activity",...n};return ch(o,a),[...r,o]})}function x2(e,t={}){let a=e?.kind==="gate",n=e?.kind==="auth_required",r=n&&t.toolStatus==="declined";if(!e?.runId||!e?.gateRef||!e.invocationId&&!r||!a&&!n)return null;let s=e.invocationId||P4(e),i=e.toolName||e.headline||e.gateKind||"gate";return{invocationId:s,callId:s,capabilityId:e.toolName||e.gateKind||null,toolName:Io(i)||i,toolStatus:t.toolStatus||"running",toolDetail:null,toolParameters:null,toolResultPreview:null,toolError:t.toolError||null,toolErrorKind:t.toolErrorKind||null,toolDurationMs:null,updatedAt:t.updatedAt||new Date().toISOString(),resultRef:null,truncated:!1,outputBytes:null,outputKind:null,turnRunId:e.runId,gateRef:e.gateRef,gateActivity:!0}}function P4(e){return`gate:${e.runId}:${e.kind}:${e.gateRef}`}function $2(e){return`tool-${e.invocationId}`}function U4(e,t,a){let n=e.findIndex(s=>s?.id===a);if(n>=0)return n;let r=t.gateRef||null;if(r){let s=e.findIndex(i=>i?.role==="tool_activity"&&i.turnRunId===t.turnRunId&&i.gateRef===r);if(s>=0)return s}return-1}function j4(e,t){let a=qo(e.toolStatus),n=qo(t.toolStatus),r=a&&!n,s=t.gateActivity&&!e.gateActivity,i={...e,...t,id:e.id,role:"tool_activity",invocationId:e.gateActivity&&!t.gateActivity?t.invocationId:e.invocationId||t.invocationId,callId:e.gateActivity&&!t.gateActivity?t.callId:e.callId||t.callId,toolName:s?e.toolName:t.toolName||e.toolName,toolStatus:r?e.toolStatus:t.toolStatus,toolError:t.toolError||e.toolError,toolErrorKind:t.toolErrorKind||e.toolErrorKind||null,updatedAt:r?e.updatedAt||t.updatedAt:t.updatedAt||e.updatedAt,turnRunId:t.turnRunId||e.turnRunId||null,gateRef:t.gateRef||e.gateRef||null,gateActivity:e.gateActivity&&t.gateActivity,capabilityId:s?e.capabilityId||t.capabilityId||null:t.capabilityId||e.capabilityId||null,activityOrder:F4(e,t),activityOrderSource:t.activityOrderSource||e.activityOrderSource||null};return e.gateActivity&&!t.gateActivity&&(i.id=$2(t),i.gateActivity=!1),i}function F4(e,t){return Number.isFinite(t.activityOrder)?t.activityOrder:e.activityOrder}function B4(e,t){if(!e?.invocationId)return e;if(qo(e.toolStatus))return ch(e,t),e;let a=t?.current?.terminalByInvocation?.get(e.invocationId);return a?Number.isFinite(e.activityOrder)?{...a,activityOrder:e.activityOrder,activityOrderSource:e.activityOrderSource||a.activityOrderSource||null}:a:e}function ch(e,t){!e?.invocationId||!qo(e.toolStatus)||t?.current?.terminalByInvocation?.set(e.invocationId,e)}function z4(e){let t=Io(e.toolName||e.capabilityId);return{...e,toolName:t||e.toolName||"tool"}}function k2({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o,onRunSettled:u}){let c=h.default.useRef(new Set),d=h.default.useRef(null),m=h.default.useRef(null);return h.default.useCallback(f=>{let{type:p,frame:x}=f||{};if(!(!p||!x))switch(p){case"accepted":{let y=x.ack||{};y.run_id&&(d.current=y.run_id),r?.({runId:y.run_id||null,threadId:y.thread_id||e,status:y.status||null}),a(!0);return}case"running":case"capability_progress":{let y=x.progress||{};y.turn_run_id&&(d.current=y.turn_run_id,r?.(w=>w&&w.runId===y.turn_run_id?{...w,status:"running"}:{runId:y.turn_run_id,threadId:e,status:"running"}),q4(n,y.turn_run_id,m)),a(!0);return}case"capability_activity":{let y=x.activity;if(!y||!y.invocation_id)return;li(t,Up(y),o);return}case"capability_display_preview":{let y=x.preview;if(!y||!y.invocation_id)return;let w=Pp(y);li(t,w,o);return}case"gate":case"auth_required":{let y=p2(p,x.prompt);y&&(dh(t,y,o),n(y),r?.({runId:y.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let y=x.reply||{};t(w=>[...w,{id:`reply-${y.turn_run_id||Date.now()}`,role:"assistant",content:y.text||"",timestamp:y.generated_at||new Date().toISOString(),turnRunId:y.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let y=x.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Bc(c,u,y,!1);return}case"failed":{let y=x.run_state||{},w=y.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),fh(t,{runId:w,status:y.status||"failed",failureCategory:Q4(y),failureSummary:null}),Bc(c,u,w,!1);return}case"projection_snapshot":case"projection_update":{let y=x.state?.items||[];K4({items:y,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:u,settledRunsRef:c,latestRunIdRef:d,promptRunIdRef:m,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i,o,u])}function Bc(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var w2=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),S2=new Set(["completed","succeeded"]),jc=new Set(["blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]),Fc=new Set(["awaiting_gate","blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]);function N2(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function q4(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function I4(e,t,a,n,r,s){let i=t?.runId||null;if(!i||r?.has(i))return!0;let o=a?.get(i);if(o)return!Fc.has(o);let u=e?.current,c=u?.runId||n?.current||null;if(c&&i!==c)return!0;let d=s?.current===c;return c&&i===c&&!d&&u?.status&&!Fc.has(u.status)?!0:!u?.runId||!u.status?!1:!Fc.has(u.status)}function K4({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,activeRunRef:d,locallyResolvedGatesRef:m,toolActivityStateRef:f}){let p=new Map,x=new Set,y=d?.current||null,w=y?.runId||u?.current||null;for(let v of e){let b=v.run_status;b?.run_id&&b.status&&(p.set(b.run_id,b.status),w&&w!==b.run_id&&y?.status&&!w2.has(y.status)&&jc.has(b.status)&&x.add(b.run_id))}let g=u?.current??null;for(let v of e){if(v.run_status){let{run_id:b,status:$,failure_category:S,failure_summary:E}=v.run_status,_=w2.has($),T=d?.current?.source==="local"?d.current.runId:null,L=!!(b&&T&&T!==b),D=g??u?.current??null,P=!!(_&&b&&D&&D!==b),R=b&&jc.has($)?_2(m,b):null;if(b&&x.has(b)||L)continue;if(P){_2(m,d?.current?.runId)?.outcome==="resumed"&&(H4({runId:b,activePromptRunId:d?.current?.runId,success:S2.has($),status:$,failureCategory:S,failureSummary:E,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,locallyResolvedGatesRef:m}),g=null);continue}if(R){N2(r,b,c),R.outcome==="resumed"?(n(!0),s?.(F=>F&&F.runId===b?{...F,status:F.status==="awaiting_gate"?"queued":F.status||"queued"}:{runId:b,threadId:t,status:"queued"}),g=b,u&&(u.current=b)):(n(!1),d?.current?.runId===b&&s?.(null),g=null,u?.current===b&&(u.current=null));continue}b&&(g=b,!_&&u&&(u.current=b),s?.(F=>F&&F.runId===b?{...F,status:$}:{runId:b,threadId:t,status:$})),b&&jc.has($)?c&&(c.current=b):b&&c?.current===b&&(c.current=null),_?(n(!1),r(null),s?.(null),mh(m,b),g=null,u&&(u.current=null),b&&c?.current===b&&(c.current=null),Bc(o,i,b,S2.has($)),($==="failed"||$==="recovery_required")&&fh(a,{runId:b,status:$,failureCategory:S,failureSummary:E})):jc.has($)||(N2(r,b,c),mh(m,b),n(!0))}if(v.text){let b=`text-${v.text.id}`;a($=>{let S=$.findIndex(_=>_.id===b),E={id:b,role:"assistant",content:v.text.body||"",timestamp:new Date().toISOString(),isFinalReply:!0};if(S>=0){let _=[...$];return _[S]=E,_}return[...$,E]}),n(!1)}if(v.thinking){let b=`thinking-${v.thinking.id}`;a($=>{let S=$.findIndex(_=>_.id===b),E={id:b,role:"thinking",content:v.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:v.thinking.run_id||null};if(S>=0){let _=[...$];return _[S]=E,_}return[...$,E]})}if(v.capability_activity){let b=v.capability_activity;b.invocation_id&&li(a,Up(b),f)}if(v.gate){let b=h2(v.gate),$=b?.runId||null;$&&!I4(d,b,p,u,x,c)&&!G4(m,$,b.gateRef)&&(dh(a,b,f),r(S=>S||b),s?.(S=>S&&S.runId===$?{...S,status:Fc.has(S.status)?S.status:"awaiting_gate"}:{runId:$,threadId:t,status:"awaiting_gate"}),c&&(c.current=$),n(!1))}if(v.skill_activation){let{id:b,skill_names:$=[],feedback:S=[]}=v.skill_activation;if($.length||S.length){let E=`skill-${b||$.join("-")||"activation"}`,_=[$.length?`Skill activated: ${$.join(", ")}`:"",...S].filter(Boolean).join(`
`);a(T=>T.some(L=>L.id===E)?T:[...T,{id:E,role:"system",content:_,timestamp:new Date().toISOString()}])}}}u&&g&&(u.current=g)}function H4({runId:e,activePromptRunId:t,success:a,status:n,failureCategory:r,failureSummary:s,setMessages:i,setIsProcessing:o,setPendingGate:u,setActiveRun:c,onRunSettled:d,settledRunsRef:m,latestRunIdRef:f,promptRunIdRef:p,locallyResolvedGatesRef:x}){o(!1),u(null),c?.(null),mh(x,t),f&&(f.current=null),p?.current===t&&(p.current=null),Bc(m,d,e,a),(n==="failed"||n==="recovery_required")&&fh(i,{runId:e,status:n,failureCategory:r,failureSummary:s})}function Q4(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function fh(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`;e(i=>{let o=i.findIndex(c=>c.id===s),u=v2({status:a,failureCategory:n,failureSummary:r});if(o>=0){if(!r||i[o].content===u)return i;let c=[...i];return c[o]={...c[o],content:u},c}return[...i,{id:s,role:"error",content:u,timestamp:new Date().toISOString()}]})}function _2(e,t){if(!t)return null;let a=e?.current;if(!a)return null;for(let[n,r]of a.entries())if(n.startsWith(`${t}
`))return V4(r);return null}function V4(e){return e&&typeof e=="object"?{resolution:e.resolution||null,outcome:e.outcome||null}:{resolution:e||null,outcome:null}}function mh(e,t){if(!t)return;let a=e?.current;if(a)for(let n of Array.from(a.keys()))n.startsWith(`${t}
`)&&a.delete(n)}function G4(e,t,a){return!t||!a?!1:!!e?.current?.has(`${t}
${a}`)}function R2(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function C2(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function E2(e,t,a,n){let r=ph(n);return r?(Y4(e,t,a,{timelineMessageId:r}),r):null}function Y4(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function ph(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var J4=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function T2({threadId:e,onEvent:t,enabled:a}){let[n,r]=h.default.useState("idle"),s=h.default.useRef(t);s.current=t;let i=h.default.useRef(null);return h.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,u=null,c=0,d=3e4;function m(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=Kx({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let y=Math.min(1e3*2**c,d);u=setTimeout(m,y)};let x=(y,w)=>{let g=null;try{g=JSON.parse(y.data)}catch{return}!g||typeof g!="object"||(y.lastEventId&&(i.current=y.lastEventId),s.current?.({type:g.type||w,frame:g,lastEventId:y.lastEventId||null}))};o.onmessage=y=>x(y,"message");for(let y of J4)o.addEventListener(y,w=>x(w,y))}function f(){u&&(clearTimeout(u),u=null),o&&(o.close(),o=null),r("paused")}function p(){document.visibilityState==="hidden"?f():o||m()}return m(),document.addEventListener("visibilitychange",p),()=>{document.removeEventListener("visibilitychange",p),u&&clearTimeout(u),o&&o.close()}},[a,e]),{status:n}}var X4=3e4,Z4="credential_stored_gate_resolution_failed",W4="approval_gate_pending_send_blocked",e5="ironclaw-product-auth",hh="ironclaw:product-auth:oauth-complete",t5="ironclaw:product-auth:oauth-complete";async function A2(e){let t=new AbortController,a=setTimeout(()=>t.abort(),X4);try{return await e(t.signal)}finally{clearTimeout(a)}}function a5(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=Z4,t.cause=e,t}function n5(){let e=new Error("Resolve the approval request before sending another message.");return e.safeErrorCode=W4,e}function r5(e){let a=Et.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function D2(e,t){return!e||!t?.runId||!t?.gateRef?null:`${e}
${t.runId}
${t.gateRef}`}function s5(e){return e?.continuation?.type==="turn_gate_resume"}function i5(e){if(e?.outcome)return e.outcome;let t=String(e?.status||"").toLowerCase();return t==="queued"||t==="running"?"resumed":t==="cancelled"||e?.already_terminal===!0?"cancelled":e?.already_terminal===!1?"resumed":null}function M2(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function o5(e){return e?.type===t5&&e?.status==="completed"}function l5(e,t,a){if(!o5(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function vh(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function u5(e){if(!uh(e))return null;try{let a=(await Et.fetchQuery({queryKey:["connectable-channels"],queryFn:Pc}))?.channels||[];return d2(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function O2(e){let t=h.default.useRef(e),a=h.default.useRef(new Map),n=h.default.useRef(1),[r,s]=h.default.useState(0),[i,o]=h.default.useState(Date.now()),[u,c]=h.default.useState(null),d=h.default.useRef(u),m=h.default.useCallback(ae=>{let ne=typeof ae=="function"?ae(d.current):ae;d.current=ne,c(ne)},[]);h.default.useEffect(()=>{d.current=u},[u]);let[f,p]=h.default.useState(null),x=h.default.useCallback(()=>a.current.get(e||"__new__")||[],[e]),y=h.default.useCallback(ae=>{let ne=e||"__new__";ae.length>0?a.current.set(ne,ae):a.current.delete(ne)},[e]),{messages:w,hasMore:g,nextCursor:v,isLoading:b,loadError:$,loadHistory:S,seedThreadMessages:E,setMessages:_}=y$(e,{getPendingMessages:x,setPendingMessages:y}),[T,L]=h.default.useState(!1),D=h.default.useRef(T),P=h.default.useCallback(ae=>{let ne=typeof ae=="function"?ae(D.current):ae;D.current=ne,L(ne)},[]),[R,F]=h.default.useState(null),G=h.default.useRef(R),[se,ce]=h.default.useState(null),oe=h.default.useCallback(ae=>{let ne=G.current,Ee=typeof ae=="function"?ae(ne):ae;Object.is(Ee,ne)||(G.current=Ee,F(Ee))},[]),[Je,He]=h.default.useState(e),yt=h.default.useRef(g2()),pt=h.default.useRef(new Map),Ue=h.default.useRef({gateKey:null,credentialRef:null,inFlight:!1});Je!==e&&(He(e),L(!1),F(null),ce(null),c(null),p(null)),h.default.useEffect(()=>{t.current=e},[e]),h.default.useEffect(()=>{G.current=R},[R]),h.default.useEffect(()=>{D.current=T},[T]),h.default.useEffect(()=>{let ae=D2(e,R);ce(ne=>ne&&ne.gateKey!==ae?null:ne)},[R,e]),h.default.useEffect(()=>{y2(yt),pt.current.clear()},[e]);let tn=Math.max(0,Math.ceil((r-i)/1e3)),Aa=R?.runId&&R?.gateRef?`${R.runId}
${R.gateRef}`:null;h.default.useEffect(()=>{if(!r)return;let ae=setInterval(()=>o(Date.now()),250);return()=>clearInterval(ae)},[r]),h.default.useEffect(()=>{Ue.current.gateKey!==Aa&&(Ue.current={gateKey:Aa,credentialRef:null,inFlight:!1})},[Aa]),h.default.useEffect(()=>{if(!M2(R))return;let ae=Date.now(),ne=Be=>{l5(Be,R,ae)&&(oe(N=>M2(N)?null:N),P(!0))},Ee=null;typeof window.BroadcastChannel=="function"&&(Ee=new window.BroadcastChannel(e5),Ee.onmessage=Be=>ne(Be.data));let je=Be=>{Be.key===hh&&ne(vh(Be.newValue))};window.addEventListener("storage",je),ne(vh(window.localStorage?.getItem?.(hh)));let bt=window.setInterval(()=>{ne(vh(window.localStorage?.getItem?.(hh)))},500);return()=>{window.clearInterval(bt),Ee&&Ee.close(),window.removeEventListener("storage",je)}},[R]);let xa=k2({threadId:e,setMessages:_,setIsProcessing:P,setPendingGate:oe,setActiveRun:m,activeRunRef:d,locallyResolvedGatesRef:pt,toolActivityStateRef:yt,onRunSettled:(ae,{success:ne})=>{ne&&y([]),S(void 0,{preserveClientOnly:!0,finalReplyTimestampByRun:ae&&ne?{[ae]:new Date().toISOString()}:null})}}),{status:Da}=T2({threadId:e,onEvent:xa,enabled:!!e}),Ma=h.default.useCallback(async(ae,ne={})=>{let{threadId:Ee,attachments:je=[]}=ne,bt=je.map(u$),Be=je.map(c$);if(R||G.current)throw n5();if(D.current)return null;if(je.length===0){let re=await u5(ae);if(re)return p(re),{channel_connect_action:re}}p(null);let N=Ee||e;if(!N){let re=await uc();if(Et.invalidateQueries({queryKey:["threads"]}),N=re?.thread?.thread_id,!N)throw new Error("createThread returned no thread_id")}let k=N,M={id:`pending-${n.current++}`,role:"user",content:ae,attachments:Be,timestamp:new Date().toISOString(),isOptimistic:!0},K={id:M.id,role:"user",content:ae,attachments:Be,timestamp:M.timestamp,isOptimistic:!0};R2(a.current,k,M);let B=M.id,j=!e||N===e,X=re=>{j&&_(re)},de=re=>{N!==e&&E(N,re)},xe=re=>{j&&re()};X(re=>[...re,K]),de(re=>[...re,K]),xe(()=>{P(!0),G.current||oe(null)});try{let re=await zx({threadId:N,content:ae,attachments:bt});r5(N)&&Et.invalidateQueries({queryKey:["threads"]}),re?.run_id&&j&&m({runId:re.run_id,threadId:re.thread_id||N,status:re.status||null,source:"local"});let La=E2(a.current,k,B,re?.accepted_message_ref)||ph(re?.accepted_message_ref);if(La){let xt=ia=>ia.map(oa=>oa.id===B?{...oa,timelineMessageId:La}:oa);X(xt),de(xt)}if(re?.outcome==="rejected_busy"){let xt=ia=>ia.map(oa=>oa.id===B?{...oa,isOptimistic:!1,status:"error"}:oa);if(X(xt),de(xt),re?.notice){let ia=(an=j)=>{let pd={id:`system-rejected-${n.current++}`,role:"system",content:re.notice,timestamp:new Date().toISOString(),isOptimistic:!1},wi=hd=>[...hd,pd];an&&_(wi),(!an||N!==e)&&E(N,wi)};if(!t.current||t.current===N){let an=D2(N,G.current);an?ce({gateKey:an,content:re.notice}):ia()}else ia(!1)}xe(()=>P(!1))}return re}catch(re){re.status===429&&s(Date.now()+d5(re));let La=xt=>xt.map(ia=>ia.id===B?{...ia,isOptimistic:!1,status:"error",error:re.message}:ia);throw X(La),de(La),xe(()=>P(!1)),re}finally{C2(a.current,k,B)}},[e,R,_,E,P,oe,m]),Oa=h.default.useCallback(async(ae,ne={})=>{if(!R)return;let{runId:Ee,gateRef:je}=R;if(!Ee||!je)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");let bt=await Ap({threadId:e,runId:Ee,gateRef:je,resolution:ae,always:ne.always,credentialRef:ne.credentialRef}),Be=i5(bt);if(pt.current.set(`${Ee}
${je}`,{resolution:ae,outcome:Be}),c5(ae)&&Be==="resumed"&&b2(_,R,yt),oe(null),Be==="resumed"){P(!0),m({runId:bt?.run_id||Ee,threadId:bt?.thread_id||e,status:bt?.status||"queued"});return}P(!1),m(null)},[R,e,_,m]),$a=h.default.useCallback(async ae=>{if(!R)throw new Error("auth gate is no longer pending");let{runId:ne,gateRef:Ee,provider:je}=R;if(!ne||!Ee||!je)throw new Error("auth gate is missing required credential metadata");let bt=R.accountLabel||`${je} credential`,Be=`${ne}
${Ee}`;if(Ue.current.gateKey!==Be&&(Ue.current={gateKey:Be,credentialRef:null,inFlight:!1}),Ue.current.inFlight)throw new Error("auth token submission already in progress");Ue.current.inFlight=!0;try{let N=Ue.current.credentialRef,k=null;if(!N){if(k=await A2(M=>Qx({provider:je,accountLabel:bt,token:ae,threadId:e,runId:ne,gateRef:Ee,signal:M})),N=k?.credential_ref,!N)throw new Error("manual token submit returned no credential_ref");Ue.current.credentialRef=N}if(!s5(k))try{await A2(M=>Ap({threadId:e,runId:ne,gateRef:Ee,resolution:"credential_provided",credentialRef:N,signal:M}))}catch(M){throw a5(M)}Ue.current={gateKey:null,credentialRef:null,inFlight:!1},oe(null),P(!0)}catch(N){throw Ue.current.gateKey===Be&&(Ue.current.inFlight=!1),N}},[R,e]),lt=h.default.useCallback(async ae=>{let ne=u?.runId;!ne||!e||(oe(null),P(!1),m(null),await Hx({threadId:e,runId:ne,reason:ae}))},[u,e]),sa=h.default.useCallback(()=>{g&&v&&S(v)},[g,v,S]),wa=h.default.useCallback(async(ae,ne,Ee)=>{let je="approved",bt=!1;ne==="deny"?je="denied":ne==="cancel"?je="cancelled":ne==="always"&&(je="approved",bt=!0),await Oa(je,{always:bt})},[Oa]),It=h.default.useCallback(()=>{},[]);return{messages:w,isProcessing:T,pendingGate:R,busyGateNotice:se,channelConnectAction:f,activeRun:u,sseStatus:Da,historyLoading:b,historyLoadError:$,hasMore:g,cooldownSeconds:tn,send:Ma,resolveGate:Oa,submitAuthToken:$a,cancelRun:lt,loadMore:sa,dismissChannelConnectAction:()=>p(null),suggestions:[],setSuggestions:It,retryMessage:It,approve:wa,recoverHistory:It,recoveryNotice:null}}function c5(e){return e==="denied"||e==="cancelled"}function d5(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function L2({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}function m5(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function zc({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let u=o.toString(),c=`/logs${u?`?${u}`:""}`;return i?`/v2${c}`:c}function P2(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(m5),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}var f5=1500;function U2({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let o=C(),{messages:u,isProcessing:c,pendingGate:d,busyGateNotice:m,channelConnectAction:f,suggestions:p,sseStatus:x,historyLoading:y,historyLoadError:w,hasMore:g,cooldownSeconds:v,recoveryNotice:b,activeRun:$,send:S,cancelRun:E,retryMessage:_,approve:T,recoverHistory:L,loadMore:D,setSuggestions:P,submitAuthToken:R,dismissChannelConnectAction:F}=O2(t),G=h.default.useMemo(()=>e.find(lt=>lt.id===t)||null,[e,t]),se=h.default.useMemo(()=>L2({gatewayStatus:i,activeThread:G}),[i,G]),ce=u.length>0||c||!!d||!!f,oe=!y&&!ce&&!w,Je=d?"Resolve the approval request before sending another message.":"",He=!!d||c&&!d||v>0,yt=h.default.useRef(He);yt.current=He;let pt=Je||(v>0?`Retry in ${v}s`:void 0),Ue=t||Vo,tn=!!(t&&$?.runId&&$.threadId===t&&c&&!d),Aa=t&&$?.runId&&$.threadId===t?zc({threadId:t,runId:$.runId},{absolute:!0}):null,xa=h.default.useCallback(async(lt,{images:sa=[],attachments:wa=[]}={})=>{if(d)throw new Error(Je);if(yt.current)return null;let It=await S(lt,{images:sa,attachments:wa,threadId:t}),ae=It?.thread_id||t;return!t&&ae&&a&&a(ae,{replace:!0}),It},[t,Je,He,a,d,S]),Da=h.default.useCallback(async lt=>{He||(P([]),await xa(lt))},[He,xa,P]),Ma=h.default.useCallback(()=>E("user_requested"),[E]);h.default.useEffect(()=>{if(!t)return;if(d){bc(t,_n.NEEDS_ATTENTION);return}if(c){bc(t,_n.RUNNING);return}let lt=setTimeout(()=>Cw(t),f5);return()=>clearTimeout(lt)},[t,d,c]);let[Oa,$a]=h.default.useState(!1);return h.default.useEffect(()=>{let lt=sa=>{if(sa.key==="Escape"){$a(!1);return}if(sa.key!=="?")return;let wa=sa.target,It=wa?.tagName;It==="INPUT"||It==="TEXTAREA"||wa?.isContentEditable||(sa.preventDefault(),$a(ae=>!ae))};return window.addEventListener("keydown",lt),()=>window.removeEventListener("keydown",lt)},[]),l`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${A1} status=${x} />

        ${w&&l`
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            ${w}
          </div>
        `}

        ${oe&&l`
          <${D1}
            onSuggestion=${Da}
            onSend=${xa}
            disabled=${!1}
            sendDisabled=${He}
            initialText=${r}
            resetKey=${s}
            draftKey=${Ue}
            context=${se}
            statusText=${pt}
            canCancel=${tn}
            onCancel=${Ma}
          />
        `}
        ${!oe&&l`
          <${o2}
            messages=${u}
            isLoading=${y}
            hasMore=${g}
            onLoadMore=${D}
            onRetryMessage=${_}
            threadId=${t}
            pending=${c}
          >
            ${b&&l`
              <${l2}
                notice=${b}
                onRecover=${L}
              />
            `}
            ${c&&!d&&l`
              <div className="flex flex-wrap items-center gap-3">
                <${c2} />
                ${Aa&&l`
                  <${Sn}
                    to=${Aa}
                    className="text-xs font-medium text-signal hover:underline"
                  >
                    ${o("nav.logs")}
                  <//>
                `}
              </div>
            `}
            ${f&&l`
              <${C1}
                connectAction=${f}
                onDismiss=${F}
              />
            `}
            ${d&&(d.kind==="auth_required"?d.challengeKind==="oauth_url"?l`
                  <${_1}
                    gate=${d}
                    onCancel=${()=>T(d.requestId,"cancel",d.kind)}
                  />
                `:d.challengeKind==="manual_token"?l`
                  <${k1}
                    gate=${d}
                    onSubmit=${R}
                    onCancel=${()=>T(d.requestId,"cancel",d.kind)}
                  />
                `:l`
                  <${N1}
                    gate=${d}
                    onCancel=${()=>T(d.requestId,"cancel",d.kind)}
                  />
                `:l`
              <${S1}
                gate=${d}
                onApprove=${()=>T(d.requestId,"approve",d.kind)}
                onDeny=${()=>T(d.requestId,"deny",d.kind)}
                onAlways=${()=>T(d.requestId,"always",d.kind)}
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

          <${u2}
            suggestions=${p}
            onSelect=${Da}
            disabled=${He}
          />

          <${Ac}
            onSend=${xa}
            disabled=${!1}
            sendDisabled=${He}
            initialText=${r}
            resetKey=${s}
            draftKey=${Ue}
            context=${se}
            statusText=${pt}
            canCancel=${tn}
            onCancel=${Ma}
          />
        `}
      </div>
      <${M1}
        open=${Oa}
        onClose=${()=>$a(!1)}
      />
    </div>
  `}function gh(){let{threadsState:e,gatewayStatus:t}=ya(),{threadId:a}=it(),n=pe(),r=Le(),s=r.state?.composerDraft||"";h.default.useEffect(()=>{a&&a!==e.activeThreadId?e.setActiveThreadId(a):a||e.setActiveThreadId(null)},[a]);let i=h.default.useCallback((o,u={})=>{if(!o){e.setActiveThreadId(null),n("/chat",u);return}e.setActiveThreadId(o),n(`/chat/${o}`,u)},[e,n]);return l`
    <${U2}
      threads=${e.threads}
      activeThreadId=${e.activeThreadId}
      onSelectThread=${i}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function j2(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?Zs(e,t):"",model:e?gc(e,t):""}}function F2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u}){let[c,d]=h.default.useState(()=>j2(e,a)),[m,f]=h.default.useState(""),[p,x]=h.default.useState([]),[y,w]=h.default.useState(null),[g,v]=h.default.useState(""),b=h.default.useRef(!!e);h.default.useEffect(()=>{n&&(d(j2(e,a)),f(""),x([]),w(null),v(""),b.current=!!e)},[n,e,a]);let $=e?.builtin===!0,S=e&&!e.builtin,E=h.default.useCallback((P,R)=>{d(F=>{let G={...F,[P]:R};return P==="name"&&!b.current&&(G.id=rw(R)),G})},[]),_=h.default.useCallback(()=>!$&&(!c.name.trim()||!c.id.trim())?u("llm.fieldsRequired"):!$&&!sw(c.id.trim())?u("llm.invalidId"):!S&&!$&&t.includes(c.id.trim())?u("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,$,S,u]),T=h.default.useCallback(async()=>{let P=_();if(P){w({tone:"error",text:P});return}v("save");try{await s({form:c,apiKey:m,provider:e}),r()}catch(R){w({tone:"error",text:R.message})}finally{v("")}},[m,c,r,s,e,_]),L=h.default.useCallback(async()=>{if(!c.model.trim()){w({tone:"error",text:u("llm.modelRequired")});return}v("test");try{let P=await i(Hp(e,c,m,a));w({tone:P.ok?"success":"error",text:P.message})}catch(P){w({tone:"error",text:P.message})}finally{v("")}},[m,a,c,i,e,u]),D=h.default.useCallback(async()=>{if(($?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){w({tone:"error",text:u("llm.baseUrlRequired")});return}v("models");try{let R=await o(Hp(e,c,m,a));if(!R.ok||!Array.isArray(R.models)||!R.models.length)w({tone:"error",text:R.message||u("llm.modelsFetchFailed")});else{x(R.models);let F=iw(c.model,R.models);F!==null&&E("model",F),w({tone:"success",text:u("llm.modelsFetched",{count:R.models.length})})}}catch(R){w({tone:"error",text:R.message})}finally{v("")}},[m,a,c,$,o,e,u,E]);return{form:c,apiKey:m,models:p,message:y,busy:g,isBuiltin:$,isEditing:S,setApiKey:f,update:E,submit:T,runTest:L,fetchModels:D,markIdEdited:()=>{b.current=!0}}}function qc({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let u=C(),c=F2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u});if(!n)return null;let{form:d,apiKey:m,models:f,message:p,busy:x,isBuiltin:y,isEditing:w}=c,g=y?u("llm.configureProvider",{name:e.name||e.id}):u(w?"llm.editProvider":"llm.newProvider");return l`
    <${si} open=${n} onClose=${r} title=${g} size="lg">
      <${ii} className="space-y-4">
        ${!y&&l`
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${u("llm.providerName")}
              <${At} value=${d.name} onChange=${v=>c.update("name",v.target.value)} />
            </label>
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${u("llm.providerId")}
              <${At}
                value=${d.id}
                disabled=${w}
                onChange=${v=>{c.markIdEdited(),c.update("id",v.target.value)}}
              />
            </label>
          </div>
          <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
            ${u("llm.adapter")}
            <${rh} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${Kp.map(v=>l`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
            <//>
          </label>
        `}

        ${y&&l`
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            ${Jo(e.adapter)}
          </div>
        `}

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.baseUrl")}
          <${At} value=${d.baseUrl} placeholder=${e?.base_url||""} onChange=${v=>c.update("baseUrl",v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.apiKey")}
          <${At} type="password" value=${m} placeholder=${u("llm.apiKeyPlaceholder")} onChange=${v=>c.setApiKey(v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.defaultModel")}
          <div className="flex items-stretch gap-2">
            <${At} value=${d.model} onChange=${v=>c.update("model",v.target.value)} />
            <${A} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${x!==""} onClick=${c.fetchModels}>
              ${u(x==="models"?"llm.fetchingModels":"llm.fetchModels")}
            <//>
          </div>
        </label>

        ${f.length>0&&l`
          <${rh} value=${d.model} onChange=${v=>c.update("model",v.target.value)}>
            ${f.map(v=>l`<option key=${v} value=${v}>${v}</option>`)}
          <//>
        `}

        ${p&&l`
          <div className=${p.tone==="error"?"text-sm text-red-200":"text-sm text-mint"} role="status">
            ${p.text}
          </div>
        `}
      <//>
      <${oi}>
        <${A} type="button" variant="secondary" disabled=${x!==""} onClick=${c.runTest}>
          ${u(x==="test"?"llm.testing":"llm.testConnection")}
        <//>
        <${A} type="button" variant="ghost" disabled=${x!==""} onClick=${r}>${u("common.cancel")}<//>
        <${A} type="button" disabled=${x!==""} onClick=${c.submit}>
          ${u(x==="save"?"common.saving":"common.save")}
        <//>
      <//>
    <//>
  `}function Ic({login:e}){let t=C(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return l`
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
  `}function p5(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function Kc({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=Ws({settings:e,gatewayStatus:t}),[s,i]=h.default.useState(null),[o,u]=h.default.useState(!1),[c,d]=h.default.useState(null),m=h.default.useRef(null),f=h.default.useCallback((g,v)=>{m.current&&window.clearTimeout(m.current),d({tone:g,text:v}),m.current=window.setTimeout(()=>d(null),3500)},[]);h.default.useEffect(()=>()=>{m.current&&window.clearTimeout(m.current)},[]);let p=h.default.useCallback((g=null)=>{i(g),u(!0)},[]),x=h.default.useCallback(async g=>{try{await r.setActiveProvider(g),f("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(p(g),f("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):f("error",v.message)}},[p,r,f,n]),y=h.default.useCallback(async({form:g,apiKey:v,provider:b})=>{if(b?.builtin){await r.saveBuiltinProvider({provider:b,form:g,apiKey:v}),f("success",n("llm.providerConfigured",{name:b.name||b.id}));return}let $=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:b});f("success",n(b?"llm.providerUpdated":"llm.providerAdded",{name:$.name||$.id}))},[r,f,n]),w=h.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),f("success",n("llm.providerDeleted"))}catch(v){f("error",v.message)}},[r,f,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>p5(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:p,closeDialog:()=>u(!1),handleUse:x,handleSave:y,handleDelete:w}}var h5=3e5;function v5(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function g5(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function y5(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=u=>{let c=u.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},h5);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var b5=3e5,x5=9e5,$5=2e3;async function B2(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,$5)),(await vc().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function Hc({onSuccess:e}={}){let t=C(),a=J(),[n,r]=h.default.useState(!1),[s,i]=h.default.useState(""),[o,u]=h.default.useState(!1),[c,d]=h.default.useState(""),[m,f]=h.default.useState(null),p=h.default.useCallback(()=>{i(""),d(""),f(null)},[]),x=h.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),y=h.default.useCallback(async v=>{if(p(),v5()){i(t("onboarding.nearaiLocalSso"));return}let b=window.open("about:blank","_blank");if(!b){i(t("onboarding.nearaiFailed"));return}try{b.opener=null}catch{}r(!0);try{let{auth_url:$}=await P$({provider:v,origin:window.location.origin});b.location.href=$;let S=await B2("nearai",b5,b);if(S==="active"){await x();return}b.close(),i(t(S==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{b.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,p,t]),w=h.default.useCallback(async()=>{p(),r(!0);try{let v=g5(),b=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!b){i(t("onboarding.nearaiFailed"));return}b.opener=null;let $=await y5(b,v);if(!$){i(t("onboarding.nearaiFailed"));return}await U$({account_id:$.accountId,public_key:$.publicKey,signature:$.signature,message:$.message,recipient:$.recipient,nonce:$.nonce}),await x()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,p,t]),g=h.default.useCallback(async()=>{p();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}u(!0);try{let{user_code:b,verification_uri:$}=await j$();f({userCode:b,verificationUri:$}),v&&(v.location.href=$);let S=await B2("openai_codex",x5,v);if(S==="active"){await x();return}v&&v.close(),d(t(S==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{u(!1)}},[x,p,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:m,startNearai:y,startNearaiWallet:w,startCodex:g}}var z2="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",w5="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",S5="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",N5="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",_5={nearai:{color:"#00ec97",path:w5},openai_codex:{color:"#10a37f",path:z2},openai:{color:"#10a37f",path:z2},anthropic:{color:"#d97757",path:S5},ollama:{color:null,path:N5}};function q2({id:e,name:t}){let a=_5[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return l`
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
  `}var k5=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function R5({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=h.default.useState(!1),o=h.default.useRef(null),u=t||a.nearaiBusy;h.default.useEffect(()=>{if(!s)return;let d=f=>{o.current&&!o.current.contains(f.target)&&i(!1)},m=f=>{f.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",m),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",m)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return l`
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
        <${O} name="chevron" className="h-3.5 w-3.5" />
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
  `}function C5({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let u=s(e.nameKey),c;return e.auth==="nearai"?c=l`<${R5} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=l`
      <${A} type="button" variant="secondary" size="sm" disabled=${r.codexBusy} onClick=${r.startCodex}>
        ${s("onboarding.signIn")}
      <//>
    `:a?c=l`<${A} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>i(t)}>
      ${s("llm.use")}
    <//>`:c=l`<${A} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>o(t)}>
      ${s("onboarding.setUp")}
    <//>`,l`
    <${ee} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-4">
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <${q2} id=${e.id} name=${u} />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">${u}</span>
            ${a&&l`<${z} tone="positive" label=${s("onboarding.ready")} size="sm" />`}
          </div>
          <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">${s(e.descKey)}</div>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap gap-2 sm:justify-end">${c}</div>
    <//>
  `}function I2(){let{isAdmin:e=!1,isChecking:t=!1}=ya();return t?null:e?l`<${E5} />`:l`<${ot} to="/chat" replace />`}function E5(){let e=C(),t=pe(),a=J(),{gatewayStatus:n}=ya(),r=Kc({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=k5.map(m=>({entry:m,provider:s.providers.find(f=>f.id===m.id)})).filter(m=>m.provider),o=h.default.useCallback(()=>t("/chat"),[t]),u=Hc({onSuccess:o}),c=h.default.useCallback(async m=>{let f=m.active_model||m.default_model||"";await Yo({provider_id:m.id,model:f}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=h.default.useCallback(async({form:m,apiKey:f,provider:p})=>{await r.handleSave({form:m,apiKey:f,provider:p});let x=p?.id||m.id.trim(),y=m.model?.trim()||p?.default_model||"";await Yo({provider_id:x,model:y}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?l`
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
              <${C5}
                key=${m.id}
                entry=${m}
                provider=${f}
                configured=${Br(f,s.builtinOverrides)}
                isBusy=${s.isBusy}
                login=${u}
                t=${e}
                onUse=${c}
                onSetUp=${r.openDialog}
              />
            `)}
        </div>

        <${Ic} login=${u} />

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

      <${qc}
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
  `}function q({children:e,className:t="",...a}){return l`<${ee} className=${t} ...${a}>${e}<//>`}function tt({label:e,value:t,tone:a="muted",badgeLabel:n,detail:r,showDivider:s=!0,className:i="",valueClassName:o="text-[1.75rem] md:text-[2rem]"}){return l`
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
        <${z} tone=${a} label=${n??a} />
      </div>
    </div>
  `}function K2({items:e}){return l`
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
  `}function be({title:e,description:t,children:a,boxed:n=!0}){let r=l`
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
  `;return n?l`<${ee} padding="lg">${r}<//>`:l`<div className="py-8">${r}</div>`}var H2={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function Wa({result:e,onDismiss:t}){return e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",H2[e.type]||H2.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}var Q2="",T5={workspace:"home"};function Qc(e){return T5[e]||e}function nl(e){return[...e||[]].sort((t,a)=>t.is_dir!==a.is_dir?t.is_dir?-1:1:t.name.localeCompare(a.name,void 0,{sensitivity:"base"}))}function ui(e){return e?e.split("/").filter(Boolean):[]}function Vc(e){return e?`/workspace/${ui(e).map(encodeURIComponent).join("/")}`:"/workspace"}function yh(e){let t=ui(e);return t.pop(),t.join("/")}function V2(e){return/\.mdx?$/i.test(e||"")}function Gc({path:e,onNavigate:t}){let a=C(),n=ui(e),r="";return l`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button
        type="button"
        onClick=${()=>t("/workspace")}
        className="text-signal hover:underline"
      >
        ${a("workspace.breadcrumbRoot")}
      </button>
      ${n.map((s,i)=>{r=r?`${r}/${s}`:s;let o=r,u=i===0?Qc(s):s;return l`
          <span key=${o} className="text-iron-400">/</span>
          <button
            key=${`${o}-button`}
            type="button"
            onClick=${()=>t(Vc(o))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${u}
          </button>
        `})}
    </div>
  `}function A5(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function G2({path:e,entries:t,isLoading:a,filter:n,onOpen:r,onNavigate:s}){let i=C();if(a)return l`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;let o=(t||[]).filter(f=>!A5(f.path)),u=String(n||"").trim().toLowerCase(),c=u?o.filter(f=>f.name.toLowerCase().includes(u)):o,d=nl(c),m;return o.length?d.length?m=l`
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
    <${q} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${Gc} path=${e} onNavigate=${s} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">${m}</div>
    <//>
  `}var Yc="/api/webchat/v2/fs",D5=1024*1024,M5=8*1024*1024;function Y2(e){let t=String(e||"").split("/").filter(Boolean);return{mount:t.shift()||"",path:t.join("/")}}function O5(e,t){return t?`${e}/${t}`:e}function L5(e){let t=String(e||"").toLowerCase();return t.startsWith("text/")||t==="application/json"||t==="application/javascript"||t==="application/xml"||t.endsWith("+json")||t.endsWith("+xml")}function P5(e){return String(e||"").toLowerCase().startsWith("image/")}function U5(e){let t=String(e||"").toLowerCase();return t.startsWith("audio/")||t.startsWith("video/")||t.startsWith("font/")||t==="application/pdf"||t==="application/zip"||t==="application/gzip"}function j5(e){if(e.subarray(0,Math.min(e.length,8192)).indexOf(0)!==-1)return!0;try{return new TextDecoder("utf-8",{fatal:!0}).decode(e),!1}catch{return!0}}function F5(e,t){let a=new URL(`${Yc}/content`,window.location.origin);return a.searchParams.set("mount",e),a.searchParams.set("path",t),a.pathname+a.search}async function B5(){return(await H(`${Yc}/mounts`))?.mounts||[]}async function ci(e=""){if(!e)return{entries:(await B5()).map(o=>({name:Qc(o.mount),path:o.mount,is_dir:!0}))};let{mount:t,path:a}=Y2(e),n=new URL(`${Yc}/list`,window.location.origin);return n.searchParams.set("mount",t),a&&n.searchParams.set("path",a),{entries:((await H(n.pathname+n.search))?.entries||[]).map(i=>({name:i.name,path:O5(t,i.path),is_dir:i.kind==="directory"}))}}async function J2(e){let{mount:t,path:a}=Y2(e);if(!t||!a)return{kind:"directory",path:e};let n=new URL(`${Yc}/stat`,window.location.origin);n.searchParams.set("mount",t),n.searchParams.set("path",a);let s=(await H(n.pathname+n.search))?.stat||{},i=s.mime_type||"application/octet-stream",o=Number(s.size_bytes||0),u=F5(t,a),c={path:e,mime:i,size_bytes:o,download_path:u};if(s.kind&&s.kind!=="file")return{...c,kind:"directory"};if(P5(i)){if(o>M5)return{...c,kind:"binary"};let p=await dc(u);return{...c,kind:"image",image_data_url:p}}if(U5(i)||o>D5)return{...c,kind:"binary"};let d=await Ra(u),m=new Uint8Array(await d.arrayBuffer());if(!L5(i)&&j5(m))return{...c,kind:"binary"};let f=new TextDecoder("utf-8").decode(m);return{...c,kind:"text",content:f}}function X2(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function z5(e,t,a){let n=String(t||"").trim().toLowerCase(),r=(e||[]).filter(s=>!X2(s.path)).filter(s=>!n||s.name.toLowerCase().includes(n)?!0:s.is_dir&&a.has(s.path));return nl(r)}function Z2({entry:e,depth:t,selectedPath:a,expandedPaths:n,filter:r,onToggleDirectory:s,onSelectFile:i}){let o=C(),u=n.has(e.path),c=I({queryKey:["workspace-list",e.path],queryFn:()=>ci(e.path),enabled:e.is_dir&&u});if(e.is_dir){let d=z5(c.data?.entries,r,n);return l`
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
                  <${Z2}
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
  `}function W2({entries:e,selectedPath:t,expandedPaths:a,filter:n,onToggleDirectory:r,onSelectFile:s,isLoading:i}){let o=C();if(i)return l`<div className="space-y-2 p-3">${[1,2,3,4].map(c=>l`<div key=${c} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let u=nl(e.filter(c=>!X2(c.path)));return u.length?l`
    <div className="space-y-1 p-2">
      ${u.map(c=>l`
        <${Z2}
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
  `:l`<div className="px-4 py-8 text-sm text-iron-300">${o("workspace.noFiles")}</div>`}function eS({rootEntries:e,selectedPath:t,expandedPaths:a,filter:n,onFilterChange:r,isLoadingTree:s,onToggleDirectory:i,onSelectFile:o}){let u=C();return l`
    <${q} className="flex min-h-[420px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="border-b border-white/10 p-3">
        <input
          value=${n}
          onInput=${c=>r(c.target.value)}
          placeholder=${u("workspace.filterPlaceholder")}
          className="h-9 w-full rounded-md border border-white/10 bg-iron-950/80 px-3 text-sm text-white outline-none placeholder:text-iron-400 focus:border-signal/45"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <${W2}
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
  `}function tS(e){return ui(e).pop()||"download"}function q5({path:e,file:t}){let a=C();return t.kind==="image"?l`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${t.image_data_url}
          alt=${tS(e)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `:t.kind==="text"?l`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${V2(e)?l`<${aa} content=${t.content} className="max-w-4xl text-base leading-7" />`:l`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
      </div>
    `:l`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${a("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `}function aS({path:e,file:t,isLoading:a,onNavigate:n}){let r=C(),[s,i]=h.default.useState(!1),o=h.default.useCallback(async()=>{if(t?.download_path){i(!0);try{let c=await Ra(t.download_path);Dc(c,tS(e))}catch{}finally{i(!1)}}},[t,e]);if(a)return l`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;if(!t||t.kind==="directory")return l`
      <${be}
        title=${r("workspace.pickFileTitle")}
        description=${r("workspace.pickFileDesc")}
      />
    `;let u=r("workspace.fileMeta",{mime:t.mime||"application/octet-stream",size:Number(t.size_bytes||0)});return l`
    <${q} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${Gc} path=${e} onNavigate=${n} />
        <div className="flex items-center gap-2">
          <${z} tone="muted" label=${u} />
          <${A}
            variant="secondary"
            size="sm"
            onClick=${o}
            disabled=${s}
          >${r("workspace.download")}<//>
        </div>
      </div>

      <${q5} path=${e} file=${t} />

      ${yh(e)&&l`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${r("workspace.parent",{path:yh(e)})}
        </div>
      `}
    <//>
  `}function nS(e){let t=C(),a=J(),[n,r]=h.default.useState(new Set),[s,i]=h.default.useState(""),[o,u]=h.default.useState(null),c=I({queryKey:["workspace-list",""],queryFn:()=>ci("")}),d=I({queryKey:["workspace-file",e],queryFn:()=>J2(e),enabled:!!e}),m=e===""||d.data?.kind==="directory",f=I({queryKey:["workspace-list",e],queryFn:()=>ci(e),enabled:m});h.default.useEffect(()=>{u(null)},[e]);let p=h.default.useCallback(y=>a.fetchQuery({queryKey:["workspace-list",y],queryFn:()=>ci(y)}),[a]),x=h.default.useCallback(async y=>{let w=new Set(n);if(w.has(y)){w.delete(y),r(w);return}w.add(y),r(w);try{await p(y)}catch(g){u({type:"error",message:g.message||t("workspace.unableOpenDirectory")})}},[n,p,t]);return{rootEntries:c.data?.entries||[],file:d.data||null,selectionIsDirectory:m,currentEntries:f.data?.entries||[],expandedPaths:n,filter:s,setFilter:i,result:o,clearResult:()=>u(null),isLoadingTree:c.isLoading,isLoadingFile:d.isLoading,isLoadingListing:f.isLoading,isFetching:c.isFetching||d.isFetching||f.isFetching,error:c.error||d.error||f.error||null,loadDirectory:p,toggleDirectory:x,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function bh(){let e=C(),t=pe(),n=it()["*"]||Q2,r=nS(n),s=h.default.useCallback(i=>{t(Vc(i))},[t]);return l`
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

          ${r.error&&l`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${r.error.message}
            </div>
          `}
          <${Wa}
            result=${r.result}
            onDismiss=${r.clearResult}
          />

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <${eS}
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
                  <${G2}
                    path=${n}
                    entries=${r.currentEntries}
                    isLoading=${r.isLoadingListing}
                    filter=${r.filter}
                    onOpen=${s}
                    onNavigate=${t}
                  />
                `:l`
                  <${aS}
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
  `}function rS(e){if(!e)return null;let t=e.metadata&&typeof e.metadata=="object"&&!Array.isArray(e.metadata)?e.metadata:{};return{id:e.project_id,name:e.name,description:e.description,goals:Array.isArray(t.goals)?t.goals:[],icon:e.icon||null,color:e.color||null,state:e.state,role:e.role,metadata:t,created_at:e.created_at,updated_at:e.updated_at,health:e.state==="archived"?"muted":"green"}}async function sS(){let t=((await Lx({limit:200}))?.projects||[]).map(rS);return{attention:[],projects:t}}async function iS(e){if(!e)return null;let t=await Px({projectId:e});return rS(t?.project)}function oS(e){return Promise.resolve({missions:[],todo:!0})}function lS(e){return Promise.resolve({threads:[],todo:!0})}function uS(e){return Promise.resolve({widgets:[],todo:!0})}function cS(e){return Promise.resolve(null)}function dS(e){return Promise.resolve(null)}function mS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function fS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function pS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function hS(){let e=J(),t=I({queryKey:["projects-overview"],queryFn:sS,refetchInterval:5e3}),a=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function vS(e){let t=J(),a=!!e,n=I({queryKey:["project-detail",e],queryFn:()=>iS(e),enabled:a,refetchInterval:a?7e3:!1}),r=I({queryKey:["project-missions",e],queryFn:()=>oS(e),enabled:a,refetchInterval:a?5e3:!1}),s=I({queryKey:["project-threads",e],queryFn:()=>lS(e),enabled:a,refetchInterval:a?4e3:!1}),i=I({queryKey:["project-widgets",e],queryFn:()=>uS(e),enabled:a,refetchInterval:a?15e3:!1}),o=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function gS({projectId:e,missionId:t,threadId:a}){let n=J(),[r,s]=h.default.useState(null),i=I({queryKey:["project-mission-detail",t],queryFn:()=>cS(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=I({queryKey:["project-thread-detail",a],queryFn:()=>dS(a),enabled:!!a,refetchInterval:a?4e3:!1}),u=h.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=Q({mutationFn:({targetMissionId:f})=>mS(f),onSuccess:f=>{s({type:"success",message:f?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),u()},onError:f=>{s({type:"error",message:f.message||"Unable to fire mission"})}}),d=Q({mutationFn:({targetMissionId:f})=>fS(f),onSuccess:()=>{s({type:"success",message:"Mission paused."}),u()},onError:f=>{s({type:"error",message:f.message||"Unable to pause mission"})}}),m=Q({mutationFn:({targetMissionId:f})=>pS(f),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),u()},onError:f=>{s({type:"error",message:f.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending}}function Jc(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function Xc(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function yS(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function bS(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function I5(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function xS(e){let t=I5(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function $S(e){let t=e?.projects||[],a=t.reduce((o,u)=>o+Number(u.cost_today_usd||0),0),n=t.reduce((o,u)=>o+Number(u.active_missions||0),0),r=t.reduce((o,u)=>o+Number(u.threads_today||0),0),s=t.reduce((o,u)=>o+Number(u.pending_gates||0),0),i=t.reduce((o,u)=>o+Number(u.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function rl(e,t){return`${e} ${t}${e===1?"":"s"}`}var K5={projects:"muted",attention:"warning",spend:"success"};function wS({overview:e}){let t=$S(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:Xc(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return l`
    <${q} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${z} tone=${K5[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function H5(e){return e?.type==="failure"?"danger":"warning"}function Q5(e){return e?.type==="failure"?"failure":"gate"}function SS({items:e,onOpenItem:t}){return e?.length?l`
    <${q} className="overflow-hidden border-amber-300/10 p-0">
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
              <${z} tone=${H5(a)} label=${Q5(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function V5({project:e,onOpen:t,t:a}){return l`
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
        <${z} tone=${yS(e.health)} label=${e.health||"unknown"} />
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
            ${a("projects.card.threadsToday",{count:rl(e.threads_today||0,"thread")})}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${a("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">${rl(e.pending_gates||0,"gate")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${a("projects.card.failures24h",{count:rl(e.failures_24h||0,"failure")})}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>${a("projects.card.spendToday",{value:Xc(e.cost_today_usd||0)})}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${Jc(e.last_activity)}</div>
        </div>
        <${A}
          variant="secondary"
          onClick=${n=>{n.stopPropagation(),t(e.id)}}
        >${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function G5({project:e,onOpen:t,t:a}){return l`
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
            ${rl(e.threads_today||0,"thread")} today
          </div>
          <${A}
            variant="secondary"
            onClick=${n=>{n.stopPropagation(),t(e.id)}}
          >${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function NS({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=C(),u=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return!e.length&&t>0?l`
      <${be}
        title=${o("projects.empty.noMatchTitle")}
        description=${o("projects.empty.noMatchDesc")}
      />
    `:e.length?l`
    <div className="space-y-5">
      ${u&&l`<${G5} project=${u} onOpen=${r} t=${o} />`}

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
            ${c.map(d=>l`<${V5} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
          </div>`:l`
            <${be}
              title=${o("projects.scoped.onlyGeneralTitle")}
              description=${o("projects.scoped.onlyGeneralDesc")}
            >
              <${A} onClick=${s}>${o(i?"projects.preparingChat":"projects.startProject")}<//>
            <//>
          `}
    </div>
  `:l`
      <${be}
        title=${o("projects.empty.noneTitle")}
        description=${o("projects.empty.noneDesc")}
      >
        <${A} onClick=${s}>${o("projects.createFromChat")}<//>
      <//>
    `}function _S({threads:e,selectedThreadId:t,onSelectThread:a,onNewConversation:n,isStartingConversation:r}){let s=[...e].sort((i,o)=>new Date(o.updated_at||o.created_at)-new Date(i.updated_at||i.created_at));return l`
    <${q} className="p-4 sm:p-5">
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
        ${s.length?s.slice(0,18).map(i=>{let o=xS(i);return l`
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
                    <${z} tone=${bS(i.state)} label=${i.state} />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                    <span>${i.step_count||0} steps</span>
                    <span>${i.total_tokens||0} tokens</span>
                    <span>${Jc(i.updated_at||i.created_at)}</span>
                  </div>
                </button>
              `}):l`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                No project threads yet. When an automation runs or scoped chat work happens inside this project, activity will appear here.
              </div>
            `}
      </div>
    <//>
  `}var Y5="/workspace";function J5(e){let t=a=>a.kind==="directory"?0:1;return[...e].sort((a,n)=>t(a)-t(n)||a.name.localeCompare(n.name,void 0,{sensitivity:"base"}))}function X5(e){return e?String(e).replace(/^\/workspace\/?/,"").split("/").filter(Boolean):[]}function kS({threadId:e}){let t=C(),[a,n]=h.default.useState(void 0),[r,s]=h.default.useState(null),i=I({queryKey:["project-files",e||"",a||""],queryFn:()=>Cx({threadId:e,path:a}),enabled:!!e}),o=h.default.useMemo(()=>J5(i.data?.entries||[]),[i.data]),u=h.default.useCallback(async m=>{if(m.kind==="directory"){s(null),n(m.path);return}try{s(null);let f=await Ra(cc({threadId:e,path:m.path})),p=URL.createObjectURL(f),x=document.createElement("a");x.href=p,x.download=m.name,document.body.appendChild(x),x.click(),x.remove(),URL.revokeObjectURL(p)}catch(f){s(f?.message||"Unable to download file")}},[e]),c=X5(a),d=l`
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
  `;return e?l`
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
        ${c.map((m,f)=>{let p=`${Y5}/${c.slice(0,f+1).join("/")}`;return l`
            <span key=${p} className="text-iron-500">/</span>
            <button
              key=${`${p}-button`}
              type="button"
              onClick=${()=>n(p)}
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
                  <${O}
                    name=${m.kind==="directory"?"folder":"file"}
                    className="h-4 w-4 shrink-0 text-iron-300"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm text-white">${m.name}</span>
                  ${m.kind==="directory"?l`<${O} name="chevron" className="h-3.5 w-3.5 shrink-0 -rotate-90 text-iron-500" />`:l`<${O} name="download" className="h-3.5 w-3.5 shrink-0 text-iron-500" />`}
                </button>
              `):l`
              <div className="rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                ${"This folder is empty."}
              </div>
            `}
      </div>
    <//>
  `:l`
      <${q} className="p-4 sm:p-5">
        ${d}
        <div className="mt-4 rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
          ${"No files yet \u2014 they appear once a thread has run in this project."}
        </div>
      <//>
    `}function Z5(e){return[...e||[]].sort((a,n)=>new Date(n.updated_at||n.created_at)-new Date(a.updated_at||a.created_at))[0]?.id||null}function RS({project:e,threads:t,selectedThreadId:a,onSelectThread:n,onNewConversation:r,isStartingConversation:s}){let i=Z5(t);return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]">
      <div className="space-y-5">
        <div className="min-w-0">
          <h2 className="text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
          ${e.description?l`<p className="mt-1 text-sm leading-6 text-iron-300">${e.description}</p>`:null}
        </div>

        <${_S}
          threads=${t}
          selectedThreadId=${a}
          onSelectThread=${n}
          onNewConversation=${r}
          isStartingConversation=${s}
        />
      </div>

      <${kS} threadId=${i} />
    </div>
  `}function sl(){let e=C(),t=pe(),{threadsState:a}=ya(),{projectId:n=null,threadId:r=null}=it(),[s,i]=h.default.useState(""),[o,u]=h.default.useState(null),c=hS(),d=vS(n),m=gS({projectId:n,threadId:r}),f=h.default.useMemo(()=>{let _=s.trim().toLowerCase();return _?c.overview.projects.filter(T=>[T.name,T.description,...T.goals||[]].some(L=>String(L||"").toLowerCase().includes(_))):c.overview.projects},[c.overview.projects,s]),p=h.default.useMemo(()=>c.overview.projects.find(_=>_.id===n)||null,[c.overview.projects,n]),x=h.default.useCallback(()=>{c.invalidate(),d.invalidate()},[c,d]),y=h.default.useCallback(_=>{t(`/projects/${_}`)},[t]),w=h.default.useCallback(_=>{if(_.thread_id){t(`/projects/${_.project_id}/threads/${_.thread_id}`);return}t(`/projects/${_.project_id}`)},[t]),g=h.default.useCallback(async()=>{let _=null;u(null);try{_=await a.createThread()}catch(T){u({type:"error",message:T.message||e("projects.chatAutoFail")})}t("/chat",{state:{composerDraft:e("projects.creationDraft"),threadId:_}})},[t,a]),v=h.default.useCallback(_=>{t(`/projects/${n}/threads/${_}`)},[t,n]),b=h.default.useCallback(async()=>{u(null);try{let _=await a.createThread(n);t("/chat",{state:{threadId:_}}),d.invalidate()}catch(_){u({type:"error",message:_.message||e("projects.chatAutoFail")})}},[t,a,n,d,e]),$=h.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),S=l`
    ${n&&l`<${A} variant="ghost" onClick=${()=>t("/projects")}>${e("projects.allProjects")}<//>`}
  `,E=null;return n?d.isLoading?E=l`
        <div className="space-y-4">
          ${[1,2,3].map(_=>l`<div key=${_} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `:d.error||!d.project&&!p?E=l`
        <${be}
          title=${e("projects.unavailable")}
          description=${d.error?.message||e("projects.unavailableDesc")}
        >
          <${A} variant="secondary" onClick=${()=>t("/projects")}>${e("projects.returnToProjects")}<//>
        <//>
      `:E=l`
        <${RS}
          project=${d.project||p}
          threads=${d.threads}
          selectedThreadId=${r}
          onSelectThread=${v}
          onNewConversation=${b}
          isStartingConversation=${a.isCreating}
        />
      `:E=c.isLoading?l`
          <div className="space-y-4">
            ${[1,2,3].map(_=>l`<div key=${_} className="v2-skeleton h-40 rounded-[20px]" />`)}
          </div>
        `:l`
          <${NS}
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
          <${Wa} result=${o} onDismiss=${()=>u(null)} />
          <${Wa} result=${m.actionResult} onDismiss=${m.clearActionResult} />
          ${!n&&l`
            <${wS} overview=${c.overview} />
            <${SS} items=${c.overview.attention} onOpenItem=${w} />
          `}
          ${E}
        </div>
      </div>
    </div>
  `}function il(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not scheduled"}function ol(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function CS(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function ES(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);return r!==0?r:new Date(n.updated_at||0).getTime()-new Date(a.updated_at||0).getTime()})}function Zc({label:e,value:t}){return l`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function W5({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=C();return e.status==="Active"?l`
      <${A} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.fireNow")}<//>
      <${A} variant="secondary" onClick=${()=>n(e.id)} disabled=${t}>${s("missions.action.pause")}<//>
    `:e.status==="Paused"?l`
      <${A} onClick=${()=>r(e.id)} disabled=${t}>${s("missions.action.resume")}<//>
      <${A} variant="secondary" onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runOnce")}<//>
    `:l`<${A} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runAgain")}<//>`}function TS({mission:e,isLoading:t,error:a,isBusy:n,onFire:r,onPause:s,onResume:i,onOpenProject:o,onOpenThread:u}){let c=C();return t?l`
      <div className="space-y-4">
        ${[1,2,3].map(d=>l`<div key=${d} className="v2-skeleton h-36 rounded-xl" />`)}
      </div>
    `:a||!e?l`
      <${be}
        title=${c("missions.unavailable")}
        description=${a?.message||c("missions.unavailableDesc")}
      />
    `:l`
    <div className="space-y-4">
      <${q} className="p-4 sm:p-5">
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
          <${z} tone=${ol(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${Zc} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${Zc} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${Zc} label=${c("missions.meta.nextFire")} value=${il(e.next_fire_at)} />
          <${Zc} label=${c("missions.meta.updated")} value=${il(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${W5}
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
          <${aa} content=${e.goal||c("missions.noGoal")} />
        </div>
      <//>

      ${e.current_focus&&l`
        <${q} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${aa} content=${e.current_focus} />
          </div>
        <//>
      `}

      ${e.success_criteria&&l`
        <${q} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${aa} content=${e.success_criteria} />
          </div>
        <//>
      `}

      ${e.threads?.length?l`
        <${q} className="p-4 sm:p-5">
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
                  <${z} tone=${ol(d.state==="Running"?"Active":d.state==="Failed"?"Failed":"Completed")} label=${d.state} />
                </div>
              </button>
            `)}
          </div>
        <//>
      `:null}
    </div>
  `}function eD(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function AS({value:e,onChange:t,children:a,label:n}){return l`
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
  `}function tD({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=C(),s=t===e.id;return l`
    <div
      className=${["w-full rounded-xl border p-4 text-left",s?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80"].join(" ")}
    >
      <button type="button" onClick=${()=>a(e.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${e.name}</div>
              <${z} tone=${ol(e.status)} label=${e.status} />
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
          ${r("missions.updated",{value:il(e.updated_at)})}
        </span>
        <${A}
          variant="ghost"
          onClick=${i=>{i.stopPropagation(),n(e.project.id)}}
        >
          ${e.project.name}
        <//>
      </div>
    </div>
  `}function xh({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:u,projectOptions:c,onSelectMission:d,onOpenProject:m}){let f=C(),p=eD(f);return l`
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
        <${AS} value=${s} onChange=${i} label=${f("missions.filter.status")}>
          ${p.map(x=>l`<option key=${x.value} value=${x.value}>${x.label}<//>`)}
        <//>
        <${AS} value=${o} onChange=${u} label=${f("missions.filter.project")}>
          <option value="all">${f("missions.filter.allProjects")}</option>
          ${c.map(x=>l`<option key=${x.id} value=${x.id}>${x.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(x=>l`
              <${tD}
                key=${x.id}
                mission=${x}
                selectedMissionId=${a}
                onSelectMission=${d}
                onOpenProject=${m}
              />
            `):l`
              <${be}
                title=${f("missions.emptyTitle")}
                description=${f("missions.emptyDesc")}
                boxed=${!1}
              />
            `}
      </div>
    <//>
  `}function aD(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function DS({summary:e}){let t=C(),a=aD(t);return l`
    <${q} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
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
  `}function MS(){return Promise.resolve({projects:[],todo:!0})}function OS({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function LS(e){return Promise.resolve(null)}function PS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function US(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function jS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function FS(e){let t=I({queryKey:["mission-detail",e],queryFn:()=>LS(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function nD(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function BS(){let e=J(),[t,a]=h.default.useState(null),n=I({queryKey:["projects-overview"],queryFn:MS,refetchInterval:7e3}),r=n.data?.projects||[],s=Ed({queries:r.map(f=>({queryKey:["missions","project",f.id],queryFn:()=>OS({projectId:f.id}),refetchInterval:5e3,select:p=>p?.missions||[]}))}),i=s.flatMap((f,p)=>{let x=r[p];return(f.data||[]).map(y=>nD(y,x))}),o=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),u=(f,p)=>({mutationFn:({missionId:x})=>f(x),onSuccess:()=>{a({type:"success",message:p}),o()},onError:x=>{a({type:"error",message:x.message||"Unable to update mission"})}}),c=Q(u(PS,"Mission fired and a run was queued.")),d=Q(u(US,"Mission paused.")),m=Q(u(jS,"Mission resumed."));return{projects:r,missions:i,summary:CS(i),isLoading:n.isLoading||s.some(f=>f.isLoading),isRefreshing:n.isFetching||s.some(f=>f.isFetching),error:n.error||s.find(f=>f.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending,invalidate:o}}function $h(){let e=C(),t=pe(),{missionId:a=null}=it(),[n,r]=h.default.useState(""),[s,i]=h.default.useState("all"),[o,u]=h.default.useState("all"),c=BS(),d=FS(a),m=h.default.useMemo(()=>{let g=n.trim().toLowerCase();return ES(c.missions).filter(v=>{let b=!g||[v.name,v.goal,v.project?.name].some(E=>String(E||"").toLowerCase().includes(g)),$=s==="all"||v.status===s,S=o==="all"||v.project?.id===o;return b&&$&&S})},[c.missions,o,n,s]),f=h.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),p=d.mission?{...f,...d.mission,project:f?.project||null}:f,x=h.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),y=h.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),w=a?l`
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <${xh}
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
          <${TS}
            mission=${p}
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
        <${xh}
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

          <${Wa}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${DS} summary=${c.summary} />

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
  `}var zS=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],rD=new Set(["pending","in_progress"]),qS=new Set(["failed","interrupted","stuck","cancelled"]);function ir(e){return e?String(e).replace(/_/g," "):"unknown"}function di(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":qS.has(e)?"danger":"muted":"muted"}function sD(e){return rD.has(e)}function Wc(e){return sD(e?.state)}function IS(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":qS.has(e.state):!1}function qr(e,t=8){return e?String(e).slice(0,t):"unknown"}function na(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function KS(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function wh(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${na(e.started_at)}`:null].filter(Boolean).join(" / ")}var iD=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function HS(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function oD({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?l`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${t==="tool_use"?a.tool_name||"Tool call":a.tool_name||"Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${HS(t==="tool_use"?a.input:a.output||a.error||a)}</pre>
      </details>
    `:t==="message"?l`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a.role||"assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${a.content||""}</div>
      </div>
    `:l`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t.replace(/_/g," ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${a.message||a.status||HS(a)}</div>
    </div>
  `}function QS({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=C(),[s,i]=h.default.useState("all"),[o,u]=h.default.useState(""),[c,d]=h.default.useState(!0),m=h.default.useRef(null),f=h.default.useMemo(()=>s==="all"?t:t.filter(x=>x.event_type===s),[t,s]);h.default.useEffect(()=>{c&&m.current&&(m.current.scrollTop=m.current.scrollHeight)},[c,f.length]);let p=h.default.useCallback(async(x=!1)=>{let y=o.trim();if(!(!y&&!x))try{await a({content:y||"(done)",done:x}),u("")}catch{}},[o,a]);return l`
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
            ${iD.map(x=>l`<option key=${x.value} value=${x.value}>${x.label}</option>`)}
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
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${na(x.created_at)}</div>
                <${oD} event=${x} />
              </div>
            `):l`
              <${be}
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
            onKeyDown=${x=>{x.key==="Enter"&&!x.shiftKey&&(x.preventDefault(),p(!1))}}
            placeholder=${r("job.followupPlaceholder")}
            className="h-11 rounded-md border border-white/10 bg-iron-950/90 px-3 text-sm text-white outline-none focus:border-signal/45"
          />
          <${A} variant="secondary" disabled=${n} onClick=${()=>p(!0)}>${r("common.done")}<//>
          <${A} variant="primary" disabled=${n} onClick=${()=>p(!1)}>${r("common.send")}<//>
        </div>
      `}
    <//>
  `}function VS({job:e,activeTab:t,onTabChange:a,onBack:n,onCancel:r,onRestart:s,isBusy:i,children:o}){return l`
    <div className="space-y-5">
      <${q} className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick=${n} className="text-sm text-signal hover:text-white">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-semibold tracking-tight text-white">${e.title||"Untitled job"}</h2>
              <${z} tone=${di(e.state)} label=${ir(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${qr(e.id)}</span>
              <span>created ${na(e.created_at)}</span>
              ${wh(e)&&l`<span>${wh(e)}</span>`}
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
            ${Wc(e)&&l`
              <${A} variant="secondary" disabled=${i} onClick=${()=>r(e.id)}>Cancel<//>
            `}
            ${IS(e)&&l`
              <${A} variant="primary" disabled=${i} onClick=${()=>s(e.id)}>Restart<//>
            `}
          </div>
        </div>
      <//>

      <div className="flex flex-wrap gap-2">
        ${zS.map(u=>l`
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
  `}function GS({nodes:e,depth:t=0,selectedPath:a,expandingPath:n,onToggleDirectory:r,onSelectPath:s}){return l`
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
        ${i.isDir&&i.expanded&&i.children?.length?l`<${GS}
              nodes=${i.children}
              depth=${t+1}
              selectedPath=${a}
              expandingPath=${n}
              onToggleDirectory=${r}
              onSelectPath=${s}
            />`:null}
      </div>
    `)}
  `}function YS({canBrowse:e,tree:t,selectedPath:a,selectedFile:n,fileError:r,isLoadingTree:s,isLoadingFile:i,expandingPath:o,treeError:u,onToggleDirectory:c,onSelectPath:d}){return e?l`
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <${q} className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          ${u&&l`<div className="mx-2 mb-3 rounded-md border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${u}</div>`}
          ${s?l`<div className="space-y-2 px-2">${[1,2,3,4].map(m=>l`<div key=${m} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?l`
                  <${GS}
                    nodes=${t}
                    selectedPath=${a}
                    expandingPath=${o}
                    onToggleDirectory=${c}
                    onSelectPath=${d}
                  />
                `:l`<div className="px-2 py-6 text-sm text-iron-300">No files were recorded for this workspace.</div>`}
        </div>
      <//>

      <${q} className="min-h-[440px] p-5 sm:p-6">
        <div className="border-b border-white/10 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">File preview</div>
          <p className="mt-2 break-all text-sm leading-6 text-iron-300">${n?.path||a||"Select a file from the tree to inspect its contents."}</p>
        </div>

        ${r&&!i?l`<div className="mt-5 rounded-md border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">${r}</div>`:i?l`<div className="mt-5 space-y-3">${[1,2,3,4,5].map(m=>l`<div key=${m} className="v2-skeleton h-4 rounded" />`)}</div>`:n?l`<pre className="mt-5 max-h-[60vh] overflow-auto whitespace-pre-wrap rounded-[18px] border border-white/10 bg-iron-950/90 p-4 font-mono text-xs leading-6 text-iron-100">${n.content}</pre>`:l`
                <${be}
                  title="No file selected"
                  description="Pick a concrete file from the workspace tree to render it here."
                />
              `}
      <//>
    </div>
  `:l`
      <${be}
        title="No project workspace"
        description="File browsing is only available for sandbox jobs that produced a mounted project directory."
      />
    `}function mi({label:e,value:t}){return l`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function JS({job:e}){let t=(e.transitions||[]).map(a=>({title:`${ir(a.from)} -> ${ir(a.to)}`,description:[na(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${q} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${z} tone=${di(e.state)} label=${ir(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${mi} label="Created" value=${na(e.created_at)} />
          <${mi} label="Started" value=${na(e.started_at)} />
          <${mi} label="Completed" value=${na(e.completed_at)} />
          <${mi} label="Duration" value=${KS(e.elapsed_secs)} />
          <${mi} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${mi} label="Mode" value=${e.job_mode||"Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${q} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${e.description?l`<${aa} content=${e.description} className="mt-4 text-sm leading-7 text-iron-200" />`:l`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
        <//>

        ${t.length?l`
              <${q} className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Transitions</div>
                <h3 className="mt-2 text-xl font-semibold text-white">State timeline</h3>
                <div className="mt-3">
                  <${K2} items=${t} />
                </div>
              <//>
            `:l`
              <${be}
                title="No state history yet"
                description="Transitions appear here once the job advances or records a recovery event."
              />
            `}
      </div>
    </div>
  `}function XS({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:u,isBusy:c,isRefreshing:d}){let m=C(),f=[{value:"all",label:m("jobs.list.filter.all")},{value:"pending",label:m("jobs.list.filter.pending")},{value:"in_progress",label:m("jobs.list.filter.inProgress")},{value:"completed",label:m("jobs.list.filter.completed")},{value:"failed",label:m("jobs.list.filter.failed")},{value:"stuck",label:m("jobs.list.filter.stuck")}];if(!e.length){let p=!!n.trim()||s!=="all";return l`
      <${be}
        title=${m(t&&p?"jobs.list.empty.noMatchTitle":"jobs.list.empty.noJobsTitle")}
        description=${m(t&&p?"jobs.list.empty.noMatchDesc":"jobs.list.empty.noJobsDesc")}
      />
    `}return l`
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
            onInput=${p=>r(p.target.value)}
            placeholder=${m("jobs.list.searchPlaceholder")}
            className="h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          />
          <select
            value=${s}
            onChange=${p=>i(p.target.value)}
            className="v2-select h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          >
            ${f.map(p=>l`<option key=${p.value} value=${p.value}>${p.label}</option>`)}
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
                  <h3 className="truncate text-lg font-semibold text-iron-100">${p.title||m("jobs.list.untitled")}</h3>
                  <${z} tone=${di(p.state)} label=${ir(p.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${qr(p.id)}</span>
                  <span>${m("jobs.list.created",{value:na(p.created_at)})}</span>
                  ${p.started_at&&l`<span>${m("jobs.list.started",{value:na(p.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${Wc(p)&&l`
                  <${A}
                    variant="secondary"
                    className="h-9 px-3 text-xs"
                    disabled=${c}
                    onClick=${()=>u(p.id)}
                  >
                    ${m("jobs.action.cancel")}
                  <//>
                `}
                <${A} variant="ghost" className="h-9 px-3 text-xs" onClick=${()=>o(p.id)}>${m("jobs.action.open")}<//>
              </div>
            </div>
          </article>
        `)}
      </div>
    </div>
  `}var lD=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function ZS({summary:e}){return l`
    <${q} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${lD.map(t=>l`
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
  `}function WS(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function eN(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function tN(e){return Promise.resolve(null)}function aN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function nN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function rN(e){return Promise.resolve({events:[],todo:!0})}function sN(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function Sh(e,t=""){return Promise.resolve({entries:[],todo:!0})}function iN(e,t){return Promise.resolve({content:"",todo:!0})}function oN(e){let t=J(),[a,n]=h.default.useState(null),r=I({queryKey:["job-detail",e],queryFn:()=>tN(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=I({queryKey:["job-events",e],queryFn:()=>rN(e),enabled:!!e,refetchInterval:e?2500:!1}),i=Q({mutationFn:({content:o,done:u})=>sN(e,{content:o,done:u}),onSuccess:(o,{done:u})=>{n({type:"success",message:u?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return h.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function lN(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function uN(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=uN(a.children,t);if(n)return n}}return null}function ed(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:ed(n.children,t,a)}:n)}function cN(e){let[t,a]=h.default.useState([]),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState(""),c=!!(e?.project_dir&&e?.id),d=I({queryKey:["job-files-root",e?.id],queryFn:()=>Sh(e.id,""),enabled:c}),m=I({queryKey:["job-file",e?.id,n],queryFn:()=>iN(e.id,n),enabled:!!(c&&n)});h.default.useEffect(()=>{a([]),r(""),i(""),u("")},[e?.id]),h.default.useEffect(()=>{d.data?.entries?(a(lN(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let f=h.default.useCallback(async p=>{let x=uN(t,p);if(!(!x||!e?.id)){if(x.expanded){a(y=>ed(y,p,w=>({...w,expanded:!1})));return}if(x.loaded){a(y=>ed(y,p,w=>({...w,expanded:!0})));return}u(p);try{let y=await Sh(e.id,p);a(w=>ed(w,p,g=>({...g,expanded:!0,loaded:!0,children:lN(y.entries)}))),i("")}catch(y){i(y.message||"Unable to open folder")}finally{u("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:m.data||null,fileError:m.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:m.isLoading||m.isFetching,expandingPath:o,treeError:s,toggleDirectory:f}}function dN(){let e=J(),[t,a]=h.default.useState(null),n=I({queryKey:["jobs-summary"],queryFn:eN,refetchInterval:5e3}),r=I({queryKey:["jobs"],queryFn:WS,refetchInterval:5e3}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=Q({mutationFn:({jobId:u})=>aN(u),onSuccess:(u,{jobId:c})=>{a({type:"success",message:`Job ${qr(c)} cancelled`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to cancel job"})}}),o=Q({mutationFn:({jobId:u})=>nN(u),onSuccess:u=>{a({type:"success",message:`Restart queued as ${qr(u?.new_job_id)}`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function mN({result:e,onDismiss:t}){let a=C();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return l`
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
  `}function Nh(){let e=C(),t=pe(),{jobId:a=null}=it(),[n,r]=h.default.useState(""),[s,i]=h.default.useState("all"),[o,u]=h.default.useState(a?"activity":"overview"),c=dN(),d=oN(a),m=cN(d.job);h.default.useEffect(()=>{u(a?"activity":"overview")},[a]);let f=h.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(b=>{let $=!v||b.title.toLowerCase().includes(v)||b.id.toLowerCase().includes(v),S=s==="all"||b.state===s;return $&&S})},[c.jobs,n,s]),p=h.default.useCallback(v=>t(`/jobs/${v}`),[t]),x=h.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),y=h.default.useCallback(async v=>{try{let b=await c.restartJob({jobId:v});b?.new_job_id&&t(`/jobs/${b.new_job_id}`)}catch{}},[c,t]),w=l`
    ${a&&l`<${A} variant="ghost" onClick=${()=>t("/jobs")}
      >${e("jobs.allJobs")}<//
    >`}
  `,g=null;if(a)if(d.isLoading)g=l`
        <div className="space-y-4">
          ${[1,2,3].map(v=>l`<div key=${v} className="v2-skeleton h-32 rounded-[18px]" />`)}
        </div>
      `;else if(d.error||!d.job)g=l`
        <${be}
          title=${e("jobs.unavailable")}
          description=${d.error?.message||e("jobs.unavailableDesc")}
        >
          <${A} variant="secondary" onClick=${()=>t("/jobs")}
            >${e("jobs.returnToJobs")}<//
          >
        <//>
      `;else{let v={overview:l`<${JS} job=${d.job} />`,activity:l`
          <${QS}
            job=${d.job}
            events=${d.events}
            onSendPrompt=${d.sendPrompt}
            isSendingPrompt=${d.isSendingPrompt}
          />
        `,files:l`
          <${YS}
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
        <${VS}
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
          <${XS}
            jobs=${f}
            totalJobs=${c.jobs.length}
            selectedJobId=${a}
            search=${n}
            onSearchChange=${r}
            stateFilter=${s}
            onStateFilterChange=${i}
            onSelectJob=${p}
            onCancelJob=${x}
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
          <${mN}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${mN}
            result=${d.promptResult}
            onDismiss=${d.clearPromptResult}
          />
          <${ZS} summary=${c.summary} />
          ${g}
        </div>
      </div>
    </div>
  `}function or(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not scheduled"}function td(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function ad(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function fN(e=[]){return[...e].sort((t,a)=>t.enabled!==a.enabled?t.enabled?-1:1:new Date(a.next_fire_at||a.last_run_at||0).getTime()-new Date(t.next_fire_at||t.last_run_at||0).getTime())}function pN(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function uD(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function hN({runs:e}){return e?.length?l`
    <div className="space-y-3">
      ${e.map(t=>l`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${z} tone=${uD(t.status)} label=${t.status} />
              <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                ${or(t.started_at)}
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
    `}function lr({label:e,value:t}){return l`
    <div className="rounded-xl border border-iron-700 bg-iron-950/50 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div className="mt-2 min-w-0 break-words text-sm text-iron-100">
        ${t||"\u2014"}
      </div>
    </div>
  `}function vN({title:e,value:t}){return l`
    <div>
      <h3 className="text-sm font-semibold text-iron-100">${e}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >${JSON.stringify(t||{},null,2)}</pre>
    </div>
  `}function gN({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=pe(),u=C();return t?l`
      <div className="space-y-4">
        ${[1,2,3].map(c=>l`<div key=${c} className="v2-skeleton h-32 rounded-xl" />`)}
      </div>
    `:a||!e?l`
      <${be}
        title=${u("routine.unavailable")}
        description=${a?.message||u("routine.unavailableDesc")}
      />
    `:l`
    <${q} className="p-4 sm:p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="truncate text-2xl font-semibold tracking-tight text-iron-100">
              ${e.name}
            </h2>
            <${z}
              tone=${td(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${z}
              tone=${ad(e.verification_status)}
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
        <${lr} label="Trigger" value=${e.trigger_summary||e.trigger_type} />
        <${lr} label="Action" value=${pN(e.action)} />
        <${lr} label="Next fire" value=${or(e.next_fire_at)} />
        <${lr} label="Last run" value=${or(e.last_run_at)} />
        <${lr} label="Run count" value=${e.run_count} />
        <${lr} label="Failures" value=${e.consecutive_failures} />
        <${lr} label="Created" value=${or(e.created_at)} />
        <${lr} label="Routine ID" value=${e.id} />
      </div>

      ${e.conversation_id&&l`
        <div className="mt-5">
          <${A} variant="secondary" onClick=${()=>o(`/chat/${e.conversation_id}`)}>
            Open routine thread
          <//>
        </div>
      `}

      <div className="mt-6 grid gap-6 xl:grid-cols-2">
        <${vN} title=${u("routine.triggerPayload")} value=${e.trigger} />
        <${vN} title=${u("routine.actionPayload")} value=${e.action} />
      </div>

      <div className="mt-6">
        <h3 className="mb-3 text-sm font-semibold text-iron-100">Recent runs</h3>
        <${hN} runs=${e.recent_runs} />
      </div>
    <//>
  `}function yN({routine:e,selectedRoutineId:t,onSelectRoutine:a,onTriggerRoutine:n,onToggleRoutine:r,isBusy:s}){let i=t===e.id;return l`
    <article
      className=${["group flex flex-col gap-4 rounded-[18px] border p-5",i?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick=${()=>a(e.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">${e.name}</h3>
            <${z}
              tone=${td(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${z}
              tone=${ad(e.verification_status)}
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
            <span>next ${or(e.next_fire_at)}</span>
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
  `}var cD=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function _h({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:u,onToggleRoutine:c,isBusy:d,isRefreshing:m}){let f=C();if(!e.length){let p=!!n.trim()||s!=="all";return l`
      <${be}
        title=${t&&p?"No routines match":"No routines yet"}
        description=${t&&p?"Adjust the search or status filter to find a saved routine.":"Routines created from chat will appear here after they are saved."}
      />
    `}return l`
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
            onInput=${p=>r(p.target.value)}
            placeholder="Search routine name, trigger, or action"
            className="h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          />
          <select
            value=${s}
            onChange=${p=>i(p.target.value)}
            className="v2-select h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          >
            ${cD.map(p=>l`<option key=${p.value} value=${p.value}>${p.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(p=>l`
            <${yN}
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
  `}var dD=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function bN({summary:e}){return l`
    <${q} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${dD.map(t=>l`
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
  `}function xN(e){let[t,a]=h.default.useState(""),[n,r]=h.default.useState("all");return{filteredRoutines:h.default.useMemo(()=>{let i=t.trim().toLowerCase();return fN(e).filter(o=>{let u=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||u.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function $N(){return Promise.resolve({routines:[],todo:!0})}function wN(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function SN(e){return Promise.resolve(null)}function nd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function rd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function NN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function _N(e){let t=J(),[a,n]=h.default.useState(null),r=I({queryKey:["routine-detail",e],queryFn:()=>SN(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:m=>{n({type:"error",message:m.message||"Unable to update routine"})}}),o=Q(i(nd,"Routine run queued.")),u=Q(i(rd,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,isBusy:o.isPending||u.isPending}}function kN(){let e=J(),[t,a]=h.default.useState(null),n=I({queryKey:["routines-summary"],queryFn:wN,refetchInterval:5e3}),r=I({queryKey:["routines"],queryFn:$N,refetchInterval:5e3}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,m)=>({mutationFn:({routineId:f})=>d(f),onSuccess:()=>{a({type:"success",message:m}),s()},onError:f=>{a({type:"error",message:f.message||"Unable to update routine"})}}),o=Q(i(nd,"Routine run queued.")),u=Q(i(rd,"Routine status updated.")),c=Q(i(NN,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||u.isPending||c.isPending,invalidate:s}}function kh(){let e=pe(),{routineId:t=null}=it(),a=kN(),n=_N(t),r=xN(a.routines),s=h.default.useCallback(async(u,c)=>{try{await u({routineId:c})}catch{}},[]),i=h.default.useCallback(async(u,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:u}),e("/routines")}catch{}},[e,a]),o=t?l`
        <div className="grid gap-5 xl:grid-cols-[minmax(0,0.9fr)_minmax(440px,1.1fr)]">
          <${_h}
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
          <${gN}
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
        <${_h}
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

          <${Wa}
            result=${a.actionResult}
            onDismiss=${a.clearActionResult}
          />
          <${Wa}
            result=${n.actionResult}
            onDismiss=${n.clearActionResult}
          />
          <${bN} summary=${a.summary} />

          ${a.isLoading?l`
                <div className="space-y-4">
                  ${[1,2,3].map(u=>l`<div key=${u} className="v2-skeleton h-32 rounded-xl" />`)}
                </div>
              `:o}
        </div>
      </div>
    </div>
  `}function mD(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function fD(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function RN({deliveryState:e}){let t=C(),a=e.currentTarget?.target_id||"",[n,r]=h.default.useState(a),[s,i]=h.default.useState(!1),o=h.default.useRef(null);h.default.useEffect(()=>{r(a)},[a]),h.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let u=n!==a,c=e.isLoading||e.isSaving,d=u&&!c,m=!!a&&!c,f=e.finalReplyTargets.length>0,p=e.targets.some(L=>L?.capabilities?.final_replies&&L?.target?.status==="unavailable"),x=f||p,y=L=>(o.current&&clearTimeout(o.current),i(!1),L.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),w=()=>{d&&y(e.saveFinalReplyTarget(n||null))},g=()=>{m&&(r(""),y(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),b=e.currentStatus,$=b==="available"?"success":b==="unavailable"?"warning":"muted",S=t(b==="available"?"automations.delivery.pill.ready":b==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),E=!!e.currentTarget,_=t(E?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),T=fD(t("automations.delivery.footnote"),{command:l`<code
        key="cmd"
        className="rounded px-1.5 py-0.5 font-mono text-[0.6875rem] bg-[var(--v2-surface-muted)] text-[var(--v2-accent-text)]"
      >
        approve &lt;code&gt;
      </code>`});return l`
    <${q} className="p-5 sm:p-6">
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
              <${z} tone=${$} label=${S} />
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
            ${e.finalReplyTargets.map(L=>{let D=L?.target?.target_id??"",P=L?.target?.display_name||L?.target?.target_id||"",R=L?.target?.description||"",F=L?.target?.status??"available",G=n===D;return l`
                <label
                  key=${D}
                  className=${V("flex items-start gap-3.5 rounded-xl border px-4 py-3.5 cursor-pointer","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]","hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",G&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
                >
                  <input
                    type="radio"
                    name="delivery-target"
                    value=${D}
                    checked=${G}
                    disabled=${c}
                    onChange=${()=>r(D)}
                    className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-semibold text-[var(--v2-text-strong)] leading-snug">
                      ${P}
                    </div>
                    ${R&&l`<div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                      ${R}
                    </div>`}
                  </div>
                  <${z}
                    tone=${mD(F)}
                    label=${t(F==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.ready")}
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
                <${z}
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
            variant="primary"
            size="sm"
            disabled=${!d}
            onClick=${w}
          >
            <${O} name="check" className="h-3.5 w-3.5" />
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
              <${O} name="check" className="h-3 w-3" />
              ${t("automations.delivery.saved")}
            </span>
          `}
          ${e.saveError&&!s&&l`
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
        ${x&&l`
          <div
            className="rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-faint)]"
          >
            ${T}
          </div>
        `}

      </div>
    <//>
  `}var pD=["schedule","once"],EN={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},TN={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},AN={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function ra(e){return typeof e=="function"?e:t=>t}var Ch=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:Cn},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:ED},{value:"completed",labelKey:"automations.filter.completed",predicate:TD}];function DN(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>pD.includes(r?.source?.type)).map(r=>ND(r,t,a)).sort(CD)}function MN(e,t){let a=Ch.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function ON(e){let t=e.filter(i=>i.state!=="completed"),a=t.filter(i=>Cn(i)).length,n=t.filter(i=>i.has_running_run).length,r=t.filter(i=>i.has_failed_runs).length,s=t.filter(i=>Cn(i)&&Rh(i)!=null).sort((i,o)=>(i.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(o.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:t.length,active:a,running:n,failures:r,nextRun:s?.next_run_label||null}}function hD(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=OD(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:u,month:c,dayOfWeek:d,year:m}=s,f=t&&typeof t=="string"?t:null,p=f?` (${f})`:"",x=m==="*"&&u==="*"&&c==="*"&&d==="*";if(x&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=LD(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(ur(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let y=AD(o,i,n);if(!y)return r("automations.schedule.custom");if(x)return r("automations.schedule.everyDayAt",{time:y})+p;let w=PD(d);if(m==="*"&&u==="*"&&c==="*"&&w==="1-5")return r("automations.schedule.weekdaysAt",{time:y})+p;if(m==="*"&&u==="*"&&c==="*"&&ur(w,0,7)){let g=DD(Number(w)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:y})+p}if(m==="*"&&ur(u,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(u),time:y})+p;if(ur(u,1,31)&&ur(c,1,12)&&d==="*"&&(m==="*"||ur(m,1970,9999))){let g=MD(Number(c),Number(u),m==="*"?null:Number(m),n);return r("automations.schedule.dateAt",{date:g,time:y})+p}return r("automations.schedule.custom")}function Ir(e,t="Unknown",a,n){if(!e)return t;let r=new Date(e);if(Number.isNaN(r.getTime()))return t;let s=n&&typeof n=="string"?{timeZone:n}:{};try{return r.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...s})}catch{return r.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function LN(e,t){let a=EN[e]?.labelKey||"automations.state.unknown";return ra(t)(a)}function PN(e){return EN[e]?.tone||"muted"}function vD(e,t){return Cn(e)&&e?.has_running_run?ra(t)("automations.status.running"):Cn(e)&&e?.has_failed_runs?ra(t)("automations.status.needsReview"):LN(e?.state,t)}function gD(e){return Cn(e)&&e?.has_running_run?"info":Cn(e)&&e?.has_failed_runs?"danger":PN(e?.state)}function yD(e,t){let a=TN[e]?.labelKey||"automations.lastStatus.none";return ra(t)(a)}function bD(e){return TN[e]?.tone||"muted"}function xD(e,t){let a=AN[sd(e)]?.labelKey||"automations.runStatus.unknown";return ra(t)(a)}function $D(e){return AN[sd(e)]?.tone||"muted"}function wD(e,t,a,n){if(!e)return ra(a)("automations.schedule.custom");let r=Ir(e,null,n,t);if(!r)return ra(a)("automations.schedule.custom");let s=t&&typeof t=="string"?` (${t})`:"";return ra(a)("automations.schedule.onceAt",{datetime:r})+s}function SD(e,t,a){return e?.type==="once"?wD(e.at,e.timezone,t,a):e?.type==="schedule"?hD(e.cron,e.timezone||"UTC",t,a):ra(t)("automations.schedule.custom")}function ND(e,t,a){let n=ra(t),r=_D(e.recent_runs,t,a),s=r[0]||null,i=r.find(m=>m.status==="running")||null,o=r.find(m=>m.status==="ok"||m.status==="error")||null,u=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null,d={...e,recent_runs:r,has_running_run:r.some(m=>m.status==="running"),has_failed_runs:r.some(m=>m.status==="error")};return{...d,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:SD(e.source,t,a),state_label:LN(e.state,t),state_tone:PN(e.state),primary_status_label:vD(d,t),primary_status_tone:gD(d),next_run_timestamp:Eh(e.next_run_at),next_run_label:Ir(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:Ir(c,n("automations.date.noRuns"),a),last_status_label:yD(u,t),last_status_tone:bD(u),created_label:Ir(e.created_at,n("automations.date.unknown"),a),latest_run:s,current_run:i,success_rate_label:RD(r,t)}}function _D(e,t,a){let n=ra(t);return Array.isArray(e)?e.map(r=>{let s=sd(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=Eh(i);return{...r,status:s,status_label:xD(s,t),status_tone:$D(s),timestamp:o,timestamp_source:i,fired_label:Ir(i,n("automations.date.unscheduled"),a),submitted_label:Ir(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:Ir(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function sd(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function UN(e){let t=Array.isArray(e)?e:[],a={total:t.length,ok:0,error:0,running:0,unknown:0};for(let n of t){let r=sd(n?.status);Object.prototype.hasOwnProperty.call(a,r)?a[r]+=1:a.unknown+=1}return a}function kD(e){let t=UN(e);return[{key:"ok",tone:"text-emerald-300",count:t.ok},{key:"error",tone:"text-red-300",count:t.error},{key:"running",tone:"text-sky-300",count:t.running},{key:"unknown",tone:"text-iron-400",count:t.unknown}].filter(a=>a.count>0)}function jN(e,t){let a=ra(t),n=UN(e),r=kD(e).map(s=>({...s,text:a(`automations.runs.${s.key}`,{count:s.count})}));return{total:n.total,totalText:a("automations.runs.total",{count:n.total}),chips:r}}function RD(e,t){let a=ra(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function CD(e,t){let a=Cn(e),n=Cn(t);return a!==n?a?-1:1:(Rh(e)??Number.MAX_SAFE_INTEGER)-(Rh(t)??Number.MAX_SAFE_INTEGER)}function Eh(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function Cn(e){return e?.state==="active"||e?.state==="scheduled"}function ED(e){return["paused","disabled","inactive"].includes(e?.state)}function TD(e){return e?.state==="completed"}function Rh(e){return e?.next_run_timestamp??Eh(e?.next_run_at)}function Th(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function AD(e,t,a){return!ur(e,0,23)||!ur(t,0,59)?null:Th(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function DD(e,t){return Th(t,{weekday:"long"},new Date(2001,0,7+e))}function MD(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return Th(n,r,new Date(a??2e3,e-1,t))}function OD(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&CN(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&CN(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function CN(e){return/^0+$/.test(e)}function ur(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function LD(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function PD(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}var UD=8;function Ah(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function id({runs:e=[]}){let t=C(),a=Array.isArray(e)?e:[],n=a.slice(0,UD);if(!n.length)return l`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;let r=a.length-n.length,s=`+${Math.min(r,999)}`;return l`
    <div
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
    >
      ${n.map(i=>l`
        <span
          key=${Ah(i)}
          title=${`${i.status_label} \xB7 ${i.fired_label}`}
          className=${V("h-3 w-3 rounded-full border",i.status==="ok"&&"border-emerald-300/50 bg-emerald-400",i.status==="error"&&"border-red-300/50 bg-red-400",i.status==="running"&&"border-sky-300/60 bg-sky-400",i.status==="unknown"&&"border-iron-500 bg-iron-600")}
        />
      `)}
      ${r>0&&l`<span
        className="ml-0.5 font-mono text-[11px] text-iron-400"
        title=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
      >
        ${s}
      </span>`}
    </div>
  `}function od({runs:e=[],className:t=""}){let a=C(),n=jN(e,a);return n.total?l`
    <div className=${V("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]",t)}>
      <span className="text-iron-300">${n.totalText}</span>
      ${n.chips.map(r=>l`<span key=${r.key} className=${r.tone}>${r.text}</span>`)}
    </div>
  `:l`<span className=${V("text-[11px] text-iron-400",t)}>
      ${a("automations.table.noRuns")}
    </span>`}function FN({run:e,onOpenRun:t,onOpenLogs:a}){let n=C(),r=!!e.chat_path,s=zc({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return l`
    <div className="grid gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto] sm:items-center">
      <div>
        <${z} tone=${e.status_tone} label=${e.status_label} />
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
          <${O} name="chat" className="mr-1.5 h-4 w-4" />
          ${n("automations.detail.openRun")}
        <//>
        <${A}
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
  `}function ld({label:e,value:t,tone:a}){return l`
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
  `}function BN({automation:e,isMutating:t=!1,onPauseAutomation:a,onResumeAutomation:n,onDeleteAutomation:r}){let s=C(),i=pe();if(!e)return l`
      <${q} className="p-4 sm:p-5">
        <${be}
          boxed=${!1}
          title=${s("automations.detail.emptyTitle")}
          description=${s("automations.detail.emptyDescription")}
        />
      <//>
    `;let o=e.current_run,u=e.state==="paused",c=e.state==="active"||e.state==="scheduled",m=`${s(u?"missions.action.resume":"missions.action.pause")}: ${e.display_name}`,f=()=>{if(u){n?.(e.automation_id);return}c&&a?.(e.automation_id)},p=`${s("common.delete")}: ${e.display_name}`,x=()=>{window.confirm(p)&&r?.(e.automation_id)};return l`
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
                <${O} name=${u?"play":"pause"} className="h-4 w-4" />
              <//>
            `}
            <${A}
              type="button"
              variant="danger"
              size="icon-sm"
              aria-label=${p}
              title=${p}
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
          <${ld} label=${s("automations.detail.schedule")} value=${e.schedule_label} />
          <${ld}
            label=${s("automations.detail.successRate")}
            value=${e.success_rate_label}
            tone=${e.has_failed_runs?"danger":"success"}
          />
          <${ld} label=${s("automations.detail.lastCompleted")} value=${e.last_run_label} />
          <${ld}
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
              <${id} runs=${e.recent_runs} />
              <${od} runs=${e.recent_runs} />
            </div>
          </div>

          ${e.recent_runs.length?l`
                <div>
                  ${e.recent_runs.map(y=>l`
                    <${FN}
                      key=${Ah(y)}
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
  `}var jD=["automations.empty.example1","automations.empty.example2","automations.empty.example3"];function FD({promptKey:e}){let t=C(),a=t(e),[n,r]=h.default.useState(!1),s=h.default.useRef(null);return h.default.useEffect(()=>()=>clearTimeout(s.current),[]),l`
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
  `}function zN(){let e=C(),t=pe();return l`
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
            ${jD.map(a=>l`<${FD} key=${a} promptKey=${a} />`)}
          </ul>
        </div>

        <div className="mt-6">
          <${A} variant="primary" size="sm" onClick=${()=>t("/chat")}>
            <${O} name="chat" className="mr-1.5 h-4 w-4" />
            ${e("automations.empty.startInChat")}
          <//>
        </div>
      </div>
    <//>
  `}function qN({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,isMutating:s,selectedAutomationId:i,onSelectAutomation:o,onPauseAutomation:u,onResumeAutomation:c,onDeleteAutomation:d}){let m=C(),f=MN(e,t),p=e.length>0,x=f.find(y=>y.automation_id===i)||f[0]||null;return l`
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
              ${Ch.map(y=>l`
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
            <${A}
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

      ${f.length?l`
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
                      ${f.map(y=>{let w=y.automation_id===x?.automation_id;return l`
                          <tr
                            key=${y.automation_id}
                            className=${V("border-b border-[var(--v2-panel-border)] last:border-0 hover:bg-white/[0.03]",w&&"bg-[var(--v2-accent-soft)]/30")}
                          >
                            <td className="max-w-[280px] px-5 py-4 align-top">
                              <button
                                type="button"
                                aria-pressed=${w}
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
                                <${id} runs=${y.recent_runs} />
                                <${od} runs=${y.recent_runs} />
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

              <${BN}
                automation=${x}
                isMutating=${s}
                onPauseAutomation=${u}
                onResumeAutomation=${c}
                onDeleteAutomation=${d}
              />
            </div>
          `:p?l`
              <${be}
                title=${m("automations.empty.matchingTitle")}
                description=${m("automations.empty.matchingDescription")}
              />
            `:l`<${zN} />`}
    </div>
  `}function IN({summary:e,activeFilter:t,onSelectFilter:a}){let n=C(),r=[{key:"scheduled",label:n("automations.summary.scheduled"),value:e?.scheduled??0,tone:"muted",detail:n("automations.summary.scheduledDetail"),filter:"all"},{key:"active",label:n("automations.summary.active"),value:e?.active??0,tone:"signal",detail:n("automations.summary.activeDetail"),filter:"active"},{key:"running",label:n("automations.summary.running"),value:e?.running??0,tone:"info",detail:n("automations.summary.runningDetail"),filter:"running"},{key:"failures",label:n("automations.summary.failures"),value:e?.failures??0,tone:(e?.failures??0)>0?"danger":"success",detail:n("automations.summary.failuresDetail"),filter:(e?.failures??0)>0?"failures":null},{key:"nextRun",label:n("automations.summary.nextRun"),value:e?.nextRun||n("automations.summary.none"),tone:"info",detail:n("automations.summary.nextRunDetail"),valueClassName:"text-lg md:text-xl"}];return l`
    <${q} className="p-4 sm:p-5">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        ${r.map(s=>{let i=!!(s.filter&&a),o=i&&t===s.filter,u=l`
            <${tt}
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
  `}function BD(e){return e==="active"||e==="scheduled"}function zD(e){return Number.isFinite(e)?e:null}function KN(e,t=Date.now()){let a=Array.isArray(e)?e:[],n=null;for(let r of a){if(!r||(r.has_running_run&&(n=n==null?5e3:Math.min(n,5e3)),!BD(r.state)))continue;let s=zD(r.next_run_timestamp);if(s==null)continue;let i=s-t,o=i<=0?2e3:i<3e4?Math.max(1e3,i+1200):null;o!=null&&(n=n==null?o:Math.min(n,o))}return n}var ID=50,KD=25;function HN(e=!1){let{t,lang:a}=cl(),n=J(),r=I({queryKey:["automations",{includeCompleted:e}],queryFn:()=>Tx({limit:ID,runLimit:KD,includeCompleted:e}),refetchInterval:3e4,refetchIntervalInBackground:!1}),s=h.default.useMemo(()=>DN(r.data,t,a),[r.data,t,a]),i=h.default.useMemo(()=>ON(s),[s]),o=h.default.useMemo(()=>KN(s),[s]);h.default.useEffect(()=>{if(o==null)return;let p=setTimeout(()=>{r.refetch()},o);return()=>clearTimeout(p)},[o,r.refetch]);let u=r.data?.scheduler_enabled!==!1,c=h.default.useCallback(()=>{n.invalidateQueries({queryKey:["automations"]})},[n]),d=Q({mutationFn:p=>Ax({automationId:p}),onSuccess:c}),m=Q({mutationFn:p=>Dx({automationId:p}),onSuccess:c}),f=Q({mutationFn:p=>Mx({automationId:p}),onSuccess:c});return{automations:s,summary:i,schedulerEnabled:u,isLoading:r.isLoading,isRefreshing:r.isFetching,isMutating:d.isPending||m.isPending||f.isPending,error:r.error||null,actionError:d.error||m.error||f.error||null,pauseAutomation:d.mutate,resumeAutomation:m.mutate,deleteAutomation:f.mutate,refetch:r.refetch}}var QN=["outbound-delivery","preferences"],VN=["outbound-delivery","targets"];function GN(){let e=J(),t=I({queryKey:QN,queryFn:Ux}),a=I({queryKey:VN,queryFn:jx}),n=Q({mutationFn:({finalReplyTargetId:i})=>Fx({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(QN,i),e.invalidateQueries({queryKey:VN})}}),r=h.default.useMemo(()=>a.data?.targets??[],[a.data]),s=h.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function YN(){let e=C(),[t,a]=h.default.useState("all"),[n,r]=h.default.useState(null),i=HN(t==="completed"),o=GN(),[u,c]=h.default.useState(!1),d=h.default.useRef(null);h.default.useEffect(()=>()=>clearTimeout(d.current),[]);let m=h.default.useCallback(()=>{c(!0),clearTimeout(d.current),d.current=setTimeout(()=>c(!1),1e3),i.refetch()},[i.refetch]),f=i.isRefreshing||u,p=i.error&&!i.isLoading&&i.automations.length===0;return h.default.useEffect(()=>{if(!i.automations.length){r(null);return}i.automations.some(y=>y.automation_id===n)||r(i.automations[0].automation_id)},[i.automations,n]),l`
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
                <${IN}
                  summary=${i.summary}
                  activeFilter=${t}
                  onSelectFilter=${a}
                />
                <${RN} deliveryState=${o} />

                ${i.isLoading?l`
                      <div className="space-y-4">
                        ${[1,2,3].map(x=>l`<div
                              key=${x}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`)}
                      </div>
                    `:l`
                      <${qN}
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
  `}var JN={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function XN({result:e,onDismiss:t}){return h.default.useEffect(()=>{if(!e)return;let a=setTimeout(t,4e3);return()=>clearTimeout(a)},[e,t]),e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",JN[e.type]||JN.info].join(" ")}>
      <${O}
        name=${e.type==="success"?"check":e.type==="error"?"close":"bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">
        <${O} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `:null}var WN="/api/webchat/v2/channels/slack/setup";function e_(){return H(WN)}function t_(e){let t={installation_id:String(e.installation_id||"").trim(),team_id:String(e.team_id||"").trim(),api_app_id:String(e.api_app_id||"").trim(),user_id:ZN(e.user_id),shared_subject_user_id:ZN(e.shared_subject_user_id)},a=String(e.bot_token||"").trim(),n=String(e.signing_secret||"").trim();return a&&(t.bot_token=a),n&&(t.signing_secret=n),H(WN,{method:"PUT",body:JSON.stringify(t)})}function Dh(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function ZN(e){let t=String(e||"").trim();return t||null}var a_="/api/webchat/v2/channels/slack/allowed",HD="/api/webchat/v2/channels/slack/subjects";function n_(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function r_(){return H(a_)}function s_(){return H(HD)}function i_(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return H(a_,{method:"PUT",body:JSON.stringify(n)})}function o_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var l_=["slack-allowed-channels"];function c_({action:e}){let t=C(),a=J(),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState([]),c=VD(e,t),d=I({queryKey:l_,queryFn:r_}),m=I({queryKey:["slack-routable-subjects"],queryFn:s_}),f=m.data?.subjects||[],p=u_(f),x=m.isSuccess||m.isError,y=f.length>0;h.default.useEffect(()=>{d.data&&u(Mh(d.data.channels||[]))},[d.data]);let w=Q({mutationFn:({channels:E})=>i_(E),onSuccess:E=>{u(Mh(E.channels||[])),a.invalidateQueries({queryKey:l_}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let E=n.trim();!E||!m.isSuccess||(u(_=>Mh([..._,{channel_id:E,subject_user_id:s}])),r(""))},v=E=>{u(_=>_.filter(T=>T.channel_id!==E))},b=(E,_)=>{u(T=>T.map(L=>L.channel_id===E?{...L,subject_user_id:_}:L))},$=()=>{w.mutate({channels:QD(o)})},S=m.isError&&o.some(E=>!E.subject_user_id);return l`
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
          ${p.map(E=>l`
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
                      onChange=${_=>b(E.channel_id,_.target.value)}
                      className="h-8 rounded-md border border-white/10 bg-white/[0.04] px-2 text-xs text-iron-100 outline-none focus:border-signal/45"
                    >
                      <option value="">${c.autoSubjectLabel}</option>
                      ${u_(f,E).map(_=>l`
                          <option key=${_.subject_user_id} value=${_.subject_user_id}>
                            ${_.display_name}
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
          onClick=${$}
          disabled=${!d.isSuccess||!x||w.isPending||S}
        >
          ${w.isPending?c.savingLabel:c.submitLabel}
        <//>
        ${w.isSuccess&&l`<p className="text-xs text-emerald-300">
          ${c.successMessage}
        </p>`}
        ${(d.isError||m.isError||w.isError)&&l`<p className="text-xs text-red-300">
          ${o_(w.error||d.error||m.error,c.errorMessage)}
        </p>`}
      </div>
    </div>
  `}function u_(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function Mh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return n_(Array.from(t.keys())).map(a=>t.get(a))}function QD(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function VD(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var Oh=["slack-setup"],Kr={installationId:{body:"Local IronClaw name for this Slack install. Choose one and keep it stable.",example:"Example: local-slack"},teamId:{body:"Slack workspace/team ID from the workspace that installed the app.",example:"Example: T0123456789"},appId:{body:"Slack app Basic Information > App Credentials.",example:"Example: A0123456789"},botUser:{body:"Optional Reborn user. Blank uses the current WebUI operator.",example:"Example: user:operator"},sharedSubject:{body:"Optional default team agent for shared channel turns. Usually blank.",example:"Example: user:slack-shared"},botToken:{body:"Slack app OAuth & Permissions > Bot User OAuth Token.",example:"Example: xoxb-..."},signingSecret:{body:"Slack app Basic Information > App Credentials > Signing Secret.",example:""}};function f_({action:e}){let t=I({queryKey:Oh,queryFn:e_}),a=t.data?.configured===!0;return l`
    <div className="space-y-3">
      <${GD} action=${e} setupQuery=${t} />
      ${a&&l`<${c_} action=${e} />`}
    </div>
  `}function GD({action:e,setupQuery:t}){let a=J(),[n,r]=h.default.useState(YD()),s=h.default.useRef(!1),i=h.default.useRef(!1),o=t.data,u=JD(e);h.default.useEffect(()=>{!o||s.current||i.current||(r(d_(o)),s.current=!0)},[o]);let c=Q({mutationFn:t_,onSuccess:p=>{i.current=!1,r(d_(p)),s.current=!0,a.setQueryData(Oh,p),a.invalidateQueries({queryKey:Oh}),a.invalidateQueries({queryKey:["slack-allowed-channels"]}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["extensions"]})}}),d=p=>x=>{i.current=!0,r(y=>({...y,[p]:x.target.value}))},m=()=>c.mutate(n),f=n.installation_id.trim()&&n.team_id.trim()&&n.api_app_id.trim()&&(o?.bot_token_configured||n.bot_token.trim())&&(o?.signing_secret_configured||n.signing_secret.trim());return l`
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
        ${ll("Installation ID",n.installation_id,d("installation_id"),"",Kr.installationId)}
        ${ll("Team ID",n.team_id,d("team_id"),"",Kr.teamId)}
        ${ll("App ID",n.api_app_id,d("api_app_id"),"",Kr.appId)}
        ${ll("Bot user",n.user_id,d("user_id"),"default operator",Kr.botUser)}
        ${ll("Shared subject",n.shared_subject_user_id,d("shared_subject_user_id"),"optional",Kr.sharedSubject)}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        ${m_("Bot token",n.bot_token,d("bot_token"),o?.bot_token_configured,Kr.botToken)}
        ${m_("Signing secret",n.signing_secret,d("signing_secret"),o?.signing_secret_configured,Kr.signingSecret)}
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
          ${Dh(t.error,u.errorMessage)}
        </p>`}
        ${c.isError&&l`<p className="text-xs text-red-300">
          ${Dh(c.error,u.errorMessage)}
        </p>`}
        ${c.isSuccess&&l`<p className="text-xs text-emerald-300">${u.successMessage}</p>`}
      </div>
    </div>
  `}function d_(e){return{installation_id:e.installation_id||"",team_id:e.team_id||"",api_app_id:e.api_app_id||"",user_id:e.user_id||"",shared_subject_user_id:e.shared_subject_user_id||"",bot_token:"",signing_secret:""}}function YD(){return{installation_id:"",team_id:"",api_app_id:"",user_id:"",shared_subject_user_id:"",bot_token:"",signing_secret:""}}function ll(e,t,a,n="",r=null){return l`
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-iron-500">${e}</span>
      <input
        type="text"
        value=${t}
        onChange=${a}
        placeholder=${n}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <${p_} help=${r} />
    </label>
  `}function m_(e,t,a,n,r=null){return l`
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
      <${p_} help=${r} />
    </label>
  `}function p_({help:e}){return e?l`
    <p className="mt-1.5 min-h-8 text-[11px] leading-4 text-iron-400">
      <span className="block">${e.body}</span>
      ${e.example&&l`<span className="mt-0.5 block font-mono text-iron-300">${e.example}</span>`}
    </p>
  `:null}function JD(e){return{title:"Slack setup",instructions:e?.instructions||"Configure the Slack app before assigning channels.",submitLabel:"Save setup",successMessage:"Slack setup saved.",errorMessage:"Slack setup update failed."}}var Lh={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function Hr(e){return e==="wasm_channel"||e==="channel"}var h_={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},v_={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function g_(e){let t=y_(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||Hr(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function y_(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function Ph(e){let t=y_(e);return t==="active"||t==="ready"}function b_({extension:e,secrets:t=[],fields:a=[]}={}){return Ph(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var x_="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",$_="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",w_="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",S_="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",N_="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",XD="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function __(e){return e.package_ref?.id||""}function ZD({actions:e,isBusy:t}){let a=C(),[n,r]=h.default.useState(!1),s=h.default.useRef(null);return h.default.useEffect(()=>{if(!n)return;let i=o=>{s.current&&!s.current.contains(o.target)&&r(!1)};return document.addEventListener("mousedown",i),()=>document.removeEventListener("mousedown",i)},[n]),l`
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
                <${O} name=${i.icon||"settings"} className="h-3.5 w-3.5" />
                ${i.label}
              </button>
            `)}
        </div>
      `}
    </div>
  `}function k_({items:e}){return!e||e.length===0?null:l`
    <div className="mt-3 flex flex-wrap gap-1">
      ${e.map(t=>l`<span key=${t} className=${XD}>${t}</span>`)}
    </div>
  `}function fi({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=C(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=h_[i]||"muted",u=s(`extensions.state.${i}`)||v_[i]||i,c=s(`extensions.kind.${e.kind}`)||Lh[e.kind]||e.kind,d=e.display_name||__(e),m=!!e.package_ref,f=e.tools||[],[p,x]=h.default.useState(!1),w=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],b=[],$=g_(e);$==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):$==="activate"&&v.push({id:"activate",label:"Activate",run:()=>t(g)}),m&&(e.needs_setup||e.has_auth)&&$!=="configure"&&b.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)}),m&&Hr(e.kind)&&(i==="setup_required"||i==="failed")&&b.push({id:"setup",label:"Setup",icon:"settings",run:()=>a(g)}),m&&Hr(e.kind)&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&b.push({id:"reconfigure",label:"Reconfigure",icon:"settings",run:()=>a(g)}),m&&b.push({id:"remove",label:s("common.remove")||"Remove",icon:"trash",danger:!0,run:()=>n(g)});let S=v[0];return l`
    <div className=${x_}>
      <div className="flex items-start gap-2">
        <${z} tone=${o} label=${u} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${b.length>0&&l`<${ZD} actions=${b} isBusy=${r} />`}
      </div>

      <div className=${$_}>
        <span>${c}</span>
        ${e.version&&l`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&l`<p className=${w_}>${e.description}</p>`}

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

      <div className=${S_}>
        ${f.length>0?l`
              <button
                type="button"
                aria-expanded=${p?"true":"false"}
                onClick=${()=>x(E=>!E)}
                className=${N_}
              >
                <${O} name="layers" className="h-3.5 w-3.5" />
                <span>${f.length===1?s("extensions.oneCapability"):s("extensions.pluralCapabilities",{count:f.length})}</span>
                <${O}
                  name="chevron"
                  className=${["h-3 w-3",p?"rotate-180":""].join(" ")}
                />
              </button>
            `:l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">No capabilities</span>`}
        <span className="flex-1"></span>
        ${S&&l`
          <${A} variant="secondary" size="sm" onClick=${S.run} disabled=${r}>
            ${S.label}
          <//>
        `}
      </div>

      ${p&&l`<${k_} items=${f} />`}
    </div>
  `}function Qr({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=C(),s=r(`extensions.kind.${e.kind}`)||Lh[e.kind]||e.kind,i=e.display_name||__(e),o=!!(e.package_ref&&t),u=e.keywords||[],[c,d]=h.default.useState(!1);return l`
    <div className=${x_}>
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

      <div className=${$_}>
        <span>${s}</span>
        ${e.version&&l`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&l`<p className=${w_}>${e.description}</p>`}

      <div className=${S_}>
        ${u.length>0?l`
              <button
                type="button"
                aria-expanded=${c?"true":"false"}
                onClick=${()=>d(m=>!m)}
                className=${N_}
              >
                <${O} name="list" className="h-3.5 w-3.5" />
                <span>${u.length===1?r("extensions.oneKeyword"):r("extensions.pluralKeywords",{count:u.length})}</span>
                <${O}
                  name="chevron"
                  className=${["h-3 w-3",c?"rotate-180":""].join(" ")}
                />
              </button>
            `:l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]"></span>`}
        <span className="flex-1"></span>
        ${o&&l`
          <${A}
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

      ${c&&l`<${k_} items=${u} />`}
    </div>
  `}function R_(){return H("/api/webchat/v2/extensions")}function C_(){return H("/api/webchat/v2/extensions/registry")}function E_(e){return H("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function T_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(ul(e))}/activate`,{method:"POST"})}function A_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(ul(e))}/remove`,{method:"POST"})}function D_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(ul(e))}/setup`)}function M_(e,t,a){return Vx(ul(e),{action:"submit",payload:{secrets:t,fields:a}})}function O_(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return H(`/api/webchat/v2/extensions/${encodeURIComponent(ul(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function L_(){return Promise.resolve({requests:[]})}function P_(){return Promise.resolve({success:!1,message:"Pairing requires a v2 pairing endpoint."})}function ul(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var WD=2e3,eM=10*60*1e3;function pi(e){return e?.package_ref?.id||null}function Uh(e){return e?.display_name||pi(e)||""}function U_(e,t,a){return pi(t)||`${e}:${Uh(t)||"unknown"}:${a}`}function tM(e,t){return e.installed!==t.installed?e.installed?-1:1:Uh(e.entry||e.extension).localeCompare(Uh(t.entry||t.extension))}function j_(){let e=J(),t=I({queryKey:["gateway-status-extensions"],queryFn:Ys,staleTime:1e4}),a=I({queryKey:["extensions"],queryFn:R_}),n=I({queryKey:["extension-registry"],queryFn:C_}),r=I({queryKey:["connectable-channels"],queryFn:Pc}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["extensions"]}),e.invalidateQueries({queryKey:["extension-registry"]}),e.invalidateQueries({queryKey:["gateway-status-extensions"]}),e.invalidateQueries({queryKey:["connectable-channels"]})},[e]),[i,o]=h.default.useState(null),u=h.default.useCallback(()=>o(null),[]),c=Q({mutationFn:({packageRef:R})=>E_(R),onSuccess:(R,{displayName:F})=>{R.success?(o({type:"success",message:R.message||R.instructions||`${F||"Extension"} installed`}),R.auth_url&&window.open(R.auth_url,"_blank","noopener,noreferrer")):o({type:"error",message:R.message||"Install failed"}),s()},onError:R=>{o({type:"error",message:R.message}),s()}}),d=Q({mutationFn:({packageRef:R})=>T_(R),onSuccess:(R,{displayName:F})=>{R.success?(o({type:"success",message:R.message||R.instructions||`${F||"Extension"} activated`}),R.auth_url&&window.open(R.auth_url,"_blank","noopener,noreferrer")):R.auth_url?(window.open(R.auth_url,"_blank","noopener,noreferrer"),o({type:"info",message:"Opening authentication\u2026"})):R.awaiting_token?o({type:"info",message:"Configuration required"}):o({type:"error",message:R.message||"Activation failed"}),s()},onError:R=>{o({type:"error",message:R.message})}}),m=Q({mutationFn:({packageRef:R})=>A_(R),onSuccess:(R,{displayName:F})=>{R.success?o({type:"success",message:`${F||"Extension"} removed`}):o({type:"error",message:R.message||"Remove failed"}),s()},onError:R=>{o({type:"error",message:R.message})}}),f=t.data||{},p=a.data?.extensions||[],x=n.data?.entries||[],y=r.data?.channels||[],w=new Map(p.map(R=>[pi(R),R]).filter(([R])=>!!R)),g=new Set(x.map(R=>pi(R)).filter(Boolean)),v=[...x.map((R,F)=>{let G=pi(R),se=G&&w.get(G)||null;return{id:U_("registry",R,F),installed:!!(se||R.installed),entry:R,extension:se}}),...p.filter(R=>{let F=pi(R);return!F||!g.has(F)}).map((R,F)=>({id:U_("installed",R,F),installed:!0,entry:null,extension:R}))].sort(tM),b=R=>Hr(R.kind),$=p.filter(b),S=p.filter(R=>R.kind==="mcp_server"),E=p.filter(R=>!b(R)&&R.kind!=="mcp_server"),_=x.filter(R=>b(R)&&!R.installed),T=x.filter(R=>R.kind==="mcp_server"&&!R.installed),L=x.filter(R=>R.kind!=="mcp_server"&&!b(R)&&!R.installed),D=a.isLoading||n.isLoading,P=c.isPending||d.isPending||m.isPending;return{status:f,extensions:p,channels:$,mcpServers:S,tools:E,channelRegistry:_,mcpRegistry:T,toolRegistry:L,registry:x,catalogEntries:v,connectableChannels:y,isLoading:D,isBusy:P,actionResult:i,clearResult:u,install:c.mutate,activate:d.mutate,remove:m.mutate,invalidate:s}}function F_(e){let t=I({queryKey:["extension-setup",e?.id||e],queryFn:()=>D_(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function B_(e,t){let a=J(),n=e?.id||e;return Q({mutationFn:({secrets:r,fields:s})=>M_(e,r,s),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function z_(e){let t=J(),a=e?.id||e,n=h.default.useRef(null),r=h.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=h.default.useCallback(()=>{let u=t.getQueryData(["extension-setup",a]);if(u?.secrets?.length>0&&u.secrets.every(f=>f.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(f=>f.package_ref?.id===a),m=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return m==="active"||m==="ready"},[a,t]),o=h.default.useCallback(u=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||u&&u.closed||Date.now()-c>eM)&&(r(),s())},WD)},[r,s,i]);return h.default.useEffect(()=>r,[r]),Q({mutationFn:({secret:u,popup:c})=>O_(e,u).then(d=>({res:d,popup:c})),onSuccess:({res:u,popup:c})=>{let d=c;u.authorization_url&&c&&!c.closed?c.location.href=u.authorization_url:u.authorization_url?d=window.open(u.authorization_url,"_blank","noopener,noreferrer"):c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(u,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function q_(e,t={}){let a=I({queryKey:["pairing",e],queryFn:()=>L_(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=J(),r=Q({mutationFn:({code:s})=>P_(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function I_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var aM={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function K_({channel:e,redeemFn:t,i18nKeys:a=aM,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=C(),o=typeof t=="function",u=q_(e,{enabled:!o}),c=J(),[d,m]=h.default.useState(""),f=nM(i,a,r),p=Q({mutationFn:({code:S})=>t(e,S),onSuccess:()=>{m("");for(let S of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:S})}}),x=h.default.useCallback(S=>u.approve({code:S}),[u.approve]),y=h.default.useCallback(()=>{let S=d.trim();S&&(o?p.mutate({code:S}):(u.approve({code:S}),m("")))},[o,d,u.approve,p]),w=o?[]:u.requests,g=o?!1:u.isLoading,v=o?p.isPending:u.isApproving,b=o?p.isSuccess?p.data:null:u.result,$=o?p.isError?p.error:null:u.error;return g?l`
      <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
        <div className="v2-skeleton h-3 w-24 rounded" />
      </div>
    `:l`
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

      ${b?.success&&l`<p className="mb-3 text-xs text-emerald-300">
        ${b.message||f.success}
      </p>`}
      ${b&&!b.success&&l`<p className="mb-3 text-xs text-red-300">
        ${b.message||f.error}
      </p>`}
      ${$&&l`<p className="mb-3 text-xs text-red-300">
        ${I_($,f.error)}
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
  `}function nM(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function ud(e){return e.package_ref?.id||""}function H_(e){return ud(e)==="slack"}function V_(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function G_(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function rM(e){let t=e||[],a=[t.find(V_),t.find(G_)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function Q_({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>V_(r)?l`<${f_} action=${r.action} />`:G_(r)?l`<${Tc} action=${r.action} />`:null).filter(Boolean);return n.length>0?l`<div className="space-y-3">${n}</div>`:null}function Y_({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:u}){let c=C(),d=t||[],m=e.enabled_channels||[],f=rM(a),p=d.some(H_),x=f.length>0&&!p;return l`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          ${c("channels.builtIn")}
        </h3>
        <${hi}
          name="Web Gateway"
          description=${c("channels.webGatewayDesc")||"Browser-based chat with SSE streaming"}
          enabled=${!0}
          detail=${"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)}
        />
        <${hi}
          name="HTTP Webhook"
          description=${c("channels.httpWebhookDesc")||"Inbound webhook endpoint for external integrations"}
          enabled=${m.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${hi}
          name="CLI"
          description=${c("channels.cliDesc")||"Terminal interface with TUI or simple REPL"}
          enabled=${m.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${hi}
          name="REPL"
          description=${c("channels.replDesc")||"Minimal read-eval-print loop for testing"}
          enabled=${m.includes("repl")}
          detail="ironclaw run --repl"
        />
        ${x&&l`
          <${hi}
            name=${c("channels.slack")||"Slack"}
            description=${c("channels.slackDesc")||"Tenant app channel for DMs and app mentions"}
            enabled=${!1}
            statusLabel="setup"
            statusTone="muted"
            detail=${c("channels.slackDetail")||"Tenant Slack app install"}
          >
            <${Q_}
              slackConnectActions=${f}
            />
          </${hi}>
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
                <div key=${ud(y)} className="flex flex-col gap-3">
                  <${fi}
                    ext=${y}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${u}
                  />
                  ${H_(y)&&l`<${Q_}
                    slackConnectActions=${f}
                  />`}
                  ${(y.onboarding_state==="pairing_required"||y.onboarding_state==="pairing")&&l` <${K_} channel=${ud(y)} /> `}
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
                <${Qr}
                  key=${ud(y)}
                  entry=${y}
                  onInstall=${o}
                  isBusy=${u}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function hi({name:e,description:t,enabled:a,detail:n,children:r,statusLabel:s=a?"on":"off",statusTone:i=a?"success":"muted"}){return l`
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
          ${n&&l`<div className="mt-1 font-mono text-[11px] text-iron-700">
            ${n}
          </div>`}
        </div>
      </div>
      ${r}
    </div>
  `}function J_({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=C(),s=e?.displayName||e?.packageRef?.id||"Extension",{secrets:i=[],fields:o=[],onboarding:u,isLoading:c,error:d}=F_(e?.packageRef),[m,f]=h.default.useState({}),[p,x]=h.default.useState({}),y=z_(e?.packageRef),w=B_(e?.packageRef,_=>{_.success!==!1&&(n&&n(_),a())}),g=h.default.useCallback(()=>{let _={};for(let[T,L]of Object.entries(m)){let D=(L||"").trim();D&&(_[T]=D)}w.mutate({secrets:_,fields:p})},[m,p,w]),v=h.default.useCallback(_=>{let T=window.open("about:blank","_blank","width=600,height=600");T&&(T.opener=null),y.mutate({secret:_,popup:T})},[y]),$=i.filter(_=>(_.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,S=Ph(e),E=b_({extension:e,secrets:i,fields:o});return c?l`
      <${cd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(_=>l`<div
                key=${_}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?l`
      <${cd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")||"Failed to load setup:"} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?l`
      <${cd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")||"No configuration required for this extension."}
        </p>
      <//>
    `:l`
    <${cd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
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
          <${O} name="bolt" className="h-3.5 w-3.5" />
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
                      <${A}
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
                value=${m[_.name]||""}
                onChange=${T=>f(L=>({...L,[_.name]:T.target.value}))}
                onKeyDown=${T=>T.key==="Enter"&&g()}
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
                onChange=${T=>x(L=>({...L,[_.name]:T.target.value}))}
                onKeyDown=${T=>T.key==="Enter"&&g()}
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
        ${$&&l`
        <${A}
          variant=${E?"secondary":"primary"}
          onClick=${g}
          disabled=${w.isPending}
        >
          ${w.isPending?"Saving\u2026":r("common.save")||"Save"}
        <//>
        `}
      </div>
    <//>
  `}function cd({onClose:e,title:t,children:a}){return h.default.useEffect(()=>{let n=r=>{r.key==="Escape"&&e()};return window.addEventListener("keydown",n),()=>window.removeEventListener("keydown",n)},[e]),l`
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
  `}function X_(e){return e.package_ref?.id||""}function Z_({mcpServers:e,mcpRegistry:t,onActivate:a,onConfigure:n,onRemove:r,onInstall:s,isBusy:i}){let o=C();return e.length===0&&t.length===0?l`
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
                <${fi}
                  key=${X_(u)}
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
                <${Qr}
                  key=${X_(u)}
                  entry=${u}
                  onInstall=${s}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function sM(e){return e?.package_ref?.id||""}function iM(e){return e.entry||e.extension||{}}function W_({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=C(),[o,u]=h.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(y=>{let w=iM(y);return(w.display_name||sM(w)).toLowerCase().includes(c)||(w.description||"").toLowerCase().includes(c)||(w.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,m=d.filter(y=>y.installed&&y.extension),f=d.filter(y=>y.installed&&!y.extension&&y.entry),p=m.length+f.length,x=d.filter(y=>!y.installed&&y.entry);return e.length===0?l`
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
                  ${m.map(y=>l`
                      <${fi}
                        key=${y.id}
                        ext=${y.extension||y.entry}
                        onActivate=${a}
                        onConfigure=${n}
                        onRemove=${r}
                        isBusy=${s}
                      />
                    `)}
                  ${f.map(y=>l`
                      <${Qr}
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
                  className=${["mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal",p>0?"mt-6":""].join(" ")}
                >
                  ${i("ext.registry.availableTitle")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${x.map(y=>l`
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
  `}function jh(){let{tab:e="registry"}=it(),[t,a]=h.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:u,connectableChannels:c,isLoading:d,isBusy:m,actionResult:f,clearResult:p,install:x,activate:y,remove:w,invalidate:g}=j_(),v=h.default.useCallback(_=>a(_),[]),b=h.default.useCallback(()=>a(null),[]),$=h.default.useCallback(()=>g(),[g]),S=h.default.useCallback(_=>{_&&(y(_),a(null))},[y]);if(d)return l`
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
    `;if(e==="installed")return l`<${ot} to="/extensions/registry" replace />`;let E={channels:l`<${Y_}
      status=${n}
      channels=${r}
      connectableChannels=${c}
      channelRegistry=${i}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${w}
      onInstall=${x}
      isBusy=${m}
    />`,mcp:l`<${Z_}
      mcpServers=${s}
      mcpRegistry=${o}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${w}
      onInstall=${x}
      isBusy=${m}
    />`,registry:l`<${W_}
      catalogEntries=${u}
      onInstall=${x}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${w}
      isBusy=${m}
    />`};return E[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <${XN} result=${f} onDismiss=${p} />
          ${E[e]}
        </div>
      </div>

      ${t&&l`
        <${J_}
          extension=${t}
          onActivate=${S}
          onClose=${b}
          onSaved=${$}
        />
      `}
    </div>
  `:l`<${ot} to="/extensions/registry" replace />`}var ek=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],tk=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],ak=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],Fh=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function nk(e){return String(e||"").trim().toLowerCase()}function rk(e){if(e==null)return"";if(Array.isArray(e))return e.map(rk).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function at(e,t){let a=nk(e);return a?t.map(rk).join(" ").toLowerCase().includes(a):!0}function vi(e,t,a,n){let r=nk(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(u=>at(r,[i,u.key,u.labelKey?n(u.labelKey):u.label,u.descKey?n(u.descKey):u.description,t[u.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function oM({visible:e}){let t=C();return e?l`
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      ${t("tools.saved")}
    </span>
  `:null}function lM({checked:e,onChange:t,label:a}){return l`
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
  `}function uM({field:e,value:t,onSave:a,isSaved:n}){let r=C(),[s,i]=h.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",u=e.descKey?r(e.descKey):e.description||"";h.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=h.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let m=parseInt(d,10);isNaN(m)||a(e.key,m)}else if(e.type==="float"){let m=parseFloat(d);isNaN(m)||a(e.key,m)}else a(e.key,d)},[e.key,e.type,a]);return l`
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">${o}</div>
        ${u&&l`<div className="mt-1 text-xs leading-5 text-iron-300">${u}</div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${e.type==="boolean"?l`
              <${lM}
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
        <${oM} visible=${n} />
      </div>
    </div>
  `}function gi({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=C(),o=t?i(t):e||"";return l`
    <${ee} className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${o}</h3>
      <div>
        ${a.map(u=>l`
              <${uM}
                key=${u.key}
                field=${u}
                value=${n[u.key]}
                onSave=${r}
                isSaved=${s[u.key]}
              />
            `)}
      </div>
    <//>
  `}function _t({query:e}){let t=C();return l`
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
  `}function sk({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=C();if(n)return l`<${cM} />`;let i=vi(tk,e,r,s);return i.length===0?l`<${_t} query=${r} />`:l`
    <div className="space-y-5">
      ${i.map(o=>l`
            <${gi}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function cM(){return l`
    <div className="space-y-5">
      ${[1,2,3].map(e=>l`
            <${ee} key=${e} padding="md">
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
  `}function ik(){let e=I({queryKey:["gateway-status-settings"],queryFn:Ys,staleTime:1e4}),t=I({queryKey:["extensions"],queryFn:z$}),a=I({queryKey:["extension-registry"],queryFn:q$}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(m=>m.kind==="wasm_channel"||m.kind==="channel"),o=s.filter(m=>(m.kind==="wasm_channel"||m.kind==="channel")&&!m.installed),u=r.filter(m=>m.kind==="mcp_server"),c=s.filter(m=>m.kind==="mcp_server"&&!m.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:u,mcpRegistry:c,extensions:r,isLoading:d}}function dM({name:e,description:t,enabled:a,detail:n}){let r=C();return l`
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
        ${n&&l`<div className="mt-1 font-mono text-[11px] text-[var(--v2-text-faint)]">
          ${n}
        </div>`}
      </div>
    </div>
  `}function ok({channel:e,registryEntry:t}){let a=C(),n=t?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},u={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return l`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${n}</span>
          ${s?l`<${z}
                tone=${o[i]||"muted"}
                label=${u[i]||i}
                size="sm"
              />`:l`<${z}
                tone="muted"
                label=${a("channels.available")}
                size="sm"
              />`}
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${r}</div>
      </div>
    </div>
  `}function mM(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function fM({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=mM(e,i).filter(x=>at(s,[i("channels.builtIn"),x.id,x.name,x.description,x.detail])),u=new Set(t.map(x=>x.name)),c=t.filter(x=>at(s,[i("channels.messaging"),x.name,x.display_name,x.description,x.onboarding_state])),d=a.filter(x=>!u.has(x.name)).filter(x=>at(s,[i("channels.messaging"),x.name,x.display_name,x.description])),m=new Set(n.map(x=>x.name)),f=n.filter(x=>at(s,[i("channels.mcpServers"),x.name,x.display_name,x.description,x.active?i("channels.active"):i("channels.inactive")])),p=r.filter(x=>!m.has(x.name)).filter(x=>at(s,[i("channels.mcpServers"),x.name,x.display_name,x.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:p}}function lk({searchQuery:e=""}){let t=C(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=ik();if(o)return l`
      <div className="space-y-5">
        <${ee} padding="md">
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
    `;let{builtInChannels:u,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:f}=fM({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return u.length===0&&c.length===0&&d.length===0&&m.length===0&&f.length===0?l`<${_t} query=${e} />`:l`
    <div className="space-y-5">
      ${u.length>0&&l`
      <${ee} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${u.map(p=>l`
            <${dM}
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
        <${ee} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.messaging")}
          </h3>
          ${c.map(p=>l`
              <${ok}
                key=${p.name}
                channel=${p}
                registryEntry=${r.find(x=>x.name===p.name)}
              />
            `)}
          ${d.map(p=>l`
              <${ok} key=${p.name} registryEntry=${p} />
            `)}
        <//>
      `}
      ${(m.length>0||f.length>0)&&l`
        <${ee} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.mcpServers")}
          </h3>
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
                      <${z}
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
                      <${z}
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
  `}function uk({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:u,onNearaiWallet:c,onCodexLogin:d,loginBusy:m}){let f=C(),p=e.id===t,x=Br(e,n),y=Zs(e,n),w=tw(e,n,t,a),g=yc(e,n),v=aw(e),b=f(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[$,S]=h.default.useState(p),E=h.default.useCallback(()=>S(Je=>!Je),[]);h.default.useEffect(()=>{S(p)},[p]);let _=x?l`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${Jo(e.adapter)} · ${w||e.default_model||f("llm.none")}
      </span>`:l`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${b}
      </span>`,T=e.id==="nearai"||e.id==="openai_codex",L=e.api_key_set===!0||e.has_api_key===!0,D=e.builtin?e.id==="nearai"&&v&&!L?f("llm.addApiKey"):f("llm.configure"):f("common.edit"),P=v&&e.builtin?l`
          <${A}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${r}
            onClick=${()=>i(e)}
          >
            ${D}
          <//>
        `:null,R=!p&&e.id==="nearai"?l`
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
        `:!p&&e.id==="openai_codex"?l`
          <${A} type="button" variant="secondary" size="sm" disabled=${m} onClick=${d}>
            ${f("onboarding.codexSignIn")}
          <//>
        `:null,G=!p&&x&&(!T||e.id==="nearai"&&e.has_api_key===!0)?l`
        <${A}
          type="button"
          variant="primary"
          size="sm"
          disabled=${r}
          onClick=${()=>s(e)}
        >
          ${f("llm.use")}
        <//>
      `:null,se=x?null:l`
        <${A}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${r}
          onClick=${()=>i(e)}
        >
          ${f(g==="api_key"?"llm.addApiKey":"llm.configure")}
        <//>
      `,ce=p?null:G||(T?R:se),oe=!T&&(e.builtin&&e.id!=="bedrock"||!e.builtin)||e.id==="nearai"&&v;return l`
    <${ee}
      padding="none"
      data-testid="llm-provider-card"
      data-provider-id=${e.id}
      className=${["transition-colors",p?"border-[color-mix(in_srgb,var(--v2-positive-text)_36%,var(--v2-panel-border))]":$?"border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))]":""].join(" ")}
    >
      <div className="flex w-full items-stretch hover:bg-[var(--v2-surface-soft)]">
        <button
          type="button"
          aria-expanded=${$?"true":"false"}
          aria-label=${f($?"llm.collapseDetails":"llm.expandDetails")}
          data-testid="llm-provider-disclosure"
          onClick=${E}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 px-4 py-3 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)] sm:pl-5 sm:pr-3"
        >
          <span
            className=${["h-2 w-2 shrink-0 rounded-full",p?"bg-[var(--v2-positive-text)]":x?"bg-[var(--v2-accent)]":"bg-[var(--v2-warning-text)]"].join(" ")}
          />
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
            <span className="min-w-0 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
              ${e.name||e.id}
            </span>
            <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">${e.id}</span>
            ${p&&l`<${z} tone="positive" label=${f("llm.active")} size="sm" />`}
            ${e.builtin&&!p&&l`<${z} tone="muted" label=${f("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${_}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 py-3 pr-4 sm:pr-5">
          ${ce}
          <button
            type="button"
            onClick=${E}
            data-testid="llm-provider-chevron"
            aria-label=${f($?"llm.collapseDetails":"llm.expandDetails")}
            className=${["grid h-7 w-7 place-items-center rounded-md text-[var(--v2-text-faint)] transition-transform hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]",$?"rotate-180":""].join(" ")}
          >
            <${O} name="chevron" className="h-4 w-4" />
          </button>
        </div>
      </div>

      ${$&&l`
        <div data-testid="llm-provider-details" className="border-t border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-4 sm:px-5">
          <div className="grid gap-3 text-xs text-[var(--v2-text-muted)] sm:grid-cols-3">
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${f("llm.adapter")}</div>
              <div className="mt-1 truncate">${Jo(e.adapter)}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${f("llm.baseUrl")}</div>
              <div className="mt-1 truncate font-mono">${y||f("llm.none")}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${f("llm.model")}</div>
              <div className="mt-1 truncate font-mono">${w||f("llm.none")}</div>
            </div>
          </div>

          <div className="mt-4 flex flex-wrap justify-end gap-2 border-t border-[var(--v2-panel-border)] pt-3">
            ${oe&&l`
              <${A}
                type="button"
                variant="secondary"
                size="sm"
                disabled=${r}
                onClick=${()=>i(e)}
              >
                ${D}
              <//>
            `}
            ${!e.builtin&&!p&&l`
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
  `}var pM=[{key:"active",labelKey:"llm.groupActive",dotClass:"bg-[var(--v2-positive-text)]"},{key:"ready",labelKey:"llm.groupReady",dotClass:"bg-[var(--v2-accent)]"},{key:"setup",labelKey:"llm.groupSetup",dotClass:"bg-[var(--v2-warning-text)]"}];function hM({label:e,count:t,dotClass:a}){return l`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full "+a} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${t}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `}function ck({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=C(),r=Kc({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=Hc(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return l`<${_t} query=${a} />`;let u=nw(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return l`
    <${ee} className="p-4 sm:p-6">
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${n("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">${n("llm.providersDesc")}</p>
        </div>
        <${A} type="button" variant="secondary" size="sm" className="gap-2" onClick=${()=>r.openDialog(null)}>
          <${O} name="plus" className="h-3.5 w-3.5" />
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

      <${Ic} login=${i} />

      ${s.isLoading?l`<div className="text-sm text-[var(--v2-text-muted)]">${n("common.loading")}</div>`:s.error?l`<div className="text-sm text-red-200">${n("error.loadFailed",{what:n("llm.providers"),message:s.error.message})}</div>`:l`
            <div className="space-y-1">
              ${pM.flatMap(c=>{let d=u[c.key];return d.length?[l`
                    <section
                      key=${c.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${c.key}
                      className="mb-3"
                    >
                      <${hM}
                        label=${n(c.labelKey)}
                        count=${d.length}
                        dotClass=${c.dotClass}
                      />
                      <div className="space-y-2">
                      ${d.map(m=>l`
                          <${uk}
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

      <${qc}
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
  `}function dk({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=C(),{activeProviderId:o,selectedModel:u,providers:c,hasActiveProvider:d}=Ws({settings:e,gatewayStatus:t});if(r)return l`<${vM} />`;let m=d?o:"",f=c.find(g=>g.id===o),p=d&&(u||f?.default_model||e.selected_model)||"",x=vi(ek,e,s,i),y=at(s,[i("inference.provider"),i("inference.backend"),m,i("inference.model"),p]),w=at(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!y&&!w&&x.length===0?l`<${_t} query=${s} />`:l`
    <div className="space-y-5">
      ${y&&l`
      <${ee} padding="none" className="p-4 sm:p-5">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${i("inference.provider")}</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${i("inference.backend")}</div>
            <div className="mt-1 flex items-center gap-2">
              <span className="font-mono text-lg font-semibold text-[var(--v2-text-strong)]">${m||i("inference.none")}</span>
              ${d?l`<${z} tone="positive" label=${i("inference.active")} size="sm" />`:l`<${z} tone="muted" label=${i("llm.notConfigured")} size="sm" />`}
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
        <${ck}
          settings=${e}
          gatewayStatus=${t}
          searchQuery=${s}
        />
      `}

      ${x.map(g=>l`
            <${gi}
              key=${g.groupKey}
              groupKey=${g.groupKey}
              fields=${g.fields}
              settings=${e}
              onSave=${a}
              savedKeys=${n}
            />
          `)}
    </div>
  `}function cr({className:e=""}){return l`
    <div
      className=${"rounded animate-pulse bg-[var(--v2-surface-muted)] "+e}
    />
  `}function vM(){return l`
    <div className="space-y-5">
      <${ee} padding="md">
        <${cr} className="mb-4 h-3 w-24" />
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${cr} className="h-3 w-16" />
            <${cr} className="mt-2 h-6 w-28" />
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${cr} className="h-3 w-16" />
            <${cr} className="mt-2 h-6 w-40" />
          </div>
        </div>
      <//>
      ${[1,2].map(e=>l`
            <${ee} key=${e} padding="md">
              <${cr} className="mb-4 h-3 w-20" />
              ${[1,2,3].map(t=>l`
                    <div key=${t} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                      <${cr} className="h-4 w-32" />
                      <${cr} className="h-9 w-36" />
                    </div>
                  `)}
            <//>
          `)}
    </div>
  `}function mk({searchQuery:e=""}){let t=C(),{lang:a,setLang:n}=cl(),r=dl.find(i=>i.code===a)||dl[0],s=dl.filter(i=>at(e,[i.code,i.name,i.native]));return s.length===0?l`<${_t} query=${e} />`:l`
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
  `}function fk({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=C();if(n)return l`
      <div className="space-y-5">
        ${[1,2].map(o=>l`
              <${ee} key=${o} padding="md">
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
    `;let i=vi(ak,e,r,s);return i.length===0?l`<${_t} query=${r} />`:l`
    <div className="space-y-5">
      ${i.map(o=>l`
            <${gi}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function pk(){let e=C(),[t,a]=h.default.useState(!1),n=h.default.useCallback(()=>a(!0),[]),r=h.default.useCallback(()=>a(!1),[]),s=h.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function hk({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=C(),r=pk({gatewayStatus:t,gatewayStatusQuery:a});return e?l`
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
          <${O} name=${r.isRestarting?"pulse":"bolt"} className="h-4 w-4" />
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

    <${si}
      open=${r.confirmOpen}
      onClose=${r.closeConfirm}
      title=${n("restart.title")}
      size="sm"
    >
      <${ii} className="space-y-3">
        <p className="text-sm text-[var(--v2-text)]">
          ${n("restart.description")}
        </p>
        <div className="rounded-xl border border-copper/25 bg-copper/10 px-3 py-2 text-xs text-copper">
          ${n("restart.warning")}
        </div>
      <//>
      <${oi}>
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
          <${O} name="bolt" className="h-4 w-4" />
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
  `:null}function vk(){let e=J(),t=I({queryKey:["skills"],queryFn:I$}),a=Q({mutationFn:H$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),n=Q({mutationFn:V$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),r=Q({mutationFn:({name:c,content:d})=>Q$(c,{content:d}),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),s=Q({mutationFn:({name:c,enabled:d})=>G$(c,d),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),i=Q({mutationFn:c=>Y$(c),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),o=t.data?.skills||[],u=t.data?.auto_activate_learned!==!1;return{skills:o,query:t,autoActivateLearned:u,fetchSkillContent:K$,installSkill:a.mutateAsync,removeSkill:n.mutateAsync,updateSkill:r.mutateAsync,setSkillAutoActivate:s.mutateAsync,setAutoActivateLearned:i.mutateAsync,isInstalling:a.isPending,isRemoving:n.isPending,isUpdating:r.isPending,isSettingAutoActivate:s.isPending,isSettingAutoActivateLearned:i.isPending}}function gk({skill:e,onEdit:t,onRemove:a,onUpdate:n,onSetAutoActivate:r,isRemoving:s,isUpdating:i,isSettingAutoActivate:o}){let u=C(),c=e.name||e.id,d=e.trust||e.trust_level||"installed",m=e.source_kind||"installed",f=!!e.can_edit,p=!!e.can_delete,x=e.auto_activate!==!1,[y,w]=h.default.useState(!1),[g,v]=h.default.useState(""),[b,$]=h.default.useState(""),[S,E]=h.default.useState(!1);h.default.useEffect(()=>{y||(v(""),$(""))},[y]);let _=h.default.useCallback(async()=>{E(!0),$("");try{let L=await t(c);v(L?.content||""),w(!0)}catch(L){$(L.message||u("skills.contentLoadFailed"))}finally{E(!1)}},[c,t,u]),T=h.default.useCallback(async()=>{(await n(c,g))?.success&&w(!1)},[g,c,n]);return l`
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
              label=${u(`skills.source.${m}`)}
              size="sm"
            />
            ${e.version&&l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v${e.version}</span>`}
          </div>

          ${e.description&&l`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${e.description}</div>`}

          ${y?l`
                <div className="mt-3">
                  <${Cc}
                    rows=${12}
                    value=${g}
                    className="font-mono text-xs leading-5"
                    onInput=${L=>v(L.currentTarget.value)}
                  />
                </div>
              `:l`<${gM} skill=${e} />`}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          ${f&&!y&&l`
            <${A}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${i||S}
              title=${u("skills.edit")}
              onClick=${_}
            >
              <${O} name="file" className="h-4 w-4" />
              ${u(S?"skills.loading":"skills.edit")}
            <//>
          `}
          ${y&&l`
            <${A}
              type="button"
              variant="ghost"
              size="sm"
              disabled=${i}
              onClick=${()=>{v(""),w(!1)}}
            >
              <${O} name="close" className="h-4 w-4" />
              ${u("skills.cancel")}
            <//>
            <${A}
              type="button"
              variant="primary"
              size="sm"
              disabled=${i}
              onClick=${T}
            >
              <${O} name="check" className="h-4 w-4" />
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
              <${O} name=${x?"check":"close"} className="h-4 w-4" />
              ${u(x?"skills.autoActivateOnLabel":"skills.autoActivateOffLabel")}
            <//>
          `}
          ${p&&!y&&l`
            <${A}
              type="button"
              variant="danger"
              size="sm"
              disabled=${s}
              title=${u("skills.delete")}
              onClick=${()=>a(c)}
            >
              <${O} name="trash" className="h-4 w-4" />
              ${u("skills.delete")}
            <//>
          `}
        </div>
      </div>
      ${b&&l`<p className="mt-2 text-xs text-[var(--v2-danger-text)]">${b}</p>`}
    </div>
  `}function gM({skill:e}){let t=C();return l`
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
        ${e.has_requirements&&l`<${Bh}>requirements.txt<//>`}
        ${e.has_scripts&&l`<${Bh}>scripts/<//>`}
        ${e.install_source_url&&l`<${Bh}>${t("skills.imported")}<//>`}
      </div>
    `}
  `}function Bh({children:e}){return l`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${e}
    </span>
  `}function yk({onInstall:e,isInstalling:t}){let a=C(),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState({name:"",content:""}),[c,d]=h.default.useState(""),[m,f]=h.default.useState(""),p=h.default.useCallback((y,w)=>{u(g=>!g[y]||!w.trim()?g:{...g,[y]:""})},[]),x=h.default.useCallback(async()=>{let y=yM({name:n,content:s}),w=bM(y,a);if(w.name||w.content){u(w),d(""),f("");return}u({name:"",content:""}),d(""),f("");try{let g=await e(y);if(!g?.success){d(g?.message||a("skills.installFailed"));return}r(""),i(""),f(g.message||a("skills.installedSuccess",{name:y.name}))}catch(g){d(g.message||a("skills.installFailed"))}},[s,n,e,a]);return l`
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

      <${Rn} label=${a("skills.name")} error=${o.name} required>
        <${At}
          size="sm"
          error=${!!o.name}
          aria-invalid=${o.name?"true":void 0}
          value=${n}
          placeholder=${a("skills.namePlaceholder")}
          onInput=${y=>{let w=y.currentTarget.value;r(w),p("name",w)}}
        />
      <//>

      <${Rn}
        className="mt-3"
        label=${a("skills.content")}
        error=${o.content}
        hint=${a("skills.contentHint")}
        required
      >
        <${Cc}
          rows=${5}
          error=${!!o.content}
          aria-invalid=${o.content?"true":void 0}
          value=${s}
          placeholder=${a("skills.contentPlaceholder")}
          onInput=${y=>{let w=y.currentTarget.value;i(w),p("content",w)}}
        />
      <//>

      ${c&&l`<p className="mt-3 text-sm text-[var(--v2-danger-text)]">${c}</p>`}
      ${m&&l`<p className="mt-3 text-sm text-[var(--v2-positive-text)]">${m}</p>`}

      <div className="mt-4 flex justify-end">
        <${A} type="button" size="sm" disabled=${t} onClick=${x}>
          <${O} name="upload" className="h-4 w-4" />
          ${a(t?"skills.installing":"skills.install")}
        <//>
      </div>
    <//>
  `}function yM({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function bM(e,t){return{name:e.name?"":t("skills.nameRequired"),content:e.content?"":t("skills.contentRequired")}}function bk({searchQuery:e=""}){let t=C(),{skills:a,query:n,autoActivateLearned:r,fetchSkillContent:s,installSkill:i,removeSkill:o,updateSkill:u,setSkillAutoActivate:c,setAutoActivateLearned:d,isInstalling:m,isRemoving:f,isUpdating:p,isSettingAutoActivate:x,isSettingAutoActivateLearned:y}=vk(),[w,g]=h.default.useState(""),[v,b]=h.default.useState(""),$=h.default.useCallback(async L=>{if(window.confirm(t("skills.confirmDelete",{name:L}))){g(""),b("");try{let D=await o(L);if(!D?.success){g(D?.message||t("skills.removeFailed"));return}b(D.message||t("skills.removed",{name:L}))}catch(D){g(D.message||t("skills.removeFailed"))}}},[o,t]),S=h.default.useCallback(async(L,D)=>{if(!D.trim())return g(t("skills.contentRequired")),b(""),{success:!1,message:t("skills.contentRequired")};g(""),b("");try{let P=await u({name:L,content:D});return P?.success?(b(P.message||t("skills.updated",{name:L})),P):(g(P?.message||t("skills.updateFailed")),P)}catch(P){let R=P.message||t("skills.updateFailed");return g(R),{success:!1,message:R}}},[t,u]),E=h.default.useCallback(async(L,D)=>{g(""),b("");try{let P=await c({name:L,enabled:D});if(!P?.success){g(P?.message||t("skills.updateFailed"));return}b(P.message)}catch(P){g(P.message||t("skills.updateFailed"))}},[c,t]),_=h.default.useCallback(async L=>{g(""),b("");try{let D=await d(L);if(!D?.success){g(D?.message||t("skills.updateFailed"));return}b(D.message)}catch(D){g(D.message||t("skills.updateFailed"))}},[d,t]),T;if(n.isLoading)T=l`
      <${ee} padding="md">
          <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(L=>l`
            <div key=${L} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
              <div>
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              </div>
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
        <//>
    `;else if(n.error)T=l`
      <${ee} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">${t("skills.failedLoad",{message:n.error.message})}</p>
        <//>
    `;else{let L=a.filter(P=>at(e,[P.name,P.id,P.description,P.keywords,P.trust_level,P.source_kind,P.version])),D=wM(L);a.length===0?T=l`
        <${ee} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:L.length===0?T=l`<${_t} query=${e} />`:T=l`
        <div id="skills-list">
          ${D.map(P=>l`
              <${$M}
                key=${P.id}
                title=${t(P.labelKey)}
                skills=${P.skills}
                onEdit=${s}
                onRemove=${$}
                onUpdate=${S}
                onSetAutoActivate=${E}
                isRemoving=${f}
                isUpdating=${p}
                isSettingAutoActivate=${x}
              />
            `)}
        </div>
      `}return l`
    <div className="space-y-4">
      <${xM}
        enabled=${r}
        isSaving=${y}
        onToggle=${_}
      />
      <${yk} onInstall=${i} isInstalling=${m} />
      <${SM} error=${w} result=${v} />
      ${T}
    </div>
  `}function xM({enabled:e,isSaving:t,onToggle:a}){let n=C();return l`
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
  `}function $M({title:e,skills:t,onEdit:a,onRemove:n,onUpdate:r,onSetAutoActivate:s,isRemoving:i,isUpdating:o,isSettingAutoActivate:u}){return t.length===0?null:l`
    <${ee} padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${e}
      </h3>
      ${t.map(c=>l`
          <${gk}
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
  `}function wM(e){let t=[{id:"user",labelKey:"skills.group.user",skills:[]},{id:"system",labelKey:"skills.group.system",skills:[]},{id:"workspace",labelKey:"skills.group.workspace",skills:[]}],a=t[0];for(let n of e){let r=n.source_kind||"";(r==="system"?t[1]:r==="workspace"?t[2]:a).skills.push(n)}return t.filter(n=>n.skills.length>0)}function SM({error:e,result:t}){return!e&&!t?null:l`
    <div
      className=${e?"rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200":"rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200"}
    >
      ${e||t}
    </div>
  `}function dd(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function xk(){let e=J(),t=I({queryKey:["settings-tools"],queryFn:F$}),a=t.data?.tools||[],[n,r]=h.default.useState({}),s=Q({mutationFn:async({name:o,state:u})=>dd(await B$(o,u),"Save failed"),onSuccess:(o,{name:u,state:c})=>{e.setQueryData(["settings-tools"],d=>{if(!d)return d;let m=o?.tool;return{...d,tools:d.tools.map(f=>f.name===u?{...f,state:c,...m||{}}:f)}}),r(d=>({...d,[u]:!0})),setTimeout(()=>r(d=>({...d,[u]:!1})),2e3)}}),i=h.default.useCallback((o,u)=>s.mutate({name:o,state:u}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}var md="agent.auto_approve_tools";function NM({visible:e}){let t=C();return e?l`
    <span className="font-mono text-[11px] text-[var(--v2-accent-text)]" role="status">
      ${t("tools.saved")}
    </span>
  `:null}function _M({checked:e,disabled:t=!1,label:a,onChange:n}){return l`
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
  `}function zh({settings:e,onSave:t,savedKeys:a,isLoading:n}){let r=C(),s=r("settings.field.autoApproveEligibleTools"),i=e?.[md]===!0||e?.[md]==="true";return l`
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
        <${NM} visible=${a?.[md]} />
        <${_M}
          checked=${i}
          disabled=${n}
          label=${s}
          onChange=${o=>t(md,o)}
        />
      </div>
    <//>
  `}function kM({tool:e,onPermissionChange:t,isSaved:a}){let n=C(),r=[{value:"default",label:n("tools.followDefault"),tone:"neutral"},{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask_each_time",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],s={default:n("tools.sourceDefault"),global:n("tools.sourceGlobal"),override:n("tools.sourceOverride")},i=e.locked,o=r.find(m=>m.value===e.state)||r[1],u=e.effective_source||"default",c=u==="override"?e.state:"default",d=u==="default"&&e.state===e.default_state;return l`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="flex min-w-0 items-center gap-3">
        ${i&&l`<${O}
          name="lock"
          className="h-3.5 w-3.5 shrink-0 text-[var(--v2-text-faint)]"
        />`}
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
        ${i?l`<${z} tone=${o.tone} label=${o.label} size="sm" />`:l`
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
  `}function $k({settings:e={},onSave:t=()=>{},savedKeys:a={},isLoading:n=!1,searchQuery:r=""}){let s=C(),{tools:i,query:o,setPermission:u,savedTools:c}=xk();if(o.isLoading)return l`
      <div className="space-y-4">
        <${zh}
          settings=${e}
          onSave=${t}
          savedKeys=${a}
          isLoading=${n}
        />
        <${ee} padding="md">
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
        <${zh}
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
    `;let d=i.filter(m=>at(r,[m.name,m.description,m.state,m.default_state,m.effective_source,m.locked?s("tools.disabled"):""]));return l`
    <div className="space-y-4">
      <${zh}
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

      <${ee} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${s("tools.permissions")}
        </h3>
        ${d.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${s("tools.noMatch")}
            </p>`:d.map(m=>l`
                  <${kM}
                    key=${m.name}
                    tool=${m}
                    onPermissionChange=${u}
                    isSaved=${c[m.name]}
                  />
                `)}
      <//>
    </div>
  `}function wk(e){return(Number(e)||0).toFixed(2)}function RM(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function Sk(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function Vr({label:e,value:t,description:a}){return l`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&l`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function Nk({searchQuery:e=""}){let t=C(),{credits:a,query:n,authorize:r}=$c();if(!at(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return l`<${_t} query=${e} />`;let s;if(n.isLoading)s=l`
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
        <${Vr}
          label=${t("traceCommons.enrollment")}
          value=${a.enrolled?t("traceCommons.enrolled"):t("traceCommons.notEnrolled")}
        />
        <${Vr}
          label=${t("traceCommons.pendingCredit")}
          description=${t("traceCommons.pendingCreditDesc")}
          value=${wk(a.pending_credit)}
        />
        <${Vr}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${wk(a.final_credit)}
        />
        <${Vr}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${RM(a.delayed_credit_delta)}
        />
        <${Vr}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${Vr}
          label=${t("traceCommons.lastSubmission")}
          value=${Sk(a.last_submission_at,t)}
        />
        <${Vr}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${Sk(a.last_credit_sync_at,t)}
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
  `}function _k(){let e=J(),t=I({queryKey:["admin-users"],queryFn:Z$,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=Q({mutationFn:W$,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=Q({mutationFn:({id:i,payload:o})=>ew(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function CM({onCreate:e,isCreating:t,error:a}){let n=C(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState("member"),[d,m]=h.default.useState(!1),f=p=>{p.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:u},{onSuccess:()=>{s(""),o(""),m(!1)}})};return d?l`
    <${ee} padding="md">
      <h3
        className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${n("users.newUser")}
      </h3>
      <form onSubmit=${f} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <${Rn} label=${n("users.displayName")} htmlFor="user-name">
            <${At}
              id="user-name"
              type="text"
              value=${r}
              onChange=${p=>s(p.target.value)}
              required
            />
          <//>
          <${Rn} label=${n("users.email")} htmlFor="user-email">
            <${At}
              id="user-email"
              type="email"
              value=${i}
              onChange=${p=>o(p.target.value)}
            />
          <//>
        </div>
        <${Rn} label=${n("users.role")} htmlFor="user-role">
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
        <${O} name="plus" className="mr-2 h-4 w-4" />
        ${n("users.addUser")}
      <//>
    `}function EM({user:e}){let t=C(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return l`
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
  `}function kk({searchQuery:e=""}){let t=C(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=_k();if(n.isLoading)return l`
      <${ee} padding="md">
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
    `;if(n.error)return l`
      <${ee} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("users.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let u=a.filter(c=>at(e,[c.id,c.display_name,c.email,c.role,c.status,c.last_active]));return l`
    <div className="space-y-5">
      <${CM}
        onCreate=${s}
        isCreating=${o}
        error=${i}
      />

      <${ee} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("users.title",{count:u.length})}
        </h3>
        ${a.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("users.noUsers")}
            </p>`:u.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("settings.noMatchingSettings",{query:e})}
            </p>`:u.map(c=>l`<${EM} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function Rk(){let e=J(),t=I({queryKey:["settings-export"],queryFn:T$,staleTime:3e4}),a=t.data?.settings||{},[n,r]=h.default.useState({}),[s,i]=h.default.useState(!1),o=Q({mutationFn:async({key:m,value:f})=>dd(await qp(m,f),"Save failed"),onSuccess:(m,{key:f,value:p})=>{e.setQueryData(["settings-export"],x=>{if(!x)return x;let y={...x,settings:{...x.settings}};return p==null?delete y.settings[f]:y.settings[f]=p,y}),r(x=>({...x,[f]:!0})),setTimeout(()=>r(x=>({...x,[f]:!1})),2e3),Fh.has(f)&&i(!0),f==="agent.auto_approve_tools"&&e.invalidateQueries({queryKey:["settings-tools"]})}}),u=h.default.useCallback((m,f)=>o.mutate({key:m,value:f}),[o]),c=Q({mutationFn:A$,onSuccess:(m,f)=>{e.invalidateQueries({queryKey:["settings-export"]}),Object.keys(f?.settings||{}).some(x=>Fh.has(x))&&i(!0)}}),d=h.default.useCallback(m=>c.mutateAsync(m),[c]);return{settings:a,query:t,save:u,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error||c.error}}function qh(){let e=C(),{tab:t}=it(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=ya(),s=r?"inference":"language",i=t||s,{settings:o,query:u,save:c,savedKeys:d,needsRestart:m,saveError:f}=Rk(),[p,x]=h.default.useState("");h.default.useEffect(()=>{x("")},[i]);let y=u.isLoading,w={inference:l`<${dk}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,agent:l`<${sk}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,channels:l`<${lk} searchQuery=${p} />`,networking:l`<${fk}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,tools:l`<${$k}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,skills:l`<${bk} searchQuery=${p} />`,traces:l`<${Nk} searchQuery=${p} />`,users:l`<${kk} searchQuery=${p} />`,language:l`<${mk} searchQuery=${p} />`},g=E=>E==="users"||E==="inference",v=E=>Object.prototype.hasOwnProperty.call(w,E),b=Object.keys(w).filter(E=>r||!g(E)),S=v(s)&&b.includes(s)?s:b[0]||"language";return!v(i)||!r&&g(i)?l`<${ot} to=${`/settings/${S}`} replace />`:l`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${m&&l`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${hk}
                visible=${!0}
                gatewayStatus=${a}
                gatewayStatusQuery=${n}
              />
            </div>`}

            ${f&&l`
              <div
                className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
              >
                ${e("error.saveFailed",{message:f.message})}
              </div>
            `}

            ${w[i]}
          </div>
        </div>
      </div>
    </div>
  `}var Ih=Object.freeze({todo:!0});function Ck(){return Promise.resolve({users:[],total:0,...Ih})}function Ek(e){return Promise.resolve(null)}function Tk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Ak(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Dk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Mk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Ok(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Lk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Pk(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...Ih})}function Uk(e="day",t){return Promise.resolve({entries:[],...Ih})}function jk(){return I({queryKey:["admin","usage-summary"],queryFn:Pk,refetchInterval:3e4})}function fd(e="day",t){return I({queryKey:["admin","usage",e,t],queryFn:()=>Uk(e,t),refetchInterval:3e4})}function yi(){let e=J(),t=I({queryKey:["admin","users"],queryFn:Ck,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=Q({mutationFn:Tk,onSuccess:s}),o=Q({mutationFn:({id:f,payload:p})=>Ak(f,p),onSuccess:s}),u=Q({mutationFn:f=>Dk(f),onSuccess:s}),c=Q({mutationFn:f=>Mk(f),onSuccess:s}),d=Q({mutationFn:f=>Ok(f),onSuccess:s}),m=Q({mutationFn:({userId:f,name:p})=>Lk(f,p)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(f,p)=>o.mutateAsync({id:f,payload:p}),deleteUser:u.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(f,p)=>m.mutateAsync({userId:f,name:p}),newToken:m.data,clearToken:()=>m.reset()}}function Fk(e){return I({queryKey:["admin","user",e],queryFn:()=>Ek(e),enabled:!!e,refetchInterval:1e4})}function en(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function Ta(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function Bk(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function dr(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function bi(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function xi(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function $i(e){return e==="admin"?"signal":"muted"}function zk(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function qk(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function Ik(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function Kk(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function Hk(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function TM({users:e,onSelectUser:t}){let a=C(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?l`
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
                <td className="py-3 pr-4"><${z} tone=${$i(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${z} tone=${xi(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${dr(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:l`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function Qk({onSelectUser:e,onNavigateTab:t}){let a=C(),n=jk(),{users:r,query:s}=yi(),i=n.data||{},o=zk(r),u=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?l`
      <div className="space-y-5">
        <${q} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            ${[1,2,3,4].map(m=>l`<div key=${m} className="v2-skeleton h-28 rounded-lg" />`)}
          </div>
        <//>
      </div>
    `:l`
    <div className="space-y-5">
      <${q} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.systemOverview")}</h3>
          ${i.uptime_seconds!=null&&l`
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:Bk(i.uptime_seconds)})}</span>
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

      <${q} className="p-5 sm:p-6">
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
            value=${Ta(u.total_cost)}
            tone="signal"
          />
          <${tt}
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
        <${TM} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var AM=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function DM({value:e,max:t}){let a=t>0?e/t*100:0;return l`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function Vk({onSelectUser:e}){let t=C(),[a,n]=h.default.useState("day"),r=fd(a),s=r.data?.usage||[],i=Ik(s),o=Kk(s),u=Hk(i),c=i.length>0?i[0].cost:0;return r.isLoading?l`
      <${q} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
        <div className="grid gap-4 sm:grid-cols-4">
          ${[1,2,3,4].map(d=>l`<div key=${d} className="v2-skeleton h-28 rounded-lg" />`)}
        </div>
      <//>
    `:l`
    <div className="space-y-5">
      <${q} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.overview")}</h3>
          <div className="flex gap-1">
            ${AM.map(d=>l`
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
                <${tt} label=${t("admin.usage.inputTokens")} value=${en(u.input_tokens)} tone="muted" />
                <${tt} label=${t("admin.usage.outputTokens")} value=${en(u.output_tokens)} tone="muted" />
                <${tt} label=${t("admin.usage.totalCost")} value=${Ta(u.cost.toFixed(2))} tone="signal" />
              </div>
            `}
      <//>

      ${i.length>0&&l`
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
                ${i.map(d=>l`
                    <tr key=${d.user_id} className="border-b border-white/[0.06] last:border-0">
                      <td className="py-3 pr-4">
                        <button
                          onClick=${()=>e(d.user_id)}
                          className="font-mono text-xs text-signal hover:underline"
                        >
                          ${bi(d.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${en(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${en(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${Ta(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${DM} value=${d.cost} max=${c} />
                      </td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}

      ${o.length>0&&l`
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
                ${o.map(d=>l`
                    <tr key=${d.model} className="border-b border-white/[0.06] last:border-0">
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${d.model}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${en(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${en(d.output_tokens)}</td>
                      <td className="py-3 font-mono text-xs text-iron-100">${Ta(d.cost.toFixed(2))}</td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}
    </div>
  `}function mr({label:e,children:t}){return l`
    <div className="flex items-start justify-between gap-4 border-t border-white/[0.06] py-3 first:border-0 first:pt-0">
      <span className="text-xs text-iron-300">${e}</span>
      <span className="text-right text-sm text-iron-100">${t}</span>
    </div>
  `}function Gk({userId:e,onBack:t}){let a=C(),n=Fk(e),r=fd("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:u,createToken:c,newToken:d,clearToken:m}=yi(),[f,p]=h.default.useState(null),[x,y]=h.default.useState(!1),w=n.data,g=r.data?.usage||[];if(h.default.useEffect(()=>{w&&f===null&&p(w.role)},[w]),n.isLoading)return l`
      <div className="space-y-5">
        <${q} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-2 h-6 w-48 rounded" />
          <div className="v2-skeleton h-4 w-32 rounded" />
        <//>
      </div>
    `;if(n.error)return l`
      <${q} className="p-5 sm:p-6">
        <p className="text-sm text-red-200">${a("error.loadFailed",{what:a("admin.users.user"),message:n.error.message})}</p>
      <//>
    `;if(!w)return null;let v=async()=>{f&&f!==w.role&&await o(w.id,{role:f})},b=async()=>{await u(w.id),t()},$=async()=>{let S=window.prompt(a("admin.users.tokenNamePrompt",{name:w.display_name||a("admin.users.userFallback")}));S&&await c(w.id,S)};return l`
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
            <h2 className="text-2xl font-semibold tracking-tight text-white">${w.display_name||w.id}</h2>
            <div className="mt-2 flex items-center gap-2">
              <${z} tone=${$i(w.role)} label=${w.role||"member"} />
              <${z} tone=${xi(w.status)} label=${w.status||"active"} />
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            ${w.status==="active"?l`<${A} variant="secondary" onClick=${()=>s(w.id)}>${a("admin.users.suspend")}<//>`:l`<${A} variant="secondary" onClick=${()=>i(w.id)}>${a("admin.users.activate")}<//>`}
            <${A} variant="secondary" onClick=${$}>${a("admin.users.createToken")}<//>
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
              <${O} name="close" className="h-4 w-4" />
            </button>
          </div>
        </div>
      `}

      <div className="grid gap-5 lg:grid-cols-2">
        <${q} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.profile")}</h3>
          <${mr} label=${a("admin.user.id")}>
            <span className="font-mono text-xs">${w.id}</span>
          <//>
          <${mr} label=${a("admin.user.email")}>${w.email||a("admin.user.notSet")}<//>
          <${mr} label=${a("admin.user.created")}>${dr(w.created_at)}<//>
          <${mr} label=${a("admin.user.lastLogin")}>${dr(w.last_login_at)}<//>
          ${w.created_by&&l`
            <${mr} label=${a("admin.user.createdBy")}>
              <span className="font-mono text-xs">${bi(w.created_by)}</span>
            <//>
          `}
        <//>

        <${q} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${mr} label=${a("admin.user.jobs")}>${w.job_count??0}<//>
          <${mr} label=${a("admin.user.totalCost")}>${Ta(w.total_cost)}<//>
          <${mr} label=${a("admin.user.lastActive")}>${dr(w.last_active_at)}<//>
        <//>
      </div>

      <${q} className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.roleManagement")}</h3>
        <div className="flex items-end gap-3">
          <div>
            <label className="mb-1 block text-xs text-iron-300">${a("admin.user.currentRole")}</label>
            <select
              value=${f||w.role}
              onChange=${S=>p(S.target.value)}
              className="v2-select h-9 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            >
              <option value="member">${a("admin.users.member")}</option>
              <option value="admin">${a("admin.users.admin")}</option>
            </select>
          </div>
          <${A} onClick=${v} disabled=${!f||f===w.role}>
            ${a("admin.user.saveRole")}
          <//>
        </div>
      <//>

      <${q} className="p-5 sm:p-6">
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
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${en(S.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${en(S.output_tokens)}</td>
                          <td className="py-3 font-mono text-xs text-iron-100">${Ta(S.total_cost)}</td>
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
              ${a("admin.users.deleteUserDesc",{name:w.display_name})}
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
  `}function MM(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function OM({token:e,onDismiss:t}){let a=C(),[n,r]=h.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return l`
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
          <${O} name="close" className="h-4 w-4" />
        </button>
      </div>
    </div>
  `}function LM({onCreate:e,isCreating:t,error:a}){let n=C(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState("member"),[d,m]=h.default.useState(!1),f=async p=>{p.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:u}),s(""),o(""),m(!1))};return d?l`
    <${q} className="p-5 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${n("admin.users.createUser")}</h3>
      <form onSubmit=${f} className="space-y-4">
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
          <${A} type="submit" disabled=${t}>
            ${n(t?"admin.users.creating":"admin.users.createUser")}
          <//>
          <${A} variant="ghost" type="button" onClick=${()=>m(!1)}>${n("admin.users.cancel")}<//>
        </div>
      </form>
    <//>
  `:l`
      <${A} variant="secondary" onClick=${()=>m(!0)}>
        <${O} name="plus" className="mr-2 h-4 w-4" />
        ${n("admin.users.newUser")}
      <//>
    `}function PM({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=C();return l`
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
  `}function UM({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=C();return l`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${z} tone=${$i(e.role)} label=${e.role||"member"} />
          <${z} tone=${xi(e.status)} label=${e.status||"active"} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          ${e.email&&l`<span className="font-mono text-xs text-iron-300">${e.email}</span>`}
          <span className="font-mono text-xs text-iron-700">${bi(e.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-iron-300 sm:inline">
          ${e.job_count!=null?i("admin.users.jobsCount",{count:e.job_count}):""}
          ${e.total_cost!=null?` \xB7 ${Ta(e.total_cost)}`:""}
        </span>
        <span className="hidden text-xs text-iron-700 lg:inline">${dr(e.last_active_at)}</span>
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
  `}function Yk({selectedUserId:e,onSelectUser:t}){let a=C(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:u,updateUser:c,deleteUser:d,suspendUser:m,activateUser:f,createToken:p,newToken:x,clearToken:y}=yi(),[w,g]=h.default.useState(""),[v,b]=h.default.useState("all"),[$,S]=h.default.useState(null),E=qk(n,{search:w,filter:v}),_=MM(a),T=D=>{S({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{m(D),S(null)}})},L=async(D,P)=>{let R=window.prompt(a("admin.users.tokenNamePrompt",{name:P||a("admin.users.userFallback")}));R&&await p(D,R)};return r.isLoading?l`
      <${q} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-3 w-24 rounded" />
        ${[1,2,3].map(D=>l`
          <div key=${D} className="flex items-center justify-between border-t border-iron-700 py-3.5 first:border-0">
            <div className="v2-skeleton h-4 w-32 rounded" />
            <div className="v2-skeleton h-6 w-20 rounded-full" />
          </div>
        `)}
      <//>
    `:s?l`
      <${q} className="p-6 sm:p-8">
        <div className="flex items-center gap-3">
          <${O} name="lock" className="h-5 w-5 text-iron-700" />
          <h3 className="text-lg font-semibold text-iron-100">${a("users.adminRequired")}</h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${a("users.adminRequiredDesc")}
        </p>
      <//>
    `:l`
    <div className="space-y-5">
      ${x&&l`
        <${OM}
          token=${x.token||x.plaintext_token}
          onDismiss=${y}
        />
      `}

      <${LM} onCreate=${i} isCreating=${o} error=${u} />

      <${q} className="p-5 sm:p-6">
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${a("admin.users.title",{count:E.length,total:n.length})}
          </h3>
          <div className="flex items-center gap-2">
            <input
              type="text"
              placeholder=${a("admin.users.searchPlaceholder")}
              value=${w}
              onChange=${D=>g(D.target.value)}
              className="h-8 w-48 rounded-md border border-iron-700 bg-iron-800/70 px-3 text-xs text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
            />
            <div className="flex gap-1">
              ${_.map(D=>l`
                  <button
                    key=${D.value}
                    onClick=${()=>b(D.value)}
                    className=${["rounded-md px-2.5 py-1.5 text-[11px] font-medium",v===D.value?"border border-signal/35 bg-signal/10 text-iron-100":"border border-transparent text-iron-300 hover:text-iron-100"].join(" ")}
                  >
                    ${D.label}
                  </button>
                `)}
            </div>
          </div>
        </div>

        ${E.length===0?l`<p className="py-4 text-sm text-iron-300">${a("admin.users.noMatch")}</p>`:E.map(D=>l`
                <${UM}
                  key=${D.id}
                  user=${D}
                  onSelect=${t}
                  onSuspend=${T}
                  onActivate=${f}
                  onChangeRole=${(P,R)=>c(P,{role:R})}
                  onCreateToken=${L}
                />
              `)}
      <//>

      ${$&&l`
        <${PM}
          title=${$.title}
          message=${$.message}
          confirmLabel=${$.confirmLabel}
          onConfirm=${$.onConfirm}
          onCancel=${()=>S(null)}
        />
      `}
    </div>
  `}function Jk(){let{tab:e="dashboard"}=it(),t=pe(),[a,n]=h.default.useState(null),r=h.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=h.default.useCallback(()=>{n(null)},[]),i={dashboard:l`<${Qk}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?l`<${Gk} userId=${a} onBack=${s} />`:l`<${Yk}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:l`<${Vk} onSelectUser=${r} />`};return i[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:l`<${ot} to="/admin/dashboard" replace />`}var jM=2e3,FM=500,BM=2e3,zM=new Set([403,404]),qM=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function IM(e=globalThis.location,t=null){let a=new URLSearchParams(e?.search||""),n={active:[]};for(let[r,s,i]of qM){let o=a.get(s)?.trim();o?(n[r]=o,n.active.push({key:r,param:s,labelKey:i,value:o})):n[r]=null}return!n.threadId&&t&&(n.threadId=t),n}function Xk({isAdmin:e=!1,defaultThreadId:t=null}={}){let a=Le(),n=a?.search||"",r=h.default.useMemo(()=>IM(a,t),[t,n]),{runId:s,source:i,threadId:o,toolCallId:u,toolName:c,turnId:d}=r,[m,f]=h.default.useState([]),[p,x]=h.default.useState("all"),[y,w]=h.default.useState(""),[g,v]=h.default.useState(!1),[b,$]=h.default.useState(!0),[S,E]=h.default.useState(!0),[_,T]=h.default.useState(null),L=h.default.useRef(new Set),D=h.default.useRef(0),P=!e&&!o;h.default.useEffect(()=>{D.current+=1,f([]),T(null)},[e,s,i,o,u,c,d]);let R=h.default.useCallback(async()=>{if(P){E(!1);return}let se=++D.current;E(!0);try{let ce={limit:FM,level:p==="all"?null:p,target:y.trim()||null,threadId:o,runId:s,turnId:d,toolCallId:u,toolName:c,source:i},oe;try{oe=await(e?Bx(ce):Ep(ce))}catch(pt){if(!e||!zM.has(pt?.status))throw pt;oe=await Ep(ce)}if(se!==D.current)return;let Je=L.current,yt=P2(oe).entries.filter(pt=>!Je.has(pt.id));f(yt),T(null)}catch(ce){if(se!==D.current)return;T(ce)}finally{se===D.current&&E(!1)}},[e,p,P,s,i,y,o,u,c,d]);h.default.useEffect(()=>{R()},[R]),h.default.useEffect(()=>{if(g||P)return;let se=setInterval(R,jM);return()=>clearInterval(se)},[R,P,g]);let F=h.default.useCallback(()=>{v(se=>!se)},[]),G=h.default.useCallback(()=>{let se=[...L.current,...m.map(ce=>ce.id)].slice(-BM);L.current=new Set(se),f([])},[m]);return{entries:m,totalCount:m.length,paused:g,togglePause:F,clearEntries:G,levelFilter:p,setLevelFilter:x,targetFilter:y,setTargetFilter:w,autoScroll:b,setAutoScroll:$,serverLevel:null,changeServerLevel:async()=>{},scope:r,needsThreadScope:P,status:P?"needs_scope":_?"error":S?"loading":"ready",isLoading:S,error:_}}var KM=["all","trace","debug","info","warn","error"],HM=["trace","debug","info","warn","error"],Zk={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},QM={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function VM({entry:e}){let t=C(),[a,n]=h.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=Zk[e.level]||Zk.info,i=QM[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(u=>!!u.value);return l`
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
  `}function Wk({value:e,onChange:t,options:a,labelKey:n,t:r}){return l`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>l`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function GM({label:e,value:t,scopeKey:a}){return l`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function eR(){let e=C(),{isAdmin:t=!1,threadsState:a}=ya()||{},{entries:n,totalCount:r,paused:s,togglePause:i,clearEntries:o,levelFilter:u,setLevelFilter:c,targetFilter:d,setTargetFilter:m,autoScroll:f,setAutoScroll:p,serverLevel:x,changeServerLevel:y,scope:w,isLoading:g,error:v,needsThreadScope:b}=Xk({isAdmin:t,defaultThreadId:t?null:a?.activeThreadId||null}),$=h.default.useRef(null),S=h.default.useRef(!0);h.default.useEffect(()=>{f&&S.current&&$.current&&($.current.scrollTop=0)},[n,f]);let E=h.default.useCallback(L=>{S.current=L.currentTarget.scrollTop<=48},[]),_=n.length>0,T=w?.active||[];return l`
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${Wk}
          value=${u}
          onChange=${c}
          options=${KM}
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
              onChange=${L=>p(L.target.checked)}
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

        ${T.length>0&&l`
          <div
            data-testid="logs-scope-toolbar"
            className="flex w-full flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]"
          >
            <span className="font-medium text-[var(--v2-text-strong)]">${e("logs.scoped")}</span>
            ${T.map(L=>l`<${GM} key=${L.param} scopeKey=${L.param} label=${e(L.labelKey)} value=${L.value} />`)}
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
            <${Wk}
              value=${x}
              onChange=${y}
              options=${HM}
              labelKey=${L=>`logs.level.${L}`}
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
        ref=${$}
        onScroll=${E}
        className="min-h-0 flex-1 overflow-y-auto bg-[var(--v2-canvas)]"
      >
        ${v&&_?l`
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
            `:v&&!_?l`
              <div
                className="flex h-full items-center justify-center px-6 text-center text-sm text-red-300"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:v.message||v.statusText||"Request failed"})}
              </div>
            `:g&&!_?l`
                <div
                  className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
                >
                  ${e("common.loading")}
                </div>
              `:_?n.map(L=>l`<${VM} key=${L.id} entry=${L} />`):l`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function aR(){return l`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function YM({auth:e}){let t=pe(),n=Le().state?.from,r=n?`${n.pathname||Fr}${n.search||""}${n.hash||""}`:Fr,s=`/v2${r==="/"?"":r}`,i=h.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?l`<${aR} />`:e.isAuthenticated?l`<${ot} to=${r} replace />`:l`<${v1}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function JM({auth:e,children:t}){let a=Le();return e.isChecking?l`<${aR} />`:e.isAuthenticated?t:l`<${ot} to="/login" replace state=${{from:a}} />`}function XM({auth:e}){return l`
    <${JM} auth=${e}>
      <${Hw}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        rebornProjectsEnabled=${e.rebornProjectsEnabled}
        onSignOut=${e.signOut}
      />
    <//>
  `}function tR({auth:e}){return e.isAdmin?l`<${Jk} />`:l`<${ot} to=${Fr} replace />`}function nR(){let e=N$();return l`
    <${_p} basename="/v2">
      <${wp}>
        <${ye} path="/login" element=${l`<${YM} auth=${e} />`} />
        <${ye} path="/" element=${l`<${XM} auth=${e} />`}>
          <${ye} index element=${l`<${ot} to=${Fr} replace />`} />
          <${ye} path="overview" element=${l`<${ot} to=${Fr} replace />`} />
          <${ye} path="welcome" element=${l`<${I2} />`} />
          <${ye} path="chat" element=${l`<${gh} />`} />
          <${ye} path="chat/:threadId" element=${l`<${gh} />`} />
          <${ye} path="workspace" element=${l`<${bh} />`} />
          <${ye} path="workspace/*" element=${l`<${bh} />`} />
          <${ye} path="projects" element=${l`<${sl} />`} />
          <${ye} path="projects/:projectId" element=${l`<${sl} />`} />
          <${ye} path="projects/:projectId/missions/:missionId" element=${l`<${sl} />`} />
          <${ye} path="projects/:projectId/threads/:threadId" element=${l`<${sl} />`} />
          <${ye} path="missions" element=${l`<${$h} />`} />
          <${ye} path="missions/:missionId" element=${l`<${$h} />`} />
          <${ye} path="jobs" element=${l`<${Nh} />`} />
          <${ye} path="jobs/:jobId" element=${l`<${Nh} />`} />
          <${ye} path="routines" element=${l`<${kh} />`} />
          <${ye} path="routines/:routineId" element=${l`<${kh} />`} />
          <${ye} path="automations" element=${l`<${YN} />`} />
          <${ye} path="extensions" element=${l`<${jh} />`} />
          <${ye} path="extensions/:tab" element=${l`<${jh} />`} />
          <${ye} path="logs" element=${l`<${eR} />`} />
          <${ye} path="settings" element=${l`<${qh} />`} />
          <${ye} path="settings/:tab" element=${l`<${qh} />`} />
          <${ye} path="admin" element=${l`<${tR} auth=${e} />`} />
          <${ye} path="admin/:tab" element=${l`<${tR} auth=${e} />`} />
        <//>
        <${ye} path="*" element=${l`<${ot} to=${Fr} replace />`} />
      <//>
    <//>
  `}Hh("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","approval.viewFullCommand":"View full command","approval.showCommandPreview":"Show preview","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.tabDeclined":"Declined","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitDeclined":"declined","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.serviceUnavailable":"Service unavailable","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveEligibleTools":"Always allow eligible tools","settings.field.autoApproveEligibleToolsDesc":"Applies to tools set to \u201CFollow global\u201D. Per-tool settings override this; hard-floor approval gates still ask.","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.followDefault":"Follow global","tools.sourceDefault":"default permission","tools.sourceGlobal":"global setting","tools.sourceOverride":"per-tool override","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","skills.defaultAutoActivationEnabled":"Default skill auto-activation enabled","skills.defaultAutoActivationDisabled":"Default skill auto-activation disabled","skills.defaultAutoActivationOnDesc":"Skills auto-activate by keyword on matching requests. Turn off to require an explicit /name.","skills.defaultAutoActivationOffDesc":"Skills run only when you type /name. Turn on to let them auto-activate by keyword.","skills.defaultAutoActivationOnButton":"Default: On","skills.defaultAutoActivationOffButton":"Default: Off","skills.autoActivateOnTitle":"Auto-activation on \u2014 runs on matching requests. Click to make it explicit-only (/name).","skills.autoActivateOffTitle":"Explicit-only \u2014 runs only when you type /name. Click to enable auto-activation.","skills.autoActivateOnLabel":"Auto-activate: On","skills.autoActivateOffLabel":"Auto-activate: Off","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Memory, files & attachments","workspace.readOnly":"Read-only","workspace.filterPlaceholder":"Filter by name\u2026","workspace.emptyDir":"This folder is empty.","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.noFiles":"No files here.","workspace.noMatches":"Nothing matches that filter.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a file","workspace.pickFileDesc":"Choose a file from the tree to preview or download it. This viewer is read-only.","workspace.parent":"Parent: {path}","workspace.download":"Download","workspace.binaryPreviewUnavailable":"No inline preview for this file type. Download it to view the contents.","workspace.fileMeta":"{mime} \xB7 {size} bytes","workspace.unableOpenDirectory":"Unable to open directory","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.filter.completed":"Completed","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.schedule.onceAt":"Once on {datetime}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.filterAction":"Show {label}","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.empty.onboardingTitle":"No automations yet","automations.empty.onboardingDescription":"Automations are created by chatting with your agent \u2014 there's no form to fill out. Ask it to do something on a schedule and it will set up a recurring automation for you.","automations.empty.examplesTitle":"Try asking your agent","automations.empty.example1":"Check the nearai/ironclaw repo every 10 minutes and summarize new issues, PRs, and commits.","automations.empty.example2":"Every weekday at 9am, send me a summary of my unread email.","automations.empty.example3":"Remind me to review open pull requests every afternoon at 3pm.","automations.empty.startInChat":"Start in chat","automations.empty.copyPrompt":"Copy prompt","automations.empty.copied":"Copied","automations.refreshing":"Refreshing\u2026","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.runs.total":"Recent runs: {count}","automations.runs.ok":"OK: {count}","automations.runs.error":"Failed: {count}","automations.runs.running":"Running: {count}","automations.runs.unknown":"Unknown: {count}","automations.runs.showingOf":"Showing {shown} of {total} recent runs","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up a project for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open project","projects.openGeneralWorkspace":"Open project","projects.noDescription":"No project description yet. The project is still being shaped by recent activity and thread history.","projects.general.label":"General project","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,rR.createRoot)(document.getElementById("v2-root")).render(l`
  <${Qh}>
    <${Cd} client=${Et}>
      <${nR} />
    <//>
  <//>
`);
