import{a as En,b as Be,c as Ke,d as h,e as u,f as Xh,g as Zh,h as yl,i as R,j as bl}from"./chunks/chunk-IGTNS7XG.js";var gv=En(Cl=>{"use strict";var kR=Symbol.for("react.transitional.element"),RR=Symbol.for("react.fragment");function vv(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:kR,type:e,key:n,ref:t!==void 0?t:null,props:a}}Cl.Fragment=RR;Cl.jsx=vv;Cl.jsxs=vv});var Td=En((GL,yv)=>{"use strict";yv.exports=gv()});var Dv=En(Me=>{"use strict";function Ud(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<Ul(r,t))e[n]=t,e[a]=r,a=n;else break e}}function La(e){return e.length===0?null:e[0]}function Fl(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],l=i+1,c=e[l];if(0>Ul(o,a))l<r&&0>Ul(c,o)?(e[n]=c,e[l]=a,n=l):(e[n]=o,e[i]=a,n=i);else if(l<r&&0>Ul(c,a))e[n]=c,e[l]=a,n=l;else break e}}return t}function Ul(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}Me.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(Sv=performance,Me.unstable_now=function(){return Sv.now()}):(Od=Date,Nv=Od.now(),Me.unstable_now=function(){return Od.now()-Nv});var Sv,Od,Nv,tn=[],Dn=[],AR=1,la=null,xt=3,jd=!1,Fi=!1,Bi=!1,Fd=!1,Rv=typeof setTimeout=="function"?setTimeout:null,Cv=typeof clearTimeout=="function"?clearTimeout:null,_v=typeof setImmediate<"u"?setImmediate:null;function jl(e){for(var t=La(Dn);t!==null;){if(t.callback===null)Fl(Dn);else if(t.startTime<=e)Fl(Dn),t.sortIndex=t.expirationTime,Ud(tn,t);else break;t=La(Dn)}}function Bd(e){if(Bi=!1,jl(e),!Fi)if(La(tn)!==null)Fi=!0,os||(os=!0,is());else{var t=La(Dn);t!==null&&zd(Bd,t.startTime-e)}}var os=!1,zi=-1,Ev=5,Tv=-1;function Av(){return Fd?!0:!(Me.unstable_now()-Tv<Ev)}function Ld(){if(Fd=!1,os){var e=Me.unstable_now();Tv=e;var t=!0;try{e:{Fi=!1,Bi&&(Bi=!1,Cv(zi),zi=-1),jd=!0;var a=xt;try{t:{for(jl(e),la=La(tn);la!==null&&!(la.expirationTime>e&&Av());){var n=la.callback;if(typeof n=="function"){la.callback=null,xt=la.priorityLevel;var r=n(la.expirationTime<=e);if(e=Me.unstable_now(),typeof r=="function"){la.callback=r,jl(e),t=!0;break t}la===La(tn)&&Fl(tn),jl(e)}else Fl(tn);la=La(tn)}if(la!==null)t=!0;else{var s=La(Dn);s!==null&&zd(Bd,s.startTime-e),t=!1}}break e}finally{la=null,xt=a,jd=!1}t=void 0}}finally{t?is():os=!1}}}var is;typeof _v=="function"?is=function(){_v(Ld)}:typeof MessageChannel<"u"?(Pd=new MessageChannel,kv=Pd.port2,Pd.port1.onmessage=Ld,is=function(){kv.postMessage(null)}):is=function(){Rv(Ld,0)};var Pd,kv;function zd(e,t){zi=Rv(function(){e(Me.unstable_now())},t)}Me.unstable_IdlePriority=5;Me.unstable_ImmediatePriority=1;Me.unstable_LowPriority=4;Me.unstable_NormalPriority=3;Me.unstable_Profiling=null;Me.unstable_UserBlockingPriority=2;Me.unstable_cancelCallback=function(e){e.callback=null};Me.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):Ev=0<e?Math.floor(1e3/e):5};Me.unstable_getCurrentPriorityLevel=function(){return xt};Me.unstable_next=function(e){switch(xt){case 1:case 2:case 3:var t=3;break;default:t=xt}var a=xt;xt=t;try{return e()}finally{xt=a}};Me.unstable_requestPaint=function(){Fd=!0};Me.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=xt;xt=e;try{return t()}finally{xt=a}};Me.unstable_scheduleCallback=function(e,t,a){var n=Me.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:AR++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,Ud(Dn,e),La(tn)===null&&e===La(Dn)&&(Bi?(Cv(zi),zi=-1):Bi=!0,zd(Bd,a-n))):(e.sortIndex=r,Ud(tn,e),Fi||jd||(Fi=!0,os||(os=!0,is()))),e};Me.unstable_shouldYield=Av;Me.unstable_wrapCallback=function(e){var t=xt;return function(){var a=xt;xt=t;try{return e.apply(this,arguments)}finally{xt=a}}}});var Ov=En((T6,Mv)=>{"use strict";Mv.exports=Dv()});var Pv=En(kt=>{"use strict";var DR=Ke();function Lv(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Mn(){}var _t={d:{f:Mn,r:function(){throw Error(Lv(522))},D:Mn,C:Mn,L:Mn,m:Mn,X:Mn,S:Mn,M:Mn},p:0,findDOMNode:null},MR=Symbol.for("react.portal");function OR(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:MR,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var qi=DR.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function Bl(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}kt.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=_t;kt.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(Lv(299));return OR(e,t,null,a)};kt.flushSync=function(e){var t=qi.T,a=_t.p;try{if(qi.T=null,_t.p=2,e)return e()}finally{qi.T=t,_t.p=a,_t.d.f()}};kt.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,_t.d.C(e,t))};kt.prefetchDNS=function(e){typeof e=="string"&&_t.d.D(e)};kt.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=Bl(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?_t.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&_t.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};kt.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=Bl(t.as,t.crossOrigin);_t.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&_t.d.M(e)};kt.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=Bl(a,t.crossOrigin);_t.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};kt.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=Bl(t.as,t.crossOrigin);_t.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else _t.d.m(e)};kt.requestFormReset=function(e){_t.d.r(e)};kt.unstable_batchedUpdates=function(e,t){return e(t)};kt.useFormState=function(e,t,a){return qi.H.useFormState(e,t,a)};kt.useFormStatus=function(){return qi.H.useHostTransitionStatus()};kt.version="19.1.0"});var Fv=En((D6,jv)=>{"use strict";function Uv(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(Uv)}catch(e){console.error(e)}}Uv(),jv.exports=Pv()});var z0=En(lc=>{"use strict";var rt=Ov(),oy=Ke(),LR=Fv();function U(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function ly(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function To(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function uy(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function Bv(e){if(To(e)!==e)throw Error(U(188))}function PR(e){var t=e.alternate;if(!t){if(t=To(e),t===null)throw Error(U(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return Bv(r),e;if(s===n)return Bv(r),t;s=s.sibling}throw Error(U(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(U(189))}}if(a.alternate!==n)throw Error(U(190))}if(a.tag!==3)throw Error(U(188));return a.stateNode.current===a?e:t}function cy(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=cy(e),t!==null)return t;e=e.sibling}return null}var Ae=Object.assign,UR=Symbol.for("react.element"),zl=Symbol.for("react.transitional.element"),Xi=Symbol.for("react.portal"),ps=Symbol.for("react.fragment"),dy=Symbol.for("react.strict_mode"),bm=Symbol.for("react.profiler"),jR=Symbol.for("react.provider"),my=Symbol.for("react.consumer"),on=Symbol.for("react.context"),hf=Symbol.for("react.forward_ref"),xm=Symbol.for("react.suspense"),$m=Symbol.for("react.suspense_list"),vf=Symbol.for("react.memo"),Pn=Symbol.for("react.lazy");Symbol.for("react.scope");var wm=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var FR=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var zv=Symbol.iterator;function Ii(e){return e===null||typeof e!="object"?null:(e=zv&&e[zv]||e["@@iterator"],typeof e=="function"?e:null)}var BR=Symbol.for("react.client.reference");function Sm(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===BR?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case ps:return"Fragment";case bm:return"Profiler";case dy:return"StrictMode";case xm:return"Suspense";case $m:return"SuspenseList";case wm:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case Xi:return"Portal";case on:return(e.displayName||"Context")+".Provider";case my:return(e._context.displayName||"Context")+".Consumer";case hf:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case vf:return t=e.displayName||null,t!==null?t:Sm(e.type)||"Memo";case Pn:t=e._payload,e=e._init;try{return Sm(e(t))}catch{}}return null}var Zi=Array.isArray,te=oy.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,ye=LR.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,$r={pending:!1,data:null,method:null,action:null},Nm=[],hs=-1;function qa(e){return{current:e}}function dt(e){0>hs||(e.current=Nm[hs],Nm[hs]=null,hs--)}function Le(e,t){hs++,Nm[hs]=e.current,e.current=t}var Fa=qa(null),vo=qa(null),Qn=qa(null),gu=qa(null);function yu(e,t){switch(Le(Qn,t),Le(vo,e),Le(Fa,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?Vg(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=Vg(t),e=E0(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}dt(Fa),Le(Fa,e)}function Os(){dt(Fa),dt(vo),dt(Qn)}function _m(e){e.memoizedState!==null&&Le(gu,e);var t=Fa.current,a=E0(t,e.type);t!==a&&(Le(vo,e),Le(Fa,a))}function bu(e){vo.current===e&&(dt(Fa),dt(vo)),gu.current===e&&(dt(gu),ko._currentValue=$r)}var km=Object.prototype.hasOwnProperty,gf=rt.unstable_scheduleCallback,qd=rt.unstable_cancelCallback,zR=rt.unstable_shouldYield,qR=rt.unstable_requestPaint,Ba=rt.unstable_now,IR=rt.unstable_getCurrentPriorityLevel,fy=rt.unstable_ImmediatePriority,py=rt.unstable_UserBlockingPriority,xu=rt.unstable_NormalPriority,KR=rt.unstable_LowPriority,hy=rt.unstable_IdlePriority,HR=rt.log,QR=rt.unstable_setDisableYieldValue,Ao=null,Yt=null;function qn(e){if(typeof HR=="function"&&QR(e),Yt&&typeof Yt.setStrictMode=="function")try{Yt.setStrictMode(Ao,e)}catch{}}var Jt=Math.clz32?Math.clz32:YR,VR=Math.log,GR=Math.LN2;function YR(e){return e>>>=0,e===0?32:31-(VR(e)/GR|0)|0}var ql=256,Il=4194304;function yr(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Vu(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=yr(n):(i&=o,i!==0?r=yr(i):a||(a=o&~e,a!==0&&(r=yr(a))))):(o=n&~s,o!==0?r=yr(o):i!==0?r=yr(i):a||(a=n&~e,a!==0&&(r=yr(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function Do(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function JR(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function vy(){var e=ql;return ql<<=1,(ql&4194048)===0&&(ql=256),e}function gy(){var e=Il;return Il<<=1,(Il&62914560)===0&&(Il=4194304),e}function Id(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function Mo(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function XR(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,l=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Jt(a),m=1<<d;o[d]=0,l[d]=-1;var f=c[d];if(f!==null)for(c[d]=null,d=0;d<f.length;d++){var p=f[d];p!==null&&(p.lane&=-536870913)}a&=~m}n!==0&&yy(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function yy(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Jt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function by(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Jt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function yf(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function bf(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function xy(){var e=ye.p;return e!==0?e:(e=window.event,e===void 0?32:F0(e.type))}function ZR(e,t){var a=ye.p;try{return ye.p=e,t()}finally{ye.p=a}}var nr=Math.random().toString(36).slice(2),$t="__reactFiber$"+nr,Ft="__reactProps$"+nr,Hs="__reactContainer$"+nr,Rm="__reactEvents$"+nr,WR="__reactListeners$"+nr,eC="__reactHandles$"+nr,qv="__reactResources$"+nr,Oo="__reactMarker$"+nr;function xf(e){delete e[$t],delete e[Ft],delete e[Rm],delete e[WR],delete e[eC]}function vs(e){var t=e[$t];if(t)return t;for(var a=e.parentNode;a;){if(t=a[Hs]||a[$t]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=Jg(e);e!==null;){if(a=e[$t])return a;e=Jg(e)}return t}e=a,a=e.parentNode}return null}function Qs(e){if(e=e[$t]||e[Hs]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function Wi(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(U(33))}function ks(e){var t=e[qv];return t||(t=e[qv]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function ut(e){e[Oo]=!0}var $y=new Set,wy={};function Dr(e,t){Ls(e,t),Ls(e+"Capture",t)}function Ls(e,t){for(wy[e]=t,e=0;e<t.length;e++)$y.add(t[e])}var tC=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),Iv={},Kv={};function aC(e){return km.call(Kv,e)?!0:km.call(Iv,e)?!1:tC.test(e)?Kv[e]=!0:(Iv[e]=!0,!1)}function ru(e,t,a){if(aC(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function Kl(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function an(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var Kd,Hv;function ds(e){if(Kd===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);Kd=t&&t[1]||"",Hv=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+Kd+e+Hv}var Hd=!1;function Qd(e,t){if(!e||Hd)return"";Hd=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var m=function(){throw Error()};if(Object.defineProperty(m.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(m,[])}catch(p){var f=p}Reflect.construct(e,[],m)}else{try{m.call()}catch(p){f=p}e.call(m.prototype)}}else{try{throw Error()}catch(p){f=p}(m=e())&&typeof m.catch=="function"&&m.catch(function(){})}}catch(p){if(p&&f&&typeof p.stack=="string")return[p.stack,f.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var l=i.split(`
`),c=o.split(`
`);for(r=n=0;n<l.length&&!l[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===l.length||r===c.length)for(n=l.length-1,r=c.length-1;1<=n&&0<=r&&l[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(l[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||l[n]!==c[r]){var d=`
`+l[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{Hd=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?ds(a):""}function nC(e){switch(e.tag){case 26:case 27:case 5:return ds(e.type);case 16:return ds("Lazy");case 13:return ds("Suspense");case 19:return ds("SuspenseList");case 0:case 15:return Qd(e.type,!1);case 11:return Qd(e.type.render,!1);case 1:return Qd(e.type,!0);case 31:return ds("Activity");default:return""}}function Qv(e){try{var t="";do t+=nC(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function ca(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function Sy(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function rC(e){var t=Sy(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function $u(e){e._valueTracker||(e._valueTracker=rC(e))}function Ny(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=Sy(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function wu(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var sC=/[\n"\\]/g;function fa(e){return e.replace(sC,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function Cm(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+ca(t)):e.value!==""+ca(t)&&(e.value=""+ca(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?Em(e,i,ca(t)):a!=null?Em(e,i,ca(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+ca(o):e.removeAttribute("name")}function _y(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+ca(a):"",t=t!=null?""+ca(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function Em(e,t,a){t==="number"&&wu(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function Rs(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+ca(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function ky(e,t,a){if(t!=null&&(t=""+ca(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+ca(a):""}function Ry(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(U(92));if(Zi(n)){if(1<n.length)throw Error(U(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=ca(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function Ps(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var iC=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function Vv(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||iC.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function Cy(e,t,a){if(t!=null&&typeof t!="object")throw Error(U(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&Vv(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&Vv(e,s,t[s])}function $f(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var oC=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),lC=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function su(e){return lC.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var Tm=null;function wf(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var gs=null,Cs=null;function Gv(e){var t=Qs(e);if(t&&(e=t.stateNode)){var a=e[Ft]||null;e:switch(e=t.stateNode,t.type){case"input":if(Cm(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+fa(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[Ft]||null;if(!r)throw Error(U(90));Cm(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&Ny(n)}break e;case"textarea":ky(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&Rs(e,!!a.multiple,t,!1)}}}var Vd=!1;function Ey(e,t,a){if(Vd)return e(t,a);Vd=!0;try{var n=e(t);return n}finally{if(Vd=!1,(gs!==null||Cs!==null)&&(nc(),gs&&(t=gs,e=Cs,Cs=gs=null,Gv(t),e)))for(t=0;t<e.length;t++)Gv(e[t])}}function go(e,t){var a=e.stateNode;if(a===null)return null;var n=a[Ft]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(U(231,t,typeof a));return a}var pn=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),Am=!1;if(pn)try{ls={},Object.defineProperty(ls,"passive",{get:function(){Am=!0}}),window.addEventListener("test",ls,ls),window.removeEventListener("test",ls,ls)}catch{Am=!1}var ls,In=null,Sf=null,iu=null;function Ty(){if(iu)return iu;var e,t=Sf,a=t.length,n,r="value"in In?In.value:In.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return iu=r.slice(e,1<n?1-n:void 0)}function ou(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function Hl(){return!0}function Yv(){return!1}function Bt(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?Hl:Yv,this.isPropagationStopped=Yv,this}return Ae(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=Hl)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=Hl)},persist:function(){},isPersistent:Hl}),t}var Mr={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Gu=Bt(Mr),Lo=Ae({},Mr,{view:0,detail:0}),uC=Bt(Lo),Gd,Yd,Ki,Yu=Ae({},Lo,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:Nf,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==Ki&&(Ki&&e.type==="mousemove"?(Gd=e.screenX-Ki.screenX,Yd=e.screenY-Ki.screenY):Yd=Gd=0,Ki=e),Gd)},movementY:function(e){return"movementY"in e?e.movementY:Yd}}),Jv=Bt(Yu),cC=Ae({},Yu,{dataTransfer:0}),dC=Bt(cC),mC=Ae({},Lo,{relatedTarget:0}),Jd=Bt(mC),fC=Ae({},Mr,{animationName:0,elapsedTime:0,pseudoElement:0}),pC=Bt(fC),hC=Ae({},Mr,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),vC=Bt(hC),gC=Ae({},Mr,{data:0}),Xv=Bt(gC),yC={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},bC={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},xC={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function $C(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=xC[e])?!!t[e]:!1}function Nf(){return $C}var wC=Ae({},Lo,{key:function(e){if(e.key){var t=yC[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=ou(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?bC[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:Nf,charCode:function(e){return e.type==="keypress"?ou(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?ou(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),SC=Bt(wC),NC=Ae({},Yu,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),Zv=Bt(NC),_C=Ae({},Lo,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:Nf}),kC=Bt(_C),RC=Ae({},Mr,{propertyName:0,elapsedTime:0,pseudoElement:0}),CC=Bt(RC),EC=Ae({},Yu,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),TC=Bt(EC),AC=Ae({},Mr,{newState:0,oldState:0}),DC=Bt(AC),MC=[9,13,27,32],_f=pn&&"CompositionEvent"in window,to=null;pn&&"documentMode"in document&&(to=document.documentMode);var OC=pn&&"TextEvent"in window&&!to,Ay=pn&&(!_f||to&&8<to&&11>=to),Wv=" ",eg=!1;function Dy(e,t){switch(e){case"keyup":return MC.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function My(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var ys=!1;function LC(e,t){switch(e){case"compositionend":return My(t);case"keypress":return t.which!==32?null:(eg=!0,Wv);case"textInput":return e=t.data,e===Wv&&eg?null:e;default:return null}}function PC(e,t){if(ys)return e==="compositionend"||!_f&&Dy(e,t)?(e=Ty(),iu=Sf=In=null,ys=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return Ay&&t.locale!=="ko"?null:t.data;default:return null}}var UC={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function tg(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!UC[e.type]:t==="textarea"}function Oy(e,t,a,n){gs?Cs?Cs.push(n):Cs=[n]:gs=n,t=Bu(t,"onChange"),0<t.length&&(a=new Gu("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var ao=null,yo=null;function jC(e){k0(e,0)}function Ju(e){var t=Wi(e);if(Ny(t))return e}function ag(e,t){if(e==="change")return t}var Ly=!1;pn&&(pn?(Vl="oninput"in document,Vl||(Xd=document.createElement("div"),Xd.setAttribute("oninput","return;"),Vl=typeof Xd.oninput=="function"),Ql=Vl):Ql=!1,Ly=Ql&&(!document.documentMode||9<document.documentMode));var Ql,Vl,Xd;function ng(){ao&&(ao.detachEvent("onpropertychange",Py),yo=ao=null)}function Py(e){if(e.propertyName==="value"&&Ju(yo)){var t=[];Oy(t,yo,e,wf(e)),Ey(jC,t)}}function FC(e,t,a){e==="focusin"?(ng(),ao=t,yo=a,ao.attachEvent("onpropertychange",Py)):e==="focusout"&&ng()}function BC(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return Ju(yo)}function zC(e,t){if(e==="click")return Ju(t)}function qC(e,t){if(e==="input"||e==="change")return Ju(t)}function IC(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var Wt=typeof Object.is=="function"?Object.is:IC;function bo(e,t){if(Wt(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!km.call(t,r)||!Wt(e[r],t[r]))return!1}return!0}function rg(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function sg(e,t){var a=rg(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=rg(a)}}function Uy(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?Uy(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function jy(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=wu(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=wu(e.document)}return t}function kf(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var KC=pn&&"documentMode"in document&&11>=document.documentMode,bs=null,Dm=null,no=null,Mm=!1;function ig(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;Mm||bs==null||bs!==wu(n)||(n=bs,"selectionStart"in n&&kf(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),no&&bo(no,n)||(no=n,n=Bu(Dm,"onSelect"),0<n.length&&(t=new Gu("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=bs)))}function gr(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var xs={animationend:gr("Animation","AnimationEnd"),animationiteration:gr("Animation","AnimationIteration"),animationstart:gr("Animation","AnimationStart"),transitionrun:gr("Transition","TransitionRun"),transitionstart:gr("Transition","TransitionStart"),transitioncancel:gr("Transition","TransitionCancel"),transitionend:gr("Transition","TransitionEnd")},Zd={},Fy={};pn&&(Fy=document.createElement("div").style,"AnimationEvent"in window||(delete xs.animationend.animation,delete xs.animationiteration.animation,delete xs.animationstart.animation),"TransitionEvent"in window||delete xs.transitionend.transition);function Or(e){if(Zd[e])return Zd[e];if(!xs[e])return e;var t=xs[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in Fy)return Zd[e]=t[a];return e}var By=Or("animationend"),zy=Or("animationiteration"),qy=Or("animationstart"),HC=Or("transitionrun"),QC=Or("transitionstart"),VC=Or("transitioncancel"),Iy=Or("transitionend"),Ky=new Map,Om="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");Om.push("scrollEnd");function Na(e,t){Ky.set(e,t),Dr(t,[e])}var og=new WeakMap;function pa(e,t){if(typeof e=="object"&&e!==null){var a=og.get(e);return a!==void 0?a:(t={value:e,source:t,stack:Qv(t)},og.set(e,t),t)}return{value:e,source:t,stack:Qv(t)}}var ua=[],$s=0,Rf=0;function Xu(){for(var e=$s,t=Rf=$s=0;t<e;){var a=ua[t];ua[t++]=null;var n=ua[t];ua[t++]=null;var r=ua[t];ua[t++]=null;var s=ua[t];if(ua[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&Hy(a,r,s)}}function Zu(e,t,a,n){ua[$s++]=e,ua[$s++]=t,ua[$s++]=a,ua[$s++]=n,Rf|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function Cf(e,t,a,n){return Zu(e,t,a,n),Su(e)}function Vs(e,t){return Zu(e,null,null,t),Su(e)}function Hy(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Jt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function Su(e){if(50<po)throw po=0,tf=null,Error(U(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var ws={};function GC(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Gt(e,t,a,n){return new GC(e,t,a,n)}function Ef(e){return e=e.prototype,!(!e||!e.isReactComponent)}function mn(e,t){var a=e.alternate;return a===null?(a=Gt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function Qy(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function lu(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")Ef(e)&&(i=1);else if(typeof e=="string")i=GE(e,a,Fa.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case wm:return e=Gt(31,a,t,r),e.elementType=wm,e.lanes=s,e;case ps:return wr(a.children,r,s,t);case dy:i=8,r|=24;break;case bm:return e=Gt(12,a,t,r|2),e.elementType=bm,e.lanes=s,e;case xm:return e=Gt(13,a,t,r),e.elementType=xm,e.lanes=s,e;case $m:return e=Gt(19,a,t,r),e.elementType=$m,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case jR:case on:i=10;break e;case my:i=9;break e;case hf:i=11;break e;case vf:i=14;break e;case Pn:i=16,n=null;break e}i=29,a=Error(U(130,e===null?"null":typeof e,"")),n=null}return t=Gt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function wr(e,t,a,n){return e=Gt(7,e,n,t),e.lanes=a,e}function Wd(e,t,a){return e=Gt(6,e,null,t),e.lanes=a,e}function em(e,t,a){return t=Gt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var Ss=[],Ns=0,Nu=null,_u=0,da=[],ma=0,Sr=null,ln=1,un="";function br(e,t){Ss[Ns++]=_u,Ss[Ns++]=Nu,Nu=e,_u=t}function Vy(e,t,a){da[ma++]=ln,da[ma++]=un,da[ma++]=Sr,Sr=e;var n=ln;e=un;var r=32-Jt(n)-1;n&=~(1<<r),a+=1;var s=32-Jt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,ln=1<<32-Jt(t)+r|a<<r|n,un=s+e}else ln=1<<s|a<<r|n,un=e}function Tf(e){e.return!==null&&(br(e,1),Vy(e,1,0))}function Af(e){for(;e===Nu;)Nu=Ss[--Ns],Ss[Ns]=null,_u=Ss[--Ns],Ss[Ns]=null;for(;e===Sr;)Sr=da[--ma],da[ma]=null,un=da[--ma],da[ma]=null,ln=da[--ma],da[ma]=null}var Rt=null,ze=null,ge=!1,Nr=null,Ua=!1,Lm=Error(U(519));function Cr(e){var t=Error(U(418,""));throw xo(pa(t,e)),Lm}function lg(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[$t]=e,t[Ft]=n,a){case"dialog":le("cancel",t),le("close",t);break;case"iframe":case"object":case"embed":le("load",t);break;case"video":case"audio":for(a=0;a<So.length;a++)le(So[a],t);break;case"source":le("error",t);break;case"img":case"image":case"link":le("error",t),le("load",t);break;case"details":le("toggle",t);break;case"input":le("invalid",t),_y(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),$u(t);break;case"select":le("invalid",t);break;case"textarea":le("invalid",t),Ry(t,n.value,n.defaultValue,n.children),$u(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||C0(t.textContent,a)?(n.popover!=null&&(le("beforetoggle",t),le("toggle",t)),n.onScroll!=null&&le("scroll",t),n.onScrollEnd!=null&&le("scrollend",t),n.onClick!=null&&(t.onclick=ic),t=!0):t=!1,t||Cr(e)}function ug(e){for(Rt=e.return;Rt;)switch(Rt.tag){case 5:case 13:Ua=!1;return;case 27:case 3:Ua=!0;return;default:Rt=Rt.return}}function Hi(e){if(e!==Rt)return!1;if(!ge)return ug(e),ge=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||lf(e.type,e.memoizedProps)),a=!a),a&&ze&&Cr(e),ug(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(U(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){ze=Sa(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}ze=null}}else t===27?(t=ze,rr(e.type)?(e=df,df=null,ze=e):ze=t):ze=Rt?Sa(e.stateNode.nextSibling):null;return!0}function Po(){ze=Rt=null,ge=!1}function cg(){var e=Nr;return e!==null&&(jt===null?jt=e:jt.push.apply(jt,e),Nr=null),e}function xo(e){Nr===null?Nr=[e]:Nr.push(e)}var Pm=qa(null),Lr=null,cn=null;function jn(e,t,a){Le(Pm,t._currentValue),t._currentValue=a}function fn(e){e._currentValue=Pm.current,dt(Pm)}function Um(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function jm(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var l=0;l<t.length;l++)if(o.context===t[l]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),Um(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(U(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),Um(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function Uo(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(U(387));if(i=i.memoizedProps,i!==null){var o=r.type;Wt(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===gu.current){if(i=r.alternate,i===null)throw Error(U(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(ko):e=[ko])}r=r.return}e!==null&&jm(t,e,a,n),t.flags|=262144}function ku(e){for(e=e.firstContext;e!==null;){if(!Wt(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function Er(e){Lr=e,cn=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function wt(e){return Gy(Lr,e)}function Gl(e,t){return Lr===null&&Er(e),Gy(e,t)}function Gy(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},cn===null){if(e===null)throw Error(U(308));cn=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else cn=cn.next=t;return a}var YC=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},JC=rt.unstable_scheduleCallback,XC=rt.unstable_NormalPriority,at={$$typeof:on,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function Df(){return{controller:new YC,data:new Map,refCount:0}}function jo(e){e.refCount--,e.refCount===0&&JC(XC,function(){e.controller.abort()})}var ro=null,Fm=0,Us=0,Es=null;function ZC(e,t){if(ro===null){var a=ro=[];Fm=0,Us=tp(),Es={status:"pending",value:void 0,then:function(n){a.push(n)}}}return Fm++,t.then(dg,dg),t}function dg(){if(--Fm===0&&ro!==null){Es!==null&&(Es.status="fulfilled");var e=ro;ro=null,Us=0,Es=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function WC(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var mg=te.S;te.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&ZC(e,t),mg!==null&&mg(e,t)};var _r=qa(null);function Mf(){var e=_r.current;return e!==null?e:Re.pooledCache}function uu(e,t){t===null?Le(_r,_r.current):Le(_r,t.pool)}function Yy(){var e=Mf();return e===null?null:{parent:at._currentValue,pool:e}}var Fo=Error(U(460)),Jy=Error(U(474)),Wu=Error(U(542)),Bm={then:function(){}};function fg(e){return e=e.status,e==="fulfilled"||e==="rejected"}function Yl(){}function Xy(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(Yl,Yl),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,hg(e),e;default:if(typeof t.status=="string")t.then(Yl,Yl);else{if(e=Re,e!==null&&100<e.shellSuspendCounter)throw Error(U(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,hg(e),e}throw so=t,Fo}}var so=null;function pg(){if(so===null)throw Error(U(459));var e=so;return so=null,e}function hg(e){if(e===Fo||e===Wu)throw Error(U(483))}var Un=!1;function Of(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function zm(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function Vn(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function Gn(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,(Se&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=Su(e),Hy(e,null,a),t}return Zu(e,n,t,a),Su(e)}function io(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,by(e,a)}}function tm(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var qm=!1;function oo(){if(qm){var e=Es;if(e!==null)throw e}}function lo(e,t,a,n){qm=!1;var r=e.updateQueue;Un=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var l=o,c=l.next;l.next=null,i===null?s=c:i.next=c,i=l;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=l))}if(s!==null){var m=r.baseState;i=0,d=c=l=null,o=s;do{var f=o.lane&-536870913,p=f!==o.lane;if(p?(de&f)===f:(n&f)===f){f!==0&&f===Us&&(qm=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var x=e,y=o;f=t;var $=a;switch(y.tag){case 1:if(x=y.payload,typeof x=="function"){m=x.call($,m,f);break e}m=x;break e;case 3:x.flags=x.flags&-65537|128;case 0:if(x=y.payload,f=typeof x=="function"?x.call($,m,f):x,f==null)break e;m=Ae({},m,f);break e;case 2:Un=!0}}f=o.callback,f!==null&&(e.flags|=64,p&&(e.flags|=8192),p=r.callbacks,p===null?r.callbacks=[f]:p.push(f))}else p={lane:f,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=p,l=m):d=d.next=p,i|=f;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;p=o,o=p.next,p.next=null,r.lastBaseUpdate=p,r.shared.pending=null}}while(!0);d===null&&(l=m),r.baseState=l,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),ar|=i,e.lanes=i,e.memoizedState=m}}function Zy(e,t){if(typeof e!="function")throw Error(U(191,e));e.call(t)}function Wy(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)Zy(a[e],t)}var js=qa(null),Ru=qa(0);function vg(e,t){e=gn,Le(Ru,e),Le(js,t),gn=e|t.baseLanes}function Im(){Le(Ru,gn),Le(js,js.current)}function Lf(){gn=Ru.current,dt(js),dt(Ru)}var er=0,oe=null,_e=null,Ye=null,Cu=!1,Ts=!1,Tr=!1,Eu=0,$o=0,As=null,eE=0;function He(){throw Error(U(321))}function Pf(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!Wt(e[a],t[a]))return!1;return!0}function Uf(e,t,a,n,r,s){return er=s,oe=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,te.H=e===null||e.memoizedState===null?Tb:Ab,Tr=!1,s=a(n,r),Tr=!1,Ts&&(s=tb(t,a,n,r)),eb(e),s}function eb(e){te.H=Tu;var t=_e!==null&&_e.next!==null;if(er=0,Ye=_e=oe=null,Cu=!1,$o=0,As=null,t)throw Error(U(300));e===null||ct||(e=e.dependencies,e!==null&&ku(e)&&(ct=!0))}function tb(e,t,a,n){oe=e;var r=0;do{if(Ts&&(As=null),$o=0,Ts=!1,25<=r)throw Error(U(301));if(r+=1,Ye=_e=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}te.H=oE,s=t(a,n)}while(Ts);return s}function tE(){var e=te.H,t=e.useState()[0];return t=typeof t.then=="function"?Bo(t):t,e=e.useState()[0],(_e!==null?_e.memoizedState:null)!==e&&(oe.flags|=1024),t}function jf(){var e=Eu!==0;return Eu=0,e}function Ff(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function Bf(e){if(Cu){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}Cu=!1}er=0,Ye=_e=oe=null,Ts=!1,$o=Eu=0,As=null}function Pt(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return Ye===null?oe.memoizedState=Ye=e:Ye=Ye.next=e,Ye}function Je(){if(_e===null){var e=oe.alternate;e=e!==null?e.memoizedState:null}else e=_e.next;var t=Ye===null?oe.memoizedState:Ye.next;if(t!==null)Ye=t,_e=e;else{if(e===null)throw oe.alternate===null?Error(U(467)):Error(U(310));_e=e,e={memoizedState:_e.memoizedState,baseState:_e.baseState,baseQueue:_e.baseQueue,queue:_e.queue,next:null},Ye===null?oe.memoizedState=Ye=e:Ye=Ye.next=e}return Ye}function zf(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function Bo(e){var t=$o;return $o+=1,As===null&&(As=[]),e=Xy(As,e,t),t=oe,(Ye===null?t.memoizedState:Ye.next)===null&&(t=t.alternate,te.H=t===null||t.memoizedState===null?Tb:Ab),e}function ec(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return Bo(e);if(e.$$typeof===on)return wt(e)}throw Error(U(438,String(e)))}function qf(e){var t=null,a=oe.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=oe.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=zf(),oe.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=FR;return t.index++,a}function hn(e,t){return typeof t=="function"?t(e):t}function cu(e){var t=Je();return If(t,_e,e)}function If(e,t,a){var n=e.queue;if(n===null)throw Error(U(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,l=null,c=t,d=!1;do{var m=c.lane&-536870913;if(m!==c.lane?(de&m)===m:(er&m)===m){var f=c.revertLane;if(f===0)l!==null&&(l=l.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),m===Us&&(d=!0);else if((er&f)===f){c=c.next,f===Us&&(d=!0);continue}else m={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},l===null?(o=l=m,i=s):l=l.next=m,oe.lanes|=f,ar|=f;m=c.action,Tr&&a(s,m),s=c.hasEagerState?c.eagerState:a(s,m)}else f={lane:m,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},l===null?(o=l=f,i=s):l=l.next=f,oe.lanes|=m,ar|=m;c=c.next}while(c!==null&&c!==t);if(l===null?i=s:l.next=o,!Wt(s,e.memoizedState)&&(ct=!0,d&&(a=Es,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=l,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function am(e){var t=Je(),a=t.queue;if(a===null)throw Error(U(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);Wt(s,t.memoizedState)||(ct=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function ab(e,t,a){var n=oe,r=Je(),s=ge;if(s){if(a===void 0)throw Error(U(407));a=a()}else a=t();var i=!Wt((_e||r).memoizedState,a);i&&(r.memoizedState=a,ct=!0),r=r.queue;var o=sb.bind(null,n,r,e);if(zo(2048,8,o,[e]),r.getSnapshot!==t||i||Ye!==null&&Ye.memoizedState.tag&1){if(n.flags|=2048,Fs(9,tc(),rb.bind(null,n,r,a,t),null),Re===null)throw Error(U(349));s||(er&124)!==0||nb(n,t,a)}return a}function nb(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=oe.updateQueue,t===null?(t=zf(),oe.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function rb(e,t,a,n){t.value=a,t.getSnapshot=n,ib(t)&&ob(e)}function sb(e,t,a){return a(function(){ib(t)&&ob(e)})}function ib(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!Wt(e,a)}catch{return!0}}function ob(e){var t=Vs(e,2);t!==null&&Zt(t,e,2)}function Km(e){var t=Pt();if(typeof e=="function"){var a=e;if(e=a(),Tr){qn(!0);try{a()}finally{qn(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:hn,lastRenderedState:e},t}function lb(e,t,a,n){return e.baseState=a,If(e,_e,typeof n=="function"?n:hn)}function aE(e,t,a,n,r){if(ac(e))throw Error(U(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};te.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,ub(t,s)):(s.next=a.next,t.pending=a.next=s)}}function ub(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=te.T,i={};te.T=i;try{var o=a(r,n),l=te.S;l!==null&&l(i,o),gg(e,t,o)}catch(c){Hm(e,t,c)}finally{te.T=s}}else try{s=a(r,n),gg(e,t,s)}catch(c){Hm(e,t,c)}}function gg(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){yg(e,t,n)},function(n){return Hm(e,t,n)}):yg(e,t,a)}function yg(e,t,a){t.status="fulfilled",t.value=a,cb(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,ub(e,a)))}function Hm(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,cb(t),t=t.next;while(t!==n)}e.action=null}function cb(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function db(e,t){return t}function bg(e,t){if(ge){var a=Re.formState;if(a!==null){e:{var n=oe;if(ge){if(ze){t:{for(var r=ze,s=Ua;r.nodeType!==8;){if(!s){r=null;break t}if(r=Sa(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){ze=Sa(r.nextSibling),n=r.data==="F!";break e}}Cr(n)}n=!1}n&&(t=a[0])}}return a=Pt(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:db,lastRenderedState:t},a.queue=n,a=Rb.bind(null,oe,n),n.dispatch=a,n=Km(!1),s=Vf.bind(null,oe,!1,n.queue),n=Pt(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=aE.bind(null,oe,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function xg(e){var t=Je();return mb(t,_e,e)}function mb(e,t,a){if(t=If(e,t,db)[0],e=cu(hn)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=Bo(t)}catch(i){throw i===Fo?Wu:i}else n=t;t=Je();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(oe.flags|=2048,Fs(9,tc(),nE.bind(null,r,a),null)),[n,s,e]}function nE(e,t){e.action=t}function $g(e){var t=Je(),a=_e;if(a!==null)return mb(t,a,e);Je(),t=t.memoizedState,a=Je();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function Fs(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=oe.updateQueue,t===null&&(t=zf(),oe.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function tc(){return{destroy:void 0,resource:void 0}}function fb(){return Je().memoizedState}function du(e,t,a,n){var r=Pt();n=n===void 0?null:n,oe.flags|=e,r.memoizedState=Fs(1|t,tc(),a,n)}function zo(e,t,a,n){var r=Je();n=n===void 0?null:n;var s=r.memoizedState.inst;_e!==null&&n!==null&&Pf(n,_e.memoizedState.deps)?r.memoizedState=Fs(t,s,a,n):(oe.flags|=e,r.memoizedState=Fs(1|t,s,a,n))}function wg(e,t){du(8390656,8,e,t)}function pb(e,t){zo(2048,8,e,t)}function hb(e,t){return zo(4,2,e,t)}function vb(e,t){return zo(4,4,e,t)}function gb(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function yb(e,t,a){a=a!=null?a.concat([e]):null,zo(4,4,gb.bind(null,t,e),a)}function Kf(){}function bb(e,t){var a=Je();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&Pf(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function xb(e,t){var a=Je();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&Pf(t,n[1]))return n[0];if(n=e(),Tr){qn(!0);try{e()}finally{qn(!1)}}return a.memoizedState=[n,t],n}function Hf(e,t,a){return a===void 0||(er&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=c0(),oe.lanes|=e,ar|=e,a)}function $b(e,t,a,n){return Wt(a,t)?a:js.current!==null?(e=Hf(e,a,n),Wt(e,t)||(ct=!0),e):(er&42)===0?(ct=!0,e.memoizedState=a):(e=c0(),oe.lanes|=e,ar|=e,t)}function wb(e,t,a,n,r){var s=ye.p;ye.p=s!==0&&8>s?s:8;var i=te.T,o={};te.T=o,Vf(e,!1,t,a);try{var l=r(),c=te.S;if(c!==null&&c(o,l),l!==null&&typeof l=="object"&&typeof l.then=="function"){var d=WC(l,n);uo(e,t,d,Xt(e))}else uo(e,t,n,Xt(e))}catch(m){uo(e,t,{then:function(){},status:"rejected",reason:m},Xt())}finally{ye.p=s,te.T=i}}function rE(){}function Qm(e,t,a,n){if(e.tag!==5)throw Error(U(476));var r=Sb(e).queue;wb(e,r,t,$r,a===null?rE:function(){return Nb(e),a(n)})}function Sb(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:$r,baseState:$r,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:hn,lastRenderedState:$r},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:hn,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function Nb(e){var t=Sb(e).next.queue;uo(e,t,{},Xt())}function Qf(){return wt(ko)}function _b(){return Je().memoizedState}function kb(){return Je().memoizedState}function sE(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=Xt();e=Vn(a);var n=Gn(t,e,a);n!==null&&(Zt(n,t,a),io(n,t,a)),t={cache:Df()},e.payload=t;return}t=t.return}}function iE(e,t,a){var n=Xt();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},ac(e)?Cb(t,a):(a=Cf(e,t,a,n),a!==null&&(Zt(a,e,n),Eb(a,t,n)))}function Rb(e,t,a){var n=Xt();uo(e,t,a,n)}function uo(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(ac(e))Cb(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,Wt(o,i))return Zu(e,t,r,0),Re===null&&Xu(),!1}catch{}finally{}if(a=Cf(e,t,r,n),a!==null)return Zt(a,e,n),Eb(a,t,n),!0}return!1}function Vf(e,t,a,n){if(n={lane:2,revertLane:tp(),action:n,hasEagerState:!1,eagerState:null,next:null},ac(e)){if(t)throw Error(U(479))}else t=Cf(e,a,n,2),t!==null&&Zt(t,e,2)}function ac(e){var t=e.alternate;return e===oe||t!==null&&t===oe}function Cb(e,t){Ts=Cu=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function Eb(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,by(e,a)}}var Tu={readContext:wt,use:ec,useCallback:He,useContext:He,useEffect:He,useImperativeHandle:He,useLayoutEffect:He,useInsertionEffect:He,useMemo:He,useReducer:He,useRef:He,useState:He,useDebugValue:He,useDeferredValue:He,useTransition:He,useSyncExternalStore:He,useId:He,useHostTransitionStatus:He,useFormState:He,useActionState:He,useOptimistic:He,useMemoCache:He,useCacheRefresh:He},Tb={readContext:wt,use:ec,useCallback:function(e,t){return Pt().memoizedState=[e,t===void 0?null:t],e},useContext:wt,useEffect:wg,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,du(4194308,4,gb.bind(null,t,e),a)},useLayoutEffect:function(e,t){return du(4194308,4,e,t)},useInsertionEffect:function(e,t){du(4,2,e,t)},useMemo:function(e,t){var a=Pt();t=t===void 0?null:t;var n=e();if(Tr){qn(!0);try{e()}finally{qn(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=Pt();if(a!==void 0){var r=a(t);if(Tr){qn(!0);try{a(t)}finally{qn(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=iE.bind(null,oe,e),[n.memoizedState,e]},useRef:function(e){var t=Pt();return e={current:e},t.memoizedState=e},useState:function(e){e=Km(e);var t=e.queue,a=Rb.bind(null,oe,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:Kf,useDeferredValue:function(e,t){var a=Pt();return Hf(a,e,t)},useTransition:function(){var e=Km(!1);return e=wb.bind(null,oe,e.queue,!0,!1),Pt().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=oe,r=Pt();if(ge){if(a===void 0)throw Error(U(407));a=a()}else{if(a=t(),Re===null)throw Error(U(349));(de&124)!==0||nb(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,wg(sb.bind(null,n,s,e),[e]),n.flags|=2048,Fs(9,tc(),rb.bind(null,n,s,a,t),null),a},useId:function(){var e=Pt(),t=Re.identifierPrefix;if(ge){var a=un,n=ln;a=(n&~(1<<32-Jt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=Eu++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=eE++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:Qf,useFormState:bg,useActionState:bg,useOptimistic:function(e){var t=Pt();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=Vf.bind(null,oe,!0,a),a.dispatch=t,[e,t]},useMemoCache:qf,useCacheRefresh:function(){return Pt().memoizedState=sE.bind(null,oe)}},Ab={readContext:wt,use:ec,useCallback:bb,useContext:wt,useEffect:pb,useImperativeHandle:yb,useInsertionEffect:hb,useLayoutEffect:vb,useMemo:xb,useReducer:cu,useRef:fb,useState:function(){return cu(hn)},useDebugValue:Kf,useDeferredValue:function(e,t){var a=Je();return $b(a,_e.memoizedState,e,t)},useTransition:function(){var e=cu(hn)[0],t=Je().memoizedState;return[typeof e=="boolean"?e:Bo(e),t]},useSyncExternalStore:ab,useId:_b,useHostTransitionStatus:Qf,useFormState:xg,useActionState:xg,useOptimistic:function(e,t){var a=Je();return lb(a,_e,e,t)},useMemoCache:qf,useCacheRefresh:kb},oE={readContext:wt,use:ec,useCallback:bb,useContext:wt,useEffect:pb,useImperativeHandle:yb,useInsertionEffect:hb,useLayoutEffect:vb,useMemo:xb,useReducer:am,useRef:fb,useState:function(){return am(hn)},useDebugValue:Kf,useDeferredValue:function(e,t){var a=Je();return _e===null?Hf(a,e,t):$b(a,_e.memoizedState,e,t)},useTransition:function(){var e=am(hn)[0],t=Je().memoizedState;return[typeof e=="boolean"?e:Bo(e),t]},useSyncExternalStore:ab,useId:_b,useHostTransitionStatus:Qf,useFormState:$g,useActionState:$g,useOptimistic:function(e,t){var a=Je();return _e!==null?lb(a,_e,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:qf,useCacheRefresh:kb},Ds=null,wo=0;function Jl(e){var t=wo;return wo+=1,Ds===null&&(Ds=[]),Xy(Ds,e,t)}function Qi(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function Xl(e,t){throw t.$$typeof===UR?Error(U(525)):(e=Object.prototype.toString.call(t),Error(U(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function Sg(e){var t=e._init;return t(e._payload)}function Db(e){function t(g,v){if(e){var b=g.deletions;b===null?(g.deletions=[v],g.flags|=16):b.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=mn(g,v),g.index=0,g.sibling=null,g}function s(g,v,b){return g.index=b,e?(b=g.alternate,b!==null?(b=b.index,b<v?(g.flags|=67108866,v):b):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,b,w){return v===null||v.tag!==6?(v=Wd(b,g.mode,w),v.return=g,v):(v=r(v,b),v.return=g,v)}function l(g,v,b,w){var S=b.type;return S===ps?d(g,v,b.props.children,w,b.key):v!==null&&(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Pn&&Sg(S)===v.type)?(v=r(v,b.props),Qi(v,b),v.return=g,v):(v=lu(b.type,b.key,b.props,null,g.mode,w),Qi(v,b),v.return=g,v)}function c(g,v,b,w){return v===null||v.tag!==4||v.stateNode.containerInfo!==b.containerInfo||v.stateNode.implementation!==b.implementation?(v=em(b,g.mode,w),v.return=g,v):(v=r(v,b.children||[]),v.return=g,v)}function d(g,v,b,w,S){return v===null||v.tag!==7?(v=wr(b,g.mode,w,S),v.return=g,v):(v=r(v,b),v.return=g,v)}function m(g,v,b){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=Wd(""+v,g.mode,b),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case zl:return b=lu(v.type,v.key,v.props,null,g.mode,b),Qi(b,v),b.return=g,b;case Xi:return v=em(v,g.mode,b),v.return=g,v;case Pn:var w=v._init;return v=w(v._payload),m(g,v,b)}if(Zi(v)||Ii(v))return v=wr(v,g.mode,b,null),v.return=g,v;if(typeof v.then=="function")return m(g,Jl(v),b);if(v.$$typeof===on)return m(g,Gl(g,v),b);Xl(g,v)}return null}function f(g,v,b,w){var S=v!==null?v.key:null;if(typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint")return S!==null?null:o(g,v,""+b,w);if(typeof b=="object"&&b!==null){switch(b.$$typeof){case zl:return b.key===S?l(g,v,b,w):null;case Xi:return b.key===S?c(g,v,b,w):null;case Pn:return S=b._init,b=S(b._payload),f(g,v,b,w)}if(Zi(b)||Ii(b))return S!==null?null:d(g,v,b,w,null);if(typeof b.then=="function")return f(g,v,Jl(b),w);if(b.$$typeof===on)return f(g,v,Gl(g,b),w);Xl(g,b)}return null}function p(g,v,b,w,S){if(typeof w=="string"&&w!==""||typeof w=="number"||typeof w=="bigint")return g=g.get(b)||null,o(v,g,""+w,S);if(typeof w=="object"&&w!==null){switch(w.$$typeof){case zl:return g=g.get(w.key===null?b:w.key)||null,l(v,g,w,S);case Xi:return g=g.get(w.key===null?b:w.key)||null,c(v,g,w,S);case Pn:var C=w._init;return w=C(w._payload),p(g,v,b,w,S)}if(Zi(w)||Ii(w))return g=g.get(b)||null,d(v,g,w,S,null);if(typeof w.then=="function")return p(g,v,b,Jl(w),S);if(w.$$typeof===on)return p(g,v,b,Gl(v,w),S);Xl(v,w)}return null}function x(g,v,b,w){for(var S=null,C=null,_=v,A=v=0,L=null;_!==null&&A<b.length;A++){_.index>A?(L=_,_=null):L=_.sibling;var M=f(g,_,b[A],w);if(M===null){_===null&&(_=L);break}e&&_&&M.alternate===null&&t(g,_),v=s(M,v,A),C===null?S=M:C.sibling=M,C=M,_=L}if(A===b.length)return a(g,_),ge&&br(g,A),S;if(_===null){for(;A<b.length;A++)_=m(g,b[A],w),_!==null&&(v=s(_,v,A),C===null?S=_:C.sibling=_,C=_);return ge&&br(g,A),S}for(_=n(_);A<b.length;A++)L=p(_,g,A,b[A],w),L!==null&&(e&&L.alternate!==null&&_.delete(L.key===null?A:L.key),v=s(L,v,A),C===null?S=L:C.sibling=L,C=L);return e&&_.forEach(function(P){return t(g,P)}),ge&&br(g,A),S}function y(g,v,b,w){if(b==null)throw Error(U(151));for(var S=null,C=null,_=v,A=v=0,L=null,M=b.next();_!==null&&!M.done;A++,M=b.next()){_.index>A?(L=_,_=null):L=_.sibling;var P=f(g,_,M.value,w);if(P===null){_===null&&(_=L);break}e&&_&&P.alternate===null&&t(g,_),v=s(P,v,A),C===null?S=P:C.sibling=P,C=P,_=L}if(M.done)return a(g,_),ge&&br(g,A),S;if(_===null){for(;!M.done;A++,M=b.next())M=m(g,M.value,w),M!==null&&(v=s(M,v,A),C===null?S=M:C.sibling=M,C=M);return ge&&br(g,A),S}for(_=n(_);!M.done;A++,M=b.next())M=p(_,g,A,M.value,w),M!==null&&(e&&M.alternate!==null&&_.delete(M.key===null?A:M.key),v=s(M,v,A),C===null?S=M:C.sibling=M,C=M);return e&&_.forEach(function(k){return t(g,k)}),ge&&br(g,A),S}function $(g,v,b,w){if(typeof b=="object"&&b!==null&&b.type===ps&&b.key===null&&(b=b.props.children),typeof b=="object"&&b!==null){switch(b.$$typeof){case zl:e:{for(var S=b.key;v!==null;){if(v.key===S){if(S=b.type,S===ps){if(v.tag===7){a(g,v.sibling),w=r(v,b.props.children),w.return=g,g=w;break e}}else if(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Pn&&Sg(S)===v.type){a(g,v.sibling),w=r(v,b.props),Qi(w,b),w.return=g,g=w;break e}a(g,v);break}else t(g,v);v=v.sibling}b.type===ps?(w=wr(b.props.children,g.mode,w,b.key),w.return=g,g=w):(w=lu(b.type,b.key,b.props,null,g.mode,w),Qi(w,b),w.return=g,g=w)}return i(g);case Xi:e:{for(S=b.key;v!==null;){if(v.key===S)if(v.tag===4&&v.stateNode.containerInfo===b.containerInfo&&v.stateNode.implementation===b.implementation){a(g,v.sibling),w=r(v,b.children||[]),w.return=g,g=w;break e}else{a(g,v);break}else t(g,v);v=v.sibling}w=em(b,g.mode,w),w.return=g,g=w}return i(g);case Pn:return S=b._init,b=S(b._payload),$(g,v,b,w)}if(Zi(b))return x(g,v,b,w);if(Ii(b)){if(S=Ii(b),typeof S!="function")throw Error(U(150));return b=S.call(b),y(g,v,b,w)}if(typeof b.then=="function")return $(g,v,Jl(b),w);if(b.$$typeof===on)return $(g,v,Gl(g,b),w);Xl(g,b)}return typeof b=="string"&&b!==""||typeof b=="number"||typeof b=="bigint"?(b=""+b,v!==null&&v.tag===6?(a(g,v.sibling),w=r(v,b),w.return=g,g=w):(a(g,v),w=Wd(b,g.mode,w),w.return=g,g=w),i(g)):a(g,v)}return function(g,v,b,w){try{wo=0;var S=$(g,v,b,w);return Ds=null,S}catch(_){if(_===Fo||_===Wu)throw _;var C=Gt(29,_,null,g.mode);return C.lanes=w,C.return=g,C}finally{}}}var Bs=Db(!0),Mb=Db(!1),va=qa(null),za=null;function Fn(e){var t=e.alternate;Le(nt,nt.current&1),Le(va,e),za===null&&(t===null||js.current!==null||t.memoizedState!==null)&&(za=e)}function Ob(e){if(e.tag===22){if(Le(nt,nt.current),Le(va,e),za===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(za=e)}}else Bn(e)}function Bn(){Le(nt,nt.current),Le(va,va.current)}function dn(e){dt(va),za===e&&(za=null),dt(nt)}var nt=qa(0);function Au(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||cf(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function nm(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:Ae({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var Vm={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=Xt(),r=Vn(n);r.payload=t,a!=null&&(r.callback=a),t=Gn(e,r,n),t!==null&&(Zt(t,e,n),io(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=Xt(),r=Vn(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=Gn(e,r,n),t!==null&&(Zt(t,e,n),io(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=Xt(),n=Vn(a);n.tag=2,t!=null&&(n.callback=t),t=Gn(e,n,a),t!==null&&(Zt(t,e,a),io(t,e,a))}};function Ng(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!bo(a,n)||!bo(r,s):!0}function _g(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&Vm.enqueueReplaceState(t,t.state,null)}function Ar(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=Ae({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var Du=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function Lb(e){Du(e)}function Pb(e){console.error(e)}function Ub(e){Du(e)}function Mu(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function kg(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function Gm(e,t,a){return a=Vn(a),a.tag=3,a.payload={element:null},a.callback=function(){Mu(e,t)},a}function jb(e){return e=Vn(e),e.tag=3,e}function Fb(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){kg(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){kg(t,a,n),typeof r!="function"&&(Yn===null?Yn=new Set([this]):Yn.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function lE(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&Uo(t,a,r,!0),a=va.current,a!==null){switch(a.tag){case 13:return za===null?af():a.alternate===null&&qe===0&&(qe=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===Bm?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),pm(e,n,r)),!1;case 22:return a.flags|=65536,n===Bm?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),pm(e,n,r)),!1}throw Error(U(435,a.tag))}return pm(e,n,r),af(),!1}if(ge)return t=va.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==Lm&&(e=Error(U(422),{cause:n}),xo(pa(e,a)))):(n!==Lm&&(t=Error(U(423),{cause:n}),xo(pa(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=pa(n,a),r=Gm(e.stateNode,n,r),tm(e,r),qe!==4&&(qe=2)),!1;var s=Error(U(520),{cause:n});if(s=pa(s,a),fo===null?fo=[s]:fo.push(s),qe!==4&&(qe=2),t===null)return!0;n=pa(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=Gm(a.stateNode,n,e),tm(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(Yn===null||!Yn.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=jb(r),Fb(r,e,a,n),tm(a,r),!1}a=a.return}while(a!==null);return!1}var Bb=Error(U(461)),ct=!1;function pt(e,t,a,n){t.child=e===null?Mb(t,null,a,n):Bs(t,e.child,a,n)}function Rg(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return Er(t),n=Uf(e,t,a,i,s,r),o=jf(),e!==null&&!ct?(Ff(e,t,r),vn(e,t,r)):(ge&&o&&Tf(t),t.flags|=1,pt(e,t,n,r),t.child)}function Cg(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!Ef(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,zb(e,t,s,n,r)):(e=lu(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!Gf(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:bo,a(i,n)&&e.ref===t.ref)return vn(e,t,r)}return t.flags|=1,e=mn(s,n),e.ref=t.ref,e.return=t,t.child=e}function zb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if(bo(s,n)&&e.ref===t.ref)if(ct=!1,t.pendingProps=n=s,Gf(e,r))(e.flags&131072)!==0&&(ct=!0);else return t.lanes=e.lanes,vn(e,t,r)}return Ym(e,t,a,n,r)}function qb(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return Eg(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&uu(t,s!==null?s.cachePool:null),s!==null?vg(t,s):Im(),Ob(t);else return t.lanes=t.childLanes=536870912,Eg(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(uu(t,s.cachePool),vg(t,s),Bn(t),t.memoizedState=null):(e!==null&&uu(t,null),Im(),Bn(t));return pt(e,t,r,a),t.child}function Eg(e,t,a,n){var r=Mf();return r=r===null?null:{parent:at._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&uu(t,null),Im(),Ob(t),e!==null&&Uo(e,t,n,!0),null}function mu(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(U(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function Ym(e,t,a,n,r){return Er(t),a=Uf(e,t,a,n,void 0,r),n=jf(),e!==null&&!ct?(Ff(e,t,r),vn(e,t,r)):(ge&&n&&Tf(t),t.flags|=1,pt(e,t,a,r),t.child)}function Tg(e,t,a,n,r,s){return Er(t),t.updateQueue=null,a=tb(t,n,a,r),eb(e),n=jf(),e!==null&&!ct?(Ff(e,t,s),vn(e,t,s)):(ge&&n&&Tf(t),t.flags|=1,pt(e,t,a,s),t.child)}function Ag(e,t,a,n,r){if(Er(t),t.stateNode===null){var s=ws,i=a.contextType;typeof i=="object"&&i!==null&&(s=wt(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=Vm,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},Of(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?wt(i):ws,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(nm(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&Vm.enqueueReplaceState(s,s.state,null),lo(t,n,s,r),oo(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,l=Ar(a,o);s.props=l;var c=s.context,d=a.contextType;i=ws,typeof d=="object"&&d!==null&&(i=wt(d));var m=a.getDerivedStateFromProps;d=typeof m=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&_g(t,s,n,i),Un=!1;var f=t.memoizedState;s.state=f,lo(t,n,s,r),oo(),c=t.memoizedState,o||f!==c||Un?(typeof m=="function"&&(nm(t,a,m,n),c=t.memoizedState),(l=Un||Ng(t,a,l,n,f,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=l):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,zm(e,t),i=t.memoizedProps,d=Ar(a,i),s.props=d,m=t.pendingProps,f=s.context,c=a.contextType,l=ws,typeof c=="object"&&c!==null&&(l=wt(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==m||f!==l)&&_g(t,s,n,l),Un=!1,f=t.memoizedState,s.state=f,lo(t,n,s,r),oo();var p=t.memoizedState;i!==m||f!==p||Un||e!==null&&e.dependencies!==null&&ku(e.dependencies)?(typeof o=="function"&&(nm(t,a,o,n),p=t.memoizedState),(d=Un||Ng(t,a,d,n,f,p,l)||e!==null&&e.dependencies!==null&&ku(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,p,l),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,p,l)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=p),s.props=n,s.state=p,s.context=l,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&f===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,mu(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=Bs(t,e.child,null,r),t.child=Bs(t,null,a,r)):pt(e,t,a,r),t.memoizedState=s.state,e=t.child):e=vn(e,t,r),e}function Dg(e,t,a,n){return Po(),t.flags|=256,pt(e,t,a,n),t.child}var rm={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function sm(e){return{baseLanes:e,cachePool:Yy()}}function im(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=ha),e}function Ib(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(nt.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(ge){if(r?Fn(t):Bn(t),ge){var o=ze,l;if(l=o){e:{for(l=o,o=Ua;l.nodeType!==8;){if(!o){o=null;break e}if(l=Sa(l.nextSibling),l===null){o=null;break e}}o=l}o!==null?(t.memoizedState={dehydrated:o,treeContext:Sr!==null?{id:ln,overflow:un}:null,retryLane:536870912,hydrationErrors:null},l=Gt(18,null,null,0),l.stateNode=o,l.return=t,t.child=l,Rt=t,ze=null,l=!0):l=!1}l||Cr(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return cf(o)?t.lanes=32:t.lanes=536870912,null;dn(t)}return o=n.children,n=n.fallback,r?(Bn(t),r=t.mode,o=Ou({mode:"hidden",children:o},r),n=wr(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=sm(a),r.childLanes=im(e,i,a),t.memoizedState=rm,n):(Fn(t),Jm(t,o))}if(l=e.memoizedState,l!==null&&(o=l.dehydrated,o!==null)){if(s)t.flags&256?(Fn(t),t.flags&=-257,t=om(e,t,a)):t.memoizedState!==null?(Bn(t),t.child=e.child,t.flags|=128,t=null):(Bn(t),r=n.fallback,o=t.mode,n=Ou({mode:"visible",children:n.children},o),r=wr(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,Bs(t,e.child,null,a),n=t.child,n.memoizedState=sm(a),n.childLanes=im(e,i,a),t.memoizedState=rm,t=r);else if(Fn(t),cf(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(U(419)),n.stack="",n.digest=i,xo({value:n,source:null,stack:null}),t=om(e,t,a)}else if(ct||Uo(e,t,a,!1),i=(a&e.childLanes)!==0,ct||i){if(i=Re,i!==null&&(n=a&-a,n=(n&42)!==0?1:yf(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==l.retryLane))throw l.retryLane=n,Vs(e,n),Zt(i,e,n),Bb;o.data==="$?"||af(),t=om(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=l.treeContext,ze=Sa(o.nextSibling),Rt=t,ge=!0,Nr=null,Ua=!1,e!==null&&(da[ma++]=ln,da[ma++]=un,da[ma++]=Sr,ln=e.id,un=e.overflow,Sr=t),t=Jm(t,n.children),t.flags|=4096);return t}return r?(Bn(t),r=n.fallback,o=t.mode,l=e.child,c=l.sibling,n=mn(l,{mode:"hidden",children:n.children}),n.subtreeFlags=l.subtreeFlags&65011712,c!==null?r=mn(c,r):(r=wr(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=sm(a):(l=o.cachePool,l!==null?(c=at._currentValue,l=l.parent!==c?{parent:c,pool:c}:l):l=Yy(),o={baseLanes:o.baseLanes|a,cachePool:l}),r.memoizedState=o,r.childLanes=im(e,i,a),t.memoizedState=rm,n):(Fn(t),a=e.child,e=a.sibling,a=mn(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function Jm(e,t){return t=Ou({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function Ou(e,t){return e=Gt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function om(e,t,a){return Bs(t,e.child,null,a),e=Jm(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function Mg(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),Um(e.return,t,a)}function lm(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function Kb(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(pt(e,t,n.children,a),n=nt.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&Mg(e,a,t);else if(e.tag===19)Mg(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Le(nt,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&Au(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),lm(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&Au(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}lm(t,!0,a,null,s);break;case"together":lm(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function vn(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),ar|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(Uo(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(U(153));if(t.child!==null){for(e=t.child,a=mn(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=mn(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function Gf(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&ku(e)))}function uE(e,t,a){switch(t.tag){case 3:yu(t,t.stateNode.containerInfo),jn(t,at,e.memoizedState.cache),Po();break;case 27:case 5:_m(t);break;case 4:yu(t,t.stateNode.containerInfo);break;case 10:jn(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(Fn(t),t.flags|=128,null):(a&t.child.childLanes)!==0?Ib(e,t,a):(Fn(t),e=vn(e,t,a),e!==null?e.sibling:null);Fn(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(Uo(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return Kb(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Le(nt,nt.current),n)break;return null;case 22:case 23:return t.lanes=0,qb(e,t,a);case 24:jn(t,at,e.memoizedState.cache)}return vn(e,t,a)}function Hb(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)ct=!0;else{if(!Gf(e,a)&&(t.flags&128)===0)return ct=!1,uE(e,t,a);ct=(e.flags&131072)!==0}else ct=!1,ge&&(t.flags&1048576)!==0&&Vy(t,_u,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")Ef(n)?(e=Ar(n,e),t.tag=1,t=Ag(null,t,n,e,a)):(t.tag=0,t=Ym(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===hf){t.tag=11,t=Rg(null,t,n,e,a);break e}else if(r===vf){t.tag=14,t=Cg(null,t,n,e,a);break e}}throw t=Sm(n)||n,Error(U(306,t,""))}}return t;case 0:return Ym(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Ar(n,t.pendingProps),Ag(e,t,n,r,a);case 3:e:{if(yu(t,t.stateNode.containerInfo),e===null)throw Error(U(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,zm(e,t),lo(t,n,null,a);var i=t.memoizedState;if(n=i.cache,jn(t,at,n),n!==s.cache&&jm(t,[at],a,!0),oo(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=Dg(e,t,n,a);break e}else if(n!==r){r=pa(Error(U(424)),t),xo(r),t=Dg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(ze=Sa(e.firstChild),Rt=t,ge=!0,Nr=null,Ua=!0,a=Mb(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if(Po(),n===r){t=vn(e,t,a);break e}pt(e,t,n,a)}t=t.child}return t;case 26:return mu(e,t),e===null?(a=Zg(t.type,null,t.pendingProps,null))?t.memoizedState=a:ge||(a=t.type,e=t.pendingProps,n=zu(Qn.current).createElement(a),n[$t]=t,n[Ft]=e,vt(n,a,e),ut(n),t.stateNode=n):t.memoizedState=Zg(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return _m(t),e===null&&ge&&(n=t.stateNode=A0(t.type,t.pendingProps,Qn.current),Rt=t,Ua=!0,r=ze,rr(t.type)?(df=r,ze=Sa(n.firstChild)):ze=r),pt(e,t,t.pendingProps.children,a),mu(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&ge&&((r=n=ze)&&(n=LE(n,t.type,t.pendingProps,Ua),n!==null?(t.stateNode=n,Rt=t,ze=Sa(n.firstChild),Ua=!1,r=!0):r=!1),r||Cr(t)),_m(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,lf(r,s)?n=null:i!==null&&lf(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=Uf(e,t,tE,null,null,a),ko._currentValue=r),mu(e,t),pt(e,t,n,a),t.child;case 6:return e===null&&ge&&((e=a=ze)&&(a=PE(a,t.pendingProps,Ua),a!==null?(t.stateNode=a,Rt=t,ze=null,e=!0):e=!1),e||Cr(t)),null;case 13:return Ib(e,t,a);case 4:return yu(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=Bs(t,null,n,a):pt(e,t,n,a),t.child;case 11:return Rg(e,t,t.type,t.pendingProps,a);case 7:return pt(e,t,t.pendingProps,a),t.child;case 8:return pt(e,t,t.pendingProps.children,a),t.child;case 12:return pt(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,jn(t,t.type,n.value),pt(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,Er(t),r=wt(r),n=n(r),t.flags|=1,pt(e,t,n,a),t.child;case 14:return Cg(e,t,t.type,t.pendingProps,a);case 15:return zb(e,t,t.type,t.pendingProps,a);case 19:return Kb(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=Ou(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=mn(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return qb(e,t,a);case 24:return Er(t),n=wt(at),e===null?(r=Mf(),r===null&&(r=Re,s=Df(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},Of(t),jn(t,at,r)):((e.lanes&a)!==0&&(zm(e,t),lo(t,null,null,a),oo()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),jn(t,at,n)):(n=s.cache,jn(t,at,n),n!==r.cache&&jm(t,[at],a,!0))),pt(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(U(156,t.tag))}function nn(e){e.flags|=4}function Og(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!O0(t)){if(t=va.current,t!==null&&((de&4194048)===de?za!==null:(de&62914560)!==de&&(de&536870912)===0||t!==za))throw so=Bm,Jy;e.flags|=8192}}function Zl(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?gy():536870912,e.lanes|=t,zs|=t)}function Vi(e,t){if(!ge)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function Fe(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function cE(e,t,a){var n=t.pendingProps;switch(Af(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return Fe(t),null;case 1:return Fe(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),fn(at),Os(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Hi(t)?nn(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,cg())),Fe(t),null;case 26:return a=t.memoizedState,e===null?(nn(t),a!==null?(Fe(t),Og(t,a)):(Fe(t),t.flags&=-16777217)):a?a!==e.memoizedState?(nn(t),Fe(t),Og(t,a)):(Fe(t),t.flags&=-16777217):(e.memoizedProps!==n&&nn(t),Fe(t),t.flags&=-16777217),null;case 27:bu(t),a=Qn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&nn(t);else{if(!n){if(t.stateNode===null)throw Error(U(166));return Fe(t),null}e=Fa.current,Hi(t)?lg(t,e):(e=A0(r,n,a),t.stateNode=e,nn(t))}return Fe(t),null;case 5:if(bu(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&nn(t);else{if(!n){if(t.stateNode===null)throw Error(U(166));return Fe(t),null}if(e=Fa.current,Hi(t))lg(t,e);else{switch(r=zu(Qn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[$t]=t,e[Ft]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(vt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&nn(t)}}return Fe(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&nn(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(U(166));if(e=Qn.current,Hi(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=Rt,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[$t]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||C0(e.nodeValue,a)),e||Cr(t)}else e=zu(e).createTextNode(n),e[$t]=t,t.stateNode=e}return Fe(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Hi(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(U(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(U(317));r[$t]=t}else Po(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;Fe(t),r=!1}else r=cg(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(dn(t),t):(dn(t),null)}if(dn(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),Zl(t,t.updateQueue),Fe(t),null;case 4:return Os(),e===null&&ap(t.stateNode.containerInfo),Fe(t),null;case 10:return fn(t.type),Fe(t),null;case 19:if(dt(nt),r=t.memoizedState,r===null)return Fe(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)Vi(r,!1);else{if(qe!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=Au(e),s!==null){for(t.flags|=128,Vi(r,!1),e=s.updateQueue,t.updateQueue=e,Zl(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)Qy(a,e),a=a.sibling;return Le(nt,nt.current&1|2),t.child}e=e.sibling}r.tail!==null&&Ba()>Pu&&(t.flags|=128,n=!0,Vi(r,!1),t.lanes=4194304)}else{if(!n)if(e=Au(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,Zl(t,e),Vi(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!ge)return Fe(t),null}else 2*Ba()-r.renderingStartTime>Pu&&a!==536870912&&(t.flags|=128,n=!0,Vi(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=Ba(),t.sibling=null,e=nt.current,Le(nt,n?e&1|2:e&1),t):(Fe(t),null);case 22:case 23:return dn(t),Lf(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(Fe(t),t.subtreeFlags&6&&(t.flags|=8192)):Fe(t),a=t.updateQueue,a!==null&&Zl(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&dt(_r),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),fn(at),Fe(t),null;case 25:return null;case 30:return null}throw Error(U(156,t.tag))}function dE(e,t){switch(Af(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return fn(at),Os(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return bu(t),null;case 13:if(dn(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(U(340));Po()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return dt(nt),null;case 4:return Os(),null;case 10:return fn(t.type),null;case 22:case 23:return dn(t),Lf(),e!==null&&dt(_r),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return fn(at),null;case 25:return null;default:return null}}function Qb(e,t){switch(Af(t),t.tag){case 3:fn(at),Os();break;case 26:case 27:case 5:bu(t);break;case 4:Os();break;case 13:dn(t);break;case 19:dt(nt);break;case 10:fn(t.type);break;case 22:case 23:dn(t),Lf(),e!==null&&dt(_r);break;case 24:fn(at)}}function qo(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){ke(t,t.return,o)}}function tr(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var l=a,c=o;try{c()}catch(d){ke(r,l,d)}}}n=n.next}while(n!==s)}}catch(d){ke(t,t.return,d)}}function Vb(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{Wy(t,a)}catch(n){ke(e,e.return,n)}}}function Gb(e,t,a){a.props=Ar(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){ke(e,t,n)}}function co(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){ke(e,t,r)}}function ja(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){ke(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){ke(e,t,r)}else a.current=null}function Yb(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){ke(e,e.return,r)}}function um(e,t,a){try{var n=e.stateNode;TE(n,e.type,a,t),n[Ft]=t}catch(r){ke(e,e.return,r)}}function Jb(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&rr(e.type)||e.tag===4}function cm(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||Jb(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&rr(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function Xm(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=ic));else if(n!==4&&(n===27&&rr(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(Xm(e,t,a),e=e.sibling;e!==null;)Xm(e,t,a),e=e.sibling}function Lu(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&rr(e.type)&&(a=e.stateNode),e=e.child,e!==null))for(Lu(e,t,a),e=e.sibling;e!==null;)Lu(e,t,a),e=e.sibling}function Xb(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);vt(t,n,a),t[$t]=e,t[Ft]=a}catch(s){ke(e,e.return,s)}}var sn=!1,Qe=!1,dm=!1,Lg=typeof WeakSet=="function"?WeakSet:Set,lt=null;function mE(e,t){if(e=e.containerInfo,sf=Hu,e=jy(e),kf(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,l=-1,c=0,d=0,m=e,f=null;t:for(;;){for(var p;m!==a||r!==0&&m.nodeType!==3||(o=i+r),m!==s||n!==0&&m.nodeType!==3||(l=i+n),m.nodeType===3&&(i+=m.nodeValue.length),(p=m.firstChild)!==null;)f=m,m=p;for(;;){if(m===e)break t;if(f===a&&++c===r&&(o=i),f===s&&++d===n&&(l=i),(p=m.nextSibling)!==null)break;m=f,f=m.parentNode}m=p}a=o===-1||l===-1?null:{start:o,end:l}}else a=null}a=a||{start:0,end:0}}else a=null;for(of={focusedElem:e,selectionRange:a},Hu=!1,lt=t;lt!==null;)if(t=lt,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,lt=e;else for(;lt!==null;){switch(t=lt,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var x=Ar(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(x,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(y){ke(a,a.return,y)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)uf(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":uf(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(U(163))}if(e=t.sibling,e!==null){e.return=t.return,lt=e;break}lt=t.return}}function Zb(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:On(e,a),n&4&&qo(5,a);break;case 1:if(On(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){ke(a,a.return,i)}else{var r=Ar(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){ke(a,a.return,i)}}n&64&&Vb(a),n&512&&co(a,a.return);break;case 3:if(On(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{Wy(e,t)}catch(i){ke(a,a.return,i)}}break;case 27:t===null&&n&4&&Xb(a);case 26:case 5:On(e,a),t===null&&n&4&&Yb(a),n&512&&co(a,a.return);break;case 12:On(e,a);break;case 13:On(e,a),n&4&&t0(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=$E.bind(null,a),UE(e,a))));break;case 22:if(n=a.memoizedState!==null||sn,!n){t=t!==null&&t.memoizedState!==null||Qe,r=sn;var s=Qe;sn=n,(Qe=t)&&!s?Ln(e,a,(a.subtreeFlags&8772)!==0):On(e,a),sn=r,Qe=s}break;case 30:break;default:On(e,a)}}function Wb(e){var t=e.alternate;t!==null&&(e.alternate=null,Wb(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&xf(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var Oe=null,Ut=!1;function rn(e,t,a){for(a=a.child;a!==null;)e0(e,t,a),a=a.sibling}function e0(e,t,a){if(Yt&&typeof Yt.onCommitFiberUnmount=="function")try{Yt.onCommitFiberUnmount(Ao,a)}catch{}switch(a.tag){case 26:Qe||ja(a,t),rn(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:Qe||ja(a,t);var n=Oe,r=Ut;rr(a.type)&&(Oe=a.stateNode,Ut=!1),rn(e,t,a),ho(a.stateNode),Oe=n,Ut=r;break;case 5:Qe||ja(a,t);case 6:if(n=Oe,r=Ut,Oe=null,rn(e,t,a),Oe=n,Ut=r,Oe!==null)if(Ut)try{(Oe.nodeType===9?Oe.body:Oe.nodeName==="HTML"?Oe.ownerDocument.body:Oe).removeChild(a.stateNode)}catch(s){ke(a,t,s)}else try{Oe.removeChild(a.stateNode)}catch(s){ke(a,t,s)}break;case 18:Oe!==null&&(Ut?(e=Oe,Yg(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),Eo(e)):Yg(Oe,a.stateNode));break;case 4:n=Oe,r=Ut,Oe=a.stateNode.containerInfo,Ut=!0,rn(e,t,a),Oe=n,Ut=r;break;case 0:case 11:case 14:case 15:Qe||tr(2,a,t),Qe||tr(4,a,t),rn(e,t,a);break;case 1:Qe||(ja(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&Gb(a,t,n)),rn(e,t,a);break;case 21:rn(e,t,a);break;case 22:Qe=(n=Qe)||a.memoizedState!==null,rn(e,t,a),Qe=n;break;default:rn(e,t,a)}}function t0(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{Eo(e)}catch(a){ke(t,t.return,a)}}function fE(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new Lg),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new Lg),t;default:throw Error(U(435,e.tag))}}function mm(e,t){var a=fE(e);t.forEach(function(n){var r=wE.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function Ht(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(rr(o.type)){Oe=o.stateNode,Ut=!1;break e}break;case 5:Oe=o.stateNode,Ut=!1;break e;case 3:case 4:Oe=o.stateNode.containerInfo,Ut=!0;break e}o=o.return}if(Oe===null)throw Error(U(160));e0(s,i,r),Oe=null,Ut=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)a0(t,e),t=t.sibling}var wa=null;function a0(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:Ht(t,e),Qt(e),n&4&&(tr(3,e,e.return),qo(3,e),tr(5,e,e.return));break;case 1:Ht(t,e),Qt(e),n&512&&(Qe||a===null||ja(a,a.return)),n&64&&sn&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=wa;if(Ht(t,e),Qt(e),n&512&&(Qe||a===null||ja(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[Oo]||s[$t]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),vt(s,n,a),s[$t]=e,ut(s),n=s;break e;case"link":var i=ey("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),vt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=ey("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),vt(s,n,a),r.head.appendChild(s);break;default:throw Error(U(468,n))}s[$t]=e,ut(s),n=s}e.stateNode=n}else ty(r,e.type,e.stateNode);else e.stateNode=Wg(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?ty(r,e.type,e.stateNode):Wg(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&um(e,e.memoizedProps,a.memoizedProps)}break;case 27:Ht(t,e),Qt(e),n&512&&(Qe||a===null||ja(a,a.return)),a!==null&&n&4&&um(e,e.memoizedProps,a.memoizedProps);break;case 5:if(Ht(t,e),Qt(e),n&512&&(Qe||a===null||ja(a,a.return)),e.flags&32){r=e.stateNode;try{Ps(r,"")}catch(p){ke(e,e.return,p)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,um(e,r,a!==null?a.memoizedProps:r)),n&1024&&(dm=!0);break;case 6:if(Ht(t,e),Qt(e),n&4){if(e.stateNode===null)throw Error(U(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(p){ke(e,e.return,p)}}break;case 3:if(hu=null,r=wa,wa=qu(t.containerInfo),Ht(t,e),wa=r,Qt(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{Eo(t.containerInfo)}catch(p){ke(e,e.return,p)}dm&&(dm=!1,n0(e));break;case 4:n=wa,wa=qu(e.stateNode.containerInfo),Ht(t,e),Qt(e),wa=n;break;case 12:Ht(t,e),Qt(e);break;case 13:Ht(t,e),Qt(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(Wf=Ba()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,mm(e,n)));break;case 22:r=e.memoizedState!==null;var l=a!==null&&a.memoizedState!==null,c=sn,d=Qe;if(sn=c||r,Qe=d||l,Ht(t,e),Qe=d,sn=c,Qt(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||l||sn||Qe||xr(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){l=a=t;try{if(s=l.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=l.stateNode;var m=l.memoizedProps.style,f=m!=null&&m.hasOwnProperty("display")?m.display:null;o.style.display=f==null||typeof f=="boolean"?"":(""+f).trim()}}catch(p){ke(l,l.return,p)}}}else if(t.tag===6){if(a===null){l=t;try{l.stateNode.nodeValue=r?"":l.memoizedProps}catch(p){ke(l,l.return,p)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,mm(e,a))));break;case 19:Ht(t,e),Qt(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,mm(e,n)));break;case 30:break;case 21:break;default:Ht(t,e),Qt(e)}}function Qt(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(Jb(n)){a=n;break}n=n.return}if(a==null)throw Error(U(160));switch(a.tag){case 27:var r=a.stateNode,s=cm(e);Lu(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(Ps(i,""),a.flags&=-33);var o=cm(e);Lu(e,o,i);break;case 3:case 4:var l=a.stateNode.containerInfo,c=cm(e);Xm(e,c,l);break;default:throw Error(U(161))}}catch(d){ke(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function n0(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;n0(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function On(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)Zb(e,t.alternate,t),t=t.sibling}function xr(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:tr(4,t,t.return),xr(t);break;case 1:ja(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&Gb(t,t.return,a),xr(t);break;case 27:ho(t.stateNode);case 26:case 5:ja(t,t.return),xr(t);break;case 22:t.memoizedState===null&&xr(t);break;case 30:xr(t);break;default:xr(t)}e=e.sibling}}function Ln(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:Ln(r,s,a),qo(4,s);break;case 1:if(Ln(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){ke(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var l=r.shared.hiddenCallbacks;if(l!==null)for(r.shared.hiddenCallbacks=null,r=0;r<l.length;r++)Zy(l[r],o)}catch(c){ke(n,n.return,c)}}a&&i&64&&Vb(s),co(s,s.return);break;case 27:Xb(s);case 26:case 5:Ln(r,s,a),a&&n===null&&i&4&&Yb(s),co(s,s.return);break;case 12:Ln(r,s,a);break;case 13:Ln(r,s,a),a&&i&4&&t0(r,s);break;case 22:s.memoizedState===null&&Ln(r,s,a),co(s,s.return);break;case 30:break;default:Ln(r,s,a)}t=t.sibling}}function Yf(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&jo(a))}function Jf(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&jo(e))}function Pa(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)r0(e,t,a,n),t=t.sibling}function r0(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:Pa(e,t,a,n),r&2048&&qo(9,t);break;case 1:Pa(e,t,a,n);break;case 3:Pa(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&jo(e)));break;case 12:if(r&2048){Pa(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(l){ke(t,t.return,l)}}else Pa(e,t,a,n);break;case 13:Pa(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?Pa(e,t,a,n):mo(e,t):s._visibility&2?Pa(e,t,a,n):(s._visibility|=2,ms(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&Yf(i,t);break;case 24:Pa(e,t,a,n),r&2048&&Jf(t.alternate,t);break;default:Pa(e,t,a,n)}}function ms(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,l=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:ms(s,i,o,l,r),qo(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?ms(s,i,o,l,r):mo(s,i):(d._visibility|=2,ms(s,i,o,l,r)),r&&c&2048&&Yf(i.alternate,i);break;case 24:ms(s,i,o,l,r),r&&c&2048&&Jf(i.alternate,i);break;default:ms(s,i,o,l,r)}t=t.sibling}}function mo(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:mo(a,n),r&2048&&Yf(n.alternate,n);break;case 24:mo(a,n),r&2048&&Jf(n.alternate,n);break;default:mo(a,n)}t=t.sibling}}var eo=8192;function us(e){if(e.subtreeFlags&eo)for(e=e.child;e!==null;)s0(e),e=e.sibling}function s0(e){switch(e.tag){case 26:us(e),e.flags&eo&&e.memoizedState!==null&&JE(wa,e.memoizedState,e.memoizedProps);break;case 5:us(e);break;case 3:case 4:var t=wa;wa=qu(e.stateNode.containerInfo),us(e),wa=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=eo,eo=16777216,us(e),eo=t):us(e));break;default:us(e)}}function i0(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function Gi(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];lt=n,l0(n,e)}i0(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)o0(e),e=e.sibling}function o0(e){switch(e.tag){case 0:case 11:case 15:Gi(e),e.flags&2048&&tr(9,e,e.return);break;case 3:Gi(e);break;case 12:Gi(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,fu(e)):Gi(e);break;default:Gi(e)}}function fu(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];lt=n,l0(n,e)}i0(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:tr(8,t,t.return),fu(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,fu(t));break;default:fu(t)}e=e.sibling}}function l0(e,t){for(;lt!==null;){var a=lt;switch(a.tag){case 0:case 11:case 15:tr(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:jo(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,lt=n;else e:for(a=e;lt!==null;){n=lt;var r=n.sibling,s=n.return;if(Wb(n),n===a){lt=null;break e}if(r!==null){r.return=s,lt=r;break e}lt=s}}}var pE={getCacheForType:function(e){var t=wt(at),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},hE=typeof WeakMap=="function"?WeakMap:Map,Se=0,Re=null,ue=null,de=0,we=0,Vt=null,Kn=!1,Gs=!1,Xf=!1,gn=0,qe=0,ar=0,kr=0,Zf=0,ha=0,zs=0,fo=null,jt=null,Zm=!1,Wf=0,Pu=1/0,Uu=null,Yn=null,ht=0,Jn=null,qs=null,Ms=0,Wm=0,ef=null,u0=null,po=0,tf=null;function Xt(){if((Se&2)!==0&&de!==0)return de&-de;if(te.T!==null){var e=Us;return e!==0?e:tp()}return xy()}function c0(){ha===0&&(ha=(de&536870912)===0||ge?vy():536870912);var e=va.current;return e!==null&&(e.flags|=32),ha}function Zt(e,t,a){(e===Re&&(we===2||we===9)||e.cancelPendingCommit!==null)&&(Is(e,0),Hn(e,de,ha,!1)),Mo(e,a),((Se&2)===0||e!==Re)&&(e===Re&&((Se&2)===0&&(kr|=a),qe===4&&Hn(e,de,ha,!1)),Ia(e))}function d0(e,t,a){if((Se&6)!==0)throw Error(U(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||Do(e,t),r=n?yE(e,t):fm(e,t,!0),s=n;do{if(r===0){Gs&&!n&&Hn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!vE(a)){r=fm(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=fo;var l=o.current.memoizedState.isDehydrated;if(l&&(Is(o,i).flags|=256),i=fm(o,i,!1),i!==2){if(Xf&&!l){o.errorRecoveryDisabledLanes|=s,kr|=s,r=4;break e}s=jt,jt=r,s!==null&&(jt===null?jt=s:jt.push.apply(jt,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){Is(e,0),Hn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(U(345));case 4:if((t&4194048)!==t)break;case 6:Hn(n,t,ha,!Kn);break e;case 2:jt=null;break;case 3:case 5:break;default:throw Error(U(329))}if((t&62914560)===t&&(r=Wf+300-Ba(),10<r)){if(Hn(n,t,ha,!Kn),Vu(n,0,!0)!==0)break e;n.timeoutHandle=T0(Pg.bind(null,n,a,jt,Uu,Zm,t,ha,kr,zs,Kn,s,2,-0,0),r);break e}Pg(n,a,jt,Uu,Zm,t,ha,kr,zs,Kn,s,0,-0,0)}}break}while(!0);Ia(e)}function Pg(e,t,a,n,r,s,i,o,l,c,d,m,f,p){if(e.timeoutHandle=-1,m=t.subtreeFlags,(m&8192||(m&16785408)===16785408)&&(_o={stylesheets:null,count:0,unsuspend:YE},s0(t),m=XE(),m!==null)){e.cancelPendingCommit=m(jg.bind(null,e,t,s,a,n,r,i,o,l,d,1,f,p)),Hn(e,s,i,!c);return}jg(e,t,s,a,n,r,i,o,l)}function vE(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!Wt(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function Hn(e,t,a,n){t&=~Zf,t&=~kr,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Jt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&yy(e,a,t)}function nc(){return(Se&6)===0?(Io(0,!1),!1):!0}function ep(){if(ue!==null){if(we===0)var e=ue.return;else e=ue,cn=Lr=null,Bf(e),Ds=null,wo=0,e=ue;for(;e!==null;)Qb(e.alternate,e),e=e.return;ue=null}}function Is(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,DE(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),ep(),Re=e,ue=a=mn(e.current,null),de=t,we=0,Vt=null,Kn=!1,Gs=Do(e,t),Xf=!1,zs=ha=Zf=kr=ar=qe=0,jt=fo=null,Zm=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Jt(n),s=1<<r;t|=e[r],n&=~s}return gn=t,Xu(),a}function m0(e,t){oe=null,te.H=Tu,t===Fo||t===Wu?(t=pg(),we=3):t===Jy?(t=pg(),we=4):we=t===Bb?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Vt=t,ue===null&&(qe=1,Mu(e,pa(t,e.current)))}function f0(){var e=te.H;return te.H=Tu,e===null?Tu:e}function p0(){var e=te.A;return te.A=pE,e}function af(){qe=4,Kn||(de&4194048)!==de&&va.current!==null||(Gs=!0),(ar&134217727)===0&&(kr&134217727)===0||Re===null||Hn(Re,de,ha,!1)}function fm(e,t,a){var n=Se;Se|=2;var r=f0(),s=p0();(Re!==e||de!==t)&&(Uu=null,Is(e,t)),t=!1;var i=qe;e:do try{if(we!==0&&ue!==null){var o=ue,l=Vt;switch(we){case 8:ep(),i=6;break e;case 3:case 2:case 9:case 6:va.current===null&&(t=!0);var c=we;if(we=0,Vt=null,_s(e,o,l,c),a&&Gs){i=0;break e}break;default:c=we,we=0,Vt=null,_s(e,o,l,c)}}gE(),i=qe;break}catch(d){m0(e,d)}while(!0);return t&&e.shellSuspendCounter++,cn=Lr=null,Se=n,te.H=r,te.A=s,ue===null&&(Re=null,de=0,Xu()),i}function gE(){for(;ue!==null;)h0(ue)}function yE(e,t){var a=Se;Se|=2;var n=f0(),r=p0();Re!==e||de!==t?(Uu=null,Pu=Ba()+500,Is(e,t)):Gs=Do(e,t);e:do try{if(we!==0&&ue!==null){t=ue;var s=Vt;t:switch(we){case 1:we=0,Vt=null,_s(e,t,s,1);break;case 2:case 9:if(fg(s)){we=0,Vt=null,Ug(t);break}t=function(){we!==2&&we!==9||Re!==e||(we=7),Ia(e)},s.then(t,t);break e;case 3:we=7;break e;case 4:we=5;break e;case 7:fg(s)?(we=0,Vt=null,Ug(t)):(we=0,Vt=null,_s(e,t,s,7));break;case 5:var i=null;switch(ue.tag){case 26:i=ue.memoizedState;case 5:case 27:var o=ue;if(!i||O0(i)){we=0,Vt=null;var l=o.sibling;if(l!==null)ue=l;else{var c=o.return;c!==null?(ue=c,rc(c)):ue=null}break t}}we=0,Vt=null,_s(e,t,s,5);break;case 6:we=0,Vt=null,_s(e,t,s,6);break;case 8:ep(),qe=6;break e;default:throw Error(U(462))}}bE();break}catch(d){m0(e,d)}while(!0);return cn=Lr=null,te.H=n,te.A=r,Se=a,ue!==null?0:(Re=null,de=0,Xu(),qe)}function bE(){for(;ue!==null&&!zR();)h0(ue)}function h0(e){var t=Hb(e.alternate,e,gn);e.memoizedProps=e.pendingProps,t===null?rc(e):ue=t}function Ug(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=Tg(a,t,t.pendingProps,t.type,void 0,de);break;case 11:t=Tg(a,t,t.pendingProps,t.type.render,t.ref,de);break;case 5:Bf(t);default:Qb(a,t),t=ue=Qy(t,gn),t=Hb(a,t,gn)}e.memoizedProps=e.pendingProps,t===null?rc(e):ue=t}function _s(e,t,a,n){cn=Lr=null,Bf(t),Ds=null,wo=0;var r=t.return;try{if(lE(e,r,t,a,de)){qe=1,Mu(e,pa(a,e.current)),ue=null;return}}catch(s){if(r!==null)throw ue=r,s;qe=1,Mu(e,pa(a,e.current)),ue=null;return}t.flags&32768?(ge||n===1?e=!0:Gs||(de&536870912)!==0?e=!1:(Kn=e=!0,(n===2||n===9||n===3||n===6)&&(n=va.current,n!==null&&n.tag===13&&(n.flags|=16384))),v0(t,e)):rc(t)}function rc(e){var t=e;do{if((t.flags&32768)!==0){v0(t,Kn);return}e=t.return;var a=cE(t.alternate,t,gn);if(a!==null){ue=a;return}if(t=t.sibling,t!==null){ue=t;return}ue=t=e}while(t!==null);qe===0&&(qe=5)}function v0(e,t){do{var a=dE(e.alternate,e);if(a!==null){a.flags&=32767,ue=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){ue=e;return}ue=e=a}while(e!==null);qe=6,ue=null}function jg(e,t,a,n,r,s,i,o,l){e.cancelPendingCommit=null;do sc();while(ht!==0);if((Se&6)!==0)throw Error(U(327));if(t!==null){if(t===e.current)throw Error(U(177));if(s=t.lanes|t.childLanes,s|=Rf,XR(e,a,s,i,o,l),e===Re&&(ue=Re=null,de=0),qs=t,Jn=e,Ms=a,Wm=s,ef=r,u0=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,SE(xu,function(){return $0(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=te.T,te.T=null,r=ye.p,ye.p=2,i=Se,Se|=4;try{mE(e,t,a)}finally{Se=i,ye.p=r,te.T=n}}ht=1,g0(),y0(),b0()}}function g0(){if(ht===1){ht=0;var e=Jn,t=qs,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=te.T,te.T=null;var n=ye.p;ye.p=2;var r=Se;Se|=4;try{a0(t,e);var s=of,i=jy(e.containerInfo),o=s.focusedElem,l=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&Uy(o.ownerDocument.documentElement,o)){if(l!==null&&kf(o)){var c=l.start,d=l.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var m=o.ownerDocument||document,f=m&&m.defaultView||window;if(f.getSelection){var p=f.getSelection(),x=o.textContent.length,y=Math.min(l.start,x),$=l.end===void 0?y:Math.min(l.end,x);!p.extend&&y>$&&(i=$,$=y,y=i);var g=sg(o,y),v=sg(o,$);if(g&&v&&(p.rangeCount!==1||p.anchorNode!==g.node||p.anchorOffset!==g.offset||p.focusNode!==v.node||p.focusOffset!==v.offset)){var b=m.createRange();b.setStart(g.node,g.offset),p.removeAllRanges(),y>$?(p.addRange(b),p.extend(v.node,v.offset)):(b.setEnd(v.node,v.offset),p.addRange(b))}}}}for(m=[],p=o;p=p.parentNode;)p.nodeType===1&&m.push({element:p,left:p.scrollLeft,top:p.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<m.length;o++){var w=m[o];w.element.scrollLeft=w.left,w.element.scrollTop=w.top}}Hu=!!sf,of=sf=null}finally{Se=r,ye.p=n,te.T=a}}e.current=t,ht=2}}function y0(){if(ht===2){ht=0;var e=Jn,t=qs,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=te.T,te.T=null;var n=ye.p;ye.p=2;var r=Se;Se|=4;try{Zb(e,t.alternate,t)}finally{Se=r,ye.p=n,te.T=a}}ht=3}}function b0(){if(ht===4||ht===3){ht=0,qR();var e=Jn,t=qs,a=Ms,n=u0;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?ht=5:(ht=0,qs=Jn=null,x0(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(Yn=null),bf(a),t=t.stateNode,Yt&&typeof Yt.onCommitFiberRoot=="function")try{Yt.onCommitFiberRoot(Ao,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=te.T,r=ye.p,ye.p=2,te.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{te.T=t,ye.p=r}}(Ms&3)!==0&&sc(),Ia(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===tf?po++:(po=0,tf=e):po=0,Io(0,!1)}}function x0(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,jo(t)))}function sc(e){return g0(),y0(),b0(),$0(e)}function $0(){if(ht!==5)return!1;var e=Jn,t=Wm;Wm=0;var a=bf(Ms),n=te.T,r=ye.p;try{ye.p=32>a?32:a,te.T=null,a=ef,ef=null;var s=Jn,i=Ms;if(ht=0,qs=Jn=null,Ms=0,(Se&6)!==0)throw Error(U(331));var o=Se;if(Se|=4,o0(s.current),r0(s,s.current,i,a),Se=o,Io(0,!1),Yt&&typeof Yt.onPostCommitFiberRoot=="function")try{Yt.onPostCommitFiberRoot(Ao,s)}catch{}return!0}finally{ye.p=r,te.T=n,x0(e,t)}}function Fg(e,t,a){t=pa(a,t),t=Gm(e.stateNode,t,2),e=Gn(e,t,2),e!==null&&(Mo(e,2),Ia(e))}function ke(e,t,a){if(e.tag===3)Fg(e,e,a);else for(;t!==null;){if(t.tag===3){Fg(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(Yn===null||!Yn.has(n))){e=pa(a,e),a=jb(2),n=Gn(t,a,2),n!==null&&(Fb(a,n,t,e),Mo(n,2),Ia(n));break}}t=t.return}}function pm(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new hE;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(Xf=!0,r.add(a),e=xE.bind(null,e,t,a),t.then(e,e))}function xE(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,Re===e&&(de&a)===a&&(qe===4||qe===3&&(de&62914560)===de&&300>Ba()-Wf?(Se&2)===0&&Is(e,0):Zf|=a,zs===de&&(zs=0)),Ia(e)}function w0(e,t){t===0&&(t=gy()),e=Vs(e,t),e!==null&&(Mo(e,t),Ia(e))}function $E(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),w0(e,a)}function wE(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(U(314))}n!==null&&n.delete(t),w0(e,a)}function SE(e,t){return gf(e,t)}var ju=null,fs=null,nf=!1,Fu=!1,hm=!1,Rr=0;function Ia(e){e!==fs&&e.next===null&&(fs===null?ju=fs=e:fs=fs.next=e),Fu=!0,nf||(nf=!0,_E())}function Io(e,t){if(!hm&&Fu){hm=!0;do for(var a=!1,n=ju;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Jt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,Bg(n,s))}else s=de,s=Vu(n,n===Re?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||Do(n,s)||(a=!0,Bg(n,s));n=n.next}while(a);hm=!1}}function NE(){S0()}function S0(){Fu=nf=!1;var e=0;Rr!==0&&(AE()&&(e=Rr),Rr=0);for(var t=Ba(),a=null,n=ju;n!==null;){var r=n.next,s=N0(n,t);s===0?(n.next=null,a===null?ju=r:a.next=r,r===null&&(fs=a)):(a=n,(e!==0||(s&3)!==0)&&(Fu=!0)),n=r}Io(e,!1)}function N0(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Jt(s),o=1<<i,l=r[i];l===-1?((o&a)===0||(o&n)!==0)&&(r[i]=JR(o,t)):l<=t&&(e.expiredLanes|=o),s&=~o}if(t=Re,a=de,a=Vu(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&(we===2||we===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&qd(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||Do(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&qd(n),bf(a)){case 2:case 8:a=py;break;case 32:a=xu;break;case 268435456:a=hy;break;default:a=xu}return n=_0.bind(null,e),a=gf(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&qd(n),e.callbackPriority=2,e.callbackNode=null,2}function _0(e,t){if(ht!==0&&ht!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(sc(!0)&&e.callbackNode!==a)return null;var n=de;return n=Vu(e,e===Re?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:(d0(e,n,t),N0(e,Ba()),e.callbackNode!=null&&e.callbackNode===a?_0.bind(null,e):null)}function Bg(e,t){if(sc())return null;d0(e,t,!0)}function _E(){ME(function(){(Se&6)!==0?gf(fy,NE):S0()})}function tp(){return Rr===0&&(Rr=vy()),Rr}function zg(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:su(""+e)}function qg(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function kE(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=zg((r[Ft]||null).action),i=n.submitter;i&&(t=(t=i[Ft]||null)?zg(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Gu("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(Rr!==0){var l=i?qg(r,i):new FormData(r);Qm(a,{pending:!0,data:l,method:r.method,action:s},null,l)}}else typeof s=="function"&&(o.preventDefault(),l=i?qg(r,i):new FormData(r),Qm(a,{pending:!0,data:l,method:r.method,action:s},s,l))},currentTarget:r}]})}}for(Wl=0;Wl<Om.length;Wl++)eu=Om[Wl],Ig=eu.toLowerCase(),Kg=eu[0].toUpperCase()+eu.slice(1),Na(Ig,"on"+Kg);var eu,Ig,Kg,Wl;Na(By,"onAnimationEnd");Na(zy,"onAnimationIteration");Na(qy,"onAnimationStart");Na("dblclick","onDoubleClick");Na("focusin","onFocus");Na("focusout","onBlur");Na(HC,"onTransitionRun");Na(QC,"onTransitionStart");Na(VC,"onTransitionCancel");Na(Iy,"onTransitionEnd");Ls("onMouseEnter",["mouseout","mouseover"]);Ls("onMouseLeave",["mouseout","mouseover"]);Ls("onPointerEnter",["pointerout","pointerover"]);Ls("onPointerLeave",["pointerout","pointerover"]);Dr("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Dr("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Dr("onBeforeInput",["compositionend","keypress","textInput","paste"]);Dr("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Dr("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Dr("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var So="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),RE=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(So));function k0(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],l=o.instance,c=o.currentTarget;if(o=o.listener,l!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Du(d)}r.currentTarget=null,s=l}else for(i=0;i<n.length;i++){if(o=n[i],l=o.instance,c=o.currentTarget,o=o.listener,l!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Du(d)}r.currentTarget=null,s=l}}}}function le(e,t){var a=t[Rm];a===void 0&&(a=t[Rm]=new Set);var n=e+"__bubble";a.has(n)||(R0(t,e,2,!1),a.add(n))}function vm(e,t,a){var n=0;t&&(n|=4),R0(a,e,n,t)}var tu="_reactListening"+Math.random().toString(36).slice(2);function ap(e){if(!e[tu]){e[tu]=!0,$y.forEach(function(a){a!=="selectionchange"&&(RE.has(a)||vm(a,!1,e),vm(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[tu]||(t[tu]=!0,vm("selectionchange",!1,t))}}function R0(e,t,a,n){switch(F0(t)){case 2:var r=e3;break;case 8:r=t3;break;default:r=ip}a=r.bind(null,t,a,e),r=void 0,!Am||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function gm(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var l=i.tag;if((l===3||l===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=vs(o),i===null)return;if(l=i.tag,l===5||l===6||l===26||l===27){n=s=i;continue e}o=o.parentNode}}n=n.return}Ey(function(){var c=s,d=wf(a),m=[];e:{var f=Ky.get(e);if(f!==void 0){var p=Gu,x=e;switch(e){case"keypress":if(ou(a)===0)break e;case"keydown":case"keyup":p=SC;break;case"focusin":x="focus",p=Jd;break;case"focusout":x="blur",p=Jd;break;case"beforeblur":case"afterblur":p=Jd;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":p=Jv;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":p=dC;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":p=kC;break;case By:case zy:case qy:p=pC;break;case Iy:p=CC;break;case"scroll":case"scrollend":p=uC;break;case"wheel":p=TC;break;case"copy":case"cut":case"paste":p=vC;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":p=Zv;break;case"toggle":case"beforetoggle":p=DC}var y=(t&4)!==0,$=!y&&(e==="scroll"||e==="scrollend"),g=y?f!==null?f+"Capture":null:f;y=[];for(var v=c,b;v!==null;){var w=v;if(b=w.stateNode,w=w.tag,w!==5&&w!==26&&w!==27||b===null||g===null||(w=go(v,g),w!=null&&y.push(No(v,w,b))),$)break;v=v.return}0<y.length&&(f=new p(f,x,null,a,d),m.push({event:f,listeners:y}))}}if((t&7)===0){e:{if(f=e==="mouseover"||e==="pointerover",p=e==="mouseout"||e==="pointerout",f&&a!==Tm&&(x=a.relatedTarget||a.fromElement)&&(vs(x)||x[Hs]))break e;if((p||f)&&(f=d.window===d?d:(f=d.ownerDocument)?f.defaultView||f.parentWindow:window,p?(x=a.relatedTarget||a.toElement,p=c,x=x?vs(x):null,x!==null&&($=To(x),y=x.tag,x!==$||y!==5&&y!==27&&y!==6)&&(x=null)):(p=null,x=c),p!==x)){if(y=Jv,w="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(y=Zv,w="onPointerLeave",g="onPointerEnter",v="pointer"),$=p==null?f:Wi(p),b=x==null?f:Wi(x),f=new y(w,v+"leave",p,a,d),f.target=$,f.relatedTarget=b,w=null,vs(d)===c&&(y=new y(g,v+"enter",x,a,d),y.target=b,y.relatedTarget=$,w=y),$=w,p&&x)t:{for(y=p,g=x,v=0,b=y;b;b=cs(b))v++;for(b=0,w=g;w;w=cs(w))b++;for(;0<v-b;)y=cs(y),v--;for(;0<b-v;)g=cs(g),b--;for(;v--;){if(y===g||g!==null&&y===g.alternate)break t;y=cs(y),g=cs(g)}y=null}else y=null;p!==null&&Hg(m,f,p,y,!1),x!==null&&$!==null&&Hg(m,$,x,y,!0)}}e:{if(f=c?Wi(c):window,p=f.nodeName&&f.nodeName.toLowerCase(),p==="select"||p==="input"&&f.type==="file")var S=ag;else if(tg(f))if(Ly)S=qC;else{S=BC;var C=FC}else p=f.nodeName,!p||p.toLowerCase()!=="input"||f.type!=="checkbox"&&f.type!=="radio"?c&&$f(c.elementType)&&(S=ag):S=zC;if(S&&(S=S(e,c))){Oy(m,S,a,d);break e}C&&C(e,f,c),e==="focusout"&&c&&f.type==="number"&&c.memoizedProps.value!=null&&Em(f,"number",f.value)}switch(C=c?Wi(c):window,e){case"focusin":(tg(C)||C.contentEditable==="true")&&(bs=C,Dm=c,no=null);break;case"focusout":no=Dm=bs=null;break;case"mousedown":Mm=!0;break;case"contextmenu":case"mouseup":case"dragend":Mm=!1,ig(m,a,d);break;case"selectionchange":if(KC)break;case"keydown":case"keyup":ig(m,a,d)}var _;if(_f)e:{switch(e){case"compositionstart":var A="onCompositionStart";break e;case"compositionend":A="onCompositionEnd";break e;case"compositionupdate":A="onCompositionUpdate";break e}A=void 0}else ys?Dy(e,a)&&(A="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(A="onCompositionStart");A&&(Ay&&a.locale!=="ko"&&(ys||A!=="onCompositionStart"?A==="onCompositionEnd"&&ys&&(_=Ty()):(In=d,Sf="value"in In?In.value:In.textContent,ys=!0)),C=Bu(c,A),0<C.length&&(A=new Xv(A,e,null,a,d),m.push({event:A,listeners:C}),_?A.data=_:(_=My(a),_!==null&&(A.data=_)))),(_=OC?LC(e,a):PC(e,a))&&(A=Bu(c,"onBeforeInput"),0<A.length&&(C=new Xv("onBeforeInput","beforeinput",null,a,d),m.push({event:C,listeners:A}),C.data=_)),kE(m,e,c,a,d)}k0(m,t)})}function No(e,t,a){return{instance:e,listener:t,currentTarget:a}}function Bu(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=go(e,a),r!=null&&n.unshift(No(e,r,s)),r=go(e,t),r!=null&&n.push(No(e,r,s))),e.tag===3)return n;e=e.return}return[]}function cs(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function Hg(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,l=o.alternate,c=o.stateNode;if(o=o.tag,l!==null&&l===n)break;o!==5&&o!==26&&o!==27||c===null||(l=c,r?(c=go(a,s),c!=null&&i.unshift(No(a,c,l))):r||(c=go(a,s),c!=null&&i.push(No(a,c,l)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var CE=/\r\n?/g,EE=/\u0000|\uFFFD/g;function Qg(e){return(typeof e=="string"?e:""+e).replace(CE,`
`).replace(EE,"")}function C0(e,t){return t=Qg(t),Qg(e)===t}function ic(){}function Ne(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||Ps(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&Ps(e,""+n);break;case"className":Kl(e,"class",n);break;case"tabIndex":Kl(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":Kl(e,a,n);break;case"style":Cy(e,n,s);break;case"data":if(t!=="object"){Kl(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=su(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&Ne(e,t,"name",r.name,r,null),Ne(e,t,"formEncType",r.formEncType,r,null),Ne(e,t,"formMethod",r.formMethod,r,null),Ne(e,t,"formTarget",r.formTarget,r,null)):(Ne(e,t,"encType",r.encType,r,null),Ne(e,t,"method",r.method,r,null),Ne(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=su(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=ic);break;case"onScroll":n!=null&&le("scroll",e);break;case"onScrollEnd":n!=null&&le("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(U(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(U(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=su(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":le("beforetoggle",e),le("toggle",e),ru(e,"popover",n);break;case"xlinkActuate":an(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":an(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":an(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":an(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":an(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":an(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":an(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":an(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":an(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":ru(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=oC.get(a)||a,ru(e,a,n))}}function rf(e,t,a,n,r,s){switch(a){case"style":Cy(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(U(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(U(60));e.innerHTML=a}}break;case"children":typeof n=="string"?Ps(e,n):(typeof n=="number"||typeof n=="bigint")&&Ps(e,""+n);break;case"onScroll":n!=null&&le("scroll",e);break;case"onScrollEnd":n!=null&&le("scrollend",e);break;case"onClick":n!=null&&(e.onclick=ic);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!wy.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[Ft]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):ru(e,a,n)}}}function vt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":le("error",e),le("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(U(137,t));default:Ne(e,t,s,i,a,null)}}r&&Ne(e,t,"srcSet",a.srcSet,a,null),n&&Ne(e,t,"src",a.src,a,null);return;case"input":le("invalid",e);var o=s=i=r=null,l=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":l=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(U(137,t));break;default:Ne(e,t,n,d,a,null)}}_y(e,s,o,l,c,i,r,!1),$u(e);return;case"select":le("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:Ne(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?Rs(e,!!n,t,!1):a!=null&&Rs(e,!!n,a,!0);return;case"textarea":le("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(U(91));break;default:Ne(e,t,i,o,a,null)}Ry(e,n,r,s),$u(e);return;case"option":for(l in a)if(a.hasOwnProperty(l)&&(n=a[l],n!=null))switch(l){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:Ne(e,t,l,n,a,null)}return;case"dialog":le("beforetoggle",e),le("toggle",e),le("cancel",e),le("close",e);break;case"iframe":case"object":le("load",e);break;case"video":case"audio":for(n=0;n<So.length;n++)le(So[n],e);break;case"image":le("error",e),le("load",e);break;case"details":le("toggle",e);break;case"embed":case"source":case"link":le("error",e),le("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(U(137,t));default:Ne(e,t,c,n,a,null)}return;default:if($f(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&rf(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&Ne(e,t,o,n,a,null))}function TE(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,l=null,c=null,d=null;for(p in a){var m=a[p];if(a.hasOwnProperty(p)&&m!=null)switch(p){case"checked":break;case"value":break;case"defaultValue":l=m;default:n.hasOwnProperty(p)||Ne(e,t,p,null,n,m)}}for(var f in n){var p=n[f];if(m=a[f],n.hasOwnProperty(f)&&(p!=null||m!=null))switch(f){case"type":s=p;break;case"name":r=p;break;case"checked":c=p;break;case"defaultChecked":d=p;break;case"value":i=p;break;case"defaultValue":o=p;break;case"children":case"dangerouslySetInnerHTML":if(p!=null)throw Error(U(137,t));break;default:p!==m&&Ne(e,t,f,p,n,m)}}Cm(e,i,o,l,c,d,s,r);return;case"select":p=i=o=f=null;for(s in a)if(l=a[s],a.hasOwnProperty(s)&&l!=null)switch(s){case"value":break;case"multiple":p=l;default:n.hasOwnProperty(s)||Ne(e,t,s,null,n,l)}for(r in n)if(s=n[r],l=a[r],n.hasOwnProperty(r)&&(s!=null||l!=null))switch(r){case"value":f=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==l&&Ne(e,t,r,s,n,l)}t=o,a=i,n=p,f!=null?Rs(e,!!a,f,!1):!!n!=!!a&&(t!=null?Rs(e,!!a,t,!0):Rs(e,!!a,a?[]:"",!1));return;case"textarea":p=f=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:Ne(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":f=r;break;case"defaultValue":p=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(U(91));break;default:r!==s&&Ne(e,t,i,r,n,s)}ky(e,f,p);return;case"option":for(var x in a)if(f=a[x],a.hasOwnProperty(x)&&f!=null&&!n.hasOwnProperty(x))switch(x){case"selected":e.selected=!1;break;default:Ne(e,t,x,null,n,f)}for(l in n)if(f=n[l],p=a[l],n.hasOwnProperty(l)&&f!==p&&(f!=null||p!=null))switch(l){case"selected":e.selected=f&&typeof f!="function"&&typeof f!="symbol";break;default:Ne(e,t,l,f,n,p)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var y in a)f=a[y],a.hasOwnProperty(y)&&f!=null&&!n.hasOwnProperty(y)&&Ne(e,t,y,null,n,f);for(c in n)if(f=n[c],p=a[c],n.hasOwnProperty(c)&&f!==p&&(f!=null||p!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(f!=null)throw Error(U(137,t));break;default:Ne(e,t,c,f,n,p)}return;default:if($f(t)){for(var $ in a)f=a[$],a.hasOwnProperty($)&&f!==void 0&&!n.hasOwnProperty($)&&rf(e,t,$,void 0,n,f);for(d in n)f=n[d],p=a[d],!n.hasOwnProperty(d)||f===p||f===void 0&&p===void 0||rf(e,t,d,f,n,p);return}}for(var g in a)f=a[g],a.hasOwnProperty(g)&&f!=null&&!n.hasOwnProperty(g)&&Ne(e,t,g,null,n,f);for(m in n)f=n[m],p=a[m],!n.hasOwnProperty(m)||f===p||f==null&&p==null||Ne(e,t,m,f,n,p)}var sf=null,of=null;function zu(e){return e.nodeType===9?e:e.ownerDocument}function Vg(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function E0(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function lf(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var ym=null;function AE(){var e=window.event;return e&&e.type==="popstate"?e===ym?!1:(ym=e,!0):(ym=null,!1)}var T0=typeof setTimeout=="function"?setTimeout:void 0,DE=typeof clearTimeout=="function"?clearTimeout:void 0,Gg=typeof Promise=="function"?Promise:void 0,ME=typeof queueMicrotask=="function"?queueMicrotask:typeof Gg<"u"?function(e){return Gg.resolve(null).then(e).catch(OE)}:T0;function OE(e){setTimeout(function(){throw e})}function rr(e){return e==="head"}function Yg(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&ho(i.documentElement),a&2&&ho(i.body),a&4)for(a=i.head,ho(a),i=a.firstChild;i;){var o=i.nextSibling,l=i.nodeName;i[Oo]||l==="SCRIPT"||l==="STYLE"||l==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),Eo(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);Eo(t)}function uf(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":uf(a),xf(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function LE(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[Oo])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=Sa(e.nextSibling),e===null)break}return null}function PE(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=Sa(e.nextSibling),e===null))return null;return e}function cf(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function UE(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function Sa(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var df=null;function Jg(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function A0(e,t,a){switch(t=zu(a),e){case"html":if(e=t.documentElement,!e)throw Error(U(452));return e;case"head":if(e=t.head,!e)throw Error(U(453));return e;case"body":if(e=t.body,!e)throw Error(U(454));return e;default:throw Error(U(451))}}function ho(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);xf(e)}var ga=new Map,Xg=new Set;function qu(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var yn=ye.d;ye.d={f:jE,r:FE,D:BE,C:zE,L:qE,m:IE,X:HE,S:KE,M:QE};function jE(){var e=yn.f(),t=nc();return e||t}function FE(e){var t=Qs(e);t!==null&&t.tag===5&&t.type==="form"?Nb(t):yn.r(e)}var Ys=typeof document>"u"?null:document;function D0(e,t,a){var n=Ys;if(n&&typeof t=="string"&&t){var r=fa(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),Xg.has(r)||(Xg.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),vt(t,"link",e),ut(t),n.head.appendChild(t)))}}function BE(e){yn.D(e),D0("dns-prefetch",e,null)}function zE(e,t){yn.C(e,t),D0("preconnect",e,t)}function qE(e,t,a){yn.L(e,t,a);var n=Ys;if(n&&e&&t){var r='link[rel="preload"][as="'+fa(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+fa(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+fa(a.imageSizes)+'"]')):r+='[href="'+fa(e)+'"]';var s=r;switch(t){case"style":s=Ks(e);break;case"script":s=Js(e)}ga.has(s)||(e=Ae({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),ga.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(Ko(s))||t==="script"&&n.querySelector(Ho(s))||(t=n.createElement("link"),vt(t,"link",e),ut(t),n.head.appendChild(t)))}}function IE(e,t){yn.m(e,t);var a=Ys;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+fa(n)+'"][href="'+fa(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=Js(e)}if(!ga.has(s)&&(e=Ae({rel:"modulepreload",href:e},t),ga.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(Ho(s)))return}n=a.createElement("link"),vt(n,"link",e),ut(n),a.head.appendChild(n)}}}function KE(e,t,a){yn.S(e,t,a);var n=Ys;if(n&&e){var r=ks(n).hoistableStyles,s=Ks(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(Ko(s)))o.loading=5;else{e=Ae({rel:"stylesheet",href:e,"data-precedence":t},a),(a=ga.get(s))&&np(e,a);var l=i=n.createElement("link");ut(l),vt(l,"link",e),l._p=new Promise(function(c,d){l.onload=c,l.onerror=d}),l.addEventListener("load",function(){o.loading|=1}),l.addEventListener("error",function(){o.loading|=2}),o.loading|=4,pu(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function HE(e,t){yn.X(e,t);var a=Ys;if(a&&e){var n=ks(a).hoistableScripts,r=Js(e),s=n.get(r);s||(s=a.querySelector(Ho(r)),s||(e=Ae({src:e,async:!0},t),(t=ga.get(r))&&rp(e,t),s=a.createElement("script"),ut(s),vt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function QE(e,t){yn.M(e,t);var a=Ys;if(a&&e){var n=ks(a).hoistableScripts,r=Js(e),s=n.get(r);s||(s=a.querySelector(Ho(r)),s||(e=Ae({src:e,async:!0,type:"module"},t),(t=ga.get(r))&&rp(e,t),s=a.createElement("script"),ut(s),vt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function Zg(e,t,a,n){var r=(r=Qn.current)?qu(r):null;if(!r)throw Error(U(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=Ks(a.href),a=ks(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=Ks(a.href);var s=ks(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(Ko(e)))&&!s._p&&(i.instance=s,i.state.loading=5),ga.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},ga.set(e,a),s||VE(r,e,a,i.state))),t&&n===null)throw Error(U(528,""));return i}if(t&&n!==null)throw Error(U(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=Js(a),a=ks(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(U(444,e))}}function Ks(e){return'href="'+fa(e)+'"'}function Ko(e){return'link[rel="stylesheet"]['+e+"]"}function M0(e){return Ae({},e,{"data-precedence":e.precedence,precedence:null})}function VE(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),vt(t,"link",a),ut(t),e.head.appendChild(t))}function Js(e){return'[src="'+fa(e)+'"]'}function Ho(e){return"script[async]"+e}function Wg(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+fa(a.href)+'"]');if(n)return t.instance=n,ut(n),n;var r=Ae({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),ut(n),vt(n,"style",r),pu(n,a.precedence,e),t.instance=n;case"stylesheet":r=Ks(a.href);var s=e.querySelector(Ko(r));if(s)return t.state.loading|=4,t.instance=s,ut(s),s;n=M0(a),(r=ga.get(r))&&np(n,r),s=(e.ownerDocument||e).createElement("link"),ut(s);var i=s;return i._p=new Promise(function(o,l){i.onload=o,i.onerror=l}),vt(s,"link",n),t.state.loading|=4,pu(s,a.precedence,e),t.instance=s;case"script":return s=Js(a.src),(r=e.querySelector(Ho(s)))?(t.instance=r,ut(r),r):(n=a,(r=ga.get(s))&&(n=Ae({},a),rp(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),ut(r),vt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(U(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,pu(n,a.precedence,e));return t.instance}function pu(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function np(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function rp(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var hu=null;function ey(e,t,a){if(hu===null){var n=new Map,r=hu=new Map;r.set(a,n)}else r=hu,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[Oo]||s[$t]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function ty(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function GE(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function O0(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var _o=null;function YE(){}function JE(e,t,a){if(_o===null)throw Error(U(475));var n=_o;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=Ks(a.href),s=e.querySelector(Ko(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=Iu.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,ut(s);return}s=e.ownerDocument||e,a=M0(a),(r=ga.get(r))&&np(a,r),s=s.createElement("link"),ut(s);var i=s;i._p=new Promise(function(o,l){i.onload=o,i.onerror=l}),vt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=Iu.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function XE(){if(_o===null)throw Error(U(475));var e=_o;return e.stylesheets&&e.count===0&&mf(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&mf(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function Iu(){if(this.count--,this.count===0){if(this.stylesheets)mf(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var Ku=null;function mf(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,Ku=new Map,t.forEach(ZE,e),Ku=null,Iu.call(e))}function ZE(e,t){if(!(t.state.loading&4)){var a=Ku.get(e);if(a)var n=a.get(null);else{a=new Map,Ku.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=Iu.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var ko={$$typeof:on,Provider:null,Consumer:null,_currentValue:$r,_currentValue2:$r,_threadCount:0};function WE(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=Id(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=Id(0),this.hiddenUpdates=Id(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function L0(e,t,a,n,r,s,i,o,l,c,d,m){return e=new WE(e,t,a,i,o,l,c,m),t=1,s===!0&&(t|=24),s=Gt(3,null,null,t),e.current=s,s.stateNode=e,t=Df(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},Of(s),e}function P0(e){return e?(e=ws,e):ws}function U0(e,t,a,n,r,s){r=P0(r),n.context===null?n.context=r:n.pendingContext=r,n=Vn(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=Gn(e,n,t),a!==null&&(Zt(a,e,t),io(a,e,t))}function ay(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function sp(e,t){ay(e,t),(e=e.alternate)&&ay(e,t)}function j0(e){if(e.tag===13){var t=Vs(e,67108864);t!==null&&Zt(t,e,67108864),sp(e,67108864)}}var Hu=!0;function e3(e,t,a,n){var r=te.T;te.T=null;var s=ye.p;try{ye.p=2,ip(e,t,a,n)}finally{ye.p=s,te.T=r}}function t3(e,t,a,n){var r=te.T;te.T=null;var s=ye.p;try{ye.p=8,ip(e,t,a,n)}finally{ye.p=s,te.T=r}}function ip(e,t,a,n){if(Hu){var r=ff(n);if(r===null)gm(e,t,n,Qu,a),ny(e,n);else if(n3(r,e,t,a,n))n.stopPropagation();else if(ny(e,n),t&4&&-1<a3.indexOf(e)){for(;r!==null;){var s=Qs(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=yr(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var l=1<<31-Jt(i);o.entanglements[1]|=l,i&=~l}Ia(s),(Se&6)===0&&(Pu=Ba()+500,Io(0,!1))}}break;case 13:o=Vs(s,2),o!==null&&Zt(o,s,2),nc(),sp(s,2)}if(s=ff(n),s===null&&gm(e,t,n,Qu,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else gm(e,t,n,null,a)}}function ff(e){return e=wf(e),op(e)}var Qu=null;function op(e){if(Qu=null,e=vs(e),e!==null){var t=To(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=uy(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return Qu=e,null}function F0(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(IR()){case fy:return 2;case py:return 8;case xu:case KR:return 32;case hy:return 268435456;default:return 32}default:return 32}}var pf=!1,Xn=null,Zn=null,Wn=null,Ro=new Map,Co=new Map,zn=[],a3="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function ny(e,t){switch(e){case"focusin":case"focusout":Xn=null;break;case"dragenter":case"dragleave":Zn=null;break;case"mouseover":case"mouseout":Wn=null;break;case"pointerover":case"pointerout":Ro.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":Co.delete(t.pointerId)}}function Yi(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Qs(t),t!==null&&j0(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function n3(e,t,a,n,r){switch(t){case"focusin":return Xn=Yi(Xn,e,t,a,n,r),!0;case"dragenter":return Zn=Yi(Zn,e,t,a,n,r),!0;case"mouseover":return Wn=Yi(Wn,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return Ro.set(s,Yi(Ro.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,Co.set(s,Yi(Co.get(s)||null,e,t,a,n,r)),!0}return!1}function B0(e){var t=vs(e.target);if(t!==null){var a=To(t);if(a!==null){if(t=a.tag,t===13){if(t=uy(a),t!==null){e.blockedOn=t,ZR(e.priority,function(){if(a.tag===13){var n=Xt();n=yf(n);var r=Vs(a,n);r!==null&&Zt(r,a,n),sp(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function vu(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=ff(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);Tm=n,a.target.dispatchEvent(n),Tm=null}else return t=Qs(a),t!==null&&j0(t),e.blockedOn=a,!1;t.shift()}return!0}function ry(e,t,a){vu(e)&&a.delete(t)}function r3(){pf=!1,Xn!==null&&vu(Xn)&&(Xn=null),Zn!==null&&vu(Zn)&&(Zn=null),Wn!==null&&vu(Wn)&&(Wn=null),Ro.forEach(ry),Co.forEach(ry)}function au(e,t){e.blockedOn===t&&(e.blockedOn=null,pf||(pf=!0,rt.unstable_scheduleCallback(rt.unstable_NormalPriority,r3)))}var nu=null;function sy(e){nu!==e&&(nu=e,rt.unstable_scheduleCallback(rt.unstable_NormalPriority,function(){nu===e&&(nu=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(op(n||a)===null)continue;break}var s=Qs(a);s!==null&&(e.splice(t,3),t-=3,Qm(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function Eo(e){function t(l){return au(l,e)}Xn!==null&&au(Xn,e),Zn!==null&&au(Zn,e),Wn!==null&&au(Wn,e),Ro.forEach(t),Co.forEach(t);for(var a=0;a<zn.length;a++){var n=zn[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<zn.length&&(a=zn[0],a.blockedOn===null);)B0(a),a.blockedOn===null&&zn.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[Ft]||null;if(typeof s=="function")i||sy(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[Ft]||null)o=i.formAction;else if(op(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),sy(a)}}}function lp(e){this._internalRoot=e}oc.prototype.render=lp.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(U(409));var a=t.current,n=Xt();U0(a,n,e,t,null,null)};oc.prototype.unmount=lp.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;U0(e.current,2,null,e,null,null),nc(),t[Hs]=null}};function oc(e){this._internalRoot=e}oc.prototype.unstable_scheduleHydration=function(e){if(e){var t=xy();e={blockedOn:null,target:e,priority:t};for(var a=0;a<zn.length&&t!==0&&t<zn[a].priority;a++);zn.splice(a,0,e),a===0&&B0(e)}};var iy=oy.version;if(iy!=="19.1.0")throw Error(U(527,iy,"19.1.0"));ye.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(U(188)):(e=Object.keys(e).join(","),Error(U(268,e)));return e=PR(t),e=e!==null?cy(e):null,e=e===null?null:e.stateNode,e};var s3={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:te,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(Ji=__REACT_DEVTOOLS_GLOBAL_HOOK__,!Ji.isDisabled&&Ji.supportsFiber))try{Ao=Ji.inject(s3),Yt=Ji}catch{}var Ji;lc.createRoot=function(e,t){if(!ly(e))throw Error(U(299));var a=!1,n="",r=Lb,s=Pb,i=Ub,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=L0(e,1,!1,null,null,a,n,r,s,i,o,null),e[Hs]=t.current,ap(e),new lp(t)};lc.hydrateRoot=function(e,t,a){if(!ly(e))throw Error(U(299));var n=!1,r="",s=Lb,i=Pb,o=Ub,l=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(l=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=L0(e,1,!0,t,a??null,n,r,s,i,o,l,c),t.context=P0(null),a=t.current,n=Xt(),n=yf(n),r=Vn(n),r.callback=null,Gn(a,r,n),a=n,t.current.lanes=a,Mo(t,a),Ia(t),e[Hs]=t.current,ap(e),new oc(t)};lc.version="19.1.0"});var K0=En((O6,I0)=>{"use strict";function q0(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(q0)}catch(e){console.error(e)}}q0(),I0.exports=z0()});var Mt=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var hR={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},vR=class{#t=hR;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},Da=new vR;function Wh(e){setTimeout(e,0)}var Ot=typeof window>"u"||"Deno"in globalThis;function De(){}function av(e,t){return typeof e=="function"?e(t):e}function Ai(e){return typeof e=="number"&&e>=0&&e!==1/0}function xl(e,t){return Math.max(e+(t||0)-Date.now(),0)}function $a(e,t){return typeof e=="function"?e(t):e}function Lt(e,t){return typeof e=="function"?e(t):e}function $l(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==Di(i,t.options))return!1}else if(!hr(t.queryKey,i))return!1}if(a!=="all"){let l=t.isActive();if(a==="active"&&!l||a==="inactive"&&l)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function wl(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(Ma(t.options.mutationKey)!==Ma(s))return!1}else if(!hr(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function Di(e,t){return(t?.queryKeyHashFn||Ma)(e)}function Ma(e){return JSON.stringify(e,(t,a)=>xd(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function hr(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>hr(e[a],t[a])):!1}var gR=Object.prototype.hasOwnProperty;function Mi(e,t){if(e===t)return e;let a=ev(e)&&ev(t);if(!a&&!(xd(e)&&xd(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},l=0;for(let c=0;c<i;c++){let d=a?c:s[c],m=e[d],f=t[d];if(m===f){o[d]=m,(a?c<r:gR.call(e,d))&&l++;continue}if(m===null||f===null||typeof m!="object"||typeof f!="object"){o[d]=f;continue}let p=Mi(m,f);o[d]=p,p===m&&l++}return r===i&&l===r?e:o}function Tn(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function ev(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function xd(e){if(!tv(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!tv(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function tv(e){return Object.prototype.toString.call(e)==="[object Object]"}function nv(e){return new Promise(t=>{Da.setTimeout(t,e)})}function Oi(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?Mi(e,t):t}function rv(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function sv(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var ts=Symbol();function Sl(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===ts?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function Li(e,t){return typeof e=="function"?e(...t):!!e}var yR=class extends Mt{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!Ot&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},as=new yR;function Pi(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var iv=Wh;function bR(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=iv,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(l=>{a(l)})})})};return{batch:o=>{let l;t++;try{l=o()}finally{t--,t||i()}return l},batchCalls:o=>(...l)=>{s(()=>{o(...l)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var ce=bR();var xR=class extends Mt{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!Ot&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},ns=new xR;function $R(e){return Math.min(1e3*2**e,3e4)}function $d(e){return(e??"online")==="online"?ns.isOnline():!0}var Nl=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function _l(e){let t=!1,a=0,n,r=Pi(),s=()=>r.status!=="pending",i=y=>{if(!s()){let $=new Nl(y);f($),e.onCancel?.($)}},o=()=>{t=!0},l=()=>{t=!1},c=()=>as.isFocused()&&(e.networkMode==="always"||ns.isOnline())&&e.canRun(),d=()=>$d(e.networkMode)&&e.canRun(),m=y=>{s()||(n?.(),r.resolve(y))},f=y=>{s()||(n?.(),r.reject(y))},p=()=>new Promise(y=>{n=$=>{(s()||c())&&y($)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),x=()=>{if(s())return;let y,$=a===0?e.initialPromise:void 0;try{y=$??e.fn()}catch(g){y=Promise.reject(g)}Promise.resolve(y).then(m).catch(g=>{if(s())return;let v=e.retry??(Ot?0:3),b=e.retryDelay??$R,w=typeof b=="function"?b(a,g):b,S=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!S){f(g);return}a++,e.onFail?.(a,g),nv(w).then(()=>c()?void 0:p()).then(()=>{t?f(g):x()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:l,canStart:d,start:()=>(d()?x():p().then(x),r)}}var kl=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),Ai(this.gcTime)&&(this.#t=Da.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(Ot?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(Da.clearTimeout(this.#t),this.#t=void 0)}};var lv=class extends kl{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=ov(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=ov(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=Oi(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(De).catch(De):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>Lt(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===ts||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>$a(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!xl(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(l=>l.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=Sl(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=_l({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof Nl&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,l)=>{this.#i({type:"failed",failureCount:o,error:l})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof Nl){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,...wd(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),ce.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function wd(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:$d(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function ov(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var vr=class extends Mt{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=Pi(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),uv(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return Sd(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return Sd(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof Lt(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!Tn(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&cv(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||Lt(this.options.enabled,this.#e)!==Lt(t.enabled,this.#e)||$a(this.options.staleTime,this.#e)!==$a(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||Lt(this.options.enabled,this.#e)!==Lt(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return SR(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(De)),t}#v(){this.#x();let e=$a(this.options.staleTime,this.#e);if(Ot||this.#n.isStale||!Ai(e))return;let a=xl(this.#n.dataUpdatedAt,e)+1;this.#u=Da.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(Ot||Lt(this.options.enabled,this.#e)===!1||!Ai(this.#l)||this.#l===0)&&(this.#c=Da.setInterval(()=>{(this.options.refetchIntervalInBackground||as.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(Da.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(Da.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,l=e!==a?e.state:this.#a,{state:c}=e,d={...c},m=!1,f;if(t._optimisticResults){let A=this.hasListeners(),L=!A&&uv(e,t),M=A&&cv(e,a,t,n);(L||M)&&(d={...d,...wd(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:p,errorUpdatedAt:x,status:y}=d;f=d.data;let $=!1;if(t.placeholderData!==void 0&&f===void 0&&y==="pending"){let A;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(A=r.data,$=!0):A=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,A!==void 0&&(y="success",f=Oi(r?.data,A,t),m=!0)}if(t.select&&f!==void 0&&!$)if(r&&f===s?.data&&t.select===this.#f)f=this.#d;else try{this.#f=t.select,f=t.select(f),f=Oi(r?.data,f,t),this.#d=f,this.#i=null}catch(A){this.#i=A}this.#i&&(p=this.#i,f=this.#d,x=Date.now(),y="error");let g=d.fetchStatus==="fetching",v=y==="pending",b=y==="error",w=v&&g,S=f!==void 0,_={status:y,fetchStatus:d.fetchStatus,isPending:v,isSuccess:y==="success",isError:b,isInitialLoading:w,isLoading:w,data:f,dataUpdatedAt:d.dataUpdatedAt,error:p,errorUpdatedAt:x,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>l.dataUpdateCount||d.errorUpdateCount>l.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:b&&!S,isPaused:d.fetchStatus==="paused",isPlaceholderData:m,isRefetchError:b&&S,isStale:Nd(e,t),refetch:this.refetch,promise:this.#o,isEnabled:Lt(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let A=P=>{_.status==="error"?P.reject(_.error):_.data!==void 0&&P.resolve(_.data)},L=()=>{let P=this.#o=_.promise=Pi();A(P)},M=this.#o;switch(M.status){case"pending":e.queryHash===a.queryHash&&A(M);break;case"fulfilled":(_.status==="error"||_.data!==M.value)&&L();break;case"rejected":(_.status!=="error"||_.error!==M.reason)&&L();break}}return _}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),Tn(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){ce.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function wR(e,t){return Lt(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function uv(e,t){return wR(e,t)||e.state.data!==void 0&&Sd(e,t,t.refetchOnMount)}function Sd(e,t,a){if(Lt(t.enabled,e)!==!1&&$a(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&Nd(e,t)}return!1}function cv(e,t,a,n){return(e!==t||Lt(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&Nd(e,a)}function Nd(e,t){return Lt(t.enabled,e)!==!1&&e.isStaleByTime($a(t.staleTime,e))}function SR(e,t){return!Tn(e.getCurrentResult(),t)}function _d(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},l=0,c=async()=>{let d=!1,m=x=>{Object.defineProperty(x,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},f=Sl(t.options,t.fetchOptions),p=async(x,y,$)=>{if(d)return Promise.reject();if(y==null&&x.pages.length)return Promise.resolve(x);let v=(()=>{let C={client:t.client,queryKey:t.queryKey,pageParam:y,direction:$?"backward":"forward",meta:t.options.meta};return m(C),C})(),b=await f(v),{maxPages:w}=t.options,S=$?sv:rv;return{pages:S(x.pages,b,w),pageParams:S(x.pageParams,y,w)}};if(r&&s.length){let x=r==="backward",y=x?NR:dv,$={pages:s,pageParams:i},g=y(n,$);o=await p($,g,x)}else{let x=e??s.length;do{let y=l===0?i[0]??n.initialPageParam:dv(n,o);if(l>0&&y==null)break;o=await p(o,y),l++}while(l<x)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function dv(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function NR(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var mv=class extends kl{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||kd(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=_l({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),ce.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function kd(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var fv=class extends Mt{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new mv({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=Rl(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=Rl(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=Rl(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=Rl(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){ce.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>wl(t,a))}findAll(e={}){return this.getAll().filter(t=>wl(e,t))}notify(e){ce.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return ce.batch(()=>Promise.all(e.map(t=>t.continue().catch(De))))}};function Rl(e){return e.options.scope?.id}var Rd=class extends Mt{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),Tn(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&Ma(t.mutationKey)!==Ma(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??kd();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){ce.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function pv(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function _R(e,t,a){let n=e.slice(0);return n[t]=a,n}var Cd=class extends Mt{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,ce.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,m)=>d!==a[m]),l=i||o,c=l?!0:s.some((d,m)=>{let f=this.#e[m];return!f||!Tn(d,f)});!l&&!c||(l&&(this.#r=r),this.#e=s,this.hasListeners()&&(l&&(pv(a,r).forEach(d=>{d.destroy()}),pv(r,a).forEach(d=>{d.subscribe(m=>{this.#c(d,m)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=Mi(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new vr(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=_R(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&ce.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var hv=class extends Mt{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??Di(n,t),s=this.get(r);return s||(s=new lv({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){ce.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>$l(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>$l(e,a)):t}notify(e){ce.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){ce.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){ce.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var Ed=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new hv,this.#e=e.mutationCache||new fv,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=as.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=ns.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime($a(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=av(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return ce.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;ce.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return ce.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=ce.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(De).catch(De)}invalidateQueries(e,t={}){return ce.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=ce.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(De)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(De)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime($a(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(De).catch(De)}fetchInfiniteQuery(e){return e.behavior=_d(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(De).catch(De)}ensureInfiniteQueryData(e){return e.behavior=_d(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return ns.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(Ma(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{hr(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(Ma(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{hr(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=Di(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===ts&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var Oa=Be(Ke(),1);var rs=Be(Ke(),1),bv=Be(Td(),1),Ad=rs.createContext(void 0),Y=e=>{let t=rs.useContext(Ad);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},Dd=({client:e,children:t})=>(rs.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,bv.jsx)(Ad.Provider,{value:e,children:t}));var El=Be(Ke(),1),xv=El.createContext(!1),Tl=()=>El.useContext(xv),JL=xv.Provider;var Ui=Be(Ke(),1),CR=Be(Td(),1);function ER(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var TR=Ui.createContext(ER()),Al=()=>Ui.useContext(TR);var $v=Be(Ke(),1);var Dl=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},Ml=e=>{$v.useEffect(()=>{e.clearReset()},[e])},Ol=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||Li(a,[e.error,n]));var Ll=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},Pl=(e,t)=>e.isLoading&&e.isFetching&&!t,ji=(e,t)=>e?.suspense&&t.isPending,ss=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function Md({queries:e,...t},a){let n=Y(a),r=Tl(),s=Al(),i=Oa.useMemo(()=>e.map(y=>{let $=n.defaultQueryOptions(y);return $._optimisticResults=r?"isRestoring":"optimistic",$}),[e,n,r]);i.forEach(y=>{Ll(y),Dl(y,s)}),Ml(s);let[o]=Oa.useState(()=>new Cd(n,i,t)),[l,c,d]=o.getOptimisticResult(i,t.combine),m=!r&&t.subscribed!==!1;Oa.useSyncExternalStore(Oa.useCallback(y=>m?o.subscribe(ce.batchCalls(y)):De,[o,m]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),Oa.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let p=l.some((y,$)=>ji(i[$],y))?l.flatMap((y,$)=>{let g=i[$];if(g){let v=new vr(n,g);if(ji(g,y))return ss(g,v,s);Pl(y,r)&&ss(g,v,s)}return[]}):[];if(p.length>0)throw Promise.all(p);let x=l.find((y,$)=>{let g=i[$];return g&&Ol({result:y,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(x?.error)throw x.error;return c(d())}var An=Be(Ke(),1);function wv(e,t,a){let n=Tl(),r=Al(),s=Y(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",Ll(i),Dl(i,r),Ml(r);let o=!s.getQueryCache().get(i.queryHash),[l]=An.useState(()=>new t(s,i)),c=l.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(An.useSyncExternalStore(An.useCallback(m=>{let f=d?l.subscribe(ce.batchCalls(m)):De;return l.updateResult(),f},[l,d]),()=>l.getCurrentResult(),()=>l.getCurrentResult()),An.useEffect(()=>{l.setOptions(i)},[i,l]),ji(i,c))throw ss(i,l,r);if(Ol({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!Ot&&Pl(c,n)&&(o?ss(i,l,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(De).finally(()=>{l.updateResult()}),i.notifyOnChangeProps?c:l.trackResult(c)}function K(e,t){return wv(e,vr,t)}var en=Be(Ke(),1);function Q(e,t){let a=Y(t),[n]=en.useState(()=>new Rd(a,e));en.useEffect(()=>{n.setOptions(e)},[n,e]);let r=en.useSyncExternalStore(en.useCallback(i=>n.subscribe(ce.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=en.useCallback((i,o)=>{n.mutate(i,o).catch(De)},[n]);if(r.error&&Li(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var mR=Be(K0());var ta=Be(Ke(),1),X=Be(Ke(),1),Ee=Be(Ke(),1),Rp=Be(Ke(),1),fx=Be(Ke(),1),be=Be(Ke(),1),iT=Be(Ke(),1),oT=Be(Ke(),1),lT=Be(Ke(),1),W=Be(Ke(),1),Cx=Be(Ke(),1);var H0="popstate";function J0(e={}){function t(n,r){let{pathname:s,search:i,hash:o}=n.location;return dp("",{pathname:s,search:i,hash:o},r.state&&r.state.usr||null,r.state&&r.state.key||"default")}function a(n,r){return typeof r=="string"?r:Xs(r)}return o3(t,a,null,e)}function Ce(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function ea(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function i3(){return Math.random().toString(36).substring(2,10)}function Q0(e,t){return{usr:e.state,key:e.key,idx:t}}function dp(e,t,a=null,n){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?Pr(t):t,state:a,key:t&&t.key||n||i3()}}function Xs({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function Pr(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function o3(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",l=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function m(){o="POP";let $=d(),g=$==null?null:$-c;c=$,l&&l({action:o,location:y.location,delta:g})}function f($,g){o="PUSH";let v=dp(y.location,$,g);a&&a(v,$),c=d()+1;let b=Q0(v,c),w=y.createHref(v);try{i.pushState(b,"",w)}catch(S){if(S instanceof DOMException&&S.name==="DataCloneError")throw S;r.location.assign(w)}s&&l&&l({action:o,location:y.location,delta:1})}function p($,g){o="REPLACE";let v=dp(y.location,$,g);a&&a(v,$),c=d();let b=Q0(v,c),w=y.createHref(v);i.replaceState(b,"",w),s&&l&&l({action:o,location:y.location,delta:0})}function x($){return l3($)}let y={get action(){return o},get location(){return e(r,i)},listen($){if(l)throw new Error("A history only accepts one active listener");return r.addEventListener(H0,m),l=$,()=>{r.removeEventListener(H0,m),l=null}},createHref($){return t(r,$)},createURL:x,encodeLocation($){let g=x($);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:f,replace:p,go($){return i.go($)}};return y}function l3(e,t=!1){let a="http://localhost";typeof window<"u"&&(a=window.location.origin!=="null"?window.location.origin:window.location.href),Ce(a,"No window.location.(origin|href) available to create URL");let n=typeof e=="string"?e:Xs(e);return n=n.replace(/ $/,"%20"),!t&&n.startsWith("//")&&(n=a+n),new URL(n,a)}var u3;u3=new WeakMap;function hp(e,t,a="/"){return c3(e,t,a,!1)}function c3(e,t,a,n){let r=typeof t=="string"?Pr(t):t,s=Ka(r.pathname||"/",a);if(s==null)return null;let i=X0(e);m3(i);let o=null;for(let l=0;o==null&&l<i.length;++l){let c=S3(s);o=$3(i[l],c,n)}return o}function d3(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function X0(e,t=[],a=[],n="",r=!1){let s=(i,o,l=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&l)return;Ce(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let m=bn([n,d.relativePath]),f=a.concat(d);i.children&&i.children.length>0&&(Ce(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${m}".`),X0(i.children,t,f,m,l)),!(i.path==null&&!i.index)&&t.push({path:m,score:b3(m,i.index),routesMeta:f})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let l of Z0(i.path))s(i,o,!0,l)}),t}function Z0(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=Z0(n.join("/")),o=[];return o.push(...i.map(l=>l===""?s:[s,l].join("/"))),r&&o.push(...i),o.map(l=>e.startsWith("/")&&l===""?"/":l)}function m3(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:x3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var f3=/^:[\w-]+$/,p3=3,h3=2,v3=1,g3=10,y3=-2,V0=e=>e==="*";function b3(e,t){let a=e.split("/"),n=a.length;return a.some(V0)&&(n+=y3),t&&(n+=h3),a.filter(r=>!V0(r)).reduce((r,s)=>r+(f3.test(s)?p3:s===""?v3:g3),n)}function x3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function $3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let l=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",m=Vo({path:l.relativePath,caseSensitive:l.caseSensitive,end:c},d),f=l.route;if(!m&&c&&a&&!n[n.length-1].route.index&&(m=Vo({path:l.relativePath,caseSensitive:l.caseSensitive,end:!1},d)),!m)return null;Object.assign(r,m.params),i.push({params:r,pathname:bn([s,m.pathname]),pathnameBase:k3(bn([s,m.pathnameBase])),route:f}),m.pathnameBase!=="/"&&(s=bn([s,m.pathnameBase]))}return i}function Vo(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=w3(e.path,e.caseSensitive,e.end),r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:m},f)=>{if(d==="*"){let x=o[f]||"";i=s.slice(0,s.length-x.length).replace(/(.)\/+$/,"$1")}let p=o[f];return m&&!p?c[d]=void 0:c[d]=(p||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function w3(e,t=!1,a=!0){ea(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,l)=>(n.push({paramName:o,isOptional:l!=null}),l?"/?([^\\/]+)?":"/([^\\/]+)")).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function S3(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return ea(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function Ka(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}function W0(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?Pr(e):e;return{pathname:a?a.startsWith("/")?a:N3(a,t):t,search:R3(n),hash:C3(r)}}function N3(e,t){let a=t.replace(/\/+$/,"").split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function up(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function _3(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function vp(e){let t=_3(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function gp(e,t,a,n=!1){let r;typeof e=="string"?r=Pr(e):(r={...e},Ce(!r.pathname||!r.pathname.includes("?"),up("?","pathname","search",r)),Ce(!r.pathname||!r.pathname.includes("#"),up("#","pathname","hash",r)),Ce(!r.search||!r.search.includes("#"),up("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let m=t.length-1;if(!n&&i.startsWith("..")){let f=i.split("/");for(;f[0]==="..";)f.shift(),m-=1;r.pathname=f.join("/")}o=m>=0?t[m]:"/"}let l=W0(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!l.pathname.endsWith("/")&&(c||d)&&(l.pathname+="/"),l}var bn=e=>e.join("/").replace(/\/\/+/g,"/"),k3=e=>e.replace(/\/+$/,"").replace(/^\/*/,"/"),R3=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,C3=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;function ex(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}var tx=["POST","PUT","PATCH","DELETE"],L6=new Set(tx),E3=["GET",...tx],P6=new Set(E3);var U6=Symbol("ResetLoaderData");var Ur=ta.createContext(null);Ur.displayName="DataRouter";var Zs=ta.createContext(null);Zs.displayName="DataRouterState";var j6=ta.createContext(!1);var yp=ta.createContext({isTransitioning:!1});yp.displayName="ViewTransition";var ax=ta.createContext(new Map);ax.displayName="Fetchers";var T3=ta.createContext(null);T3.displayName="Await";var zt=ta.createContext(null);zt.displayName="Navigation";var Ws=ta.createContext(null);Ws.displayName="Location";var aa=ta.createContext({outlet:null,matches:[],isDataRoute:!1});aa.displayName="Route";var bp=ta.createContext(null);bp.displayName="RouteError";var mp=!0;function nx(e,{relative:t}={}){Ce(jr(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=X.useContext(zt),{hash:r,pathname:s,search:i}=ei(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:bn([a,s])),n.createHref({pathname:o,search:i,hash:r})}function jr(){return X.useContext(Ws)!=null}function Pe(){return Ce(jr(),"useLocation() may be used only in the context of a <Router> component."),X.useContext(Ws).location}var rx="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function sx(e){X.useContext(zt).static||X.useLayoutEffect(e)}function pe(){let{isDataRoute:e}=X.useContext(aa);return e?B3():A3()}function A3(){Ce(jr(),"useNavigate() may be used only in the context of a <Router> component.");let e=X.useContext(Ur),{basename:t,navigator:a}=X.useContext(zt),{matches:n}=X.useContext(aa),{pathname:r}=Pe(),s=JSON.stringify(vp(n)),i=X.useRef(!1);return sx(()=>{i.current=!0}),X.useCallback((l,c={})=>{if(ea(i.current,rx),!i.current)return;if(typeof l=="number"){a.go(l);return}let d=gp(l,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:bn([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var ix=X.createContext(null);function ya(){return X.useContext(ix)}function ox(e){let t=X.useContext(aa).outlet;return t&&X.createElement(ix.Provider,{value:e},t)}function st(){let{matches:e}=X.useContext(aa),t=e[e.length-1];return t?t.params:{}}function ei(e,{relative:t}={}){let{matches:a}=X.useContext(aa),{pathname:n}=Pe(),r=JSON.stringify(vp(a));return X.useMemo(()=>gp(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function lx(e,t){return ux(e,t)}function ux(e,t,a,n,r){Ce(jr(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:s}=X.useContext(zt),{matches:i}=X.useContext(aa),o=i[i.length-1],l=o?o.params:{},c=o?o.pathname:"/",d=o?o.pathnameBase:"/",m=o&&o.route;if(mp){let v=m&&m.path||"";mx(c,!m||v.endsWith("*")||v.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${c}" (under <Route path="${v}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${v}"> to <Route path="${v==="/"?"*":`${v}/*`}">.`)}let f=Pe(),p;if(t){let v=typeof t=="string"?Pr(t):t;Ce(d==="/"||v.pathname?.startsWith(d),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${d}" but pathname "${v.pathname}" was given in the \`location\` prop.`),p=v}else p=f;let x=p.pathname||"/",y=x;if(d!=="/"){let v=d.replace(/^\//,"").split("/");y="/"+x.replace(/^\//,"").split("/").slice(v.length).join("/")}let $=hp(e,{pathname:y});mp&&(ea(m||$!=null,`No routes matched location "${p.pathname}${p.search}${p.hash}" `),ea($==null||$[$.length-1].route.element!==void 0||$[$.length-1].route.Component!==void 0||$[$.length-1].route.lazy!==void 0,`Matched leaf route at location "${p.pathname}${p.search}${p.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let g=P3($&&$.map(v=>Object.assign({},v,{params:Object.assign({},l,v.params),pathname:bn([d,s.encodeLocation?s.encodeLocation(v.pathname).pathname:v.pathname]),pathnameBase:v.pathnameBase==="/"?d:bn([d,s.encodeLocation?s.encodeLocation(v.pathnameBase).pathname:v.pathnameBase])})),i,a,n,r);return t&&g?X.createElement(Ws.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",...p},navigationType:"POP"}},g):g}function D3(){let e=dx(),t=ex(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return mp&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=X.createElement(X.Fragment,null,X.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),X.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",X.createElement("code",{style:s},"ErrorBoundary")," or"," ",X.createElement("code",{style:s},"errorElement")," prop on your route."))),X.createElement(X.Fragment,null,X.createElement("h2",null,"Unexpected Application Error!"),X.createElement("h3",{style:{fontStyle:"italic"}},t),a?X.createElement("pre",{style:r},a):null,i)}var M3=X.createElement(D3,null),O3=class extends X.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.unstable_onError?this.props.unstable_onError(e,t):console.error("React Router caught the following error during render",e)}render(){return this.state.error!==void 0?X.createElement(aa.Provider,{value:this.props.routeContext},X.createElement(bp.Provider,{value:this.state.error,children:this.props.component})):this.props.children}};function L3({routeContext:e,match:t,children:a}){let n=X.useContext(Ur);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),X.createElement(aa.Provider,{value:e},a)}function P3(e,t=[],a=null,n=null,r=null){if(e==null){if(!a)return null;if(a.errors)e=a.matches;else if(t.length===0&&!a.initialized&&a.matches.length>0)e=a.matches;else return null}let s=e,i=a?.errors;if(i!=null){let c=s.findIndex(d=>d.route.id&&i?.[d.route.id]!==void 0);Ce(c>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(i).join(",")}`),s=s.slice(0,Math.min(s.length,c+1))}let o=!1,l=-1;if(a)for(let c=0;c<s.length;c++){let d=s[c];if((d.route.HydrateFallback||d.route.hydrateFallbackElement)&&(l=c),d.route.id){let{loaderData:m,errors:f}=a,p=d.route.loader&&!m.hasOwnProperty(d.route.id)&&(!f||f[d.route.id]===void 0);if(d.route.lazy||p){o=!0,l>=0?s=s.slice(0,l+1):s=[s[0]];break}}}return s.reduceRight((c,d,m)=>{let f,p=!1,x=null,y=null;a&&(f=i&&d.route.id?i[d.route.id]:void 0,x=d.route.errorElement||M3,o&&(l<0&&m===0?(mx("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),p=!0,y=null):l===m&&(p=!0,y=d.route.hydrateFallbackElement||null)));let $=t.concat(s.slice(0,m+1)),g=()=>{let v;return f?v=x:p?v=y:d.route.Component?v=X.createElement(d.route.Component,null):d.route.element?v=d.route.element:v=c,X.createElement(L3,{match:d,routeContext:{outlet:c,matches:$,isDataRoute:a!=null},children:v})};return a&&(d.route.ErrorBoundary||d.route.errorElement||m===0)?X.createElement(O3,{location:a.location,revalidation:a.revalidation,component:x,error:f,children:g(),routeContext:{outlet:null,matches:$,isDataRoute:!0},unstable_onError:n}):g()},null)}function xp(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function U3(e){let t=X.useContext(Ur);return Ce(t,xp(e)),t}function $p(e){let t=X.useContext(Zs);return Ce(t,xp(e)),t}function j3(e){let t=X.useContext(aa);return Ce(t,xp(e)),t}function wp(e){let t=j3(e),a=t.matches[t.matches.length-1];return Ce(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function F3(){return wp("useRouteId")}function cx(){return $p("useNavigation").navigation}function Sp(){let{matches:e,loaderData:t}=$p("useMatches");return X.useMemo(()=>e.map(a=>d3(a,t)),[e,t])}function dx(){let e=X.useContext(bp),t=$p("useRouteError"),a=wp("useRouteError");return e!==void 0?e:t.errors?.[a]}function B3(){let{router:e}=U3("useNavigate"),t=wp("useNavigate"),a=X.useRef(!1);return sx(()=>{a.current=!0}),X.useCallback(async(r,s={})=>{ea(a.current,rx),a.current&&(typeof r=="number"?e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var G0={};function mx(e,t,a){!t&&!G0[e]&&(G0[e]=!0,ea(!1,a))}var F6=Ee.memo(z3);function z3({routes:e,future:t,state:a,unstable_onError:n}){return ux(e,void 0,a,n,t)}function it({to:e,replace:t,state:a,relative:n}){Ce(jr(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=Ee.useContext(zt);ea(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=Ee.useContext(aa),{pathname:i}=Pe(),o=pe(),l=gp(e,vp(s),i,n==="path"),c=JSON.stringify(l);return Ee.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function Np(e){return ox(e.context)}function xe(e){Ce(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function _p({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1}){Ce(!jr(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let i=e.replace(/^\/*/,"/"),o=Ee.useMemo(()=>({basename:i,navigator:r,static:s,future:{}}),[i,r,s]);typeof a=="string"&&(a=Pr(a));let{pathname:l="/",search:c="",hash:d="",state:m=null,key:f="default"}=a,p=Ee.useMemo(()=>{let x=Ka(l,i);return x==null?null:{location:{pathname:x,search:c,hash:d,state:m,key:f},navigationType:n}},[i,l,c,d,m,f,n]);return ea(p!=null,`<Router basename="${i}"> is not able to match the URL "${l}${c}${d}" because it does not start with the basename, so the <Router> won't render anything.`),p==null?null:Ee.createElement(zt.Provider,{value:o},Ee.createElement(Ws.Provider,{children:t,value:p}))}function kp({children:e,location:t}){return lx(fc(e),t)}function fc(e,t=[]){let a=[];return Ee.Children.forEach(e,(n,r)=>{if(!Ee.isValidElement(n))return;let s=[...t,r];if(n.type===Ee.Fragment){a.push.apply(a,fc(n.props.children,s));return}Ce(n.type===xe,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),Ce(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=fc(n.props.children,s)),a.push(i)}),a}var dc="get",mc="application/x-www-form-urlencoded";function pc(e){return e!=null&&typeof e.tagName=="string"}function q3(e){return pc(e)&&e.tagName.toLowerCase()==="button"}function I3(e){return pc(e)&&e.tagName.toLowerCase()==="form"}function K3(e){return pc(e)&&e.tagName.toLowerCase()==="input"}function H3(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function Q3(e,t){return e.button===0&&(!t||t==="_self")&&!H3(e)}var uc=null;function V3(){if(uc===null)try{new FormData(document.createElement("form"),0),uc=!1}catch{uc=!0}return uc}var G3=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function cp(e){return e!=null&&!G3.has(e)?(ea(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${mc}"`),null):e}function Y3(e,t){let a,n,r,s,i;if(I3(e)){let o=e.getAttribute("action");n=o?Ka(o,t):null,a=e.getAttribute("method")||dc,r=cp(e.getAttribute("enctype"))||mc,s=new FormData(e)}else if(q3(e)||K3(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let l=e.getAttribute("formaction")||o.getAttribute("action");if(n=l?Ka(l,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||dc,r=cp(e.getAttribute("formenctype"))||cp(o.getAttribute("enctype"))||mc,s=new FormData(o,e),!V3()){let{name:c,type:d,value:m}=e;if(d==="image"){let f=c?`${c}.`:"";s.append(`${f}x`,"0"),s.append(`${f}y`,"0")}else c&&s.append(c,m)}}else{if(pc(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=dc,n=null,r=mc,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var B6=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");function Cp(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var J3=Symbol("SingleFetchRedirect");function X3(e,t,a){let n=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return n.pathname==="/"?n.pathname=`_root.${a}`:t&&Ka(n.pathname,t)==="/"?n.pathname=`${t.replace(/\/$/,"")}/_root.${a}`:n.pathname=`${n.pathname.replace(/\/$/,"")}.${a}`,n}async function Z3(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function W3(e){return e!=null&&typeof e.page=="string"}function eT(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function tT(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await Z3(s,a);return i.links?i.links():[]}return[]}));return sT(n.flat(1).filter(eT).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function Y0(e,t,a,n,r,s){let i=(l,c)=>a[c]?l.route.id!==a[c].route.id:!0,o=(l,c)=>a[c].pathname!==l.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==l.params["*"];return s==="assets"?t.filter((l,c)=>i(l,c)||o(l,c)):s==="data"?t.filter((l,c)=>{let d=n.routes[l.route.id];if(!d||!d.hasLoader)return!1;if(i(l,c)||o(l,c))return!0;if(l.route.shouldRevalidate){let m=l.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:l.params,defaultShouldRevalidate:!0});if(typeof m=="boolean")return m}return!0}):[]}function aT(e,t,{includeHydrateFallback:a}={}){return nT(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function nT(e){return[...new Set(e)]}function rT(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function sT(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!W3(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(rT(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function px(){let e=be.useContext(Ur);return Cp(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function uT(){let e=be.useContext(Zs);return Cp(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var Go=be.createContext(void 0);Go.displayName="FrameworkContext";function hx(){let e=be.useContext(Go);return Cp(e,"You must render this element inside a <HydratedRouter> element"),e}function cT(e,t){let a=be.useContext(Go),[n,r]=be.useState(!1),[s,i]=be.useState(!1),{onFocus:o,onBlur:l,onMouseEnter:c,onMouseLeave:d,onTouchStart:m}=t,f=be.useRef(null);be.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let y=g=>{g.forEach(v=>{i(v.isIntersecting)})},$=new IntersectionObserver(y,{threshold:.5});return f.current&&$.observe(f.current),()=>{$.disconnect()}}},[e]),be.useEffect(()=>{if(n){let y=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(y)}}},[n]);let p=()=>{r(!0)},x=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,f,{}]:[s,f,{onFocus:Qo(o,p),onBlur:Qo(l,x),onMouseEnter:Qo(c,p),onMouseLeave:Qo(d,x),onTouchStart:Qo(m,p)}]:[!1,f,{}]}function Qo(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function vx({page:e,...t}){let{router:a}=px(),n=be.useMemo(()=>hp(a.routes,e,a.basename),[a.routes,e,a.basename]);return n?be.createElement(mT,{page:e,matches:n,...t}):null}function dT(e){let{manifest:t,routeModules:a}=hx(),[n,r]=be.useState([]);return be.useEffect(()=>{let s=!1;return tT(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function mT({page:e,matches:t,...a}){let n=Pe(),{manifest:r,routeModules:s}=hx(),{basename:i}=px(),{loaderData:o,matches:l}=uT(),c=be.useMemo(()=>Y0(e,t,l,r,n,"data"),[e,t,l,r,n]),d=be.useMemo(()=>Y0(e,t,l,r,n,"assets"),[e,t,l,r,n]),m=be.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let x=new Set,y=!1;if(t.forEach(g=>{let v=r.routes[g.route.id];!v||!v.hasLoader||(!c.some(b=>b.route.id===g.route.id)&&g.route.id in o&&s[g.route.id]?.shouldRevalidate||v.hasClientLoader?y=!0:x.add(g.route.id))}),x.size===0)return[];let $=X3(e,i,"data");return y&&x.size>0&&$.searchParams.set("_routes",t.filter(g=>x.has(g.route.id)).map(g=>g.route.id).join(",")),[$.pathname+$.search]},[i,o,n,r,c,t,e,s]),f=be.useMemo(()=>aT(d,r),[d,r]),p=dT(d);return be.createElement(be.Fragment,null,m.map(x=>be.createElement("link",{key:x,rel:"prefetch",as:"fetch",href:x,...a})),f.map(x=>be.createElement("link",{key:x,rel:"modulepreload",href:x,...a})),p.map(({key:x,link:y})=>be.createElement("link",{key:x,nonce:a.nonce,...y})))}function fT(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var gx=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{gx&&(window.__reactRouterVersion="7.9.1")}catch{}function Ep({basename:e,children:t,window:a}){let n=W.useRef();n.current==null&&(n.current=J0({window:a,v5Compat:!0}));let r=n.current,[s,i]=W.useState({action:r.action,location:r.location}),o=W.useCallback(l=>{W.startTransition(()=>i(l))},[i]);return W.useLayoutEffect(()=>r.listen(o),[r,o]),W.createElement(_p,{basename:e,children:t,location:s.location,navigationType:s.action,navigator:r})}function yx({basename:e,children:t,history:a}){let[n,r]=W.useState({action:a.action,location:a.location}),s=W.useCallback(i=>{W.startTransition(()=>r(i))},[r]);return W.useLayoutEffect(()=>a.listen(s),[a,s]),W.createElement(_p,{basename:e,children:t,location:n.location,navigationType:n.action,navigator:a})}yx.displayName="unstable_HistoryRouter";var bx=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i,Fr=W.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,state:o,target:l,to:c,preventScrollReset:d,viewTransition:m,...f},p){let{basename:x}=W.useContext(zt),y=typeof c=="string"&&bx.test(c),$,g=!1;if(typeof c=="string"&&y&&($=c,gx))try{let L=new URL(window.location.href),M=c.startsWith("//")?new URL(L.protocol+c):new URL(c),P=Ka(M.pathname,x);M.origin===L.origin&&P!=null?c=P+M.search+M.hash:g=!0}catch{ea(!1,`<Link to="${c}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}let v=nx(c,{relative:r}),[b,w,S]=cT(n,f),C=Sx(c,{replace:i,state:o,target:l,preventScrollReset:d,relative:r,viewTransition:m});function _(L){t&&t(L),L.defaultPrevented||C(L)}let A=W.createElement("a",{...f,...S,href:$||v,onClick:g||s?t:_,ref:fT(p,w),target:l,"data-discover":!y&&a==="render"?"true":void 0});return b&&!y?W.createElement(W.Fragment,null,A,W.createElement(vx,{page:v})):A});Fr.displayName="Link";var Ha=W.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:l,...c},d){let m=ei(i,{relative:c.relative}),f=Pe(),p=W.useContext(Zs),{navigator:x,basename:y}=W.useContext(zt),$=p!=null&&Rx(m)&&o===!0,g=x.encodeLocation?x.encodeLocation(m).pathname:m.pathname,v=f.pathname,b=p&&p.navigation&&p.navigation.location?p.navigation.location.pathname:null;a||(v=v.toLowerCase(),b=b?b.toLowerCase():null,g=g.toLowerCase()),b&&y&&(b=Ka(b,y)||b);let w=g!=="/"&&g.endsWith("/")?g.length-1:g.length,S=v===g||!r&&v.startsWith(g)&&v.charAt(w)==="/",C=b!=null&&(b===g||!r&&b.startsWith(g)&&b.charAt(g.length)==="/"),_={isActive:S,isPending:C,isTransitioning:$},A=S?t:void 0,L;typeof n=="function"?L=n(_):L=[n,S?"active":null,C?"pending":null,$?"transitioning":null].filter(Boolean).join(" ");let M=typeof s=="function"?s(_):s;return W.createElement(Fr,{...c,"aria-current":A,className:L,ref:d,style:M,to:i,viewTransition:o},typeof l=="function"?l(_):l)});Ha.displayName="NavLink";var xx=W.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=dc,action:o,onSubmit:l,relative:c,preventScrollReset:d,viewTransition:m,...f},p)=>{let x=Nx(),y=_x(o,{relative:c}),$=i.toLowerCase()==="get"?"get":"post",g=typeof o=="string"&&bx.test(o);return W.createElement("form",{ref:p,method:$,action:y,onSubmit:n?l:b=>{if(l&&l(b),b.defaultPrevented)return;b.preventDefault();let w=b.nativeEvent.submitter,S=w?.getAttribute("formmethod")||i;x(w||b.currentTarget,{fetcherKey:t,method:S,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:m})},...f,"data-discover":!g&&e==="render"?"true":void 0})});xx.displayName="Form";function $x({getKey:e,storageKey:t,...a}){let n=W.useContext(Go),{basename:r}=W.useContext(zt),s=Pe(),i=Sp();kx({getKey:e,storageKey:t});let o=W.useMemo(()=>{if(!n||!e)return null;let c=pp(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let l=((c,d)=>{if(!window.history.state||!window.history.state.key){let m=Math.random().toString(32).slice(2);window.history.replaceState({key:m},"")}try{let f=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof f=="number"&&window.scrollTo(0,f)}catch(m){console.error(m),sessionStorage.removeItem(c)}}).toString();return W.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${l})(${JSON.stringify(t||fp)}, ${JSON.stringify(o)})`}})}$x.displayName="ScrollRestoration";function wx(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function Tp(e){let t=W.useContext(Ur);return Ce(t,wx(e)),t}function pT(e){let t=W.useContext(Zs);return Ce(t,wx(e)),t}function Sx(e,{target:t,replace:a,state:n,preventScrollReset:r,relative:s,viewTransition:i}={}){let o=pe(),l=Pe(),c=ei(e,{relative:s});return W.useCallback(d=>{if(Q3(d,t)){d.preventDefault();let m=a!==void 0?a:Xs(l)===Xs(c);o(e,{replace:m,state:n,preventScrollReset:r,relative:s,viewTransition:i})}},[l,o,c,a,n,t,e,r,s,i])}var hT=0,vT=()=>`__${String(++hT)}__`;function Nx(){let{router:e}=Tp("useSubmit"),{basename:t}=W.useContext(zt),a=F3();return W.useCallback(async(n,r={})=>{let{action:s,method:i,encType:o,formData:l,body:c}=Y3(n,t);if(r.navigate===!1){let d=r.fetcherKey||vT();await e.fetch(d,a,r.action||s,{preventScrollReset:r.preventScrollReset,formData:l,body:c,formMethod:r.method||i,formEncType:r.encType||o,flushSync:r.flushSync})}else await e.navigate(r.action||s,{preventScrollReset:r.preventScrollReset,formData:l,body:c,formMethod:r.method||i,formEncType:r.encType||o,replace:r.replace,state:r.state,fromRouteId:a,flushSync:r.flushSync,viewTransition:r.viewTransition})},[e,t,a])}function _x(e,{relative:t}={}){let{basename:a}=W.useContext(zt),n=W.useContext(aa);Ce(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...ei(e||".",{relative:t})},i=Pe();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),l=o.getAll("index");if(l.some(d=>d==="")){o.delete("index"),l.filter(m=>m).forEach(m=>o.append("index",m));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:bn([a,s.pathname])),Xs(s)}var fp="react-router-scroll-positions",cc={};function pp(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:Ka(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function kx({getKey:e,storageKey:t}={}){let{router:a}=Tp("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=pT("useScrollRestoration"),{basename:s}=W.useContext(zt),i=Pe(),o=Sp(),l=cx();W.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),gT(W.useCallback(()=>{if(l.state==="idle"){let c=pp(i,o,s,e);cc[c]=window.scrollY}try{sessionStorage.setItem(t||fp,JSON.stringify(cc))}catch(c){ea(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[l.state,e,s,i,o,t])),typeof document<"u"&&(W.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||fp);c&&(cc=JSON.parse(c))}catch{}},[t]),W.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(cc,()=>window.scrollY,e?(d,m)=>pp(d,m,s,e):void 0);return()=>c&&c()},[a,s,e]),W.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{ea(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function gT(e,t){let{capture:a}=t||{};W.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function Rx(e,{relative:t}={}){let a=W.useContext(yp);Ce(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=Tp("useViewTransitionState"),r=ei(e,{relative:t});if(!a.isTransitioning)return!1;let s=Ka(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=Ka(a.nextLocation.pathname,n)||a.nextLocation.pathname;return Vo(r.pathname,i)!=null||Vo(r.pathname,s)!=null}var Ct=new Ed({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var Ap="ironclaw_token",Ie="/api/webchat/v2",Br=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function ba(){return sessionStorage.getItem(Ap)||""}function ti(e){e?sessionStorage.setItem(Ap,e):sessionStorage.removeItem(Ap)}function hc(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function Tx(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function Ex(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function Ax({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=Ex(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=Ex(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function H(e,t={}){let a=ba(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await Tx(r);throw new Br(Ax({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function vc(){return H(`${Ie}/session`)}function gc({clientActionId:e,requestedThreadId:t,projectId:a}={}){let n={client_action_id:e||hc()};return t&&(n.requested_thread_id=t),a&&(n.project_id=a),H(`${Ie}/threads`,{method:"POST",body:JSON.stringify(n)})}function Dx({limit:e,cursor:t}={}){let a=new URL(`${Ie}/threads`,window.location.origin);return e!=null&&a.searchParams.set("limit",String(e)),t&&a.searchParams.set("cursor",t),H(a.pathname+a.search)}function Mx({threadId:e}={}){return e?H(`${Ie}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function Dp(e){return`${Ie}/threads/${encodeURIComponent(e)}/files`}function Ox({threadId:e,path:t}={}){if(!e)return Promise.reject(new Error("threadId is required"));let a=new URL(Dp(e),window.location.origin);return t&&a.searchParams.set("path",t),H(a.pathname+a.search)}function Lx({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${Dp(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),H(a.pathname+a.search)}function yc({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${Dp(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function Px({limit:e,runLimit:t,includeCompleted:a}={}){let n=new URLSearchParams;e!=null&&n.set("limit",String(e)),t!=null&&n.set("run_limit",String(t)),a===!0&&n.set("include_completed","true");let r=n.toString();return H(`${Ie}/automations${r?`?${r}`:""}`)}function Ux({automationId:e}={}){return e?H(`${Ie}/automations/${encodeURIComponent(e)}/pause`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function jx({automationId:e}={}){return e?H(`${Ie}/automations/${encodeURIComponent(e)}/resume`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function Fx({automationId:e}={}){return e?H(`${Ie}/automations/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("automationId is required"))}var Bx=`${Ie}/projects`;function yT(e){return`${Bx}/${encodeURIComponent(e)}`}function zx({limit:e}={}){let t=new URL(Bx,window.location.origin);return e!=null&&t.searchParams.set("limit",String(e)),H(t.pathname+t.search)}function qx({projectId:e}={}){return e?H(yT(e)):Promise.reject(new Error("projectId is required"))}function Ix(){return H(`${Ie}/outbound/preferences`)}function Kx(){return H(`${Ie}/outbound/targets`)}function Hx({finalReplyTargetId:e}={}){return H(`${Ie}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function Mp({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:l,source:c,tail:d,follow:m}={}){let f=new URL(`${Ie}/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),l&&f.searchParams.set("tool_name",l),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),H(f.pathname+f.search)}function Qx({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:l,source:c,tail:d,follow:m}={}){let f=new URL(`${Ie}/operator/logs`,window.location.origin);return e!=null&&f.searchParams.set("limit",String(e)),t&&f.searchParams.set("cursor",t),a&&f.searchParams.set("level",a),n&&f.searchParams.set("target",n),r&&f.searchParams.set("thread_id",r),s&&f.searchParams.set("run_id",s),i&&f.searchParams.set("turn_id",i),o&&f.searchParams.set("tool_call_id",o),l&&f.searchParams.set("tool_name",l),c&&f.searchParams.set("source",c),d&&f.searchParams.set("tail","true"),m&&f.searchParams.set("follow","true"),H(f.pathname+f.search)}function Vx({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||hc(),content:t};return a.length>0&&(r.attachments=a),H(`${Ie}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function Gx({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${Ie}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),H(n.pathname+n.search)}function Yx({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${Ie}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function _a(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new Br("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=ba(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await Tx(r);throw new Br(Ax({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function Op(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function bc(e){return Op(await _a(e))}function Jx({threadId:e,afterCursor:t}={}){let a=new URL(`${Ie}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=ba();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function Xx({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||hc()};return a&&(r.reason=a),H(`${Ie}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function Lp({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let l={client_action_id:i||hc(),resolution:n};return r!=null&&(l.always=r),s&&(l.credential_ref=s),H(`${Ie}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(l)})}function Zx({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return H("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function Wx(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),H(`${Ie}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function ai(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function e$(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function t$(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new Br("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new Br("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function a$(){let e=ba();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var xc="anon",n$=xc;function r$(e){n$=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:xc}function St(){return n$}var s$="ironclaw:v2-thread-pins:",Pp=new Set,xn=new Set,Up=null;function jp(){return`${s$}${St()}`}function bT(){try{let e=window.localStorage.getItem(jp());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function xT(){try{xn.size===0?window.localStorage.removeItem(jp()):window.localStorage.setItem(jp(),JSON.stringify([...xn]))}catch{}}function i$(){let e=St();if(e!==Up){xn.clear();for(let t of bT())xn.add(t);Up=e}}function o$(){return new Set(xn)}function l$(){let e=o$();for(let t of Pp)try{t(e)}catch{}}function u$(e){e&&(i$(),xn.has(e)?xn.delete(e):xn.add(e),xT(),l$())}function c$(){return i$(),o$()}function d$(e){return Pp.add(e),()=>{Pp.delete(e)}}function m$(){xn.clear(),Up=St();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(s$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}l$()}var $T=0,zr={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function Fp(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function f$(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":wT(t)?"text":"download"}function wT(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function Yo(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function ST(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function NT(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function _T(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function p$(e,{limits:t,existing:a=[],t:n}){let r=t||zr,s=[],i=[],o=a.length,l=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!ST(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:Yo(r.maxFileBytes)}));continue}if(l+c.size>r.maxTotalBytes){let y=n("chat.attachmentTotalTooLarge",{max:Yo(r.maxTotalBytes)});i.includes(y)||i.push(y);continue}let d;try{d=await NT(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:m,base64:f}=_T(d,c.type),p=m||"application/octet-stream",x=Fp(p);s.push({id:`staged-${$T++}`,filename:c.name||"attachment",mimeType:p,kind:x,sizeBytes:c.size,sizeLabel:Yo(c.size),dataBase64:f,previewUrl:x==="image"?d:null}),o+=1,l+=c.size}return{staged:s,errors:i}}function h$(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function v$(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}function kT(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||Fp(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?Yx({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?Yo(n.size_bytes):"",preview_url:null,fetch_url:s}})}function y$(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let c=TT(s);if(!c)continue;let d=`tool-${c.invocationId}`;if(n.has(d))continue;n.add(d),r.push({id:d,role:"tool_activity",...c,timestamp:g$(s)||c.updatedAt||null,sequence:s.sequence,activityOrder:c.activityOrder,activityOrderSource:c.activityOrderSource,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=ET(s),l=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy");r.push({id:i,role:o,content:s.content||"",attachments:kT(s,a),timestamp:g$(s),kind:s.kind,status:l?"error":s.status,...l&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:CT(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=RT(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function RT(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function CT(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function ET(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function g$(e){return e.received_at||e.created_at||null}function TT(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:Bp(t)}var AT="gate_declined";function Bp(e){let t=e.status==="failed"||e.status==="killed",a=e.error_kind||null,n=$$(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Xo(e.title||e.capability_id)||"tool",toolStatus:x$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(b$(a)||e.output_summary||e.output_preview||e.result_ref)||null,toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null,activityOrder:n,activityOrderSource:Number.isFinite(n)?"projection":null}}function zp(e){let t=$$(e.activity_order),a=e.error_kind||null;return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Xo(e.capability_id)||"tool",toolStatus:x$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:null,toolError:b$(a),toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null,activityOrder:t,activityOrderSource:Number.isFinite(t)?"projection":null}}function b$(e){return e||null}function Jo(e){return e==="success"||e==="error"||e==="declined"}function Xo(e){let t=typeof e=="string"?e.trim():"";if(!t)return"";let a=t.split(".");return a[a.length-1]||t}function x$(e,t=null){if(t===AT)return"declined";switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}function $$(e){let t=Number(e);return Number.isFinite(t)?t:null}var DT=50,Qa=new Map,MT=30;function Zo(e,t){for(Qa.delete(e),Qa.set(e,t);Qa.size>MT;){let a=Qa.keys().next().value;Qa.delete(a)}}function Wo(e){return`${St()}:${e}`}function S$(){Qa.clear()}function N$(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?Qa.get(Wo(e)):null,[s,i]=h.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),o=h.default.useRef(new Set),l=h.default.useRef(e);l.current=e;let c=h.default.useCallback(async(m,f={})=>{let{preserveClientOnly:p=!1,finalReplyTimestampByRun:x=null}=f;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(o.current.has(e))return;o.current.add(e);let y=St(),$=Wo(e);i(g=>({...g,isLoading:!0}));try{let g=await Gx({threadId:e,limit:DT,cursor:m});if(St()!==y)return;let v=m?[]:a?.()||[],b=y$(g.messages||[],v,e),w=g.next_cursor||null;if(m||n?.([]),!m){let S=Qa.get($)?.messages||[],C=w$(b,S,{preserveClientOnly:p,finalReplyTimestampByRun:x});Zo($,{messages:C,nextCursor:w})}i(S=>{if(l.current!==e)return S;let C;return m?C=OT(b,S.messages):C=w$(b,S.messages,{preserveClientOnly:p,finalReplyTimestampByRun:x}),Zo($,{messages:C,nextCursor:w}),{messages:C,nextCursor:w,isLoading:!1,loadError:null}})}catch(g){if(console.error("Failed to load timeline:",g),St()!==y)return;i(v=>l.current===e?{...v,isLoading:!1,loadError:"Failed to load conversation history."}:v)}finally{o.current.delete(e)}},[e,a,n]);h.default.useEffect(()=>{let m=e?Qa.get(Wo(e)):null;i({messages:m?.messages||[],nextCursor:m?.nextCursor||null,isLoading:!!e&&!m,loadError:null}),e&&c()},[e,c]);let d=h.default.useCallback((m,f)=>{if(!m)return;let p=Wo(m),x=g=>typeof f=="function"?f(g||[]):f;if(l.current===m){i(g=>{let v=x(g.messages||[]);return Zo(p,{messages:v,nextCursor:g.nextCursor||null}),{...g,messages:v}});return}let y=Qa.get(p)||{messages:[],nextCursor:null},$=x(y.messages||[]);Zo(p,{messages:$,nextCursor:y.nextCursor||null})},[]);return{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:c,seedThreadMessages:d,setMessages:m=>i(f=>{let p=typeof m=="function"?m(f.messages):m;return e&&Zo(Wo(e),{messages:p,nextCursor:f.nextCursor}),{...f,messages:p}})}}function OT(e,t){let a=new Set(t.map(n=>n?.id).filter(Boolean));return[...e.filter(n=>!a.has(n?.id)),...t]}function w$(e,t,a={}){let{preserveClientOnly:n=!1,finalReplyTimestampByRun:r=null}=a,s=FT(e,t,{finalReplyTimestampByRun:r}),i=new Set(s.map(l=>l?.id).filter(Boolean)),o=t.filter(l=>!l||typeof l.id!="string"||i.has(l.id)?!1:_$(l)?!0:typeof l.timelineMessageId=="string"&&i.has(`msg-${l.timelineMessageId}`)?!1:jT(l)?!0:n&&l.id.startsWith("err-"));return LT(s,o)}function LT(e,t){if(t.length===0)return e;let a=new Map;for(let i=0;i<e.length;i+=1){let o=qp(e[i]);o&&a.set(o,i)}let n=new Map,r=[];for(let i of t){let o=PT(i)?qp(i):null;if(o&&a.has(o)){let l=n.get(o)||[];l.push(i),n.set(o,l)}else r.push(i)}if(n.size===0)return[...e,...r];let s=[];for(let i=0;i<e.length;i+=1){let o=e[i];s.push(o);let l=qp(o);l&&a.get(l)===i&&s.push(...n.get(l)||[])}return r.length>0?[...s,...r]:s}function PT(e){return _$(e)||UT(e)}function UT(e){return e?.role==="error"&&typeof e.id=="string"&&e.id.startsWith("err-")}function qp(e){return typeof e?.turnRunId=="string"&&e.turnRunId?e.turnRunId:null}function jT(e){return e?.isOptimistic===!0&&typeof e.id=="string"&&e.id.startsWith("pending-")&&(e.role==="user"||e.role==="assistant")}function FT(e,t,a={}){let{finalReplyTimestampByRun:n=null}=a,r=new Map,s=new Map;for(let i of t||[])!i||!i.timestamp||(typeof i.id=="string"&&r.set(i.id,i),typeof i.timelineMessageId=="string"&&r.set(`msg-${i.timelineMessageId}`,i),Ip(i)&&typeof i.turnRunId=="string"&&s.set(i.turnRunId,i));return r.size===0&&s.size===0&&!n?e:e.map(i=>{if(!i||i.timestamp||typeof i.id!="string")return i;let o=typeof i.turnRunId=="string"?i.turnRunId:null,l=r.get(i.id)||(Ip(i)&&o?s.get(o):null),c=Ip(i)&&o?n?.[o]:null,d=l?.timestamp||c;return d?{...i,timestamp:d}:i})}function Ip(e){return e?.role==="assistant"&&e?.isFinalReply===!0}function _$(e){return e?.role==="tool_activity"||e?.role==="thinking"}var tl="__new__",k$="ironclaw:v2-draft:";function ni(e){return`${k$}${St()}:${e||tl}`}function Kp(e){try{return window.localStorage.getItem(ni(e))||""}catch{return""}}function Hp(e,t){try{t?window.localStorage.setItem(ni(e),t):window.localStorage.removeItem(ni(e))}catch{}}function R$(e){Hp(e,"")}var el=new Map;function Qp(e){return el.get(ni(e))||[]}function C$(e,t){let a=ni(e);t&&t.length>0?el.set(a,t):el.delete(a)}function E$(e){el.delete(ni(e))}function T$(){el.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(k$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function BT(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function zT(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function qT(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=BT(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?zT(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),ba()?"":(ti(n),n)}function IT(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var KT={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function HT(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),KT[t]||"Could not complete sign-in. Please try again."):""}function A$(){let[e,t]=h.default.useState(()=>qT()||ba()),[a,n]=h.default.useState(()=>HT()),[r]=h.default.useState(()=>IT()),[s,i]=h.default.useState(null),[o,l]=h.default.useState(()=>!!(r&&!ba())),[c,d]=h.default.useState(()=>!!ba());h.default.useEffect(()=>{if(!r||ba()){l(!1);return}let x=!1;return t$(r).then(y=>{x||(ti(y),d(!0),t(y),i(null),n(""),l(!1),Ct.clear())}).catch(()=>{x||(n("Could not complete sign-in. Please try again."),l(!1))}),()=>{x=!0}},[r]),h.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let x=!1;return d(!0),vc().then(y=>{x||(i(y),d(!1))}).catch(y=>{x||(i(null),d(!1),(y?.status===401||y?.status===403)&&(ti(""),t(""),n("Your session expired. Please sign in again."),Ct.clear()))}),()=>{x=!0}},[e,o]),r$(s);let m=h.default.useRef(null);h.default.useEffect(()=>{let x=St();m.current&&m.current!==xc&&m.current!==x&&(S$(),T$(),m$()),m.current=x},[s]);let f=h.default.useCallback(x=>{ti(x),d(!!x),t(x),i(null),n(""),Ct.clear()},[]),p=h.default.useCallback(()=>{a$().catch(()=>{}),ti(""),d(!1),t(""),i(null),n(""),Ct.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,rebornProjectsEnabled:!!s?.features?.reborn_projects,signIn:f,signOut:p}}var qr="/chat",al=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace"},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"logs",path:"/logs",labelKey:"nav.logs",hidden:!0},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var QT=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"tools",labelKey:"settings.tools",icon:"tool"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],VT=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],GT=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],$c={settings:QT,extensions:VT,admin:GT};var D$="ironclaw:v2-theme";function YT(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(D$);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function wc(){let[e,t]=h.default.useState(YT);h.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(D$,e)}catch{}},[e]);let a=h.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function M$(e){return K({enabled:!!e,queryKey:["gateway-status",e],queryFn:ai,refetchInterval:3e4})}var JT="/api/webchat/v2/operator/config",Sc="/api/webchat/v2/settings/tools",ri="agent.auto_approve_tools",O$="tool.",XT=new Set(["always_allow","ask_each_time","disabled"]),ZT=new Set(["default","always_allow","ask_each_time","disabled"]);function L$(e){return e==="ask"?"ask_each_time":XT.has(e)?e:"ask_each_time"}function WT(e){return e==="ask"?"ask_each_time":ZT.has(e)?e:"default"}function eA(e){return["default","global","override"].includes(e)?e:"default"}function P$(e){if(!e?.key?.startsWith(O$))return null;let t=e.value||{};return{name:t.name||e.key.slice(O$.length),description:t.description||"",state:L$(t.state),default_state:L$(t.default_state),locked:!!(t.locked||e.mutable===!1),effective_source:eA(t.effective_source||e.source)}}function tA(e){let t={};for(let a of e.entries||[])a?.key===ri&&(t[ri]=!!a.value);return t}async function U$(){let e=await H(Sc);return{settings:tA(e),diagnostics:e.diagnostics||[],precedence:e.precedence||[]}}async function Vp(e,t){if(e===ri){let n=await H(Sc,{method:"POST",body:JSON.stringify({enabled:!!t})});return{success:!0,entry:n.entry,value:n.entry?.value}}let a=await H(`${JT}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({value:t})});return{success:!0,entry:a.entry,value:a.entry?.value}}async function j$(e){let t=e?.settings||{},a=[];return Object.prototype.hasOwnProperty.call(t,ri)&&a.push(await Vp(ri,!!t[ri])),{success:!0,imported:a.length,results:a}}function Nc(){return H("/api/webchat/v2/llm/providers")}function F$(e){return H("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function B$(e){return H(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function nl(e){return H("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function z$(e){return H("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function q$(e){return H("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function I$(e){return H("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function K$(e){return H("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function H$(){return H("/api/webchat/v2/llm/codex/login",{method:"POST"})}async function Q$(){let e=await H(Sc);return{tools:(e.entries||[]).map(P$).filter(Boolean),diagnostics:e.diagnostics||[]}}async function V$(e,t){let a=WT(t),n=await H(`${Sc}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({state:a})});return{success:!0,tool:P$(n.entry),entry:n.entry}}function G$(){return H("/api/webchat/v2/extensions")}function Y$(){return H("/api/webchat/v2/extensions/registry")}function J$(){return H("/api/webchat/v2/skills")}function X$(e){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function Z$(e){return H("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function W$(e,t){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function ew(e){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function tw(e,t){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}/auto-activate`,{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:t})})}function aw(e){return H("/api/webchat/v2/skills/auto-activate-learned",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function nw(){return H("/api/webchat/v2/traces/credit")}function rw(e){return H(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function sw(){return Promise.resolve({users:[],todo:!0})}function iw(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function ow(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var Gp="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",Yp=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function rl(e){return Yp.find(t=>t.value===e)?.label||e}function si(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function lw(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function _c(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function uw(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function Ir(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Gp||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?si(e,t).trim().length>0:!0:!1}function aA(e,t,a){return e.id===a?"active":Ir(e,t)?"ready":"setup"}function cw(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=aA(r,t,a);n[s]&&n[s].push(r)}return n}function kc(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Gp||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!si(e,t).trim()?"base_url":"ok"}function Jp(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===Gp&&(i.api_key=void 0),i}function dw(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function mw(e){return/^[a-z0-9_-]+$/.test(e)}function fw(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var nA=Object.freeze({});function ii({settings:e,gatewayStatus:t,enabled:a=!0}){let n=Y(),r=K({queryKey:["llm-providers"],queryFn:Nc,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=nA,l=(s.providers||[]).map(w=>({...w,name:w.description,has_api_key:w.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,m=d||"nearai",f=s.active?.model||t?.llm_model||"",p=l.filter(w=>w.builtin),x=l.filter(w=>!w.builtin),y=[...l].sort((w,S)=>w.id===d?-1:S.id===d?1:(w.name||w.id).localeCompare(S.name||S.id)),$=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=Q({mutationFn:async w=>{if(!Ir(w,o)){let C=kc(w,o);throw new Error(C==="base_url"?"base_url":"api_key")}let S=_c(w,o);if(!S)throw new Error("model");return await nl({provider_id:w.id,model:S}),w},onSuccess:$}),v=Q({mutationFn:async({provider:w,form:S,apiKey:C,editingProvider:_})=>{let A=!!w?.builtin,M={id:(A?w.id:S.id.trim()).trim(),name:A?w.name||w.id:S.name.trim(),adapter:A?w.adapter:S.adapter,base_url:S.baseUrl.trim()||w?.base_url||"",default_model:S.model.trim()||void 0};return C.trim()&&(M.api_key=C.trim()),(_||w)?.id===m&&M.default_model&&(M.set_active=!0,M.model=M.default_model),await F$(M),M},onSuccess:$}),b=Q({mutationFn:async w=>(await B$(w.id),w),onSuccess:$});return{providers:y,builtinProviders:p,customProviders:x,builtinOverrides:o,activeProviderId:d,selectedModel:f,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:w=>g.mutateAsync(w),saveCustomProvider:w=>v.mutateAsync(w),saveBuiltinProvider:w=>v.mutateAsync(w),deleteCustomProvider:w=>b.mutateAsync(w),testConnection:z$,listModels:q$,isBusy:g.isPending||v.isPending||b.isPending}}function pw({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}var hw="ironclaw:v2-sidebar-open";function vw(){return typeof window>"u"?null:window}function gw(){try{return vw()?.localStorage||null}catch{return null}}function yw(e=gw()){try{return e?.getItem(hw)!=="false"}catch{return!0}}function bw(e,t=gw()){try{t?.setItem(hw,e?"true":"false")}catch{}}function xw(e=vw()){try{return e?.matchMedia?.("(min-width: 768px)").matches===!0}catch{return!1}}function $w(e,t){return t?{...e,desktopOpen:!e.desktopOpen}:{...e,mobileOpen:!e.mobileOpen}}function ww(e,t){return t?e.desktopOpen:e.mobileOpen}function Sw({onNewChat:e}={}){let t=pe(),[a,n]=h.default.useState(()=>({mobileOpen:!1,desktopOpen:yw()})),[r,s]=h.default.useState(()=>xw());h.default.useEffect(()=>{let d=window.matchMedia("(min-width: 768px)"),m=()=>s(d.matches);return m(),d.addEventListener?.("change",m),()=>d.removeEventListener?.("change",m)},[]),h.default.useEffect(()=>{bw(a.desktopOpen)},[a.desktopOpen]);let i=h.default.useCallback(()=>{n(d=>({...d,mobileOpen:!1}))},[]),o=h.default.useCallback(()=>{n(d=>$w(d,r))},[r]),l=h.default.useCallback(async()=>{let d=await e?.(),m=typeof d=="string"&&d.length>0?d:null;t(m?`/chat/${m}`:"/chat"),i()},[t,i,e]),c=h.default.useCallback(d=>{t(`/chat/${d}`),i()},[t,i]);return{mobileOpen:a.mobileOpen,desktopOpen:a.desktopOpen,currentOpen:ww(a,r),close:i,toggle:o,newChat:l,selectThread:c}}var Xp=new Set,rA=0;function oi(e,t={}){let a={id:++rA,message:e,tone:t.tone||"info",duration:t.duration??2600};return Xp.forEach(n=>n(a)),a.id}function Nw(e){return Xp.add(e),()=>Xp.delete(e)}function sA(e){return e?.status===409&&e?.payload?.kind==="busy"}function _w(e,t){return sA(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function kw(){let e=K({queryKey:["threads"],queryFn:()=>Dx({})}),[t,a]=h.default.useState(null),[n,r]=h.default.useState(!1),s=h.default.useRef(new Map),i=h.default.useCallback(async c=>{let d=c||"__global__",m=s.current.get(d);if(m)return m;r(!0);let f=(async()=>{try{let p=await gc(c?{projectId:c}:void 0);Ct.invalidateQueries({queryKey:["threads"]});let x=p?.thread?.thread_id;return x&&a(x),x}finally{s.current.delete(d),r(s.current.size>0)}})();return s.current.set(d,f),f},[]),o=h.default.useCallback(async c=>{await Mx({threadId:c}),t===c&&a(null),Ct.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:h.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var Rw={attach:u`<path
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
      ${Rw[e]||Rw.spark}
    </svg>
  `}function V(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=V(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function Cw(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function iA(e){return Cw(e).trim().charAt(0).toUpperCase()||"I"}function oA(){let[e,t]=h.default.useState(!1),a=h.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function Ew({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=R(),s=oA(),i=Cw(a),o=a?.email||a?.role||r("common.gatewaySession");return u`
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
            />`:u`<span className="place-self-center">${iA(a)}</span>`}
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
  `}var Tw={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",logs:"list",settings:"settings",admin:"shield"},lA=al.filter(e=>e.id!=="chat"&&!e.hidden);function uA({route:e,label:t,onNavigate:a}){return u`
    <${Ha}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${O} name=${Tw[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function cA({route:e,label:t,subRoutes:a,onNavigate:n}){let r=R(),s=Pe(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return u`
    <div className="flex flex-col">
      <${Ha}
        to=${o}
        onClick=${n}
        className=${()=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${O}
          name=${Tw[e.id]||"bolt"}
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
              <${Ha}
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
  `}function Aw({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=R(),s=h.default.useMemo(()=>lA.filter(i=>a||i.id!=="admin"),[a]);return u`
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
        ${s.map(i=>{let o=($c[i.id]||[]).filter(l=>a||!(i.id==="settings"&&["users","inference"].includes(l.id)));return o.length>0?u`
              <${cA}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:u`
            <${uA}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var $n=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),sl=new Set([$n.NEEDS_ATTENTION,$n.FAILED]),Zp="ironclaw:v2-thread-attention",Wp=new Set,li=new Map;function dA(){try{let e=window.localStorage.getItem(Zp);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&sl.has(a[1])):[]}catch{return[]}}function Dw(){let e=[];for(let[t,a]of li)sl.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(Zp):window.localStorage.setItem(Zp,JSON.stringify(e))}catch{}}for(let[e,t]of dA())li.set(e,t);function Ow(){return new Map(li)}function Mw(){let e=Ow();for(let t of Wp)try{t(e)}catch{}}function Rc(e,t){if(!e)return;let a=li.get(e);if(t==null){if(!li.delete(e))return;sl.has(a)&&Dw(),Mw();return}a!==t&&(li.set(e,t),(sl.has(t)||sl.has(a))&&Dw(),Mw())}function Lw(e){Rc(e,null)}function mA(){return Ow()}function fA(e){return Wp.add(e),()=>{Wp.delete(e)}}function Pw(){let[e,t]=h.default.useState(mA);return h.default.useEffect(()=>fA(t),[]),e}function Cc(e){return e.updated_at||e.created_at||null}function eh(e,t){let a=Cc(e)||"",n=Cc(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function Uw(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function jw(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function pA(){let[e,t]=h.default.useState(c$);return h.default.useEffect(()=>d$(t),[]),e}var hA=Object.freeze({[$n.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-transparent"},[$n.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[$n.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function vA(e){return e&&hA[e]||null}function gA({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=R(),o=Cc(e),l=Uw(o),c=jw(o),d=h.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(p=>{window.alert(p?.message||"Unable to delete chat")})},[s,e.id]),m=h.default.useCallback(f=>{f.preventDefault(),f.stopPropagation(),u$(e.id)},[e.id]);return u`
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
  `}function Fw({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:u`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>u`
          <${gA}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${vA(n.get(o.id))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function Bw({threads:e,activeThreadId:t,rebornProjectsEnabled:a=!1,onSelect:n,onDelete:r,onNavigate:s}){let[i,o]=h.default.useState(!1),[l,c]=h.default.useState(""),d=Pw(),m=pA(),f=R(),{pinned:p,recent:x,totalMatches:y}=h.default.useMemo(()=>{let $=l.trim().toLowerCase(),g=$?e.filter(w=>(w.title||w.id||"").toLowerCase().includes($)):e,v=[],b=[];for(let w of g)m.has(w.id)?v.push(w):b.push(w);return v.sort(eh),b.sort(eh),{pinned:v,recent:b,totalMatches:v.length+b.length}},[e,l,m]);return u`
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
          <${Ha}
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

          <${Fw}
            label=${f("common.pinned")}
            items=${p}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${m}
            onSelect=${n}
            onDelete=${r}
          />
          <${Fw}
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
  `}function Ec(){let e=Y(),t=K({queryKey:["trace-credits"],queryFn:nw,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=Q({mutationFn:rw,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function yA(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function zw(){let e=R(),{credits:t}=Ec();if(!t||!t.enrolled)return null;let a=yA(t.final_credit),n=t.submissions_accepted||0,r=t.submissions_submitted||0,s=t.manual_review_hold_count||0;return u`
    <div className="px-3 pb-1">
      <${Fr}
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
  `}function qw({id:e,threadsState:t,theme:a,toggleTheme:n,profile:r,isAdmin:s,rebornProjectsEnabled:i=!1,onSignOut:o,onClose:l,onNewChat:c,onSelectThread:d,onDeleteThread:m}){return u`
    <aside
      id=${e}
      className="flex h-full w-[260px] shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
    >
      <div className="flex items-center gap-2.5 px-4 py-5">
        <${Fr}
          to="/chat"
          onClick=${l}
          className="flex items-center gap-2.5 opacity-90 hover:opacity-100"
          aria-label="IronClaw"
        >
          <img src="/v2/assets/logo.jpg" alt="IronClaw" className="h-7 w-auto" />
        <//>
      </div>

      <${Aw}
        onNewChat=${c}
        isCreating=${t.isCreating}
        isAdmin=${s}
        onNavigate=${l}
      />

      <${zw} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${Bw}
          threads=${t.threads}
          activeThreadId=${t.activeThreadId}
          rebornProjectsEnabled=${i}
          onSelect=${d}
          onDelete=${m}
          onNavigate=${l}
        />
      </div>

      <${Ew}
        theme=${a}
        toggleTheme=${n}
        profile=${r}
        onSignOut=${o}
      />
    </aside>
  `}var bA="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",xA="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",Iw="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",Kw={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},Hw={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function D({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=Kw[n]??Kw.md,l=r?"w-full":"";if(a==="primary")return u`
      <${s}
        style=${{background:bA,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${V(Iw,o,l,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:xA}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=Hw[a]??Hw.outline;return u`
    <${s}
      className=${V(Iw,o,l,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function Qw(){let e=h.default.useMemo(()=>$A(window.location),[]),[t,a]=h.default.useState(null),[n,r]=h.default.useState(null),[s,i]=h.default.useState(!1),[o,l]=h.default.useState(""),[c,d]=h.default.useState(!1);h.default.useEffect(()=>{if(!e)return;let p=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:p.signal}).then(x=>{if(!x.ok)throw new Error(String(x.status));return x.json()}).then(a).catch(()=>{p.signal.aborted||a(null)}),()=>p.abort()},[e]);let m=h.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),l("");try{let p=await fetch(`${e.base}/attestation/report`);if(!p.ok)throw new Error(String(p.status));let x=await p.json();return r(x),x}catch(p){return l(p.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),f=h.default.useCallback(async()=>{let p=n||await m();return!p||!navigator.clipboard?!1:(await navigator.clipboard.writeText(JSON.stringify({...p,instance_attestation:t},null,2)),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[m,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:m,copyReport:f}}function $A(e){let t=e.hostname;if(!t||t==="localhost"||wA(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function wA(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}var SA=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function Vw(){let e=R(),t=Qw(),[a,n]=h.default.useState(!1),r=h.default.useCallback(()=>{n(o=>{let l=!o;return l&&t.loadReport(),l})},[t]),s=h.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=NA({teeInfo:t.teeInfo,report:t.report,t:e});return u`
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
  `}function NA({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return SA.map(([r,s])=>({label:a(s),value:_A(n[r])||a("common.unknown")}))}function _A(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var kA="https://docs.ironclaw.com";function Gw({threadsState:e,onToggleSidebar:t,sidebarOpen:a=!0}){let n=R(),r=Pe(),s=h.default.useMemo(()=>{for(let o of al){let l=$c[o.id];if(!l)continue;let c=o.path+"/";if(r.pathname.startsWith(c)){let d=r.pathname.slice(c.length).split("/")[0],m=l.find(f=>f.id===d);if(m)return{parent:n(o.labelKey),current:n(m.labelKey)}}}return null},[r.pathname,n]),i=h.default.useMemo(()=>{if(s)return null;if(r.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(c=>c.id===e.activeThreadId)?.title||n("nav.chat");let o=al.find(l=>r.pathname.startsWith(l.path));return o?n(o.labelKey):""},[r.pathname,e.activeThreadId,e.threads,n,s]);return u`
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
        <${Vw} />
        <${Ha}
          to="/logs"
          className=${({isActive:o})=>V("inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",o&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
          title=${n("nav.logs")}
        >
          ${n("nav.logs")}
        <//>
        <a
          href=${kA}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${n("nav.docs")}
        >
          ${n("nav.docs")}
        </a>
      </div>
    </header>
  `}function Yw({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=pe(),i=R(),[o,l]=h.default.useState(""),[c,d]=h.default.useState(0),m=h.default.useRef(null),f=h.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(b=>({id:`thread-${b.id}`,label:b.title||`Thread ${b.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${b.id}`)}));return[...g,...v]},[a,s,n,r]),p=h.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?f.filter(v=>v.label.toLowerCase().includes(g)):f},[f,o]);h.default.useEffect(()=>{if(!e)return;l(""),d(0);let g=window.requestAnimationFrame(()=>m.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),h.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,p.length-1)))},[p.length]);let x=h.default.useCallback(g=>{g&&(t(),g.run())},[t]),y=h.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,p.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),x(p[c])):g.key==="Escape"&&(g.preventDefault(),t())},[p,c,x,t]);if(!e)return null;let $=null;return u`
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
          ${p.length===0&&u`<li className="px-3 py-6 text-center text-sm text-[var(--v2-text-faint)]">No matches</li>`}
          ${p.map((g,v)=>{let b=g.group!==$;return $=g.group,u`
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
  `}var Jw={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},RA={info:"bolt",success:"check",error:"close"};function Xw(){let[e,t]=h.default.useState([]);return h.default.useEffect(()=>Nw(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:u`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${e.map(a=>u`
          <div
            key=${a.id}
            role="status"
            className=${["pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",Jw[a.tone]||Jw.info].join(" ")}
          >
            <${O} name=${RA[a.tone]||"bolt"} className="h-4 w-4 shrink-0" />
            <span>${a.message}</span>
          </div>
        `)}
    </div>
  `}function Zw({token:e,profile:t,isChecking:a=!1,isAdmin:n,rebornProjectsEnabled:r=!1,onSignOut:s}){let i=R(),{theme:o,toggleTheme:l}=wc(),c=M$(e),d=kw(),m=Sw({onNewChat:()=>d.setActiveThreadId(null)}),f=c.data,p=Pe(),x=pe(),y=ii({settings:{},gatewayStatus:f,enabled:n}),$=n&&pw({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),g=p.pathname==="/welcome"||p.pathname.startsWith("/settings"),[v,b]=h.default.useState(!1);h.default.useEffect(()=>{let S=C=>{(C.metaKey||C.ctrlKey)&&C.key.toLowerCase()==="k"&&(C.preventDefault(),b(_=>!_))};return window.addEventListener("keydown",S),()=>window.removeEventListener("keydown",S)},[]);let w=h.default.useCallback(async S=>{let C=d.activeThreadId===S;try{await d.deleteThread(S),C&&x("/chat",{replace:!0})}catch(_){console.error("Failed to delete thread:",_),oi(_w(_,i),{tone:"error"})}},[x,d,i]);return $&&!g?u`<${it} to="/welcome" replace />`:u`
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
        <${qw}
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
        <${Gw}
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
          <${Np}
            context=${{gatewayStatus:f,gatewayStatusQuery:c,currentUser:t,isChecking:a,isAdmin:n,threadsState:d}}
          />
        </main>
      </div>
      <${Yw}
        open=${v}
        onClose=${()=>b(!1)}
        threadsState=${d}
        onNewChat=${m.newChat}
        onToggleTheme=${l}
      />
      <${Xw} />
    </div>
  `}var qt=Be(Ke(),1),cl=e=>e.type==="checkbox",Kr=e=>e instanceof Date,Et=e=>e==null,d1=e=>typeof e=="object",Ve=e=>!Et(e)&&!Array.isArray(e)&&d1(e)&&!Kr(e),CA=e=>Ve(e)&&e.target?cl(e.target)?e.target.checked:e.target.value:e,EA=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,TA=(e,t)=>e.has(EA(t)),AA=e=>{let t=e.constructor&&e.constructor.prototype;return Ve(t)&&t.hasOwnProperty("isPrototypeOf")},nh=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function mt(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(nh&&(e instanceof Blob||n))&&(a||Ve(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!AA(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=mt(e[r]));else return e;return t}var Oc=e=>/^\w*$/.test(e),Ze=e=>e===void 0,rh=e=>Array.isArray(e)?e.filter(Boolean):[],sh=e=>rh(e.replace(/["|']|\]/g,"").split(/\.|\[/)),G=(e,t,a)=>{if(!t||!Ve(e))return a;let n=(Oc(t)?[t]:sh(t)).reduce((r,s)=>Et(r)?r:r[s],e);return Ze(n)||n===e?Ze(e[t])?a:e[t]:n},Va=e=>typeof e=="boolean",Ue=(e,t,a)=>{let n=-1,r=Oc(t)?[t]:sh(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],l=a;if(n!==i){let c=e[o];l=Ve(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=l,e=e[o]}},Ww={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},ka={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},wn={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},DA=qt.default.createContext(null);DA.displayName="HookFormContext";var MA=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==ka.all&&(t._proxyFormState[i]=!n||ka.all),a&&(a[i]=!0),e[i]}});return r},OA=typeof window<"u"?qt.default.useLayoutEffect:qt.default.useEffect;var Ga=e=>typeof e=="string",LA=(e,t,a,n,r)=>Ga(e)?(n&&t.watch.add(e),G(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),G(a,s))):(n&&(t.watchAll=!0),a),ah=e=>Et(e)||!d1(e);function sr(e,t,a=new WeakSet){if(ah(e)||ah(t))return e===t;if(Kr(e)&&Kr(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(Kr(i)&&Kr(o)||Ve(i)&&Ve(o)||Array.isArray(i)&&Array.isArray(o)?!sr(i,o,a):i!==o)return!1}}return!0}var PA=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},ll=e=>Array.isArray(e)?e:[e],e1=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},It=e=>Ve(e)&&!Object.keys(e).length,ih=e=>e.type==="file",Ra=e=>typeof e=="function",Ac=e=>{if(!nh)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},m1=e=>e.type==="select-multiple",oh=e=>e.type==="radio",UA=e=>oh(e)||cl(e),th=e=>Ac(e)&&e.isConnected;function jA(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=Ze(e)?n++:e[t[n++]];return e}function FA(e){for(let t in e)if(e.hasOwnProperty(t)&&!Ze(e[t]))return!1;return!0}function Xe(e,t){let a=Array.isArray(t)?t:Oc(t)?[t]:sh(t),n=a.length===1?e:jA(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Ve(n)&&It(n)||Array.isArray(n)&&FA(n))&&Xe(e,a.slice(0,-1)),e}var f1=e=>{for(let t in e)if(Ra(e[t]))return!0;return!1};function Dc(e,t={}){let a=Array.isArray(e);if(Ve(e)||a)for(let n in e)Array.isArray(e[n])||Ve(e[n])&&!f1(e[n])?(t[n]=Array.isArray(e[n])?[]:{},Dc(e[n],t[n])):Et(e[n])||(t[n]=!0);return t}function p1(e,t,a){let n=Array.isArray(e);if(Ve(e)||n)for(let r in e)Array.isArray(e[r])||Ve(e[r])&&!f1(e[r])?Ze(t)||ah(a[r])?a[r]=Array.isArray(e[r])?Dc(e[r],[]):{...Dc(e[r])}:p1(e[r],Et(t)?{}:t[r],a[r]):a[r]=!sr(e[r],t[r]);return a}var il=(e,t)=>p1(e,t,Dc(t)),t1={value:!1,isValid:!1},a1={value:!0,isValid:!0},h1=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!Ze(e[0].attributes.value)?Ze(e[0].value)||e[0].value===""?a1:{value:e[0].value,isValid:!0}:a1:t1}return t1},v1=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>Ze(e)?e:t?e===""?NaN:e&&+e:a&&Ga(e)?new Date(e):n?n(e):e,n1={isValid:!1,value:null},g1=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,n1):n1;function r1(e){let t=e.ref;return ih(t)?t.files:oh(t)?g1(e.refs).value:m1(t)?[...t.selectedOptions].map(({value:a})=>a):cl(t)?h1(e.refs).value:v1(Ze(t.value)?e.ref.value:t.value,e)}var BA=(e,t,a,n)=>{let r={};for(let s of e){let i=G(t,s);i&&Ue(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},Mc=e=>e instanceof RegExp,ol=e=>Ze(e)?e:Mc(e)?e.source:Ve(e)?Mc(e.value)?e.value.source:e.value:e,s1=e=>({isOnSubmit:!e||e===ka.onSubmit,isOnBlur:e===ka.onBlur,isOnChange:e===ka.onChange,isOnAll:e===ka.all,isOnTouch:e===ka.onTouched}),i1="AsyncFunction",zA=e=>!!e&&!!e.validate&&!!(Ra(e.validate)&&e.validate.constructor.name===i1||Ve(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===i1)),qA=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),o1=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),ul=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=G(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(ul(o,t))break}else if(Ve(o)&&ul(o,t))break}}};function l1(e,t,a){let n=G(e,a);if(n||Oc(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=G(t,s),o=G(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var IA=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return It(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||ka.all))},KA=(e,t,a)=>!e||!t||e===t||ll(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),HA=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,QA=(e,t)=>!rh(G(e,t)).length&&Xe(e,t),VA=(e,t,a)=>{let n=ll(G(e,a));return Ue(n,"root",t[a]),Ue(e,a,n),e},Tc=e=>Ga(e);function u1(e,t,a="validate"){if(Tc(e)||Array.isArray(e)&&e.every(Tc)||Va(e)&&!e)return{type:a,message:Tc(e)?e:"",ref:t}}var ui=e=>Ve(e)&&!Mc(e)?e:{value:e,message:""},c1=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:l,maxLength:c,minLength:d,min:m,max:f,pattern:p,validate:x,name:y,valueAsNumber:$,mount:g}=e._f,v=G(a,y);if(!g||t.has(y))return{};let b=o?o[0]:i,w=k=>{r&&b.reportValidity&&(b.setCustomValidity(Va(k)?"":k||""),b.reportValidity())},S={},C=oh(i),_=cl(i),A=C||_,L=($||ih(i))&&Ze(i.value)&&Ze(v)||Ac(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,M=PA.bind(null,y,n,S),P=(k,z,Z,re=wn.maxLength,me=wn.minLength)=>{let he=k?z:Z;S[y]={type:k?re:me,message:he,ref:i,...M(k?re:me,he)}};if(s?!Array.isArray(v)||!v.length:l&&(!A&&(L||Et(v))||Va(v)&&!v||_&&!h1(o).isValid||C&&!g1(o).isValid)){let{value:k,message:z}=Tc(l)?{value:!!l,message:l}:ui(l);if(k&&(S[y]={type:wn.required,message:z,ref:b,...M(wn.required,z)},!n))return w(z),S}if(!L&&(!Et(m)||!Et(f))){let k,z,Z=ui(f),re=ui(m);if(!Et(v)&&!isNaN(v)){let me=i.valueAsNumber||v&&+v;Et(Z.value)||(k=me>Z.value),Et(re.value)||(z=me<re.value)}else{let me=i.valueAsDate||new Date(v),he=gt=>new Date(new Date().toDateString()+" "+gt),Te=i.type=="time",je=i.type=="week";Ga(Z.value)&&v&&(k=Te?he(v)>he(Z.value):je?v>Z.value:me>new Date(Z.value)),Ga(re.value)&&v&&(z=Te?he(v)<he(re.value):je?v<re.value:me<new Date(re.value))}if((k||z)&&(P(!!k,Z.message,re.message,wn.max,wn.min),!n))return w(S[y].message),S}if((c||d)&&!L&&(Ga(v)||s&&Array.isArray(v))){let k=ui(c),z=ui(d),Z=!Et(k.value)&&v.length>+k.value,re=!Et(z.value)&&v.length<+z.value;if((Z||re)&&(P(Z,k.message,z.message),!n))return w(S[y].message),S}if(p&&!L&&Ga(v)){let{value:k,message:z}=ui(p);if(Mc(k)&&!v.match(k)&&(S[y]={type:wn.pattern,message:z,ref:i,...M(wn.pattern,z)},!n))return w(z),S}if(x){if(Ra(x)){let k=await x(v,a),z=u1(k,b);if(z&&(S[y]={...z,...M(wn.validate,z.message)},!n))return w(z.message),S}else if(Ve(x)){let k={};for(let z in x){if(!It(k)&&!n)break;let Z=u1(await x[z](v,a),b,z);Z&&(k={...Z,...M(z,Z.message)},w(Z.message),n&&(S[y]=k))}if(!It(k)&&(S[y]={ref:b,...k},!n))return S}}return w(!0),S},GA={mode:ka.onSubmit,reValidateMode:ka.onChange,shouldFocusError:!0};function YA(e={}){let t={...GA,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:Ra(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Ve(t.defaultValues)||Ve(t.values)?mt(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:mt(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},l,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},m={...d},f={array:e1(),state:e1()},p=t.criteriaMode===ka.all,x=N=>T=>{clearTimeout(c),c=setTimeout(N,T)},y=async N=>{if(!t.disabled&&(d.isValid||m.isValid||N)){let T=t.resolver?It((await _()).errors):await L(n,!0);T!==a.isValid&&f.state.next({isValid:T})}},$=(N,T)=>{!t.disabled&&(d.isValidating||d.validatingFields||m.isValidating||m.validatingFields)&&((N||Array.from(o.mount)).forEach(E=>{E&&(T?Ue(a.validatingFields,E,T):Xe(a.validatingFields,E))}),f.state.next({validatingFields:a.validatingFields,isValidating:!It(a.validatingFields)}))},g=(N,T=[],E,j,B=!0,F=!0)=>{if(j&&E&&!t.disabled){if(i.action=!0,F&&Array.isArray(G(n,N))){let J=E(G(n,N),j.argA,j.argB);B&&Ue(n,N,J)}if(F&&Array.isArray(G(a.errors,N))){let J=E(G(a.errors,N),j.argA,j.argB);B&&Ue(a.errors,N,J),QA(a.errors,N)}if((d.touchedFields||m.touchedFields)&&F&&Array.isArray(G(a.touchedFields,N))){let J=E(G(a.touchedFields,N),j.argA,j.argB);B&&Ue(a.touchedFields,N,J)}(d.dirtyFields||m.dirtyFields)&&(a.dirtyFields=il(r,s)),f.state.next({name:N,isDirty:P(N,T),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else Ue(s,N,T)},v=(N,T)=>{Ue(a.errors,N,T),f.state.next({errors:a.errors})},b=N=>{a.errors=N,f.state.next({errors:a.errors,isValid:!1})},w=(N,T,E,j)=>{let B=G(n,N);if(B){let F=G(s,N,Ze(E)?G(r,N):E);Ze(F)||j&&j.defaultChecked||T?Ue(s,N,T?F:r1(B._f)):Z(N,F),i.mount&&y()}},S=(N,T,E,j,B)=>{let F=!1,J=!1,ve={name:N};if(!t.disabled){if(!E||j){(d.isDirty||m.isDirty)&&(J=a.isDirty,a.isDirty=ve.isDirty=P(),F=J!==ve.isDirty);let ae=sr(G(r,N),T);J=!!G(a.dirtyFields,N),ae?Xe(a.dirtyFields,N):Ue(a.dirtyFields,N,!0),ve.dirtyFields=a.dirtyFields,F=F||(d.dirtyFields||m.dirtyFields)&&J!==!ae}if(E){let ae=G(a.touchedFields,N);ae||(Ue(a.touchedFields,N,E),ve.touchedFields=a.touchedFields,F=F||(d.touchedFields||m.touchedFields)&&ae!==E)}F&&B&&f.state.next(ve)}return F?ve:{}},C=(N,T,E,j)=>{let B=G(a.errors,N),F=(d.isValid||m.isValid)&&Va(T)&&a.isValid!==T;if(t.delayError&&E?(l=x(()=>v(N,E)),l(t.delayError)):(clearTimeout(c),l=null,E?Ue(a.errors,N,E):Xe(a.errors,N)),(E?!sr(B,E):B)||!It(j)||F){let J={...j,...F&&Va(T)?{isValid:T}:{},errors:a.errors,name:N};a={...a,...J},f.state.next(J)}},_=async N=>{$(N,!0);let T=await t.resolver(s,t.context,BA(N||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return $(N),T},A=async N=>{let{errors:T}=await _(N);if(N)for(let E of N){let j=G(T,E);j?Ue(a.errors,E,j):Xe(a.errors,E)}else a.errors=T;return T},L=async(N,T,E={valid:!0})=>{for(let j in N){let B=N[j];if(B){let{_f:F,...J}=B;if(F){let ve=o.array.has(F.name),ae=B._f&&zA(B._f);ae&&d.validatingFields&&$([j],!0);let bt=await c1(B,o.disabled,s,p,t.shouldUseNativeValidation&&!T,ve);if(ae&&d.validatingFields&&$([j]),bt[F.name]&&(E.valid=!1,T))break;!T&&(G(bt,F.name)?ve?VA(a.errors,bt,F.name):Ue(a.errors,F.name,bt[F.name]):Xe(a.errors,F.name))}!It(J)&&await L(J,T,E)}}return E.valid},M=()=>{for(let N of o.unMount){let T=G(n,N);T&&(T._f.refs?T._f.refs.every(E=>!th(E)):!th(T._f.ref))&&Kt(N)}o.unMount=new Set},P=(N,T)=>!t.disabled&&(N&&T&&Ue(s,N,T),!sr(gt(),r)),k=(N,T,E)=>LA(N,o,{...i.mount?s:Ze(T)?r:Ga(N)?{[N]:T}:T},E,T),z=N=>rh(G(i.mount?s:r,N,t.shouldUnregister?G(r,N,[]):[])),Z=(N,T,E={})=>{let j=G(n,N),B=T;if(j){let F=j._f;F&&(!F.disabled&&Ue(s,N,v1(T,F)),B=Ac(F.ref)&&Et(T)?"":T,m1(F.ref)?[...F.ref.options].forEach(J=>J.selected=B.includes(J.value)):F.refs?cl(F.ref)?F.refs.forEach(J=>{(!J.defaultChecked||!J.disabled)&&(Array.isArray(B)?J.checked=!!B.find(ve=>ve===J.value):J.checked=B===J.value||!!B)}):F.refs.forEach(J=>J.checked=J.value===B):ih(F.ref)?F.ref.value="":(F.ref.value=B,F.ref.type||f.state.next({name:N,values:mt(s)})))}(E.shouldDirty||E.shouldTouch)&&S(N,B,E.shouldTouch,E.shouldDirty,!0),E.shouldValidate&&je(N)},re=(N,T,E)=>{for(let j in T){if(!T.hasOwnProperty(j))return;let B=T[j],F=N+"."+j,J=G(n,F);(o.array.has(N)||Ve(B)||J&&!J._f)&&!Kr(B)?re(F,B,E):Z(F,B,E)}},me=(N,T,E={})=>{let j=G(n,N),B=o.array.has(N),F=mt(T);Ue(s,N,F),B?(f.array.next({name:N,values:mt(s)}),(d.isDirty||d.dirtyFields||m.isDirty||m.dirtyFields)&&E.shouldDirty&&f.state.next({name:N,dirtyFields:il(r,s),isDirty:P(N,F)})):j&&!j._f&&!Et(F)?re(N,F,E):Z(N,F,E),o1(N,o)&&f.state.next({...a,name:N}),f.state.next({name:i.mount?N:void 0,values:mt(s)})},he=async N=>{i.mount=!0;let T=N.target,E=T.name,j=!0,B=G(n,E),F=ae=>{j=Number.isNaN(ae)||Kr(ae)&&isNaN(ae.getTime())||sr(ae,G(s,E,ae))},J=s1(t.mode),ve=s1(t.reValidateMode);if(B){let ae,bt,Za=T.type?r1(B._f):CA(N),xa=N.type===Ww.BLUR||N.type===Ww.FOCUS_OUT,fr=!qA(B._f)&&!t.resolver&&!G(a.errors,E)&&!B._f.deps||HA(xa,G(a.touchedFields,E),a.isSubmitted,ve,J),Wa=o1(E,o,xa);Ue(s,E,Za),xa?(!T||!T.readOnly)&&(B._f.onBlur&&B._f.onBlur(N),l&&l(0)):B._f.onChange&&B._f.onChange(N);let kn=S(E,Za,xa),Wr=!It(kn)||Wa;if(!xa&&f.state.next({name:E,type:N.type,values:mt(s)}),fr)return(d.isValid||m.isValid)&&(t.mode==="onBlur"?xa&&y():xa||y()),Wr&&f.state.next({name:E,...Wa?{}:kn});if(!xa&&Wa&&f.state.next({...a}),t.resolver){let{errors:es}=await _([E]);if(F(Za),j){let Rn=l1(a.errors,n,E),ie=l1(es,n,Rn.name||E);ae=ie.error,E=ie.name,bt=It(es)}}else $([E],!0),ae=(await c1(B,o.disabled,s,p,t.shouldUseNativeValidation))[E],$([E]),F(Za),j&&(ae?bt=!1:(d.isValid||m.isValid)&&(bt=await L(n,!0)));j&&(B._f.deps&&je(B._f.deps),C(E,bt,ae,kn))}},Te=(N,T)=>{if(G(a.errors,T)&&N.focus)return N.focus(),1},je=async(N,T={})=>{let E,j,B=ll(N);if(t.resolver){let F=await A(Ze(N)?N:B);E=It(F),j=N?!B.some(J=>G(F,J)):E}else N?(j=(await Promise.all(B.map(async F=>{let J=G(n,F);return await L(J&&J._f?{[F]:J}:J)}))).every(Boolean),!(!j&&!a.isValid)&&y()):j=E=await L(n);return f.state.next({...!Ga(N)||(d.isValid||m.isValid)&&E!==a.isValid?{}:{name:N},...t.resolver||!N?{isValid:E}:{},errors:a.errors}),T.shouldFocus&&!j&&ul(n,Te,N?B:o.mount),j},gt=N=>{let T={...i.mount?s:r};return Ze(N)?T:Ga(N)?G(T,N):N.map(E=>G(T,E))},ft=(N,T)=>({invalid:!!G((T||a).errors,N),isDirty:!!G((T||a).dirtyFields,N),error:G((T||a).errors,N),isValidating:!!G(a.validatingFields,N),isTouched:!!G((T||a).touchedFields,N)}),tt=N=>{N&&ll(N).forEach(T=>Xe(a.errors,T)),f.state.next({errors:N?a.errors:{}})},Ge=(N,T,E)=>{let j=(G(n,N,{_f:{}})._f||{}).ref,B=G(a.errors,N)||{},{ref:F,message:J,type:ve,...ae}=B;Ue(a.errors,N,{...ae,...T,ref:j}),f.state.next({name:N,errors:a.errors,isValid:!1}),E&&E.shouldFocus&&j&&j.focus&&j.focus()},yt=(N,T)=>Ra(N)?f.state.subscribe({next:E=>"values"in E&&N(k(void 0,T),E)}):k(N,T,!0),At=N=>f.state.subscribe({next:T=>{KA(N.name,T.name,N.exact)&&IA(T,N.formState||d,se,N.reRenderRoot)&&N.callback({values:{...s},...a,...T,defaultValues:r})}}).unsubscribe,_n=N=>(i.mount=!0,m={...m,...N.formState},At({...N,formState:m})),Kt=(N,T={})=>{for(let E of N?ll(N):o.mount)o.mount.delete(E),o.array.delete(E),T.keepValue||(Xe(n,E),Xe(s,E)),!T.keepError&&Xe(a.errors,E),!T.keepDirty&&Xe(a.dirtyFields,E),!T.keepTouched&&Xe(a.touchedFields,E),!T.keepIsValidating&&Xe(a.validatingFields,E),!t.shouldUnregister&&!T.keepDefaultValue&&Xe(r,E);f.state.next({values:mt(s)}),f.state.next({...a,...T.keepDirty?{isDirty:P()}:{}}),!T.keepIsValid&&y()},Ea=({disabled:N,name:T})=>{(Va(N)&&i.mount||N||o.disabled.has(T))&&(N?o.disabled.add(T):o.disabled.delete(T))},ot=(N,T={})=>{let E=G(n,N),j=Va(T.disabled)||Va(t.disabled);return Ue(n,N,{...E||{},_f:{...E&&E._f?E._f:{ref:{name:N}},name:N,mount:!0,...T}}),o.mount.add(N),E?Ea({disabled:Va(T.disabled)?T.disabled:t.disabled,name:N}):w(N,!0,T.value),{...j?{disabled:T.disabled||t.disabled}:{},...t.progressive?{required:!!T.required,min:ol(T.min),max:ol(T.max),minLength:ol(T.minLength),maxLength:ol(T.maxLength),pattern:ol(T.pattern)}:{},name:N,onChange:he,onBlur:he,ref:B=>{if(B){ot(N,T),E=G(n,N);let F=Ze(B.value)&&B.querySelectorAll&&B.querySelectorAll("input,select,textarea")[0]||B,J=UA(F),ve=E._f.refs||[];if(J?ve.find(ae=>ae===F):F===E._f.ref)return;Ue(n,N,{_f:{...E._f,...J?{refs:[...ve.filter(th),F,...Array.isArray(G(r,N))?[{}]:[]],ref:{type:F.type,name:N}}:{ref:F}}}),w(N,!1,void 0,F)}else E=G(n,N,{}),E._f&&(E._f.mount=!1),(t.shouldUnregister||T.shouldUnregister)&&!(TA(o.array,N)&&i.action)&&o.unMount.add(N)}}},Dt=()=>t.shouldFocusError&&ul(n,Te,o.mount),ia=N=>{Va(N)&&(f.state.next({disabled:N}),ul(n,(T,E)=>{let j=G(n,E);j&&(T.disabled=j._f.disabled||N,Array.isArray(j._f.refs)&&j._f.refs.forEach(B=>{B.disabled=j._f.disabled||N}))},0,!1))},oa=(N,T)=>async E=>{let j;E&&(E.preventDefault&&E.preventDefault(),E.persist&&E.persist());let B=mt(s);if(f.state.next({isSubmitting:!0}),t.resolver){let{errors:F,values:J}=await _();a.errors=F,B=mt(J)}else await L(n);if(o.disabled.size)for(let F of o.disabled)Xe(B,F);if(Xe(a.errors,"root"),It(a.errors)){f.state.next({errors:{}});try{await N(B,E)}catch(F){j=F}}else T&&await T({...a.errors},E),Dt(),setTimeout(Dt);if(f.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:It(a.errors)&&!j,submitCount:a.submitCount+1,errors:a.errors}),j)throw j},Ta=(N,T={})=>{G(n,N)&&(Ze(T.defaultValue)?me(N,mt(G(r,N))):(me(N,T.defaultValue),Ue(r,N,mt(T.defaultValue))),T.keepTouched||Xe(a.touchedFields,N),T.keepDirty||(Xe(a.dirtyFields,N),a.isDirty=T.defaultValue?P(N,mt(G(r,N))):P()),T.keepError||(Xe(a.errors,N),d.isValid&&y()),f.state.next({...a}))},Xr=(N,T={})=>{let E=N?mt(N):r,j=mt(E),B=It(N),F=B?r:j;if(T.keepDefaultValues||(r=E),!T.keepValues){if(T.keepDirtyValues){let J=new Set([...o.mount,...Object.keys(il(r,s))]);for(let ve of Array.from(J))G(a.dirtyFields,ve)?Ue(F,ve,G(s,ve)):me(ve,G(F,ve))}else{if(nh&&Ze(N))for(let J of o.mount){let ve=G(n,J);if(ve&&ve._f){let ae=Array.isArray(ve._f.refs)?ve._f.refs[0]:ve._f.ref;if(Ac(ae)){let bt=ae.closest("form");if(bt){bt.reset();break}}}}if(T.keepFieldsRef)for(let J of o.mount)me(J,G(F,J));else n={}}s=t.shouldUnregister?T.keepDefaultValues?mt(r):{}:mt(F),f.array.next({values:{...F}}),f.state.next({values:{...F}})}o={mount:T.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!T.keepIsValid||!!T.keepDirtyValues,i.watch=!!t.shouldUnregister,f.state.next({submitCount:T.keepSubmitCount?a.submitCount:0,isDirty:B?!1:T.keepDirty?a.isDirty:!!(T.keepDefaultValues&&!sr(N,r)),isSubmitted:T.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:B?{}:T.keepDirtyValues?T.keepDefaultValues&&s?il(r,s):a.dirtyFields:T.keepDefaultValues&&N?il(r,N):T.keepDirty?a.dirtyFields:{},touchedFields:T.keepTouched?a.touchedFields:{},errors:T.keepErrors?a.errors:{},isSubmitSuccessful:T.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},Zr=(N,T)=>Xr(Ra(N)?N(s):N,T),Xa=(N,T={})=>{let E=G(n,N),j=E&&E._f;if(j){let B=j.refs?j.refs[0]:j.ref;B.focus&&(B.focus(),T.shouldSelect&&Ra(B.select)&&B.select())}},se=N=>{a={...a,...N}},fe={control:{register:ot,unregister:Kt,getFieldState:ft,handleSubmit:oa,setError:Ge,_subscribe:At,_runSchema:_,_focusError:Dt,_getWatch:k,_getDirty:P,_setValid:y,_setFieldArray:g,_setDisabledField:Ea,_setErrors:b,_getFieldArray:z,_reset:Xr,_resetDefaultValues:()=>Ra(t.defaultValues)&&t.defaultValues().then(N=>{Zr(N,t.resetOptions),f.state.next({isLoading:!1})}),_removeUnmounted:M,_disableForm:ia,_subjects:f,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(N){i=N},get _defaultValues(){return r},get _names(){return o},set _names(N){o=N},get _formState(){return a},get _options(){return t},set _options(N){t={...t,...N}}},subscribe:_n,trigger:je,register:ot,handleSubmit:oa,watch:yt,setValue:me,getValues:gt,reset:Zr,resetField:Ta,clearErrors:tt,unregister:Kt,setError:Ge,setFocus:Xa,getFieldState:ft};return{...fe,formControl:fe}}function y1(e={}){let t=qt.default.useRef(void 0),a=qt.default.useRef(void 0),[n,r]=qt.default.useState({isDirty:!1,isValidating:!1,isLoading:Ra(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:Ra(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!Ra(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=YA(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,OA(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),qt.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),qt.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),qt.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),qt.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),qt.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),qt.default.useEffect(()=>{e.values&&!sr(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),qt.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=MA(n,s),t.current}var b1={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},x1={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},JA={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function ee({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return u`
    <${s}
      className=${V(b1[a]??b1.default,x1[n]??x1.md,JA[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var lh="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",Lc={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function Tt({className:e="",size:t="md",error:a=!1,...n}){return u`
    <input
      className=${V(lh,Lc[t]??Lc.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Pc({className:e="",error:t=!1,rows:a=4,...n}){return u`
    <textarea
      rows=${a}
      className=${V(lh,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function uh({children:e,className:t="",size:a="md",error:n=!1,...r}){return u`
    <div className="relative w-full">
      <select
        className=${V(lh,Lc[a]??Lc.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
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
  `}function XA({children:e,className:t="",required:a=!1,...n}){return u`
    <label
      className=${V("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&u`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function Sn({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return u`
    <div className=${V("flex flex-col gap-2",s)}>
      ${e&&u`<${XA} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&u`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&u`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var ZA={google:"Google",github:"GitHub",apple:"Apple"};function WA(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function $1({providers:e,redirectAfter:t}){let a=R();return e.length?u`
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
              href=${WA(n,t)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <${O} name="shield" className="h-4 w-4" />
              ${a("login.oauthProvider",{provider:ZA[n]||n})}
            <//>
          `)}
      </div>
    </div>
  `:null}var e4=["google","github","apple"];function w1(){let[e,t]=h.default.useState([]);return h.default.useEffect(()=>{let a=!1;return e$().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(e4.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function S1({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=R(),{theme:s,toggleTheme:i}=wc(),o=w1(),{formState:{errors:l,isSubmitting:c},handleSubmit:d,register:m}=y1({defaultValues:{token:e||""}});return u`
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
          <${Sn}
            label=${r("login.tokenLabel")}
            htmlFor="v2-token"
            error=${l.token?.message??""}
            hint=${r("login.tokenHint")}
          >
            <${Tt}
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

        <${$1}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var N1={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},_1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function q({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return u`
    <span
      className=${V("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",_1[n]??_1.md,N1[e]??N1.muted,r)}
    >
      ${a&&u`<span
          className=${V("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var t4=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,k1=/(bash|shell|exec|run|command|terminal|spawn|process)/,R1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function C1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return t4.test(n)?{tone:"danger",key:"tool.riskWrite"}:k1.test(n)?{tone:"warning",key:"tool.riskExec"}:R1.test(n)?{tone:"info",key:"tool.riskNetwork"}:k1.test(r)?{tone:"warning",key:"tool.riskExec"}:R1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}var Uc=480;function a4(e,t){return t&&t.length>0?t.some(a=>typeof a?.value=="string"&&a.value.length>Uc):typeof e=="string"&&e.length>Uc}function E1(e,t){return typeof e!="string"||t||e.length<=Uc?e:`${e.slice(0,Uc).trimEnd()}
...`}function T1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=R(),{toolName:s,description:i,parameters:o,allowAlways:l,approvalDetails:c=[]}=e,[d,m]=h.default.useState(!1),[f,p]=h.default.useState(!1);h.default.useEffect(()=>{p(!1)},[e]);let x=h.default.useMemo(()=>C1(s,i,o),[s,i,o]),y=s||r("approval.thisTool"),$=a4(o,c),g=f?"max-h-72":"max-h-36",v=h.default.useCallback(()=>{d&&l?n?.():t?.()},[d,l,n,t]);return u`
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
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${E1(b.value,f)}</dd>
                  </div>
                `)}
            </dl>
          `:o&&u`<pre className=${`mb-2 ${g} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${E1(o,f)}</pre>`}

      ${$&&u`
        <${D}
          variant="ghost"
          size="sm"
          className="mb-3 px-0 text-[var(--v2-accent)] hover:bg-transparent"
          onClick=${()=>p(b=>!b)}
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
  `}function ci({icon:e="lock",headline:t,provider:a,accountLabel:n,body:r,expiresAt:s,pillHint:i,defaultExpanded:o=!0,children:l}){let c=R(),[d,m]=h.default.useState(o),f=h.default.useId(),p=n||a||"";return u`
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
          ${p&&u`<span className="block truncate text-xs text-iron-300">${p}</span>`}
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
  `}function A1({gate:e,onCancel:t}){let a=R();return u`
    <${ci}
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
  `}function D1({gate:e,onCancel:t}){let a=R(),[n,r]=h.default.useState(!1),[s,i]=h.default.useState(""),o=h.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]);h.default.useEffect(()=>{i("")},[e.authorizationUrl,e.gateRef,e.runId]);let l=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),c=h.default.useCallback(()=>{if(!o){i(a("authGate.serviceUnavailable"));return}i(""),window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0)},[e.authorizationUrl,o]),d=n?a("authGate.reopenAuthorization",{provider:l}):a("authGate.openAuthorization",{provider:l});return u`
    <${ci}
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
  `}function M1({gate:e,onSubmit:t,onCancel:a}){let n=R(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[l,c]=h.default.useState(!1),d=h.default.useCallback(async m=>{m.preventDefault();let f=r.trim();if(!f){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(f),s("")}catch(p){o(p?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return u`
    <${ci}
      icon="lock"
      headline=${e?.headline||n("authGate.title")}
      provider=${e?.provider||""}
      accountLabel=${e?.accountLabel||""}
      body=${e?.body||""}
      pillHint=${n("authGate.pillEnterToken")}
    >
      <form onSubmit=${d}>
        <div className="mb-3">
          <${Tt}
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
  `}var n4="/api/webchat/v2/extensions/pairing/redeem";function O1(e){return H(n4,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function jc({action:e}){let t=R(),a=Y(),n=Q({mutationFn:({code:l})=>O1(l),onSuccess:()=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=h.default.useState(""),i=r4(e,t),o=()=>{let l=r.trim();l&&(n.mutate({code:l}),s(""))};return u`
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
        ${s4(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function r4(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function s4(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function i4(e,t){return e?.channel==="slack"&&e.strategy===t}function L1({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return u`
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

      ${i4(e,"inbound_proof_code")?u`<${jc} action=${e.action} />`:u`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function o4(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):zr.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:zr.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:zr.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:zr.maxTotalBytes}:zr}function P1(){let e=ba(),t=K({enabled:!!e,queryKey:["session"],queryFn:vc,staleTime:5*6e4});return o4(t.data)}function Fc({onSend:e,onCancel:t,disabled:a,sendDisabled:n=a,canCancel:r=!1,initialText:s="",resetKey:i="",draftKey:o=tl,variant:l="dock",context:c={},statusText:d=""}){let m=R(),f=l==="hero",p=P1(),[x,y]=h.default.useState(()=>Kp(o)),[$,g]=h.default.useState(()=>Qp(o)),[v,b]=h.default.useState(""),[w,S]=h.default.useState(!1),[C,_]=h.default.useState(!1),[A,L]=h.default.useState(!1),M=h.default.useRef(null),P=h.default.useRef(null),k=h.default.useRef(!1),z=a||n||w,Z=h.default.useRef(a||n);Z.current=a||n,k.current=z;let re=h.default.useRef([]),me=h.default.useRef(Promise.resolve());h.default.useEffect(()=>{re.current=$},[$]);let he=h.default.useRef(null),Te=h.default.useRef(null),je=h.default.useCallback(()=>{Te.current&&(window.clearTimeout(Te.current),Te.current=null);let E=he.current;he.current=null,E&&E.scope===St()&&Hp(E.key,E.text)},[]),gt=h.default.useCallback(()=>{Te.current&&(window.clearTimeout(Te.current),Te.current=null),he.current=null},[]),ft=h.default.useCallback(()=>{let E=M.current;E&&(E.style.height="auto",E.style.height=`${Math.min(E.scrollHeight,200)}px`)},[]);h.default.useEffect(()=>{ft()},[x,ft]),h.default.useEffect(()=>(y(Kp(o)),()=>je()),[o,je]);let tt=h.default.useRef(o);h.default.useEffect(()=>{if(tt.current!==o){tt.current=o,g(Qp(o)),b("");return}C$(o,$)},[o,$]),h.default.useEffect(()=>{s&&(y(s),window.requestAnimationFrame(()=>{M.current&&(M.current.focus(),M.current.setSelectionRange(s.length,s.length))}))},[s,i]);let Ge=h.default.useCallback(E=>{a||!E||E.length===0||(me.current=me.current.then(async()=>{let{staged:j,errors:B}=await p$(E,{limits:p,existing:re.current,t:m});j.length>0&&g(F=>{let J=[...F,...j];return re.current=J,J}),b(B.length>0?B.join(" "):"")}).catch(()=>{b(m("chat.attachmentStagingFailed"))}))},[a,p,m]),yt=h.default.useCallback(E=>{g(j=>{let B=j.filter(F=>F.id!==E);return re.current=B,B}),b("")},[]),At=h.default.useCallback(()=>{a||P.current?.click()},[a]),_n=h.default.useCallback(E=>{let j=Array.from(E.target.files||[]);Ge(j),E.target.value=""},[Ge]),Kt=h.default.useCallback(async()=>{if(!(!x.trim()||k.current)){k.current=!0,S(!0);try{if(await e(x.trim(),{attachments:$})===null)return;y(""),g([]),re.current=[],b(""),gt(),R$(o),E$(o),M.current&&(M.current.style.height="auto")}catch{}finally{k.current=Z.current,S(!1)}}},[x,$,e,o,gt]),Ea=h.default.useCallback(E=>{let j=E.target.value;y(j),he.current={key:o,text:j,scope:St()},Te.current&&window.clearTimeout(Te.current),Te.current=window.setTimeout(je,300)},[o,je]),ot=h.default.useCallback(async()=>{if(!(!r||C||!t)){_(!0);try{await t()}finally{_(!1)}}},[r,C,t]),Dt=h.default.useCallback(E=>{if(E.key==="Enter"&&!E.shiftKey){if(E.preventDefault(),M.current?.dataset?.sendDisabled==="true"||k.current)return;Kt()}},[Kt]),ia=h.default.useCallback(E=>{let j=Array.from(E.clipboardData?.files||[]);j.length>0&&(E.preventDefault(),Ge(j))},[Ge]),oa=h.default.useCallback(E=>{E.preventDefault(),L(!1);let j=Array.from(E.dataTransfer?.files||[]);j.length>0&&Ge(j)},[Ge]),Ta=h.default.useCallback(E=>{E.preventDefault(),!a&&L(!0)},[a]),Xr=h.default.useCallback(E=>{E.currentTarget.contains(E.relatedTarget)||L(!1)},[]),Zr=x.trim(),Xa=a||n,se=m(f?"chat.heroPlaceholder":"chat.followUpPlaceholder"),ne=p.accept.length>0?p.accept.join(","):void 0,fe=f?"w-full":"px-4 py-3 sm:px-5 lg:px-8",N=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",f?"min-h-[120px]":"",a?"opacity-70":""].join(" "),T=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",f?"min-h-[72px]":"min-h-[40px]"].join(" ");return u`
    <div className=${fe}>
      <div
        className=${N}
        onDrop=${oa}
        onDragOver=${Ta}
        onDragLeave=${Xr}
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
                    onClick=${()=>yt(E.id)}
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
          onKeyDown=${Dt}
          onPaste=${ia}
          data-send-disabled=${Xa?"true":"false"}
          placeholder=${se}
          rows=${1}
          disabled=${a}
          className=${T}
        />

        <input
          ref=${P}
          type="file"
          multiple
          accept=${ne}
          className="hidden"
          onChange=${_n}
        />

        <div className="mt-2 flex items-center gap-2">
          ${Xa&&u`
            <span className="inline-flex items-center gap-2 text-xs text-[var(--v2-text-muted)]">
              <span className="h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
              ${d||m("chat.statusWorking")}
            </span>
          `}
          <div className="ml-auto flex items-center gap-1.5">
            <button
              type="button"
              onClick=${At}
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
                  onClick=${ot}
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
                  onClick=${Kt}
                  disabled=${Xa||w||!Zr}
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
  `}var U1={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function j1({status:e}){let t=R();if(e==="idle"||e==="connecting"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return u`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",U1[e]||U1.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function F1({onSuggestion:e,onSend:t,disabled:a,sendDisabled:n,initialText:r,resetKey:s,draftKey:i,context:o,statusText:l,canCancel:c,onCancel:d}){let m=R(),f=[{icon:"tool",title:m("chat.suggestion1"),detail:m("chat.suggestion1Desc")},{icon:"shield",title:m("chat.suggestion2"),detail:m("chat.suggestion2Desc")},{icon:"plug",title:m("chat.suggestion3"),detail:m("chat.suggestion3Desc")}];return u`
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
        <${Fc}
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
        ${f.map(p=>u`
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
  `}var l4=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function B1({open:e,onClose:t}){let a=R();return e?u`
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
          ${l4.map((n,r)=>u`
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
  `:null}function q1(e){let t=0,a=0,n=0,r=0,s=0;for(let o of e){if(o.role==="thinking"&&(t+=1),o.role==="tool_activity"){let l=z1([o]);a+=l.tools,n+=l.failed,r+=l.declined,s+=l.running}if(u4(o)){let l=z1(o.toolCalls);a+=l.tools,n+=l.failed,r+=l.declined,s+=l.running}}let i=[];return t&&i.push(`${t} reasoning`),a&&i.push(`${a} ${a===1?"tool":"tools"}`),n&&i.push(`${n} failed`),r&&i.push(`${r} declined`),!n&&!r&&s&&i.push("running"),{hasError:n>0,hasDeclined:r>0,label:`Activity${i.length?` - ${i.join(", ")}`:""}`}}function z1(e){let t=0,a=0,n=0;for(let r of e)r.toolStatus==="error"&&(t+=1),r.toolStatus==="declined"&&(a+=1),r.toolStatus==="running"&&(n+=1);return{tools:e.length,failed:t,declined:a,running:n}}function u4(e){return e.toolCalls&&e.toolCalls.length>0}var I1=!1;function c4(){I1||!window.DOMPurify||(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),I1=!0)}function K1(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}c4();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var ch=360;function d4(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let l=s("Copy");if(l.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),l.textContent="Copied",oi("Code copied",{tone:"success"}),setTimeout(()=>l.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(l),n.appendChild(r),t.scrollHeight>ch){t.style.maxHeight=`${ch}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${ch}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function m4({content:e,className:t=""}){let a=h.default.useRef(null),n=h.default.useMemo(()=>K1(e),[e]);return h.default.useEffect(()=>{d4(a.current)},[n]),u`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var na=h.default.memo(m4);var H1={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",declined:"bg-iron-400",error:"bg-[var(--v2-danger-text)]"},f4={success:"ok",declined:"declined",error:"err",running:"run"},p4=2;function di({activity:e}){return e.toolCalls&&e.toolCalls.length>0?u`<${v4} tools=${e.toolCalls} />`:u`<${g4} activity=${e} />`}function h4(e,t){let a=0,n=0,r=0,s=0;for(let l of t){let c=String(l.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function v4({tools:e}){let t=R(),a=e.some(o=>o.toolStatus==="error"),n=e.some(o=>o.toolStatus==="error"||o.toolStatus==="declined"),[r,s]=h.default.useState(n);if(h.default.useEffect(()=>{n&&s(!0)},[n]),e.length<=p4)return u`
      <div className="flex flex-col gap-3">
        ${e.map((o,l)=>u`<${di}
            key=${o.id||o.callId||`${o.toolName}-${l}`}
            activity=${o}
          />`)}
      </div>
    `;let i=h4(t,e);return u`
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
          ${e.map((o,l)=>u`<${di}
              key=${o.id||o.callId||`${o.toolName}-${l}`}
              activity=${o}
            />`)}
        </div>
      `}
    </div>
  `}function g4({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:l}=e,[c,d]=h.default.useState(n==="error"||n==="declined");h.default.useEffect(()=>{(n==="error"||n==="declined")&&d(!0)},[n]);let m=H1[n]||H1.running,f=i!=null,p=h.default.useId(),x=u`
    <button
      type="button"
      onClick=${()=>d(y=>!y)}
      aria-expanded=${c?"true":"false"}
      aria-controls=${p}
      className="v2-button flex w-full items-center gap-2.5 border-0 border-b border-iron-700/40 bg-transparent px-1 py-2 text-left text-sm"
    >
      <span className=${["h-2 w-2 shrink-0 rounded-full",m].join(" ")} />
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-wide text-iron-300"
        >${f4[n]||"run"}</span
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
        ${c&&u`<${y4}
          controlsId=${p}
          toolDetail=${r}
          toolParameters=${o}
          toolResultPreview=${l}
          toolError=${s}
          toolStatus=${n}
          toolDurationMs=${f?i:null}
        />`}
      </div>
    </div>
  `}function y4({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=R(),l=h.default.useMemo(()=>{let f=[];return r&&f.push({id:s==="declined"?"declined":"error",label:o(s==="declined"?"tool.tabDeclined":"tool.tabError")}),t&&f.push({id:"details",label:o("tool.tabDetails")}),a&&f.push({id:"params",label:o("tool.tabParameters")}),n&&f.push({id:"result",label:o("tool.tabResult")}),f},[o,r,t,a,n,s]),[c,d]=h.default.useState(null),m=c&&l.some(f=>f.id===c)?c:l[0]?.id;return h.default.useEffect(()=>{r&&d(s==="declined"?"declined":"error")},[r,s]),l.length===0?u`
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
        ${m==="result"&&u`<${b4} text=${n} />`}
        ${(m==="error"||m==="declined")&&u`<pre
          className=${["overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono",m==="declined"?"text-iron-300":"text-[var(--v2-danger-text)]"].join(" ")}
        >${r}</pre>`}
      </div>
    </div>
  `}function b4({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return u`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(x4)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return u`
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
                  >${$4(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?u`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:u`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function x4(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function $4(e){return e==null?"":String(e)}function Q1({activity:e}){let t=q1(e),a=N4(e),[n,r]=h.default.useState(a);return h.default.useEffect(()=>{a&&r(!0)},[a]),u`
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
            <${w4}
              key=${s.id||`${s.role||"activity"}-${i}`}
              item=${s}
            />
          `)}
        </div>
      `}
    </div>
  `}function w4({item:e}){if(e.role==="thinking")return u`<${S4} content=${e.content} />`;if(e.role==="tool_activity"||dh(e)){let t=dh(e)?{id:e.id,toolCalls:e.toolCalls}:e;return u`<${di} activity=${t} />`}return null}function S4({content:e}){return e?u`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <${O} name="spark" className="h-4 w-4" />
      </div>
      <div className="min-w-0 max-w-[85%] flex-1 border-l-2 border-white/10 pl-3 text-iron-300">
        <${na} content=${e} className="text-[13px]" />
      </div>
    </div>
  `:null}function dh(e){return e?.toolCalls&&e.toolCalls.length>0}function N4(e){return(e||[]).some(t=>t?.role==="thinking"||t?.toolStatus==="running"||t?.toolStatus==="error"||t?.toolStatus==="declined"?!0:dh(t)?t.toolCalls.some(a=>a?.toolStatus==="running"||a?.toolStatus==="error"||a?.toolStatus==="declined"):!1)}function Bc(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function _4({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=h.default.useState(()=>t&&e.preview_url||null);return h.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return bc(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?u`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:u`<${O} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var V1="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",G1="px-3 py-2";function zc({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=h.default.useState(!1),o=h.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await _a(e.fetch_url);Bc(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),l=u`
    <${_4} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?u`<div
      className=${`${V1} ${G1} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${l}
    </div>`:u`<div className=${`${V1} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${G1} text-left transition-colors hover:bg-iron-900/80`}
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
  </div>`}var Y1={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function mi({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return h.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),h.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?u`
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
        className=${V("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",Y1[n]??Y1.md,r)}
      >
        ${a?u`<${mh} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function mh({children:e,onClose:t,className:a=""}){return u`
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
  `}function fi({children:e,className:t=""}){return u`
    <div className=${V("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function pi({children:e,className:t=""}){return u`
    <div
      className=${V("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var J1=1e5;function qc({attachment:e,onClose:t}){let a=!!e,[n,r]=h.default.useState("loading"),[s,i]=h.default.useState({}),o=e?f$(e.mime_type):"download";if(h.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return _a(e.fetch_url).then(async m=>{d=URL.createObjectURL(m);let f={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")f.dataUrl=await Op(m);else if(o==="pdf")f.frameUrl=d;else if(o==="text"){let p=await m.text();f.truncated=p.length>J1,f.text=f.truncated?p.slice(0,J1):p}if(c){URL.revokeObjectURL(d);return}i(f),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let l=e.filename||"attachment";return u`
    <${mi} open=${a} onClose=${t} size="xl">
      <${mh} onClose=${t}>
        <span className="block truncate">${l}</span>
      <//>
      <${fi} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&u`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&u`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&u`<${k4} mode=${o} view=${s} filename=${l} />`}
      <//>
      <${pi}>
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
  `}function k4({mode:e,view:t,filename:a}){switch(e){case"image":return u`<img
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
      </div>`}}var R4=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function C4(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function X1(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of C4(e).matchAll(R4)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function Z1(e){return e.split("/").filter(Boolean).pop()||e}function W1(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function E4({threadId:e,path:t,onPreview:a}){let[n,r]=h.default.useState({mime_type:"",size_label:""});h.default.useEffect(()=>{let i=!0;return Lx({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:W1(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:Z1(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:yc({threadId:e,path:t})};return u`<${zc}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function e2({threadId:e,content:t}){let a=h.default.useMemo(()=>X1(t),[t]),[n,r]=h.default.useState(null);return!e||a.length===0?null:u`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>u`<${E4}
          key=${s}
          threadId=${e}
          path=${s}
          onPreview=${r}
        />`)}
      <${qc}
        attachment=${n}
        onClose=${()=>r(null)}
      />
    </div>
  `}var t2={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function T4(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function A4({content:e}){let[t,a]=h.default.useState(!1);return e?u`
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
          <${na} content=${e} className="text-[13px]" />
        </div>
      `}
    </div>
  `:null}function D4({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:l,status:c,error:d,toolCalls:m,timestamp:f}=e,p=n==="user",[x,y]=h.default.useState(!1),[$,g]=h.default.useState(null),v=h.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),y(!0),oi("Copied to clipboard",{tone:"success"}),setTimeout(()=>y(!1),1400)}catch{}},[r]);if(n==="tool_activity"||m&&m.length>0){let M=m&&m.length>0?{id:e.id,toolCalls:m}:e;return u`<${di} activity=${M} />`}if(n==="thinking")return u`<${A4} content=${r} />`;if(n==="image")return u`
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
    `;let b=T4(f),w=n==="user"||n==="assistant"&&!l,S=n==="system"||n==="error",C=p?"max-w-[85%]":S?"mx-auto max-w-[85%]":"w-full max-w-[85%]",_=p?"":"w-full min-w-0 max-w-full",A=c==="error"&&t,L=w||A||b;return u`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",p?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col",C].join(" ")}>
        <div
          className=${["text-base leading-7",_,t2[n]||t2.assistant,l?"opacity-70":""].join(" ")}
        >
          ${n==="assistant"||n==="system"||n==="error"?u`<${na} content=${r} />`:u`<div className="whitespace-pre-wrap">${r}</div>`}

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
              ${i.map((M,P)=>u`<${zc}
                key=${M.id||P}
                att=${M}
                onPreview=${g}
              />`)}
            </div>
            <${qc}
              attachment=${$}
              onClose=${()=>g(null)}
            />
          `}

          ${n==="assistant"&&u`<${e2}
            threadId=${a}
            content=${typeof r=="string"?r:""}
          />`}
        </div>
      </div>

      ${L&&u`
        <div
          className=${["mt-1 flex min-h-7 w-max max-w-[85%] flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100",p?"self-end justify-end":S?"self-center justify-center":"self-start justify-start"].join(" ")}
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
  `}var a2=h.default.memo(D4);function l2(e){let t=M4(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(u2(r)){let s=n2(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){r2(a,s),s2(a,r),n+=s.length;continue}}if(fh(r)){let s=n2(t,n);r2(a,s),n+=s.length-1;continue}s2(a,r)}return a}function M4(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=Ic(i);o&&u2(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!fh(i))continue;let o=Ic(i),l=o?t.get(o):void 0;if(l===void 0||l>=s)continue;let c=a.get(l)||[];c.push(i),a.set(l,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function n2(e,t){let a=t,n=Ic(e[t]);for(;a<e.length&&fh(e[a])&&O4(n,e[a]);)a+=1;return e.slice(t,a)}function O4(e,t){let a=Ic(t);return!e||!a||a===e}function r2(e,t){if(t.length===0)return;let a=L4(t);e.push({type:"activity-run",id:`activity-run-${a[0].id}`,activity:a})}function s2(e,t){e.push({type:"message",id:t.id,message:t})}function u2(e){return e.role==="assistant"&&!c2(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function fh(e){return e.role==="thinking"||e.role==="tool_activity"||c2(e)}function c2(e){return e?.toolCalls&&e.toolCalls.length>0}function Ic(e){return e?.turnRunId||null}function L4(e){return[...e].sort((t,a)=>t?.role!=="tool_activity"||a?.role!=="tool_activity"?0:P4(t,a))}function P4(e,t){if(Number.isFinite(e.activityOrder)&&Number.isFinite(t.activityOrder)){let n=e.activityOrder-t.activityOrder;if(n!==0)return n}let a=i2(o2(e.updatedAt||e.timestamp),o2(t.updatedAt||t.timestamp));return a!==0?a:i2(e.sequence,t.sequence)}function i2(e,t){let a=Number.isFinite(e)?e:null,n=Number.isFinite(t)?t:null;return a===null&&n===null?0:a===null?1:n===null?-1:a-n}function o2(e){if(!e)return null;let t=Date.parse(e);return Number.isFinite(t)?t:null}var U4=100,j4=100;function F4(e){return e?e.scrollHeight-e.scrollTop-e.clientHeight:Number.POSITIVE_INFINITY}function d2(e,t=U4){return F4(e)<=t}function m2(e){e&&(e.scrollTop=Math.max(0,e.scrollHeight-e.clientHeight))}function f2(e){return e?.id?`${e.role||""}:${e.id}`:null}function B4(e,t){let a=f2(t);return!!(a&&t?.role==="user"&&a!==e)}function p2({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let l=R(),c=h.default.useRef(null),d=h.default.useRef(null),m=h.default.useRef(!0),f=h.default.useRef(null),p=h.default.useRef(null),x=h.default.useRef(null),y=h.default.useRef(0),$=h.default.useRef(!1),[g,v]=h.default.useState(!0),b=h.default.useCallback(()=>{p.current!==null&&(window.cancelAnimationFrame(p.current),p.current=null)},[]),w=h.default.useCallback((k=!1)=>{c.current&&(k&&(m.current=!0,$.current=!1),m.current&&(b(),p.current=window.requestAnimationFrame(()=>{p.current=null;let Z=c.current;!Z||!k&&!m.current||(m2(Z),y.current=Z.scrollTop,$.current=!1,v(!0))})))},[b]),S=h.default.useCallback(()=>{x.current!==null&&(window.cancelAnimationFrame(x.current),x.current=null)},[]);h.default.useLayoutEffect(()=>{let k=e.length>0?e[e.length-1]:null,z=f2(k),Z=B4(f.current,k);return f.current=z,w(Z),b},[e,i,w,b]),h.default.useLayoutEffect(()=>{let k=d.current;if(!k||typeof ResizeObserver!="function")return;let z=new ResizeObserver(()=>{w()});return z.observe(k),()=>{z.disconnect(),b()}},[w,b]);let C=h.default.useCallback(()=>{x.current=null;let k=c.current;if(!k)return;let z=d2(k);y.current=k.scrollTop,z?(m.current=!0,$.current=!1,v(!0)):$.current?(m.current=!1,v(!1)):(m.current=!0,v(!0),w()),a&&k.scrollTop<j4&&n&&!t&&n()},[a,n,t,w]),_=h.default.useCallback(()=>{$.current=!0},[]),A=h.default.useCallback(k=>{let z=c.current;if(!z||typeof k?.clientX!="number")return;let Z=z.offsetWidth-z.clientWidth;if(Z<=0)return;let re=z.getBoundingClientRect().right;k.clientX>=re-Z-2&&($.current=!0)},[]),L=h.default.useCallback(()=>{let k=c.current;if(!k)return;let z=d2(k),Z=k.scrollTop<y.current;y.current=k.scrollTop,!z&&Z&&($.current=!0),z?(m.current=!0,$.current=!1):$.current?(m.current=!1,b()):m.current=!0,x.current===null&&(x.current=window.requestAnimationFrame(C))},[b,C]),M=h.default.useCallback(()=>{let k=c.current;k&&(m2(k),y.current=k.scrollTop,m.current=!0,$.current=!1,v(!0))},[]);h.default.useEffect(()=>S,[S]);let P=h.default.useMemo(()=>l2(e),[e]);return u`
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
        ${P.map(k=>k.type==="activity-run"?u`<${Q1} key=${k.id} activity=${k.activity} />`:u`<${a2}
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
  `}function h2({notice:e,onRecover:t}){return u`
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
  `}function v2({suggestions:e,onSelect:t,disabled:a=!1}){return!e||e.length===0?null:u`
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
  `}function g2(){return u`
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
  `}function Kc(){return H("/api/webchat/v2/channels/connectable")}function y2(e,t){if(!ph(e))return null;let a=Hc(e),n=K4(a),r=null;for(let s of t||[]){if(!I4(s))continue;let i=H4(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function ph(e){let t=Hc(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function z4(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function q4(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>b2(Hc(n))):a}function I4(e){return e?.strategy!=="admin_managed_channels"}function K4(e){return x2(e,"slack")&&b2(e)}function b2(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function Hc(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function H4(e,t,a={}){return(a.commandAliasesOnly?q4(t,{channelManagementOnly:!0}):z4(t)).reduce((r,s)=>{let i=Hc(s);return x2(e,i)?Math.max(r,i.length):r},0)}function x2(e,t){return t?` ${e} `.includes(` ${t} `):!1}function $2(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n={kind:"gate",gateKind:"approval",runId:t.turn_run_id,gateRef:t.gate_ref,invocationId:t.invocation_id||null,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return Q4(n,a,t.body)}return e==="auth_required"?{kind:"auth_required",gateKind:"auth",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,invocationId:t.invocation_id||null,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function w2(e){if(!e?.run_id||!e.gate_ref)return null;let t=e.gate_kind||"generic",a={gateKind:t,runId:e.run_id,gateRef:e.gate_ref,invocationId:e.invocation_id||null,headline:e.headline,body:e.body||"",allowAlways:e.allow_always===!0};if(t==="auth"){let n=e.auth_context||{};return{...a,kind:"auth_required",challengeKind:n.challenge_kind||"other",provider:n.provider||null,accountLabel:n.account_label||"",authorizationUrl:n.authorization_url||null,expiresAt:n.expires_at||null}}return{...a,kind:"gate"}}function Q4(e,t,a){if(!t)return e;let n=V4(t);return{...e,toolName:t.tool_name||null,description:t.reason||a,actionLabel:t.action?.label||null,destination:t.destination||null,approvalScope:t.scope||null,approvalDetails:n,parameters:n.length?n.map(r=>`${r.label}: ${r.value}`).join(`
`):null}}function V4(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function S2({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function N2(){return{terminalByInvocation:new Map}}function _2(e){e?.current?.terminalByInvocation?.clear()}function vh(e,t,a){let n=R2(t,{toolStatus:"running"});n&&hi(e,n,a)}function k2(e,t,a,n="gate_declined"){let r=R2(t,{toolStatus:"declined",toolError:n,toolErrorKind:"gate_declined"});r&&hi(e,r,a)}function hi(e,t,a){if(!t)return;let n=W4(t);n=Z4(n,a),e(r=>{let s=C2(n),i=Y4(r,n,s);if(i>=0){let l=[...r];return l[i]=J4(l[i],n),hh(l[i],a),l}let o={id:s,role:"tool_activity",...n};return hh(o,a),[...r,o]})}function R2(e,t={}){let a=e?.kind==="gate",n=e?.kind==="auth_required",r=n&&t.toolStatus==="declined";if(!e?.runId||!e?.gateRef||!e.invocationId&&!r||!a&&!n)return null;let s=e.invocationId||G4(e),i=e.toolName||e.headline||e.gateKind||"gate";return{invocationId:s,callId:s,capabilityId:e.toolName||e.gateKind||null,toolName:Xo(i)||i,toolStatus:t.toolStatus||"running",toolDetail:null,toolParameters:null,toolResultPreview:null,toolError:t.toolError||null,toolErrorKind:t.toolErrorKind||null,toolDurationMs:null,updatedAt:t.updatedAt||new Date().toISOString(),resultRef:null,truncated:!1,outputBytes:null,outputKind:null,turnRunId:e.runId,gateRef:e.gateRef,gateActivity:!0}}function G4(e){return`gate:${e.runId}:${e.kind}:${e.gateRef}`}function C2(e){return`tool-${e.invocationId}`}function Y4(e,t,a){let n=e.findIndex(s=>s?.id===a);if(n>=0)return n;let r=t.gateRef||null;if(r){let s=e.findIndex(i=>i?.role==="tool_activity"&&i.turnRunId===t.turnRunId&&i.gateRef===r);if(s>=0)return s}return-1}function J4(e,t){let a=Jo(e.toolStatus),n=Jo(t.toolStatus),r=a&&!n,s=t.gateActivity&&!e.gateActivity,i={...e,...t,id:e.id,role:"tool_activity",invocationId:e.gateActivity&&!t.gateActivity?t.invocationId:e.invocationId||t.invocationId,callId:e.gateActivity&&!t.gateActivity?t.callId:e.callId||t.callId,toolName:s?e.toolName:t.toolName||e.toolName,toolStatus:r?e.toolStatus:t.toolStatus,toolError:t.toolError||e.toolError,toolErrorKind:t.toolErrorKind||e.toolErrorKind||null,updatedAt:r?e.updatedAt||t.updatedAt:t.updatedAt||e.updatedAt,turnRunId:t.turnRunId||e.turnRunId||null,gateRef:t.gateRef||e.gateRef||null,gateActivity:e.gateActivity&&t.gateActivity,capabilityId:s?e.capabilityId||t.capabilityId||null:t.capabilityId||e.capabilityId||null,activityOrder:X4(e,t),activityOrderSource:t.activityOrderSource||e.activityOrderSource||null};return e.gateActivity&&!t.gateActivity&&(i.id=C2(t),i.gateActivity=!1),i}function X4(e,t){return Number.isFinite(t.activityOrder)?t.activityOrder:e.activityOrder}function Z4(e,t){if(!e?.invocationId)return e;if(Jo(e.toolStatus))return hh(e,t),e;let a=t?.current?.terminalByInvocation?.get(e.invocationId);return a?Number.isFinite(e.activityOrder)?{...a,activityOrder:e.activityOrder,activityOrderSource:e.activityOrderSource||a.activityOrderSource||null}:a:e}function hh(e,t){!e?.invocationId||!Jo(e.toolStatus)||t?.current?.terminalByInvocation?.set(e.invocationId,e)}function W4(e){let t=Xo(e.toolName||e.capabilityId);return{...e,toolName:t||e.toolName||"tool"}}function M2({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o,onRunSettled:l}){let c=h.default.useRef(new Set),d=h.default.useRef(null),m=h.default.useRef(null);return h.default.useCallback(f=>{let{type:p,frame:x}=f||{};if(!(!p||!x))switch(p){case"accepted":{let y=x.ack||{};y.run_id&&(d.current=y.run_id),r?.({runId:y.run_id||null,threadId:y.thread_id||e,status:y.status||null}),a(!0);return}case"running":case"capability_progress":{let y=x.progress||{};y.turn_run_id&&(d.current=y.turn_run_id,r?.($=>$&&$.runId===y.turn_run_id?{...$,status:"running"}:{runId:y.turn_run_id,threadId:e,status:"running"}),e5(n,y.turn_run_id,m)),a(!0);return}case"capability_activity":{let y=x.activity;if(!y||!y.invocation_id)return;hi(t,zp(y),o);return}case"capability_display_preview":{let y=x.preview;if(!y||!y.invocation_id)return;let $=Bp(y);hi(t,$,o);return}case"gate":case"auth_required":{let y=$2(p,x.prompt);y&&(vh(t,y,o),n(y),r?.({runId:y.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let y=x.reply||{};t($=>[...$,{id:`reply-${y.turn_run_id||Date.now()}`,role:"assistant",content:y.text||"",timestamp:y.generated_at||new Date().toISOString(),turnRunId:y.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let y=x.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Gc(c,l,y,!1);return}case"failed":{let y=x.run_state||{},$=y.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),yh(t,{runId:$,status:y.status||"failed",failureCategory:r5(y),failureSummary:null}),Gc(c,l,$,!1);return}case"projection_snapshot":case"projection_update":{let y=x.state?.items||[];a5({items:y,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:l,settledRunsRef:c,latestRunIdRef:d,promptRunIdRef:m,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i,o,l])}function Gc(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var E2=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),T2=new Set(["completed","succeeded"]),Qc=new Set(["blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]),Vc=new Set(["awaiting_gate","blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]);function A2(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function e5(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function t5(e,t,a,n,r,s){let i=t?.runId||null;if(!i||r?.has(i))return!0;let o=a?.get(i);if(o)return!Vc.has(o);let l=e?.current,c=l?.runId||n?.current||null;if(c&&i!==c)return!0;let d=s?.current===c;return c&&i===c&&!d&&l?.status&&!Vc.has(l.status)?!0:!l?.runId||!l.status?!1:!Vc.has(l.status)}function a5({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:l,promptRunIdRef:c,activeRunRef:d,locallyResolvedGatesRef:m,toolActivityStateRef:f}){let p=new Map,x=new Set,y=d?.current||null,$=y?.runId||l?.current||null;for(let v of e){let b=v.run_status;b?.run_id&&b.status&&(p.set(b.run_id,b.status),$&&$!==b.run_id&&y?.status&&!E2.has(y.status)&&Qc.has(b.status)&&x.add(b.run_id))}let g=l?.current??null;for(let v of e){if(v.run_status){let{run_id:b,status:w,failure_category:S,failure_summary:C}=v.run_status,_=E2.has(w),A=d?.current?.source==="local"?d.current.runId:null,L=!!(b&&A&&A!==b),M=g??l?.current??null,P=!!(_&&b&&M&&M!==b),k=b&&Qc.has(w)?D2(m,b):null;if(b&&x.has(b)||L)continue;if(P){D2(m,d?.current?.runId)?.outcome==="resumed"&&(n5({runId:b,activePromptRunId:d?.current?.runId,success:T2.has(w),status:w,failureCategory:S,failureSummary:C,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:l,promptRunIdRef:c,locallyResolvedGatesRef:m}),g=null);continue}if(k){A2(r,b,c),k.outcome==="resumed"?(n(!0),s?.(z=>z&&z.runId===b?{...z,status:z.status==="awaiting_gate"?"queued":z.status||"queued"}:{runId:b,threadId:t,status:"queued"}),g=b,l&&(l.current=b)):(n(!1),d?.current?.runId===b&&s?.(null),g=null,l?.current===b&&(l.current=null));continue}b&&(g=b,!_&&l&&(l.current=b),s?.(z=>z&&z.runId===b?{...z,status:w}:{runId:b,threadId:t,status:w})),b&&Qc.has(w)?c&&(c.current=b):b&&c?.current===b&&(c.current=null),_?(n(!1),r(null),s?.(null),gh(m,b),g=null,l&&(l.current=null),b&&c?.current===b&&(c.current=null),Gc(o,i,b,T2.has(w)),(w==="failed"||w==="recovery_required")&&yh(a,{runId:b,status:w,failureCategory:S,failureSummary:C})):Qc.has(w)||(A2(r,b,c),gh(m,b),n(!0))}if(v.text){let b=`text-${v.text.id}`;a(w=>{let S=w.findIndex(_=>_.id===b),C={id:b,role:"assistant",content:v.text.body||"",timestamp:new Date().toISOString(),isFinalReply:!0};if(S>=0){let _=[...w];return _[S]=C,_}return[...w,C]}),n(!1)}if(v.thinking){let b=`thinking-${v.thinking.id}`;a(w=>{let S=w.findIndex(_=>_.id===b),C={id:b,role:"thinking",content:v.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:v.thinking.run_id||null};if(S>=0){let _=[...w];return _[S]=C,_}return[...w,C]})}if(v.capability_activity){let b=v.capability_activity;b.invocation_id&&hi(a,zp(b),f)}if(v.gate){let b=w2(v.gate),w=b?.runId||null;w&&!t5(d,b,p,l,x,c)&&!i5(m,w,b.gateRef)&&(vh(a,b,f),r(S=>S||b),s?.(S=>S&&S.runId===w?{...S,status:Vc.has(S.status)?S.status:"awaiting_gate"}:{runId:w,threadId:t,status:"awaiting_gate"}),c&&(c.current=w),n(!1))}if(v.skill_activation){let{id:b,skill_names:w=[],feedback:S=[]}=v.skill_activation;if(w.length||S.length){let C=`skill-${b||w.join("-")||"activation"}`,_=[w.length?`Skill activated: ${w.join(", ")}`:"",...S].filter(Boolean).join(`
`);a(A=>A.some(L=>L.id===C)?A:[...A,{id:C,role:"system",content:_,timestamp:new Date().toISOString()}])}}}l&&g&&(l.current=g)}function n5({runId:e,activePromptRunId:t,success:a,status:n,failureCategory:r,failureSummary:s,setMessages:i,setIsProcessing:o,setPendingGate:l,setActiveRun:c,onRunSettled:d,settledRunsRef:m,latestRunIdRef:f,promptRunIdRef:p,locallyResolvedGatesRef:x}){o(!1),l(null),c?.(null),gh(x,t),f&&(f.current=null),p?.current===t&&(p.current=null),Gc(m,d,e,a),(n==="failed"||n==="recovery_required")&&yh(i,{runId:e,status:n,failureCategory:r,failureSummary:s})}function r5(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function yh(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`,i=typeof t=="string"&&t?t:null;e(o=>{let l=o.findIndex(d=>d.id===s),c=S2({status:a,failureCategory:n,failureSummary:r});if(l>=0){let d=!!(r&&o[l].content!==c),m=!!(i&&o[l].turnRunId!==i);if(!d&&!m)return o;let f=[...o];return f[l]={...f[l],...d&&{content:c},...m&&{turnRunId:i}},f}return[...o,{id:s,role:"error",content:c,timestamp:new Date().toISOString(),...i&&{turnRunId:i}}]})}function D2(e,t){if(!t)return null;let a=e?.current;if(!a)return null;for(let[n,r]of a.entries())if(n.startsWith(`${t}
`))return s5(r);return null}function s5(e){return e&&typeof e=="object"?{resolution:e.resolution||null,outcome:e.outcome||null}:{resolution:e||null,outcome:null}}function gh(e,t){if(!t)return;let a=e?.current;if(a)for(let n of Array.from(a.keys()))n.startsWith(`${t}
`)&&a.delete(n)}function i5(e,t,a){return!t||!a?!1:!!e?.current?.has(`${t}
${a}`)}function O2(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function L2(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function P2(e,t,a,n){let r=bh(n);return r?(o5(e,t,a,{timelineMessageId:r}),r):null}function o5(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function bh(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var l5=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function U2({threadId:e,onEvent:t,enabled:a}){let[n,r]=h.default.useState("idle"),s=h.default.useRef(t);s.current=t;let i=h.default.useRef(null);return h.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,l=null,c=0,d=3e4;function m(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=Jx({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let y=Math.min(1e3*2**c,d);l=setTimeout(m,y)};let x=(y,$)=>{let g=null;try{g=JSON.parse(y.data)}catch{return}!g||typeof g!="object"||(y.lastEventId&&(i.current=y.lastEventId),s.current?.({type:g.type||$,frame:g,lastEventId:y.lastEventId||null}))};o.onmessage=y=>x(y,"message");for(let y of l5)o.addEventListener(y,$=>x($,y))}function f(){l&&(clearTimeout(l),l=null),o&&(o.close(),o=null),r("paused")}function p(){document.visibilityState==="hidden"?f():o||m()}return m(),document.addEventListener("visibilitychange",p),()=>{document.removeEventListener("visibilitychange",p),l&&clearTimeout(l),o&&o.close()}},[a,e]),{status:n}}var u5=3e4,c5="credential_stored_gate_resolution_failed",d5="approval_gate_pending_send_blocked",m5="ironclaw-product-auth",xh="ironclaw:product-auth:oauth-complete",f5="ironclaw:product-auth:oauth-complete";async function j2(e){let t=new AbortController,a=setTimeout(()=>t.abort(),u5);try{return await e(t.signal)}finally{clearTimeout(a)}}function p5(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=c5,t.cause=e,t}function h5(){let e=new Error("Resolve the approval request before sending another message.");return e.safeErrorCode=d5,e}function v5(e){let a=Ct.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function F2(e,t){return!e||!t?.runId||!t?.gateRef?null:`${e}
${t.runId}
${t.gateRef}`}function g5(e){return e?.continuation?.type==="turn_gate_resume"}function y5(e){if(e?.outcome)return e.outcome;let t=String(e?.status||"").toLowerCase();return t==="queued"||t==="running"?"resumed":t==="cancelled"||e?.already_terminal===!0?"cancelled":e?.already_terminal===!1?"resumed":null}function B2(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function b5(e){return e?.type===f5&&e?.status==="completed"}function x5(e,t,a){if(!b5(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function $h(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function $5(e){if(!ph(e))return null;try{let a=(await Ct.fetchQuery({queryKey:["connectable-channels"],queryFn:Kc}))?.channels||[];return y2(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function z2(e){let t=h.default.useRef(e),a=h.default.useRef(new Map),n=h.default.useRef(1),[r,s]=h.default.useState(0),[i,o]=h.default.useState(Date.now()),[l,c]=h.default.useState(null),d=h.default.useRef(l),m=h.default.useCallback(se=>{let ne=typeof se=="function"?se(d.current):se;d.current=ne,c(ne)},[]);h.default.useEffect(()=>{d.current=l},[l]);let[f,p]=h.default.useState(null),x=h.default.useCallback(()=>a.current.get(e||"__new__")||[],[e]),y=h.default.useCallback(se=>{let ne=e||"__new__";se.length>0?a.current.set(ne,se):a.current.delete(ne)},[e]),{messages:$,hasMore:g,nextCursor:v,isLoading:b,loadError:w,loadHistory:S,seedThreadMessages:C,setMessages:_}=N$(e,{getPendingMessages:x,setPendingMessages:y}),[A,L]=h.default.useState(!1),M=h.default.useRef(A),P=h.default.useCallback(se=>{let ne=typeof se=="function"?se(M.current):se;M.current=ne,L(ne)},[]),[k,z]=h.default.useState(null),Z=h.default.useRef(k),[re,me]=h.default.useState(null),he=h.default.useCallback(se=>{let ne=Z.current,fe=typeof se=="function"?se(ne):se;Object.is(fe,ne)||(Z.current=fe,z(fe))},[]),[Te,je]=h.default.useState(e),gt=h.default.useRef(N2()),ft=h.default.useRef(new Map),tt=h.default.useRef({gateKey:null,credentialRef:null,inFlight:!1}),Ge=h.default.useRef(!1),yt=h.default.useRef(new Set),At=h.default.useRef(new Map);Te!==e&&(je(e),L(!1),z(null),me(null),c(null),p(null)),h.default.useEffect(()=>{t.current=e},[e]),h.default.useEffect(()=>{Z.current=k},[k]),h.default.useEffect(()=>{M.current=A},[A]),h.default.useEffect(()=>{let se=F2(e,k);me(ne=>ne&&ne.gateKey!==se?null:ne)},[k,e]),h.default.useEffect(()=>{_2(gt),ft.current.clear()},[e]);let _n=Math.max(0,Math.ceil((r-i)/1e3)),Kt=k?.runId&&k?.gateRef?`${k.runId}
${k.gateRef}`:null;h.default.useEffect(()=>{if(!r)return;let se=setInterval(()=>o(Date.now()),250);return()=>clearInterval(se)},[r]),h.default.useEffect(()=>{tt.current.gateKey!==Kt&&(tt.current={gateKey:Kt,credentialRef:null,inFlight:!1})},[Kt]),h.default.useEffect(()=>{if(!B2(k))return;let se=Date.now(),ne=E=>{x5(E,k,se)&&(he(j=>B2(j)?null:j),P(!0))},fe=null;typeof window.BroadcastChannel=="function"&&(fe=new window.BroadcastChannel(m5),fe.onmessage=E=>ne(E.data));let N=E=>{E.key===xh&&ne($h(E.newValue))};window.addEventListener("storage",N),ne($h(window.localStorage?.getItem?.(xh)));let T=window.setInterval(()=>{ne($h(window.localStorage?.getItem?.(xh)))},500);return()=>{window.clearInterval(T),fe&&fe.close(),window.removeEventListener("storage",N)}},[k]);let Ea=M2({threadId:e,setMessages:_,setIsProcessing:P,setPendingGate:he,setActiveRun:m,activeRunRef:d,locallyResolvedGatesRef:ft,toolActivityStateRef:gt,onRunSettled:(se,{success:ne})=>{let fe=At.current.get(se);fe&&(At.current.delete(se),yt.current.delete(fe)),ne&&y([]),S(void 0,{preserveClientOnly:!0,finalReplyTimestampByRun:se&&ne?{[se]:new Date().toISOString()}:null})}}),{status:ot}=U2({threadId:e,onEvent:Ea,enabled:!!e}),Dt=h.default.useCallback(async(se,ne={})=>{let{threadId:fe,attachments:N=[]}=ne,T=N.map(h$),E=N.map(v$);if(k||Z.current)throw h5();let j=fe||e,B=d.current,F=!!B&&!!j&&B.threadId===j,J=M.current&&!!j&&j===e,ve=!!j&&yt.current.has(j);if(Ge.current||J||F||ve)return null;if(N.length===0){let ie=await $5(se);if(ie)return p(ie),{channel_connect_action:ie}}p(null);let ae=fe||e;if(!ae){let ie=await gc();if(Ct.invalidateQueries({queryKey:["threads"]}),ae=ie?.thread?.thread_id,!ae)throw new Error("createThread returned no thread_id")}let bt=ae,Za={id:`pending-${n.current++}`,role:"user",content:se,attachments:E,timestamp:new Date().toISOString(),isOptimistic:!0},xa={id:Za.id,role:"user",content:se,attachments:E,timestamp:Za.timestamp,isOptimistic:!0};O2(a.current,bt,Za);let fr=Za.id,Wa=!e||ae===e,kn=ie=>{Wa&&_(ie)},Wr=ie=>{ae!==e&&C(ae,ie)},es=ie=>{Wa&&ie()},Rn=Wa;Rn&&yt.current.add(ae),Ge.current=!0,kn(ie=>[...ie,xa]),Wr(ie=>[...ie,xa]),es(()=>{P(!0),Z.current||he(null)});try{let ie=await Vx({threadId:ae,content:se,attachments:T});v5(ae)&&Ct.invalidateQueries({queryKey:["threads"]}),ie?.run_id&&Rn&&At.current.set(ie.run_id,ae),ie?.run_id&&Wa&&m({runId:ie.run_id,threadId:ie.thread_id||ae,status:ie.status||null,source:"local"});let Ei=P2(a.current,bt,fr,ie?.accepted_message_ref)||bh(ie?.accepted_message_ref);if(Ei){let pr=Aa=>Aa.map(Cn=>Cn.id===fr?{...Cn,timelineMessageId:Ei}:Cn);kn(pr),Wr(pr)}if(ie?.outcome==="rejected_busy"){ie?.run_id&&Rn&&At.current.delete(ie.run_id),Rn&&yt.current.delete(ae);let pr=Aa=>Aa.map(Cn=>Cn.id===fr?{...Cn,isOptimistic:!1,status:"error"}:Cn);if(kn(pr),Wr(pr),ie?.notice){let Aa=(Ti=Wa)=>{let fR={id:`system-rejected-${n.current++}`,role:"system",content:ie.notice,timestamp:new Date().toISOString(),isOptimistic:!1},Jh=pR=>[...pR,fR];Ti&&_(Jh),(!Ti||ae!==e)&&C(ae,Jh)};if(!t.current||t.current===ae){let Ti=F2(ae,Z.current);Ti?me({gateKey:Ti,content:ie.notice}):Aa()}else Aa(!1)}es(()=>P(!1)),Ge.current=!1}else ie?.run_id||(Rn&&yt.current.delete(ae),Ge.current=!1);return ie}catch(ie){Rn&&yt.current.delete(ae),ie.status===429&&s(Date.now()+S5(ie));let Ei=pr=>pr.map(Aa=>Aa.id===fr?{...Aa,isOptimistic:!1,status:"error",error:ie.message}:Aa);throw kn(Ei),Wr(Ei),es(()=>P(!1)),Ge.current=!1,ie}finally{Ge.current=!1,L2(a.current,bt,fr)}},[e,k,_,C,P,he,m]),ia=h.default.useCallback(async(se,ne={})=>{if(!k)return;let{runId:fe,gateRef:N}=k;if(!fe||!N)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");let T=await Lp({threadId:e,runId:fe,gateRef:N,resolution:se,always:ne.always,credentialRef:ne.credentialRef}),E=y5(T);if(ft.current.set(`${fe}
${N}`,{resolution:se,outcome:E}),w5(se)&&E==="resumed"&&k2(_,k,gt),he(null),E==="resumed"){P(!0),m({runId:T?.run_id||fe,threadId:T?.thread_id||e,status:T?.status||"queued"});return}P(!1),m(null)},[k,e,_,m]),oa=h.default.useCallback(async se=>{if(!k)throw new Error("auth gate is no longer pending");let{runId:ne,gateRef:fe,provider:N}=k;if(!ne||!fe||!N)throw new Error("auth gate is missing required credential metadata");let T=k.accountLabel||`${N} credential`,E=`${ne}
${fe}`;if(tt.current.gateKey!==E&&(tt.current={gateKey:E,credentialRef:null,inFlight:!1}),tt.current.inFlight)throw new Error("auth token submission already in progress");tt.current.inFlight=!0;try{let j=tt.current.credentialRef,B=null;if(!j){if(B=await j2(F=>Zx({provider:N,accountLabel:T,token:se,threadId:e,runId:ne,gateRef:fe,signal:F})),j=B?.credential_ref,!j)throw new Error("manual token submit returned no credential_ref");tt.current.credentialRef=j}if(!g5(B))try{await j2(F=>Lp({threadId:e,runId:ne,gateRef:fe,resolution:"credential_provided",credentialRef:j,signal:F}))}catch(F){throw p5(F)}tt.current={gateKey:null,credentialRef:null,inFlight:!1},he(null),P(!0)}catch(j){throw tt.current.gateKey===E&&(tt.current.inFlight=!1),j}},[k,e]),Ta=h.default.useCallback(async se=>{let ne=l?.runId;if(!ne||!e)return;he(null),P(!1),m(null),Ge.current=!1;let fe=At.current.get(ne)||l?.threadId||e;At.current.delete(ne),fe&&yt.current.delete(fe),await Xx({threadId:e,runId:ne,reason:se})},[l,e]),Xr=h.default.useCallback(()=>{g&&v&&S(v)},[g,v,S]),Zr=h.default.useCallback(async(se,ne,fe)=>{let N="approved",T=!1;ne==="deny"?N="denied":ne==="cancel"?N="cancelled":ne==="always"&&(N="approved",T=!0),await ia(N,{always:T})},[ia]),Xa=h.default.useCallback(()=>{},[]);return{messages:$,isProcessing:A,pendingGate:k,busyGateNotice:re,channelConnectAction:f,activeRun:l,sseStatus:ot,historyLoading:b,historyLoadError:w,hasMore:g,cooldownSeconds:_n,send:Dt,resolveGate:ia,submitAuthToken:oa,cancelRun:Ta,loadMore:Xr,dismissChannelConnectAction:()=>p(null),suggestions:[],setSuggestions:Xa,retryMessage:Xa,approve:Zr,recoverHistory:Xa,recoveryNotice:null}}function w5(e){return e==="denied"||e==="cancelled"}function S5(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function q2({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}var N5=1500;function I2({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let o=R(),{messages:l,isProcessing:c,pendingGate:d,busyGateNotice:m,channelConnectAction:f,suggestions:p,sseStatus:x,historyLoading:y,historyLoadError:$,hasMore:g,cooldownSeconds:v,recoveryNotice:b,activeRun:w,send:S,cancelRun:C,retryMessage:_,approve:A,recoverHistory:L,loadMore:M,setSuggestions:P,submitAuthToken:k,dismissChannelConnectAction:z}=z2(t),Z=h.default.useMemo(()=>e.find(ot=>ot.id===t)||null,[e,t]),re=h.default.useMemo(()=>q2({gatewayStatus:i,activeThread:Z}),[i,Z]),me=l.length>0||c||!!d||!!f,he=!y&&!me&&!$,Te=d?"Resolve the approval request before sending another message.":"",je=!!d||c&&!d||v>0,gt=h.default.useRef(je);gt.current=je;let ft=Te||(v>0?`Retry in ${v}s`:void 0),tt=t||tl,Ge=!!(t&&w?.runId&&w.threadId===t&&c&&!d),yt=h.default.useCallback(async(ot,{images:Dt=[],attachments:ia=[]}={})=>{if(d)throw new Error(Te);if(gt.current)return null;let oa=await S(ot,{images:Dt,attachments:ia,threadId:t}),Ta=oa?.thread_id||t;return!t&&Ta&&a&&a(Ta,{replace:!0}),oa},[t,Te,je,a,d,S]),At=h.default.useCallback(async ot=>{je||(P([]),await yt(ot))},[je,yt,P]),_n=h.default.useCallback(()=>C("user_requested"),[C]);h.default.useEffect(()=>{if(!t)return;if(d){Rc(t,$n.NEEDS_ATTENTION);return}if(c){Rc(t,$n.RUNNING);return}let ot=setTimeout(()=>Lw(t),N5);return()=>clearTimeout(ot)},[t,d,c]);let[Kt,Ea]=h.default.useState(!1);return h.default.useEffect(()=>{let ot=Dt=>{if(Dt.key==="Escape"){Ea(!1);return}if(Dt.key!=="?")return;let ia=Dt.target,oa=ia?.tagName;oa==="INPUT"||oa==="TEXTAREA"||ia?.isContentEditable||(Dt.preventDefault(),Ea(Ta=>!Ta))};return window.addEventListener("keydown",ot),()=>window.removeEventListener("keydown",ot)},[]),u`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${j1} status=${x} />

        ${$&&u`
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            ${$}
          </div>
        `}

        ${he&&u`
          <${F1}
            onSuggestion=${At}
            onSend=${yt}
            disabled=${!1}
            sendDisabled=${je}
            initialText=${r}
            resetKey=${s}
            draftKey=${tt}
            context=${re}
            statusText=${ft}
            canCancel=${Ge}
            onCancel=${_n}
          />
        `}
        ${!he&&u`
          <${p2}
            messages=${l}
            isLoading=${y}
            hasMore=${g}
            onLoadMore=${M}
            onRetryMessage=${_}
            threadId=${t}
            pending=${c}
          >
            ${b&&u`
              <${h2}
                notice=${b}
                onRecover=${L}
              />
            `}
            ${c&&!d&&u`<${g2} />`}
            ${f&&u`
              <${L1}
                connectAction=${f}
                onDismiss=${z}
              />
            `}
            ${d&&(d.kind==="auth_required"?d.challengeKind==="oauth_url"?u`
                  <${D1}
                    gate=${d}
                    onCancel=${()=>A(d.requestId,"cancel",d.kind)}
                  />
                `:d.challengeKind==="manual_token"?u`
                  <${M1}
                    gate=${d}
                    onSubmit=${k}
                    onCancel=${()=>A(d.requestId,"cancel",d.kind)}
                  />
                `:u`
                  <${A1}
                    gate=${d}
                    onCancel=${()=>A(d.requestId,"cancel",d.kind)}
                  />
                `:u`
              <${T1}
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

          <${v2}
            suggestions=${p}
            onSelect=${At}
            disabled=${je}
          />

          <${Fc}
            onSend=${yt}
            disabled=${!1}
            sendDisabled=${je}
            initialText=${r}
            resetKey=${s}
            draftKey=${tt}
            context=${re}
            statusText=${ft}
            canCancel=${Ge}
            onCancel=${_n}
          />
        `}
      </div>
      <${B1}
        open=${Kt}
        onClose=${()=>Ea(!1)}
      />
    </div>
  `}function wh(){let{threadsState:e,gatewayStatus:t}=ya(),{threadId:a}=st(),n=pe(),r=Pe(),s=r.state?.composerDraft||"",i=a||null;h.default.useEffect(()=>{i&&i!==e.activeThreadId?e.setActiveThreadId(i):i||e.setActiveThreadId(null)},[i]);let o=h.default.useCallback((l,c={})=>{if(!l){e.setActiveThreadId(null),n("/chat",c);return}e.setActiveThreadId(l),n(`/chat/${l}`,c)},[e,n]);return u`
    <${I2}
      threads=${e.threads}
      activeThreadId=${i}
      onSelectThread=${o}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function K2(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?si(e,t):"",model:e?_c(e,t):""}}function H2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:l}){let[c,d]=h.default.useState(()=>K2(e,a)),[m,f]=h.default.useState(""),[p,x]=h.default.useState([]),[y,$]=h.default.useState(null),[g,v]=h.default.useState(""),b=h.default.useRef(!!e);h.default.useEffect(()=>{n&&(d(K2(e,a)),f(""),x([]),$(null),v(""),b.current=!!e)},[n,e,a]);let w=e?.builtin===!0,S=e&&!e.builtin,C=h.default.useCallback((P,k)=>{d(z=>{let Z={...z,[P]:k};return P==="name"&&!b.current&&(Z.id=dw(k)),Z})},[]),_=h.default.useCallback(()=>!w&&(!c.name.trim()||!c.id.trim())?l("llm.fieldsRequired"):!w&&!mw(c.id.trim())?l("llm.invalidId"):!S&&!w&&t.includes(c.id.trim())?l("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,w,S,l]),A=h.default.useCallback(async()=>{let P=_();if(P){$({tone:"error",text:P});return}v("save");try{await s({form:c,apiKey:m,provider:e}),r()}catch(k){$({tone:"error",text:k.message})}finally{v("")}},[m,c,r,s,e,_]),L=h.default.useCallback(async()=>{if(!c.model.trim()){$({tone:"error",text:l("llm.modelRequired")});return}v("test");try{let P=await i(Jp(e,c,m,a));$({tone:P.ok?"success":"error",text:P.message})}catch(P){$({tone:"error",text:P.message})}finally{v("")}},[m,a,c,i,e,l]),M=h.default.useCallback(async()=>{if((w?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){$({tone:"error",text:l("llm.baseUrlRequired")});return}v("models");try{let k=await o(Jp(e,c,m,a));if(!k.ok||!Array.isArray(k.models)||!k.models.length)$({tone:"error",text:k.message||l("llm.modelsFetchFailed")});else{x(k.models);let z=fw(c.model,k.models);z!==null&&C("model",z),$({tone:"success",text:l("llm.modelsFetched",{count:k.models.length})})}}catch(k){$({tone:"error",text:k.message})}finally{v("")}},[m,a,c,w,o,e,l,C]);return{form:c,apiKey:m,models:p,message:y,busy:g,isBuiltin:w,isEditing:S,setApiKey:f,update:C,submit:A,runTest:L,fetchModels:M,markIdEdited:()=>{b.current=!0}}}function Yc({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let l=R(),c=H2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:l});if(!n)return null;let{form:d,apiKey:m,models:f,message:p,busy:x,isBuiltin:y,isEditing:$}=c,g=y?l("llm.configureProvider",{name:e.name||e.id}):l($?"llm.editProvider":"llm.newProvider");return u`
    <${mi} open=${n} onClose=${r} title=${g} size="lg">
      <${fi} className="space-y-4">
        ${!y&&u`
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${l("llm.providerName")}
              <${Tt} value=${d.name} onChange=${v=>c.update("name",v.target.value)} />
            </label>
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${l("llm.providerId")}
              <${Tt}
                value=${d.id}
                disabled=${$}
                onChange=${v=>{c.markIdEdited(),c.update("id",v.target.value)}}
              />
            </label>
          </div>
          <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
            ${l("llm.adapter")}
            <${uh} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${Yp.map(v=>u`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
            <//>
          </label>
        `}

        ${y&&u`
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            ${rl(e.adapter)}
          </div>
        `}

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${l("llm.baseUrl")}
          <${Tt} value=${d.baseUrl} placeholder=${e?.base_url||""} onChange=${v=>c.update("baseUrl",v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${l("llm.apiKey")}
          <${Tt} type="password" value=${m} placeholder=${l("llm.apiKeyPlaceholder")} onChange=${v=>c.setApiKey(v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${l("llm.defaultModel")}
          <div className="flex items-stretch gap-2">
            <${Tt} value=${d.model} onChange=${v=>c.update("model",v.target.value)} />
            <${D} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${x!==""} onClick=${c.fetchModels}>
              ${l(x==="models"?"llm.fetchingModels":"llm.fetchModels")}
            <//>
          </div>
        </label>

        ${f.length>0&&u`
          <${uh} value=${d.model} onChange=${v=>c.update("model",v.target.value)}>
            ${f.map(v=>u`<option key=${v} value=${v}>${v}</option>`)}
          <//>
        `}

        ${p&&u`
          <div className=${p.tone==="error"?"text-sm text-red-200":"text-sm text-mint"} role="status">
            ${p.text}
          </div>
        `}
      <//>
      <${pi}>
        <${D} type="button" variant="secondary" disabled=${x!==""} onClick=${c.runTest}>
          ${l(x==="test"?"llm.testing":"llm.testConnection")}
        <//>
        <${D} type="button" variant="ghost" disabled=${x!==""} onClick=${r}>${l("common.cancel")}<//>
        <${D} type="button" disabled=${x!==""} onClick=${c.submit}>
          ${l(x==="save"?"common.saving":"common.save")}
        <//>
      <//>
    <//>
  `}function Jc({login:e}){let t=R(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return u`
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
  `}function _5(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function Xc({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=ii({settings:e,gatewayStatus:t}),[s,i]=h.default.useState(null),[o,l]=h.default.useState(!1),[c,d]=h.default.useState(null),m=h.default.useRef(null),f=h.default.useCallback((g,v)=>{m.current&&window.clearTimeout(m.current),d({tone:g,text:v}),m.current=window.setTimeout(()=>d(null),3500)},[]);h.default.useEffect(()=>()=>{m.current&&window.clearTimeout(m.current)},[]);let p=h.default.useCallback((g=null)=>{i(g),l(!0)},[]),x=h.default.useCallback(async g=>{try{await r.setActiveProvider(g),f("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(p(g),f("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):f("error",v.message)}},[p,r,f,n]),y=h.default.useCallback(async({form:g,apiKey:v,provider:b})=>{if(b?.builtin){await r.saveBuiltinProvider({provider:b,form:g,apiKey:v}),f("success",n("llm.providerConfigured",{name:b.name||b.id}));return}let w=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:b});f("success",n(b?"llm.providerUpdated":"llm.providerAdded",{name:w.name||w.id}))},[r,f,n]),$=h.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),f("success",n("llm.providerDeleted"))}catch(v){f("error",v.message)}},[r,f,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>_5(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:p,closeDialog:()=>l(!1),handleUse:x,handleSave:y,handleDelete:$}}var k5=3e5;function R5(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function C5(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function E5(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=l=>{let c=l.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},k5);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var T5=3e5,A5=9e5,D5=2e3;async function Q2(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,D5)),(await Nc().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function Zc({onSuccess:e}={}){let t=R(),a=Y(),[n,r]=h.default.useState(!1),[s,i]=h.default.useState(""),[o,l]=h.default.useState(!1),[c,d]=h.default.useState(""),[m,f]=h.default.useState(null),p=h.default.useCallback(()=>{i(""),d(""),f(null)},[]),x=h.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),y=h.default.useCallback(async v=>{if(p(),R5()){i(t("onboarding.nearaiLocalSso"));return}let b=window.open("about:blank","_blank");if(!b){i(t("onboarding.nearaiFailed"));return}try{b.opener=null}catch{}r(!0);try{let{auth_url:w}=await I$({provider:v,origin:window.location.origin});b.location.href=w;let S=await Q2("nearai",T5,b);if(S==="active"){await x();return}b.close(),i(t(S==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{b.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,p,t]),$=h.default.useCallback(async()=>{p(),r(!0);try{let v=C5(),b=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!b){i(t("onboarding.nearaiFailed"));return}b.opener=null;let w=await E5(b,v);if(!w){i(t("onboarding.nearaiFailed"));return}await K$({account_id:w.accountId,public_key:w.publicKey,signature:w.signature,message:w.message,recipient:w.recipient,nonce:w.nonce}),await x()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[x,p,t]),g=h.default.useCallback(async()=>{p();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}l(!0);try{let{user_code:b,verification_uri:w}=await H$();f({userCode:b,verificationUri:w}),v&&(v.location.href=w);let S=await Q2("openai_codex",A5,v);if(S==="active"){await x();return}v&&v.close(),d(t(S==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{l(!1)}},[x,p,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:m,startNearai:y,startNearaiWallet:$,startCodex:g}}var V2="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",M5="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",O5="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",L5="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",P5={nearai:{color:"#00ec97",path:M5},openai_codex:{color:"#10a37f",path:V2},openai:{color:"#10a37f",path:V2},anthropic:{color:"#d97757",path:O5},ollama:{color:null,path:L5}};function G2({id:e,name:t}){let a=P5[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return u`
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
  `}var U5=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function j5({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=h.default.useState(!1),o=h.default.useRef(null),l=t||a.nearaiBusy;h.default.useEffect(()=>{if(!s)return;let d=f=>{o.current&&!o.current.contains(f.target)&&i(!1)},m=f=>{f.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",m),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",m)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return u`
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
  `}function F5({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let l=s(e.nameKey),c;return e.auth==="nearai"?c=u`<${j5} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=u`
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
        <${G2} id=${e.id} name=${l} />
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
  `}function Y2(){let{isAdmin:e=!1,isChecking:t=!1}=ya();return t?null:e?u`<${B5} />`:u`<${it} to="/chat" replace />`}function B5(){let e=R(),t=pe(),a=Y(),{gatewayStatus:n}=ya(),r=Xc({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=U5.map(m=>({entry:m,provider:s.providers.find(f=>f.id===m.id)})).filter(m=>m.provider),o=h.default.useCallback(()=>t("/chat"),[t]),l=Zc({onSuccess:o}),c=h.default.useCallback(async m=>{let f=m.active_model||m.default_model||"";await nl({provider_id:m.id,model:f}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=h.default.useCallback(async({form:m,apiKey:f,provider:p})=>{await r.handleSave({form:m,apiKey:f,provider:p});let x=p?.id||m.id.trim(),y=m.model?.trim()||p?.default_model||"";await nl({provider_id:x,model:y}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?u`
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
              <${F5}
                key=${m.id}
                entry=${m}
                provider=${f}
                configured=${Ir(f,s.builtinOverrides)}
                isBusy=${s.isBusy}
                login=${l}
                t=${e}
                onUse=${c}
                onSetUp=${r.openDialog}
              />
            `)}
        </div>

        <${Jc} login=${l} />

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

      <${Yc}
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
  `}function J2({items:e}){return u`
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
  `;return n?u`<${ee} padding="lg">${r}<//>`:u`<div className="py-8">${r}</div>`}var X2={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function Ya({result:e,onDismiss:t}){return e?u`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",X2[e.type]||X2.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}var Z2="",z5={workspace:"home"};function Wc(e){return z5[e]||e}function dl(e){return[...e||[]].sort((t,a)=>t.is_dir!==a.is_dir?t.is_dir?-1:1:t.name.localeCompare(a.name,void 0,{sensitivity:"base"}))}function vi(e){return e?e.split("/").filter(Boolean):[]}function ed(e){return e?`/workspace/${vi(e).map(encodeURIComponent).join("/")}`:"/workspace"}function Sh(e){let t=vi(e);return t.pop(),t.join("/")}function W2(e){return/\.mdx?$/i.test(e||"")}function td({path:e,onNavigate:t}){let a=R(),n=vi(e),r="";return u`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button
        type="button"
        onClick=${()=>t("/workspace")}
        className="text-signal hover:underline"
      >
        ${a("workspace.breadcrumbRoot")}
      </button>
      ${n.map((s,i)=>{r=r?`${r}/${s}`:s;let o=r,l=i===0?Wc(s):s;return u`
          <span key=${o} className="text-iron-400">/</span>
          <button
            key=${`${o}-button`}
            type="button"
            onClick=${()=>t(ed(o))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${l}
          </button>
        `})}
    </div>
  `}function q5(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function eS({path:e,entries:t,isLoading:a,filter:n,onOpen:r,onNavigate:s}){let i=R();if(a)return u`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;let o=(t||[]).filter(f=>!q5(f.path)),l=String(n||"").trim().toLowerCase(),c=l?o.filter(f=>f.name.toLowerCase().includes(l)):o,d=dl(c),m;return o.length?d.length?m=u`
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
        <${td} path=${e} onNavigate=${s} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">${m}</div>
    <//>
  `}var ad="/api/webchat/v2/fs",I5=1024*1024,K5=8*1024*1024;function tS(e){let t=String(e||"").split("/").filter(Boolean);return{mount:t.shift()||"",path:t.join("/")}}function H5(e,t){return t?`${e}/${t}`:e}function Q5(e){let t=String(e||"").toLowerCase();return t.startsWith("text/")||t==="application/json"||t==="application/javascript"||t==="application/xml"||t.endsWith("+json")||t.endsWith("+xml")}function V5(e){return String(e||"").toLowerCase().startsWith("image/")}function G5(e){let t=String(e||"").toLowerCase();return t.startsWith("audio/")||t.startsWith("video/")||t.startsWith("font/")||t==="application/pdf"||t==="application/zip"||t==="application/gzip"}function Y5(e){if(e.subarray(0,Math.min(e.length,8192)).indexOf(0)!==-1)return!0;try{return new TextDecoder("utf-8",{fatal:!0}).decode(e),!1}catch{return!0}}function J5(e,t){let a=new URL(`${ad}/content`,window.location.origin);return a.searchParams.set("mount",e),a.searchParams.set("path",t),a.pathname+a.search}async function X5(){return(await H(`${ad}/mounts`))?.mounts||[]}async function gi(e=""){if(!e)return{entries:(await X5()).map(o=>({name:Wc(o.mount),path:o.mount,is_dir:!0}))};let{mount:t,path:a}=tS(e),n=new URL(`${ad}/list`,window.location.origin);return n.searchParams.set("mount",t),a&&n.searchParams.set("path",a),{entries:((await H(n.pathname+n.search))?.entries||[]).map(i=>({name:i.name,path:H5(t,i.path),is_dir:i.kind==="directory"}))}}async function aS(e){let{mount:t,path:a}=tS(e);if(!t||!a)return{kind:"directory",path:e};let n=new URL(`${ad}/stat`,window.location.origin);n.searchParams.set("mount",t),n.searchParams.set("path",a);let s=(await H(n.pathname+n.search))?.stat||{},i=s.mime_type||"application/octet-stream",o=Number(s.size_bytes||0),l=J5(t,a),c={path:e,mime:i,size_bytes:o,download_path:l};if(s.kind&&s.kind!=="file")return{...c,kind:"directory"};if(V5(i)){if(o>K5)return{...c,kind:"binary"};let p=await bc(l);return{...c,kind:"image",image_data_url:p}}if(G5(i)||o>I5)return{...c,kind:"binary"};let d=await _a(l),m=new Uint8Array(await d.arrayBuffer());if(!Q5(i)&&Y5(m))return{...c,kind:"binary"};let f=new TextDecoder("utf-8").decode(m);return{...c,kind:"text",content:f}}function nS(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function Z5(e,t,a){let n=String(t||"").trim().toLowerCase(),r=(e||[]).filter(s=>!nS(s.path)).filter(s=>!n||s.name.toLowerCase().includes(n)?!0:s.is_dir&&a.has(s.path));return dl(r)}function rS({entry:e,depth:t,selectedPath:a,expandedPaths:n,filter:r,onToggleDirectory:s,onSelectFile:i}){let o=R(),l=n.has(e.path),c=K({queryKey:["workspace-list",e.path],queryFn:()=>gi(e.path),enabled:e.is_dir&&l});if(e.is_dir){let d=Z5(c.data?.entries,r,n);return u`
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
                  <${rS}
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
  `}function sS({entries:e,selectedPath:t,expandedPaths:a,filter:n,onToggleDirectory:r,onSelectFile:s,isLoading:i}){let o=R();if(i)return u`<div className="space-y-2 p-3">${[1,2,3,4].map(c=>u`<div key=${c} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let l=dl(e.filter(c=>!nS(c.path)));return l.length?u`
    <div className="space-y-1 p-2">
      ${l.map(c=>u`
        <${rS}
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
  `:u`<div className="px-4 py-8 text-sm text-iron-300">${o("workspace.noFiles")}</div>`}function iS({rootEntries:e,selectedPath:t,expandedPaths:a,filter:n,onFilterChange:r,isLoadingTree:s,onToggleDirectory:i,onSelectFile:o}){let l=R();return u`
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
        <${sS}
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
  `}function oS(e){return vi(e).pop()||"download"}function W5({path:e,file:t}){let a=R();return t.kind==="image"?u`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${t.image_data_url}
          alt=${oS(e)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `:t.kind==="text"?u`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${W2(e)?u`<${na} content=${t.content} className="max-w-4xl text-base leading-7" />`:u`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
      </div>
    `:u`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${a("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `}function lS({path:e,file:t,isLoading:a,onNavigate:n}){let r=R(),[s,i]=h.default.useState(!1),o=h.default.useCallback(async()=>{if(t?.download_path){i(!0);try{let c=await _a(t.download_path);Bc(c,oS(e))}catch{}finally{i(!1)}}},[t,e]);if(a)return u`
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
        <${td} path=${e} onNavigate=${n} />
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

      <${W5} path=${e} file=${t} />

      ${Sh(e)&&u`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${r("workspace.parent",{path:Sh(e)})}
        </div>
      `}
    <//>
  `}function uS(e){let t=R(),a=Y(),[n,r]=h.default.useState(new Set),[s,i]=h.default.useState(""),[o,l]=h.default.useState(null),c=K({queryKey:["workspace-list",""],queryFn:()=>gi("")}),d=K({queryKey:["workspace-file",e],queryFn:()=>aS(e),enabled:!!e}),m=e===""||d.data?.kind==="directory",f=K({queryKey:["workspace-list",e],queryFn:()=>gi(e),enabled:m});h.default.useEffect(()=>{l(null)},[e]);let p=h.default.useCallback(y=>a.fetchQuery({queryKey:["workspace-list",y],queryFn:()=>gi(y)}),[a]),x=h.default.useCallback(async y=>{let $=new Set(n);if($.has(y)){$.delete(y),r($);return}$.add(y),r($);try{await p(y)}catch(g){l({type:"error",message:g.message||t("workspace.unableOpenDirectory")})}},[n,p,t]);return{rootEntries:c.data?.entries||[],file:d.data||null,selectionIsDirectory:m,currentEntries:f.data?.entries||[],expandedPaths:n,filter:s,setFilter:i,result:o,clearResult:()=>l(null),isLoadingTree:c.isLoading,isLoadingFile:d.isLoading,isLoadingListing:f.isLoading,isFetching:c.isFetching||d.isFetching||f.isFetching,error:c.error||d.error||f.error||null,loadDirectory:p,toggleDirectory:x,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function Nh(){let e=R(),t=pe(),n=st()["*"]||Z2,r=uS(n),s=h.default.useCallback(i=>{t(ed(i))},[t]);return u`
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
          <${Ya}
            result=${r.result}
            onDismiss=${r.clearResult}
          />

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <${iS}
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
                  <${eS}
                    path=${n}
                    entries=${r.currentEntries}
                    isLoading=${r.isLoadingListing}
                    filter=${r.filter}
                    onOpen=${s}
                    onNavigate=${t}
                  />
                `:u`
                  <${lS}
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
  `}function cS(e){if(!e)return null;let t=e.metadata&&typeof e.metadata=="object"&&!Array.isArray(e.metadata)?e.metadata:{};return{id:e.project_id,name:e.name,description:e.description,goals:Array.isArray(t.goals)?t.goals:[],icon:e.icon||null,color:e.color||null,state:e.state,role:e.role,metadata:t,created_at:e.created_at,updated_at:e.updated_at,health:e.state==="archived"?"muted":"green"}}async function dS(){let t=((await zx({limit:200}))?.projects||[]).map(cS);return{attention:[],projects:t}}async function mS(e){if(!e)return null;let t=await qx({projectId:e});return cS(t?.project)}function fS(e){return Promise.resolve({missions:[],todo:!0})}function pS(e){return Promise.resolve({threads:[],todo:!0})}function hS(e){return Promise.resolve({widgets:[],todo:!0})}function vS(e){return Promise.resolve(null)}function gS(e){return Promise.resolve(null)}function yS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function bS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function xS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function $S(){let e=Y(),t=K({queryKey:["projects-overview"],queryFn:dS,refetchInterval:5e3}),a=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function wS(e){let t=Y(),a=!!e,n=K({queryKey:["project-detail",e],queryFn:()=>mS(e),enabled:a,refetchInterval:a?7e3:!1}),r=K({queryKey:["project-missions",e],queryFn:()=>fS(e),enabled:a,refetchInterval:a?5e3:!1}),s=K({queryKey:["project-threads",e],queryFn:()=>pS(e),enabled:a,refetchInterval:a?4e3:!1}),i=K({queryKey:["project-widgets",e],queryFn:()=>hS(e),enabled:a,refetchInterval:a?15e3:!1}),o=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function SS({projectId:e,missionId:t,threadId:a}){let n=Y(),[r,s]=h.default.useState(null),i=K({queryKey:["project-mission-detail",t],queryFn:()=>vS(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=K({queryKey:["project-thread-detail",a],queryFn:()=>gS(a),enabled:!!a,refetchInterval:a?4e3:!1}),l=h.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=Q({mutationFn:({targetMissionId:f})=>yS(f),onSuccess:f=>{s({type:"success",message:f?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to fire mission"})}}),d=Q({mutationFn:({targetMissionId:f})=>bS(f),onSuccess:()=>{s({type:"success",message:"Mission paused."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to pause mission"})}}),m=Q({mutationFn:({targetMissionId:f})=>xS(f),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),l()},onError:f=>{s({type:"error",message:f.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending}}function nd(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function rd(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function NS(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function _S(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function eD(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function kS(e){let t=eD(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function RS(e){let t=e?.projects||[],a=t.reduce((o,l)=>o+Number(l.cost_today_usd||0),0),n=t.reduce((o,l)=>o+Number(l.active_missions||0),0),r=t.reduce((o,l)=>o+Number(l.threads_today||0),0),s=t.reduce((o,l)=>o+Number(l.pending_gates||0),0),i=t.reduce((o,l)=>o+Number(l.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function ml(e,t){return`${e} ${t}${e===1?"":"s"}`}var tD={projects:"muted",attention:"warning",spend:"success"};function CS({overview:e}){let t=RS(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:rd(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return u`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>u`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${q} tone=${tD[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function aD(e){return e?.type==="failure"?"danger":"warning"}function nD(e){return e?.type==="failure"?"failure":"gate"}function ES({items:e,onOpenItem:t}){return e?.length?u`
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
              <${q} tone=${aD(a)} label=${nD(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function rD({project:e,onOpen:t,t:a}){return u`
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
        <${q} tone=${NS(e.health)} label=${e.health||"unknown"} />
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
            ${a("projects.card.threadsToday",{count:ml(e.threads_today||0,"thread")})}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${a("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">${ml(e.pending_gates||0,"gate")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${a("projects.card.failures24h",{count:ml(e.failures_24h||0,"failure")})}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>${a("projects.card.spendToday",{value:rd(e.cost_today_usd||0)})}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${nd(e.last_activity)}</div>
        </div>
        <${D}
          variant="secondary"
          onClick=${n=>{n.stopPropagation(),t(e.id)}}
        >${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function sD({project:e,onOpen:t,t:a}){return u`
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
            ${ml(e.threads_today||0,"thread")} today
          </div>
          <${D}
            variant="secondary"
            onClick=${n=>{n.stopPropagation(),t(e.id)}}
          >${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function TS({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=R(),l=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return!e.length&&t>0?u`
      <${$e}
        title=${o("projects.empty.noMatchTitle")}
        description=${o("projects.empty.noMatchDesc")}
      />
    `:e.length?u`
    <div className="space-y-5">
      ${l&&u`<${sD} project=${l} onOpen=${r} t=${o} />`}

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
            ${c.map(d=>u`<${rD} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
          </div>`:u`
            <${$e}
              title=${o("projects.scoped.onlyGeneralTitle")}
              description=${o("projects.scoped.onlyGeneralDesc")}
            >
              <${D} onClick=${s}>${o(i?"projects.preparingChat":"projects.startProject")}<//>
            <//>
          `}
    </div>
  `:u`
      <${$e}
        title=${o("projects.empty.noneTitle")}
        description=${o("projects.empty.noneDesc")}
      >
        <${D} onClick=${s}>${o("projects.createFromChat")}<//>
      <//>
    `}function AS({threads:e,selectedThreadId:t,onSelectThread:a,onNewConversation:n,isStartingConversation:r}){let s=[...e].sort((i,o)=>new Date(o.updated_at||o.created_at)-new Date(i.updated_at||i.created_at));return u`
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
        ${s.length?s.slice(0,18).map(i=>{let o=kS(i);return u`
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
                    <${q} tone=${_S(i.state)} label=${i.state} />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                    <span>${i.step_count||0} steps</span>
                    <span>${i.total_tokens||0} tokens</span>
                    <span>${nd(i.updated_at||i.created_at)}</span>
                  </div>
                </button>
              `}):u`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                No project threads yet. When an automation runs or scoped chat work happens inside this project, activity will appear here.
              </div>
            `}
      </div>
    <//>
  `}var iD="/workspace";function oD(e){let t=a=>a.kind==="directory"?0:1;return[...e].sort((a,n)=>t(a)-t(n)||a.name.localeCompare(n.name,void 0,{sensitivity:"base"}))}function lD(e){return e?String(e).replace(/^\/workspace\/?/,"").split("/").filter(Boolean):[]}function DS({threadId:e}){let t=R(),[a,n]=h.default.useState(void 0),[r,s]=h.default.useState(null),i=K({queryKey:["project-files",e||"",a||""],queryFn:()=>Ox({threadId:e,path:a}),enabled:!!e}),o=h.default.useMemo(()=>oD(i.data?.entries||[]),[i.data]),l=h.default.useCallback(async m=>{if(m.kind==="directory"){s(null),n(m.path);return}try{s(null);let f=await _a(yc({threadId:e,path:m.path})),p=URL.createObjectURL(f),x=document.createElement("a");x.href=p,x.download=m.name,document.body.appendChild(x),x.click(),x.remove(),URL.revokeObjectURL(p)}catch(f){s(f?.message||"Unable to download file")}},[e]),c=lD(a),d=u`
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
        ${c.map((m,f)=>{let p=`${iD}/${c.slice(0,f+1).join("/")}`;return u`
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
    `}function uD(e){return[...e||[]].sort((a,n)=>new Date(n.updated_at||n.created_at)-new Date(a.updated_at||a.created_at))[0]?.id||null}function MS({project:e,threads:t,selectedThreadId:a,onSelectThread:n,onNewConversation:r,isStartingConversation:s}){let i=uD(t);return u`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]">
      <div className="space-y-5">
        <div className="min-w-0">
          <h2 className="text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
          ${e.description?u`<p className="mt-1 text-sm leading-6 text-iron-300">${e.description}</p>`:null}
        </div>

        <${AS}
          threads=${t}
          selectedThreadId=${a}
          onSelectThread=${n}
          onNewConversation=${r}
          isStartingConversation=${s}
        />
      </div>

      <${DS} threadId=${i} />
    </div>
  `}function fl(){let e=R(),t=pe(),{threadsState:a}=ya(),{projectId:n=null,threadId:r=null}=st(),[s,i]=h.default.useState(""),[o,l]=h.default.useState(null),c=$S(),d=wS(n),m=SS({projectId:n,threadId:r}),f=h.default.useMemo(()=>{let _=s.trim().toLowerCase();return _?c.overview.projects.filter(A=>[A.name,A.description,...A.goals||[]].some(L=>String(L||"").toLowerCase().includes(_))):c.overview.projects},[c.overview.projects,s]),p=h.default.useMemo(()=>c.overview.projects.find(_=>_.id===n)||null,[c.overview.projects,n]),x=h.default.useCallback(()=>{c.invalidate(),d.invalidate()},[c,d]),y=h.default.useCallback(_=>{t(`/projects/${_}`)},[t]),$=h.default.useCallback(_=>{if(_.thread_id){t(`/projects/${_.project_id}/threads/${_.thread_id}`);return}t(`/projects/${_.project_id}`)},[t]),g=h.default.useCallback(async()=>{let _=null;l(null);try{_=await a.createThread()}catch(A){l({type:"error",message:A.message||e("projects.chatAutoFail")})}t("/chat",{state:{composerDraft:e("projects.creationDraft"),threadId:_}})},[t,a]),v=h.default.useCallback(_=>{t(`/projects/${n}/threads/${_}`)},[t,n]),b=h.default.useCallback(async()=>{l(null);try{let _=await a.createThread(n);t("/chat",{state:{threadId:_}}),d.invalidate()}catch(_){l({type:"error",message:_.message||e("projects.chatAutoFail")})}},[t,a,n,d,e]),w=h.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),S=u`
    ${n&&u`<${D} variant="ghost" onClick=${()=>t("/projects")}>${e("projects.allProjects")}<//>`}
  `,C=null;return n?d.isLoading?C=u`
        <div className="space-y-4">
          ${[1,2,3].map(_=>u`<div key=${_} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `:d.error||!d.project&&!p?C=u`
        <${$e}
          title=${e("projects.unavailable")}
          description=${d.error?.message||e("projects.unavailableDesc")}
        >
          <${D} variant="secondary" onClick=${()=>t("/projects")}>${e("projects.returnToProjects")}<//>
        <//>
      `:C=u`
        <${MS}
          project=${d.project||p}
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
          <${TS}
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
          <${Ya} result=${o} onDismiss=${()=>l(null)} />
          <${Ya} result=${m.actionResult} onDismiss=${m.clearActionResult} />
          ${!n&&u`
            <${CS} overview=${c.overview} />
            <${ES} items=${c.overview.attention} onOpenItem=${$} />
          `}
          ${C}
        </div>
      </div>
    </div>
  `}function pl(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not scheduled"}function hl(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function OS(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function LS(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);return r!==0?r:new Date(n.updated_at||0).getTime()-new Date(a.updated_at||0).getTime()})}function sd({label:e,value:t}){return u`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function cD({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=R();return e.status==="Active"?u`
      <${D} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.fireNow")}<//>
      <${D} variant="secondary" onClick=${()=>n(e.id)} disabled=${t}>${s("missions.action.pause")}<//>
    `:e.status==="Paused"?u`
      <${D} onClick=${()=>r(e.id)} disabled=${t}>${s("missions.action.resume")}<//>
      <${D} variant="secondary" onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runOnce")}<//>
    `:u`<${D} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runAgain")}<//>`}function PS({mission:e,isLoading:t,error:a,isBusy:n,onFire:r,onPause:s,onResume:i,onOpenProject:o,onOpenThread:l}){let c=R();return t?u`
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
          <${q} tone=${hl(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${sd} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${sd} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${sd} label=${c("missions.meta.nextFire")} value=${pl(e.next_fire_at)} />
          <${sd} label=${c("missions.meta.updated")} value=${pl(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${cD}
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
          <${na} content=${e.goal||c("missions.noGoal")} />
        </div>
      <//>

      ${e.current_focus&&u`
        <${I} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${na} content=${e.current_focus} />
          </div>
        <//>
      `}

      ${e.success_criteria&&u`
        <${I} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${na} content=${e.success_criteria} />
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
                  <${q} tone=${hl(d.state==="Running"?"Active":d.state==="Failed"?"Failed":"Completed")} label=${d.state} />
                </div>
              </button>
            `)}
          </div>
        <//>
      `:null}
    </div>
  `}function dD(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function US({value:e,onChange:t,children:a,label:n}){return u`
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
  `}function mD({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=R(),s=t===e.id;return u`
    <div
      className=${["w-full rounded-xl border p-4 text-left",s?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80"].join(" ")}
    >
      <button type="button" onClick=${()=>a(e.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${e.name}</div>
              <${q} tone=${hl(e.status)} label=${e.status} />
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
          ${r("missions.updated",{value:pl(e.updated_at)})}
        </span>
        <${D}
          variant="ghost"
          onClick=${i=>{i.stopPropagation(),n(e.project.id)}}
        >
          ${e.project.name}
        <//>
      </div>
    </div>
  `}function _h({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:l,projectOptions:c,onSelectMission:d,onOpenProject:m}){let f=R(),p=dD(f);return u`
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
        <${US} value=${s} onChange=${i} label=${f("missions.filter.status")}>
          ${p.map(x=>u`<option key=${x.value} value=${x.value}>${x.label}<//>`)}
        <//>
        <${US} value=${o} onChange=${l} label=${f("missions.filter.project")}>
          <option value="all">${f("missions.filter.allProjects")}</option>
          ${c.map(x=>u`<option key=${x.id} value=${x.id}>${x.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(x=>u`
              <${mD}
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
  `}function fD(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function jS({summary:e}){let t=R(),a=fD(t);return u`
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
  `}function FS(){return Promise.resolve({projects:[],todo:!0})}function BS({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function zS(e){return Promise.resolve(null)}function qS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function IS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function KS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function HS(e){let t=K({queryKey:["mission-detail",e],queryFn:()=>zS(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function pD(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function QS(){let e=Y(),[t,a]=h.default.useState(null),n=K({queryKey:["projects-overview"],queryFn:FS,refetchInterval:7e3}),r=n.data?.projects||[],s=Md({queries:r.map(f=>({queryKey:["missions","project",f.id],queryFn:()=>BS({projectId:f.id}),refetchInterval:5e3,select:p=>p?.missions||[]}))}),i=s.flatMap((f,p)=>{let x=r[p];return(f.data||[]).map(y=>pD(y,x))}),o=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),l=(f,p)=>({mutationFn:({missionId:x})=>f(x),onSuccess:()=>{a({type:"success",message:p}),o()},onError:x=>{a({type:"error",message:x.message||"Unable to update mission"})}}),c=Q(l(qS,"Mission fired and a run was queued.")),d=Q(l(IS,"Mission paused.")),m=Q(l(KS,"Mission resumed."));return{projects:r,missions:i,summary:OS(i),isLoading:n.isLoading||s.some(f=>f.isLoading),isRefreshing:n.isFetching||s.some(f=>f.isFetching),error:n.error||s.find(f=>f.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:m.mutateAsync,isBusy:c.isPending||d.isPending||m.isPending,invalidate:o}}function kh(){let e=R(),t=pe(),{missionId:a=null}=st(),[n,r]=h.default.useState(""),[s,i]=h.default.useState("all"),[o,l]=h.default.useState("all"),c=QS(),d=HS(a),m=h.default.useMemo(()=>{let g=n.trim().toLowerCase();return LS(c.missions).filter(v=>{let b=!g||[v.name,v.goal,v.project?.name].some(C=>String(C||"").toLowerCase().includes(g)),w=s==="all"||v.status===s,S=o==="all"||v.project?.id===o;return b&&w&&S})},[c.missions,o,n,s]),f=h.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),p=d.mission?{...f,...d.mission,project:f?.project||null}:f,x=h.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),y=h.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),$=a?u`
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <${_h}
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
          <${PS}
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
      `:u`
        <${_h}
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

          <${Ya}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${jS} summary=${c.summary} />

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
  `}var VS=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],hD=new Set(["pending","in_progress"]),GS=new Set(["failed","interrupted","stuck","cancelled"]);function ir(e){return e?String(e).replace(/_/g," "):"unknown"}function yi(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":GS.has(e)?"danger":"muted":"muted"}function vD(e){return hD.has(e)}function id(e){return vD(e?.state)}function YS(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":GS.has(e.state):!1}function Hr(e,t=8){return e?String(e).slice(0,t):"unknown"}function ra(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function JS(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function Rh(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${ra(e.started_at)}`:null].filter(Boolean).join(" / ")}var gD=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function XS(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function yD({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?u`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${t==="tool_use"?a.tool_name||"Tool call":a.tool_name||"Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${XS(t==="tool_use"?a.input:a.output||a.error||a)}</pre>
      </details>
    `:t==="message"?u`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a.role||"assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${a.content||""}</div>
      </div>
    `:u`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t.replace(/_/g," ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${a.message||a.status||XS(a)}</div>
    </div>
  `}function ZS({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=R(),[s,i]=h.default.useState("all"),[o,l]=h.default.useState(""),[c,d]=h.default.useState(!0),m=h.default.useRef(null),f=h.default.useMemo(()=>s==="all"?t:t.filter(x=>x.event_type===s),[t,s]);h.default.useEffect(()=>{c&&m.current&&(m.current.scrollTop=m.current.scrollHeight)},[c,f.length]);let p=h.default.useCallback(async(x=!1)=>{let y=o.trim();if(!(!y&&!x))try{await a({content:y||"(done)",done:x}),l("")}catch{}},[o,a]);return u`
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
            ${gD.map(x=>u`<option key=${x.value} value=${x.value}>${x.label}</option>`)}
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
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${ra(x.created_at)}</div>
                <${yD} event=${x} />
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
            onKeyDown=${x=>{x.key==="Enter"&&!x.shiftKey&&(x.preventDefault(),p(!1))}}
            placeholder=${r("job.followupPlaceholder")}
            className="h-11 rounded-md border border-white/10 bg-iron-950/90 px-3 text-sm text-white outline-none focus:border-signal/45"
          />
          <${D} variant="secondary" disabled=${n} onClick=${()=>p(!0)}>${r("common.done")}<//>
          <${D} variant="primary" disabled=${n} onClick=${()=>p(!1)}>${r("common.send")}<//>
        </div>
      `}
    <//>
  `}function WS({job:e,activeTab:t,onTabChange:a,onBack:n,onCancel:r,onRestart:s,isBusy:i,children:o}){return u`
    <div className="space-y-5">
      <${I} className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick=${n} className="text-sm text-signal hover:text-white">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-semibold tracking-tight text-white">${e.title||"Untitled job"}</h2>
              <${q} tone=${yi(e.state)} label=${ir(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${Hr(e.id)}</span>
              <span>created ${ra(e.created_at)}</span>
              ${Rh(e)&&u`<span>${Rh(e)}</span>`}
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
            ${id(e)&&u`
              <${D} variant="secondary" disabled=${i} onClick=${()=>r(e.id)}>Cancel<//>
            `}
            ${YS(e)&&u`
              <${D} variant="primary" disabled=${i} onClick=${()=>s(e.id)}>Restart<//>
            `}
          </div>
        </div>
      <//>

      <div className="flex flex-wrap gap-2">
        ${VS.map(l=>u`
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
  `}function eN({nodes:e,depth:t=0,selectedPath:a,expandingPath:n,onToggleDirectory:r,onSelectPath:s}){return u`
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
        ${i.isDir&&i.expanded&&i.children?.length?u`<${eN}
              nodes=${i.children}
              depth=${t+1}
              selectedPath=${a}
              expandingPath=${n}
              onToggleDirectory=${r}
              onSelectPath=${s}
            />`:null}
      </div>
    `)}
  `}function tN({canBrowse:e,tree:t,selectedPath:a,selectedFile:n,fileError:r,isLoadingTree:s,isLoadingFile:i,expandingPath:o,treeError:l,onToggleDirectory:c,onSelectPath:d}){return e?u`
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <${I} className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          ${l&&u`<div className="mx-2 mb-3 rounded-md border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${l}</div>`}
          ${s?u`<div className="space-y-2 px-2">${[1,2,3,4].map(m=>u`<div key=${m} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?u`
                  <${eN}
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
    `}function bi({label:e,value:t}){return u`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function aN({job:e}){let t=(e.transitions||[]).map(a=>({title:`${ir(a.from)} -> ${ir(a.to)}`,description:[ra(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return u`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${I} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${q} tone=${yi(e.state)} label=${ir(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${bi} label="Created" value=${ra(e.created_at)} />
          <${bi} label="Started" value=${ra(e.started_at)} />
          <${bi} label="Completed" value=${ra(e.completed_at)} />
          <${bi} label="Duration" value=${JS(e.elapsed_secs)} />
          <${bi} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${bi} label="Mode" value=${e.job_mode||"Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${I} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${e.description?u`<${na} content=${e.description} className="mt-4 text-sm leading-7 text-iron-200" />`:u`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
        <//>

        ${t.length?u`
              <${I} className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Transitions</div>
                <h3 className="mt-2 text-xl font-semibold text-white">State timeline</h3>
                <div className="mt-3">
                  <${J2} items=${t} />
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
  `}function nN({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:l,isBusy:c,isRefreshing:d}){let m=R(),f=[{value:"all",label:m("jobs.list.filter.all")},{value:"pending",label:m("jobs.list.filter.pending")},{value:"in_progress",label:m("jobs.list.filter.inProgress")},{value:"completed",label:m("jobs.list.filter.completed")},{value:"failed",label:m("jobs.list.filter.failed")},{value:"stuck",label:m("jobs.list.filter.stuck")}];if(!e.length){let p=!!n.trim()||s!=="all";return u`
      <${$e}
        title=${m(t&&p?"jobs.list.empty.noMatchTitle":"jobs.list.empty.noJobsTitle")}
        description=${m(t&&p?"jobs.list.empty.noMatchDesc":"jobs.list.empty.noJobsDesc")}
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
            onInput=${p=>r(p.target.value)}
            placeholder=${m("jobs.list.searchPlaceholder")}
            className="h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          />
          <select
            value=${s}
            onChange=${p=>i(p.target.value)}
            className="v2-select h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          >
            ${f.map(p=>u`<option key=${p.value} value=${p.value}>${p.label}</option>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(p=>u`
          <article
            key=${p.id}
            className=${["group flex flex-col gap-4 rounded-[18px] border p-5",a===p.id?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
          >
            <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
              <button onClick=${()=>o(p.id)} className="min-w-0 text-left">
                <div className="flex flex-wrap items-center gap-2">
                  <h3 className="truncate text-lg font-semibold text-iron-100">${p.title||m("jobs.list.untitled")}</h3>
                  <${q} tone=${yi(p.state)} label=${ir(p.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${Hr(p.id)}</span>
                  <span>${m("jobs.list.created",{value:ra(p.created_at)})}</span>
                  ${p.started_at&&u`<span>${m("jobs.list.started",{value:ra(p.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${id(p)&&u`
                  <${D}
                    variant="secondary"
                    className="h-9 px-3 text-xs"
                    disabled=${c}
                    onClick=${()=>l(p.id)}
                  >
                    ${m("jobs.action.cancel")}
                  <//>
                `}
                <${D} variant="ghost" className="h-9 px-3 text-xs" onClick=${()=>o(p.id)}>${m("jobs.action.open")}<//>
              </div>
            </div>
          </article>
        `)}
      </div>
    </div>
  `}var bD=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function rN({summary:e}){return u`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${bD.map(t=>u`
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
  `}function sN(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function iN(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function oN(e){return Promise.resolve(null)}function lN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function uN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function cN(e){return Promise.resolve({events:[],todo:!0})}function dN(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function Ch(e,t=""){return Promise.resolve({entries:[],todo:!0})}function mN(e,t){return Promise.resolve({content:"",todo:!0})}function fN(e){let t=Y(),[a,n]=h.default.useState(null),r=K({queryKey:["job-detail",e],queryFn:()=>oN(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=K({queryKey:["job-events",e],queryFn:()=>cN(e),enabled:!!e,refetchInterval:e?2500:!1}),i=Q({mutationFn:({content:o,done:l})=>dN(e,{content:o,done:l}),onSuccess:(o,{done:l})=>{n({type:"success",message:l?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return h.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function pN(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function hN(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=hN(a.children,t);if(n)return n}}return null}function od(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:od(n.children,t,a)}:n)}function vN(e){let[t,a]=h.default.useState([]),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,l]=h.default.useState(""),c=!!(e?.project_dir&&e?.id),d=K({queryKey:["job-files-root",e?.id],queryFn:()=>Ch(e.id,""),enabled:c}),m=K({queryKey:["job-file",e?.id,n],queryFn:()=>mN(e.id,n),enabled:!!(c&&n)});h.default.useEffect(()=>{a([]),r(""),i(""),l("")},[e?.id]),h.default.useEffect(()=>{d.data?.entries?(a(pN(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let f=h.default.useCallback(async p=>{let x=hN(t,p);if(!(!x||!e?.id)){if(x.expanded){a(y=>od(y,p,$=>({...$,expanded:!1})));return}if(x.loaded){a(y=>od(y,p,$=>({...$,expanded:!0})));return}l(p);try{let y=await Ch(e.id,p);a($=>od($,p,g=>({...g,expanded:!0,loaded:!0,children:pN(y.entries)}))),i("")}catch(y){i(y.message||"Unable to open folder")}finally{l("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:m.data||null,fileError:m.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:m.isLoading||m.isFetching,expandingPath:o,treeError:s,toggleDirectory:f}}function gN(){let e=Y(),[t,a]=h.default.useState(null),n=K({queryKey:["jobs-summary"],queryFn:iN,refetchInterval:5e3}),r=K({queryKey:["jobs"],queryFn:sN,refetchInterval:5e3}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=Q({mutationFn:({jobId:l})=>lN(l),onSuccess:(l,{jobId:c})=>{a({type:"success",message:`Job ${Hr(c)} cancelled`}),s()},onError:l=>{a({type:"error",message:l.message||"Unable to cancel job"})}}),o=Q({mutationFn:({jobId:l})=>uN(l),onSuccess:l=>{a({type:"success",message:`Restart queued as ${Hr(l?.new_job_id)}`}),s()},onError:l=>{a({type:"error",message:l.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function yN({result:e,onDismiss:t}){let a=R();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return u`
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
  `}function Eh(){let e=R(),t=pe(),{jobId:a=null}=st(),[n,r]=h.default.useState(""),[s,i]=h.default.useState("all"),[o,l]=h.default.useState(a?"activity":"overview"),c=gN(),d=fN(a),m=vN(d.job);h.default.useEffect(()=>{l(a?"activity":"overview")},[a]);let f=h.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(b=>{let w=!v||b.title.toLowerCase().includes(v)||b.id.toLowerCase().includes(v),S=s==="all"||b.state===s;return w&&S})},[c.jobs,n,s]),p=h.default.useCallback(v=>t(`/jobs/${v}`),[t]),x=h.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),y=h.default.useCallback(async v=>{try{let b=await c.restartJob({jobId:v});b?.new_job_id&&t(`/jobs/${b.new_job_id}`)}catch{}},[c,t]),$=u`
    ${a&&u`<${D} variant="ghost" onClick=${()=>t("/jobs")}
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
          <${D} variant="secondary" onClick=${()=>t("/jobs")}
            >${e("jobs.returnToJobs")}<//
          >
        <//>
      `;else{let v={overview:u`<${aN} job=${d.job} />`,activity:u`
          <${ZS}
            job=${d.job}
            events=${d.events}
            onSendPrompt=${d.sendPrompt}
            isSendingPrompt=${d.isSendingPrompt}
          />
        `,files:u`
          <${tN}
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
        <${WS}
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
          <${nN}
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
          <${yN}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${yN}
            result=${d.promptResult}
            onDismiss=${d.clearPromptResult}
          />
          <${rN} summary=${c.summary} />
          ${g}
        </div>
      </div>
    </div>
  `}function or(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not scheduled"}function ld(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function ud(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function bN(e=[]){return[...e].sort((t,a)=>t.enabled!==a.enabled?t.enabled?-1:1:new Date(a.next_fire_at||a.last_run_at||0).getTime()-new Date(t.next_fire_at||t.last_run_at||0).getTime())}function xN(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function xD(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function $N({runs:e}){return e?.length?u`
    <div className="space-y-3">
      ${e.map(t=>u`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${q} tone=${xD(t.status)} label=${t.status} />
              <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                ${or(t.started_at)}
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
    `}function lr({label:e,value:t}){return u`
    <div className="rounded-xl border border-iron-700 bg-iron-950/50 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div className="mt-2 min-w-0 break-words text-sm text-iron-100">
        ${t||"\u2014"}
      </div>
    </div>
  `}function wN({title:e,value:t}){return u`
    <div>
      <h3 className="text-sm font-semibold text-iron-100">${e}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >${JSON.stringify(t||{},null,2)}</pre>
    </div>
  `}function SN({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=pe(),l=R();return t?u`
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
              tone=${ld(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${q}
              tone=${ud(e.verification_status)}
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
        <${lr} label="Trigger" value=${e.trigger_summary||e.trigger_type} />
        <${lr} label="Action" value=${xN(e.action)} />
        <${lr} label="Next fire" value=${or(e.next_fire_at)} />
        <${lr} label="Last run" value=${or(e.last_run_at)} />
        <${lr} label="Run count" value=${e.run_count} />
        <${lr} label="Failures" value=${e.consecutive_failures} />
        <${lr} label="Created" value=${or(e.created_at)} />
        <${lr} label="Routine ID" value=${e.id} />
      </div>

      ${e.conversation_id&&u`
        <div className="mt-5">
          <${D} variant="secondary" onClick=${()=>o(`/chat/${e.conversation_id}`)}>
            Open routine thread
          <//>
        </div>
      `}

      <div className="mt-6 grid gap-6 xl:grid-cols-2">
        <${wN} title=${l("routine.triggerPayload")} value=${e.trigger} />
        <${wN} title=${l("routine.actionPayload")} value=${e.action} />
      </div>

      <div className="mt-6">
        <h3 className="mb-3 text-sm font-semibold text-iron-100">Recent runs</h3>
        <${$N} runs=${e.recent_runs} />
      </div>
    <//>
  `}function NN({routine:e,selectedRoutineId:t,onSelectRoutine:a,onTriggerRoutine:n,onToggleRoutine:r,isBusy:s}){let i=t===e.id;return u`
    <article
      className=${["group flex flex-col gap-4 rounded-[18px] border p-5",i?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick=${()=>a(e.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">${e.name}</h3>
            <${q}
              tone=${ld(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${q}
              tone=${ud(e.verification_status)}
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
  `}var $D=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function Th({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:l,onToggleRoutine:c,isBusy:d,isRefreshing:m}){let f=R();if(!e.length){let p=!!n.trim()||s!=="all";return u`
      <${$e}
        title=${t&&p?"No routines match":"No routines yet"}
        description=${t&&p?"Adjust the search or status filter to find a saved routine.":"Routines created from chat will appear here after they are saved."}
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
            onInput=${p=>r(p.target.value)}
            placeholder="Search routine name, trigger, or action"
            className="h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          />
          <select
            value=${s}
            onChange=${p=>i(p.target.value)}
            className="v2-select h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          >
            ${$D.map(p=>u`<option key=${p.value} value=${p.value}>${p.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(p=>u`
            <${NN}
              key=${p.id}
              routine=${p}
              selectedRoutineId=${a}
              onSelectRoutine=${o}
              onTriggerRoutine=${l}
              onToggleRoutine=${c}
              isBusy=${d}
            />
          `)}
      </div>
    </div>
  `}var wD=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function _N({summary:e}){return u`
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
  `}function kN(e){let[t,a]=h.default.useState(""),[n,r]=h.default.useState("all");return{filteredRoutines:h.default.useMemo(()=>{let i=t.trim().toLowerCase();return bN(e).filter(o=>{let l=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||l.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function RN(){return Promise.resolve({routines:[],todo:!0})}function CN(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function EN(e){return Promise.resolve(null)}function cd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function dd(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function TN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function AN(e){let t=Y(),[a,n]=h.default.useState(null),r=K({queryKey:["routine-detail",e],queryFn:()=>EN(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:m=>{n({type:"error",message:m.message||"Unable to update routine"})}}),o=Q(i(cd,"Routine run queued.")),l=Q(i(dd,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:l.mutateAsync,isBusy:o.isPending||l.isPending}}function DN(){let e=Y(),[t,a]=h.default.useState(null),n=K({queryKey:["routines-summary"],queryFn:CN,refetchInterval:5e3}),r=K({queryKey:["routines"],queryFn:RN,refetchInterval:5e3}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,m)=>({mutationFn:({routineId:f})=>d(f),onSuccess:()=>{a({type:"success",message:m}),s()},onError:f=>{a({type:"error",message:f.message||"Unable to update routine"})}}),o=Q(i(cd,"Routine run queued.")),l=Q(i(dd,"Routine status updated.")),c=Q(i(TN,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:l.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||l.isPending||c.isPending,invalidate:s}}function Ah(){let e=pe(),{routineId:t=null}=st(),a=DN(),n=AN(t),r=kN(a.routines),s=h.default.useCallback(async(l,c)=>{try{await l({routineId:c})}catch{}},[]),i=h.default.useCallback(async(l,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:l}),e("/routines")}catch{}},[e,a]),o=t?u`
        <div className="grid gap-5 xl:grid-cols-[minmax(0,0.9fr)_minmax(440px,1.1fr)]">
          <${Th}
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
          <${SN}
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
        <${Th}
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

          <${Ya}
            result=${a.actionResult}
            onDismiss=${a.clearActionResult}
          />
          <${Ya}
            result=${n.actionResult}
            onDismiss=${n.clearActionResult}
          />
          <${_N} summary=${a.summary} />

          ${a.isLoading?u`
                <div className="space-y-4">
                  ${[1,2,3].map(l=>u`<div key=${l} className="v2-skeleton h-32 rounded-xl" />`)}
                </div>
              `:o}
        </div>
      </div>
    </div>
  `}function SD(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function ND(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function MN({deliveryState:e}){let t=R(),a=e.currentTarget?.target_id||"",[n,r]=h.default.useState(a),[s,i]=h.default.useState(!1),o=h.default.useRef(null);h.default.useEffect(()=>{r(a)},[a]),h.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let l=n!==a,c=e.isLoading||e.isSaving,d=l&&!c,m=!!a&&!c,f=e.finalReplyTargets.length>0,p=e.targets.some(L=>L?.capabilities?.final_replies&&L?.target?.status==="unavailable"),x=f||p,y=L=>(o.current&&clearTimeout(o.current),i(!1),L.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),$=()=>{d&&y(e.saveFinalReplyTarget(n||null))},g=()=>{m&&(r(""),y(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),b=e.currentStatus,w=b==="available"?"success":b==="unavailable"?"warning":"muted",S=t(b==="available"?"automations.delivery.pill.ready":b==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),C=!!e.currentTarget,_=t(C?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),A=ND(t("automations.delivery.footnote"),{command:u`<code
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
              <${q} tone=${w} label=${S} />
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
                    tone=${SD(z)}
                    label=${t(z==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.ready")}
                    className="self-center shrink-0"
                  />
                </label>
              `})}

            <!-- Unpaired notice rows (targets present but status=unavailable
                 and NOT already shown above because they lack final_replies) -->
            ${p&&u`
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
  `}var _D=["schedule","once"],LN={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},PN={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},UN={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function sa(e){return typeof e=="function"?e:t=>t}var Mh=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:Nn},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:BD},{value:"completed",labelKey:"automations.filter.completed",predicate:zD}];function jN(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>_D.includes(r?.source?.type)).map(r=>LD(r,t,a)).sort(FD)}function FN(e,t){let a=Mh.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function BN(e){let t=e.filter(i=>i.state!=="completed"),a=t.filter(i=>Nn(i)).length,n=t.filter(i=>i.has_running_run).length,r=t.filter(i=>i.has_failed_runs).length,s=t.filter(i=>Nn(i)&&Dh(i)!=null).sort((i,o)=>(i.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(o.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:t.length,active:a,running:n,failures:r,nextRun:s?.next_run_label||null}}function kD(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=HD(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:l,month:c,dayOfWeek:d,year:m}=s,f=t&&typeof t=="string"?t:null,p=f?` (${f})`:"",x=m==="*"&&l==="*"&&c==="*"&&d==="*";if(x&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=QD(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(ur(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let y=qD(o,i,n);if(!y)return r("automations.schedule.custom");if(x)return r("automations.schedule.everyDayAt",{time:y})+p;let $=VD(d);if(m==="*"&&l==="*"&&c==="*"&&$==="1-5")return r("automations.schedule.weekdaysAt",{time:y})+p;if(m==="*"&&l==="*"&&c==="*"&&ur($,0,7)){let g=ID(Number($)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:y})+p}if(m==="*"&&ur(l,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(l),time:y})+p;if(ur(l,1,31)&&ur(c,1,12)&&d==="*"&&(m==="*"||ur(m,1970,9999))){let g=KD(Number(c),Number(l),m==="*"?null:Number(m),n);return r("automations.schedule.dateAt",{date:g,time:y})+p}return r("automations.schedule.custom")}function Qr(e,t="Unknown",a,n){if(!e)return t;let r=new Date(e);if(Number.isNaN(r.getTime()))return t;let s=n&&typeof n=="string"?{timeZone:n}:{};try{return r.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...s})}catch{return r.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function zN(e,t){let a=LN[e]?.labelKey||"automations.state.unknown";return sa(t)(a)}function qN(e){return LN[e]?.tone||"muted"}function RD(e,t){return Nn(e)&&e?.has_running_run?sa(t)("automations.status.running"):Nn(e)&&e?.has_failed_runs?sa(t)("automations.status.needsReview"):zN(e?.state,t)}function CD(e){return Nn(e)&&e?.has_running_run?"info":Nn(e)&&e?.has_failed_runs?"danger":qN(e?.state)}function ED(e,t){let a=PN[e]?.labelKey||"automations.lastStatus.none";return sa(t)(a)}function TD(e){return PN[e]?.tone||"muted"}function AD(e,t){let a=UN[md(e)]?.labelKey||"automations.runStatus.unknown";return sa(t)(a)}function DD(e){return UN[md(e)]?.tone||"muted"}function MD(e,t,a,n){if(!e)return sa(a)("automations.schedule.custom");let r=Qr(e,null,n,t);if(!r)return sa(a)("automations.schedule.custom");let s=t&&typeof t=="string"?` (${t})`:"";return sa(a)("automations.schedule.onceAt",{datetime:r})+s}function OD(e,t,a){return e?.type==="once"?MD(e.at,e.timezone,t,a):e?.type==="schedule"?kD(e.cron,e.timezone||"UTC",t,a):sa(t)("automations.schedule.custom")}function LD(e,t,a){let n=sa(t),r=PD(e.recent_runs,t,a),s=r[0]||null,i=r.find(m=>m.status==="running")||null,o=r.find(m=>m.status==="ok"||m.status==="error")||null,l=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null,d={...e,recent_runs:r,has_running_run:r.some(m=>m.status==="running"),has_failed_runs:r.some(m=>m.status==="error")};return{...d,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:OD(e.source,t,a),state_label:zN(e.state,t),state_tone:qN(e.state),primary_status_label:RD(d,t),primary_status_tone:CD(d),next_run_timestamp:Oh(e.next_run_at),next_run_label:Qr(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:Qr(c,n("automations.date.noRuns"),a),last_status_label:ED(l,t),last_status_tone:TD(l),created_label:Qr(e.created_at,n("automations.date.unknown"),a),latest_run:s,current_run:i,success_rate_label:jD(r,t)}}function PD(e,t,a){let n=sa(t);return Array.isArray(e)?e.map(r=>{let s=md(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=Oh(i);return{...r,status:s,status_label:AD(s,t),status_tone:DD(s),timestamp:o,timestamp_source:i,fired_label:Qr(i,n("automations.date.unscheduled"),a),submitted_label:Qr(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:Qr(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function md(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function IN(e){let t=Array.isArray(e)?e:[],a={total:t.length,ok:0,error:0,running:0,unknown:0};for(let n of t){let r=md(n?.status);Object.prototype.hasOwnProperty.call(a,r)?a[r]+=1:a.unknown+=1}return a}function UD(e){let t=IN(e);return[{key:"ok",tone:"text-emerald-300",count:t.ok},{key:"error",tone:"text-red-300",count:t.error},{key:"running",tone:"text-sky-300",count:t.running},{key:"unknown",tone:"text-iron-400",count:t.unknown}].filter(a=>a.count>0)}function KN(e,t){let a=sa(t),n=IN(e),r=UD(e).map(s=>({...s,text:a(`automations.runs.${s.key}`,{count:s.count})}));return{total:n.total,totalText:a("automations.runs.total",{count:n.total}),chips:r}}function jD(e,t){let a=sa(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function FD(e,t){let a=Nn(e),n=Nn(t);return a!==n?a?-1:1:(Dh(e)??Number.MAX_SAFE_INTEGER)-(Dh(t)??Number.MAX_SAFE_INTEGER)}function Oh(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function Nn(e){return e?.state==="active"||e?.state==="scheduled"}function BD(e){return["paused","disabled","inactive"].includes(e?.state)}function zD(e){return e?.state==="completed"}function Dh(e){return e?.next_run_timestamp??Oh(e?.next_run_at)}function Lh(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function qD(e,t,a){return!ur(e,0,23)||!ur(t,0,59)?null:Lh(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function ID(e,t){return Lh(t,{weekday:"long"},new Date(2001,0,7+e))}function KD(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return Lh(n,r,new Date(a??2e3,e-1,t))}function HD(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&ON(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&ON(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function ON(e){return/^0+$/.test(e)}function ur(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function QD(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function VD(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}function GD(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function HN({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let l=o.toString(),c=`/logs${l?`?${l}`:""}`;return i?`/v2${c}`:c}function QN(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(GD),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}var YD=8;function Ph(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function fd({runs:e=[]}){let t=R(),a=Array.isArray(e)?e:[],n=a.slice(0,YD);if(!n.length)return u`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;let r=a.length-n.length,s=`+${Math.min(r,999)}`;return u`
    <div
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
    >
      ${n.map(i=>u`
        <span
          key=${Ph(i)}
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
  `}function pd({runs:e=[],className:t=""}){let a=R(),n=KN(e,a);return n.total?u`
    <div className=${V("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]",t)}>
      <span className="text-iron-300">${n.totalText}</span>
      ${n.chips.map(r=>u`<span key=${r.key} className=${r.tone}>${r.text}</span>`)}
    </div>
  `:u`<span className=${V("text-[11px] text-iron-400",t)}>
      ${a("automations.table.noRuns")}
    </span>`}function VN({run:e,onOpenRun:t,onOpenLogs:a}){let n=R(),r=!!e.chat_path,s=HN({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return u`
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
  `}function hd({label:e,value:t,tone:a}){return u`
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
  `}function GN({automation:e,isMutating:t=!1,onPauseAutomation:a,onResumeAutomation:n,onDeleteAutomation:r}){let s=R(),i=pe();if(!e)return u`
      <${I} className="p-4 sm:p-5">
        <${$e}
          boxed=${!1}
          title=${s("automations.detail.emptyTitle")}
          description=${s("automations.detail.emptyDescription")}
        />
      <//>
    `;let o=e.current_run,l=e.state==="paused",c=e.state==="active"||e.state==="scheduled",m=`${s(l?"missions.action.resume":"missions.action.pause")}: ${e.display_name}`,f=()=>{if(l){n?.(e.automation_id);return}c&&a?.(e.automation_id)},p=`${s("common.delete")}: ${e.display_name}`,x=()=>{window.confirm(p)&&r?.(e.automation_id)};return u`
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
          <${hd} label=${s("automations.detail.schedule")} value=${e.schedule_label} />
          <${hd}
            label=${s("automations.detail.successRate")}
            value=${e.success_rate_label}
            tone=${e.has_failed_runs?"danger":"success"}
          />
          <${hd} label=${s("automations.detail.lastCompleted")} value=${e.last_run_label} />
          <${hd}
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
              <${fd} runs=${e.recent_runs} />
              <${pd} runs=${e.recent_runs} />
            </div>
          </div>

          ${e.recent_runs.length?u`
                <div>
                  ${e.recent_runs.map(y=>u`
                    <${VN}
                      key=${Ph(y)}
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
  `}var JD=["automations.empty.example1","automations.empty.example2","automations.empty.example3"];function XD({promptKey:e}){let t=R(),a=t(e),[n,r]=h.default.useState(!1),s=h.default.useRef(null);return h.default.useEffect(()=>()=>clearTimeout(s.current),[]),u`
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
  `}function YN(){let e=R(),t=pe();return u`
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
            ${JD.map(a=>u`<${XD} key=${a} promptKey=${a} />`)}
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
  `}function JN({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,isMutating:s,selectedAutomationId:i,onSelectAutomation:o,onPauseAutomation:l,onResumeAutomation:c,onDeleteAutomation:d}){let m=R(),f=FN(e,t),p=e.length>0,x=f.find(y=>y.automation_id===i)||f[0]||null;return u`
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
              ${Mh.map(y=>u`
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
                                <${fd} runs=${y.recent_runs} />
                                <${pd} runs=${y.recent_runs} />
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

              <${GN}
                automation=${x}
                isMutating=${s}
                onPauseAutomation=${l}
                onResumeAutomation=${c}
                onDeleteAutomation=${d}
              />
            </div>
          `:p?u`
              <${$e}
                title=${m("automations.empty.matchingTitle")}
                description=${m("automations.empty.matchingDescription")}
              />
            `:u`<${YN} />`}
    </div>
  `}function XN({summary:e,activeFilter:t,onSelectFilter:a}){let n=R(),r=[{key:"scheduled",label:n("automations.summary.scheduled"),value:e?.scheduled??0,tone:"muted",detail:n("automations.summary.scheduledDetail"),filter:"all"},{key:"active",label:n("automations.summary.active"),value:e?.active??0,tone:"signal",detail:n("automations.summary.activeDetail"),filter:"active"},{key:"running",label:n("automations.summary.running"),value:e?.running??0,tone:"info",detail:n("automations.summary.runningDetail"),filter:"running"},{key:"failures",label:n("automations.summary.failures"),value:e?.failures??0,tone:(e?.failures??0)>0?"danger":"success",detail:n("automations.summary.failuresDetail"),filter:(e?.failures??0)>0?"failures":null},{key:"nextRun",label:n("automations.summary.nextRun"),value:e?.nextRun||n("automations.summary.none"),tone:"info",detail:n("automations.summary.nextRunDetail"),valueClassName:"text-lg md:text-xl"}];return u`
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
  `}function ZD(e){return e==="active"||e==="scheduled"}function WD(e){return Number.isFinite(e)?e:null}function ZN(e,t=Date.now()){let a=Array.isArray(e)?e:[],n=null;for(let r of a){if(!r||(r.has_running_run&&(n=n==null?5e3:Math.min(n,5e3)),!ZD(r.state)))continue;let s=WD(r.next_run_timestamp);if(s==null)continue;let i=s-t,o=i<=0?2e3:i<3e4?Math.max(1e3,i+1200):null;o!=null&&(n=n==null?o:Math.min(n,o))}return n}var tM=50,aM=25;function WN(e=!1){let{t,lang:a}=yl(),n=Y(),r=K({queryKey:["automations",{includeCompleted:e}],queryFn:()=>Px({limit:tM,runLimit:aM,includeCompleted:e}),refetchInterval:3e4,refetchIntervalInBackground:!1}),s=h.default.useMemo(()=>jN(r.data,t,a),[r.data,t,a]),i=h.default.useMemo(()=>BN(s),[s]),o=h.default.useMemo(()=>ZN(s),[s]);h.default.useEffect(()=>{if(o==null)return;let p=setTimeout(()=>{r.refetch()},o);return()=>clearTimeout(p)},[o,r.refetch]);let l=r.data?.scheduler_enabled!==!1,c=h.default.useCallback(()=>{n.invalidateQueries({queryKey:["automations"]})},[n]),d=Q({mutationFn:p=>Ux({automationId:p}),onSuccess:c}),m=Q({mutationFn:p=>jx({automationId:p}),onSuccess:c}),f=Q({mutationFn:p=>Fx({automationId:p}),onSuccess:c});return{automations:s,summary:i,schedulerEnabled:l,isLoading:r.isLoading,isRefreshing:r.isFetching,isMutating:d.isPending||m.isPending||f.isPending,error:r.error||null,actionError:d.error||m.error||f.error||null,pauseAutomation:d.mutate,resumeAutomation:m.mutate,deleteAutomation:f.mutate,refetch:r.refetch}}var e_=["outbound-delivery","preferences"],t_=["outbound-delivery","targets"];function a_(){let e=Y(),t=K({queryKey:e_,queryFn:Ix}),a=K({queryKey:t_,queryFn:Kx}),n=Q({mutationFn:({finalReplyTargetId:i})=>Hx({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(e_,i),e.invalidateQueries({queryKey:t_})}}),r=h.default.useMemo(()=>a.data?.targets??[],[a.data]),s=h.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function n_(){let e=R(),[t,a]=h.default.useState("all"),[n,r]=h.default.useState(null),i=WN(t==="completed"),o=a_(),[l,c]=h.default.useState(!1),d=h.default.useRef(null);h.default.useEffect(()=>()=>clearTimeout(d.current),[]);let m=h.default.useCallback(()=>{c(!0),clearTimeout(d.current),d.current=setTimeout(()=>c(!1),1e3),i.refetch()},[i.refetch]),f=i.isRefreshing||l,p=i.error&&!i.isLoading&&i.automations.length===0;return h.default.useEffect(()=>{if(!i.automations.length){r(null);return}i.automations.some(y=>y.automation_id===n)||r(i.automations[0].automation_id)},[i.automations,n]),u`
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

          ${p?null:u`
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
                <${XN}
                  summary=${i.summary}
                  activeFilter=${t}
                  onSelectFilter=${a}
                />
                <${MN} deliveryState=${o} />

                ${i.isLoading?u`
                      <div className="space-y-4">
                        ${[1,2,3].map(x=>u`<div
                              key=${x}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`)}
                      </div>
                    `:u`
                      <${JN}
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
  `}var r_={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function s_({result:e,onDismiss:t}){return h.default.useEffect(()=>{if(!e)return;let a=setTimeout(t,4e3);return()=>clearTimeout(a)},[e,t]),e?u`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",r_[e.type]||r_.info].join(" ")}>
      <${O}
        name=${e.type==="success"?"check":e.type==="error"?"close":"bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">
        <${O} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `:null}var o_="/api/webchat/v2/channels/slack/setup";function l_(){return H(o_)}function u_(e){let t={installation_id:String(e.installation_id||"").trim(),team_id:String(e.team_id||"").trim(),api_app_id:String(e.api_app_id||"").trim(),user_id:i_(e.user_id),shared_subject_user_id:i_(e.shared_subject_user_id)},a=String(e.bot_token||"").trim(),n=String(e.signing_secret||"").trim();return a&&(t.bot_token=a),n&&(t.signing_secret=n),H(o_,{method:"PUT",body:JSON.stringify(t)})}function Uh(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function i_(e){let t=String(e||"").trim();return t||null}var c_="/api/webchat/v2/channels/slack/allowed",nM="/api/webchat/v2/channels/slack/subjects";function d_(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function m_(){return H(c_)}function f_(){return H(nM)}function p_(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return H(c_,{method:"PUT",body:JSON.stringify(n)})}function h_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var v_=["slack-allowed-channels"];function y_({action:e}){let t=R(),a=Y(),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,l]=h.default.useState([]),c=sM(e,t),d=K({queryKey:v_,queryFn:m_}),m=K({queryKey:["slack-routable-subjects"],queryFn:f_}),f=m.data?.subjects||[],p=g_(f),x=m.isSuccess||m.isError,y=f.length>0;h.default.useEffect(()=>{d.data&&l(jh(d.data.channels||[]))},[d.data]);let $=Q({mutationFn:({channels:C})=>p_(C),onSuccess:C=>{l(jh(C.channels||[])),a.invalidateQueries({queryKey:v_}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let C=n.trim();!C||!m.isSuccess||(l(_=>jh([..._,{channel_id:C,subject_user_id:s}])),r(""))},v=C=>{l(_=>_.filter(A=>A.channel_id!==C))},b=(C,_)=>{l(A=>A.map(L=>L.channel_id===C?{...L,subject_user_id:_}:L))},w=()=>{$.mutate({channels:rM(o)})},S=m.isError&&o.some(C=>!C.subject_user_id);return u`
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
          ${p.map(C=>u`
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
                      ${g_(f,C).map(_=>u`
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
          disabled=${!d.isSuccess||!x||$.isPending||S}
        >
          ${$.isPending?c.savingLabel:c.submitLabel}
        <//>
        ${$.isSuccess&&u`<p className="text-xs text-emerald-300">
          ${c.successMessage}
        </p>`}
        ${(d.isError||m.isError||$.isError)&&u`<p className="text-xs text-red-300">
          ${h_($.error||d.error||m.error,c.errorMessage)}
        </p>`}
      </div>
    </div>
  `}function g_(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function jh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return d_(Array.from(t.keys())).map(a=>t.get(a))}function rM(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function sM(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var Fh=["slack-setup"],Vr={installationId:{body:"Local IronClaw name for this Slack install. Choose one and keep it stable.",example:"Example: local-slack"},teamId:{body:"Slack workspace/team ID from the workspace that installed the app.",example:"Example: T0123456789"},appId:{body:"Slack app Basic Information > App Credentials.",example:"Example: A0123456789"},botUser:{body:"Optional Reborn user. Blank uses the current WebUI operator.",example:"Example: user:operator"},sharedSubject:{body:"Optional default team agent for shared channel turns. Usually blank.",example:"Example: user:slack-shared"},botToken:{body:"Slack app OAuth & Permissions > Bot User OAuth Token.",example:"Example: xoxb-..."},signingSecret:{body:"Slack app Basic Information > App Credentials > Signing Secret.",example:""}};function $_({action:e}){let t=K({queryKey:Fh,queryFn:l_}),a=t.data?.configured===!0;return u`
    <div className="space-y-3">
      <${iM} action=${e} setupQuery=${t} />
      ${a&&u`<${y_} action=${e} />`}
    </div>
  `}function iM({action:e,setupQuery:t}){let a=Y(),[n,r]=h.default.useState(oM()),s=h.default.useRef(!1),i=h.default.useRef(!1),o=t.data,l=lM(e);h.default.useEffect(()=>{!o||s.current||i.current||(r(b_(o)),s.current=!0)},[o]);let c=Q({mutationFn:u_,onSuccess:p=>{i.current=!1,r(b_(p)),s.current=!0,a.setQueryData(Fh,p),a.invalidateQueries({queryKey:Fh}),a.invalidateQueries({queryKey:["slack-allowed-channels"]}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["extensions"]})}}),d=p=>x=>{i.current=!0,r(y=>({...y,[p]:x.target.value}))},m=()=>c.mutate(n),f=n.installation_id.trim()&&n.team_id.trim()&&n.api_app_id.trim()&&(o?.bot_token_configured||n.bot_token.trim())&&(o?.signing_secret_configured||n.signing_secret.trim());return u`
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
        ${vl("Installation ID",n.installation_id,d("installation_id"),"",Vr.installationId)}
        ${vl("Team ID",n.team_id,d("team_id"),"",Vr.teamId)}
        ${vl("App ID",n.api_app_id,d("api_app_id"),"",Vr.appId)}
        ${vl("Bot user",n.user_id,d("user_id"),"default operator",Vr.botUser)}
        ${vl("Shared subject",n.shared_subject_user_id,d("shared_subject_user_id"),"optional",Vr.sharedSubject)}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        ${x_("Bot token",n.bot_token,d("bot_token"),o?.bot_token_configured,Vr.botToken)}
        ${x_("Signing secret",n.signing_secret,d("signing_secret"),o?.signing_secret_configured,Vr.signingSecret)}
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
          ${Uh(t.error,l.errorMessage)}
        </p>`}
        ${c.isError&&u`<p className="text-xs text-red-300">
          ${Uh(c.error,l.errorMessage)}
        </p>`}
        ${c.isSuccess&&u`<p className="text-xs text-emerald-300">${l.successMessage}</p>`}
      </div>
    </div>
  `}function b_(e){return{installation_id:e.installation_id||"",team_id:e.team_id||"",api_app_id:e.api_app_id||"",user_id:e.user_id||"",shared_subject_user_id:e.shared_subject_user_id||"",bot_token:"",signing_secret:""}}function oM(){return{installation_id:"",team_id:"",api_app_id:"",user_id:"",shared_subject_user_id:"",bot_token:"",signing_secret:""}}function vl(e,t,a,n="",r=null){return u`
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-iron-500">${e}</span>
      <input
        type="text"
        value=${t}
        onChange=${a}
        placeholder=${n}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <${w_} help=${r} />
    </label>
  `}function x_(e,t,a,n,r=null){return u`
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
      <${w_} help=${r} />
    </label>
  `}function w_({help:e}){return e?u`
    <p className="mt-1.5 min-h-8 text-[11px] leading-4 text-iron-400">
      <span className="block">${e.body}</span>
      ${e.example&&u`<span className="mt-0.5 block font-mono text-iron-300">${e.example}</span>`}
    </p>
  `:null}function lM(e){return{title:"Slack setup",instructions:e?.instructions||"Configure the Slack app before assigning channels.",submitLabel:"Save setup",successMessage:"Slack setup saved.",errorMessage:"Slack setup update failed."}}var Bh={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function Gr(e){return e==="wasm_channel"||e==="channel"}var S_={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},N_={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function __(e){let t=k_(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||Gr(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function k_(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function zh(e){let t=k_(e);return t==="active"||t==="ready"}function R_({extension:e,secrets:t=[],fields:a=[]}={}){return zh(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var C_="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",E_="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",T_="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",A_="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",D_="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",uM="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function M_(e){return e.package_ref?.id||""}function cM({actions:e,isBusy:t}){let a=R(),[n,r]=h.default.useState(!1),s=h.default.useRef(null);return h.default.useEffect(()=>{if(!n)return;let i=o=>{s.current&&!s.current.contains(o.target)&&r(!1)};return document.addEventListener("mousedown",i),()=>document.removeEventListener("mousedown",i)},[n]),u`
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
  `}function O_({items:e}){return!e||e.length===0?null:u`
    <div className="mt-3 flex flex-wrap gap-1">
      ${e.map(t=>u`<span key=${t} className=${uM}>${t}</span>`)}
    </div>
  `}function xi({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=R(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=S_[i]||"muted",l=s(`extensions.state.${i}`)||N_[i]||i,c=s(`extensions.kind.${e.kind}`)||Bh[e.kind]||e.kind,d=e.display_name||M_(e),m=!!e.package_ref,f=e.tools||[],[p,x]=h.default.useState(!1),$=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],b=[],w=__(e);w==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):w==="activate"&&v.push({id:"activate",label:"Activate",run:()=>t(g)}),m&&(e.needs_setup||e.has_auth)&&w!=="configure"&&b.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)}),m&&Gr(e.kind)&&(i==="setup_required"||i==="failed")&&b.push({id:"setup",label:"Setup",icon:"settings",run:()=>a(g)}),m&&Gr(e.kind)&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&b.push({id:"reconfigure",label:"Reconfigure",icon:"settings",run:()=>a(g)}),m&&b.push({id:"remove",label:s("common.remove")||"Remove",icon:"trash",danger:!0,run:()=>n(g)});let S=v[0];return u`
    <div className=${C_}>
      <div className="flex items-start gap-2">
        <${q} tone=${o} label=${l} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${b.length>0&&u`<${cM} actions=${b} isBusy=${r} />`}
      </div>

      <div className=${E_}>
        <span>${c}</span>
        ${e.version&&u`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&u`<p className=${T_}>${e.description}</p>`}

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

      <div className=${A_}>
        ${f.length>0?u`
              <button
                type="button"
                aria-expanded=${p?"true":"false"}
                onClick=${()=>x(C=>!C)}
                className=${D_}
              >
                <${O} name="layers" className="h-3.5 w-3.5" />
                <span>${f.length===1?s("extensions.oneCapability"):s("extensions.pluralCapabilities",{count:f.length})}</span>
                <${O}
                  name="chevron"
                  className=${["h-3 w-3",p?"rotate-180":""].join(" ")}
                />
              </button>
            `:u`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">No capabilities</span>`}
        <span className="flex-1"></span>
        ${S&&u`
          <${D} variant="secondary" size="sm" onClick=${S.run} disabled=${r}>
            ${S.label}
          <//>
        `}
      </div>

      ${p&&u`<${O_} items=${f} />`}
    </div>
  `}function Yr({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=R(),s=r(`extensions.kind.${e.kind}`)||Bh[e.kind]||e.kind,i=e.display_name||M_(e),o=!!(e.package_ref&&t),l=e.keywords||[],[c,d]=h.default.useState(!1);return u`
    <div className=${C_}>
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

      <div className=${E_}>
        <span>${s}</span>
        ${e.version&&u`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&u`<p className=${T_}>${e.description}</p>`}

      <div className=${A_}>
        ${l.length>0?u`
              <button
                type="button"
                aria-expanded=${c?"true":"false"}
                onClick=${()=>d(m=>!m)}
                className=${D_}
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

      ${c&&u`<${O_} items=${l} />`}
    </div>
  `}function L_(){return H("/api/webchat/v2/extensions")}function P_(){return H("/api/webchat/v2/extensions/registry")}function U_(e){return H("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function j_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(gl(e))}/activate`,{method:"POST"})}function F_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(gl(e))}/remove`,{method:"POST"})}function B_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(gl(e))}/setup`)}function z_(e,t,a){return Wx(gl(e),{action:"submit",payload:{secrets:t,fields:a}})}function q_(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return H(`/api/webchat/v2/extensions/${encodeURIComponent(gl(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function I_(){return Promise.resolve({requests:[]})}function K_(){return Promise.resolve({success:!1,message:"Pairing requires a v2 pairing endpoint."})}function gl(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var dM=2e3,mM=10*60*1e3;function $i(e){return e?.package_ref?.id||null}function qh(e){return e?.display_name||$i(e)||""}function H_(e,t,a){return $i(t)||`${e}:${qh(t)||"unknown"}:${a}`}function fM(e,t){return e.installed!==t.installed?e.installed?-1:1:qh(e.entry||e.extension).localeCompare(qh(t.entry||t.extension))}function Q_(){let e=Y(),t=K({queryKey:["gateway-status-extensions"],queryFn:ai,staleTime:1e4}),a=K({queryKey:["extensions"],queryFn:L_}),n=K({queryKey:["extension-registry"],queryFn:P_}),r=K({queryKey:["connectable-channels"],queryFn:Kc}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["extensions"]}),e.invalidateQueries({queryKey:["extension-registry"]}),e.invalidateQueries({queryKey:["gateway-status-extensions"]}),e.invalidateQueries({queryKey:["connectable-channels"]})},[e]),[i,o]=h.default.useState(null),l=h.default.useCallback(()=>o(null),[]),c=Q({mutationFn:({packageRef:k})=>U_(k),onSuccess:(k,{displayName:z})=>{k.success?(o({type:"success",message:k.message||k.instructions||`${z||"Extension"} installed`}),k.auth_url&&window.open(k.auth_url,"_blank","noopener,noreferrer")):o({type:"error",message:k.message||"Install failed"}),s()},onError:k=>{o({type:"error",message:k.message}),s()}}),d=Q({mutationFn:({packageRef:k})=>j_(k),onSuccess:(k,{displayName:z})=>{k.success?(o({type:"success",message:k.message||k.instructions||`${z||"Extension"} activated`}),k.auth_url&&window.open(k.auth_url,"_blank","noopener,noreferrer")):k.auth_url?(window.open(k.auth_url,"_blank","noopener,noreferrer"),o({type:"info",message:"Opening authentication\u2026"})):k.awaiting_token?o({type:"info",message:"Configuration required"}):o({type:"error",message:k.message||"Activation failed"}),s()},onError:k=>{o({type:"error",message:k.message})}}),m=Q({mutationFn:({packageRef:k})=>F_(k),onSuccess:(k,{displayName:z})=>{k.success?o({type:"success",message:`${z||"Extension"} removed`}):o({type:"error",message:k.message||"Remove failed"}),s()},onError:k=>{o({type:"error",message:k.message})}}),f=t.data||{},p=a.data?.extensions||[],x=n.data?.entries||[],y=r.data?.channels||[],$=new Map(p.map(k=>[$i(k),k]).filter(([k])=>!!k)),g=new Set(x.map(k=>$i(k)).filter(Boolean)),v=[...x.map((k,z)=>{let Z=$i(k),re=Z&&$.get(Z)||null;return{id:H_("registry",k,z),installed:!!(re||k.installed),entry:k,extension:re}}),...p.filter(k=>{let z=$i(k);return!z||!g.has(z)}).map((k,z)=>({id:H_("installed",k,z),installed:!0,entry:null,extension:k}))].sort(fM),b=k=>Gr(k.kind),w=p.filter(b),S=p.filter(k=>k.kind==="mcp_server"),C=p.filter(k=>!b(k)&&k.kind!=="mcp_server"),_=x.filter(k=>b(k)&&!k.installed),A=x.filter(k=>k.kind==="mcp_server"&&!k.installed),L=x.filter(k=>k.kind!=="mcp_server"&&!b(k)&&!k.installed),M=a.isLoading||n.isLoading,P=c.isPending||d.isPending||m.isPending;return{status:f,extensions:p,channels:w,mcpServers:S,tools:C,channelRegistry:_,mcpRegistry:A,toolRegistry:L,registry:x,catalogEntries:v,connectableChannels:y,isLoading:M,isBusy:P,actionResult:i,clearResult:l,install:c.mutate,activate:d.mutate,remove:m.mutate,invalidate:s}}function V_(e){let t=K({queryKey:["extension-setup",e?.id||e],queryFn:()=>B_(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function G_(e,t){let a=Y(),n=e?.id||e;return Q({mutationFn:({secrets:r,fields:s})=>z_(e,r,s),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function Y_(e){let t=Y(),a=e?.id||e,n=h.default.useRef(null),r=h.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=h.default.useCallback(()=>{let l=t.getQueryData(["extension-setup",a]);if(l?.secrets?.length>0&&l.secrets.every(f=>f.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(f=>f.package_ref?.id===a),m=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return m==="active"||m==="ready"},[a,t]),o=h.default.useCallback(l=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||l&&l.closed||Date.now()-c>mM)&&(r(),s())},dM)},[r,s,i]);return h.default.useEffect(()=>r,[r]),Q({mutationFn:({secret:l,popup:c})=>q_(e,l).then(d=>({res:d,popup:c})),onSuccess:({res:l,popup:c})=>{let d=c;l.authorization_url&&c&&!c.closed?c.location.href=l.authorization_url:l.authorization_url?d=window.open(l.authorization_url,"_blank","noopener,noreferrer"):c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(l,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function J_(e,t={}){let a=K({queryKey:["pairing",e],queryFn:()=>I_(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=Y(),r=Q({mutationFn:({code:s})=>K_(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function X_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var pM={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function Z_({channel:e,redeemFn:t,i18nKeys:a=pM,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=R(),o=typeof t=="function",l=J_(e,{enabled:!o}),c=Y(),[d,m]=h.default.useState(""),f=hM(i,a,r),p=Q({mutationFn:({code:S})=>t(e,S),onSuccess:()=>{m("");for(let S of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:S})}}),x=h.default.useCallback(S=>l.approve({code:S}),[l.approve]),y=h.default.useCallback(()=>{let S=d.trim();S&&(o?p.mutate({code:S}):(l.approve({code:S}),m("")))},[o,d,l.approve,p]),$=o?[]:l.requests,g=o?!1:l.isLoading,v=o?p.isPending:l.isApproving,b=o?p.isSuccess?p.data:null:l.result,w=o?p.isError?p.error:null:l.error;return g?u`
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
        ${X_(w,f.error)}
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
                  <${D}
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
  `}function hM(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function vd(e){return e.package_ref?.id||""}function W_(e){return vd(e)==="slack"}function tk(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function ak(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function vM(e){let t=e||[],a=[t.find(tk),t.find(ak)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function ek({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>tk(r)?u`<${$_} action=${r.action} />`:ak(r)?u`<${jc} action=${r.action} />`:null).filter(Boolean);return n.length>0?u`<div className="space-y-3">${n}</div>`:null}function nk({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:l}){let c=R(),d=t||[],m=e.enabled_channels||[],f=vM(a),p=d.some(W_),x=f.length>0&&!p;return u`
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
            <${ek}
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
                <div key=${vd(y)} className="flex flex-col gap-3">
                  <${xi}
                    ext=${y}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${l}
                  />
                  ${W_(y)&&u`<${ek}
                    slackConnectActions=${f}
                  />`}
                  ${(y.onboarding_state==="pairing_required"||y.onboarding_state==="pairing")&&u` <${Z_} channel=${vd(y)} /> `}
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
                <${Yr}
                  key=${vd(y)}
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
  `}function rk({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=R(),s=e?.displayName||e?.packageRef?.id||"Extension",{secrets:i=[],fields:o=[],onboarding:l,isLoading:c,error:d}=V_(e?.packageRef),[m,f]=h.default.useState({}),[p,x]=h.default.useState({}),y=Y_(e?.packageRef),$=G_(e?.packageRef,_=>{_.success!==!1&&(n&&n(_),a())}),g=h.default.useCallback(()=>{let _={};for(let[A,L]of Object.entries(m)){let M=(L||"").trim();M&&(_[A]=M)}$.mutate({secrets:_,fields:p})},[m,p,$]),v=h.default.useCallback(_=>{let A=window.open("about:blank","_blank","width=600,height=600");A&&(A.opener=null),y.mutate({secret:_,popup:A})},[y]),w=i.filter(_=>(_.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,S=zh(e),C=R_({extension:e,secrets:i,fields:o});return c?u`
      <${gd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(_=>u`<div
                key=${_}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?u`
      <${gd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")||"Failed to load setup:"} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?u`
      <${gd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")||"No configuration required for this extension."}
        </p>
      <//>
    `:u`
    <${gd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
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
                value=${p[_.name]||""}
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
  `}function gd({onClose:e,title:t,children:a}){return h.default.useEffect(()=>{let n=r=>{r.key==="Escape"&&e()};return window.addEventListener("keydown",n),()=>window.removeEventListener("keydown",n)},[e]),u`
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
  `}function sk(e){return e.package_ref?.id||""}function ik({mcpServers:e,mcpRegistry:t,onActivate:a,onConfigure:n,onRemove:r,onInstall:s,isBusy:i}){let o=R();return e.length===0&&t.length===0?u`
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
                  key=${sk(l)}
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
                <${Yr}
                  key=${sk(l)}
                  entry=${l}
                  onInstall=${s}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function gM(e){return e?.package_ref?.id||""}function yM(e){return e.entry||e.extension||{}}function ok({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=R(),[o,l]=h.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(y=>{let $=yM(y);return($.display_name||gM($)).toLowerCase().includes(c)||($.description||"").toLowerCase().includes(c)||($.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,m=d.filter(y=>y.installed&&y.extension),f=d.filter(y=>y.installed&&!y.extension&&y.entry),p=m.length+f.length,x=d.filter(y=>!y.installed&&y.entry);return e.length===0?u`
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
              ${p>0&&u`
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
                      <${Yr}
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
                  className=${["mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal",p>0?"mt-6":""].join(" ")}
                >
                  ${i("ext.registry.availableTitle")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${x.map(y=>u`
                      <${Yr}
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
  `}function Ih(){let{tab:e="registry"}=st(),[t,a]=h.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:l,connectableChannels:c,isLoading:d,isBusy:m,actionResult:f,clearResult:p,install:x,activate:y,remove:$,invalidate:g}=Q_(),v=h.default.useCallback(_=>a(_),[]),b=h.default.useCallback(()=>a(null),[]),w=h.default.useCallback(()=>g(),[g]),S=h.default.useCallback(_=>{_&&(y(_),a(null))},[y]);if(d)return u`
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
    `;if(e==="installed")return u`<${it} to="/extensions/registry" replace />`;let C={channels:u`<${nk}
      status=${n}
      channels=${r}
      connectableChannels=${c}
      channelRegistry=${i}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${x}
      isBusy=${m}
    />`,mcp:u`<${ik}
      mcpServers=${s}
      mcpRegistry=${o}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${x}
      isBusy=${m}
    />`,registry:u`<${ok}
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
          <${s_} result=${f} onDismiss=${p} />
          ${C[e]}
        </div>
      </div>

      ${t&&u`
        <${rk}
          extension=${t}
          onActivate=${S}
          onClose=${b}
          onSaved=${w}
        />
      `}
    </div>
  `:u`<${it} to="/extensions/registry" replace />`}var lk=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],uk=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],ck=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],Kh=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function dk(e){return String(e||"").trim().toLowerCase()}function mk(e){if(e==null)return"";if(Array.isArray(e))return e.map(mk).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function et(e,t){let a=dk(e);return a?t.map(mk).join(" ").toLowerCase().includes(a):!0}function Si(e,t,a,n){let r=dk(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(l=>et(r,[i,l.key,l.labelKey?n(l.labelKey):l.label,l.descKey?n(l.descKey):l.description,t[l.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function bM({visible:e}){let t=R();return e?u`
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      ${t("tools.saved")}
    </span>
  `:null}function xM({checked:e,onChange:t,label:a}){return u`
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
  `}function $M({field:e,value:t,onSave:a,isSaved:n}){let r=R(),[s,i]=h.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",l=e.descKey?r(e.descKey):e.description||"";h.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=h.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let m=parseInt(d,10);isNaN(m)||a(e.key,m)}else if(e.type==="float"){let m=parseFloat(d);isNaN(m)||a(e.key,m)}else a(e.key,d)},[e.key,e.type,a]);return u`
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">${o}</div>
        ${l&&u`<div className="mt-1 text-xs leading-5 text-iron-300">${l}</div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${e.type==="boolean"?u`
              <${xM}
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
        <${bM} visible=${n} />
      </div>
    </div>
  `}function Ni({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=R(),o=t?i(t):e||"";return u`
    <${ee} className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${o}</h3>
      <div>
        ${a.map(l=>u`
              <${$M}
                key=${l.key}
                field=${l}
                value=${n[l.key]}
                onSave=${r}
                isSaved=${s[l.key]}
              />
            `)}
      </div>
    <//>
  `}function Nt({query:e}){let t=R();return u`
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
  `}function fk({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=R();if(n)return u`<${wM} />`;let i=Si(uk,e,r,s);return i.length===0?u`<${Nt} query=${r} />`:u`
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
  `}function wM(){return u`
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
  `}function pk(){let e=K({queryKey:["gateway-status-settings"],queryFn:ai,staleTime:1e4}),t=K({queryKey:["extensions"],queryFn:G$}),a=K({queryKey:["extension-registry"],queryFn:Y$}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(m=>m.kind==="wasm_channel"||m.kind==="channel"),o=s.filter(m=>(m.kind==="wasm_channel"||m.kind==="channel")&&!m.installed),l=r.filter(m=>m.kind==="mcp_server"),c=s.filter(m=>m.kind==="mcp_server"&&!m.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:l,mcpRegistry:c,extensions:r,isLoading:d}}function SM({name:e,description:t,enabled:a,detail:n}){let r=R();return u`
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
  `}function hk({channel:e,registryEntry:t}){let a=R(),n=t?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},l={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return u`
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
  `}function NM(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function _M({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=NM(e,i).filter(x=>et(s,[i("channels.builtIn"),x.id,x.name,x.description,x.detail])),l=new Set(t.map(x=>x.name)),c=t.filter(x=>et(s,[i("channels.messaging"),x.name,x.display_name,x.description,x.onboarding_state])),d=a.filter(x=>!l.has(x.name)).filter(x=>et(s,[i("channels.messaging"),x.name,x.display_name,x.description])),m=new Set(n.map(x=>x.name)),f=n.filter(x=>et(s,[i("channels.mcpServers"),x.name,x.display_name,x.description,x.active?i("channels.active"):i("channels.inactive")])),p=r.filter(x=>!m.has(x.name)).filter(x=>et(s,[i("channels.mcpServers"),x.name,x.display_name,x.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:p}}function vk({searchQuery:e=""}){let t=R(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=pk();if(o)return u`
      <div className="space-y-5">
        <${ee} padding="md">
          <div className="mb-4 h-3 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(p=>u`
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
    `;let{builtInChannels:l,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:f}=_M({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return l.length===0&&c.length===0&&d.length===0&&m.length===0&&f.length===0?u`<${Nt} query=${e} />`:u`
    <div className="space-y-5">
      ${l.length>0&&u`
      <${ee} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${l.map(p=>u`
            <${SM}
              key=${p.id}
              name=${p.name}
              description=${p.description}
              enabled=${p.enabled}
              detail=${p.detail}
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
          ${c.map(p=>u`
              <${hk}
                key=${p.name}
                channel=${p}
                registryEntry=${r.find(x=>x.name===p.name)}
              />
            `)}
          ${d.map(p=>u`
              <${hk} key=${p.name} registryEntry=${p} />
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
          ${m.map(p=>u`
                <div
                  key=${p.name}
                  className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-[var(--v2-text)]"
                        >${p.display_name||p.name}</span
                      >
                      <${q}
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
          ${f.map(p=>u`
                <div
                  key=${p.name}
                  className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-[var(--v2-text)]"
                        >${p.display_name||p.name}</span
                      >
                      <${q}
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
  `}function gk({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:l,onNearaiWallet:c,onCodexLogin:d,loginBusy:m}){let f=R(),p=e.id===t,x=Ir(e,n),y=si(e,n),$=lw(e,n,t,a),g=kc(e,n),v=uw(e),b=f(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[w,S]=h.default.useState(p),C=h.default.useCallback(()=>S(Te=>!Te),[]);h.default.useEffect(()=>{S(p)},[p]);let _=x?u`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${rl(e.adapter)} · ${$||e.default_model||f("llm.none")}
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
        `:null,k=!p&&e.id==="nearai"?u`
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
        `:!p&&e.id==="openai_codex"?u`
          <${D} type="button" variant="secondary" size="sm" disabled=${m} onClick=${d}>
            ${f("onboarding.codexSignIn")}
          <//>
        `:null,Z=!p&&x&&(!A||e.id==="nearai"&&e.has_api_key===!0)?u`
        <${D}
          type="button"
          variant="primary"
          size="sm"
          disabled=${r}
          onClick=${()=>s(e)}
        >
          ${f("llm.use")}
        <//>
      `:null,re=x?null:u`
        <${D}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${r}
          onClick=${()=>i(e)}
        >
          ${f(g==="api_key"?"llm.addApiKey":"llm.configure")}
        <//>
      `,me=p?null:Z||(A?k:re),he=!A&&(e.builtin&&e.id!=="bedrock"||!e.builtin)||e.id==="nearai"&&v;return u`
    <${ee}
      padding="none"
      data-testid="llm-provider-card"
      data-provider-id=${e.id}
      className=${["transition-colors",p?"border-[color-mix(in_srgb,var(--v2-positive-text)_36%,var(--v2-panel-border))]":w?"border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))]":""].join(" ")}
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
            className=${["h-2 w-2 shrink-0 rounded-full",p?"bg-[var(--v2-positive-text)]":x?"bg-[var(--v2-accent)]":"bg-[var(--v2-warning-text)]"].join(" ")}
          />
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
            <span className="min-w-0 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
              ${e.name||e.id}
            </span>
            <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">${e.id}</span>
            ${p&&u`<${q} tone="positive" label=${f("llm.active")} size="sm" />`}
            ${e.builtin&&!p&&u`<${q} tone="muted" label=${f("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${_}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 py-3 pr-4 sm:pr-5">
          ${me}
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
              <div className="mt-1 truncate">${rl(e.adapter)}</div>
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
            ${!e.builtin&&!p&&u`
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
  `}var kM=[{key:"active",labelKey:"llm.groupActive",dotClass:"bg-[var(--v2-positive-text)]"},{key:"ready",labelKey:"llm.groupReady",dotClass:"bg-[var(--v2-accent)]"},{key:"setup",labelKey:"llm.groupSetup",dotClass:"bg-[var(--v2-warning-text)]"}];function RM({label:e,count:t,dotClass:a}){return u`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full "+a} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${t}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `}function yk({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=R(),r=Xc({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=Zc(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return u`<${Nt} query=${a} />`;let l=cw(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return u`
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

      <${Jc} login=${i} />

      ${s.isLoading?u`<div className="text-sm text-[var(--v2-text-muted)]">${n("common.loading")}</div>`:s.error?u`<div className="text-sm text-red-200">${n("error.loadFailed",{what:n("llm.providers"),message:s.error.message})}</div>`:u`
            <div className="space-y-1">
              ${kM.flatMap(c=>{let d=l[c.key];return d.length?[u`
                    <section
                      key=${c.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${c.key}
                      className="mb-3"
                    >
                      <${RM}
                        label=${n(c.labelKey)}
                        count=${d.length}
                        dotClass=${c.dotClass}
                      />
                      <div className="space-y-2">
                      ${d.map(m=>u`
                          <${gk}
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

      <${Yc}
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
  `}function bk({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=R(),{activeProviderId:o,selectedModel:l,providers:c,hasActiveProvider:d}=ii({settings:e,gatewayStatus:t});if(r)return u`<${CM} />`;let m=d?o:"",f=c.find(g=>g.id===o),p=d&&(l||f?.default_model||e.selected_model)||"",x=Si(lk,e,s,i),y=et(s,[i("inference.provider"),i("inference.backend"),m,i("inference.model"),p]),$=et(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!y&&!$&&x.length===0?u`<${Nt} query=${s} />`:u`
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
              ${p||i("inference.none")}
            </div>
          </div>
        </div>
      <//>
      `}

      ${$&&u`
        <${yk}
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
  `}function cr({className:e=""}){return u`
    <div
      className=${"rounded animate-pulse bg-[var(--v2-surface-muted)] "+e}
    />
  `}function CM(){return u`
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
      ${[1,2].map(e=>u`
            <${ee} key=${e} padding="md">
              <${cr} className="mb-4 h-3 w-20" />
              ${[1,2,3].map(t=>u`
                    <div key=${t} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                      <${cr} className="h-4 w-32" />
                      <${cr} className="h-9 w-36" />
                    </div>
                  `)}
            <//>
          `)}
    </div>
  `}function xk({searchQuery:e=""}){let t=R(),{lang:a,setLang:n}=yl(),r=bl.find(i=>i.code===a)||bl[0],s=bl.filter(i=>et(e,[i.code,i.name,i.native]));return s.length===0?u`<${Nt} query=${e} />`:u`
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
  `}function $k({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=R();if(n)return u`
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
    `;let i=Si(ck,e,r,s);return i.length===0?u`<${Nt} query=${r} />`:u`
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
  `}function wk(){let e=R(),[t,a]=h.default.useState(!1),n=h.default.useCallback(()=>a(!0),[]),r=h.default.useCallback(()=>a(!1),[]),s=h.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function Sk({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=R(),r=wk({gatewayStatus:t,gatewayStatusQuery:a});return e?u`
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
  `:null}function Nk(){let e=Y(),t=K({queryKey:["skills"],queryFn:J$}),a=Q({mutationFn:Z$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),n=Q({mutationFn:ew,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),r=Q({mutationFn:({name:c,content:d})=>W$(c,{content:d}),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),s=Q({mutationFn:({name:c,enabled:d})=>tw(c,d),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),i=Q({mutationFn:c=>aw(c),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),o=t.data?.skills||[],l=t.data?.auto_activate_learned!==!1;return{skills:o,query:t,autoActivateLearned:l,fetchSkillContent:X$,installSkill:a.mutateAsync,removeSkill:n.mutateAsync,updateSkill:r.mutateAsync,setSkillAutoActivate:s.mutateAsync,setAutoActivateLearned:i.mutateAsync,isInstalling:a.isPending,isRemoving:n.isPending,isUpdating:r.isPending,isSettingAutoActivate:s.isPending,isSettingAutoActivateLearned:i.isPending}}function _k({skill:e,onEdit:t,onRemove:a,onUpdate:n,onSetAutoActivate:r,isRemoving:s,isUpdating:i,isSettingAutoActivate:o}){let l=R(),c=e.name||e.id,d=e.trust||e.trust_level||"installed",m=e.source_kind||"installed",f=!!e.can_edit,p=!!e.can_delete,x=e.auto_activate!==!1,[y,$]=h.default.useState(!1),[g,v]=h.default.useState(""),[b,w]=h.default.useState(""),[S,C]=h.default.useState(!1);h.default.useEffect(()=>{y||(v(""),w(""))},[y]);let _=h.default.useCallback(async()=>{C(!0),w("");try{let L=await t(c);v(L?.content||""),$(!0)}catch(L){w(L.message||l("skills.contentLoadFailed"))}finally{C(!1)}},[c,t,l]),A=h.default.useCallback(async()=>{(await n(c,g))?.success&&$(!1)},[g,c,n]);return u`
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
                  <${Pc}
                    rows=${12}
                    value=${g}
                    className="font-mono text-xs leading-5"
                    onInput=${L=>v(L.currentTarget.value)}
                  />
                </div>
              `:u`<${EM} skill=${e} />`}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          ${f&&!y&&u`
            <${D}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${i||S}
              title=${l("skills.edit")}
              onClick=${_}
            >
              <${O} name="file" className="h-4 w-4" />
              ${l(S?"skills.loading":"skills.edit")}
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
          ${p&&!y&&u`
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
  `}function EM({skill:e}){let t=R();return u`
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
        ${e.has_requirements&&u`<${Hh}>requirements.txt<//>`}
        ${e.has_scripts&&u`<${Hh}>scripts/<//>`}
        ${e.install_source_url&&u`<${Hh}>${t("skills.imported")}<//>`}
      </div>
    `}
  `}function Hh({children:e}){return u`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${e}
    </span>
  `}function kk({onInstall:e,isInstalling:t}){let a=R(),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,l]=h.default.useState({name:"",content:""}),[c,d]=h.default.useState(""),[m,f]=h.default.useState(""),p=h.default.useCallback((y,$)=>{l(g=>!g[y]||!$.trim()?g:{...g,[y]:""})},[]),x=h.default.useCallback(async()=>{let y=TM({name:n,content:s}),$=AM(y,a);if($.name||$.content){l($),d(""),f("");return}l({name:"",content:""}),d(""),f("");try{let g=await e(y);if(!g?.success){d(g?.message||a("skills.installFailed"));return}r(""),i(""),f(g.message||a("skills.installedSuccess",{name:y.name}))}catch(g){d(g.message||a("skills.installFailed"))}},[s,n,e,a]);return u`
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

      <${Sn} label=${a("skills.name")} error=${o.name} required>
        <${Tt}
          size="sm"
          error=${!!o.name}
          aria-invalid=${o.name?"true":void 0}
          value=${n}
          placeholder=${a("skills.namePlaceholder")}
          onInput=${y=>{let $=y.currentTarget.value;r($),p("name",$)}}
        />
      <//>

      <${Sn}
        className="mt-3"
        label=${a("skills.content")}
        error=${o.content}
        hint=${a("skills.contentHint")}
        required
      >
        <${Pc}
          rows=${5}
          error=${!!o.content}
          aria-invalid=${o.content?"true":void 0}
          value=${s}
          placeholder=${a("skills.contentPlaceholder")}
          onInput=${y=>{let $=y.currentTarget.value;i($),p("content",$)}}
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
  `}function TM({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function AM(e,t){return{name:e.name?"":t("skills.nameRequired"),content:e.content?"":t("skills.contentRequired")}}function Rk({searchQuery:e=""}){let t=R(),{skills:a,query:n,autoActivateLearned:r,fetchSkillContent:s,installSkill:i,removeSkill:o,updateSkill:l,setSkillAutoActivate:c,setAutoActivateLearned:d,isInstalling:m,isRemoving:f,isUpdating:p,isSettingAutoActivate:x,isSettingAutoActivateLearned:y}=Nk(),[$,g]=h.default.useState(""),[v,b]=h.default.useState(""),w=h.default.useCallback(async L=>{if(window.confirm(t("skills.confirmDelete",{name:L}))){g(""),b("");try{let M=await o(L);if(!M?.success){g(M?.message||t("skills.removeFailed"));return}b(M.message||t("skills.removed",{name:L}))}catch(M){g(M.message||t("skills.removeFailed"))}}},[o,t]),S=h.default.useCallback(async(L,M)=>{if(!M.trim())return g(t("skills.contentRequired")),b(""),{success:!1,message:t("skills.contentRequired")};g(""),b("");try{let P=await l({name:L,content:M});return P?.success?(b(P.message||t("skills.updated",{name:L})),P):(g(P?.message||t("skills.updateFailed")),P)}catch(P){let k=P.message||t("skills.updateFailed");return g(k),{success:!1,message:k}}},[t,l]),C=h.default.useCallback(async(L,M)=>{g(""),b("");try{let P=await c({name:L,enabled:M});if(!P?.success){g(P?.message||t("skills.updateFailed"));return}b(P.message)}catch(P){g(P.message||t("skills.updateFailed"))}},[c,t]),_=h.default.useCallback(async L=>{g(""),b("");try{let M=await d(L);if(!M?.success){g(M?.message||t("skills.updateFailed"));return}b(M.message)}catch(M){g(M.message||t("skills.updateFailed"))}},[d,t]),A;if(n.isLoading)A=u`
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
    `;else{let L=a.filter(P=>et(e,[P.name,P.id,P.description,P.keywords,P.trust_level,P.source_kind,P.version])),M=OM(L);a.length===0?A=u`
        <${ee} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:L.length===0?A=u`<${Nt} query=${e} />`:A=u`
        <div id="skills-list">
          ${M.map(P=>u`
              <${MM}
                key=${P.id}
                title=${t(P.labelKey)}
                skills=${P.skills}
                onEdit=${s}
                onRemove=${w}
                onUpdate=${S}
                onSetAutoActivate=${C}
                isRemoving=${f}
                isUpdating=${p}
                isSettingAutoActivate=${x}
              />
            `)}
        </div>
      `}return u`
    <div className="space-y-4">
      <${DM}
        enabled=${r}
        isSaving=${y}
        onToggle=${_}
      />
      <${kk} onInstall=${i} isInstalling=${m} />
      <${LM} error=${$} result=${v} />
      ${A}
    </div>
  `}function DM({enabled:e,isSaving:t,onToggle:a}){let n=R();return u`
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
  `}function MM({title:e,skills:t,onEdit:a,onRemove:n,onUpdate:r,onSetAutoActivate:s,isRemoving:i,isUpdating:o,isSettingAutoActivate:l}){return t.length===0?null:u`
    <${ee} padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${e}
      </h3>
      ${t.map(c=>u`
          <${_k}
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
  `}function OM(e){let t=[{id:"user",labelKey:"skills.group.user",skills:[]},{id:"system",labelKey:"skills.group.system",skills:[]},{id:"workspace",labelKey:"skills.group.workspace",skills:[]}],a=t[0];for(let n of e){let r=n.source_kind||"";(r==="system"?t[1]:r==="workspace"?t[2]:a).skills.push(n)}return t.filter(n=>n.skills.length>0)}function LM({error:e,result:t}){return!e&&!t?null:u`
    <div
      className=${e?"rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200":"rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200"}
    >
      ${e||t}
    </div>
  `}function yd(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function Ck(){let e=Y(),t=K({queryKey:["settings-tools"],queryFn:Q$}),a=t.data?.tools||[],[n,r]=h.default.useState({}),s=Q({mutationFn:async({name:o,state:l})=>yd(await V$(o,l),"Save failed"),onSuccess:(o,{name:l,state:c})=>{e.setQueryData(["settings-tools"],d=>{if(!d)return d;let m=o?.tool;return{...d,tools:d.tools.map(f=>f.name===l?{...f,state:c,...m||{}}:f)}}),r(d=>({...d,[l]:!0})),setTimeout(()=>r(d=>({...d,[l]:!1})),2e3)}}),i=h.default.useCallback((o,l)=>s.mutate({name:o,state:l}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}var Qh="agent.auto_approve_tools";function PM({visible:e}){let t=R();return e?u`
    <span className="font-mono text-[11px] text-[var(--v2-accent-text)]" role="status">
      ${t("tools.saved")}
    </span>
  `:null}function UM({checked:e,disabled:t=!1,label:a,onChange:n}){return u`
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
  `}function Vh({settings:e,onSave:t,savedKeys:a,isLoading:n}){let r=R(),s=r("settings.field.autoApproveEligibleTools"),i=e?.[Qh],o=i==null?!0:i===!0||i==="true";return u`
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
        <${PM} visible=${a?.[Qh]} />
        <${UM}
          checked=${o}
          disabled=${n}
          label=${s}
          onChange=${l=>t(Qh,l)}
        />
      </div>
    <//>
  `}function jM({tool:e,onPermissionChange:t,isSaved:a}){let n=R(),r=[{value:"default",label:n("tools.followDefault"),tone:"neutral"},{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask_each_time",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],s={default:n("tools.sourceDefault"),global:n("tools.sourceGlobal"),override:n("tools.sourceOverride")},i=e.locked,o=r.find(m=>m.value===e.state)||r[1],l=e.effective_source||"default",c=l==="override"?e.state:"default",d=l==="default"&&e.state===e.default_state;return u`
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
  `}function Ek({settings:e={},onSave:t=()=>{},savedKeys:a={},isLoading:n=!1,searchQuery:r=""}){let s=R(),{tools:i,query:o,setPermission:l,savedTools:c}=Ck();if(o.isLoading)return u`
      <div className="space-y-4">
        <${Vh}
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
        <${Vh}
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
      <${Vh}
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
                  <${jM}
                    key=${m.name}
                    tool=${m}
                    onPermissionChange=${l}
                    isSaved=${c[m.name]}
                  />
                `)}
      <//>
    </div>
  `}function Tk(e){return(Number(e)||0).toFixed(2)}function FM(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function Ak(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function Jr({label:e,value:t,description:a}){return u`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&u`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function Dk({searchQuery:e=""}){let t=R(),{credits:a,query:n,authorize:r}=Ec();if(!et(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return u`<${Nt} query=${e} />`;let s;if(n.isLoading)s=u`
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
        <${Jr}
          label=${t("traceCommons.enrollment")}
          value=${a.enrolled?t("traceCommons.enrolled"):t("traceCommons.notEnrolled")}
        />
        <${Jr}
          label=${t("traceCommons.pendingCredit")}
          description=${t("traceCommons.pendingCreditDesc")}
          value=${Tk(a.pending_credit)}
        />
        <${Jr}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${Tk(a.final_credit)}
        />
        <${Jr}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${FM(a.delayed_credit_delta)}
        />
        <${Jr}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${Jr}
          label=${t("traceCommons.lastSubmission")}
          value=${Ak(a.last_submission_at,t)}
        />
        <${Jr}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${Ak(a.last_credit_sync_at,t)}
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
  `}function Mk(){let e=Y(),t=K({queryKey:["admin-users"],queryFn:sw,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=Q({mutationFn:iw,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=Q({mutationFn:({id:i,payload:o})=>ow(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function BM({onCreate:e,isCreating:t,error:a}){let n=R(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[l,c]=h.default.useState("member"),[d,m]=h.default.useState(!1),f=p=>{p.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:l},{onSuccess:()=>{s(""),o(""),m(!1)}})};return d?u`
    <${ee} padding="md">
      <h3
        className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${n("users.newUser")}
      </h3>
      <form onSubmit=${f} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <${Sn} label=${n("users.displayName")} htmlFor="user-name">
            <${Tt}
              id="user-name"
              type="text"
              value=${r}
              onChange=${p=>s(p.target.value)}
              required
            />
          <//>
          <${Sn} label=${n("users.email")} htmlFor="user-email">
            <${Tt}
              id="user-email"
              type="email"
              value=${i}
              onChange=${p=>o(p.target.value)}
            />
          <//>
        </div>
        <${Sn} label=${n("users.role")} htmlFor="user-role">
          <select
            id="user-role"
            value=${l}
            onChange=${p=>c(p.target.value)}
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
    `}function zM({user:e}){let t=R(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return u`
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
  `}function Ok({searchQuery:e=""}){let t=R(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=Mk();if(n.isLoading)return u`
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
      <${BM}
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
            </p>`:l.map(c=>u`<${zM} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function Lk(){let e=Y(),t=K({queryKey:["settings-export"],queryFn:U$,staleTime:3e4}),a=t.data?.settings||{},[n,r]=h.default.useState({}),[s,i]=h.default.useState(!1),o=Q({mutationFn:async({key:m,value:f})=>yd(await Vp(m,f),"Save failed"),onSuccess:(m,{key:f,value:p})=>{e.setQueryData(["settings-export"],x=>{if(!x)return x;let y={...x,settings:{...x.settings}};return p==null?delete y.settings[f]:y.settings[f]=p,y}),r(x=>({...x,[f]:!0})),setTimeout(()=>r(x=>({...x,[f]:!1})),2e3),Kh.has(f)&&i(!0),f==="agent.auto_approve_tools"&&e.invalidateQueries({queryKey:["settings-tools"]})}}),l=h.default.useCallback((m,f)=>o.mutate({key:m,value:f}),[o]),c=Q({mutationFn:j$,onSuccess:(m,f)=>{e.invalidateQueries({queryKey:["settings-export"]});let p=Object.keys(f?.settings||{});p.includes("agent.auto_approve_tools")&&e.invalidateQueries({queryKey:["settings-tools"]}),p.some(x=>Kh.has(x))&&i(!0)}}),d=h.default.useCallback(m=>c.mutateAsync(m),[c]);return{settings:a,query:t,save:l,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error||c.error}}function Gh(){let e=R(),{tab:t}=st(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=ya(),s=r?"inference":"language",i=t||s,{settings:o,query:l,save:c,savedKeys:d,needsRestart:m,saveError:f}=Lk(),[p,x]=h.default.useState("");h.default.useEffect(()=>{x("")},[i]);let y=l.isLoading,$={inference:u`<${bk}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,agent:u`<${fk}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,channels:u`<${vk} searchQuery=${p} />`,networking:u`<${$k}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,tools:u`<${Ek}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,skills:u`<${Rk} searchQuery=${p} />`,traces:u`<${Dk} searchQuery=${p} />`,users:u`<${Ok} searchQuery=${p} />`,language:u`<${xk} searchQuery=${p} />`},g=C=>C==="users"||C==="inference",v=C=>Object.prototype.hasOwnProperty.call($,C),b=Object.keys($).filter(C=>r||!g(C)),S=v(s)&&b.includes(s)?s:b[0]||"language";return!v(i)||!r&&g(i)?u`<${it} to=${`/settings/${S}`} replace />`:u`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${m&&u`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${Sk}
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
  `}var Yh=Object.freeze({todo:!0});function Pk(){return Promise.resolve({users:[],total:0,...Yh})}function Uk(e){return Promise.resolve(null)}function jk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Fk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Bk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function zk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function qk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Ik(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Kk(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...Yh})}function Hk(e="day",t){return Promise.resolve({entries:[],...Yh})}function Qk(){return K({queryKey:["admin","usage-summary"],queryFn:Kk,refetchInterval:3e4})}function bd(e="day",t){return K({queryKey:["admin","usage",e,t],queryFn:()=>Hk(e,t),refetchInterval:3e4})}function _i(){let e=Y(),t=K({queryKey:["admin","users"],queryFn:Pk,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=Q({mutationFn:jk,onSuccess:s}),o=Q({mutationFn:({id:f,payload:p})=>Fk(f,p),onSuccess:s}),l=Q({mutationFn:f=>Bk(f),onSuccess:s}),c=Q({mutationFn:f=>zk(f),onSuccess:s}),d=Q({mutationFn:f=>qk(f),onSuccess:s}),m=Q({mutationFn:({userId:f,name:p})=>Ik(f,p)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(f,p)=>o.mutateAsync({id:f,payload:p}),deleteUser:l.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(f,p)=>m.mutateAsync({userId:f,name:p}),newToken:m.data,clearToken:()=>m.reset()}}function Vk(e){return K({queryKey:["admin","user",e],queryFn:()=>Uk(e),enabled:!!e,refetchInterval:1e4})}function Ja(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function Ca(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function Gk(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function dr(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function ki(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function Ri(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function Ci(e){return e==="admin"?"signal":"muted"}function Yk(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function Jk(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function Xk(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function Zk(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function Wk(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function qM({users:e,onSelectUser:t}){let a=R(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?u`
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
                <td className="py-3 pr-4"><${q} tone=${Ci(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${q} tone=${Ri(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${dr(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:u`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function eR({onSelectUser:e,onNavigateTab:t}){let a=R(),n=Qk(),{users:r,query:s}=_i(),i=n.data||{},o=Yk(r),l=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?u`
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
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:Gk(i.uptime_seconds)})}</span>
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
        <${qM} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var IM=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function KM({value:e,max:t}){let a=t>0?e/t*100:0;return u`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function tR({onSelectUser:e}){let t=R(),[a,n]=h.default.useState("day"),r=bd(a),s=r.data?.usage||[],i=Xk(s),o=Zk(s),l=Wk(i),c=i.length>0?i[0].cost:0;return r.isLoading?u`
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
            ${IM.map(d=>u`
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
                <${We} label=${t("admin.usage.inputTokens")} value=${Ja(l.input_tokens)} tone="muted" />
                <${We} label=${t("admin.usage.outputTokens")} value=${Ja(l.output_tokens)} tone="muted" />
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
                          ${ki(d.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ja(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ja(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${Ca(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${KM} value=${d.cost} max=${c} />
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
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ja(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ja(d.output_tokens)}</td>
                      <td className="py-3 font-mono text-xs text-iron-100">${Ca(d.cost.toFixed(2))}</td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}
    </div>
  `}function mr({label:e,children:t}){return u`
    <div className="flex items-start justify-between gap-4 border-t border-white/[0.06] py-3 first:border-0 first:pt-0">
      <span className="text-xs text-iron-300">${e}</span>
      <span className="text-right text-sm text-iron-100">${t}</span>
    </div>
  `}function aR({userId:e,onBack:t}){let a=R(),n=Vk(e),r=bd("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:l,createToken:c,newToken:d,clearToken:m}=_i(),[f,p]=h.default.useState(null),[x,y]=h.default.useState(!1),$=n.data,g=r.data?.usage||[];if(h.default.useEffect(()=>{$&&f===null&&p($.role)},[$]),n.isLoading)return u`
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
    `;if(!$)return null;let v=async()=>{f&&f!==$.role&&await o($.id,{role:f})},b=async()=>{await l($.id),t()},w=async()=>{let S=window.prompt(a("admin.users.tokenNamePrompt",{name:$.display_name||a("admin.users.userFallback")}));S&&await c($.id,S)};return u`
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
              <${q} tone=${Ci($.role)} label=${$.role||"member"} />
              <${q} tone=${Ri($.status)} label=${$.status||"active"} />
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
          <${mr} label=${a("admin.user.id")}>
            <span className="font-mono text-xs">${$.id}</span>
          <//>
          <${mr} label=${a("admin.user.email")}>${$.email||a("admin.user.notSet")}<//>
          <${mr} label=${a("admin.user.created")}>${dr($.created_at)}<//>
          <${mr} label=${a("admin.user.lastLogin")}>${dr($.last_login_at)}<//>
          ${$.created_by&&u`
            <${mr} label=${a("admin.user.createdBy")}>
              <span className="font-mono text-xs">${ki($.created_by)}</span>
            <//>
          `}
        <//>

        <${I} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${mr} label=${a("admin.user.jobs")}>${$.job_count??0}<//>
          <${mr} label=${a("admin.user.totalCost")}>${Ca($.total_cost)}<//>
          <${mr} label=${a("admin.user.lastActive")}>${dr($.last_active_at)}<//>
        <//>
      </div>

      <${I} className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.roleManagement")}</h3>
        <div className="flex items-end gap-3">
          <div>
            <label className="mb-1 block text-xs text-iron-300">${a("admin.user.currentRole")}</label>
            <select
              value=${f||$.role}
              onChange=${S=>p(S.target.value)}
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
                    ${g.map((S,C)=>u`
                        <tr key=${C} className="border-b border-white/[0.06] last:border-0">
                          <td className="py-3 pr-4 font-mono text-xs text-iron-100">${S.model}</td>
                          <td className="py-3 pr-4 font-mono text-xs text-iron-300">${(S.call_count||0).toLocaleString()}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ja(S.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ja(S.output_tokens)}</td>
                          <td className="py-3 font-mono text-xs text-iron-100">${Ca(S.total_cost)}</td>
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
  `}function HM(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function QM({token:e,onDismiss:t}){let a=R(),[n,r]=h.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return u`
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
  `}function VM({onCreate:e,isCreating:t,error:a}){let n=R(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[l,c]=h.default.useState("member"),[d,m]=h.default.useState(!1),f=async p=>{p.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:l}),s(""),o(""),m(!1))};return d?u`
    <${I} className="p-5 sm:p-6">
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
              value=${l}
              onChange=${p=>c(p.target.value)}
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
    `}function GM({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=R();return u`
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
  `}function YM({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=R();return u`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${q} tone=${Ci(e.role)} label=${e.role||"member"} />
          <${q} tone=${Ri(e.status)} label=${e.status||"active"} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          ${e.email&&u`<span className="font-mono text-xs text-iron-300">${e.email}</span>`}
          <span className="font-mono text-xs text-iron-700">${ki(e.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-iron-300 sm:inline">
          ${e.job_count!=null?i("admin.users.jobsCount",{count:e.job_count}):""}
          ${e.total_cost!=null?` \xB7 ${Ca(e.total_cost)}`:""}
        </span>
        <span className="hidden text-xs text-iron-700 lg:inline">${dr(e.last_active_at)}</span>
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
  `}function nR({selectedUserId:e,onSelectUser:t}){let a=R(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:l,updateUser:c,deleteUser:d,suspendUser:m,activateUser:f,createToken:p,newToken:x,clearToken:y}=_i(),[$,g]=h.default.useState(""),[v,b]=h.default.useState("all"),[w,S]=h.default.useState(null),C=Jk(n,{search:$,filter:v}),_=HM(a),A=M=>{S({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{m(M),S(null)}})},L=async(M,P)=>{let k=window.prompt(a("admin.users.tokenNamePrompt",{name:P||a("admin.users.userFallback")}));k&&await p(M,k)};return r.isLoading?u`
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
        <${QM}
          token=${x.token||x.plaintext_token}
          onDismiss=${y}
        />
      `}

      <${VM} onCreate=${i} isCreating=${o} error=${l} />

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
                <${YM}
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
        <${GM}
          title=${w.title}
          message=${w.message}
          confirmLabel=${w.confirmLabel}
          onConfirm=${w.onConfirm}
          onCancel=${()=>S(null)}
        />
      `}
    </div>
  `}function rR(){let{tab:e="dashboard"}=st(),t=pe(),[a,n]=h.default.useState(null),r=h.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=h.default.useCallback(()=>{n(null)},[]),i={dashboard:u`<${eR}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?u`<${aR} userId=${a} onBack=${s} />`:u`<${nR}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:u`<${tR} onSelectUser=${r} />`};return i[e]?u`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:u`<${it} to="/admin/dashboard" replace />`}var JM=2e3,XM=500,ZM=2e3,WM=new Set([403,404]),eO=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function tO(e=globalThis.location,t=null){let a=new URLSearchParams(e?.search||""),n={active:[]};for(let[r,s,i]of eO){let o=a.get(s)?.trim();o?(n[r]=o,n.active.push({key:r,param:s,labelKey:i,value:o})):n[r]=null}return!n.threadId&&t&&(n.threadId=t),n}function sR({isAdmin:e=!1,defaultThreadId:t=null}={}){let a=Pe(),n=a?.search||"",r=h.default.useMemo(()=>tO(a,t),[t,n]),{runId:s,source:i,threadId:o,toolCallId:l,toolName:c,turnId:d}=r,[m,f]=h.default.useState([]),[p,x]=h.default.useState("all"),[y,$]=h.default.useState(""),[g,v]=h.default.useState(!1),[b,w]=h.default.useState(!0),[S,C]=h.default.useState(!0),[_,A]=h.default.useState(null),L=h.default.useRef(new Set),M=h.default.useRef(0),P=!e&&!o;h.default.useEffect(()=>{M.current+=1,f([]),A(null)},[e,s,i,o,l,c,d]);let k=h.default.useCallback(async()=>{if(P){C(!1);return}let re=++M.current;C(!0);try{let me={limit:XM,level:p==="all"?null:p,target:y.trim()||null,threadId:o,runId:s,turnId:d,toolCallId:l,toolName:c,source:i},he;try{he=await(e?Qx(me):Mp(me))}catch(ft){if(!e||!WM.has(ft?.status))throw ft;he=await Mp(me)}if(re!==M.current)return;let Te=L.current,gt=QN(he).entries.filter(ft=>!Te.has(ft.id));f(gt),A(null)}catch(me){if(re!==M.current)return;A(me)}finally{re===M.current&&C(!1)}},[e,p,P,s,i,y,o,l,c,d]);h.default.useEffect(()=>{k()},[k]),h.default.useEffect(()=>{if(g||P)return;let re=setInterval(k,JM);return()=>clearInterval(re)},[k,P,g]);let z=h.default.useCallback(()=>{v(re=>!re)},[]),Z=h.default.useCallback(()=>{let re=[...L.current,...m.map(me=>me.id)].slice(-ZM);L.current=new Set(re),f([])},[m]);return{entries:m,totalCount:m.length,paused:g,togglePause:z,clearEntries:Z,levelFilter:p,setLevelFilter:x,targetFilter:y,setTargetFilter:$,autoScroll:b,setAutoScroll:w,serverLevel:null,changeServerLevel:async()=>{},scope:r,needsThreadScope:P,status:P?"needs_scope":_?"error":S?"loading":"ready",isLoading:S,error:_}}var aO=["all","trace","debug","info","warn","error"],nO=["trace","debug","info","warn","error"],iR={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},rO={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function sO({entry:e}){let t=R(),[a,n]=h.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=iR[e.level]||iR.info,i=rO[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(l=>!!l.value);return u`
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
  `}function oR({value:e,onChange:t,options:a,labelKey:n,t:r}){return u`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>u`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function iO({label:e,value:t,scopeKey:a}){return u`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function lR(){let e=R(),{isAdmin:t=!1,threadsState:a}=ya()||{},{entries:n,totalCount:r,paused:s,togglePause:i,clearEntries:o,levelFilter:l,setLevelFilter:c,targetFilter:d,setTargetFilter:m,autoScroll:f,setAutoScroll:p,serverLevel:x,changeServerLevel:y,scope:$,isLoading:g,error:v,needsThreadScope:b}=sR({isAdmin:t,defaultThreadId:t?null:a?.activeThreadId||null}),w=h.default.useRef(null),S=h.default.useRef(!0);h.default.useEffect(()=>{f&&S.current&&w.current&&(w.current.scrollTop=0)},[n,f]);let C=h.default.useCallback(L=>{S.current=L.currentTarget.scrollTop<=48},[]),_=n.length>0,A=$?.active||[];return u`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${oR}
          value=${l}
          onChange=${c}
          options=${aO}
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

        ${A.length>0&&u`
          <div
            data-testid="logs-scope-toolbar"
            className="flex w-full flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]"
          >
            <span className="font-medium text-[var(--v2-text-strong)]">${e("logs.scoped")}</span>
            ${A.map(L=>u`<${iO} key=${L.param} scopeKey=${L.param} label=${e(L.labelKey)} value=${L.value} />`)}
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
            <${oR}
              value=${x}
              onChange=${y}
              options=${nO}
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
              `:_?n.map(L=>u`<${sO} key=${L.id} entry=${L} />`):u`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function cR(){return u`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function oO({auth:e}){let t=pe(),n=Pe().state?.from,r=n?`${n.pathname||qr}${n.search||""}${n.hash||""}`:qr,s=`/v2${r==="/"?"":r}`,i=h.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?u`<${cR} />`:e.isAuthenticated?u`<${it} to=${r} replace />`:u`<${S1}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function lO({auth:e,children:t}){let a=Pe();return e.isChecking?u`<${cR} />`:e.isAuthenticated?t:u`<${it} to="/login" replace state=${{from:a}} />`}function uO({auth:e}){return u`
    <${lO} auth=${e}>
      <${Zw}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        rebornProjectsEnabled=${e.rebornProjectsEnabled}
        onSignOut=${e.signOut}
      />
    <//>
  `}function uR({auth:e}){return e.isAdmin?u`<${rR} />`:u`<${it} to=${qr} replace />`}function dR(){let e=A$();return u`
    <${Ep} basename="/v2">
      <${kp}>
        <${xe} path="/login" element=${u`<${oO} auth=${e} />`} />
        <${xe} path="/" element=${u`<${uO} auth=${e} />`}>
          <${xe} index element=${u`<${it} to=${qr} replace />`} />
          <${xe} path="overview" element=${u`<${it} to=${qr} replace />`} />
          <${xe} path="welcome" element=${u`<${Y2} />`} />
          <${xe} path="chat" element=${u`<${wh} />`} />
          <${xe} path="chat/:threadId" element=${u`<${wh} />`} />
          <${xe} path="workspace" element=${u`<${Nh} />`} />
          <${xe} path="workspace/*" element=${u`<${Nh} />`} />
          <${xe} path="projects" element=${u`<${fl} />`} />
          <${xe} path="projects/:projectId" element=${u`<${fl} />`} />
          <${xe} path="projects/:projectId/missions/:missionId" element=${u`<${fl} />`} />
          <${xe} path="projects/:projectId/threads/:threadId" element=${u`<${fl} />`} />
          <${xe} path="missions" element=${u`<${kh} />`} />
          <${xe} path="missions/:missionId" element=${u`<${kh} />`} />
          <${xe} path="jobs" element=${u`<${Eh} />`} />
          <${xe} path="jobs/:jobId" element=${u`<${Eh} />`} />
          <${xe} path="routines" element=${u`<${Ah} />`} />
          <${xe} path="routines/:routineId" element=${u`<${Ah} />`} />
          <${xe} path="automations" element=${u`<${n_} />`} />
          <${xe} path="extensions" element=${u`<${Ih} />`} />
          <${xe} path="extensions/:tab" element=${u`<${Ih} />`} />
          <${xe} path="logs" element=${u`<${lR} />`} />
          <${xe} path="settings" element=${u`<${Gh} />`} />
          <${xe} path="settings/:tab" element=${u`<${Gh} />`} />
          <${xe} path="admin" element=${u`<${uR} auth=${e} />`} />
          <${xe} path="admin/:tab" element=${u`<${uR} auth=${e} />`} />
        <//>
        <${xe} path="*" element=${u`<${it} to=${qr} replace />`} />
      <//>
    <//>
  `}Xh("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","approval.viewFullCommand":"View full command","approval.showCommandPreview":"Show preview","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.tabDeclined":"Declined","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitDeclined":"declined","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.serviceUnavailable":"Service unavailable","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveEligibleTools":"Always allow eligible tools","settings.field.autoApproveEligibleToolsDesc":"Applies to tools set to \u201CFollow global\u201D. Per-tool settings override this; hard-floor approval gates still ask.","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.followDefault":"Follow global","tools.sourceDefault":"default permission","tools.sourceGlobal":"global setting","tools.sourceOverride":"per-tool override","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","skills.defaultAutoActivationEnabled":"Default skill auto-activation enabled","skills.defaultAutoActivationDisabled":"Default skill auto-activation disabled","skills.defaultAutoActivationOnDesc":"Skills auto-activate by keyword on matching requests. Turn off to require an explicit /name.","skills.defaultAutoActivationOffDesc":"Skills run only when you type /name. Turn on to let them auto-activate by keyword.","skills.defaultAutoActivationOnButton":"Default: On","skills.defaultAutoActivationOffButton":"Default: Off","skills.autoActivateOnTitle":"Auto-activation on \u2014 runs on matching requests. Click to make it explicit-only (/name).","skills.autoActivateOffTitle":"Explicit-only \u2014 runs only when you type /name. Click to enable auto-activation.","skills.autoActivateOnLabel":"Auto-activate: On","skills.autoActivateOffLabel":"Auto-activate: Off","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Memory, files & attachments","workspace.readOnly":"Read-only","workspace.filterPlaceholder":"Filter by name\u2026","workspace.emptyDir":"This folder is empty.","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.noFiles":"No files here.","workspace.noMatches":"Nothing matches that filter.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a file","workspace.pickFileDesc":"Choose a file from the tree to preview or download it. This viewer is read-only.","workspace.parent":"Parent: {path}","workspace.download":"Download","workspace.binaryPreviewUnavailable":"No inline preview for this file type. Download it to view the contents.","workspace.fileMeta":"{mime} \xB7 {size} bytes","workspace.unableOpenDirectory":"Unable to open directory","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.filter.completed":"Completed","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.schedule.onceAt":"Once on {datetime}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.filterAction":"Show {label}","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.empty.onboardingTitle":"No automations yet","automations.empty.onboardingDescription":"Automations are created by chatting with your agent \u2014 there's no form to fill out. Ask it to do something on a schedule and it will set up a recurring automation for you.","automations.empty.examplesTitle":"Try asking your agent","automations.empty.example1":"Check the nearai/ironclaw repo every 10 minutes and summarize new issues, PRs, and commits.","automations.empty.example2":"Every weekday at 9am, send me a summary of my unread email.","automations.empty.example3":"Remind me to review open pull requests every afternoon at 3pm.","automations.empty.startInChat":"Start in chat","automations.empty.copyPrompt":"Copy prompt","automations.empty.copied":"Copied","automations.refreshing":"Refreshing\u2026","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.runs.total":"Recent runs: {count}","automations.runs.ok":"OK: {count}","automations.runs.error":"Failed: {count}","automations.runs.running":"Running: {count}","automations.runs.unknown":"Unknown: {count}","automations.runs.showingOf":"Showing {shown} of {total} recent runs","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up a project for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open project","projects.openGeneralWorkspace":"Open project","projects.noDescription":"No project description yet. The project is still being shaped by recent activity and thread history.","projects.general.label":"General project","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,mR.createRoot)(document.getElementById("v2-root")).render(u`
  <${Zh}>
    <${Dd} client=${Ct}>
      <${dR} />
    <//>
  <//>
`);
